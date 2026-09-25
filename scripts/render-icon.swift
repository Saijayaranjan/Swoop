// Renders the Swoop app icon (original artwork): a white bird in a vertical dive whose swept-back
// wings form a download arrow, above a soft blue water line, on a deep indigo glass tile.
//
// Usage:
//   swift scripts/render-icon.swift [<out-dir>] [--web <dir>] [--extension <dir>] [--glyph <dir>]
//                                   [--sheet <file.png>]
//
//   <out-dir>      Swoop.icns, Swoop-1024.png and Swoop.iconset/ (default: apps/macos/Resources)
//   --web          favicon-32.png, favicon-64.png, apple-touch-icon.png (180), icon-512.png
//   --extension    icon16.png, icon32.png, icon48.png, icon128.png (browser extension toolbar)
//   --glyph        glyph.svg, glyph-inline.txt and SwoopGlyph.swift: the monochrome brand glyph
//   --sheet        a contact sheet of every size on light and dark backgrounds, for review
//
// All artwork is defined once below in "tile units": a 100 × 100 square with y pointing down,
// mapped onto the 824 pt squircle of the 1024 pt macOS icon grid. Small sizes use simplified,
// bolder variants (no disc, no second ripple, no shading) so they stay crisp.
import AppKit
import CoreGraphics
import Foundation

// MARK: - Command line

var outDir = "apps/macos/Resources"
var webDir: String?
var extDir: String?
var glyphDir: String?
var sheetPath: String?
do {
    var args = Array(CommandLine.arguments.dropFirst())
    while !args.isEmpty {
        let a = args.removeFirst()
        func value() -> String {
            guard !args.isEmpty else { FileHandle.standardError.write("missing value for \(a)\n".data(using: .utf8)!); exit(2) }
            return args.removeFirst()
        }
        switch a {
        case "--web": webDir = value()
        case "--extension": extDir = value()
        case "--glyph": glyphDir = value()
        case "--sheet": sheetPath = value()
        default: outDir = a
        }
    }
}

// MARK: - Palette

func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: a)
}
let srgb = CGColorSpace(name: CGColorSpace.sRGB)!
func gradient(_ stops: [(CGColor, CGFloat)]) -> CGGradient {
    CGGradient(colorsSpace: srgb, colors: stops.map { $0.0 } as CFArray, locations: stops.map { $0.1 })!
}

let tileTop = rgb(0x2E2A7E), tileBottom = rgb(0x1A1848)
let discTop = rgb(0x3A3698), discBottom = rgb(0x2B2774)
let waterLight = rgb(0x9CCBFF), waterDeep = rgb(0x6BA3FF)

// MARK: - Geometry (tile units, y down)

/// A tiny path description that renders to CoreGraphics, SVG and SwiftUI from one source.
enum Seg { case m(CGPoint), l(CGPoint), q(CGPoint, CGPoint), c(CGPoint, CGPoint, CGPoint), z }
func P(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: x, y: y) }
func mirror(_ p: CGPoint) -> CGPoint { CGPoint(x: 100 - p.x, y: p.y) }

func cgPath(_ segs: [Seg], _ t: CGAffineTransform = .identity) -> CGPath {
    let p = CGMutablePath()
    for s in segs {
        switch s {
        case .m(let a): p.move(to: a, transform: t)
        case .l(let a): p.addLine(to: a, transform: t)
        case .q(let c, let a): p.addQuadCurve(to: a, control: c, transform: t)
        case .c(let c1, let c2, let a): p.addCurve(to: a, control1: c1, control2: c2, transform: t)
        case .z: p.closeSubpath()
        }
    }
    return p
}

func segs(of path: CGPath) -> [Seg] {
    var out: [Seg] = []
    path.applyWithBlock { e in
        let pts = e.pointee.points
        switch e.pointee.type {
        case .moveToPoint: out.append(.m(pts[0]))
        case .addLineToPoint: out.append(.l(pts[0]))
        case .addQuadCurveToPoint: out.append(.q(pts[0], pts[1]))
        case .addCurveToPoint: out.append(.c(pts[0], pts[1], pts[2]))
        case .closeSubpath: out.append(.z)
        @unknown default: break
        }
    }
    return out
}

