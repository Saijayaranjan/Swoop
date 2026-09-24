import AppKit
import OspreyKit

/// Dock icon with the active-download badge and an overall progress bar drawn under the icon.
@MainActor
final class DockTileController {
    private let model: AppModel
    private let view = DockProgressView()
    private var lastFraction: Double = -1
    private var lastBadge: String?

    init(model: AppModel) {
        self.model = model
        view.frame = NSRect(x: 0, y: 0, width: 128, height: 128)
        NSApp.dockTile.contentView = view
        NSApp.dockTile.display()
    }

    func refresh() {
        let showBadge = model.settings.bool("appearance.show_dock_badge", default: true)
        let active = model.tasks.items.filter { $0.state == .downloading || $0.state == .connecting || $0.state == .resolving }
        var total: UInt64 = 0, done: UInt64 = 0
        for t in active {
            if let size = t.progress.total, size > 0 {
                total += size
                done += min(t.progress.downloaded, size)
            }
        }
        let fraction = active.isEmpty || total == 0 ? -1 : Double(done) / Double(total)
        let badge = showBadge && !active.isEmpty ? "\(active.count)" : nil
        guard abs(fraction - lastFraction) > 0.002 || badge != lastBadge else { return }
        lastFraction = fraction
        lastBadge = badge
        view.fraction = fraction
        NSApp.dockTile.badgeLabel = badge
        NSApp.dockTile.display()
    }
}

final class DockProgressView: NSView {
    var fraction: Double = -1

    override func draw(_ dirtyRect: NSRect) {
        NSApp.applicationIconImage?.draw(in: bounds)
        guard fraction >= 0 else { return }
        let inset: CGFloat = 16
        let barHeight: CGFloat = 12
        let track = NSRect(x: inset, y: 10, width: bounds.width - inset * 2, height: barHeight)
        let trackPath = NSBezierPath(roundedRect: track, xRadius: barHeight / 2, yRadius: barHeight / 2)
        NSColor.black.withAlphaComponent(0.55).setFill()
        trackPath.fill()
        NSColor.white.withAlphaComponent(0.35).setStroke()
        trackPath.lineWidth = 1
        trackPath.stroke()
        var fill = track.insetBy(dx: 2, dy: 2)
        fill.size.width = max(fill.height, fill.width * CGFloat(min(1, fraction)))
        let fillPath = NSBezierPath(roundedRect: fill, xRadius: fill.height / 2, yRadius: fill.height / 2)
        let gradient = NSGradient(colors: [NSColor(red: 0.30, green: 0.83, blue: 0.88, alpha: 1),
                                           NSColor(red: 0.09, green: 0.64, blue: 0.71, alpha: 1)])
        gradient?.draw(in: fillPath, angle: 0)
    }
}
