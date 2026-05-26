import AppKit

let path = "/Users/klos/Development/dub/apple/Dub/Assets.xcassets/AboutSplash.imageset/AboutSplash.png"
guard let img = NSImage(contentsOfFile: path),
      let tiff = img.tiffRepresentation,
      let rep = NSBitmapImageRep(data: tiff)
else {
    fatalError("load failed")
}
let w = rep.pixelsWide
let h = rep.pixelsHigh
print("size: \(w)x\(h)")

func rgb(_ x: Int, _ y: Int) -> (Int, Int, Int) {
    guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { return (0, 0, 0) }
    return (
        Int(round(c.redComponent * 255)),
        Int(round(c.greenComponent * 255)),
        Int(round(c.blueComponent * 255))
    )
}

func stats(_ name: String, coords: [(Int, Int)]) {
    var rs = [Int](), gs = [Int](), bs = [Int]()
    for (x, y) in coords {
        let (r, g, b) = rgb(x, y)
        rs.append(r)
        gs.append(g)
        bs.append(b)
    }
    let avg = (rs.reduce(0, +) / rs.count, gs.reduce(0, +) / gs.count, bs.reduce(0, +) / bs.count)
    print("\(name): avg=\(avg) #\(String(format: "%02X%02X%02X", avg.0, avg.1, avg.2)) n=\(coords.count)")
}

stats("TL", coords: (0 ..< 20).flatMap { x in (0 ..< 20).map { y in (x, y) } })
stats("TR", coords: ((w - 20) ..< w).flatMap { x in (0 ..< 20).map { y in (x, y) } })
stats("BL", coords: (0 ..< 20).flatMap { x in ((h - 20) ..< h).map { y in (x, y) } })
stats("BR", coords: ((w - 20) ..< w).flatMap { x in ((h - 20) ..< h).map { y in (x, y) } })
stats("top", coords: stride(from: 0, to: w, by: max(1, w / 80)).map { x in (x, 0) })
stats("bottom", coords: stride(from: 0, to: w, by: max(1, w / 80)).map { x in (x, h - 1) })
stats("left", coords: stride(from: 0, to: h, by: max(1, h / 80)).map { y in (0, y) })
stats("right", coords: stride(from: 0, to: h, by: max(1, h / 80)).map { y in (w - 1, y) })
stats(
    "center-bg",
    coords: stride(from: w / 4, to: 3 * w / 4, by: 15).flatMap { x in
        stride(from: h / 4, to: h / 2, by: 15).map { y in (x, y) }
    })
print("Current surface0: #0B0C0F rgb(11,12,15)")
