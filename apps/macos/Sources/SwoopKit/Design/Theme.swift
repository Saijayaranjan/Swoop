import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// Colour, type and status styling. One accent (system blue) carries progress and primary actions;
/// the semantic colours appear only in status capsules and small indicators.
public enum Theme {
    public static var accent: Color { .accentColor }
    public static let blue = Color(nsColor: .systemBlue)
    public static let success = Color(nsColor: .systemGreen)
    public static let warning = Color(nsColor: .systemOrange)
    public static let danger = Color(nsColor: .systemRed)
    public static let neutral = Color(nsColor: .systemGray)
    /// Secondary series colour for upload lines (a cool violet that sits well on both washes).
    public static let upload = Color(light: 0x7A5CF0, dark: 0xA08BFF)

    // MARK: surfaces

    /// Window wash, top-leading → bottom-trailing.
    public static let washStart = Color(light: 0xEAF4FB, dark: 0x14132B)
    public static let washEnd = Color(light: 0xDCEBFA, dark: 0x2A1F4F)
    /// Soft glow blended into the wash.
    public static let washGlow = Color(light: 0xFFFFFF, dark: 0x6D4FD6)
    public static let washGlowOpacity: (light: Double, dark: Double) = (0.75, 0.30)

    /// The floating content panel.
    public static let panel = Color(light: 0xF8FBFE, dark: 0x15142A, lightAlpha: 0.92, darkAlpha: 0.94)
    /// Solid cards inside the panel.
    public static let card = Color(light: 0xFFFFFF, dark: 0x1F1D38)
    /// Recessed wells (heatmap cells, empty tracks) inside cards.
    public static let well = Color(light: 0xEDF2F8, dark: 0x2A2748)
    public static let hairline = Color(light: 0x0B1B33, dark: 0xFFFFFF, lightAlpha: 0.07, darkAlpha: 0.07)
    public static let rowHover = Color(light: 0x0B1B33, dark: 0xFFFFFF, lightAlpha: 0.035, darkAlpha: 0.045)
    public static let rowSelected = Color(light: 0x0A84FF, dark: 0x0A84FF, lightAlpha: 0.12, darkAlpha: 0.22)
    public static let sidebarSelection = Color(light: 0xFFFFFF, dark: 0xFFFFFF, lightAlpha: 0.80, darkAlpha: 0.10)

    // MARK: type

    public static let pageTitle = Font.system(size: 34, weight: .bold)
    public static let sidebarRow = Font.system(size: 17, weight: .regular)
    public static let cardLabel = Font.system(size: 11, weight: .semibold)
    public static let cardLabelTracking: CGFloat = 1.2

    /// Big rounded numerals for dashboards and tiles.
    public static func numeral(_ size: CGFloat, weight: Font.Weight = .semibold) -> Font {
        .system(size: size, weight: weight, design: .rounded).monospacedDigit()
    }

    // MARK: state

    /// Progress bar tint for a state: blue while moving, grey when stopped, red when failed.
    public static func progressTint(_ state: TaskState) -> Color {
        switch state {
        case .failed: return danger
        case .paused: return warning
        case .cancelled, .pending, .queued, .scheduled: return neutral
        case .completed, .seeding: return success
        default: return blue
        }
    }

    /// Four-step health colour (inspector diagnostics only).
    public static func health(_ score: UInt8) -> Color {
        switch score {
        case 65...100: return .secondary
        case 40..<65: return warning
        default: return danger
        }
    }
}

/// How a task state is drawn in a status capsule.
public struct StatusStyle: Sendable {
    public var color: Color
    public var symbol: String
    public init(color: Color, symbol: String) { self.color = color; self.symbol = symbol }

    public static func of(_ state: TaskState) -> StatusStyle {
        switch state {
        case .downloading: return .init(color: Theme.blue, symbol: "arrow.down")
        case .resolving, .connecting: return .init(color: Theme.blue, symbol: "antenna.radiowaves.left.and.right")
        case .retrying: return .init(color: Theme.blue, symbol: "arrow.clockwise")
        case .verifying: return .init(color: Theme.blue, symbol: "checkmark.shield")
        case .processing: return .init(color: Theme.blue, symbol: "gearshape")
        case .completed: return .init(color: Theme.success, symbol: "checkmark")
        case .seeding: return .init(color: Theme.success, symbol: "arrow.up")
        case .paused: return .init(color: Theme.warning, symbol: "pause.fill")
        case .failed: return .init(color: Theme.danger, symbol: "exclamationmark.triangle.fill")
        case .cancelled: return .init(color: Theme.neutral, symbol: "xmark")
        case .scheduled: return .init(color: Theme.neutral, symbol: "calendar")
        case .pending, .queued: return .init(color: Theme.neutral, symbol: "hourglass")
        }
    }
}

public extension Color {
    /// A dynamic colour from two sRGB hex values.
    init(light: UInt32, dark: UInt32, lightAlpha: Double = 1, darkAlpha: Double = 1) {
        self.init(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            let hex = isDark ? dark : light
            return NSColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                           blue: CGFloat(hex & 0xFF) / 255, alpha: isDark ? darkAlpha : lightAlpha)
        })
    }
}

/// Real Finder file icons, cached per extension.
@MainActor
public enum FileIcons {
    private static var cache: [String: NSImage] = [:]

    public static func icon(for name: String, kind: TaskKind = .http) -> NSImage {
        let ext = kind.isTorrent && (name as NSString).pathExtension.isEmpty ? "torrent" : (name as NSString).pathExtension.lowercased()
        let key = ext.isEmpty ? (kind.isTorrent ? "#torrent-folder" : "#data") : ext
        if let cached = cache[key] { return cached }
        let image: NSImage
        if key == "#torrent-folder" {
            image = NSWorkspace.shared.icon(for: .folder)
        } else if let type = UTType(filenameExtension: ext) {
            image = NSWorkspace.shared.icon(for: type)
        } else {
            image = NSWorkspace.shared.icon(for: .data)
        }
        cache[key] = image
        return image
    }
}