// The diving bird. Head (the arrow's point) at the bottom; the wings sweep back and up into a
// chevron; a slim body rises from between the wings into a small forked tail.
let head = P(50, 72)
let tipR = P(85, 27)
let shoulderR = P(54.6, 44)       // where the right wing's trailing edge meets the body
let neckR = P(52.7, 24.5)         // the body tapers from the shoulders to the base of the tail
let tailR = P(56.2, 14.2)
let tailNotch = P(50, 18.6)

/// Right half of the silhouette from the head up to the tail notch; the left half is its mirror.
let halfOutline: [Seg] = [
    // leading edge: off the head in a long sweep, lifting slightly into the wing tip
    .c(P(57.6, 63.6), P(70.5, 45), tipR),
    // trailing edge: a shallow curve back to the shoulder, so the wing is broadest at its root
    .c(P(74.5, 33.6), P(62.5, 40.8), shoulderR),
    // body tapering up to the tail, which flares into a small fork
    .c(P(53.9, 37), P(53.1, 30), neckR),
    .c(P(53.4, 20.5), P(54.8, 16.8), tailR),
    .q(P(52.6, 15.4), tailNotch),
]

func mirrored(_ s: Seg) -> Seg {
    switch s {
    case .m(let a): return .m(mirror(a))
    case .l(let a): return .l(mirror(a))
    case .q(let c, let a): return .q(mirror(c), mirror(a))
    case .c(let c1, let c2, let a): return .c(mirror(c1), mirror(c2), mirror(a))
    case .z: return .z
    }
}

/// Reverses a chain of segments that starts at `start`, so the mirrored half can be walked back.
func reversed(_ chain: [Seg], from start: CGPoint) -> [Seg] {
    var points = [start]
    for s in chain {
        switch s {
        case .l(let a), .m(let a): points.append(a)
        case .q(_, let a), .c(_, _, let a): points.append(a)
        case .z: break
        }
    }
    var out: [Seg] = []
    for (i, s) in chain.enumerated().reversed() {
        let to = points[i]
        switch s {
        case .l: out.append(.l(to))
        case .q(let c, _): out.append(.q(c, to))
        case .c(let c1, let c2, _): out.append(.c(c2, c1, to))
        default: break
        }
    }
    return out
}

let birdSegs: [Seg] = [.m(head)] + halfOutline + reversed(halfOutline.map(mirrored), from: head) + [.z]

/// The water line: a gentle S-wave beneath the head.
let waterSegs: [Seg] = [.m(P(27, 83.2)), .c(P(35, 77.6), P(42.5, 77.8), P(50, 82.2)),
                        .c(P(57.5, 86.6), P(65, 86.8), P(73, 81.2))]
let rippleSegs: [Seg] = [.m(P(40, 91)), .c(P(43.5, 88.8), P(47, 88.9), P(50, 90.4)),
                         .c(P(53, 91.9), P(56.5, 92), P(60, 89.8))]

/// The right half of the bird sits in a soft shade, as if lit from the upper left, so the body and
/// wings read as folded along the centre crease.
let foldSegs: [Seg] = [.m(P(50, 0)), .l(P(100, 0)), .l(P(100, 100)), .l(P(50, 100)), .z]

// MARK: - Squircle (continuous corners)

