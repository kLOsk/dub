//! The DJ's tempo range.
//!
//! A beat can be counted at half or double speed, and the audio does not
//! say which. Dub reads every tempo inside one octave
//! ([`dub_bpm::TempoOctave`]): the genre's where the tag names one, and
//! this setting for every other song — the same job the BPM-range
//! preference does in Traktor, Serato and rekordbox.
//!
//! It lives in the library database (`library_settings`) beside the app
//! order, so the app and the `dub` CLI analyse alike. Changing it leaves
//! stored grids alone; it applies to songs analysed afterwards.

use dub_bpm::TempoOctave;

use crate::{Library, LibraryError, Result};

const KEY: &str = "tempo_range_low";

impl Library {
    /// The octave a song with no recognised genre is analysed in.
    /// [`TempoOctave::DEFAULT`] until the DJ picks another, or if what is
    /// stored is unreadable.
    pub fn tempo_octave(&self) -> Result<TempoOctave> {
        Ok(self
            .setting(KEY)?
            .and_then(|s| s.parse::<f64>().ok())
            .and_then(TempoOctave::from_low)
            .unwrap_or_default())
    }

    /// Set the octave to `[low, 2 × low)` BPM. Refused outside what the
    /// analyser can read ([`TempoOctave::from_low`]).
    pub fn set_tempo_octave(&self, low: f64) -> Result<()> {
        let Some(octave) = TempoOctave::from_low(low) else {
            return Err(LibraryError::InvalidArgument(format!(
                "a tempo range starts between {} and {} BPM; got {low}",
                TempoOctave::MIN_LOW,
                TempoOctave::MAX_LOW
            )));
        };
        self.store_setting(KEY, &octave.low().to_string())
    }

    /// The octave a song of this genre is analysed in: its genre family's,
    /// or the DJ's range when the genre is missing or says nothing about
    /// tempo.
    pub(crate) fn tempo_octave_for_genre(&self, genre: Option<&str>) -> Result<TempoOctave> {
        Ok(genre
            .map(dub_bpm::octave_profile_from_genre)
            .unwrap_or_default()
            .tempo_octave(self.tempo_octave()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_is_78_to_156_until_the_dj_picks_another() {
        let lib = Library::open_in_memory().unwrap();
        assert_eq!(lib.tempo_octave().unwrap(), TempoOctave::DEFAULT);
        assert_eq!(lib.tempo_octave().unwrap().low(), 78.0);

        lib.set_tempo_octave(100.0).unwrap();
        assert_eq!(lib.tempo_octave().unwrap().low(), 100.0);
        lib.set_tempo_octave(60.0).unwrap();
        assert_eq!(lib.tempo_octave().unwrap().low(), 60.0);
    }

    #[test]
    fn a_range_the_analyser_cannot_read_is_refused_and_changes_nothing() {
        let lib = Library::open_in_memory().unwrap();
        lib.set_tempo_octave(88.0).unwrap();
        for low in [0.0, 59.0, 101.0, f64::NAN, f64::INFINITY] {
            assert!(
                matches!(
                    lib.set_tempo_octave(low),
                    Err(LibraryError::InvalidArgument(_))
                ),
                "{low}"
            );
        }
        assert_eq!(lib.tempo_octave().unwrap().low(), 88.0);
    }

    #[test]
    fn the_genre_picks_the_octave_and_the_range_covers_the_rest() {
        let lib = Library::open_in_memory().unwrap();
        lib.set_tempo_octave(88.0).unwrap();
        let low = |genre: Option<&str>| lib.tempo_octave_for_genre(genre).unwrap().low();
        assert_eq!(low(None), 88.0);
        assert_eq!(low(Some("Bmore - Funk - Friendly")), 88.0);
        assert_eq!(low(Some("Drum & Bass")), 100.0);
        assert_eq!(low(Some("Hip-Hop - Mid Age")), 68.0);
        assert_eq!(low(Some("Reggae - Roots")), 60.0);
        assert_eq!(low(Some("Dubstep - Bretter")), 88.0);
        lib.set_tempo_octave(60.0).unwrap();
        assert_eq!(
            low(Some("Dubstep - Bretter")),
            88.0,
            "the genre still decides"
        );
        assert_eq!(low(None), 60.0);
    }
}
