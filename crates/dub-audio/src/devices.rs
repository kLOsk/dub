//! Device classification and the editable device registry.
//!
//! Dub is opinionated about which audio devices it surfaces to the user.
//! Out of every CoreAudio device the OS reports, only two classes are
//! actually relevant for the DJ workflows the app exists to support
//! (PRD §3):
//!
//! 1. **`PerformanceInterface`** — a DJ-grade audio interface used for
//!    both timecode input and master output in Performance / Timecode
//!    mode. Identified by the heuristic
//!    *external transport (USB / Thunderbolt / FireWire / PCI / AVB)
//!    AND at least 2 stereo out (>= 4 output channels)*. The user can
//!    also force-classify a device via `devices.toml` (see
//!    [`KnownInterface`]).
//!
//!    ## Why the gate is output-channels-only (TCC safety)
//!
//!    The classifier deliberately keys off the OUTPUT channel count and
//!    transport type, never the input channel count. Reading a device's
//!    *input*-scope `kAudioDevicePropertyStreamConfiguration` on macOS
//!    14+ (Sonoma / Sequoia) can wake the microphone-permission (TCC)
//!    layer for any app that declares `NSMicrophoneUsageDescription` —
//!    including for the built-in mic — which is exactly the launch
//!    prompt we must avoid in Prep mode. A DJ interface is uniquely
//!    identified among a typical Mac's devices by "external transport
//!    with >= 4 output channels" just as well as by the old 4-in/4-out
//!    rule (no USB headset or podcast mic ships 4 outputs), so dropping
//!    the input-channel read costs us no real selectivity while keeping
//!    detection and Prep-mode enumeration strictly off the TCC path.
//!    The mic prompt then fires only when Performance capture actually
//!    opens an input AudioUnit, which is the Serato-style behaviour we
//!    want.
//!
//! 2. **`BuiltInOutput`** — the Mac's built-in speakers, used for Prep
//!    Mode output (no input, no DJ rig). Identified by transport type
//!    `Built-in` with at least 2 output channels. The built-in mic is
//!    deliberately excluded (we never want to record from it).
//!
//! Everything else — virtual devices (Camo, Loopback, Teams audio…),
//! iPhone Continuity microphones, Bluetooth headphones, HDMI displays,
//! aggregate devices — is **hard-filtered out** by [`classify`]. The
//! Apple shell, the CLI, and any future picker should only ever see
//! devices that map to one of those two categories.
//!
//! ## Known limitation: preamp detection
//!
//! CoreAudio does not expose a "has a phono preamp" property. There is
//! no driver-side flag for "this device routes RIAA-curved cartridge
//! signal vs line-level". We approximate with
//! *external + 4 in / 4 out* because the intersection of those two
//! constraints is, in practice, every DJ interface the target audience
//! actually uses and (in our testing across the dev rig and the user's
//! real machine) nothing else. When a borderline device shows up that
//! the heuristic gets wrong, the registry's allowlist (`[[interface]]`
//! in `devices.toml`) is the correction path — no UI toggle, no
//! recompile-and-ship; a single-file PR with the new entry plus its
//! per-deck channel map.

use serde::Deserialize;
use std::sync::OnceLock;

/// Embedded `devices.toml` payload. Re-parsed only on test setup; in
/// production code go through [`DeviceRegistry::embedded`].
const REGISTRY_TOML: &str = include_str!("../devices.toml");

/// Static cache for the parsed registry. Filled once on first access
/// and never mutated. The parse is cheap (~tens of µs at most for the
/// hand-edited file size we expect) but doing it at startup means we
/// don't pay it on every `list_audio_devices()` call.
static REGISTRY: OnceLock<DeviceRegistry> = OnceLock::new();