/// A rounded rect with continuous ("squircle") corners, the shape of the macOS icon grid.
func squircle(_ r: CGRect, radius rad: CGFloat) -> CGPath {
    let p = CGMutablePath()
    let k: [CGFloat] = [1.52866483, 1.08849299, 0.86840701, 0.66993427, 0.06549600, 0.37282392, 0.19262949]
    let (x0, y0, x1, y1) = (r.minX, r.minY, r.maxX, r.maxY)
    func v(_ i: Int) -> CGFloat { k[i] * rad }
    p.move(to: P(x0 + v(0), y0))
    p.addLine(to: P(x1 - v(0), y0))
    p.addCurve(to: P(x1 - v(3), y0 + v(4)), control1: P(x1 - v(1), y0), control2: P(x1 - v(2), y0))
    p.addCurve(to: P(x1 - v(4), y0 + v(3)), control1: P(x1 - v(5), y0 + v(6)), control2: P(x1 - v(6), y0 + v(5)))
    p.addCurve(to: P(x1, y0 + v(0)), control1: P(x1, y0 + v(2)), control2: P(x1, y0 + v(1)))
    p.addLine(to: P(x1, y1 - v(0)))
    p.addCurve(to: P(x1 - v(4), y1 - v(3)), control1: P(x1, y1 - v(1)), control2: P(x1, y1 - v(2)))
    p.addCurve(to: P(x1 - v(3), y1 - v(4)), control1: P(x1 - v(6), y1 - v(5)), control2: P(x1 - v(5), y1 - v(6)))
    p.addCurve(to: P(x1 - v(0), y1), control1: P(x1 - v(2), y1), control2: P(x1 - v(1), y1))
    p.addLine(to: P(x0 + v(0), y1))
    p.addCurve(to: P(x0 + v(3), y1 - v(4)), control1: P(x0 + v(1), y1), control2: P(x0 + v(2), y1))
    p.addCurve(to: P(x0 + v(4), y1 - v(3)), control1: P(x0 + v(5), y1 - v(6)), control2: P(x0 + v(6), y1 - v(5)))
    p.addCurve(to: P(x0, y1 - v(0)), control1: P(x0, y1 - v(2)), control2: P(x0, y1 - v(1)))
    p.addLine(to: P(x0, y0 + v(0)))
    p.addCurve(to: P(x0 + v(4), y0 + v(3)), control1: P(x0, y0 + v(1)), control2: P(x0, y0 + v(2)))
    p.addCurve(to: P(x0 + v(3), y0 + v(4)), control1: P(x0 + v(6), y0 + v(5)), control2: P(x0 + v(5), y0 + v(6)))
    p.addCurve(to: P(x0 + v(0), y0), control1: P(x0 + v(2), y0), control2: P(x0 + v(1), y0))
    p.closeSubpath()
    return p
}

// MARK: - Rendering

/// How much detail a rendering carries, chosen from the tile's size in pixels.
struct Detail {
    var disc: Bool, ripple: Bool, fold: Bool, glass: Bool, glyphShadow: Bool
    var glyphScale: CGFloat     // enlarges the mark on tiny tiles
    var bold: CGFloat           // extra outline weight for the bird, tile units
    var water: CGFloat          // water stroke width, tile units
    var waterLift: CGFloat      // moves the water line up (tile units) as the mark grows
    var pixelFit = false        // tiny tiles: a 2 px shaft and a flat, 1 px water line on the pixel grid

    static func forTile(_ px: CGFloat) -> Detail {
        switch px {
        case ..<20: return Detail(disc: false, ripple: false, fold: false, glass: false, glyphShadow: false,
                                  glyphScale: 1.14, bold: 3.4, water: 8.4, waterLift: 3.2, pixelFit: true)
        case ..<40: return Detail(disc: false, ripple: false, fold: false, glass: true, glyphShadow: false,
                                  glyphScale: 1.10, bold: 2.0, water: 6.2, waterLift: 2.2)
        case ..<90: return Detail(disc: true, ripple: false, fold: true, glass: true, glyphShadow: true,
                                  glyphScale: 1.04, bold: 0.8, water: 4.4, waterLift: 0.8)
        default: return Detail(disc: true, ripple: true, fold: true, glass: true, glyphShadow: true,
                               glyphScale: 1, bold: 0, water: 3.3, waterLift: 0)
        }
    }
}

enum Style {
    case macOS          // 824/1024 squircle with margins and drop shadow
    case fullBleed      // squircle filling the canvas (favicons, extension toolbar)
    case square         // opaque full square (apple-touch-icon; the OS applies its own mask)
}

