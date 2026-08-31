//! Map an external app's track-colour label onto Dub's fixed palette
//! tokens — the same eight the browser's Colour column uses
//! (`red / orange / yellow / green / aqua / blue / purple / pink`).
//!
//! rekordbox exports a hex (`Colour="0xFF0000"`); Traktor a small
//! integer index. Both resolve to a palette token **by hue**, so an
//! imported colour renders (swatch + row tint) and filters exactly like
//! a colour the DJ set themselves, with no per-source rendering code.

/// Map an `R,G,B` triple to the nearest palette token by hue. `None`
/// for near-greys (no meaningful colour label).
pub(crate) fn token_from_rgb(r: u8, g: u8, b: u8) -> Option<&'static str> {
    let rf = f64::from(r) / 255.0;
    let gf = f64::from(g) / 255.0;
    let bf = f64::from(b) / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;
    if delta < 0.08 {
        return None; // grey / black / white: no colour label
    }
    let mut hue = if max == rf {
        60.0 * ((gf - bf) / delta)
    } else if max == gf {
        60.0 * ((bf - rf) / delta) + 120.0
    } else {
        60.0 * ((rf - gf) / delta) + 240.0
    };
    if hue < 0.0 {
        hue += 360.0;
    }
    Some(if !(15.0..345.0).contains(&hue) {
        "red"
    } else if hue < 45.0 {
        "orange"
    } else if hue < 70.0 {
        "yellow"
    } else if hue < 160.0 {
        "green"
    } else if hue < 195.0 {
        "aqua"
    } else if hue < 255.0 {
        "blue"
    } else if hue < 290.0 {
        "purple"
    } else {
        "pink"
    })
}

/// Parse a hex colour string (`0xRRGGBB`, `#RRGGBB`, or bare `RRGGBB`)
/// and map it to a palette token. `None` on a malformed string or a
/// grey. Used for rekordbox's `Colour` attribute.
pub(crate) fn token_from_hex(s: &str) -> Option<&'static str> {
    let h = s.trim();
    let h = h
        .strip_prefix("0x")
        .or_else(|| h.strip_prefix("0X"))
        .or_else(|| h.strip_prefix('#'))
        .unwrap_or(h);
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    token_from_rgb(r, g, b)
}

/// Canonical `R,G,B` for a palette token — the inverse of
/// [`token_from_rgb`], for export (M11f).
///
/// Import is lossy on purpose: any hue lands on the nearest of eight
/// tokens, so the original hex cannot be recovered. What *can* be
/// guaranteed is that these eight values map back to the token they
/// came from, which is what makes a Dub → rekordbox → Dub round-trip
/// stable rather than drifting a colour each pass. The pinned test
/// below is the guarantee.
///
/// `purple` is `8000FF` rather than the more obvious `800080`: that
/// sits at hue 300 and would come back as `pink`.
pub(crate) fn rgb_from_token(token: &str) -> Option<(u8, u8, u8)> {
    Some(match token {
        "red" => (0xFF, 0x00, 0x00),
        "orange" => (0xFF, 0xA5, 0x00),
        "yellow" => (0xFF, 0xFF, 0x00),
        "green" => (0x00, 0xFF, 0x00),
        "aqua" => (0x00, 0xFF, 0xFF),
        "blue" => (0x00, 0x00, 0xFF),
        "purple" => (0x80, 0x00, 0xFF),
        "pink" => (0xFF, 0x00, 0xFF),
        _ => return None,
    })
}

/// Map Traktor's 1-based track-colour index to a palette token.
///
/// **Best-effort.** Traktor's NML colour encoding is not confirmed
/// against a real tagged export, so this 1→red … 7→pink ordering may
/// need a one-line correction once a `collection.nml` carrying colours
/// is available (the same "re-confirm against a real export" caveat the
/// rekordbox importer already carries). `0` / out-of-range → `None`.
pub(crate) fn token_from_traktor_index(n: i64) -> Option<&'static str> {
    Some(match n {
        1 => "red",
        2 => "orange",
        3 => "yellow",
        4 => "green",
        5 => "blue",
        6 => "purple",
        7 => "pink",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Export → import must land on the token it started from, or a
    /// colour drifts one band every round trip.
    #[test]
    fn every_palette_token_survives_a_round_trip() {
        for token in [
            "red", "orange", "yellow", "green", "aqua", "blue", "purple", "pink",
        ] {
            let (r, g, b) = rgb_from_token(token).expect(token);
            assert_eq!(token_from_rgb(r, g, b), Some(token), "{token} drifted");
        }
        assert_eq!(rgb_from_token("chartreuse"), None);
    }

    #[test]
    fn hex_maps_the_eight_dj_colours() {
        assert_eq!(token_from_hex("0xFF0000"), Some("red"));
        assert_eq!(token_from_hex("0xFFA500"), Some("orange"));
        assert_eq!(token_from_hex("#FFFF00"), Some("yellow"));
        assert_eq!(token_from_hex("00FF00"), Some("green"));
        assert_eq!(token_from_hex("0x00FFFF"), Some("aqua"));
        assert_eq!(token_from_hex("0x0000FF"), Some("blue"));
        // Violet-purple (~280°) vs rose-pink (~330°) — distinct hues.
        // (Pure magenta 0xFF00FF and 0x800080 share hue 300° and so
        // can't be told apart by hue; the eight DJ colours don't.)
        assert_eq!(token_from_hex("0x9933CC"), Some("purple"));
        assert_eq!(token_from_hex("0xFF3399"), Some("pink"));
    }

    #[test]
    fn greys_and_garbage_map_to_none() {
        assert_eq!(token_from_hex("0x808080"), None);
        assert_eq!(token_from_hex("0x000000"), None);
        assert_eq!(token_from_hex("0xFFFFFF"), None);
        assert_eq!(token_from_hex("not-hex"), None);
        assert_eq!(token_from_hex("0xFF00"), None);
    }

    #[test]
    fn traktor_index_in_range_only() {
        assert_eq!(token_from_traktor_index(1), Some("red"));
        assert_eq!(token_from_traktor_index(7), Some("pink"));
        assert_eq!(token_from_traktor_index(0), None);
        assert_eq!(token_from_traktor_index(8), None);
    }
}
