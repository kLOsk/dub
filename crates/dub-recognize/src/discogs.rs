//! Discogs: the pressing's own paperwork.
//!
//! Discogs has **no audio fingerprinting**. It cannot tell us what a
//! rip is; it can only say more about a release something else already
//! identified. So this is strictly enrichment, and it runs last:
//! genre, style, country, year, label and the catalogue number stamped
//! in the run-out — the fields a DJ files a record under.
//!
//! **How the release is found matters more than what comes back.**
//! Discogs' search would happily return a plausible-looking pressing
//! for a fuzzy artist/title query, and attaching the wrong pressing's
//! catalogue number is worse than attaching none: it is a confident
//! lie stamped into a file that outlives the session. So this does not
//! search. MusicBrainz releases carry a Discogs link in their
//! `url-rels` relationships, curated by hand, and that link is the only
//! way in. No link, no enrichment.
//!
//! The token is a *user* credential, unlike the AcoustID application
//! key: it belongs in the Keychain, never in the repo and never in a
//! URL query string, so it travels in an `Authorization` header.

use serde::Deserialize;

use crate::error::RecognizeError;
use crate::http::Http;

const BASE: &str = "https://api.discogs.com";
const SERVICE: &str = "Discogs";

/// What Discogs knows about a pressing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscogsRelease {
    /// Discogs release id, as written into the `DISCOGS_RELEASE_ID` tag.
    pub id: String,
    /// Broad genres ("Funk / Soul").
    pub genres: Vec<String>,
    /// Finer styles ("Disco", "Philly Soul") — usually the more useful
    /// of the two for filing a record.
    pub styles: Vec<String>,
    /// Year of *this* pressing, which is often not the year of the
    /// recording.
    pub year: Option<i32>,
    /// Country of pressing.
    pub country: Option<String>,
    /// Label name.
    pub label: Option<String>,
    /// Catalogue number — what is stamped in the run-out groove.
    pub catalog_number: Option<String>,
}

impl DiscogsRelease {
    /// Genre for a tag: the most specific thing Discogs offers.
    ///
    /// Styles are preferred over genres because "Philly Soul" files a
    /// record and "Funk / Soul" does not. Joined with `; ` so a
    /// multi-style release keeps everything rather than picking one
    /// arbitrarily.
    #[must_use]
    pub fn genre_tag(&self) -> Option<String> {
        let source = if self.styles.is_empty() {
            &self.genres
        } else {
            &self.styles
        };
        (!source.is_empty()).then(|| source.join("; "))
    }
}

// --- wire types -------------------------------------------------------

#[derive(Deserialize)]
struct ReleaseBody {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    year: Option<i32>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    styles: Vec<String>,
    #[serde(default)]
    labels: Vec<LabelRow>,
}

#[derive(Deserialize)]
struct LabelRow {
    #[serde(default)]
    name: String,
    #[serde(default)]
    catno: Option<String>,
}

// ----------------------------------------------------------------------

/// Pull one Discogs release by id.
///
/// # Errors
///
/// [`RecognizeError::MissingCredential`] when `token` is blank — the
/// request is not spent, because Discogs answers an unauthenticated
/// release fetch with a thinner body rather than an error, and silently
/// returning less would look like a sparse release.
pub fn release(
    http: &dyn Http,
    token: &str,
    release_id: &str,
) -> Result<DiscogsRelease, RecognizeError> {
    if token.trim().is_empty() {
        return Err(RecognizeError::MissingCredential {
            service: SERVICE,
            what: "personal access token (Discogs → Settings → Developers)",
        });
    }
    let url = format!("{BASE}/releases/{release_id}");
    let auth = format!("Discogs token={token}");
    let body = http.get(
        &url,
        &[("User-Agent", crate::USER_AGENT), ("Authorization", &auth)],
    )?;
    parse(&body)
}

fn parse(body: &str) -> Result<DiscogsRelease, RecognizeError> {
    let parsed: ReleaseBody =
        serde_json::from_str(body).map_err(|e| RecognizeError::Malformed {
            service: SERVICE,
            detail: e.to_string(),
        })?;
    let (label, catalog_number) = parsed.labels.into_iter().fold(
        (None, None),
        |(l, c): (Option<String>, Option<String>), row| {
            (
                l.or_else(|| (!row.name.is_empty()).then_some(row.name)),
                c.or(row.catno),
            )
        },
    );
    Ok(DiscogsRelease {
        id: parsed.id.to_string(),
        genres: parsed.genres,
        styles: parsed.styles,
        // Discogs writes 0 for "unknown", which would tag a record as
        // year zero rather than leaving the field off.
        year: parsed.year.filter(|y| *y > 0),
        country: parsed.country,
        label,
        catalog_number,
    })
}

