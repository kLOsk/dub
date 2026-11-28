// tap2 X Y GAP_MS — two clicks on one spot, the second GAP_MS after the
// first press (ECHO OUT's press-again-inside-the-beat).
import CoreGraphics
import Foundation

let a = CommandLine.arguments
guard a.count == 4, let x = Double(a[1]), let y = Double(a[2]), let gap = UInt32(a[3]) else {
    FileHandle.standardError.write("usage: tap2 X Y GAP_MS\n".data(using: .utf8)!)
    exit(2)
}
let p = CGPoint(x: x, y: y)
let src = CGEventSource(stateID: .hidSystemState)
func post(_ t: CGEventType) {
    CGEvent(mouseEventSource: src, mouseType: t, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
}
post(.mouseMoved); usleep(150_000)
post(.leftMouseDown); usleep(40_000); post(.leftMouseUp)
usleep(gap * 1000 > 40_000 ? gap * 1000 - 40_000 : 0)
post(.leftMouseDown); usleep(40_000); post(.leftMouseUp)
