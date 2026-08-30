//! Naming what was ripped.
//!
//! What a DJ needs off a rip is **artist and title**, per track. A rip
//! is frequently a *single song* lifted off a record rather than a whole
//! side, so there is not always a side to reason about, and the album is
//! useful-to-some rather than the point.
//!
//! So naming is the primary path, and it is cheap: AcoustID returns the
//! recording's title and artist alongside the fingerprint match, which
//! is all it takes to tag a file. No MusicBrainz call is involved, which
//! keeps the common case clear of its one-request-a-second throttle and
//! of its 503s.
//!
//! The release is **opt-in enrichment** — see
//! [`Recognizer::with_release_lookup`]. When asked for, the segments
//! vote: each candidate recording is asked which releases contain it,
//! and the release explaining the most segments wins. That earns the
//! album, date, label, catalogue number, and the vinyl track numbers
//! ("A1", "B3") that a whole-side rip wants. The vote is deliberately
//! not allowed to *gate* naming.
//!
//! That last point is the correction of an earlier design. Making the
//! release vote the gate looked right — a side really is one release in
//! one order — but measured against a real O'Jays sampler it named 2 of
//! 10 tracks while AcoustID had confidently identified 6, because
//! MusicBrainz models each compilation appearance as its own recording
//! entity and the votes never converge. The constraint is real; making
//! it a preconditionfor naming was not.

use std::collections::{HashMap, HashSet};

use crate::acoustid::{self, Candidate};
use crate::error::RecognizeError;
use crate::fingerprint::{self, AcoustIdFingerprint};
use crate::http::Http;
use crate::musicbrainz::{self, Release};

/// One ripped segment's PCM.
pub struct SegmentAudio<'a> {
    /// Interleaved samples.
    pub samples: &'a [i16],
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
}

/// What recognition concluded about one segment.
#[derive(Debug, Clone)]
pub struct SegmentMatch {
    /// Index into the side's segments, in side order.
    pub index: usize,
    /// Everything AcoustID offered, best first. Kept even when nothing
    /// matched the winning release, so a review UI can show the operator
    /// what was considered rather than an unexplained blank.
    pub candidates: Vec<Candidate>,
    /// What this segment will be tagged as, when it was identified.
    pub named: Option<NamedTrack>,
    /// Why there is no match, when there is none.
    pub note: Option<String>,
}

/// What one segment turned out to be.
///
/// Title and artist are the deliverable. `number` is only filled in when
/// a release lookup ran and explained this segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedTrack {
    /// Track title.
    pub title: String,
    /// Credited artist, when the source carried one.
    pub artist: Option<String>,
    /// MusicBrainz recording id this name came from.
    pub recording_mbid: String,
    /// Printed vinyl track number ("A1"), from a release lookup only.
    pub number: Option<String>,
}

/// What recognition concluded about the side.
#[derive(Debug, Clone)]
pub struct SideRecognition {
    /// The release the side voted for, with its full tracklist.
    pub release: Option<Release>,
    /// The vinyl side letter, when the matched tracks agree on one.
    pub side: Option<char>,
    /// One entry per input segment, in order.
    pub segments: Vec<SegmentMatch>,
    /// How many segments were named — the number that matters.
    pub named: usize,
    /// How many segments the winning release explained. Zero when no
    /// release lookup ran; naming does not depend on it.
    pub explained: usize,
    /// Recordings MusicBrainz could not be asked about, because it was
    /// busy or unreachable. Non-zero means the vote saw less than the
    /// whole side, so a thin answer here is not the same as "unknown
    /// record" and should not be reported as one.
    pub unresolved: usize,
}

impl SideRecognition {
    /// True when every segment was named.
    #[must_use]
    pub fn complete(&self) -> bool {
        !self.segments.is_empty() && self.named == self.segments.len()
    }
}

