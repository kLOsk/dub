import XCTest

@testable import Dub

/// The repo's `.env`, read the way `dub recognize` reads it
/// (`recognize.rs`, the same four cases).
final class DevEnvironmentTests: XCTestCase {

    func testADotEnvLineGivesItsValue() {
        let values = DevEnvironment.parse("# keys for test runs\n\nOTHER=1\nDUB_ACOUSTID_KEY=abc123\n")
        XCTAssertEqual(values["DUB_ACOUSTID_KEY"], "abc123")
        XCTAssertEqual(values["OTHER"], "1")
    }

    func testExportQuotesSpacesAndTrailingCommentsAreDropped() {
        XCTAssertEqual(DevEnvironment.parse("export DUB_ACOUSTID_KEY = \"abc 123\"\n")["DUB_ACOUSTID_KEY"], "abc 123")
        XCTAssertEqual(DevEnvironment.parse("DUB_ACOUSTID_KEY='abc'")["DUB_ACOUSTID_KEY"], "abc")
        XCTAssertEqual(
            DevEnvironment.parse("DUB_ACOUSTID_KEY=abc # the application key\r\n")["DUB_ACOUSTID_KEY"], "abc")
    }

    /// `.env.example` copied and not yet filled in reads as no key, so
    /// Identify says to add one rather than AcoustID refusing "".
    func testAnEmptyOrCommentedOutKeyIsNoKey() {
        XCTAssertNil(DevEnvironment.parse("DUB_ACOUSTID_KEY=\n")["DUB_ACOUSTID_KEY"])
        XCTAssertNil(DevEnvironment.parse("DUB_ACOUSTID_KEY=\"\"\n")["DUB_ACOUSTID_KEY"])
        XCTAssertNil(DevEnvironment.parse("# DUB_ACOUSTID_KEY=abc\n")["DUB_ACOUSTID_KEY"])
        XCTAssertNil(DevEnvironment.parse("DUB_ACOUSTID_KEYS=abc\n")["DUB_ACOUSTID_KEY"])
        XCTAssertNil(DevEnvironment.parse("")["DUB_ACOUSTID_KEY"])
    }

    func testTheLastLineForAKeyWins() {
        XCTAssertEqual(DevEnvironment.parse("DUB_ACOUSTID_KEY=old\nDUB_ACOUSTID_KEY=new\n")["DUB_ACOUSTID_KEY"], "new")
    }
}
