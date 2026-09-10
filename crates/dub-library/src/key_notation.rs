//! Musical-key notations → canonical Camelot.
//!
//! Every source that carries a key writes it its own way, and a file's
//! own tag is the least predictable of all: an ID3 `TKEY` or a Vorbis
//! `INITIALKEY` holds whatever the last tagger chose. Mixed In Key
//! writes Camelot (`8A`), Traktor writes Open Key (`1m`) or musical
//! (`Am`), rekordbox and Serato write musical (`Abm`, `F#`), and a
//! hand-tagged file may spell it out (`A minor`). `track_keys.key_notation`
//! is Camelot by contract — the browser column, the ⚠ cross-check and
//! the deck header all assume it — so anything else is converted on
//! the way in, and anything unrecognised is not written as if it were.

use dub_spectral::{camelot_notation, parse_camelot};

/// Convert a key in any of the three notations DJ software writes —
/// Camelot (`8A`), Open Key (`1m` / `1d`) or musical (`Am`, `F#m`,
/// `Ebm`, `Db`, `A minor`, `F# major`) — to canonical Camelot.
/// Enharmonic spellings collapse (`Ebm` == `D#m`). `None` for
/// anything unrecognised, including ID3's `o` "off key" marker.
#[must_use]
pub fn to_camelot(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Some((number, is_major)) = parse_camelot(s) {
        return Some(format!("{number}{}", if is_major { 'B' } else { 'A' }));
    }
    if let Some(camelot) = open_key_to_camelot(s) {
        return Some(camelot);
    }
    let (pc, is_major) = musical_key(s)?;
    Some(camelot_notation(pc, is_major).to_string())
}

/// Open Key (Traktor's wheel) is Camelot rotated by seven positions:
/// `1d` is C major where Camelot has `8B`. `d` (dur) is major and `m`
/// (moll) minor.
fn open_key_to_camelot(s: &str) -> Option<String> {
    let last = s.chars().last()?;
    let is_major = match last.to_ascii_lowercase() {
        'd' => true,
        'm' => false,
        _ => return None,
    };
    let number: u8 = s[..s.len() - last.len_utf8()].parse().ok()?;
    if !(1..=12).contains(&number) {
        return None;
    }
    let camelot = (number + 6) % 12 + 1;
    Some(format!("{camelot}{}", if is_major { 'B' } else { 'A' }))
}

/// A note-name root with an optional accidental, then a mode: nothing
/// or `maj` / `major` for major, `m` / `min` / `minor` for minor, with
/// or without a space. Returns `(pitch class, is_major)`.
fn musical_key(s: &str) -> Option<(u8, bool)> {
    let mut chars = s.chars();
    let base: i32 = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (accidental, rest) = match rest.chars().next() {
        Some(c @ ('#' | '♯')) => (1, &rest[c.len_utf8()..]),
        Some(c @ ('b' | '♭')) => (-1, &rest[c.len_utf8()..]),
        _ => (0, rest),
    };
    let is_major = match rest.trim().to_ascii_lowercase().as_str() {
        "" | "maj" | "major" => true,
        "m" | "min" | "minor" => false,
        _ => return None,
    };
    // `rem_euclid` folds `Cb` onto B and `B#` onto C.
    let pc = u8::try_from((base + accidental).rem_euclid(12)).ok()?;
    Some((pc, is_major))
}

#[cfg(test)]
mod tests {
    use super::to_camelot;

    fn camelot(raw: &str) -> Option<String> {
        to_camelot(raw)
    }

    #[test]
    fn camelot_passes_through_normalised() {
        assert_eq!(camelot("8A").as_deref(), Some("8A"));
        assert_eq!(camelot("12b").as_deref(), Some("12B"));
        assert_eq!(camelot(" 1A ").as_deref(), Some("1A"));
        assert_eq!(camelot("13A"), None);
        assert_eq!(camelot("0B"), None);
    }

    #[test]
    fn open_key_rotates_onto_camelot() {
        assert_eq!(camelot("1d").as_deref(), Some("8B")); // C major
        assert_eq!(camelot("1m").as_deref(), Some("8A")); // A minor
        assert_eq!(camelot("6d").as_deref(), Some("1B")); // B major
        assert_eq!(camelot("7m").as_deref(), Some("2A")); // Eb minor
        assert_eq!(camelot("12D").as_deref(), Some("7B")); // F major
        assert_eq!(camelot("13m"), None);
    }

    /// The Serato converter's own cases, which this replaces.
    #[test]
    fn musical_minor_and_major() {
        assert_eq!(camelot("Em").as_deref(), Some("9A"));
        assert_eq!(camelot("Bm").as_deref(), Some("10A"));
        assert_eq!(camelot("Ebm").as_deref(), Some("2A")); // D#m
        assert_eq!(camelot("Am").as_deref(), Some("8A"));
        assert_eq!(camelot("C").as_deref(), Some("8B"));
        assert_eq!(camelot("F#").as_deref(), Some("2B"));
        assert_eq!(camelot("Db").as_deref(), Some("3B")); // C#
        assert_eq!(camelot("xyz"), None);
    }

    #[test]
    fn musical_spelled_out_modes_and_unicode_accidentals() {
        assert_eq!(camelot("A minor").as_deref(), Some("8A"));
        assert_eq!(camelot("F# major").as_deref(), Some("2B"));
        assert_eq!(camelot("Ebmaj").as_deref(), Some("5B"));
        assert_eq!(camelot("Eb min").as_deref(), Some("2A"));
        assert_eq!(camelot("C♯m").as_deref(), Some("12A"));
        assert_eq!(camelot("D♭").as_deref(), Some("3B"));
        assert_eq!(camelot("am").as_deref(), Some("8A"));
    }

    #[test]
    fn enharmonics_collapse_and_wrap() {
        assert_eq!(camelot("Cb"), camelot("B"));
        assert_eq!(camelot("B#"), camelot("C"));
        assert_eq!(camelot("A#m"), camelot("Bbm"));
    }

    /// A tagger's "no key" markers and junk must not reach
    /// `track_keys` dressed as Camelot.
    #[test]
    fn rejects_what_is_not_a_key() {
        for junk in ["", "  ", "o", "H", "Cx", "8", "A/B", "minor", "Am7"] {
            assert_eq!(camelot(junk), None, "{junk:?}");
        }
    }
}
