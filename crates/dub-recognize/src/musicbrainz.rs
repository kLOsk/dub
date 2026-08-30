//! MusicBrainz: recordings in, releases and tracklists out.
//!
//! Two calls. `releases_for_recording` asks which releases contain a
//! recording — that is what lets a side vote on a release rather than
//! each track guessing alone. `release` fetches the full tracklist,
//! including the vinyl track *numbers* ("A1", "B3"), which are the
//! reason this is worth doing properly: a rip is one **side**, and a
//! side maps to a contiguous run of one letter.
//!
//! MusicBrainz asks for one request per second and a descriptive
//! User-Agent, and enforces both. The rate limit lives in the `Http`
//! implementation so it applies to whatever calls this.

use serde::Deserialize;

use crate::error::RecognizeError;
use crate::http::Http;

const BASE: &str = "https://musicbrainz.org/ws/2";
const SERVICE: &str = "MusicBrainz";

/// A release a recording appears on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseRef {
    /// MusicBrainz release id.
    pub mbid: String,
    /// Release title.
    pub title: String,
    /// Release date as MusicBrainz has it — often just a year.
    pub date: Option<String>,
}

/// One track on a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// The recording this track is, which is what AcoustID matched.
    pub recording_mbid: String,
    /// Track title.
    pub title: String,
    /// Printed track number. On vinyl this is a side letter and an
    /// index — "A1", "B3" — and on CD just "1".
    pub number: String,
    /// Length in milliseconds, when known.
    pub length_ms: Option<u32>,
}

impl Track {
    /// The vinyl side letter, when the number looks like one.
    ///
    /// A rip is one side, so this is what lets a matched release be
    /// narrowed to the half the operator actually recorded.
    #[must_use]
    pub fn side(&self) -> Option<char> {
        let c = self.number.chars().next()?;
        c.is_ascii_alphabetic().then(|| c.to_ascii_uppercase())
    }
}

/// A release with its tracklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// MusicBrainz release id.
    pub mbid: String,
    /// Release title.
    pub title: String,
    /// Credited artist, joined with MusicBrainz's own join phrases.
    pub artist: Option<String>,
    /// Release date, often just a year.
    pub date: Option<String>,
    /// Label name, when the release carries one.
    pub label: Option<String>,
    /// Catalogue number — what is actually stamped in the run-out.
    pub catalog_number: Option<String>,
    /// Discogs release id, from MusicBrainz's own curated link. The
    /// only way this crate reaches Discogs: searching for a pressing by
    /// name would sooner or later attach the wrong one's paperwork.
    pub discogs_release_id: Option<String>,
    /// Every track, in order, across all media.
    pub tracks: Vec<Track>,
}

// --- wire types -------------------------------------------------------

#[derive(Deserialize)]
struct RecordingBody {
    #[serde(default)]
    releases: Vec<ReleaseRow>,
}

#[derive(Deserialize)]
struct ReleaseRow {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Deserialize)]
struct ReleaseBody {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default, rename = "label-info")]
    label_info: Vec<LabelInfo>,
    #[serde(default)]
    media: Vec<Medium>,
    #[serde(default)]
    relations: Vec<Relation>,
}

#[derive(Deserialize)]
struct Relation {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    url: Option<UrlRow>,
}

#[derive(Deserialize)]
struct UrlRow {
    #[serde(default)]
    resource: String,
}

#[derive(Deserialize)]
struct ArtistCredit {
    #[serde(default)]
    name: String,
    #[serde(default)]
    joinphrase: String,
}

#[derive(Deserialize)]
struct LabelInfo {
    #[serde(default, rename = "catalog-number")]
    catalog_number: Option<String>,
    #[serde(default)]
    label: Option<LabelRow>,
}