/// Category the device classifier returns for a recognised device.
///
/// The Apple shell uses this to populate two separate pickers (input
/// device in Performance mode, output device in Prep mode) and to
/// decide whether to expose timecode controls. The CLI uses it to
/// pick which subcommands accept the device as a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceCategory {
    /// DJ-grade audio interface — capable of both timecode input and
    /// master output. Used for everything in Performance / Timecode mode.
    PerformanceInterface,

    /// The Mac's built-in speakers (or any built-in output device).
    /// Used for Prep Mode output only.
    BuiltInOutput,
}

impl DeviceCategory {
    /// Stable string label for logging, CLI output, and FFI serialisation.
    /// Do not change without updating Swift / CLI consumers.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PerformanceInterface => "performance_interface",
            Self::BuiltInOutput => "built_in_output",
        }
    }
}

/// CoreAudio transport-type taxonomy, mapped to the three buckets the
/// classifier actually cares about.
///
/// The mapping from raw `kAudioDeviceTransportType*` values to this
/// enum lives in `macos.rs`; this enum exists so [`classify`] can be
/// unit-tested without dragging CoreAudio constants into the test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// USB, Thunderbolt, FireWire, PCI, AVB — the buses DJ interfaces
    /// actually use.
    External,

    /// Built-in — the laptop's internal mic and speakers.
    BuiltIn,

    /// Bluetooth, HDMI, AirPlay, Virtual (Camo / Teams / Loopback),
    /// Aggregate, Continuity (iPhone mic), or Unknown. The classifier
    /// always drops these.
    Other,
}

/// One entry in the `[[interface]]` table of `devices.toml`.
///
/// `name_pattern` is matched case-insensitively as a substring against
/// the CoreAudio device name returned by `get_device_name(id)`. The
/// first entry whose pattern matches wins, so order matters when
/// patterns overlap (put more specific patterns first).
///
/// Channel indices are 1-based in the file to match every interface's
/// back-panel labelling. Convert via [`KnownInterface::deck_a_zero_based`]
/// when you need to feed `dub_engine::OutputRouting` directly.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct KnownInterface {
    /// Case-insensitive substring matched against the CoreAudio device
    /// name. The first match wins.
    pub name_pattern: String,

    /// Human-readable display name, surfaced by `dub list-inputs` /
    /// `list-outputs` and the Apple shell picker.
    pub display_name: String,

    /// Vendor (informational only — not used for matching).
    #[serde(default)]
    pub manufacturer: Option<String>,

    /// Total physical output channels we open the AU with. The SL3 has
    /// 6; opening only 4 would silently drop the deck B route.
    pub output_channels: u32,

    /// 1-based first output channel for deck A's stereo pair.
    pub deck_a_first_channel: u32,

    /// 1-based first output channel for deck B's stereo pair.
    pub deck_b_first_channel: u32,

    /// 1-based first INPUT channel for deck A's timecode pair. Optional:
    /// defaults to [`Self::deck_a_first_channel`] when omitted, because
    /// on the reference device (SL3) the deck input pairs sit on the
    /// same physical channel numbers as the output pairs (deck A on
    /// 3+4, deck B on 5+6). Override only for interfaces whose input
    /// channel layout differs from their output layout.
    #[serde(default)]
    pub deck_a_input_first_channel: Option<u32>,

    /// 1-based first INPUT channel for deck B's timecode pair. Defaults
    /// to [`Self::deck_b_first_channel`] when omitted.
    #[serde(default)]
    pub deck_b_input_first_channel: Option<u32>,

    /// `true` if this profile has been validated end-to-end on real
    /// hardware. `false` entries are best-effort guesses; the CLI warns
    /// the user to double-check.
    #[serde(default)]
    pub verified: bool,
}

impl KnownInterface {
    /// 0-based first output channel for deck A's pair. Pass directly to
    /// `dub_engine::OutputRouting`.
    #[must_use]
    pub fn deck_a_zero_based(&self) -> u32 {
        self.deck_a_first_channel.saturating_sub(1)
    }

