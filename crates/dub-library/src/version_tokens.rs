//! Version-token parser per PRD §8.1 / §8.4.
//!
//! Hip-hop, reggae, dnb, and dubstep libraries characteristically
//! contain multiple distinct files of the same recording marked with
//! version tags: `(Clean)`, `(Dirty)`, `(Instrumental)`, `(Acapella)`,
//! `(Radio Edit)`, `(Extended Mix)`, `(12" Mix)`. Auto-merging these
//! is the single most expensive dedupe mistake we could make — the
//! cost of silently collapsing "Clean" and "Dirty" is "the DJ played
//! the explicit version at a wedding."
//!
//! This module parses the version tokens out of filenames and ID3
//! titles so the M11b dedupe pipeline can refuse to merge two files
//! that carry distinct version tokens.
//!
//! # Recognition rules
//!
//! Tokens are recognised in three contexts only, in priority order:
//!
//! 1. **Parenthesised segment** at the end of the title: `... (Clean)`,
//!    `... (12" Mix)`, `... (Radio Edit)`.
//! 2. **Square-bracketed segment** at the end of the title:
//!    `... [Dirty]`, `... [Acapella]`.
//! 3. **Trailing ` - TOKEN` segment** before the file extension:
//!    `Song Title - Instrumental.mp3`.
//!
//! Recognition is **case-insensitive but word-boundary-strict**: an
//! artist named "Clean Bandit" never matches, because their name is
//! never inside parentheses, brackets, or a trailing `- TOKEN`
//! segment in any realistic filename. The price for this strictness
//! is that exotic naming schemes ("Song.Clean.mp3") don't get
//! tagged; in PRD §8.1 terms, those land in the "no version token
//! found; rely on fingerprint + duration" path, which is correct
//! (no false-positive merge, just no token to disqualify either).
//!
//! # Token vocabulary
//!
//! The full v1 token list per PRD §8.1 §8.4 (lowercase canonical form):
//!
//! ```text
//! clean, dirty, explicit, instrumental, acapella, radio, edit,
//! extended, club, dub, vip, remix, remaster, mono, stereo, intro,
//! outro, short, long, 7in, 12in, lp
//! ```
//!
//! Multi-word tokens (`radio edit`, `extended mix`, `club mix`,
//! `clean version`) are also recognised; they normalise to their
//! head token (`radio`, `extended`, `club`, `clean`).
//!
//! The dedupe pipeline cares about **token identity, not token
//! count** — two files differ in version if and only if their
//! parsed token sets are not equal.

use std::collections::BTreeSet;

/// Canonical version token recognised in a filename or title.
///
/// Ordering is `Ord`-comparable (alphabetical) so dedupe logic can
/// store tokens in a `BTreeSet` and compare sets cheaply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum VersionToken {
    /// Radio-safe / lyrics-censored.
    Clean,
    /// Original explicit lyrics.
    Dirty,
    /// Synonym of `Dirty` retained as a separate variant so the
    /// parser can preserve what the user typed; dedupe treats
    /// `Dirty` and `Explicit` as distinct (some labels ship both,
    /// with the "Explicit" cut being subtly different from the
    /// LP cut).
    Explicit,
    /// Vocals stripped.
    Instrumental,
    /// Vocals only.
    Acapella,
    /// Radio edit (typically a shortened, structurally compressed
    /// mix targeting ~3:30 broadcast length).
    Radio,
    /// Generic "edit" tag — distinct from `Radio` because not every
    /// edit is a radio edit (e.g. "Skream Edit").
    Edit,
    /// Extended mix (12" cut, club length).
    Extended,
    /// Club mix.
    Club,
    /// Dub / dub mix.
    Dub,
    /// VIP mix.
    Vip,
    /// Generic "remix" tag.
    Remix,
    /// Remastered cut.
    Remaster,
    /// Mono mixdown (vs `Stereo`).
    Mono,
    /// Stereo mixdown (vs `Mono`).
    Stereo,
    /// Intro / DJ-friendly extended intro cut.
    Intro,
    /// Outro / extended outro cut.
    Outro,
    /// Short version.
    Short,
    /// Long / full-length version.
    Long,
    /// 7" / 7-inch single cut.
    SevenInch,
    /// 12" / 12-inch single cut.
    TwelveInch,
    /// LP / album cut.
    Lp,
}

