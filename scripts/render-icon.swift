// Renders the Osprey app icon (original artwork) to PNGs at every macOS icon size and builds
// Osprey.icns with iconutil. Usage: swift scripts/render-icon.swift <out-dir>
import AppKit
import CoreGraphics
import Foundation

let outDir = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "apps/macos/Resources"
let iconset = "\(outDir)/Osprey.iconset"
try? FileManager.default.createDirectory(atPath: iconset, withIntermediateDirectories: true)

func render(size: Int, scale: Int, name: String) {
    let px = size * scale
    let cs = CGColorSpaceCreateDeviceRGB()
    guard let ctx = CGContext(data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0, space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
    let s = CGFloat(px)
    ctx.clear(CGRect(x: 0, y: 0, width: s, height: s))

    // macOS icon grid: rounded square occupying ~80% with 22.4% corner radius
    let inset = s * 0.10
    let rect = CGRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset)
    let radius = rect.width * 0.224
    let path = CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil)

    // soft shadow
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -s * 0.012), blur: s * 0.03, color: CGColor(red: 0, green: 0, blue: 0, alpha: 0.35))
    ctx.addPath(path)
    ctx.setFillColor(CGColor(red: 0.05, green: 0.20, blue: 0.30, alpha: 1))
    ctx.fillPath()
    ctx.restoreGState()

    // background gradient: deep teal → midnight
    ctx.saveGState()
    ctx.addPath(path)
    ctx.clip()
    let colors = [CGColor(red: 0.10, green: 0.56, blue: 0.62, alpha: 1), CGColor(red: 0.04, green: 0.18, blue: 0.32, alpha: 1)] as CFArray
    let grad = CGGradient(colorsSpace: cs, colors: colors, locations: [0, 1])!
    ctx.drawLinearGradient(grad, start: CGPoint(x: rect.minX, y: rect.maxY), end: CGPoint(x: rect.maxX, y: rect.minY), options: [])
    // subtle horizon band
    ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.06))
    ctx.fill(CGRect(x: rect.minX, y: rect.minY + rect.height * 0.30, width: rect.width, height: rect.height * 0.02))
    ctx.restoreGState()

    // The mark: a diving osprey — two swept wings and a plunging arrow body.
    let cx = rect.midX
    let cy = rect.midY
    let w = rect.width
    ctx.saveGState()
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)
    ctx.setStrokeColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    let lw = w * 0.085
    ctx.setLineWidth(lw)

    // wings: two curves rising from the centre outward and up
    let wing = CGMutablePath()
    wing.move(to: CGPoint(x: cx, y: cy + w * 0.02))
    wing.addCurve(to: CGPoint(x: cx - w * 0.36, y: cy + w * 0.30), control1: CGPoint(x: cx - w * 0.10, y: cy + w * 0.22), control2: CGPoint(x: cx - w * 0.24, y: cy + w * 0.30))
    wing.move(to: CGPoint(x: cx, y: cy + w * 0.02))
    wing.addCurve(to: CGPoint(x: cx + w * 0.36, y: cy + w * 0.30), control1: CGPoint(x: cx + w * 0.10, y: cy + w * 0.22), control2: CGPoint(x: cx + w * 0.24, y: cy + w * 0.30))
    ctx.addPath(wing)
    ctx.strokePath()

    // body: vertical shaft plunging down
    let shaft = CGMutablePath()
    shaft.move(to: CGPoint(x: cx, y: cy + w * 0.06))
    shaft.addLine(to: CGPoint(x: cx, y: cy - w * 0.28))
    ctx.addPath(shaft)
    ctx.strokePath()

    // arrow head (talons)
    let head = CGMutablePath()
    head.move(to: CGPoint(x: cx - w * 0.16, y: cy - w * 0.12))
    head.addLine(to: CGPoint(x: cx, y: cy - w * 0.28))
    head.addLine(to: CGPoint(x: cx + w * 0.16, y: cy - w * 0.12))
    ctx.addPath(head)
    ctx.strokePath()

    // water line: the target, three short dashes
    ctx.setStrokeColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.55))
    ctx.setLineWidth(lw * 0.55)
    let water = CGMutablePath()
    for i in -1...1 {
        let x = cx + CGFloat(i) * w * 0.15
        water.move(to: CGPoint(x: x - w * 0.05, y: cy - w * 0.36))
        water.addLine(to: CGPoint(x: x + w * 0.05, y: cy - w * 0.36))
    }
    ctx.addPath(water)
    ctx.strokePath()
    ctx.restoreGState()

    guard let img = ctx.makeImage() else { return }
    let rep = NSBitmapImageRep(cgImage: img)
    guard let png = rep.representation(using: .png, properties: [:]) else { return }
    try? png.write(to: URL(fileURLWithPath: "\(iconset)/\(name)"))
}

for (size, scale) in [(16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)] {
    let name = scale == 1 ? "icon_\(size)x\(size).png" : "icon_\(size)x\(size)@2x.png"
    render(size: size, scale: scale, name: name)
}
// also a 1024 PNG for DMG background / docs
render(size: 1024, scale: 1, name: "../Osprey-1024.png")
let task = Process()
task.launchPath = "/usr/bin/iconutil"
task.arguments = ["-c", "icns", iconset, "-o", "\(outDir)/Osprey.icns"]
task.launch()
task.waitUntilExit()
print(task.terminationStatus == 0 ? "wrote \(outDir)/Osprey.icns" : "iconutil failed")
