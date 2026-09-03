import XCTest

@testable import Dub

/// R-49 — the WhoSampled link-out. The only thing with logic in it is
/// building the search URL, and the failure that matters is a query
/// that silently truncates: an unescaped `&` in "Earth, Wind & Fire"
/// would send the DJ to a search for "Earth, Wind".
final class SampleLineageTests: XCTestCase {

    private func url(_ artist: String?, _ title: String?) -> String? {
        SampleLineage.whoSampledSearchURL(artist: artist, title: title)?.absoluteString
    }

    func testSearchesArtistThenTitle() {
        XCTAssertEqual(
            url("Augustus Pablo", "King Tubby Meets Rockers Uptown"),
            "https://www.whosampled.com/search/?q=Augustus%20Pablo%20King%20Tubby%20Meets%20Rockers%20Uptown"
        )
    }

    func testSearchesWhicheverHalfIsPresent() {
        XCTAssertEqual(url(nil, "Apache"), "https://www.whosampled.com/search/?q=Apache")
        XCTAssertEqual(url("Incredible Bongo Band", nil),
                       "https://www.whosampled.com/search/?q=Incredible%20Bongo%20Band")
        XCTAssertEqual(url("", "Apache"), "https://www.whosampled.com/search/?q=Apache")
    }

    /// A rip segment the DJ has not named yet: no URL, so the control
    /// disables rather than opening a search for nothing.
    func testNothingToSearchForIsNil() {
        XCTAssertNil(url(nil, nil))
        XCTAssertNil(url("", ""))
        XCTAssertNil(url("   ", "\n"))
    }

    func testQuerySeparatorsAreEscaped() {
        let escaped = url("Earth, Wind & Fire", "Let's Groove")
        XCTAssertNotNil(escaped)
        XCTAssertFalse(
            escaped?.contains("&") ?? true,
            "an unescaped ampersand truncates the query at 'Earth, Wind'")
        XCTAssertTrue(escaped?.contains("%26") ?? false)

        // `+` reads as a space server-side, `#` starts a fragment.
        XCTAssertTrue(url("Sly + Robbie", "Boops")?.contains("%2B") ?? false)
        XCTAssertTrue(url(nil, "Track #1")?.contains("%23") ?? false)
    }

    func testNonAsciiSurvivesAsUtf8() {
        let escaped = url("Café Tacvba", nil)
        XCTAssertEqual(escaped, "https://www.whosampled.com/search/?q=Caf%C3%A9%20Tacvba")
    }

    func testWhitespaceIsTrimmedRatherThanSearchedFor() {
        XCTAssertEqual(url("  Pablo  ", "  Java "),
                       "https://www.whosampled.com/search/?q=Pablo%20Java")
    }
}
