// Renders the DMG window background (original artwork): a pale sky wash, a wing mark, an install
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

    // Wing mark.
    let tile = CGRect(x: width / 2 - 20, y: 34, width: 40, height: 40)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: 3), blur: 10, color: rgb(0x2F6BF0, 0.35))
    ctx.addPath(CGPath(roundedRect: tile, cornerWidth: 12, cornerHeight: 12, transform: nil))
    ctx.clip()
    let tileGrad = CGGradient(colorsSpace: cs, colors: [rgb(0x409EFF), rgb(0x295CED)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(tileGrad, start: CGPoint(x: tile.minX, y: tile.minY), end: CGPoint(x: tile.maxX, y: tile.maxY), options: [])
    ctx.restoreGState()
    drawWing(in: CGRect(x: tile.minX + 6, y: tile.minY + 11, width: 28, height: 18), ctx: ctx)

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

/// The wing glyph used in the app's sidebar mark.
func drawWing(in r: CGRect, ctx: CGContext) {
    func pt(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: r.minX + x / 100 * r.width, y: r.minY + y / 64 * r.height) }
    let right = CGMutablePath()
    right.move(to: pt(50, 40))
    right.addCurve(to: pt(71, 13), control1: pt(56, 30), control2: pt(61, 15))
    right.addCurve(to: pt(99, 24), control1: pt(81, 11), control2: pt(92, 16))
    right.addLine(to: pt(90, 26.5))
    right.addLine(to: pt(94, 31))
    right.addLine(to: pt(84, 30))
    right.addLine(to: pt(87, 35))
    right.addLine(to: pt(77, 32.5))
    right.addCurve(to: pt(52, 52), control1: pt(67, 33), control2: pt(58, 42))
    right.closeSubpath()
    let all = CGMutablePath()
    all.addPath(right)
    all.addPath(right, transform: CGAffineTransform(a: -1, b: 0, c: 0, d: 1, tx: 2 * r.midX, ty: 0))
    all.move(to: pt(50, 34))
    all.addCurve(to: pt(54, 50), control1: pt(53, 38), control2: pt(55, 45))
    all.addLine(to: pt(50, 62))
    all.addLine(to: pt(46, 50))
    all.addCurve(to: pt(50, 34), control1: pt(45, 45), control2: pt(47, 38))
    all.closeSubpath()
    ctx.addPath(all)
    ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    ctx.fillPath()
}

render(scale: 1, to: "\(base).png")
render(scale: 2, to: "\(base)@2x.png")
print("  \(base).png, \(base)@2x.png")
