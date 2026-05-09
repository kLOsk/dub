# dub-io test fixtures

Each file is the same source signal — a 0.5-second 440 Hz stereo tone at
44.1 kHz, amplitude 0.3 — re-encoded into the listed format.

| File | Format | Codec | Container |
|---|---|---|---|
| `tone.wav` | WAV | PCM s16le | RIFF |
| `tone.aiff` | AIFF | PCM s16be | RIFF |
| `tone.flac` | FLAC | FLAC | FLAC |
| `tone.mp3` | MP3 | MPEG Layer III @ 192 kbps | (raw) |
| `tone-aac.m4a` | M4A (AAC) | AAC-LC @ 192 kbps | ISO MP4 |
| `tone-alac.m4a` | M4A (ALAC) | Apple Lossless | ISO MP4 |

Used by `crates/dub-io/tests/format_coverage.rs`. Total ≈ 224 KB; small
enough to live in the repo without bloating clones.

## Regeneration

If you ever need to regenerate (e.g. to test a new format), run from this
directory:

```sh
# Source: 0.5 s, 440 Hz, stereo, 44.1 kHz, 16-bit. Deterministic.
python3 -c "
import wave, struct, math
sr = 44100; duration = 0.5; freq = 440.0
n = int(sr * duration)
with wave.open('tone.wav', 'wb') as w:
    w.setnchannels(2); w.setsampwidth(2); w.setframerate(sr)
    for i in range(n):
        v = int(0.3 * 32767 * math.sin(2 * math.pi * freq * i / sr))
        w.writeframes(struct.pack('<hh', v, v))
"

ffmpeg -y -i tone.wav -c:a pcm_s16be -f aiff tone.aiff
ffmpeg -y -i tone.wav -c:a libmp3lame -b:a 192k tone.mp3
ffmpeg -y -i tone.wav -c:a flac tone.flac
ffmpeg -y -i tone.wav -c:a aac -b:a 192k tone-aac.m4a
ffmpeg -y -i tone.wav -c:a alac tone-alac.m4a
```

Tolerance bands for lossy formats are documented in `format_coverage.rs`.
