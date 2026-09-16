//
//  DubKeymapStore.swift
//  Dub
//
//  What the DJ bound in map mode, and the table it resolves to (M18).
//
//  Serato's mechanism, which is the one to copy: turn MAP on, click a
//  control, press the key. The store is the persistence and the merge
//  behind that gesture. It holds only *overrides* — an action the DJ
//  rebound, or explicitly unbound — over `DubKeymap.defaults`, so the
//  default map stays the thing a profile is diffed against, and a
//  profile is small and readable.
//
//  ## One key, one action
//
//  Binding a chord that another action already holds moves it: the other
//  action is left unbound, whether its key came from the defaults or from
//  an earlier remap. That is what a DJ expects of a key (Serato does the
//  same) and it is what keeps the dispatcher's first-match order from ever
//  mattering.
//
//  ## Only the keyboard lane dispatches
//
//  A binding carries a transport so the MIDI and HID lanes slot in
//  without a migration; the overrides are keyed by action and hold a
//  key chord today. When the MIDI lane lands it is a second value on the
//  same override, not a second store.
//

import Foundation

/// The DJ's overrides over the default map, persisted under
/// `dub.keymap.v1`.
final class DubKeymapStore: ObservableObject {

    /// The app's one store. A `var` so a test can stand in an isolated
    /// suite and put this one back — the suite runs hosted in the app,
    /// and `UserDefaults.standard` there is the DJ's real map.
    static var shared = DubKeymapStore()

    /// One override: a chord, or `nil` for "the DJ unbound this".
    struct Override: Codable, Equatable {
        var chord: DubKeyChord?
    }

    private static let key = "dub.keymap.v1"

    /// Bumped on every change so views holding legends re-read them.
    @Published private(set) var revision: Int = 0

    private(set) var overrides: [String: Override] = [:] {
        didSet {
            rebuild()
            revision &+= 1
        }
    }

    /// The resolved table — defaults with overrides applied.
    private(set) var resolved: [DubBinding] = []
    private(set) var resolvedByAction: [DubAction: DubBinding] = [:]

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        if let data = defaults.data(forKey: Self.key),
           let stored = try? JSONDecoder().decode([String: Override].self, from: data)
        {
            overrides = stored
        }
        rebuild()
    }

    // MARK: - Editing

    /// Bind `action` to `chord`, taking the chord away from whatever held
    /// it. `⌘,` cannot be taken and cannot be rebound.
    func bind(_ action: DubAction, to chord: DubKeyChord) {
        guard action.isRemappable else { return }
        var next = overrides
        for holder in resolved where holder.transport == .key && holder.action != action {
            guard holder.action.isRemappable, Self.matches(holder, chord) else { continue }
            next[holder.action.id] = Override(chord: nil)
        }
        next[action.id] = Override(chord: chord)
        overrides = next
        persist()
    }

    /// Leave `action` with no key.
    func clear(_ action: DubAction) {
        guard action.isRemappable else { return }
        var next = overrides
        next[action.id] = Override(chord: nil)
        overrides = next
        persist()
    }

    /// Back to the defaults, everywhere.
    func reset() {
        overrides = [:]
        persist()
    }

    /// Whether the DJ changed `action` from its default.
    func isOverridden(_ action: DubAction) -> Bool {
        overrides[action.id] != nil
    }

    /// Whether anything differs from the defaults — what a reset would undo.
    var hasOverrides: Bool { !overrides.isEmpty }

    // MARK: - Resolution

    private func rebuild() {
        var table: [DubBinding] = []
        var seen = Set<String>()
        for binding in DubKeymap.defaults {
            seen.insert(binding.action.id)
            if let override = overrides[binding.action.id] {
                if let chord = override.chord {
                    table.append(Self.binding(binding.action, chord))
                }
                // `nil`: unbound on purpose — drop the default.
            } else {
                table.append(binding)
            }
        }
        // Actions with no default at all — the sampler, Quick Scratch,
        // the siren, the FX rack — exist only as overrides.
        for (id, override) in overrides.sorted(by: { $0.key < $1.key }) where !seen.contains(id) {
            guard let chord = override.chord, let action = DubAction(id: id) else { continue }
            table.append(Self.binding(action, chord))
        }
        resolved = table
        resolvedByAction = Dictionary(
            table.map { ($0.action, $0) }, uniquingKeysWith: { first, _ in first })
    }

    private static func binding(_ action: DubAction, _ chord: DubKeyChord) -> DubBinding {
        DubBinding(
            action: action, match: .code(chord.code),
            requiresCommand: chord.command, legend: chord.legend)
    }

    /// Whether a binding fires on `chord`. A `.character` default is
    /// compared on its printed glyph, which is what the chord's legend
    /// carries for a plain letter.
    private static func matches(_ binding: DubBinding, _ chord: DubKeyChord) -> Bool {
        guard binding.requiresCommand == chord.command else { return false }
        switch binding.match {
        case .code(let c): return c == chord.code
        case .character(let ch):
            return ch.caseInsensitiveCompare(chord.legend) == .orderedSame
        }
    }

    private func persist() {
        if overrides.isEmpty {
            defaults.removeObject(forKey: Self.key)
        } else if let data = try? JSONEncoder().encode(overrides) {
            defaults.set(data, forKey: Self.key)
        }
    }
}
