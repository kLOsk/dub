// drag X1 Y1 X2 Y2 — press at one point, move in steps, release at the other
// (a library row onto a deck).
import CoreGraphics
import Foundation

let a = CommandLine.arguments.dropFirst().compactMap(Double.init)
guard a.count == 4 else {
    FileHandle.standardError.write("usage: drag X1 Y1 X2 Y2\n".data(using: .utf8)!)
    exit(2)
}
let from = CGPoint(x: a[0], y: a[1]), to = CGPoint(x: a[2], y: a[3])
let src = CGEventSource(stateID: .hidSystemState)
func post(_ t: CGEventType, _ p: CGPoint) {
    CGEvent(mouseEventSource: src, mouseType: t, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
}
post(.mouseMoved, from); usleep(200_000)
post(.leftMouseDown, from); usleep(300_000)
for i in 1...40 {
    let f = Double(i) / 40
    post(.leftMouseDragged, CGPoint(x: from.x + (to.x - from.x) * f, y: from.y + (to.y - from.y) * f))
    usleep(20_000)
}
usleep(400_000)
post(.leftMouseUp, to)