#[derive(Deserialize)]
struct LabelRow {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct Medium {
    #[serde(default)]
    tracks: Vec<TrackRow>,
}

#[derive(Deserialize)]
struct TrackRow {
    #[serde(default)]
    title: String,
    #[serde(default)]
    number: String,
    #[serde(default)]
    length: Option<u32>,
    #[serde(default)]
    recording: Option<RecordingRef>,
}

#[derive(Deserialize)]
struct RecordingRef {
    id: String,
}

// ----------------------------------------------------------------------

fn headers() -> [(&'static str, &'static str); 1] {
    [("User-Agent", crate::USER_AGENT)]
}

fn malformed(e: serde_json::Error) -> RecognizeError {
    RecognizeError::Malformed {
        service: SERVICE,
        detail: e.to_string(),
    }
}

/// Which releases contain this recording.
pub fn releases_for_recording(
    http: &dyn Http,
    recording_mbid: &str,
) -> Result<Vec<ReleaseRef>, RecognizeError> {
    let url = format!("{BASE}/recording/{recording_mbid}?inc=releases&fmt=json");
    let body = http.get(&url, &headers())?;
    let parsed: RecordingBody = serde_json::from_str(&body).map_err(malformed)?;
    Ok(parsed
        .releases
        .into_iter()
        .map(|r| ReleaseRef {
            mbid: r.id,
            title: r.title,
            date: r.date,
        })
        .collect())
}

/// Fetch a release with its full tracklist.
pub fn release(http: &dyn Http, release_mbid: &str) -> Result<Release, RecognizeError> {
    let url = format!(
        "{BASE}/release/{release_mbid}         ?inc=recordings+artist-credits+labels+url-rels&fmt=json"
    );
    let body = http.get(&url, &headers())?;
    let parsed: ReleaseBody = serde_json::from_str(&body).map_err(malformed)?;

    let artist = (!parsed.artist_credit.is_empty()).then(|| {
        parsed
            .artist_credit
            .iter()
            .map(|c| format!("{}{}", c.name, c.joinphrase))
            .collect::<String>()
            .trim()
            .to_string()
    });
    let (label, catalog_number) = parsed.label_info.into_iter().fold(
        (None, None),
        |(l, c): (Option<String>, Option<String>), info| {
            (
                l.or_else(|| info.label.map(|x| x.name).filter(|n| !n.is_empty())),
                c.or(info.catalog_number),
            )
        },
    );

    let discogs_release_id = parsed
        .relations
        .iter()
        .filter(|r| r.kind == "discogs")
        .filter_map(|r| r.url.as_ref())
        .find_map(|u| crate::discogs::release_id_from_url(&u.resource));

    let tracks = parsed
        .media
        .into_iter()
        .flat_map(|m| m.tracks)
        .filter_map(|t| {
            // A track with no recording id cannot be matched against an
            // AcoustID result, so it is not useful here.
            t.recording.map(|rec| Track {
                recording_mbid: rec.id,
                title: t.title,
                number: t.number,
                length_ms: t.length,
            })
        })
        .collect();

    Ok(Release {
        mbid: parsed.id,
        title: parsed.title,
        artist,
        date: parsed.date,
        label,
        catalog_number,
        discogs_release_id,
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::StubHttp;

    const RECORDING: &str = r#"{
      "id": "rec-a",
      "releases": [
        {"id": "rel-lp", "title": "There Is", "date": "1968"},
        {"id": "rel-comp", "title": "Greatest Hits", "date": "1998"}
      ]
    }"#;

    const RELEASE: &str = r#"{
      "id": "rel-lp",
      "title": "There Is",
      "date": "1968-05",
      "artist-credit": [{"name": "The Dells", "joinphrase": ""}],
      "label-info": [{"catalog-number": "LPS-804", "label": {"name": "Cadet"}}],
      "media": [{"tracks": [
        {"number": "A1", "title": "Stay In My Corner", "length": 253000,
         "recording": {"id": "rec-a"}},
        {"number": "A2", "title": "Wear It On Our Face", "length": 160000,
         "recording": {"id": "rec-b"}},
        {"number": "B1", "title": "Does Anybody Know", "length": 180000,
         "recording": {"id": "rec-c"}}
      ]}]
    }"#;

    #[test]
    fn lists_the_releases_a_recording_appears_on() {
        let http = StubHttp::new().on("/recording/rec-a", RECORDING);
        let got = releases_for_recording(&http, "rec-a").unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].mbid, "rel-lp");
        assert_eq!(got[0].date.as_deref(), Some("1968"));
    }

    #[test]
    fn asks_for_releases_and_json() {
        let http = StubHttp::new().on("/recording/rec-a", RECORDING);
        releases_for_recording(&http, "rec-a").unwrap();
        let req = &http.requests()[0];
        assert!(req.contains("inc=releases"), "{req}");
        assert!(req.contains("fmt=json"), "{req}");
    }

    #[test]
    fn reads_a_tracklist_with_label_and_catalogue_number() {
        let http = StubHttp::new().on("/release/rel-lp", RELEASE);
        let rel = release(&http, "rel-lp").unwrap();
        assert_eq!(rel.title, "There Is");
        assert_eq!(rel.artist.as_deref(), Some("The Dells"));
        assert_eq!(rel.label.as_deref(), Some("Cadet"));
        assert_eq!(rel.catalog_number.as_deref(), Some("LPS-804"));
        assert_eq!(rel.tracks.len(), 3);
        assert_eq!(rel.tracks[0].recording_mbid, "rec-a");
        assert_eq!(rel.tracks[0].length_ms, Some(253_000));
    }

    /// The reason the tracklist is worth fetching: a rip is one side.
    #[test]
    fn vinyl_track_numbers_carry_their_side() {
        let http = StubHttp::new().on("/release/rel-lp", RELEASE);
        let rel = release(&http, "rel-lp").unwrap();
        let sides: Vec<Option<char>> = rel.tracks.iter().map(Track::side).collect();
        assert_eq!(sides, vec![Some('A'), Some('A'), Some('B')]);
    }

    #[test]
    fn a_cd_track_number_has_no_side() {
        let t = Track {
            recording_mbid: "r".into(),
            title: "x".into(),
            number: "7".into(),
            length_ms: None,
        };
        assert_eq!(t.side(), None);
    }

    #[test]
    fn a_multi_artist_credit_joins_with_musicbrainz_phrases() {
        let body = r#"{"id":"r","title":"T","artist-credit":[
            {"name":"Booker T.","joinphrase":" & "},{"name":"the M.G.'s","joinphrase":""}],
            "media":[]}"#;
        let http = StubHttp::new().on("/release/r", body);
        let rel = release(&http, "r").unwrap();
        assert_eq!(rel.artist.as_deref(), Some("Booker T. & the M.G.'s"));
    }

    #[test]
    fn a_track_without_a_recording_id_is_dropped() {
        // Nothing downstream can match it, so carrying it would only
        // dilute the release vote.
        let body = r#"{"id":"r","title":"T","media":[{"tracks":[
            {"number":"A1","title":"has one","recording":{"id":"rec-a"}},
            {"number":"A2","title":"has none"}
        ]}]}"#;
        let http = StubHttp::new().on("/release/r", body);
        let rel = release(&http, "r").unwrap();
        assert_eq!(rel.tracks.len(), 1);
        assert_eq!(rel.tracks[0].recording_mbid, "rec-a");
    }
}
