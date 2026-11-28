// type TEXT — types TEXT into whatever has keyboard focus, one character at a
// time, as key events (the rip review's title / artist / genre fields).
// `type --key return|tab|escape` presses that key instead.
import CoreGraphics
import Foundation

let a = CommandLine.arguments
guard a.count >= 2 else {
    FileHandle.standardError.write("usage: type TEXT | type --key return|tab|escape\n".data(using: .utf8)!)
    exit(2)
}
let src = CGEventSource(stateID: .hidSystemState)
if a[1] == "--key", a.count == 3 {
    let codes: [String: CGKeyCode] = ["return": 36, "tab": 48, "escape": 53]
    guard let code = codes[a[2]] else { exit(2) }
    CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: true)?.post(tap: .cghidEventTap)
    usleep(30_000)
    CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: false)?.post(tap: .cghidEventTap)
    exit(0)
}
for ch in a[1].utf16 {
    var c = ch
    for down in [true, false] {
        let e = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: down)
        e?.keyboardSetUnicodeString(stringLength: 1, unicodeString: &c)
        e?.post(tap: .cghidEventTap)
        usleep(15_000)
    }
    usleep(25_000)
}
