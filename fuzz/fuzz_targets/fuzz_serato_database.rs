#![no_main]
//! Fuzz the Serato `database V2` / `.crate` binary tag parser (M11e).
//!
//! `parse_database_v2` / `parse_crate` run on user-supplied files and must
//! never panic, hang, or read out of bounds on any bytes — only return a
//! (possibly empty) `Vec`. Adversarial tag lengths, truncation, and bad
//! UTF-16 are the interesting cases.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = dub_library::serato::database::parse_database_v2(data);
    let _ = dub_library::serato::database::parse_crate(data);
});
