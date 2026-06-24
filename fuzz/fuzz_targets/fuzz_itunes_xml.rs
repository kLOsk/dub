#![no_main]
//! Fuzz the iTunes / Apple Music `Library.xml` plist parser (M12c).
//!
//! `parse_library` runs on user-supplied XML and must never panic / hang /
//! read out of bounds — only `Ok(ItunesLibrary)` or `Err(ParseError)`. The
//! interesting cases are unbalanced `<dict>`/`<array>` nesting, missing
//! values after `<key>`, and adversarial `Location` percent-encoding.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = dub_library::itunes::parse_library(data);
});
