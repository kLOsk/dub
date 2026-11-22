import Foundation

/// Whether a deck counts as playing when the master is chosen: its
/// play state, once it has stayed put for `holdSecs`.
///
/// A timecode deck "plays" whenever the decoder locks, and a cartridge
/// with one channel making poor contact locks and drops many times a
/// second. Each lock used to count as the deck starting, and the most
/// recently started deck is the master — so the master jumped between
/// the decks as fast as the contact crackled (rig, 2026-11-22). A deck
/// the DJ starts or stops from Dub is taken at its word (`force`).
struct SettledPlay: Equatable {
    static let holdSecs: TimeInterval = 0.6

    private(set) var isPlaying = false
    private var changeSeenAt: TimeInterval?

    /// Feed the deck's raw state. True at the moment it settles into
    /// playing, which is when it becomes the newest deck to start.
    mutating func update(playing: Bool, at now: TimeInterval) -> Bool {
        guard playing != isPlaying else {
            changeSeenAt = nil
            return false
        }
        guard let since = changeSeenAt else {
            changeSeenAt = now
            return false
        }
        guard now - since >= Self.holdSecs else { return false }
        isPlaying = playing
        changeSeenAt = nil
        return playing
    }

    mutating func force(_ playing: Bool) {
        isPlaying = playing
        changeSeenAt = nil
    }
}
