//! The network boundary.
//!
//! Every request this crate makes goes through [`Http`]. That exists so
//! the rest of the crate — fingerprint encoding, response parsing, the
//! release-consensus logic that is the actual value here — is testable
//! without a network, an API key, or a third party's uptime. It also
//! makes the dependency auditable: `ureq` is reachable from exactly one
//! type in this file, and nothing else in the workspace links it.
//!
//! Blocking, not async. Recognition is a background batch job over a
//! handful of tracks; an async runtime would be a large dependency
//! bought for nothing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What went wrong reaching a service.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("network error: {0}")]
    Transport(String),

    /// A response with a non-2xx status. `body` is kept because both
    /// AcoustID and MusicBrainz put the useful part of an error there.
    #[error("http {status}: {body}")]
    Status { status: u16, body: String },
}

/// A blocking HTTP client, narrow enough to stub in a test.
pub trait Http: Send + Sync {
    /// GET `url`, returning the body.
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<String, HttpError>;

    /// POST `form` as `application/x-www-form-urlencoded`.
    ///
    /// AcoustID accepts a lookup as either, but a fingerprint is one to
    /// two kilobytes of base64 and belongs in a body rather than a URL.
    fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<String, HttpError>;
}

/// The real client.
pub struct UreqHttp {
    agent: ureq::Agent,
    /// Minimum spacing between requests. MusicBrainz's terms are one
    /// request per second per application and they enforce it; going
    /// faster earns a 503 and, sustained, a block.
    min_interval: Duration,
    last: Mutex<Option<Instant>>,
    /// Requests made, for the CLI to report.
    count: AtomicU64,
}

impl UreqHttp {
    /// A client that spaces requests at least `min_interval` apart.
    ///
    /// Pass the service's own limit — MusicBrainz is 1 s, AcoustID
    /// asks for 3 requests/second, Discogs 60/minute authenticated.
    #[must_use]
    pub fn new(min_interval: Duration) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .timeout_read(Duration::from_secs(30))
                .build(),
            min_interval,
            last: Mutex::new(None),
            count: AtomicU64::new(0),
        }
    }

    /// How many requests this client has made.
    #[must_use]
    pub fn request_count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Block until the rate limit allows another request.
    fn throttle(&self) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.min_interval {
                std::thread::sleep(self.min_interval - elapsed);
            }
        }
        *last = Some(Instant::now());
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

/// How many times a request is attempted before giving up.
const MAX_ATTEMPTS: u32 = 4;

/// Statuses that mean "busy, come back" rather than "no".
///
/// MusicBrainz answers 503 when its web server is loaded and asks
/// clients to back off and retry. 429 is the explicit rate-limit signal.
/// Everything else — a 400 bad key, a 404 — fails again identically, so
/// retrying it only spends the operator's time to reach the same answer.
fn is_backpressure(status: u16) -> bool {
    matches!(status, 429 | 503)
}

/// Run `send`, retrying backpressure responses with doubling backoff.
///
/// Recognition is a sequential batch over a whole side, so one transient
/// 503 partway through would otherwise throw away every lookup already
/// paid for — measured: a 10-track side died on its MusicBrainz calls
/// after all ten AcoustID lookups had been spent. `sleep` is injected so
/// the policy is testable without actually waiting.
fn with_retry(
    attempts: u32,
    sleep: &mut dyn FnMut(Duration),
    send: &mut dyn FnMut() -> Result<String, HttpError>,
) -> Result<String, HttpError> {
    let mut backoff = Duration::from_secs(1);
    for _ in 1..attempts.max(1) {
        match send() {
            Err(HttpError::Status { status, .. }) if is_backpressure(status) => {
                sleep(backoff);
                backoff *= 2;
            }
            other => return other,
        }
    }
    send()
}

fn finish(resp: Result<ureq::Response, ureq::Error>) -> Result<String, HttpError> {
    match resp {
        Ok(r) => r
            .into_string()
            .map_err(|e| HttpError::Transport(e.to_string())),
        Err(ureq::Error::Status(status, r)) => {
            let body = r.into_string().unwrap_or_default();
            Err(HttpError::Status { status, body })
        }
        Err(e) => Err(HttpError::Transport(e.to_string())),
    }
}

impl Http for UreqHttp {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<String, HttpError> {
        with_retry(MAX_ATTEMPTS, &mut std::thread::sleep, &mut || {
            self.throttle();
            let mut req = self.agent.get(url);
            for (k, v) in headers {
                req = req.set(k, v);
            }
            finish(req.call())
        })
    }

    fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<String, HttpError> {
        with_retry(MAX_ATTEMPTS, &mut std::thread::sleep, &mut || {
            self.throttle();
            let mut req = self.agent.post(url);
            for (k, v) in headers {
                req = req.set(k, v);
            }
            finish(req.send_form(form))
        })
    }
}

/// A canned-response client for tests and dry runs.
///
/// Matches on a substring of the URL — the first rule whose key appears
/// in the request wins — so a test can answer "the AcoustID call" and
/// "the MusicBrainz call" without reconstructing exact query strings.
/// Records every request it saw, so a test can assert on *what was
/// asked*, which is usually the interesting half.
#[derive(Default)]
pub struct StubHttp {
    rules: Vec<(String, u16, String)>,
    seen: Mutex<Vec<String>>,
}