    /// 0-based first output channel for deck B's pair.
    #[must_use]
    pub fn deck_b_zero_based(&self) -> u32 {
        self.deck_b_first_channel.saturating_sub(1)
    }

    /// 1-based first input channel for deck A, falling back to the
    /// output first-channel when the registry entry doesn't pin a
    /// distinct input layout.
    #[must_use]
    pub fn deck_a_input_first(&self) -> u32 {
        self.deck_a_input_first_channel
            .unwrap_or(self.deck_a_first_channel)
    }

    /// 1-based first input channel for deck B (same fallback rule as
    /// [`Self::deck_a_input_first`]).
    #[must_use]
    pub fn deck_b_input_first(&self) -> u32 {
        self.deck_b_input_first_channel
            .unwrap_or(self.deck_b_first_channel)
    }
}

/// Resolved per-deck routing for a Performance-mode session, derived
/// from the registry (for a known interface) or a safe default (for a
/// heuristic-only match). All channel numbers are **1-based** to match
/// the FFI / CLI convention; the engine boundary converts to 0-based.
///
/// This is the single source of truth the FFI's `performance_routing_for`
/// and `start_thru*` paths consume so the Swift shell never has to know
/// a device's channel map — it just asks the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerformanceRouting {
    /// 1-based first input channel for deck A's timecode pair.
    pub deck_a_input_first: u32,
    /// 1-based first input channel for deck B's timecode pair. Only
    /// meaningful when [`Self::two_deck`] is `true`.
    pub deck_b_input_first: u32,
    /// Number of physical output channels to open the output AU with.
    pub output_channels: u32,
    /// 1-based first output channel for deck A's master pair.
    pub deck_a_output_first: u32,
    /// 1-based first output channel for deck B's master pair. Only
    /// meaningful when [`Self::two_deck`] is `true`.
    pub deck_b_output_first: u32,
    /// `true` when the interface is a known two-deck DVS box (deck A and
    /// deck B each captured + routed independently). `false` for a
    /// heuristic-only interface, where we fall back to a single deck on
    /// the first stereo input pair and a 2-channel summed output.
    pub two_deck: bool,
}

impl PerformanceRouting {
    /// The conservative default for an interface Dub detected by the
    /// heuristic but that has no `devices.toml` entry: a single deck on
    /// input channels 1+2, summed to a 2-channel master output. The
    /// user adds an `[[interface]]` block to unlock two-deck routing.
    #[must_use]
    pub fn heuristic_default() -> Self {
        Self {
            deck_a_input_first: 1,
            deck_b_input_first: 3,
            output_channels: 2,
            deck_a_output_first: 1,
            deck_b_output_first: 1,
            two_deck: false,
        }
    }
}

/// One entry in the `[[denylist]]` table of `devices.toml`. Force-excludes
/// a device whose name matches `name_pattern` even if the heuristic
/// would otherwise accept it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DenyEntry {
    /// Case-insensitive substring matched against the CoreAudio device
    /// name.
    pub name_pattern: String,
}

/// Parsed contents of `devices.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct DeviceRegistry {
    /// Allowlisted DJ interfaces with their per-deck routing maps.
    /// Always-classify-as-`PerformanceInterface` regardless of channel
    /// counts.
    #[serde(default, rename = "interface")]
    pub interfaces: Vec<KnownInterface>,

    /// Names to force-exclude even if the heuristic accepts them.
    #[serde(default)]
    pub denylist: Vec<DenyEntry>,
}