/// Drives a recognition pass.
pub struct Recognizer<'a> {
    http: &'a dyn Http,
    acoustid_key: String,
    max_candidates: usize,
    release_lookup: bool,
}

impl<'a> Recognizer<'a> {
    /// `acoustid_key` is a free key from acoustid.org/new-application.
    #[must_use]
    pub fn new(http: &'a dyn Http, acoustid_key: impl Into<String>) -> Self {
        Self {
            http,
            acoustid_key: acoustid_key.into(),
            // Three is enough to survive a remaster sitting above the
            // original on score, and keeps the MusicBrainz calls — one
            // per distinct recording, one per second — bounded.
            max_candidates: 3,
            // Off by default: naming needs no MusicBrainz, and a rip is
            // often one song, where a release vote over a single segment
            // buys little and costs a request a second.
            release_lookup: false,
        }
    }

    /// Also identify the release, for album / date / label / catalogue
    /// number and the vinyl track numbers.
    ///
    /// Costs one MusicBrainz request per distinct candidate recording
    /// plus one for the release, at one request a second. It can only
    /// add to the answer: naming never depends on it.
    #[must_use]
    pub fn with_release_lookup(mut self, on: bool) -> Self {
        self.release_lookup = on;
        self
    }

    /// How many AcoustID candidates per segment to carry into the vote.
    #[must_use]
    pub fn with_max_candidates(mut self, n: usize) -> Self {
        self.max_candidates = n.max(1);
        self
    }

    /// Recognise a whole side.
    ///
    /// Never fails for "nothing matched" — an unknown white label is an
    /// expected outcome and comes back as a result with no release.
    /// Errors are reserved for a missing key or a service misbehaving.
    pub fn recognize_side(
        &self,
        segments: &[SegmentAudio<'_>],
    ) -> Result<SideRecognition, RecognizeError> {
        // 1. Fingerprint and look up each segment.
        let mut per_segment: Vec<SegmentMatch> = Vec::with_capacity(segments.len());
        for (index, seg) in segments.iter().enumerate() {
            let (candidates, note) = match self.candidates_for(seg) {
                Ok(c) if c.is_empty() => (Vec::new(), Some("no AcoustID match".to_string())),
                Ok(c) => (c, None),
                // A segment too short to fingerprint must not sink the
                // side — the other tracks can still name the release.
                Err(RecognizeError::TooShort { secs, min }) => (
                    Vec::new(),
                    Some(format!("{secs} s is under the {min} s minimum")),
                ),
                Err(e) => return Err(e),
            };
            per_segment.push(SegmentMatch {
                index,
                candidates,
                named: None,
                note,
            });
        }

        // 2. Name every segment from AcoustID alone. This is the answer
        //    most rips need, and it costs no further requests.
        let consensus = artist_consensus(&per_segment);
        for seg in &mut per_segment {
            if let Some(c) = pick(&seg.candidates, consensus.as_deref()) {
                seg.named = Some(NamedTrack {
                    title: c.title.clone().unwrap_or_default(),
                    artist: c.artist.clone(),
                    recording_mbid: c.recording_mbid.clone(),
                    number: None,
                });
                seg.note = None;
            }
        }

        if !self.release_lookup {
            return Ok(SideRecognition {
                release: None,
                side: None,
                named: count_named(&per_segment),
                segments: per_segment,
                explained: 0,
                unresolved: 0,
            });
        }

        // 3. Ask once per distinct recording which releases contain it.
        let mut seen: HashSet<String> = HashSet::new();
        // release mbid -> the segments it can explain
        let mut votes: HashMap<String, HashSet<usize>> = HashMap::new();
        // recording mbid -> releases, so the tally is a pure fold after
        let mut recording_releases: HashMap<String, Vec<String>> = HashMap::new();
        //
        // One recording MusicBrainz will not answer for must not sink the
        // side — the other tracks can still name the release, the same
        // reasoning that already lets a too-short segment through. But if
        // it answered for *none* of them, that is an outage, and
        // reporting it as "no match" would send someone hunting for a
        // rare pressing when the service was simply down.
        let mut unresolved = 0;
        let mut asked = 0;
        let mut last_failure = None;
        for seg in &per_segment {
            for cand in &seg.candidates {
                if !seen.insert(cand.recording_mbid.clone()) {
                    continue;
                }
                asked += 1;
                match musicbrainz::releases_for_recording(self.http, &cand.recording_mbid) {
                    Ok(releases) => {
                        recording_releases.insert(
                            cand.recording_mbid.clone(),
                            releases.into_iter().map(|r| r.mbid).collect(),
                        );
                    }
                    Err(e) => {
                        unresolved += 1;
                        last_failure = Some(e);
                    }
                }
            }
        }
        if asked > 0 && unresolved == asked {
            return Err(last_failure.expect("a failure was recorded"));
        }
        for seg in &per_segment {
            for cand in &seg.candidates {
                for rel in recording_releases
                    .get(&cand.recording_mbid)
                    .into_iter()
                    .flatten()
                {
                    votes.entry(rel.clone()).or_default().insert(seg.index);
                }
            }
        }

        // 4. Most segments explained wins; a stable tie-break on the id
        //    keeps the answer reproducible rather than hash-order.
        let Some(winner) = votes
            .iter()
            .max_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| b.0.cmp(a.0)))
            .map(|(mbid, _)| mbid.clone())
        else {
            return Ok(SideRecognition {
                release: None,
                side: None,
                named: count_named(&per_segment),
                segments: per_segment,
                explained: 0,
                unresolved,
            });
        };

