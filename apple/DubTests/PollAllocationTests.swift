import Darwin
import DubCore
import XCTest

@testable import Dub

/// C-30: what the UI's engine reads allocate on the main thread. The
/// RT auditor covers the Rust side of these calls; nothing measured the
/// Swift side — the UniFFI lift of every record, the arrays and strings
/// inside them — and these run 10–60 times a second each.
///
/// Counted by `malloc_logger`, the hook malloc stack logging uses: every
/// allocation on the main thread while `body` runs, Rust's and Swift's
/// alike. The body runs synchronously, so the test host's own launch
/// work cannot land inside it.
final class PollAllocationTests: XCTestCase {

    private static let ticks = 200

    /// Main-thread allocations per call, on an engine with no audio
    /// device: the lift costs the same whether a deck plays or not.
    func testWhatEachEngineReadAllocates() {
        let engine = DubEngine()
        let reads: [(String, () -> Void)] = [
            ("positionSnapshot", { _ = engine.positionSnapshot(deckIdx: 0) }),
            ("deckTelemetry", { _ = engine.deckTelemetry(deckIdx: 0) }),
            ("peaksGeneration", { _ = engine.peaksGeneration(deckIdx: 0) }),
            ("thruTempo", { _ = engine.thruTempo(deckIdx: 0) }),
            ("samplerTelemetry", { _ = engine.samplerTelemetry() }),
        ]
        var report: [String] = []
        for (name, read) in reads {
            read()  // first-call setup is not the steady state
            let n = AllocationCount.mainThread { for _ in 0..<Self.ticks { read() } }
            let per = Double(n) / Double(Self.ticks)
            report.append("\(name) \(per)")
            XCTAssertLessThanOrEqual(per, Self.ceiling[name] ?? 0, "\(name) allocates more per call")
        }
        print("C-30 allocations per call:", report.joined(separator: ", "))
    }

    /// The model's 10 Hz deck poll, read for read (`pollDecksTimed`:
    /// both decks' position, peaks generation and telemetry, the
    /// diagnostics' telemetry again, the sampler), and the output meters'
    /// 30 Hz read. Per second, with no deck on THRU.
    func testThePollsPerSecond() {
        let engine = DubEngine()
        let deckPoll = {
            for idx in UInt64(0)..<2 {
                _ = engine.positionSnapshot(deckIdx: idx)
                _ = engine.peaksGeneration(deckIdx: idx)
                _ = engine.deckTelemetry(deckIdx: idx)
                _ = engine.deckTelemetry(deckIdx: idx)
            }
            _ = engine.samplerTelemetry()
        }
        let meters = OutputMeterPairView(frame: .zero)
        OutputMeterPair(engine: engine).configure(meters)
        deckPoll()
        meters.refresh()
        let poll = AllocationCount.mainThread { for _ in 0..<10 { deckPoll() } }
        let meter = AllocationCount.mainThread { for _ in 0..<30 { meters.refresh() } }
        print("C-30 allocations per second: deck poll \(poll), output meters \(meter)")
        // Debug 1 860 and 2 160; Release 510 and 510 — about a
        // thousand small blocks a second, a fraction of a millisecond
        // of malloc. Ten percent of room.
        XCTAssertLessThanOrEqual(poll, 2_050)
        XCTAssertLessThanOrEqual(meter, 2_400)
    }

    func testTheCounterCounts() {
        let n = AllocationCount.mainThread {
            for i in 0..<10 { _ = Unmanaged.passRetained(Box(i)).autorelease() }
        }
        XCTAssertGreaterThanOrEqual(n, 10)
        XCTAssertEqual(AllocationCount.mainThread {}, 0)
    }

    /// Measured 2026-11-15 (Xcode 16.2), two allocations of room each; a
    /// regression past these is a new array or string in a record.
    /// The suite runs Debug, where UniFFI's call glue is unoptimised:
    /// even the integer `peaksGeneration` allocates five times there and
    /// none in Release. Release, for the record: position 6, telemetry 8,
    /// peaks generation 0, THRU tempo 4, sampler 7.
    private static let ceiling: [String: Double] = [
        "positionSnapshot": 11 + 2,
        "deckTelemetry": 33 + 2,
        "peaksGeneration": 5 + 2,
        "thruTempo": 9 + 2,
        "samplerTelemetry": 52 + 2,
    ]
}

private final class Box {
    let v: Int
    init(_ v: Int) { self.v = v }
}

/// Main-thread allocations while `body` runs. Not thread-safe by
/// design: the hook counts only on the main thread, and only the main
/// thread reads or resets the count.
enum AllocationCount {
    private typealias Logger = @convention(c) (UInt32, UInt, UInt, UInt, UInt, UInt32) -> Void

    /// `MALLOC_LOG_TYPE_ALLOCATE` in libmalloc's `stack_logging.h`; a
    /// realloc reports it too.
    private static let allocate: UInt32 = 2

    nonisolated(unsafe) private static var count = 0

    private static let slot: UnsafeMutablePointer<Logger?>? = {
        guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "malloc_logger") else {
            return nil
        }
        return sym.assumingMemoryBound(to: Logger?.self)
    }()

    private static let hook: Logger = { type, _, _, _, _, _ in
        if type & AllocationCount.allocate != 0, pthread_main_np() != 0 {
            AllocationCount.count += 1
        }
    }

    static func mainThread(_ body: () -> Void) -> Int {
        precondition(Thread.isMainThread)
        guard let slot else {
            XCTFail("malloc_logger is not exported on this system")
            return 0
        }
        count = 0
        slot.pointee = hook
        body()
        slot.pointee = nil
        return count
    }
}