impl DeviceRegistry {
    /// Access the embedded registry, parsing the TOML payload on first
    /// call.
    ///
    /// # Panics
    ///
    /// Panics on parse failure. The file ships inside the binary and
    /// is covered by unit tests (see [`tests::embedded_registry_parses`]),
    /// so a parse error here is a build-time bug — a malformed
    /// `devices.toml` would fail CI before reaching a user's machine.
    /// We deliberately do not surface this as `Result` because no
    /// caller has a meaningful recovery path: if the embedded file
    /// is broken, the audio subsystem is unrecoverable.
    #[must_use]
    pub fn embedded() -> &'static DeviceRegistry {
        REGISTRY.get_or_init(|| {
            toml::from_str(REGISTRY_TOML)
                .expect("crates/dub-audio/devices.toml is malformed (this is a build-time bug)")
        })
    }

    /// Parse a TOML string directly. Public so tests and any future
    /// "load a user override file" feature can build their own registry
    /// without going through the embedded payload.
    ///
    /// # Errors
    /// Returns the underlying [`toml::de::Error`] on parse failure.
    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// First [`KnownInterface`] whose `name_pattern` is a case-insensitive
    /// substring of `device_name`. Used both for classifier allowlist
    /// override and for per-deck routing lookup.
    #[must_use]
    pub fn match_interface(&self, device_name: &str) -> Option<&KnownInterface> {
        let haystack = device_name.to_ascii_lowercase();
        self.interfaces
            .iter()
            .find(|i| haystack.contains(&i.name_pattern.to_ascii_lowercase()))
    }

    /// Look up a profile by `name_pattern` (case-insensitive exact
    /// match). Mirrors the legacy `device_profiles::profile_by_pattern`
    /// helper, used by the `--device-profile <name>` CLI override.
    #[must_use]
    pub fn interface_by_pattern(&self, pattern: &str) -> Option<&KnownInterface> {
        self.interfaces
            .iter()
            .find(|i| i.name_pattern.eq_ignore_ascii_case(pattern))
    }

    /// True iff `device_name` matches any `[[denylist]]` entry.
    #[must_use]
    pub fn is_denied(&self, device_name: &str) -> bool {
        let haystack = device_name.to_ascii_lowercase();
        self.denylist
            .iter()
            .any(|d| haystack.contains(&d.name_pattern.to_ascii_lowercase()))
    }

    /// Resolve the Performance-mode per-deck routing for a device by
    /// name. A registry-matched interface yields its full two-deck
    /// input + output map; anything else gets
    /// [`PerformanceRouting::heuristic_default`].
    #[must_use]
    pub fn performance_routing(&self, device_name: &str) -> PerformanceRouting {
        match self.match_interface(device_name) {
            Some(i) => PerformanceRouting {
                deck_a_input_first: i.deck_a_input_first(),
                deck_b_input_first: i.deck_b_input_first(),
                output_channels: i.output_channels,
                deck_a_output_first: i.deck_a_first_channel,
                deck_b_output_first: i.deck_b_first_channel,
                two_deck: true,
            },
            None => PerformanceRouting::heuristic_default(),
        }
    }
}

/// One CoreAudio device, with everything the rest of the app needs to
/// pick it, open it, and label it in the UI.
///
/// Returned by `list_audio_devices()` in `macos.rs`, exposed across the
/// FFI to Swift, and emitted by the CLI's `list-inputs` / `list-outputs`
/// subcommands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    /// `kAudioDevicePropertyDeviceUID` — stable across reconnects,
    /// safer to persist as the user's saved-device key than the
    /// human-readable name (two SL3s in the same room collide on name
    /// but not UID).
    pub uid: String,

    /// CoreAudio device name (e.g. `"SL 3"`, `"MacBook Pro Speakers"`).
    pub name: String,

    /// `kAudioDevicePropertyManufacturer`. Informational; `None` if the
    /// driver doesn't publish one.
    pub manufacturer: Option<String>,

    /// Number of physical input channels on the device. 0 for a
    /// pure-output device like the built-in speakers.
    pub input_channels: u32,

    /// Number of physical output channels on the device.
    pub output_channels: u32,

    /// Which of the two Dub-relevant categories this device falls in.
    pub category: DeviceCategory,
}

