import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// Semantic colour and iconography only. Osprey uses system colours throughout: the accent for
/// progress and the primary action, red for errors, orange for warnings, green solely for the
/// "completed" glyph. There are no custom brand colours in the interface.
public enum Theme {
    public static var accent: Color { .accentColor }
    public static let success = Color(nsColor: .systemGreen)
    public static let warning = Color(nsColor: .systemOrange)
    public static let danger = Color(nsColor: .systemRed)

    /// Progress bar tint for a state: accent while moving, grey when stopped, red when failed.
    public static func progressTint(_ state: TaskState) -> Color {
        switch state {
        case .failed: return danger
        case .paused, .cancelled, .pending, .queued, .scheduled: return Color(nsColor: .tertiaryLabelColor)
        case .completed: return Color(nsColor: .secondaryLabelColor)
        default: return .accentColor
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

    /// Big numerals (only used by the Activity popover's single hero figure).
    public static func numeral(_ size: CGFloat, weight: Font.Weight = .semibold) -> Font {
        .system(size: size, weight: weight, design: .rounded).monospacedDigit()
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
