//! Filename pattern parser per PRD §8.4.
//!
//! Parses DJ-conventional filename schemes into structured
//! artist / title / year / label / version-token fields, so the
//! M11c importer can populate
//! `track_metadata_source(source='filename')` even when no ID3
//! tags are present. Per PRD §8.4:
//!
//! > "Filename-derived metadata. When ID3 tags are absent or
//! > matched against a junk pattern (`Track 01`, `Unknown`, the
//! > bare filename without extension, `downloaded from xyzblog.com`-
//! > class garbage), Dub parses the filename for common DJ
//! > patterns."
//!
//! # Patterns recognised
//!
//! 1. `ARTIST - TITLE.ext`
//! 2. `ARTIST - TITLE (VERSION).ext`
//! 3. `ARTIST_-_TITLE_(VERSION)_[YEAR].ext`
//! 4. `[LABEL CAT#] ARTIST - TITLE.ext`
//!
//! Multi-pattern recognition is layered: extension is stripped
//! first, then any leading `[LABEL CAT#]`, then any trailing
//! `[YEAR]` or `(YEAR)`, then any trailing `(VERSION)`. The
//! remainder is split on the first ` - ` (after `_` → ` ` substitution)
//! into `artist` / `title`. Version tokens inside the trailing
//! `(VERSION)` segment are also extracted via
//! [`crate::version_tokens::parse`].
//!
//! # Junk-pattern detection
//!
//! `is_junk_title` returns `true` for titles the importer should
//! discard as ID3 junk (`Track 01`, `Unknown`, `Untitled`,
//! blogspot/website noise like `downloaded from xyzblog.com`,
//! filename-style strings like `01.mp3` re-used as the title).
//! When ID3 carries a junk title, the importer falls through to
//! the filename parser and the filename parser's output becomes
//! the displayed value via the §8.1 priority chain.

use std::collections::BTreeSet;

use crate::version_tokens::{parse_plain, VersionToken};

/// Structured result of parsing a filename per PRD §8.4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFilename {
    /// Parsed artist field. `None` when no ` - ` separator was
    /// found (e.g. the file is named after only the title).
    pub artist: Option<String>,
    /// Parsed title field. `None` only when the input is empty or
    /// consists entirely of bracketed prefix/suffix segments.
    pub title: Option<String>,
    /// Year extracted from a trailing `(YYYY)` or `[YYYY]` segment.
    pub year: Option<i32>,
    /// Label / catalog-number string from a leading `[...]`
    /// segment when present (`"DEF JAM DEFR-12345"`, `"3024CD003"`).
    /// We don't try to split it further; the verbatim text goes
    /// into `track_metadata_source.comment` until M11e ships a
    /// stronger Discogs / label-resolution path.
    pub label_catalog: Option<String>,
    /// Version tokens (from the trailing `(VERSION)` segment, or
    /// any other bracketed tail). Sourced via
    /// [`crate::version_tokens::parse`] so dedupe sees the same
    /// token set the title displays.
    pub version_tokens: BTreeSet<VersionToken>,
}

impl ParsedFilename {
    /// `true` when the parser produced no artist / title text.
    /// The caller treats this as "filename source has no useful
    /// metadata" and may fall through to the bare filename string
    /// as a last-ditch title.
    pub fn is_empty(&self) -> bool {
        self.artist.is_none() && self.title.is_none()
    }
}