        // 5. Pull the tracklist and assign each segment its track.
        let release = musicbrainz::release(self.http, &winner)?;
        let mut explained = 0;
        // Walk each segment's candidates *best first* and take the first
        // track no earlier segment has claimed.
        //
        // Both halves matter. Scanning the release's tracklist instead
        // would hand a segment whichever of its candidates happens to sit
        // earliest on the record rather than the one AcoustID was most
        // sure of; and without the claim set, a motif that returns later
        // on the side — so two segments share a candidate — lets both
        // land on the same track and leaves the real one unnamed.
        let mut claimed: HashSet<&str> = HashSet::new();
        for seg in &mut per_segment {
            let matched = seg.candidates.iter().find_map(|cand| {
                release
                    .tracks
                    .iter()
                    .find(|t| {
                        t.recording_mbid == cand.recording_mbid
                            && !claimed.contains(t.recording_mbid.as_str())
                    })
                    .map(|t| t.recording_mbid.as_str())
            });
            if let Some(mbid) = matched {
                claimed.insert(mbid);
                let track = release
                    .tracks
                    .iter()
                    .find(|t| t.recording_mbid == mbid)
                    .expect("just found it");
                // The release is authoritative for this pressing's title
                // and is the only source of the vinyl track number.
                seg.named = Some(NamedTrack {
                    title: track.title.clone(),
                    artist: seg
                        .named
                        .as_ref()
                        .and_then(|n| n.artist.clone())
                        .or_else(|| release.artist.clone()),
                    recording_mbid: track.recording_mbid.clone(),
                    number: Some(track.number.clone()),
                });
                seg.note = None;
                explained += 1;
            } else if seg.named.is_none() && seg.note.is_none() {
                // Only a segment with no name at all needs explaining;
                // one AcoustID already named is not a failure.
                seg.note = Some("matched a recording, but not on this release".to_string());
            }
        }

        // 6. The side letter, if the numbered tracks agree on one.
        let letters: HashSet<char> = per_segment
            .iter()
            .filter_map(|s| s.named.as_ref()?.number.as_deref())
            .filter_map(|n| n.chars().next())
            .filter(char::is_ascii_alphabetic)
            .map(|c| c.to_ascii_uppercase())
            .collect();
        let side = (letters.len() == 1).then(|| *letters.iter().next().unwrap());