impl StubHttp {
    /// A stub with no rules — every request is an error naming the URL.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer any request whose URL or body contains `needle` with `body`.
    #[must_use]
    pub fn on(mut self, needle: &str, body: &str) -> Self {
        self.rules.push((needle.to_string(), 200, body.to_string()));
        self
    }

    /// Answer `needle` with a non-2xx and a body.
    ///
    /// Both services put their real explanation in the body of an error
    /// response, so the status and the body have to travel together for
    /// a test to cover how that is read.
    #[must_use]
    pub fn failing(mut self, needle: &str, status: u16, body: &str) -> Self {
        self.rules
            .push((needle.to_string(), status, body.to_string()));
        self
    }

    /// Every request made, in order, as `"<url> <form-encoded body>"`.
    #[must_use]
    pub fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn answer(&self, key: &str) -> Result<String, HttpError> {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(key.to_string());
        for (needle, status, body) in &self.rules {
            if key.contains(needle.as_str()) {
                return match status {
                    200 => Ok(body.clone()),
                    &status => Err(HttpError::Status {
                        status,
                        body: body.clone(),
                    }),
                };
            }
        }
        Err(HttpError::Status {
            status: 404,
            body: format!("StubHttp has no rule matching {key}"),
        })
    }
}

impl Http for StubHttp {
    fn get(&self, url: &str, _headers: &[(&str, &str)]) -> Result<String, HttpError> {
        self.answer(url)
    }

    fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
        _headers: &[(&str, &str)],
    ) -> Result<String, HttpError> {
        let body: Vec<String> = form.iter().map(|(k, v)| format!("{k}={v}")).collect();
        self.answer(&format!("{url} {}", body.join("&")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_matches_on_a_substring_and_records_what_was_asked() {
        let http = StubHttp::new()
            .on("acoustid", r#"{"status":"ok"}"#)
            .on("musicbrainz", r#"{"releases":[]}"#);

        assert_eq!(
            http.get("https://api.acoustid.org/v2/lookup?x=1", &[])
                .unwrap(),
            r#"{"status":"ok"}"#
        );
        assert_eq!(
            http.get("https://musicbrainz.org/ws/2/recording/abc", &[])
                .unwrap(),
            r#"{"releases":[]}"#
        );
        assert_eq!(http.requests().len(), 2);
        assert!(http.requests()[0].contains("acoustid"));
    }

    #[test]
    fn an_unmatched_request_is_an_error_naming_what_was_asked() {
        let http = StubHttp::new();
        let err = http.get("https://example.test/thing", &[]).unwrap_err();
        assert!(
            format!("{err}").contains("example.test/thing"),
            "the error should name the unmatched URL: {err}"
        );
    }

    #[test]
    fn a_busy_service_is_retried_until_it_answers() {
        // MusicBrainz's 503 means "come back", so the batch must survive
        // it rather than lose every lookup already paid for.
        let mut calls = 0;
        let mut slept = Vec::new();
        let got = with_retry(4, &mut |d| slept.push(d), &mut || {
            calls += 1;
            if calls < 3 {
                Err(HttpError::Status {
                    status: 503,
                    body: "busy".into(),
                })
            } else {
                Ok("{}".to_string())
            }
        })
        .unwrap();
        assert_eq!(got, "{}");
        assert_eq!(calls, 3);
        assert_eq!(
            slept,
            vec![Duration::from_secs(1), Duration::from_secs(2)],
            "backoff must widen between attempts"
        );
    }

    #[test]
    fn a_persistent_outage_gives_up_rather_than_hanging_on() {
        let mut calls = 0;
        let err = with_retry(4, &mut |_| {}, &mut || {
            calls += 1;
            Err(HttpError::Status {
                status: 503,
                body: "busy".into(),
            })
        })
        .unwrap_err();
        assert!(format!("{err}").contains("503"), "{err}");
        assert_eq!(calls, 4, "MAX_ATTEMPTS is the ceiling, not a suggestion");
    }

    #[test]
    fn a_refusal_is_never_retried() {
        // A bad key answers 400 every time; retrying it only makes the
        // operator wait longer to be told the same thing.
        let mut calls = 0;
        let _ = with_retry(
            4,
            &mut |_| panic!("must not back off for a 400"),
            &mut || {
                calls += 1;
                Err(HttpError::Status {
                    status: 400,
                    body: "invalid API key".into(),
                })
            },
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn post_form_matches_on_the_body_too() {
        // A fingerprint lookup is a POST; a test wants to answer it
        // without knowing the fingerprint bytes.
        let http = StubHttp::new().on("fingerprint=AQAA", r#"{"status":"ok"}"#);
        let body = http
            .post_form(
                "https://api.acoustid.org/v2/lookup",
                &[("fingerprint", "AQAAbcdef"), ("duration", "180")],
                &[],
            )
            .unwrap();
        assert_eq!(body, r#"{"status":"ok"}"#);
    }
}