/// Parse a filename or bare title per PRD §8.4. Pure function.
pub fn parse(input: &str) -> ParsedFilename {
    let trimmed = strip_extension(input.trim());

    // Strip a leading `[LABEL CAT#]` segment. The parser is
    // intentionally generous about what's inside the brackets
    // (alphanum + spaces + `-` + `#` + `.` + `/`); the entire
    // verbatim text is captured and bubbled up.
    let (label_catalog, after_label) = take_leading_bracketed(trimmed);

    // Strip a trailing `[YEAR]` or `(YEAR)` segment. The parser
    // recognises a 4-digit year (1900..=2099) only — looser
    // patterns hit too many false positives (track number, BPM,
    // catalog number).
    let (year, after_year) = take_trailing_year(after_label);

    // Strip any remaining trailing parenthesised / bracketed
    // segment as the `(VERSION)` slot. We don't validate that it
    // contains a recognised token; the parser is content to
    // surface "(Skream Edit)" via the trailing-segment path.
    let (version_segment, after_version) = take_trailing_bracketed(after_year);

    // Version tokens come from the bracket-stripped inner content
    // of the trailing `(VERSION)` segment if present. `parse_plain`
    // scans the inner content directly (the bracket structure has
    // already been consumed by `take_trailing_bracketed`). When
    // there is no bracketed tail, the outer `parse` entry point on
    // the un-stripped `after_year` catches the `... - TOKEN`
    // trailing-dash pattern.
    let version_tokens = match version_segment {
        Some(seg) => parse_plain(seg),
        None => crate::version_tokens::parse(after_year),
    };

    // Replace underscores with spaces (the "ARTIST_-_TITLE" form
    // is conventional on blog rips and old peer-to-peer naming).
    let normalised = after_version.replace('_', " ");
    let normalised = collapse_whitespace(&normalised);

    let (artist, title) = split_artist_title(&normalised);

    ParsedFilename {
        artist,
        title,
        year,
        label_catalog: label_catalog.map(|s| s.trim().to_string()),
        version_tokens,
    }
}

/// `true` for titles the importer treats as ID3 junk and falls
/// through to the filename parser. List per PRD §8.4 examples.
/// Case-insensitive. Conservative: a title like "Track 1 (Live)"
/// is *not* junk because the parenthesised qualifier suggests
/// real metadata.
pub fn is_junk_title(title: &str) -> bool {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();

    // Bare-filename junk: a string that's nothing but digits, or
    // "track NN", or "tracknn", or "untitled", or "unknown".
    if lower == "unknown" || lower == "untitled" {
        return true;
    }
    if let Some(rest) = lower.strip_prefix("track ") {
        if rest.trim().chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    if let Some(rest) = lower.strip_prefix("track") {
        if rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
            return true;
        }
    }
    // Filename-style numeric junk: "01.02" (multi-segment numeric)
    // or "01" / "02" / ... (leading-zero short numeric, indicating
    // a track-number-as-title fallback). Single digits and short
    // numerics without leading zeros are kept (legitimate titles
    // like "23" by Blonde Redhead, "99 Problems", "5" by J Dilla).
    if !lower.is_empty() && lower.chars().all(|c| c.is_ascii_digit() || c == '.') {
        let has_period = lower.contains('.');
        let starts_with_zero = lower.starts_with('0') && lower.len() >= 2;
        if has_period || starts_with_zero {
            return true;
        }
    }

    // Blogspot-era noise. The parser checks for substring rather
    // than exact match because the noise often appears alongside
    // a real title ("Track Title (downloaded from xyzblog.com)").
    let noise = [
        "downloaded from",
        "free download",
        "www.",
        ".blogspot.",
        ".tumblr.",
        ".net/",
        ".com/",
        "http://",
        "https://",
    ];
    if noise.iter().any(|n| lower.contains(n)) {
        return true;
    }

    false
}

/// Strip a `.ext` suffix when the extension is 1–5 ASCII-
/// alphanumeric chars (so `Track ft. Madlib.mp3` keeps the `ft.`
/// dot and only `.mp3` is stripped).
fn strip_extension(input: &str) -> &str {
    if let Some(dot) = input.rfind('.') {
        let ext = &input[dot + 1..];
        if !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            return &input[..dot];
        }
    }
    input
}

/// Strip a leading `[…]` segment if the input begins with one.
/// Returns `(captured_segment, rest_after_segment_trimmed)`.
fn take_leading_bracketed(input: &str) -> (Option<&str>, &str) {
    let s = input.trim_start();
    if !s.starts_with('[') {
        return (None, input);
    }
    if let Some(close) = s.find(']') {
        let inner = &s[1..close];
        let rest = s[close + 1..].trim_start();
        (Some(inner), rest)
    } else {
        (None, input)
    }
}