/// Pure classifier. Given a device's identity, transport, and output
/// channel count, decide whether Dub surfaces it and as what.
///
/// Note the absence of an input-channel parameter: classification is
/// deliberately input-scope-free so detection and Prep-mode enumeration
/// never touch the macOS microphone-permission (TCC) path. See the
/// module-level docs for the full rationale.
///
/// Returns `None` for everything we want to hide. The order of checks
/// matters:
///
/// 1. **Denylist** — explicit override always wins.
/// 2. **Allowlist** — if a `[[interface]]` entry matches by name, the
///    device is classified as `PerformanceInterface` regardless of
///    channel counts. This is the only way a sub-4-output DVS
///    interface (rare but possible) makes it through the filter.
/// 3. **External transport AND >= 4 out** -> `PerformanceInterface`.
///    No USB headset, podcast mic, or stereo DAC ships four physical
///    outputs, so the output-count gate isolates DJ interfaces just as
///    cleanly as the old 4-in/4-out rule without an input-scope read.
/// 4. **Built-in transport AND >= 2 out** -> `BuiltInOutput`. Pure
///    inputs (the built-in mic alone, 0 outputs) are excluded.
/// 5. Everything else -> `None`.
#[must_use]
pub fn classify(
    name: &str,
    transport: TransportKind,
    output_channels: u32,
    registry: &DeviceRegistry,
) -> Option<DeviceCategory> {
    if registry.is_denied(name) {
        return None;
    }
    if registry.match_interface(name).is_some() {
        return Some(DeviceCategory::PerformanceInterface);
    }
    match transport {
        TransportKind::External if output_channels >= 4 => {
            Some(DeviceCategory::PerformanceInterface)
        }
        TransportKind::BuiltIn if output_channels >= 2 => Some(DeviceCategory::BuiltInOutput),
        _ => None,
    }
}

