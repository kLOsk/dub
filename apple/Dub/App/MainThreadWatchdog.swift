//
//  MainThreadWatchdog.swift
//  Dub
//
//  Debug-only: reports main-thread stalls to the unified log.
//
//  A deck's waveform that stops for a second while its audio keeps
//  going is, nine times in ten, the main thread not getting round to
//  the SwiftUI update that starts that deck's render link — the audio
//  thread and the render threads do not need the main thread, the
//  view update does. This pings the main queue from a background
//  timer and logs any hop that took longer than a frame or two, with
//  the moment it began, so the stall can be lined up against the
//  timed sections in `toggleQuickScratch` and friends:
//
//      /usr/bin/log show --predicate 'subsystem == "com.dub.app" AND \
//          category == "stall"' --last 5m --style compact
//

#if DEBUG
import Darwin
import Foundation
import os

let dubStallLog = Logger(subsystem: "com.dub.app", category: "stall")

/// Starts once at launch; never stops. Debug builds only.
///
/// Two jobs. It logs every main-thread stall over `threshold` — and,
/// while a stall is still in progress, it captures the main thread's
/// stack so the log names what it was doing. The timed sections around
/// the obvious suspects were not enough: after the overview reload was
/// fixed the rig log still showed half-second stalls that none of them
/// covered. `/usr/bin/sample` was tried first and always arrived after
/// the stall had ended — it takes a second or two to attach — so the
/// capture is in-process: suspend the main thread for the microseconds
/// it takes to walk its frame pointers, resume, then symbolicate off
/// the critical path. Full stacks go to
/// `~/Library/Logs/Dub/stall-<ms>.txt`; the top frames go to the log.
enum MainThreadWatchdog {
    private static let queue = DispatchQueue(label: "com.dub.watchdog", qos: .utility)
    private static var timer: DispatchSourceTimer?
    /// Anything under this is scheduling noise, not a stall.
    private static let threshold: TimeInterval = 0.08
    /// A ping this late means a stall is under way: capture now, while
    /// the stack is still the one that matters.
    private static let captureAfter: TimeInterval = 0.15
    /// Send times of pings the main thread has not answered yet.
    /// Touched on `queue` only.
    private static var pending: [TimeInterval] = []
    /// The stall the last capture belonged to, and when it was last
    /// sampled: a long stall is sampled every `resampleEvery` so the
    /// file shows where the time went, not one instant of it.
    private static var capturedStall: TimeInterval = -1
    private static var lastCaptureUptime: TimeInterval = 0
    private static let resampleEvery: TimeInterval = 0.1
    /// When the last stall was reported. Every ping queued during one
    /// stall runs the moment the main thread frees, each with a smaller
    /// lag than the one before; only the first — the longest — is the
    /// stall, so the rest are dropped.
    private static var lastReportUptime: TimeInterval = 0
    private static var mainThread: MainThreadHandle?

    /// Call on the main thread, once.
    static func start() {
        guard timer == nil else { return }
        mainThread = MainThreadHandle()
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + 0.5, repeating: 0.05, leeway: .milliseconds(5))
        t.setEventHandler {
            let sent = ProcessInfo.processInfo.systemUptime
            pending.append(sent)
            if let oldest = pending.first, sent - oldest > captureAfter,
               capturedStall != oldest || sent - lastCaptureUptime >= resampleEvery
            {
                let first = capturedStall != oldest
                capturedStall = oldest
                lastCaptureUptime = sent
                captureMainThread(stalledSince: oldest, first: first)
            }
            DispatchQueue.main.async {
                let now = ProcessInfo.processInfo.systemUptime
                let lag = now - sent
                queue.async { pending.removeAll { $0 <= sent } }
                guard lag > threshold, now - lastReportUptime > 0.1 else { return }
                lastReportUptime = now
                dubStallLog.warning(
                    "main thread stalled \(Int(lag * 1000), privacy: .public) ms")
            }
        }
        t.resume()
        timer = t
    }

    private static func captureMainThread(stalledSince: TimeInterval, first: Bool) {
        guard let main = mainThread else { return }
        let pcs = main.backtrace()
        guard !pcs.isEmpty else {
            dubStallLog.error("stall capture: could not read the main thread")
            return
        }
        let frames = pcs.map(symbolicate)
        let dir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/Dub", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let file = dir.appendingPathComponent("stall-\(Int(stalledSince * 1000)).txt")
        let into = Int((ProcessInfo.processInfo.systemUptime - stalledSince) * 1000)
        let report = "--- main thread, \(into) ms into the stall\n"
            + frames.enumerated().map { "\($0.offset)  \($0.element)" }.joined(separator: "\n") + "\n"
        if first {
            try? report.write(to: file, atomically: true, encoding: .utf8)
        } else if let handle = try? FileHandle(forWritingTo: file) {
            handle.seekToEndOfFile()
            handle.write(Data(report.utf8))
            try? handle.close()
        }
        guard first else { return }
        // The interesting frames are the app's own, which sit under the
        // run loop; the log gets the top of the stack, the file all of it.
        let top = frames.prefix(14).joined(separator: " ← ")
        dubStallLog.warning(
            "stalled main thread stack → \(file.lastPathComponent, privacy: .public): \(top, privacy: .public)")
    }

    private static func symbolicate(_ pc: UInt) -> String {
        var info = Dl_info()
        guard dladdr(UnsafeRawPointer(bitPattern: pc), &info) != 0 else {
            return String(format: "0x%lx", pc)
        }
        let image = info.dli_fname.map { URL(fileURLWithPath: String(cString: $0)).lastPathComponent } ?? "?"
        guard let sym = info.dli_sname else {
            return "\(image) + \(pc - UInt(bitPattern: info.dli_fbase))"
        }
        let mangled = String(cString: sym)
        return "\(demangle(mangled)) (\(image))"
    }

    private typealias DemangleFn = @convention(c) (
        UnsafePointer<CChar>?, Int, UnsafeMutablePointer<CChar>?, UnsafeMutablePointer<Int>?, UInt32
    ) -> UnsafeMutablePointer<CChar>?

    private static let demangler: DemangleFn? = {
        guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "swift_demangle") else { return nil }
        return unsafeBitCast(sym, to: DemangleFn.self)
    }()

    private static func demangle(_ mangled: String) -> String {
        guard mangled.hasPrefix("$s") || mangled.hasPrefix("_$s"), let fn = demangler else { return mangled }
        return mangled.withCString { c -> String in
            guard let out = fn(c, strlen(c), nil, nil, 0) else { return mangled }
            defer { free(out) }
            return String(cString: out)
        }
    }
}