/// Strip a trailing `(YYYY)` or `[YYYY]` segment when present.
/// Returns `(year_value, rest_before_segment_trimmed)`.
fn take_trailing_year(input: &str) -> (Option<i32>, &str) {
    // Strip trailing separator characters that the underscore-
    // style naming convention leaves dangling. We treat `_`, `-`,
    // and `.` as separator-equivalent to whitespace for the
    // purpose of bracket-tail detection so
    // `Artist_-_Title_(Mix)_[2014]` still hits the right slots.
    let s = trim_trailing_separators(input);
    let (open, close) = match s.chars().last() {
        Some(')') => ('(', ')'),
        Some(']') => ('[', ']'),
        _ => return (None, input),
    };
    if !s.ends_with(close) {
        return (None, input);
    }
    let close_pos = s.len() - close.len_utf8();
    let open_pos = match s[..close_pos].rfind(open) {
        Some(p) => p,
        None => return (None, input),
    };
    let inner = &s[open_pos + 1..close_pos];
    let cleaned = inner.trim();
    if cleaned.len() != 4 {
        return (None, input);
    }
    let year = match cleaned.parse::<i32>() {
        Ok(y) if (1900..=2099).contains(&y) => y,
        _ => return (None, input),
    };
    let rest = s[..open_pos].trim_end();
    (Some(year), rest)
}

/// Strip a single trailing `(...)` or `[...]` segment when
/// present (after year-extraction). Returns
/// `(captured_segment, rest_before_segment_trimmed)`.
fn take_trailing_bracketed(input: &str) -> (Option<&str>, &str) {
    let s = trim_trailing_separators(input);
    let (open, close) = match s.chars().last() {
        Some(')') => ('(', ')'),
        Some(']') => ('[', ']'),
        _ => return (None, input),
    };
    if !s.ends_with(close) {
        return (None, input);
    }
    let close_pos = s.len() - close.len_utf8();
    let open_pos = match s[..close_pos].rfind(open) {
        Some(p) => p,
        None => return (None, input),
    };
    let inner = &s[open_pos + 1..close_pos];
    let rest = s[..open_pos].trim_end();
    (Some(inner), rest)
}

/// Trim trailing whitespace and separator characters (`_`, `-`).
/// Used by the trailing-bracket detectors so that
/// `Artist_-_Title_(Mix)_[2014]` style underscored-separator
/// filenames still expose the bracket structure once the year is
/// peeled off (`_` would otherwise be the last char).
fn trim_trailing_separators(input: &str) -> &str {
    input.trim_end_matches(|c: char| c.is_whitespace() || c == '_' || c == '-')
}

/// Replace runs of whitespace with a single space and trim.
fn collapse_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_space = false;
    for c in input.chars() {
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    let trimmed = out.trim_end();
    trimmed.to_string()
}

