import AppKit
import Foundation

/// Sample lineage link-out (R-49, PRD §5.2.5a).
///
/// For this audience "what does it sample, what samples it" is not
/// trivia — knowing the break on the record in your hand is the one on
/// a record three crates over is how a set gets built. WhoSampled has
/// the best data and no public API, and their terms forbid scraping, so
/// an in-app integration would need a commercial agreement. A link-out
/// needs no key, breaks no terms, and with artist + title already in
/// hand from recognition it puts the DJ one click from the answer.
enum SampleLineage {
    /// Title for the menu item / button, in both places it appears.
    static let actionTitle = "Look Up Samples"

    static let helpText = "Search WhoSampled for this track's samples"

    /// WhoSampled's own search page for `artist title`, or `nil` when
    /// the track carries neither — an unnamed rip segment has nothing
    /// to search for, and a search for the empty string is a worse
    /// answer than a disabled control.
    static func whoSampledSearchURL(artist: String?, title: String?) -> URL? {
        let terms = [artist, title]
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        guard !terms.isEmpty else { return nil }

        // `.urlQueryAllowed` leaves `&`, `=`, `+` and `#` unescaped —
        // fine for a whole query string, wrong for one value inside it.
        // "Earth, Wind & Fire" would otherwise truncate at the ampersand.
        let allowed = CharacterSet.urlQueryAllowed
            .subtracting(CharacterSet(charactersIn: "&=+?#"))
        guard
            let query = terms
                .joined(separator: " ")
                .addingPercentEncoding(withAllowedCharacters: allowed)
        else {
            return nil
        }
        return URL(string: "https://www.whosampled.com/search/?q=\(query)")
    }

    /// Open the lookup in the user's browser. No-op when there is
    /// nothing to search for, so callers can wire it unconditionally
    /// and disable the control on `whoSampledSearchURL == nil`.
    static func lookUp(artist: String?, title: String?) {
        guard let url = whoSampledSearchURL(artist: artist, title: title) else { return }
        NSWorkspace.shared.open(url)
    }
}
