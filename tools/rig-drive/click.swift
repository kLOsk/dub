// click X Y [HOLD_MS] — move to a point in screen points, press, hold, release.
// The rig checklist's robot and loop items drive Dub with it; see
// scripts/rig/lib.sh, which checks Dub is in front before every press.
import CoreGraphics
import Foundation

let a = CommandLine.arguments
guard a.count >= 3, let x = Double(a[1]), let y = Double(a[2]) else {
    FileHandle.standardError.write("usage: click X Y [HOLD_MS]\n".data(using: .utf8)!)
    exit(2)
}
let hold = a.count > 3 ? (UInt32(a[3]) ?? 60) : 60
let p = CGPoint(x: x, y: y)
let src = CGEventSource(stateID: .hidSystemState)
func post(_ t: CGEventType) {
    CGEvent(mouseEventSource: src, mouseType: t, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
}
post(.mouseMoved); usleep(150_000)
post(.leftMouseDown); usleep(hold * 1000)
post(.leftMouseUp)
