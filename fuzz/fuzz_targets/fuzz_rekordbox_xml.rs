#![no_main]
//! Fuzz the rekordbox `rekordbox.xml` (`DJ_PLAYLISTS`) parser (M12d).
//!
//! `parse_xml` runs on user-supplied files (a DJ's exported collection),
//! so it must never panic, never hang, and never read out of bounds no
//! matter how malformed or adversarial the bytes are — it may only return
//! `Ok(ParsedLibrary)` or `Err(ParseError)`. This target throws random
//! bytes at it; libFuzzer reports any panic / OOM / timeout as a crash.
//!
//! Run: `cargo +nightly fuzz run fuzz_rekordbox_xml` (or `make fuzz-quick`).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Discard the result: we assert the *absence* of a panic, not any
    // property of the parse. Well-formed-enough bytes return Ok; garbage
    // returns Err; both are fine.
    let _ = dub_library::rekordbox::parse_xml(data);
});
