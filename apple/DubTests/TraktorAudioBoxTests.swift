import XCTest

@testable import Dub

/// The request numbers are the box's protocol: a wrong one sets the
/// other deck, or the other switch, on hardware in the middle of a set.
final class TraktorAudioBoxTests: XCTestCase {
    func testEachSwitchHasItsOwnRequest() {
        let all = TraktorAudioSwitch.Kind.allCases.flatMap { kind in
            TraktorAudioSwitch.Channel.allCases.map { TraktorAudioSwitch(kind: kind, channel: $0) }
        }
        let requests = all.map { [UInt16($0.request), $0.index] }
        XCTAssertEqual(Set(requests).count, 4)
    }

    func testRequestsMatchTheBox() {
        let thruA = TraktorAudioSwitch(kind: .directThru, channel: .a)
        let phonoB = TraktorAudioSwitch(kind: .phono, channel: .b)
        XCTAssertEqual([thruA.request, phonoB.request], [0x01, 0x02])
        XCTAssertEqual([thruA.index, phonoB.index], [3, 5])
    }
}
