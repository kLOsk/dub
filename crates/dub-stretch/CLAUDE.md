# dub-stretch rules

The key-lock stretcher: pure Rust, on the hot path (the deck calls it from the
audio thread). Audio-thread and DSP rules apply; the FFI rules are loaded for the
types this crate hands across to Swift through `dub-ffi`.

@../../.cursor/rules/audio-rt.mdc
@../../.cursor/rules/dsp.mdc
@../../.cursor/rules/ffi.mdc