/// Split `"Artist Name - Track Title"` on the first ` - `. Either
/// side may be empty (returns `None` for that slot).
fn split_artist_title(input: &str) -> (Option<String>, Option<String>) {
    if let Some(idx) = input.find(" - ") {
        let artist = input[..idx].trim();
        let title = input[idx + 3..].trim();
        let artist = if artist.is_empty() {
            None
        } else {
            Some(artist.to_string())
        };
        let title = if title.is_empty() {
            None
        } else {
            Some(title.to_string())
        };
        (artist, title)
    } else {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            (None, None)
        } else {
            (None, Some(trimmed.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(input: &str) -> ParsedFilename {
        parse(input)
    }

    #[test]
    fn pattern_artist_title() {
        let r = p("J Dilla - Workinonit.mp3");
        assert_eq!(r.artist.as_deref(), Some("J Dilla"));
        assert_eq!(r.title.as_deref(), Some("Workinonit"));
        assert!(r.year.is_none());
        assert!(r.label_catalog.is_none());
        assert!(r.version_tokens.is_empty());
    }

    #[test]
    fn pattern_artist_title_with_version() {
        let r = p("Lady - Lady (Clean).mp3");
        assert_eq!(r.artist.as_deref(), Some("Lady"));
        assert_eq!(r.title.as_deref(), Some("Lady"));
        assert!(r
            .version_tokens
            .iter()
            .any(|t| matches!(t, VersionToken::Clean)));
    }

    #[test]
    fn pattern_underscored_with_year() {
        let r = p("Modeselektor_-_Berlin_(VIP Mix)_[2014].mp3");
        assert_eq!(r.artist.as_deref(), Some("Modeselektor"));
        assert_eq!(r.title.as_deref(), Some("Berlin"));
        assert_eq!(r.year, Some(2014));
        assert!(r
            .version_tokens
            .iter()
            .any(|t| matches!(t, VersionToken::Vip)));
    }

    #[test]
    fn pattern_leading_label_catalog() {
        let r = p("[DEF JAM DEFR-12345] Method Man - Bring The Pain.mp3");
        assert_eq!(r.label_catalog.as_deref(), Some("DEF JAM DEFR-12345"));
        assert_eq!(r.artist.as_deref(), Some("Method Man"));
        assert_eq!(r.title.as_deref(), Some("Bring The Pain"));
    }

    #[test]
    fn extracts_radio_edit_and_year_independently() {
        let r = p("Roxanne - Roxanne (Radio Edit) [2003].mp3");
        assert_eq!(r.artist.as_deref(), Some("Roxanne"));
        assert_eq!(r.title.as_deref(), Some("Roxanne"));
        assert_eq!(r.year, Some(2003));
        assert!(r
            .version_tokens
            .iter()
            .any(|t| matches!(t, VersionToken::Radio)));
    }

    #[test]
    fn title_only_no_dash_separator() {
        let r = p("Donuts.mp3");
        assert!(r.artist.is_none());
        assert_eq!(r.title.as_deref(), Some("Donuts"));
    }

    #[test]
    fn handles_no_extension() {
        let r = p("J Dilla - Workinonit");
        assert_eq!(r.artist.as_deref(), Some("J Dilla"));
        assert_eq!(r.title.as_deref(), Some("Workinonit"));
    }

    #[test]
    fn handles_dot_inside_title_preserves_extension_strip() {
        let r = p("J Dilla feat. Madlib - Track.mp3");
        assert_eq!(r.artist.as_deref(), Some("J Dilla feat. Madlib"));
        assert_eq!(r.title.as_deref(), Some("Track"));
    }

    #[test]
    fn handles_unicode_artist_name() {
        let r = p("Björk - Hyperballad.mp3");
        assert_eq!(r.artist.as_deref(), Some("Björk"));
        assert_eq!(r.title.as_deref(), Some("Hyperballad"));
    }

    #[test]
    fn empty_input_returns_empty_parsed() {
        let r = p("");
        assert!(r.is_empty());
    }

    #[test]
    fn year_only_recognises_four_digit_range() {
        // Five-digit numbers and out-of-range years stay in the
        // title segment.
        let r = p("Track (12345).mp3");
        assert!(r.year.is_none());
        let r = p("Track (1700).mp3");
        assert!(r.year.is_none());
        let r = p("Track (2100).mp3");
        assert!(r.year.is_none());
        // Inside the range, year is captured.
        let r = p("Track (1996).mp3");
        assert_eq!(r.year, Some(1996));
    }

    #[test]
    fn is_junk_title_matches_track_nn_unknown_untitled() {
        assert!(is_junk_title("Track 01"));
        assert!(is_junk_title("Track 14"));
        assert!(is_junk_title("Track01"));
        assert!(is_junk_title("Unknown"));
        assert!(is_junk_title("UNTITLED"));
        assert!(is_junk_title("untitled"));
        assert!(is_junk_title("01"));
        assert!(is_junk_title("01.02"));
        assert!(is_junk_title(""));
        assert!(is_junk_title("   "));
    }

    #[test]
    fn is_junk_title_matches_blog_noise() {
        assert!(is_junk_title("downloaded from xyzblog.com"));
        assert!(is_junk_title("Track (downloaded from xyzblog.com)"));
        assert!(is_junk_title("Free Download by Producer"));
        assert!(is_junk_title("xyzblog.blogspot.com"));
        assert!(is_junk_title("get it at http://example.com"));
    }

    #[test]
    fn is_junk_title_rejects_legitimate_titles() {
        // The shape of these is "real title that happens to have a
        // numeric or parenthetical qualifier"; must not be flagged.
        assert!(!is_junk_title("Donuts"));
        assert!(!is_junk_title("Track 1 (Live)"));
        assert!(!is_junk_title("99 Problems"));
        assert!(!is_junk_title("23")); // Blonde Redhead's "23"
    }

    #[test]
    fn collapses_whitespace_in_artist_title() {
        let r = p("J  Dilla   -    Workinonit.mp3");
        assert_eq!(r.artist.as_deref(), Some("J Dilla"));
        assert_eq!(r.title.as_deref(), Some("Workinonit"));
    }
}
