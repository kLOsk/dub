import Metal
import XCTest

/// `WaveformRenderer` no longer zero-fills its peak rings by hand — that
/// touched every page of tens of megabytes on the main thread whenever a
/// strip was built (~0.5 s each, at every SL3 connect). It relies on
/// `makeBuffer(length:options:)` returning zeroed memory, as Metal
/// documents; this holds the assumption to it on the machine that runs
/// the suite.
final class MetalBufferZeroFillTests: XCTestCase {
    func testANewSharedBufferIsZero() throws {
        guard let device = MTLCreateSystemDefaultDevice() else {
            throw XCTSkip("no Metal device")
        }
        let bytes = 8 << 20
        let buf = try XCTUnwrap(device.makeBuffer(length: bytes, options: .storageModeShared))
        let words = buf.contents().bindMemory(to: UInt64.self, capacity: bytes / 8)
        var nonZero = 0
        for i in stride(from: 0, to: bytes / 8, by: 97) where words[i] != 0 { nonZero += 1 }
        XCTAssertEqual(nonZero, 0)
    }
}