/// The main thread's Mach port and stack bounds, taken on the main
/// thread at start so the watchdog can read it from elsewhere.
private struct MainThreadHandle {
    let port: thread_t
    let stackTop: UInt
    let stackBottom: UInt

    init() {
        port = pthread_mach_thread_np(pthread_self())
        let top = UInt(bitPattern: pthread_get_stackaddr_np(pthread_self()))
        stackTop = top
        stackBottom = top - UInt(pthread_get_stacksize_np(pthread_self()))
    }

    /// Program counters, innermost first. Suspends the thread only for
    /// the register read and the frame walk; both are a few
    /// microseconds. Empty if the thread state could not be read.
    ///
    /// **Nothing may allocate while the thread is suspended.** It is
    /// stopped wherever it happens to be, and one time in a few that
    /// is inside `malloc` holding the zone lock. This used to `append`
    /// to an empty array right after `thread_suspend`; when the
    /// suspend landed inside malloc, that append waited for a lock the
    /// frozen thread could never release, the `thread_resume` never
    /// ran, and every other thread queued up behind the same lock —
    /// the whole app stuck at 0 % CPU (2026-09-16, a 2 s `sample`
    /// showed the main thread parked in `tiny_free_list_add_ptr` and
    /// the watchdog in `swift_slowAlloc`). So the buffer is allocated
    /// first, the walk writes into it by index, and the slice that is
    /// returned is built only after the resume.
    func backtrace(maxFrames: Int = 96) -> [UInt] {
        var pcs = [UInt](repeating: 0, count: maxFrames)
        let n = pcs.withUnsafeMutableBufferPointer { walk(into: $0) }
        return Array(pcs.prefix(n))
    }

    /// The suspended section: a syscall for the registers, then raw
    /// reads of the stack. Returns the number of frames written.
    private func walk(into pcs: UnsafeMutableBufferPointer<UInt>) -> Int {
        guard thread_suspend(port) == KERN_SUCCESS else { return 0 }
        defer { thread_resume(port) }
        var n = 0
        #if arch(x86_64)
        var state = x86_thread_state64_t()
        var count = mach_msg_type_number_t(MemoryLayout<x86_thread_state64_t>.size / MemoryLayout<natural_t>.size)
        let kr = withUnsafeMutablePointer(to: &state) {
            $0.withMemoryRebound(to: natural_t.self, capacity: Int(count)) {
                thread_get_state(port, thread_state_flavor_t(x86_THREAD_STATE64), $0, &count)
            }
        }
        guard kr == KERN_SUCCESS else { return 0 }
        pcs[n] = UInt(state.__rip)
        n += 1
        var fp = UInt(state.__rbp)
        #elseif arch(arm64)
        var state = arm_thread_state64_t()
        var count = mach_msg_type_number_t(MemoryLayout<arm_thread_state64_t>.size / MemoryLayout<natural_t>.size)
        let kr = withUnsafeMutablePointer(to: &state) {
            $0.withMemoryRebound(to: natural_t.self, capacity: Int(count)) {
                thread_get_state(port, thread_state_flavor_t(ARM_THREAD_STATE64), $0, &count)
            }
        }
        guard kr == KERN_SUCCESS else { return 0 }
        pcs[n] = UInt(state.__pc)
        n += 1
        if n < pcs.count {
            pcs[n] = UInt(state.__lr)
            n += 1
        }
        var fp = UInt(state.__fp)
        #else
        return 0
        #endif
        // Frame-pointer walk: [fp] is the caller's fp, [fp + 8] its
        // return address. Bounded to the main thread's own stack so a
        // frameless leaf cannot send this reading garbage.
        while n < pcs.count, fp >= stackBottom, fp + 16 <= stackTop, fp % 8 == 0 {
            let next = UnsafePointer<UInt>(bitPattern: fp)?.pointee ?? 0
            let ret = UnsafePointer<UInt>(bitPattern: fp + 8)?.pointee ?? 0
            guard ret != 0 else { break }
            pcs[n] = ret
            n += 1
            guard next > fp else { break }
            fp = next
        }
        return n
    }
}

/// Time a main-thread section and log it when it is long enough to
/// matter. Zero cost when it is not.
@discardableResult
func stallTimed<T>(_ label: StaticString, threshold: TimeInterval = 0.02, _ body: () throws -> T) rethrows -> T {
    let start = ProcessInfo.processInfo.systemUptime
    defer {
        let took = ProcessInfo.processInfo.systemUptime - start
        if took > threshold {
            dubStallLog.warning(
                "\(label, privacy: .public) took \(Int(took * 1000), privacy: .public) ms")
        }
    }
    return try body()
}
#else
@inline(__always)
func stallTimed<T>(_ label: StaticString, threshold: TimeInterval = 0.02, _ body: () throws -> T) rethrows -> T {
    try body()
}
#endif