/// Convenience predicate over [`classify`]: is this device a DJ-grade
/// Performance interface? Used by the auto-detect / hot-plug path in
/// `macos.rs`, which only needs the yes/no answer and benefits from a
/// single, unit-testable definition shared with the full classifier.
#[must_use]
pub fn is_performance_interface(
    name: &str,
    transport: TransportKind,
    output_channels: u32,
    registry: &DeviceRegistry,
) -> bool {
    matches!(
        classify(name, transport, output_channels, registry),
        Some(DeviceCategory::PerformanceInterface)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirror of the user's real device list (from `system_profiler
    /// SPAudioDataType`) at plan time. If any classifier change breaks
    /// this fixture, the change is wrong.
    fn user_fixture() -> Vec<(
        &'static str,
        TransportKind,
        u32,
        u32,
        Option<DeviceCategory>,
    )> {
        vec![
            (
                "iPhone 16 Pro 256GB Black Microphone",
                TransportKind::Other,
                1,
                0,
                None,
            ),
            ("MacBook Pro Microphone", TransportKind::BuiltIn, 1, 0, None),
            (
                "MacBook Pro Speakers",
                TransportKind::BuiltIn,
                0,
                2,
                Some(DeviceCategory::BuiltInOutput),
            ),
            ("Camo Microphone", TransportKind::Other, 2, 2, None),
            ("Microsoft Teams Audio", TransportKind::Other, 1, 1, None),
            (
                "SL 3",
                TransportKind::External,
                6,
                6,
                Some(DeviceCategory::PerformanceInterface),
            ),
        ]
    }

    #[test]
    fn embedded_registry_parses() {
        let r = DeviceRegistry::embedded();
        assert!(
            !r.interfaces.is_empty(),
            "embedded devices.toml must at least carry SL 3"
        );
        assert!(
            r.match_interface("SL 3").is_some(),
            "SL 3 entry must survive the move from device_profiles.rs"
        );
    }

    #[test]
    fn sl3_routing_matches_legacy_device_profiles() {
        // The SL3 profile is the verified one; deck A on physical ch 3+4,
        // deck B on 5+6. Channels in the TOML are 1-based; the engine
        // wants 0-based.
        let r = DeviceRegistry::embedded();
        let sl3 = r.match_interface("Rane SL 3").expect("matches SL 3");
        assert_eq!(sl3.display_name, "Serato SL 3");
        assert_eq!(sl3.output_channels, 6);
        assert_eq!(sl3.deck_a_first_channel, 3);
        assert_eq!(sl3.deck_b_first_channel, 5);
        assert_eq!(sl3.deck_a_zero_based(), 2);
        assert_eq!(sl3.deck_b_zero_based(), 4);
        assert!(sl3.verified);
    }

    #[test]
    fn audio6_is_unverified() {
        let r = DeviceRegistry::embedded();
        let a6 = r
            .match_interface("Traktor Audio 6")
            .expect("matches Audio 6");
        assert!(
            !a6.verified,
            "Audio 6 routing is a driver-panel guess; must stay unverified \
             until validated on real hardware"
        );
        assert_eq!(a6.deck_a_zero_based(), 0);
        assert_eq!(a6.deck_b_zero_based(), 2);
    }

    #[test]
    fn interface_by_pattern_is_case_insensitive_exact() {
        let r = DeviceRegistry::embedded();
        assert_eq!(
            r.interface_by_pattern("sl 3").unwrap().display_name,
            "Serato SL 3"
        );
        assert_eq!(
            r.interface_by_pattern("AUDIO 6").unwrap().display_name,
            "Traktor Audio 6"
        );
        assert!(r.interface_by_pattern("nope").is_none());
    }

    #[test]
    fn match_interface_matches_substring_and_is_case_insensitive() {
        // CoreAudio sometimes returns "Rane SL 3" or "Scratch Live SL 3"
        // depending on driver version. Substring + case-insensitive
        // means all of these match the same profile.
        let r = DeviceRegistry::embedded();
        for case in ["sl 3", "Rane SL 3", "Scratch Live SL 3", "SL 3 (4-Out)"] {
            assert!(
                r.match_interface(case).is_some(),
                "{case} did not match SL 3"
            );
        }
    }

    #[test]
    fn classify_drops_user_clutter_keeps_sl3_and_speakers() {
        let r = DeviceRegistry::embedded();
        for (name, transport, ich, och, want) in user_fixture() {
            let got = classify(name, transport, och, r);
            assert_eq!(
                got, want,
                "fixture mismatch for {name} ({ich} in / {och} out, {transport:?})"
            );
        }
    }

    #[test]
    fn classify_rejects_usb_two_output_device() {
        // PRD §3 explicitly: a built-in mic or USB headset must not
        // trigger Performance mode. The output-count gate keeps a
        // stereo USB DAC / headset (2 outputs) out of the Performance
        // bucket regardless of how many inputs it advertises — and we
        // never read its inputs, which is the whole TCC-safety point.
        let r = DeviceRegistry::embedded();
        assert_eq!(
            classify("USB Audio Codec", TransportKind::External, 0, r),
            None
        );
        assert_eq!(
            classify("Generic USB Mic", TransportKind::External, 0, r),
            None
        );
        assert_eq!(classify("USB Headset", TransportKind::External, 2, r), None);
    }

    #[test]
    fn classify_accepts_external_four_output_dj_interface() {
        // Anything with at least 2 stereo out on a real bus is
        // classified as a Performance interface even when the registry
        // doesn't list it. This is the heuristic fallback, and it must
        // hold independent of the input-channel count (we never read
        // it): a 2-in / 4-out external box still qualifies.
        let r = DeviceRegistry::embedded();
        assert_eq!(
            classify("Some 4-out DVS Interface", TransportKind::External, 4, r),
            Some(DeviceCategory::PerformanceInterface)
        );
    }

    #[test]
    fn classify_excludes_built_in_mic_only() {
        // A built-in transport device with no output is the built-in
        // mic alone. We never want to surface it; 0-output BuiltIn
        // falls through to `None`.
        let r = DeviceRegistry::embedded();
        assert_eq!(
            classify("MacBook Pro Microphone", TransportKind::BuiltIn, 0, r),
            None
        );
    }

    #[test]
    fn classify_drops_virtual_and_unknown_transport_devices() {
        // Camo, Teams, Loopback, AirPods (Bluetooth), iPhone Continuity
        // microphone — all return `Other` transport from the macOS
        // mapping and must be dropped regardless of channel count.
        let r = DeviceRegistry::embedded();
        for (name, och) in [
            ("Camo Microphone", 2),
            ("Microsoft Teams Audio", 1),
            ("Loopback Audio", 8),
            ("AirPods Pro", 2),
            ("iPhone 16 Pro Microphone", 0),
        ] {
            assert_eq!(
                classify(name, TransportKind::Other, och, r),
                None,
                "{name} ({och} out, Other transport) must be dropped"
            );
        }
    }

    #[test]
    fn registry_allowlist_overrides_channel_heuristic() {
        // A 2-out DVS interface should still classify as Performance if
        // the user added it to the registry. Tests the allowlist path
        // without needing to ship such a device in the embedded file.
        let toml = r#"
            [[interface]]
            name_pattern = "Tiny DVS"
            display_name = "Tiny DVS 2x2"
            output_channels = 2
            deck_a_first_channel = 1
            deck_b_first_channel = 1
            verified = false
        "#;
        let r = DeviceRegistry::from_toml_str(toml).expect("custom registry parses");
        assert_eq!(
            classify("Tiny DVS 2x2", TransportKind::External, 2, &r),
            Some(DeviceCategory::PerformanceInterface),
            "allowlist must override the >=4-output heuristic"
        );
    }

    #[test]
    fn registry_denylist_overrides_external_heuristic() {
        // A 4-out external studio interface that is NOT a DVS box
        // should still be hidden if the user (or this project)
        // denylists it. Denylist beats the channel-count gate.
        let toml = r#"
            [[interface]]
            name_pattern = "SL 3"
            display_name = "Serato SL 3"
            output_channels = 6
            deck_a_first_channel = 3
            deck_b_first_channel = 5
            verified = true

            [[denylist]]
            name_pattern = "Studio Interface 4x4"
        "#;
        let r = DeviceRegistry::from_toml_str(toml).expect("custom registry parses");
        assert_eq!(
            classify("Acme Studio Interface 4x4", TransportKind::External, 4, &r),
            None,
            "denylist must hide an otherwise-qualifying 4-out device"
        );
        assert_eq!(
            classify("Rane SL 3", TransportKind::External, 6, &r),
            Some(DeviceCategory::PerformanceInterface),
            "denylist must not hide unrelated devices"
        );
    }

    #[test]
    fn registry_denylist_beats_allowlist_when_name_matches_both() {
        // If a single device name matches both an `[[interface]]` and
        // a `[[denylist]]` entry the denylist wins. Documents the
        // ordering invariant `classify` relies on.
        let toml = r#"
            [[interface]]
            name_pattern = "Tricky Device"
            display_name = "Tricky Device"
            output_channels = 4
            deck_a_first_channel = 1
            deck_b_first_channel = 3
            verified = false

            [[denylist]]
            name_pattern = "Tricky Device"
        "#;
        let r = DeviceRegistry::from_toml_str(toml).expect("custom registry parses");
        assert_eq!(
            classify("Tricky Device", TransportKind::External, 4, &r),
            None
        );
    }

    #[test]
    fn is_performance_interface_matches_classify() {
        let r = DeviceRegistry::embedded();
        // SL3 (registry + external 6-out) -> true.
        assert!(is_performance_interface(
            "Rane SL 3",
            TransportKind::External,
            6,
            r
        ));
        // Heuristic-only external 4-out -> true.
        assert!(is_performance_interface(
            "Unknown 4-out Box",
            TransportKind::External,
            4,
            r
        ));
        // USB headset (2 out) -> false.
        assert!(!is_performance_interface(
            "USB Headset",
            TransportKind::External,
            2,
            r
        ));
        // Built-in speakers -> false (that's BuiltInOutput, not a
        // Performance interface).
        assert!(!is_performance_interface(
            "MacBook Pro Speakers",
            TransportKind::BuiltIn,
            2,
            r
        ));
    }

    #[test]
    fn performance_routing_sl3_is_two_deck_3456() {
        let r = DeviceRegistry::embedded();
        let pr = r.performance_routing("Rane SL 3");
        assert!(pr.two_deck);
        assert_eq!(pr.deck_a_input_first, 3, "SL3 deck A input on ch 3+4");
        assert_eq!(pr.deck_b_input_first, 5, "SL3 deck B input on ch 5+6");
        assert_eq!(pr.output_channels, 6);
        assert_eq!(pr.deck_a_output_first, 3);
        assert_eq!(pr.deck_b_output_first, 5);
    }

    #[test]
    fn performance_routing_unknown_device_is_heuristic_default() {
        let r = DeviceRegistry::embedded();
        let pr = r.performance_routing("Some Unlisted 4-out Interface");
        assert_eq!(pr, PerformanceRouting::heuristic_default());
        assert!(!pr.two_deck);
        assert_eq!(pr.deck_a_input_first, 1);
        assert_eq!(pr.output_channels, 2);
    }

    #[test]
    fn input_first_channels_fall_back_to_output_then_override() {
        // SL3 has no explicit input channels -> falls back to output
        // firsts (3 / 5).
        let r = DeviceRegistry::embedded();
        let sl3 = r.match_interface("SL 3").expect("SL3 present");
        assert_eq!(sl3.deck_a_input_first(), 3);
        assert_eq!(sl3.deck_b_input_first(), 5);

        // An interface that pins a distinct input layout uses it.
        let toml = r#"
            [[interface]]
            name_pattern = "Split IO"
            display_name = "Split IO 8x8"
            output_channels = 8
            deck_a_first_channel = 1
            deck_b_first_channel = 3
            deck_a_input_first_channel = 5
            deck_b_input_first_channel = 7
            verified = false
        "#;
        let r2 = DeviceRegistry::from_toml_str(toml).expect("parses");
        let dev = r2.match_interface("Split IO 8x8").expect("present");
        assert_eq!(dev.deck_a_input_first(), 5);
        assert_eq!(dev.deck_b_input_first(), 7);
        let pr = r2.performance_routing("Split IO 8x8");
        assert_eq!(pr.deck_a_input_first, 5);
        assert_eq!(pr.deck_b_input_first, 7);
        assert_eq!(pr.deck_a_output_first, 1);
        assert_eq!(pr.deck_b_output_first, 3);
    }

    #[test]
    fn devices_pairs_are_disjoint_and_fit_in_output_channels() {
        // Carry over the M5.5.2 invariants previously enforced by
        // `device_profiles::tests::deck_routing_pairs_are_disjoint` /
        // `deck_pairs_fit_in_output_channels` so the migration to the
        // registry is a no-regression move.
        let r = DeviceRegistry::embedded();
        for i in &r.interfaces {
            let a = i.deck_a_zero_based();
            let b = i.deck_b_zero_based();
            let a_pair = a..a + 2;
            let b_pair = b..b + 2;
            assert!(
                a_pair.end <= b_pair.start || b_pair.end <= a_pair.start,
                "{} has overlapping deck A {a_pair:?} and deck B {b_pair:?}",
                i.display_name
            );
            assert!(
                a_pair.end <= i.output_channels,
                "{}: deck A pair end {} > output_channels {}",
                i.display_name,
                a_pair.end,
                i.output_channels
            );
            assert!(
                b_pair.end <= i.output_channels,
                "{}: deck B pair end {} > output_channels {}",
                i.display_name,
                b_pair.end,
                i.output_channels
            );
        }
    }
}