        Ok(SideRecognition {
            release: Some(release),
            side,
            named: count_named(&per_segment),
            segments: per_segment,
            explained,
            unresolved,
        })
    }

    fn candidates_for(&self, seg: &SegmentAudio<'_>) -> Result<Vec<Candidate>, RecognizeError> {
        let fp: AcoustIdFingerprint =
            fingerprint::fingerprint(seg.samples, seg.sample_rate, seg.channels)?;
        let mut c = acoustid::lookup(self.http, &self.acoustid_key, &fp)?;
        c.truncate(self.max_candidates);
        Ok(c)
    }
}

/// How close two AcoustID scores count as tied.
const SCORE_TIE: f32 = 0.05;

fn count_named(segments: &[SegmentMatch]) -> usize {
    segments.iter().filter(|s| s.named.is_some()).count()
}

/// The artist most of the side's best candidates agree on, if any.
///
/// Only meaningful when more than one segment agrees, so a single-song
/// rip has no consensus and falls through to plain score order.
fn artist_consensus(segments: &[SegmentMatch]) -> Option<String> {
    let mut tally: HashMap<&str, usize> = HashMap::new();
    for seg in segments {
        if let Some(a) = seg.candidates.first().and_then(|c| c.artist.as_deref()) {
            *tally.entry(a).or_default() += 1;
        }
    }
    tally
        .into_iter()
        .filter(|&(_, n)| n > 1)
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(a, _)| a.to_string())
}

