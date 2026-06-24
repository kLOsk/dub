#![no_main]
//! Fuzz the Serato `Serato Markers2` GEOB decoder (M11e).
//!
//! `markers2::parse` decodes a base64 blob then walks NUL-terminated entries
//! with attacker-controlled lengths — it must never panic / hang / read out
//! of bounds, only return a (possibly empty) `SeratoMarkers`. Also exercises
//! the beat-grid + autotags decoders, which share the same no-panic contract.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = dub_library::serato::markers2::parse(data);
    let _ = dub_library::serato::beatgrid::parse(data);
    let _ = dub_library::serato::autotags::parse(data);
});