impl VersionToken {
    /// Canonical lowercase string form. Stored in
    /// `track_metadata_source.version_token` per the LIBRARY-SCHEMA
    /// enum comment.
    pub fn as_str(&self) -> &'static str {
        match self {
            VersionToken::Clean => "clean",
            VersionToken::Dirty => "dirty",
            VersionToken::Explicit => "explicit",
            VersionToken::Instrumental => "instrumental",
            VersionToken::Acapella => "acapella",
            VersionToken::Radio => "radio",
            VersionToken::Edit => "edit",
            VersionToken::Extended => "extended",
            VersionToken::Club => "club",
            VersionToken::Dub => "dub",
            VersionToken::Vip => "vip",
            VersionToken::Remix => "remix",
            VersionToken::Remaster => "remaster",
            VersionToken::Mono => "mono",
            VersionToken::Stereo => "stereo",
            VersionToken::Intro => "intro",
            VersionToken::Outro => "outro",
            VersionToken::Short => "short",
            VersionToken::Long => "long",
            VersionToken::SevenInch => "7in",
            VersionToken::TwelveInch => "12in",
            VersionToken::Lp => "lp",
        }
    }
}

/// Parse the version tokens from a filename or title string.
///
/// Returns a sorted, deduplicated set. Empty when nothing is
/// recognised — which is the **expected** case for the majority of
/// files; absence of a token is not a parse error, just an absence
/// of a dedupe-disqualifier.
///
/// The input may be a bare title (`"Lady (Clean)"`), a full
/// filename (`"01 Lady (Clean).mp3"`), or the title field from any
/// metadata source.
pub fn parse(input: &str) -> BTreeSet<VersionToken> {
    let mut found = BTreeSet::new();
    // Strip the file extension if present. We don't want `.mp3` to
    // be mistaken for part of the title.
    let trimmed = strip_extension(input);

    // Scan every parenthesised and square-bracketed segment.
    for segment in bracketed_segments(trimmed) {
        for tok in scan_segment(segment) {
            found.insert(tok);
        }
    }

    // Scan the trailing `- TOKEN` segment, if any.
    if let Some(tail) = trailing_dash_segment(trimmed) {
        for tok in scan_segment(tail) {
            found.insert(tok);
        }
    }

    found
}

/// Strip a single trailing `.ext` extension. We only strip a *short*
/// extension (≤5 chars) so a title like "Mary J." doesn't lose its
/// trailing period.
fn strip_extension(input: &str) -> &str {
    if let Some(dot) = input.rfind('.') {
        let ext = &input[dot + 1..];
        if !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            return &input[..dot];
        }
    }
    input
}

/// Iterate over every parenthesised and square-bracketed segment
/// inside `input`, returning the inner content (without the
/// brackets). Nested brackets are not supported; in practice DJ
/// filenames don't nest.
fn bracketed_segments(input: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut bytes = input.as_bytes();
    let mut offset = 0;
    while !bytes.is_empty() {
        match find_segment(bytes) {
            Some((start, end)) => {
                // SAFETY-free: indices are byte offsets within
                // ASCII bracket characters, so they land on UTF-8
                // boundaries.
                let inner_start = offset + start + 1;
                let inner_end = offset + end;
                segments.push(&input[inner_start..inner_end]);
                offset += end + 1;
                bytes = &bytes[end + 1..];
            }
            None => break,
        }
    }
    segments
}

/// Return the byte offsets of the next matching bracket pair in
/// `bytes`. Returns `(open_pos, close_pos)`. `None` if no pair is
/// found.
fn find_segment(bytes: &[u8]) -> Option<(usize, usize)> {
    let open_pos = bytes.iter().position(|&b| matches!(b, b'(' | b'['))?;
    let opener = bytes[open_pos];
    let closer = if opener == b'(' { b')' } else { b']' };
    let close_pos = bytes[open_pos + 1..]
        .iter()
        .position(|&b| b == closer)
        .map(|p| p + open_pos + 1)?;
    Some((open_pos, close_pos))
}

/// Return the substring after the last ` - ` separator in `input`,
/// trimmed of surrounding whitespace. The separator must be a
/// space-dash-space pattern to avoid grabbing hyphens inside artist
/// names ("Sault - 7" doesn't match; "Title - Instrumental" does).
fn trailing_dash_segment(input: &str) -> Option<&str> {
    let idx = input.rfind(" - ")?;
    let after = input[idx + 3..].trim();
    if after.is_empty() {
        None
    } else {
        Some(after)
    }
}