/// Choose the candidate a segment is named from.
///
/// Highest score wins. Among candidates within [`SCORE_TIE`] of the
/// best, one credited to the side's consensus artist is preferred:
/// AcoustID scores the audio, not the sleeve, so different pressings of
/// one recording tie exactly and the tie has to break on something.
///
/// Its limit, measured: this only separates pressings that differ in
/// their *artist credit*. A foreign-language pressing credited to the
/// same artist — a Japanese "TSOP" still credited to MFSB — ties all the
/// way down and falls through to the deterministic last resort, which
/// may well pick the foreign title. Fixing that would need a
/// script-coherence heuristic; for now the review panel is where a human
/// corrects it, which is the same place every other tag is confirmed.
///
/// Only candidates AcoustID gave a title are eligible — a bare recording
/// id cannot name a file.
fn pick<'c>(candidates: &'c [Candidate], consensus: Option<&str>) -> Option<&'c Candidate> {
    let titled: Vec<&'c Candidate> = candidates.iter().filter(|c| c.title.is_some()).collect();
    let best = titled.iter().map(|c| c.score).fold(f32::MIN, f32::max);
    titled
        .into_iter()
        .filter(|c| c.score >= best - SCORE_TIE)
        .max_by(|a, b| {
            let agrees = |c: &Candidate| consensus.is_some() && c.artist.as_deref() == consensus;
            agrees(a)
                .cmp(&agrees(b))
                .then_with(|| a.score.total_cmp(&b.score))
                // Deterministic last resort: same input, same answer.
                .then_with(|| b.recording_mbid.cmp(&a.recording_mbid))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::StubHttp;

    /// Distinct audio per track, long enough to fingerprint.
    ///
    /// Pick pitch *classes*, not just frequencies: Chromaprint folds to
    /// chroma, so 440 Hz and 880 Hz are the same note an octave apart
    /// and fingerprint alike. A and A' is not two tracks.
    fn tone(freq: f32) -> Vec<i16> {
        let sr = 44_100;
        (0..(sr * 15) as usize)
            .map(|i| {
                let t = i as f32 / sr as f32;
                ((std::f32::consts::TAU * freq * t).sin() * 8000.0) as i16
            })
            .collect()
    }

    fn audio(v: &[i16]) -> SegmentAudio<'_> {
        SegmentAudio {
            samples: v,
            sample_rate: 44_100,
            channels: 1,
        }
    }

    /// The AcoustID answer for one track.
    fn hit(pairs: &[(&str, f32)]) -> String {
        let rows: Vec<String> = pairs
            .iter()
            .map(|(id, sc)| format!(r#"{{"score":{sc},"recordings":[{{"id":"{id}"}}]}}"#))
            .collect();
        format!(r#"{{"status":"ok","results":[{}]}}"#, rows.join(","))
    }

    /// An AcoustID answer carrying title and artist, as the real
    /// service does when asked for `meta=recordings`.
    fn hit_named(rows: &[(&str, f32, &str, &str)]) -> String {
        let rows: Vec<String> = rows
            .iter()
            .map(|(id, sc, title, artist)| {
                format!(
                    r#"{{"score":{sc},"recordings":[{{"id":"{id}","title":"{title}",
                       "artists":[{{"name":"{artist}"}}]}}]}}"#
                )
            })
            .collect();
        format!(r#"{{"status":"ok","results":[{}]}}"#, rows.join(","))
    }

    /// Key an AcoustID rule on the segment's real fingerprint, so each
    /// segment gets its own answer. Answering every lookup identically
    /// makes the release vote unable to discriminate — which is exactly
    /// the thing these tests exist to check.
    ///
    /// The *whole* fingerprint, not a prefix: the compressed form opens
    /// with an algorithm byte and a length, so two tracks of the same
    /// duration share their first base64 characters and a prefix key
    /// silently routes one segment's answer to another.
    fn fp_key(samples: &[i16]) -> String {
        let fp = fingerprint::fingerprint(samples, 44_100, 1).unwrap();
        format!("fingerprint={}", fp.encoded)
    }

    /// Three tracks, all on the LP. Track 2 also appears on a
    /// compilation — the trap a per-track "take the top hit"
    /// implementation falls into, since the compilation can score higher.
    fn stub_side(a: &[i16], b: &[i16], c: &[i16]) -> StubHttp {
        stub_side_on(StubHttp::new(), a, b, c)
    }

    /// The same side, but layered over `base` so a test can make one
    /// call fail: `StubHttp` takes the first rule that matches, so a
    /// failure rule has to be in place before these are appended.
    fn stub_side_on(base: StubHttp, a: &[i16], b: &[i16], c: &[i16]) -> StubHttp {
        base.on(&fp_key(a), &hit(&[("rec-a", 0.99)]))
            .on(&fp_key(b), &hit(&[("rec-b-comp", 0.98), ("rec-b", 0.93)]))
            // Segment C also half-matches track A — a real thing on a
            // record where a motif returns — so rec-a is asked for by two
            // segments and must still be looked up once.
            .on(&fp_key(c), &hit(&[("rec-c", 0.97), ("rec-a", 0.55)]))
            .on(
                "/recording/rec-a",
                r#"{"id":"rec-a","releases":[{"id":"rel-lp","title":"There Is"}]}"#,
            )
            .on(
                "/recording/rec-b-comp",
                r#"{"id":"rec-b-comp","releases":[{"id":"rel-comp","title":"Greatest Hits"}]}"#,
            )
            .on(
                "/recording/rec-b",
                r#"{"id":"rec-b","releases":[
                    {"id":"rel-lp","title":"There Is"},
                    {"id":"rel-comp","title":"Greatest Hits"}]}"#,
            )
            .on(
                "/recording/rec-c",
                r#"{"id":"rec-c","releases":[{"id":"rel-lp","title":"There Is"}]}"#,
            )
            .on(
                "/release/rel-lp",
                r#"{"id":"rel-lp","title":"There Is","date":"1968",
                    "artist-credit":[{"name":"The Dells","joinphrase":""}],
                    "label-info":[{"catalog-number":"LPS-804","label":{"name":"Cadet"}}],
                    "media":[{"tracks":[
                      {"number":"A1","title":"Stay In My Corner","recording":{"id":"rec-a"}},
                      {"number":"A2","title":"Wear It On Our Face","recording":{"id":"rec-b"}},
                      {"number":"A3","title":"Run For Cover","recording":{"id":"rec-c"}}]}]}"#,
            )
    }

    /// A busy MusicBrainz on one recording must not throw away the
    /// AcoustID lookups already paid for on the rest of the side.
    #[test]
    fn one_busy_musicbrainz_call_does_not_sink_the_side() {
        let (a, b, c) = (tone(440.0), tone(523.25), tone(659.25));
        let http = stub_side_on(
            StubHttp::new().failing("/recording/rec-c", 503, "busy"),
            &a,
            &b,
            &c,
        );
        let got = Recognizer::new(&http, "key")
            .with_release_lookup(true)
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap();

        assert_eq!(
            got.release.as_ref().expect("the LP still wins").mbid,
            "rel-lp"
        );
        // All three are still named, not two. The failed call cost
        // rec-c its *vote*, but the release that won carries rec-c on
        // its tracklist, so assignment finds it anyway. Losing a vote
        // only matters when it would have changed the winner.
        assert_eq!(got.explained, 3);
        assert_eq!(got.unresolved, 1, "the side must report what it missed");
    }

    /// The other half of that: if MusicBrainz answered for nothing, the
    /// side is unknown *to us*, not unknown to the database.
    #[test]
    fn a_total_musicbrainz_outage_is_an_error_not_an_unknown_record() {
        let (a, b, c) = (tone(440.0), tone(523.25), tone(659.25));
        let http = stub_side_on(
            StubHttp::new().failing("/recording/", 503, "busy"),
            &a,
            &b,
            &c,
        );
        let err = Recognizer::new(&http, "key")
            .with_release_lookup(true)
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap_err();
        assert!(format!("{err}").contains("503"), "{err}");
    }

    #[test]
    fn the_side_votes_for_the_release_that_explains_the_most_tracks() {
        let (a, b, c) = (tone(440.0), tone(523.25), tone(659.25));
        let http = stub_side(&a, &b, &c);
        let got = Recognizer::new(&http, "key")
            .with_release_lookup(true)
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap();

        let rel = got.release.as_ref().expect("a release should have won");
        assert_eq!(
            rel.mbid, "rel-lp",
            "the LP explains all three; the compilation explains one"
        );
        assert_eq!(got.explained, 3);
        assert!(got.complete());
        assert_eq!(rel.label.as_deref(), Some("Cadet"));
        assert_eq!(rel.catalog_number.as_deref(), Some("LPS-804"));
    }

    #[test]
    fn every_segment_gets_its_track_in_side_order() {
        let (a, b, c) = (tone(440.0), tone(523.25), tone(659.25));
        let http = stub_side(&a, &b, &c);
        let got = Recognizer::new(&http, "key")
            .with_release_lookup(true)
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap();
        let titles: Vec<&str> = got
            .segments
            .iter()
            .map(|s| s.named.as_ref().map_or("-", |t| t.title.as_str()))
            .collect();
        assert_eq!(
            titles,
            vec!["Stay In My Corner", "Wear It On Our Face", "Run For Cover"]
        );
        assert_eq!(got.side, Some('A'), "all three are A-side tracks");
    }

    /// One MusicBrainz call per *distinct* recording, not per segment
    /// per candidate — the service allows one request a second.
    #[test]
    fn recording_lookups_are_deduplicated_across_the_side() {
        let (a, b, c) = (tone(440.0), tone(523.25), tone(659.25));
        let http = stub_side(&a, &b, &c);
        Recognizer::new(&http, "key")
            .with_release_lookup(true)
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap();

        let recording_calls = http
            .requests()
            .iter()
            .filter(|r| r.contains("/recording/"))
            .count();
        // Candidate slots across the side: 1 + 2 + 2 = 5, over four
        // distinct recordings (rec-a is offered by two segments).
        // MusicBrainz allows one request a second, so the difference is
        // wall-clock the operator waits.
        assert_eq!(
            recording_calls, 4,
            "five candidate slots must collapse to four recording lookups"
        );
    }

    /// A returning motif means two segments can offer the same
    /// recording. Each track belongs to one segment.
    #[test]
    fn two_segments_cannot_claim_the_same_track() {
        let (a, b, c) = (tone(440.0), tone(523.25), tone(659.25));
        let http = stub_side(&a, &b, &c);
        let got = Recognizer::new(&http, "key")
            .with_release_lookup(true)
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap();

        let claimed: Vec<&str> = got
            .segments
            .iter()
            .filter_map(|s| s.named.as_ref().map(|t| t.recording_mbid.as_str()))
            .collect();
        let distinct: std::collections::HashSet<&&str> = claimed.iter().collect();
        assert_eq!(
            claimed.len(),
            distinct.len(),
            "a track was assigned to more than one segment: {claimed:?}"
        );
        // Segment C offers rec-a at 0.55 and rec-c at 0.97. It must take
        // rec-c: its own best candidate, and the one A has not claimed.
        assert_eq!(
            got.segments[2].named.as_ref().unwrap().recording_mbid,
            "rec-c"
        );
    }

    #[test]
    fn an_unknown_pressing_returns_no_release_rather_than_an_error() {
        let a = tone(440.0);
        let http = StubHttp::new().on("acoustid.org", r#"{"status":"ok","results":[]}"#);
        let got = Recognizer::new(&http, "key")
            .with_release_lookup(true)
            .recognize_side(&[audio(&a)])
            .unwrap();
        assert!(got.release.is_none());
        assert_eq!(got.explained, 0);
        assert!(!got.complete());
        assert_eq!(
            got.segments[0].note.as_deref(),
            Some("no AcoustID match"),
            "the operator should be told why, not shown a blank"
        );
    }

    /// A short lead-in snippet must not sink the rest of the side.
    #[test]
    fn a_segment_too_short_to_fingerprint_does_not_fail_the_side() {
        let short = tone(440.0)[..44_100 * 3].to_vec();
        let b = tone(523.25);
        let c = tone(659.25);
        let http = stub_side(&tone(440.0), &b, &c);
        let got = Recognizer::new(&http, "key")
            .with_release_lookup(true)
            .recognize_side(&[audio(&short), audio(&b), audio(&c)])
            .unwrap();
        assert!(
            got.release.is_some(),
            "the other two still name the release"
        );
        assert!(got.segments[0].named.is_none());
        assert!(
            got.segments[0].note.as_deref().unwrap().contains("minimum"),
            "note was {:?}",
            got.segments[0].note
        );
    }

    /// The common case the design exists for: someone lifted one song
    /// off a record. There is no side to vote with, and MusicBrainz must
    /// not be asked at all.
    #[test]
    fn a_single_song_is_named_without_asking_musicbrainz() {
        let a = tone(440.0);
        let http = StubHttp::new().on(
            &fp_key(&a),
            &hit_named(&[("rec-a", 0.96, "Love Train", "The O Jays")]),
        );
        let got = Recognizer::new(&http, "key")
            .recognize_side(&[audio(&a)])
            .unwrap();

        let named = got.segments[0].named.as_ref().expect("should be named");
        assert_eq!(named.title, "Love Train");
        assert_eq!(named.artist.as_deref(), Some("The O Jays"));
        assert_eq!(named.number, None, "no release was asked for");
        assert_eq!(got.named, 1);
        assert!(got.complete());
        assert!(got.release.is_none());
        assert!(
            http.requests().iter().all(|r| r.contains("acoustid")),
            "naming must not touch MusicBrainz: {:?}",
            http.requests()
        );
    }

    /// The O'Jays sampler, reduced: three tracks whose recordings each
    /// live on a different pressing, so no release can explain the side.
    /// Every track must still be named. This is the exact failure that
    /// made the vote-gated design wrong — it named 2 of 10 while
    /// AcoustID had identified 6.
    #[test]
    fn tracks_are_named_even_when_no_release_explains_the_side() {
        let (a, b, c) = (tone(440.0), tone(523.25), tone(659.25));
        let http = StubHttp::new()
            .on(&fp_key(&a), &hit_named(&[("rec-a", 0.93, "TSOP", "MFSB")]))
            .on(
                &fp_key(&b),
                &hit_named(&[("rec-b", 0.91, "Love Train", "The O Jays")]),
            )
            .on(
                &fp_key(&c),
                &hit_named(&[("rec-c", 0.88, "Back Stabbers", "The O Jays")]),
            )
            .on(
                "/recording/rec-a",
                r#"{"id":"rec-a","releases":[{"id":"rel-1","title":"One"}]}"#,
            )
            .on(
                "/recording/rec-b",
                r#"{"id":"rec-b","releases":[{"id":"rel-2","title":"Two"}]}"#,
            )
            .on(
                "/recording/rec-c",
                r#"{"id":"rec-c","releases":[{"id":"rel-3","title":"Three"}]}"#,
            )
            .on(
                "/release/rel-1",
                r#"{"id":"rel-1","title":"One","media":[{"tracks":[
                    {"number":"A1","title":"TSOP","recording":{"id":"rec-a"}}]}]}"#,
            );

        let got = Recognizer::new(&http, "key")
            .with_release_lookup(true)
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap();

        assert_eq!(got.named, 3, "every track must be named");
        assert_eq!(got.explained, 1, "only one is on the winning release");
        let titles: Vec<&str> = got
            .segments
            .iter()
            .map(|s| s.named.as_ref().map_or("-", |t| t.title.as_str()))
            .collect();
        assert_eq!(titles, vec!["TSOP", "Love Train", "Back Stabbers"]);
        assert!(
            got.segments[1].note.is_none(),
            "a named track is not a failure to explain"
        );
    }

    /// AcoustID scores the audio, not the sleeve, so two pressings of
    /// one recording tie exactly. When they carry *different artist
    /// credits*, the side's consensus artist breaks it.
    ///
    /// Note the boundary: had both pressings been credited to the same
    /// artist, this mechanism could not help — and on the real O'Jays
    /// sampler that is exactly what happens to "TSOP".
    #[test]
    fn naming_prefers_the_side_artist_when_scores_tie() {
        let (a, b, c) = (tone(440.0), tone(523.25), tone(659.25));
        let http = StubHttp::new()
            .on(
                &fp_key(&a),
                &hit_named(&[
                    ("rec-jp", 0.93, "SOUL TRAIN NO TEEMA", "Nihon Ban"),
                    ("rec-us", 0.93, "TSOP", "The O Jays"),
                ]),
            )
            .on(
                &fp_key(&b),
                &hit_named(&[("rec-b", 0.91, "Love Train", "The O Jays")]),
            )
            .on(
                &fp_key(&c),
                &hit_named(&[("rec-c", 0.88, "Back Stabbers", "The O Jays")]),
            );

        let got = Recognizer::new(&http, "key")
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap();
        assert_eq!(
            got.segments[0].named.as_ref().unwrap().title,
            "TSOP",
            "the side is credited to The O Jays, so break the tie that way"
        );
    }

    #[test]
    fn a_missing_api_key_is_an_error_not_a_silent_empty_result() {
        let a = tone(440.0);
        let http = StubHttp::new();
        let err = Recognizer::new(&http, "")
            .recognize_side(&[audio(&a)])
            .unwrap_err();
        assert!(
            matches!(err, RecognizeError::MissingCredential { .. }),
            "{err}"
        );
    }
}
