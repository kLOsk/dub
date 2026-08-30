//! AcoustID: fingerprint in, candidate recordings out.
//!
//! One POST per track to `/v2/lookup`, asking for `recordings` metadata
//! so a match arrives with its MusicBrainz recording ids attached and we
//! do not pay a second round trip to learn what matched.
//!
//! A refusal has to be told apart from "no match", because turning
//! "your API key is wrong" into "this record is unknown" sends someone
//! hunting for a rare pressing over a typo. AcoustID signals it two
//! ways — verified against the live service, which answers a bad key
//! with **HTTP 400** and an error body, while some conditions come back
//! `200 OK` carrying `{"status":"error"}`. Both are handled: the error
//! body is parsed whichever status it arrives under.

use serde::Deserialize;

use crate::error::RecognizeError;
use crate::fingerprint::AcoustIdFingerprint;
use crate::http::Http;

const ENDPOINT: &str = "https://api.acoustid.org/v2/lookup";
const SERVICE: &str = "AcoustID";

/// One recording AcoustID thinks this audio might be.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// MusicBrainz recording id — the key into everything downstream.
    pub recording_mbid: String,
    /// AcoustID's confidence, 0..1, for the *fingerprint* match. It says
    /// nothing about which release the recording belongs to.
    pub score: f32,
    /// Recording title as AcoustID has it, when it has one.
    pub title: Option<String>,
    /// Credited artist, joined as AcoustID presents it.
    pub artist: Option<String>,
}

// --- wire types -------------------------------------------------------

#[derive(Deserialize)]
struct Response {
    status: String,
    #[serde(default)]
    error: Option<ErrorBody>,
    #[serde(default)]
    results: Vec<ResultRow>,
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
}

#[derive(Deserialize)]
struct ResultRow {
    #[serde(default)]
    score: f32,
    #[serde(default)]
    recordings: Vec<RecordingRow>,
}

#[derive(Deserialize)]
struct RecordingRow {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artists: Vec<ArtistRow>,
}

#[derive(Deserialize)]
struct ArtistRow {
    name: String,
}

// ----------------------------------------------------------------------

/// Look one fingerprint up.
///
/// Returns candidates best-score-first. An empty vector means "AcoustID
/// answered and knows nothing about this audio", which for a private
/// pressing or a white label is the expected outcome, not a failure.
pub fn lookup(
    http: &dyn Http,
    api_key: &str,
    fp: &AcoustIdFingerprint,
) -> Result<Vec<Candidate>, RecognizeError> {
    if api_key.trim().is_empty() {
        return Err(RecognizeError::MissingCredential {
            service: SERVICE,
            what: "API key (register a free one at acoustid.org/new-application)",
        });
    }
    let duration = fp.duration_secs.to_string();
    let body = match http.post_form(
        ENDPOINT,
        &[
            ("client", api_key),
            ("duration", &duration),
            ("fingerprint", &fp.encoded),
            ("meta", "recordings"),
        ],
        &[("User-Agent", crate::USER_AGENT)],
    ) {
        Ok(b) => b,
        // A non-2xx still carries AcoustID's own explanation. Parsing it
        // turns `http 400: {...json...}` into "AcoustID refused: invalid
        // API key", which is the difference between a user fixing their
        // key and a user filing a bug.
        Err(crate::http::HttpError::Status { status, body }) => {
            // Fall back to the raw status only when the body is not an
            // AcoustID error — an unreadable 502 from a proxy, say.
            return Err(refusal_in(&body)
                .unwrap_or_else(|| crate::http::HttpError::Status { status, body }.into()));
        }
        Err(e) => return Err(e.into()),
    };
    parse(&body)
}

/// Read AcoustID's own explanation out of a body, whatever status
/// carried it. `None` means this is not an error body.
fn refusal_in(body: &str) -> Option<RecognizeError> {
    let parsed: Response = serde_json::from_str(body).ok()?;
    (parsed.status != "ok").then(|| refusal(parsed))
}

fn refusal(parsed: Response) -> RecognizeError {
    RecognizeError::Refused {
        service: SERVICE,
        message: parsed.error.map_or(parsed.status, |e| e.message),
    }
}