/// Scan a single segment for tokens. Multi-word tokens
/// (`radio edit`, `extended mix`, `clean version`) are recognised
/// before single-word fallbacks so we don't double-count.
fn scan_segment(segment: &str) -> Vec<VersionToken> {
    let lower = segment.to_ascii_lowercase();
    let mut out = Vec::new();

    // Phrase-level recognition. Multi-word forms map to their head
    // token; we still consume the whole phrase so single-word
    // recognition below doesn't double-count.
    let phrases: &[(&str, VersionToken)] = &[
        ("radio edit", VersionToken::Radio),
        ("radio mix", VersionToken::Radio),
        ("radio version", VersionToken::Radio),
        ("extended mix", VersionToken::Extended),
        ("extended version", VersionToken::Extended),
        ("club mix", VersionToken::Club),
        ("dub mix", VersionToken::Dub),
        ("clean version", VersionToken::Clean),
        ("dirty version", VersionToken::Dirty),
        ("explicit version", VersionToken::Explicit),
        ("instrumental version", VersionToken::Instrumental),
        ("vip mix", VersionToken::Vip),
        ("7\" mix", VersionToken::SevenInch),
        ("12\" mix", VersionToken::TwelveInch),
        ("7 inch", VersionToken::SevenInch),
        ("12 inch", VersionToken::TwelveInch),
        ("lp version", VersionToken::Lp),
        ("lp cut", VersionToken::Lp),
        ("album version", VersionToken::Lp),
        ("mono mix", VersionToken::Mono),
        ("stereo mix", VersionToken::Stereo),
    ];
    let mut remaining = lower.clone();
    for (phrase, token) in phrases {
        if remaining.contains(phrase) {
            out.push(*token);
            remaining = remaining.replace(phrase, " ");
        }
    }

    // Word-level recognition on whatever's left after phrase strip.
    // The token list per PRD §8.1. We tokenise on word boundaries
    // (any non-alphanumeric except `"`) so "Clean Bandit"-style
    // phrases don't appear here in the first place — the parser
    // only ever feeds us bracket / dash-trailing content.
    for word in remaining.split(|c: char| !c.is_ascii_alphanumeric() && c != '"') {
        if word.is_empty() {
            continue;
        }
        let tok = match word {
            "clean" => Some(VersionToken::Clean),
            "dirty" => Some(VersionToken::Dirty),
            "explicit" => Some(VersionToken::Explicit),
            "instrumental" => Some(VersionToken::Instrumental),
            "acapella" | "acappella" | "accapella" => Some(VersionToken::Acapella),
            "radio" => Some(VersionToken::Radio),
            "edit" => Some(VersionToken::Edit),
            "extended" => Some(VersionToken::Extended),
            "club" => Some(VersionToken::Club),
            "dub" => Some(VersionToken::Dub),
            "vip" => Some(VersionToken::Vip),
            "remix" => Some(VersionToken::Remix),
            "remaster" | "remastered" => Some(VersionToken::Remaster),
            "mono" => Some(VersionToken::Mono),
            "stereo" => Some(VersionToken::Stereo),
            "intro" => Some(VersionToken::Intro),
            "outro" => Some(VersionToken::Outro),
            "short" => Some(VersionToken::Short),
            "long" => Some(VersionToken::Long),
            "7\"" | "7in" => Some(VersionToken::SevenInch),
            "12\"" | "12in" => Some(VersionToken::TwelveInch),
            "lp" => Some(VersionToken::Lp),
            _ => None,
        };
        if let Some(tok) = tok {
            out.push(tok);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_set(input: &str) -> BTreeSet<&'static str> {
        parse(input).into_iter().map(|t| t.as_str()).collect()
    }

    #[test]
    fn recognises_parenthesised_clean_dirty() {
        assert_eq!(parse_set("Lady (Clean).mp3"), set(["clean"]));
        assert_eq!(parse_set("Lady (Dirty).mp3"), set(["dirty"]));
        assert_eq!(parse_set("Lady [Clean].mp3"), set(["clean"]));
    }

    #[test]
    fn recognises_radio_and_extended_phrases() {
        assert_eq!(parse_set("Roxanne (Radio Edit).mp3"), set(["radio"]));
        assert_eq!(parse_set("Roxanne (Extended Mix).mp3"), set(["extended"]));
        assert_eq!(parse_set("Roxanne (Club Mix).mp3"), set(["club"]));
    }

    #[test]
    fn recognises_instrumental_and_acapella() {
        assert_eq!(
            parse_set("Untitled (Instrumental).mp3"),
            set(["instrumental"])
        );
        assert_eq!(parse_set("Untitled (Acapella).mp3"), set(["acapella"]));
        // Common misspellings still recognised — DJs in the wild
        // ship files with both `acappella` and `accapella`.
        assert_eq!(parse_set("Untitled (Acappella).mp3"), set(["acapella"]));
    }

    #[test]
    fn recognises_inch_variants() {
        assert_eq!(parse_set("Roxanne (12\" Mix).mp3"), set(["12in"]));
        assert_eq!(parse_set("Roxanne (7\" Mix).mp3"), set(["7in"]));
        assert_eq!(parse_set("Roxanne (12 inch).mp3"), set(["12in"]));
        assert_eq!(parse_set("Roxanne (LP Version).mp3"), set(["lp"]));
    }

    #[test]
    fn recognises_remix_remaster_dub_vip() {
        assert_eq!(parse_set("Track (Remix).mp3"), set(["remix"]));
        assert_eq!(parse_set("Track (Remaster).mp3"), set(["remaster"]));
        assert_eq!(parse_set("Track (Remastered).mp3"), set(["remaster"]));
        assert_eq!(parse_set("Track (Dub).mp3"), set(["dub"]));
        assert_eq!(parse_set("Track (VIP).mp3"), set(["vip"]));
    }

    #[test]
    fn recognises_trailing_dash_segment() {
        assert_eq!(
            parse_set("Song Title - Instrumental.mp3"),
            set(["instrumental"])
        );
        assert_eq!(parse_set("Song - Clean.mp3"), set(["clean"]));
    }

    #[test]
    fn does_not_match_clean_bandit_artist_name() {
        // Critical false-positive guard. "Clean Bandit" appears in
        // the unbracketed prefix; must not be tagged.
        assert!(parse_set("Clean Bandit - Symphony.mp3").is_empty());
        assert!(parse_set("Clean Bandit - Rockabye.mp3").is_empty());
    }

    #[test]
    fn does_not_match_dirty_dancing_title() {
        assert!(parse_set("Dirty Dancing OST.mp3").is_empty());
        assert!(parse_set("Dirty Vegas - Days Go By.mp3").is_empty());
    }

    #[test]
    fn does_not_match_radio_in_artist_position() {
        assert!(parse_set("Radiohead - Karma Police.mp3").is_empty());
        assert!(parse_set("Radio Department - Pulling Our Weight.mp3").is_empty());
    }

    #[test]
    fn does_not_match_inch_nails_artist_name() {
        // Nine Inch Nails — "inch" in artist name, never in
        // bracketed segment.
        assert!(parse_set("Nine Inch Nails - Hurt.mp3").is_empty());
    }

    #[test]
    fn recognises_multiple_tokens_in_one_string() {
        let parsed = parse_set("Track (Radio Edit) (Clean).mp3");
        assert!(parsed.contains("radio"));
        assert!(parsed.contains("clean"));
    }

    #[test]
    fn returns_empty_for_plain_titles() {
        assert!(parse_set("Donuts.mp3").is_empty());
        assert!(parse_set("J Dilla - Workinonit.mp3").is_empty());
    }

    #[test]
    fn case_insensitive_recognition() {
        assert_eq!(parse_set("Track (CLEAN).mp3"), set(["clean"]));
        assert_eq!(parse_set("Track (clean).mp3"), set(["clean"]));
        assert_eq!(parse_set("Track (Clean).mp3"), set(["clean"]));
    }

    #[test]
    fn handles_title_without_extension() {
        assert_eq!(parse_set("Lady (Clean)"), set(["clean"]));
        assert_eq!(parse_set("Track (12\" Mix)"), set(["12in"]));
    }

    #[test]
    fn ignores_overly_long_extensions() {
        // A "real" filename has a 3-5 char extension. Title-internal
        // periods (".com.au", "feat. J Dilla") must not be mistaken
        // for an extension boundary.
        assert!(parse_set("J Dilla feat. Madlib - Track.mp3").is_empty());
    }

    fn set<const N: usize>(items: [&'static str; N]) -> BTreeSet<&'static str> {
        items.into_iter().collect()
    }
}
