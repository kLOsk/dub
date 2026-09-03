//
//  SplitMetricsTests.swift
//  DubTests
//

@testable import Dub
import XCTest

final class SplitMetricsTests: XCTestCase {

    private let chrome = DubLayout.rackBarHeight + 1
    private let deckMin = DubLayout.waveformMinHeight

    private func makeDefaults(_ name: String = #function) -> UserDefaults {
        let defaults = UserDefaults(suiteName: "SplitMetricsTests.\(name)")!
        defaults.removePersistentDomain(forName: "SplitMetricsTests.\(name)")
        return defaults
    }

    // MARK: - Defaults

    func test_defaults_areDeckHeavyInPerformance_libraryHeavyInPrep() {
        // PRD §9.2: the decks dominate vertical real estate
        // intentionally. Prep is a browsing surface and leans back.
        XCTAssertGreaterThan(SplitMetrics.defaultFraction(.timecode), 0.5)
        XCTAssertLessThan(SplitMetrics.defaultFraction(.prep), 0.5)
    }

    func test_perModeKeysAreDistinct() {
        XCTAssertNotEqual(SplitMetrics.key(.timecode), SplitMetrics.key(.prep))
    }

    // MARK: - Clamping

    /// At a normal window both halves keep their floor wherever the
    /// divider is dragged.
    func test_clampsToBothMinima_atAWorkingWindowSize() {
        let total: CGFloat = 900 - 138
        for fraction in stride(from: -0.5, through: 1.5, by: 0.1) {
            let deck = SplitMetrics.deckHeight(
                fraction: CGFloat(fraction), total: total,
                deckChrome: chrome, deckMinimum: deckMin)
            XCTAssertGreaterThanOrEqual(deck, deckMin + chrome - 0.5,
                                        "deck fell below its floor at \(fraction)")
            XCTAssertGreaterThanOrEqual(
                total - deck, DubLayout.libraryMinHeight - 0.5,
                "library fell below its floor at \(fraction)")
        }
    }

    /// The property that makes silent occlusion impossible: whatever
    /// the inputs, the two halves add up to exactly what there is.
    func test_halvesAlwaysSumToTheAvailableSpace() {
        for total in [CGFloat(0), 50, 244, 364, 664, 878, 4000] {
            for fraction in [CGFloat(-1), 0, 0.4, 0.6, 1, 2] {
                let deck = SplitMetrics.deckHeight(
                    fraction: fraction, total: total,
                    deckChrome: chrome, deckMinimum: deckMin)
                XCTAssertGreaterThanOrEqual(deck, 0)
                XCTAssertLessThanOrEqual(deck, total + 0.5,
                                         "deck exceeds total at \(total)/\(fraction)")
            }
        }
    }

    /// A 600 pt window has 364 pt to divide and the two floors want
    /// 480. They soften rather than overflowing — an explicit
    /// degradation instead of SwiftUI drawing past the edge.
    func test_softensBothMinima_whenTheyCannotBothBeSatisfied() {
        let total: CGFloat = 364
        let deck = SplitMetrics.deckHeight(
            fraction: 0.6, total: total, deckChrome: chrome, deckMinimum: deckMin)
        XCTAssertLessThanOrEqual(deck, total)
        XCTAssertGreaterThan(total - deck, 0, "library squeezed out entirely")
    }

    func test_degenerateTotals() {
        for total in [CGFloat(0), -10, .nan, .infinity] {
            let deck = SplitMetrics.deckHeight(
                fraction: 0.6, total: total,
                deckChrome: chrome, deckMinimum: deckMin)
            XCTAssertTrue(deck.isFinite)
            XCTAssertGreaterThanOrEqual(deck, 0)
        }
    }

    func test_nonFiniteFractionDoesNotProduceANonFiniteHeight() {
        let deck = SplitMetrics.deckHeight(
            fraction: .nan, total: 700, deckChrome: chrome, deckMinimum: deckMin)
        XCTAssertTrue(deck.isFinite)
    }

    // MARK: - Persistence

    func test_roundTrip() {
        let defaults = makeDefaults()
        SplitMetrics.save(0.42, .timecode, to: defaults)
        XCTAssertEqual(SplitMetrics.load(.timecode, from: defaults), 0.42, accuracy: 0.001)
    }

    func test_missingValueFallsBackToTheModeDefault() {
        let defaults = makeDefaults()
        XCTAssertEqual(SplitMetrics.load(.prep, from: defaults),
                       SplitMetrics.defaultFraction(.prep), accuracy: 0.001)
    }

    /// A corrupt or out-of-range persisted value must not be able to
    /// collapse a pane on the next launch.
    func test_corruptValuesFallBackToTheModeDefault() {
        let defaults = makeDefaults()
        for bad in [Double.nan, 2.5, -1, 0.0, 1.0, 0.01, 0.99] {
            defaults.set(bad, forKey: SplitMetrics.key(.timecode))
            XCTAssertEqual(
                SplitMetrics.load(.timecode, from: defaults),
                SplitMetrics.defaultFraction(.timecode), accuracy: 0.001,
                "accepted \(bad)")
        }
    }

    func test_modesPersistIndependently() {
        let defaults = makeDefaults()
        SplitMetrics.save(0.75, .timecode, to: defaults)
        SplitMetrics.save(0.25, .prep, to: defaults)
        XCTAssertEqual(SplitMetrics.load(.timecode, from: defaults), 0.75, accuracy: 0.001)
        XCTAssertEqual(SplitMetrics.load(.prep, from: defaults), 0.25, accuracy: 0.001)
    }
}