/// Pull the Discogs release id out of a MusicBrainz relation URL.
///
/// Accepts the shapes MusicBrainz actually stores — with or without
/// `www.`, http or https, and with a trailing slash or path — and
/// rejects anything that is not a *release* URL, because Discogs
/// `/master/` ids live in a different namespace and fetching one as a
/// release returns a different record.
#[must_use]
pub fn release_id_from_url(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("discogs.com/")?;
    let tail = rest.split_once("release/").map(|(_, t)| t)?;
    let id: String = tail.chars().take_while(char::is_ascii_digit).collect();
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::StubHttp;

    const RELEASE: &str = r#"{
      "id": 438249,
      "year": 1973,
      "country": "US",
      "genres": ["Funk / Soul"],
      "styles": ["Disco", "Philly Soul"],
      "labels": [{"name": "Philadelphia International Records", "catno": "KZ 32408"}]
    }"#;

    #[test]
    fn reads_the_pressing_paperwork() {
        let http = StubHttp::new().on("/releases/438249", RELEASE);
        let got = release(&http, "tok", "438249").unwrap();
        assert_eq!(got.id, "438249");
        assert_eq!(got.year, Some(1973));
        assert_eq!(got.country.as_deref(), Some("US"));
        assert_eq!(
            got.label.as_deref(),
            Some("Philadelphia International Records")
        );
        assert_eq!(got.catalog_number.as_deref(), Some("KZ 32408"));
    }

    /// The token is a user credential. A query string ends up in server
    /// logs and in shell history; a header does not.
    #[test]
    fn the_token_travels_in_a_header_not_the_url() {
        let http = StubHttp::new().on("/releases/438249", RELEASE);
        release(&http, "s3cret", "438249").unwrap();
        assert!(
            !http.requests()[0].contains("s3cret"),
            "token leaked into the URL: {}",
            http.requests()[0]
        );
    }

    #[test]
    fn a_missing_token_never_reaches_the_network() {
        let http = StubHttp::new();
        let err = release(&http, "  ", "438249").unwrap_err();
        assert!(
            matches!(err, RecognizeError::MissingCredential { .. }),
            "{err}"
        );
        assert!(http.requests().is_empty());
    }

    /// Styles file a record; genres barely narrow it.
    #[test]
    fn the_genre_tag_prefers_styles() {
        let http = StubHttp::new().on("/releases/438249", RELEASE);
        let got = release(&http, "tok", "438249").unwrap();
        assert_eq!(got.genre_tag().as_deref(), Some("Disco; Philly Soul"));
    }

    #[test]
    fn the_genre_tag_falls_back_to_genres_when_there_are_no_styles() {
        let body = r#"{"id":1,"genres":["Reggae"],"styles":[]}"#;
        let http = StubHttp::new().on("/releases/1", body);
        let got = release(&http, "tok", "1").unwrap();
        assert_eq!(got.genre_tag().as_deref(), Some("Reggae"));
    }

    #[test]
    fn a_release_with_nothing_to_add_is_not_an_error() {
        let http = StubHttp::new().on("/releases/9", r#"{"id":9}"#);
        let got = release(&http, "tok", "9").unwrap();
        assert_eq!(got.genre_tag(), None);
        assert_eq!(got.year, None);
    }

    /// Discogs writes 0 for an unknown year, which would tag a record
    /// as year zero.
    #[test]
    fn a_zero_year_is_treated_as_unknown() {
        let http = StubHttp::new().on("/releases/9", r#"{"id":9,"year":0}"#);
        assert_eq!(release(&http, "tok", "9").unwrap().year, None);
    }

    #[test]
    fn release_urls_yield_their_id() {
        for url in [
            "https://www.discogs.com/release/438249",
            "http://discogs.com/release/438249",
            "https://www.discogs.com/release/438249-The-OJays-Ship-Ahoy",
            "https://www.discogs.com/en/release/438249",
        ] {
            assert_eq!(
                release_id_from_url(url).as_deref(),
                Some("438249"),
                "failed on {url}"
            );
        }
    }

    /// A master is a different namespace — fetching one as a release
    /// returns a different record, so it must not be mistaken for one.
    #[test]
    fn a_master_url_is_not_a_release() {
        assert_eq!(
            release_id_from_url("https://www.discogs.com/master/12345"),
            None
        );
        assert_eq!(release_id_from_url("https://example.test/release/1"), None);
    }
}
