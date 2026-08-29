//
//  RipOverlayGeometryTests.swift
//  DubTests
//
//  The split clamp is the one piece of the rip overlay with real
//  off-by-one risk — a zero duration between review and the deck-A
//  decode landing, or inverted bounds off a salvaged manifest — so it
//  is a file-scope helper rather than a private view method, and it is
//  tested without rendering anything.
//

import XCTest

@testable import Dub

final class RipOverlayGeometryTests: XCTestCase {

    func test_noTrimClampsToTheWholeCapture() {
        XCTAssertEqual(RipTrimClamp.split(0, in: nil, duration: 100), 0)
        XCTAssertEqual(RipTrimClamp.split(50, in: nil, duration: 100), 50)
        XCTAssertEqual(RipTrimClamp.split(140, in: nil, duration: 100), 100)
        XCTAssertEqual(RipTrimClamp.split(-8, in: nil, duration: 100), 0)
    }

    func test_trimBoundsTheSplit() {
        let trim = RipTrimUi(startSecs: 10, endSecs: 80)
        XCTAssertEqual(RipTrimClamp.split(5, in: trim, duration: 100), 10)
        XCTAssertEqual(RipTrimClamp.split(45, in: trim, duration: 100), 45)
        XCTAssertEqual(RipTrimClamp.split(95, in: trim, duration: 100), 80)
    }

    /// Between entering review and the audition load landing, the
    /// overlay genuinely has no duration to work with.
    func test_zeroDurationDoesNotProduceNaNOrNegative() {
        XCTAssertEqual(RipTrimClamp.split(12, in: nil, duration: 0), 0)
        let trim = RipTrimUi(startSecs: 0, endSecs: 0)
        XCTAssertEqual(RipTrimClamp.split(12, in: trim, duration: 0), 0)
    }

    /// A salvaged manifest can claim a side that ends before it
    /// starts. Clamping must still land somewhere real.
    func test_invertedBoundsCollapseToTheStart() {
        let trim = RipTrimUi(startSecs: 90, endSecs: 20)
        let got = RipTrimClamp.split(50, in: trim, duration: 100)
        XCTAssertEqual(got, 90)
        XCTAssertFalse(got.isNaN)
    }
}
