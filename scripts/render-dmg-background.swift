// Renders the DMG window background (original artwork): a pale sky wash, the Swoop mark, an install
// hint and an arrow from the app to Applications. Writes <out>.png (1x) and <out>@2x.png.
// Usage: swift scripts/render-dmg-background.swift <out-path-without-extension>
import AppKit
import CoreGraphics
import Foundation

let base = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "build/dmg-background"
let width: CGFloat = 660, height: CGFloat = 440
// Icon centres in window points (top-left origin); keep in sync with scripts/dmg-settings.py.
let appCenter = CGPoint(x: 170, y: 232)
let appsCenter = CGPoint(x: 490, y: 232)

func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
    CGColor(red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255, alpha: a)
}

func render(scale: CGFloat, to path: String) {
    let pxW = Int(width * scale), pxH = Int(height * scale)
    let cs = CGColorSpace(name: CGColorSpace.sRGB)!
    guard let ctx = CGContext(data: nil, width: pxW, height: pxH, bitsPerComponent: 8, bytesPerRow: 0, space: cs,
                              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
    // Work in points with a top-left origin.
    ctx.scaleBy(x: scale, y: scale)
    ctx.translateBy(x: 0, y: height)
    ctx.scaleBy(x: 1, y: -1)

    // Wash.
    let wash = CGGradient(colorsSpace: cs, colors: [rgb(0xEEF6FC), rgb(0xDCEBFA)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(wash, start: .zero, end: CGPoint(x: width, y: height), options: [])
    let glow = CGGradient(colorsSpace: cs, colors: [rgb(0xFFFFFF, 0.85), rgb(0xFFFFFF, 0)] as CFArray, locations: [0, 1])!
    ctx.drawRadialGradient(glow, startCenter: CGPoint(x: 90, y: 0), startRadius: 0, endCenter: CGPoint(x: 90, y: 0), endRadius: 420, options: [])
    let violet = CGGradient(colorsSpace: cs, colors: [rgb(0x8C7CF0, 0.16), rgb(0x8C7CF0, 0)] as CFArray, locations: [0, 1])!
    ctx.drawRadialGradient(violet, startCenter: CGPoint(x: width, y: height), startRadius: 0, endCenter: CGPoint(x: width, y: height), endRadius: 380, options: [])

    // Soft pads that seat the two icons.
    for c in [appCenter, appsCenter] {
        let r = CGRect(x: c.x - 78, y: c.y - 84, width: 156, height: 168)
        let pad = CGPath(roundedRect: r, cornerWidth: 34, cornerHeight: 34, transform: nil)
        ctx.saveGState()
        ctx.setShadow(offset: CGSize(width: 0, height: 6), blur: 18, color: rgb(0x1A3A6B, 0.10))
        ctx.addPath(pad)
        ctx.setFillColor(rgb(0xFFFFFF, 0.55))
        ctx.fillPath()
        ctx.restoreGState()
        ctx.addPath(pad)
        ctx.setStrokeColor(rgb(0xFFFFFF, 0.9))
        ctx.setLineWidth(1)
        ctx.strokePath()
    }

    // Arrow: a gentle arc from the app to Applications.
    let start = CGPoint(x: appCenter.x + 92, y: appCenter.y - 6)
    let end = CGPoint(x: appsCenter.x - 96, y: appsCenter.y - 6)
    let control = CGPoint(x: (start.x + end.x) / 2, y: appCenter.y - 64)
    ctx.saveGState()
    ctx.setStrokeColor(rgb(0x2F7BF6))
    ctx.setLineWidth(3)
    ctx.setLineCap(.round)
    ctx.setLineDash(phase: 0, lengths: [1, 9])
    ctx.move(to: start)
    ctx.addQuadCurve(to: end, control: control)
    ctx.strokePath()
    ctx.restoreGState()
    // Arrowhead along the curve's final tangent.
    let angle = atan2(end.y - control.y, end.x - control.x)
    let head: CGFloat = 13
    ctx.saveGState()
    ctx.setFillColor(rgb(0x2F7BF6))
    ctx.move(to: CGPoint(x: end.x + cos(angle) * 4, y: end.y + sin(angle) * 4))
    ctx.addLine(to: CGPoint(x: end.x - cos(angle - 0.45) * head, y: end.y - sin(angle - 0.45) * head))
    ctx.addLine(to: CGPoint(x: end.x - cos(angle + 0.45) * head, y: end.y - sin(angle + 0.45) * head))
    ctx.closePath()
    ctx.fillPath()
    ctx.restoreGState()

    // Swoop mark: the app icon's indigo tile with the diving-bird glyph.
    let tile = CGRect(x: width / 2 - 20, y: 34, width: 40, height: 40)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: 3), blur: 10, color: rgb(0x2E2A7E, 0.35))
    ctx.addPath(CGPath(roundedRect: tile, cornerWidth: 12, cornerHeight: 12, transform: nil))
    ctx.clip()
    let tileGrad = CGGradient(colorsSpace: cs, colors: [rgb(0x3B3699), rgb(0x1F1C57)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(tileGrad, start: CGPoint(x: tile.midX, y: tile.minY), end: CGPoint(x: tile.midX, y: tile.maxY), options: [])
    ctx.restoreGState()
    drawGlyph(in: tile.insetBy(dx: 6, dy: 6), ctx: ctx)

    // Text.
    NSGraphicsContext.saveGraphicsState()
    let ns = NSGraphicsContext(cgContext: ctx, flipped: true)
    NSGraphicsContext.current = ns
    func draw(_ text: String, size: CGFloat, weight: NSFont.Weight, color: NSColor, y: CGFloat) {
        let para = NSMutableParagraphStyle()
        para.alignment = .center
        let font = NSFont.systemFont(ofSize: size, weight: weight)
        let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color, .paragraphStyle: para]
        (text as NSString).draw(in: CGRect(x: 0, y: y, width: width, height: size * 1.5), withAttributes: attrs)
    }
    draw("Install Swoop", size: 22, weight: .bold, color: NSColor(red: 0.08, green: 0.10, blue: 0.20, alpha: 1), y: 82)
    draw("Drag the app onto Applications", size: 13, weight: .regular, color: NSColor(red: 0.25, green: 0.30, blue: 0.42, alpha: 1), y: 112)
    draw("Fast, careful downloads for your Mac", size: 11, weight: .medium, color: NSColor(red: 0.35, green: 0.42, blue: 0.55, alpha: 0.8), y: 404)
    NSGraphicsContext.restoreGraphicsState()

    guard let image = ctx.makeImage() else { return }
    let rep = NSBitmapImageRep(cgImage: image)
    rep.size = NSSize(width: width, height: height)
    try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
}

/// The Swoop glyph (the same path as the app's `SwoopGlyph`, on a 100×100 grid), filled white.
let glyphPath = "M50 78.79C60.36 67.34 77.95 41.99 97.71 17.45C83.4 26.45 67.04 36.26 56.27 40.62C55.32 31.08 54.23 21.54 53.68 14.04C54.63 8.59 56.54 3.54 58.45 0Q53.54 1.64 50 6Q46.46 1.64 41.55 0C43.46 3.54 45.37 8.59 46.32 14.04C45.77 21.54 44.68 31.08 43.73 40.62C32.96 36.26 16.6 26.45 2.29 17.45C22.05 41.99 39.64 67.34 50 78.79ZM20.29 96.41C29.86 89.7 38.84 89.47 48.55 95.17C60.17 101.98 71.56 101.68 82.99 93.68C84.29 92.77 84.61 90.99 83.7 89.69C82.79 88.4 81.01 88.08 79.71 88.99C70.14 95.69 61.16 95.93 51.45 90.23C39.83 83.41 28.44 83.71 17.01 91.71C15.71 92.62 15.39 94.41 16.3 95.7C17.21 97 18.99 97.31 20.29 96.41Z"

/// Draws `glyphPath` (absolute M/C/Q/Z commands only) scaled into `r`.
func drawGlyph(in r: CGRect, ctx: CGContext) {
    func pt(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: r.minX + x / 100 * r.width, y: r.minY + y / 100 * r.height) }
    var tokens: [String] = []
    var current = ""
    for ch in glyphPath {
        if "MCQZ".contains(ch) {
            if !current.isEmpty { tokens.append(current) }
            tokens.append(String(ch)); current = ""
        } else if ch == " " || ch == "," {
            if !current.isEmpty { tokens.append(current) }
            current = ""
        } else if ch == "-" && !current.isEmpty {
            tokens.append(current); current = "-"
        } else {
            current.append(ch)
        }
    }
    if !current.isEmpty { tokens.append(current) }
    let path = CGMutablePath()
    var i = 0
    var command = "M"
    func num() -> CGFloat { defer { i += 1 }; return CGFloat(Double(tokens[i]) ?? 0) }
    while i < tokens.count {
        if "MCQZ".contains(tokens[i]) { command = tokens[i]; i += 1 }
        switch command {
        case "M": path.move(to: pt(num(), num())); command = "L"
        case "C":
            let c1 = pt(num(), num()), c2 = pt(num(), num()), e = pt(num(), num())
            path.addCurve(to: e, control1: c1, control2: c2)
        case "Q":
            let c = pt(num(), num()), e = pt(num(), num())
            path.addQuadCurve(to: e, control: c)
        case "Z": path.closeSubpath()
        default: path.addLine(to: pt(num(), num()))
        }
    }
    ctx.addPath(path)
    ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    ctx.fillPath()
}

render(scale: 1, to: "\(base).png")
render(scale: 2, to: "\(base)@2x.png")
print("  \(base).png, \(base)@2x.png")
