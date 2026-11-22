import Foundation

/// How long one deck has played the track it holds, to tell when the
/// track counts as played this session.
///
/// Thirty seconds of playing, added up: a track cued, checked and put
/// back is not played, and neither is one a crackling needle started a
/// few hundred times (rig, 2026-11-22). The count starts again with
/// each track the deck is given. Fed in Performance mode only.
struct PlayedTally: Equatable {
    static let playedAfterSecs: TimeInterval = 30
    /// A poll that took longer than this was a stall, not playing time.
    static let longestTickSecs: TimeInterval = 0.5

    private var trackId: String?
    private var secs: TimeInterval = 0
    private var counted = false

    /// One poll of the deck. Returns the track's id on the poll it
    /// reaches the threshold, and `nil` on every other.
    mutating func tick(trackId id: String?, playing: Bool, dt: TimeInterval) -> String? {
        if id != trackId {
            trackId = id
            secs = 0
            counted = false
        }
        guard let id, playing, !counted, dt > 0 else { return nil }
        secs += min(dt, Self.longestTickSecs)
        guard secs >= Self.playedAfterSecs else { return nil }
        counted = true
        return id
    }
}
