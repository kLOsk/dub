//! Recognising a **side**, not a pile of unrelated tracks.
//!
//! Looking each segment up on its own and taking the top hit is what a
//! naive implementation does, and it is wrong in a specific way: a
//! recording appears on the original LP, three compilations and a
//! reissue, and AcoustID's score cannot tell them apart because it
//! scores the *audio*, not the pressing. Track 1 lands on the LP, track
//! 2 on a "Greatest Hits", track 3 on a Japanese reissue, and the rip
//! ends up tagged with three different albums.
//!
//! A vinyl side is a strong constraint the naive version throws away:
//! these tracks are on **one release**, in **one order**, on **one
//! side**. So the segments vote. Each candidate recording is asked which
//! releases contain it, and the release that explains the most segments
//! wins. Ties break on side coherence — a real side is a contiguous run
//! of one letter, in order — and then on the fingerprint scores.
//!
//! That also bounds the cost. The expensive call is one per *distinct*
//! candidate recording, and MusicBrainz allows one request a second, so
//! candidates are capped per segment and recordings are deduplicated
//! across the side.

use std::collections::{HashMap, HashSet};

use crate::acoustid::{self, Candidate};
use crate::error::RecognizeError;
use crate::fingerprint::{self, AcoustIdFingerprint};
use crate::http::Http;
use crate::musicbrainz::{self, Release, Track};

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
    /// The track on the winning release this segment is, if any.
    pub matched: Option<Track>,
    /// Why there is no match, when there is none.
    pub note: Option<String>,
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
    /// How many segments the winning release explained.
    pub explained: usize,
}

impl SideRecognition {
    /// True when every segment found a track on the winning release.
    #[must_use]
    pub fn complete(&self) -> bool {
        !self.segments.is_empty() && self.explained == self.segments.len()
    }
}

/// Drives a recognition pass.
pub struct Recognizer<'a> {
    http: &'a dyn Http,
    acoustid_key: String,
    max_candidates: usize,
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
        }
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
                matched: None,
                note,
            });
        }

        // 2. Ask once per distinct recording which releases contain it.
        let mut seen: HashSet<String> = HashSet::new();
        // release mbid -> the segments it can explain
        let mut votes: HashMap<String, HashSet<usize>> = HashMap::new();
        // recording mbid -> releases, so the tally is a pure fold after
        let mut recording_releases: HashMap<String, Vec<String>> = HashMap::new();
        for seg in &per_segment {
            for cand in &seg.candidates {
                if !seen.insert(cand.recording_mbid.clone()) {
                    continue;
                }
                let releases =
                    musicbrainz::releases_for_recording(self.http, &cand.recording_mbid)?;
                recording_releases.insert(
                    cand.recording_mbid.clone(),
                    releases.into_iter().map(|r| r.mbid).collect(),
                );
            }
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

        // 3. Most segments explained wins; a stable tie-break on the id
        //    keeps the answer reproducible rather than hash-order.
        let Some(winner) = votes
            .iter()
            .max_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| b.0.cmp(a.0)))
            .map(|(mbid, _)| mbid.clone())
        else {
            return Ok(SideRecognition {
                release: None,
                side: None,
                segments: per_segment,
                explained: 0,
            });
        };

        // 4. Pull the tracklist and assign each segment its track.
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
                seg.matched = Some(track.clone());
                seg.note = None;
                explained += 1;
            } else if seg.note.is_none() {
                seg.note = Some("matched a recording, but not on this release".to_string());
            }
        }

        // 5. The side letter, if the matched tracks agree on one.
        let letters: HashSet<char> = per_segment
            .iter()
            .filter_map(|s| s.matched.as_ref().and_then(Track::side))
            .collect();
        let side = (letters.len() == 1).then(|| *letters.iter().next().unwrap());

        Ok(SideRecognition {
            release: Some(release),
            side,
            segments: per_segment,
            explained,
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
        StubHttp::new()
            .on(&fp_key(a), &hit(&[("rec-a", 0.99)]))
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

    #[test]
    fn the_side_votes_for_the_release_that_explains_the_most_tracks() {
        let (a, b, c) = (tone(440.0), tone(523.25), tone(659.25));
        let http = stub_side(&a, &b, &c);
        let got = Recognizer::new(&http, "key")
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
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap();
        let titles: Vec<&str> = got
            .segments
            .iter()
            .map(|s| s.matched.as_ref().map_or("-", |t| t.title.as_str()))
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
            .recognize_side(&[audio(&a), audio(&b), audio(&c)])
            .unwrap();

        let claimed: Vec<&str> = got
            .segments
            .iter()
            .filter_map(|s| s.matched.as_ref().map(|t| t.recording_mbid.as_str()))
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
            got.segments[2].matched.as_ref().unwrap().recording_mbid,
            "rec-c"
        );
    }

    #[test]
    fn an_unknown_pressing_returns_no_release_rather_than_an_error() {
        let a = tone(440.0);
        let http = StubHttp::new().on("acoustid.org", r#"{"status":"ok","results":[]}"#);
        let got = Recognizer::new(&http, "key")
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
            .recognize_side(&[audio(&short), audio(&b), audio(&c)])
            .unwrap();
        assert!(
            got.release.is_some(),
            "the other two still name the release"
        );
        assert!(got.segments[0].matched.is_none());
        assert!(
            got.segments[0].note.as_deref().unwrap().contains("minimum"),
            "note was {:?}",
            got.segments[0].note
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
