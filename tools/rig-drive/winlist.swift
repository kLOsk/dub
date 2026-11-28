// winlist [OWNER] — on-screen windows: id, layer, owner, x, y, width, height,
// name. With OWNER only that app's. Window names need Screen Recording.
import CoreGraphics
import Foundation

let owner = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : nil
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
    as? [[String: Any]] ?? []
for w in list {
    let name = w[kCGWindowOwnerName as String] as? String ?? "?"
    if let owner, name != owner { continue }
    let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
    let width = b["Width"] as? Double ?? 0
    if width < 50 { continue }
    print(w[kCGWindowNumber as String] ?? 0, w[kCGWindowLayer as String] ?? 0, name,
          b["X"] ?? 0, b["Y"] ?? 0, width, b["Height"] ?? 0, w[kCGWindowName as String] ?? "")
}