func makeContext(_ px: Int) -> CGContext {
    let ctx = CGContext(data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0, space: srgb,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.interpolationQuality = .high
    ctx.setShouldAntialias(true)
    return ctx
}

func drawIcon(_ ctx: CGContext, px: Int, style: Style) {
    let s = CGFloat(px)
    let tileRect: CGRect
    switch style {
    case .macOS: tileRect = CGRect(x: s * 100 / 1024, y: s * 100 / 1024, width: s * 824 / 1024, height: s * 824 / 1024)
    case .fullBleed, .square: tileRect = CGRect(x: 0, y: 0, width: s, height: s)
    }
    let u = tileRect.width / 100            // pixels per tile unit
    let d = Detail.forTile(tileRect.width)

    ctx.saveGState()
    // Flip to y-down so the artwork's coordinates read like SVG.
    ctx.translateBy(x: 0, y: s)
    ctx.scaleBy(x: 1, y: -1)

    let tile: CGPath = style == .square ? CGPath(rect: tileRect, transform: nil)
                                        : squircle(tileRect, radius: tileRect.width * 0.2237)
    // Drop shadow (macOS grid only): a soft contact shadow under the tile.
    if style == .macOS {
        ctx.saveGState()
        ctx.setShadow(offset: CGSize(width: 0, height: -s * 10 / 1024), blur: s * 22 / 1024, color: rgb(0x000000, 0.32))
        ctx.addPath(tile)
        ctx.setFillColor(tileBottom)
        ctx.fillPath()
        ctx.restoreGState()
    }

    ctx.saveGState()
    ctx.addPath(tile)
    ctx.clip()
    // Tile: a gentle top-to-bottom indigo gradient.
    ctx.drawLinearGradient(gradient([(tileTop, 0), (tileBottom, 1)]),
                           start: P(0, tileRect.minY), end: P(0, tileRect.maxY), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])

    // Everything else is drawn in tile units.
    ctx.translateBy(x: tileRect.minX, y: tileRect.minY)
    ctx.scaleBy(x: u, y: u)

    if d.disc {
        let disc = CGPath(ellipseIn: CGRect(x: 50 - 31, y: 46 - 31, width: 62, height: 62), transform: nil)
        ctx.saveGState()
        ctx.addPath(disc)
        ctx.clip()
        ctx.drawLinearGradient(gradient([(discTop, 0), (discBottom, 1)]), start: P(0, 10), end: P(0, 78), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
        ctx.restoreGState()
    }

    // Glass: a soft sheen across the top half.
    if d.glass {
        ctx.drawLinearGradient(gradient([(rgb(0xFFFFFF, 0.07), 0), (rgb(0xFFFFFF, 0.0), 0.5)]),
                               start: P(0, 0), end: P(0, 100), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
    }

    // The mark, enlarged around the tile centre on tiny sizes.
    ctx.saveGState()
    ctx.translateBy(x: 50, y: 50)
    ctx.scaleBy(x: d.glyphScale, y: d.glyphScale)
    ctx.translateBy(x: -50, y: -52)   // the mark's optical centre sits ~2 units below the tile's

    let bird = cgPath(birdSegs)
    var waterMove = CGAffineTransform(translationX: 0, y: -d.waterLift)
    var waterWidth = d.water
    if d.pixelFit {
        // Flatten the wave to a third of its height and land its centre line on a pixel centre.
        let mid: CGFloat = 82.2, pxPerUnit = u * d.glyphScale
        let py = tileRect.minY + u * (50 + d.glyphScale * (mid - d.waterLift - 52))
        let snapped = (py - 0.5).rounded() + 0.5
        waterMove = CGAffineTransform(translationX: 0, y: mid - d.waterLift + (snapped - py) / pxPerUnit)
            .scaledBy(x: 1, y: 0.35).translatedBy(x: 0, y: -mid)
        waterWidth = 1.0 / pxPerUnit
    }
    let water = cgPath(waterSegs, waterMove)
    let waterOutline = water.copy(strokingWithWidth: waterWidth, lineCap: .round, lineJoin: .round, miterLimit: 4)

    // Water line.
    ctx.saveGState()
    ctx.addPath(waterOutline)
    ctx.clip()
    ctx.drawLinearGradient(gradient([(waterLight, 0), (waterDeep, 1)]), start: P(27, 0), end: P(73, 0), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
    ctx.restoreGState()
    if d.ripple {
        ctx.addPath(cgPath(rippleSegs))
        ctx.setLineWidth(2.2)
        ctx.setLineCap(.round)
        ctx.setStrokeColor(rgb(0x7DB4FF, 0.32))
        ctx.strokePath()
    }

    // Bird: white with a soft contact shadow, then the folded-wing shading.
    ctx.saveGState()
    if d.glyphShadow {
        ctx.setShadow(offset: CGSize(width: 0, height: -tileRect.width * 0.012), blur: tileRect.width * 0.035,
                      color: rgb(0x0B0930, 0.45))
    }
    if d.pixelFit {
        // a shaft exactly 2 px wide, centred on the pixel boundary at the tile's middle
        let w = 2 / (u * d.glyphScale)
        ctx.setFillColor(rgb(0xFFFFFF))
        ctx.fill(CGRect(x: 50 - w / 2, y: 24, width: w, height: 34))
    }
    ctx.addPath(bird)
    ctx.setFillColor(rgb(0xFFFFFF))
    if d.bold > 0 {
        ctx.setLineWidth(d.bold)
        ctx.setLineJoin(.round)
        ctx.setStrokeColor(rgb(0xFFFFFF))
        ctx.drawPath(using: .fillStroke)
    } else {
        ctx.fillPath()
    }
    ctx.restoreGState()

    if d.fold {
        ctx.saveGState()
        ctx.addPath(bird)
        ctx.clip()
        // a faint cool tint that deepens toward the tail, then the shaded half
        ctx.drawLinearGradient(gradient([(rgb(0x2A2770, 0.0), 0), (rgb(0x2A2770, 0.07), 1)]),
                               start: P(0, 60), end: P(0, 15), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
        ctx.addPath(cgPath(foldSegs))
        ctx.clip()
        ctx.drawLinearGradient(gradient([(rgb(0x2A2770, 0.08), 0), (rgb(0x2A2770, 0.15), 1)]),
                               start: P(0, 70), end: P(0, 25), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
        ctx.restoreGState()
    }
    ctx.restoreGState()   // glyph transform

    // Glass rim: a fine inner highlight, brightest along the top edge.
    if d.glass && style != .square {
        ctx.saveGState()
        let rimWidth: CGFloat = max(0.32, 0.8 / u)   // never thinner than ~0.8 px on screen
        let local = CGMutablePath()
        local.addPath(tile, transform: CGAffineTransform(scaleX: 1 / u, y: 1 / u)
            .translatedBy(x: -tileRect.minX, y: -tileRect.minY))
        ctx.addPath(local.copy(strokingWithWidth: rimWidth * 2, lineCap: .round, lineJoin: .round, miterLimit: 4))
        ctx.clip()
        ctx.drawLinearGradient(gradient([(rgb(0xFFFFFF, 0.42), 0), (rgb(0xFFFFFF, 0.06), 0.45),
                                         (rgb(0xFFFFFF, 0.02), 0.8), (rgb(0xFFFFFF, 0.10), 1)]),
                               start: P(0, 0), end: P(0, 100), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
        ctx.restoreGState()
    }
    ctx.restoreGState()   // tile clip
    ctx.restoreGState()   // flip
}

func renderPNG(px: Int, style: Style = .macOS) -> Data {
    let ctx = makeContext(px)
    drawIcon(ctx, px: px, style: style)
    let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
    return rep.representation(using: .png, properties: [:])!
}

func write(_ data: Data, _ path: String) {
    try? FileManager.default.createDirectory(atPath: (path as NSString).deletingLastPathComponent,
                                             withIntermediateDirectories: true)
    do { try data.write(to: URL(fileURLWithPath: path)) } catch { fatalError("cannot write \(path): \(error)") }
}

// MARK: - macOS icon set

let iconset = "\(outDir)/Swoop.iconset"
try? FileManager.default.removeItem(atPath: iconset)
try? FileManager.default.createDirectory(atPath: iconset, withIntermediateDirectories: true)
for size in [16, 32, 128, 256, 512] {
    write(renderPNG(px: size), "\(iconset)/icon_\(size)x\(size).png")
    write(renderPNG(px: size * 2), "\(iconset)/icon_\(size)x\(size)@2x.png")
}
write(renderPNG(px: 1024), "\(outDir)/Swoop-1024.png")
let task = Process()
task.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
task.arguments = ["-c", "icns", iconset, "-o", "\(outDir)/Swoop.icns"]
try task.run()
task.waitUntilExit()
print(task.terminationStatus == 0 ? "wrote \(outDir)/Swoop.icns" : "iconutil failed")

// MARK: - Web and extension assets

if let dir = webDir {
    write(renderPNG(px: 32, style: .fullBleed), "\(dir)/favicon-32.png")
    write(renderPNG(px: 64, style: .fullBleed), "\(dir)/favicon-64.png")
    write(renderPNG(px: 180, style: .square), "\(dir)/apple-touch-icon.png")
    write(renderPNG(px: 512), "\(dir)/icon-512.png")
    print("wrote web assets to \(dir)")
}
if let dir = extDir {
    for size in [16, 32, 48, 128] { write(renderPNG(px: size, style: .fullBleed), "\(dir)/icon\(size).png") }
    print("wrote extension icons to \(dir)")
}

// MARK: - Brand glyph

/// The monochrome glyph: the bird and water line only, fitted to a unit square (y down) and centred.
let glyphPath: CGPath = {
    let art = CGMutablePath()
    art.addPath(cgPath(birdSegs))
    art.addPath(cgPath(waterSegs).copy(strokingWithWidth: 4.2, lineCap: .round, lineJoin: .round, miterLimit: 4))
    let box = art.boundingBoxOfPath
    let side = max(box.width, box.height)
    let fit = CGAffineTransform(scaleX: 1 / side, y: 1 / side)
        .translatedBy(x: -(box.midX - side / 2), y: -(box.midY - side / 2))
    let unit = CGMutablePath()
    unit.addPath(art, transform: fit)
    return unit
}()

func glyphSegs() -> [Seg] { segs(of: glyphPath) }

func fmt(_ v: CGFloat, _ scale: CGFloat) -> String {
    var s = String(format: "%.2f", v * scale)
    while s.contains(".") && (s.hasSuffix("0") || s.hasSuffix(".")) { s.removeLast() }
    return s == "-0" ? "0" : s
}

func svgPathData(scale: CGFloat) -> String {
    func p(_ a: CGPoint) -> String { "\(fmt(a.x, scale)) \(fmt(a.y, scale))" }
    return glyphSegs().map { s -> String in
        switch s {
        case .m(let a): return "M\(p(a))"
        case .l(let a): return "L\(p(a))"
        case .q(let c, let a): return "Q\(p(c)) \(p(a))"
        case .c(let c1, let c2, let a): return "C\(p(c1)) \(p(c2)) \(p(a))"
        case .z: return "Z"
        }
    }.joined()
}

if let dir = glyphDir {
    let d = svgPathData(scale: 100)
    let svg = """
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" fill="currentColor">
      <path fill-rule="nonzero" d="\(d)"/>
    </svg>

    """
    write(svg.data(using: .utf8)!, "\(dir)/glyph.svg")

    let inline = """
    Swoop brand glyph: inline SVG path data (the bird + water line, monochrome, no tile).
    viewBox="0 0 100 100", fill with currentColor, fill-rule nonzero. Generated by scripts/render-icon.swift.

    \(d)

    Example:
    <svg viewBox="0 0 100 100" width="24" height="24" fill="currentColor" aria-hidden="true"><path d="\(d)"/></svg>

    """
    write(inline.data(using: .utf8)!, "\(dir)/glyph-inline.txt")

    func sp(_ a: CGPoint) -> String { "p(\(fmt(a.x, 1000)), \(fmt(a.y, 1000)))" }
    let body = glyphSegs().map { s -> String in
        switch s {
        case .m(let a): return "        path.move(to: \(sp(a)))"
        case .l(let a): return "        path.addLine(to: \(sp(a)))"
        case .q(let c, let a): return "        path.addQuadCurve(to: \(sp(a)), control: \(sp(c)))"
        case .c(let c1, let c2, let a): return "        path.addCurve(to: \(sp(a)), control1: \(sp(c1)), control2: \(sp(c2)))"
        case .z: return "        path.closeSubpath()"
        }
    }.joined(separator: "\n")
    let swift = """
    import SwiftUI

    /// The Swoop brand glyph: a bird in a vertical dive whose swept-back wings form a download arrow,
    /// above a water line. Monochrome, drawn in a square frame (aspect-fit and centred in `rect`), so it
    /// can be filled with any style: `SwoopGlyph().fill(.white).frame(width: 18, height: 18)`.
    /// Generated by scripts/render-icon.swift; coordinates are on a 1000-unit grid.
    struct SwoopGlyph: Shape {
        func path(in rect: CGRect) -> Path {
            let side = min(rect.width, rect.height)
            let ox = rect.midX - side / 2, oy = rect.midY - side / 2
            func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: ox + x / 1000 * side, y: oy + y / 1000 * side) }
            var path = Path()
    \(body)
            return path
        }
    }

    """
    write(swift.data(using: .utf8)!, "\(dir)/SwoopGlyph.swift")
    print("wrote brand glyph to \(dir)")
}

// MARK: - Contact sheet

if let path = sheetPath {
    // Rows: every macOS size at 1x on light and dark; the small sizes magnified 8x (nearest
    // neighbour) on light and dark, macOS and full-bleed; then the glyph at several sizes.
    let W = 1400, H = 1720
    let sheet = CGContext(data: nil, width: W, height: H, bitsPerComponent: 8, bytesPerRow: 0, space: srgb,
                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    func band(_ top: Int, _ h: Int, _ x: Int, _ w: Int, _ c: CGColor) {
        sheet.setFillColor(c)
        sheet.fill(CGRect(x: x, y: H - top - h, width: w, height: h))
    }
    func place(_ px: Int, _ style: Style, x: Int, top: Int, zoom: Int = 1) {
        let img = makeContext(px)
        drawIcon(img, px: px, style: style)
        sheet.interpolationQuality = .none
        sheet.draw(img.makeImage()!, in: CGRect(x: x, y: H - top - px * zoom, width: px * zoom, height: px * zoom))
    }
    let light = rgb(0xF2F2F5), dark = rgb(0x1E1E22)
    for (i, bg) in [light, dark].enumerated() {
        let top = i * 560
        band(top, 560, 0, W, bg)
        var x = 24
        for px in [512, 256, 128, 64, 32, 16] {
            place(px, .macOS, x: x, top: top + (560 - px) / 2)
            x += px + 40
        }
    }
    for (i, bg) in [light, dark].enumerated() {
        let left = i * W / 2
        band(1120, 340, left, W / 2, bg)
        place(16, .macOS, x: left + 20, top: 1150, zoom: 8)
        place(32, .macOS, x: left + 168, top: 1150, zoom: 8)
        place(16, .fullBleed, x: left + 444, top: 1150, zoom: 8)
        place(16, .fullBleed, x: left + 590, top: 1150)
        place(32, .fullBleed, x: left + 590, top: 1190)
        place(48, .fullBleed, x: left + 590, top: 1240)
    }
    for (i, (bg, ink)) in [(light, rgb(0x221F57)), (dark, rgb(0xFFFFFF))].enumerated() {
        let left = i * W / 2
        band(1460, 260, left, W / 2, bg)
        var x = left + 30
        for gs in [200, 64, 32, 16] {
            sheet.saveGState()
            sheet.translateBy(x: CGFloat(x), y: CGFloat(H - 1490))
            sheet.scaleBy(x: CGFloat(gs), y: -CGFloat(gs))
            sheet.addPath(glyphPath)
            sheet.setFillColor(ink)
            sheet.fillPath()
            sheet.restoreGState()
            x += gs + 40
        }
    }
    let rep = NSBitmapImageRep(cgImage: sheet.makeImage()!)
    write(rep.representation(using: .png, properties: [:])!, path)
    print("wrote \(path)")
}
