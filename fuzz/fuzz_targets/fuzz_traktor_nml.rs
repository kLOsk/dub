#![no_main]
//! Fuzz the Traktor `collection.nml` parser (M12b).
//!
//! `parse_nml` runs on user-supplied files (a DJ's exported collection),
//! so it must never panic, never hang, and never read out of bounds no
//! matter how malformed or adversarial the bytes are — it may only return
//! `Ok(ParsedCollection)` or `Err(ParseError)`. This target throws random
//! bytes at it; libFuzzer reports any panic / OOM / timeout as a crash.
//!
//! Run: `cargo +nightly fuzz run fuzz_traktor_nml` (or `make fuzz-quick`).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Discard the result: we are asserting the *absence* of a panic, not
    // any property of the parse. A well-formed-enough buffer returns Ok;
    // garbage returns Err; both are fine.
    let _ = dub_library::traktor::parse_nml(data);
});
