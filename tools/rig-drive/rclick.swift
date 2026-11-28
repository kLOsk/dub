// rclick X Y — a right click (a deck's BPM menu).
import CoreGraphics
import Foundation

let a = CommandLine.arguments
guard a.count == 3, let x = Double(a[1]), let y = Double(a[2]) else {
    FileHandle.standardError.write("usage: rclick X Y\n".data(using: .utf8)!)
    exit(2)
}
let p = CGPoint(x: x, y: y)
let src = CGEventSource(stateID: .hidSystemState)
func post(_ t: CGEventType) {
    CGEvent(mouseEventSource: src, mouseType: t, mouseCursorPosition: p, mouseButton: .right)?.post(tap: .cghidEventTap)
}
CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
usleep(150_000)
post(.rightMouseDown); usleep(60_000); post(.rightMouseUp)