fn parse(body: &str) -> Result<Vec<Candidate>, RecognizeError> {
    let parsed: Response = serde_json::from_str(body).map_err(|e| RecognizeError::Malformed {
        service: SERVICE,
        detail: e.to_string(),
    })?;

    if parsed.status != "ok" {
        return Err(refusal(parsed));
    }

    let mut out: Vec<Candidate> = parsed
        .results
        .into_iter()
        .flat_map(|row| {
            let score = row.score;
            row.recordings.into_iter().map(move |rec| Candidate {
                recording_mbid: rec.id,
                score,
                title: rec.title,
                artist: (!rec.artists.is_empty()).then(|| {
                    rec.artists
                        .into_iter()
                        .map(|a| a.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                }),
            })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::StubHttp;

    const FP: &str = r#"{
      "status": "ok",
      "results": [
        {"score": 0.91, "recordings": [
          {"id": "rec-b", "title": "Bad Weather", "artists": [{"name": "The Dells"}]}
        ]},
        {"score": 0.99, "recordings": [
          {"id": "rec-a", "title": "Stay In My Corner",
           "artists": [{"name": "The Dells"}, {"name": "Marvin Junior"}]}
        ]}
      ]
    }"#;

    fn fp() -> AcoustIdFingerprint {
        AcoustIdFingerprint {
            encoded: "AQAAtestfingerprint".into(),
            duration_secs: 253,
        }
    }

    #[test]
    fn returns_candidates_best_score_first() {
        let http = StubHttp::new().on("acoustid.org", FP);
        let got = lookup(&http, "test-key", &fp()).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].recording_mbid, "rec-a", "0.99 must sort above 0.91");
        assert_eq!(got[0].artist.as_deref(), Some("The Dells, Marvin Junior"));
        assert_eq!(got[1].recording_mbid, "rec-b");
    }

    #[test]
    fn sends_the_fingerprint_duration_and_key() {
        let http = StubHttp::new().on("acoustid.org", FP);
        lookup(&http, "test-key", &fp()).unwrap();
        let req = &http.requests()[0];
        assert!(req.contains("fingerprint=AQAAtestfingerprint"), "{req}");
        assert!(req.contains("duration=253"), "{req}");
        assert!(req.contains("client=test-key"), "{req}");
        assert!(
            req.contains("meta=recordings"),
            "must ask for recordings or the ids come back empty: {req}"
        );
    }

    const BAD_KEY: &str = r#"{"status":"error","error":{"code":4,"message":"invalid API key"}}"#;

    fn assert_bad_key(err: RecognizeError) {
        match err {
            RecognizeError::Refused { service, message } => {
                assert_eq!(service, "AcoustID");
                assert!(message.contains("invalid API key"), "{message}");
            }
            other => panic!("expected a refusal, got {other}"),
        }
    }

    /// What the live service actually does: HTTP 400 with the error body.
    /// Checking only the JSON `status` field would let this arrive as a
    /// bare `http 400: {...}` and send someone off to file a bug.
    #[test]
    fn a_bad_key_is_a_refusal_when_it_arrives_as_a_400() {
        let http = StubHttp::new().failing("acoustid.org", 400, BAD_KEY);
        assert_bad_key(lookup(&http, "wrong", &fp()).unwrap_err());
    }

    /// The other shape AcoustID uses for refusals.
    #[test]
    fn a_bad_key_is_a_refusal_when_it_arrives_as_a_200() {
        let http = StubHttp::new().on("acoustid.org", BAD_KEY);
        assert_bad_key(lookup(&http, "wrong", &fp()).unwrap_err());
    }

    #[test]
    fn a_non_acoustid_error_body_keeps_its_status() {
        // A proxy's HTML 502 is not a refusal and must not be dressed up
        // as one — the status is the only diagnostic it carries.
        let http = StubHttp::new().failing("acoustid.org", 502, "<html>bad gateway</html>");
        let err = lookup(&http, "k", &fp()).unwrap_err();
        assert!(format!("{err}").contains("502"), "{err}");
    }

    #[test]
    fn an_unknown_pressing_is_an_empty_answer_not_an_error() {
        let http = StubHttp::new().on("acoustid.org", r#"{"status":"ok","results":[]}"#);
        assert!(lookup(&http, "k", &fp()).unwrap().is_empty());
    }

    #[test]
    fn a_missing_key_never_reaches_the_network() {
        let http = StubHttp::new();
        let err = lookup(&http, "   ", &fp()).unwrap_err();
        assert!(
            matches!(err, RecognizeError::MissingCredential { .. }),
            "{err}"
        );
        assert!(
            http.requests().is_empty(),
            "must not spend a request without a key"
        );
    }

    #[test]
    fn a_match_with_no_metadata_still_yields_its_mbid() {
        // The id is what everything downstream needs; title and artist
        // are conveniences AcoustID does not always carry.
        let http = StubHttp::new().on(
            "acoustid.org",
            r#"{"status":"ok","results":[{"score":0.8,"recordings":[{"id":"rec-x"}]}]}"#,
        );
        let got = lookup(&http, "k", &fp()).unwrap();
        assert_eq!(got[0].recording_mbid, "rec-x");
        assert!(got[0].title.is_none() && got[0].artist.is_none());
    }
}
