import OspreyKit
import SwiftUI

// Shared building blocks for the secondary screens, drawn in the app's card language.

/// A solid card with an optional uppercase label.
struct GlassCard<Content: View>: View {
    var title: String?
    var symbol: String?
    var padding: CGFloat
    var content: Content
    init(_ title: String? = nil, symbol: String? = nil, padding: CGFloat = 18, @ViewBuilder content: () -> Content) {
        self.title = title
        self.symbol = symbol
        self.padding = padding
        self.content = content()
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let title { CardLabel(title, symbol: symbol) }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .cardSurface(cornerRadius: 20, padding: padding)
    }
}

/// An empty state with a tinted symbol medallion.
struct EmptyStateView<Actions: View>: View {
    var symbol: String
    var title: String
    var message: String
    var actions: Actions
    init(_ symbol: String, title: String, message: String, @ViewBuilder actions: () -> Actions) {
        self.symbol = symbol
        self.title = title
        self.message = message
        self.actions = actions()
    }
    var body: some View {
        VStack(spacing: 14) {
            FeatherIllustration(size: 170, glyph: symbol)
            VStack(spacing: 6) {
                Text(LocalizedStringKey(title)).font(.system(size: 20, weight: .bold))
                Text(LocalizedStringKey(message))
                    .font(.system(size: 14))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 380)
            }
            actions
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(20)
    }
}

extension EmptyStateView where Actions == EmptyView {
    init(_ symbol: String, title: String, message: String) {
        self.init(symbol, title: title, message: message) { EmptyView() }
    }
}

struct StatePill: View {
    var state: TaskState
    var compact: Bool
    init(_ state: TaskState, compact: Bool = true) {
        self.state = state
        self.compact = compact
    }
    var body: some View { StatusCapsule(state, compact: compact) }
}

struct NameCell: View {
    let item: TaskItem
    var body: some View {
        HStack(spacing: 10) {
            FileBadge(name: item.name, kind: item.kind, size: 30)
            VStack(alignment: .leading, spacing: 1) {
                Text(item.name).font(.system(size: 13, weight: .medium)).lineLimit(1).truncationMode(.middle)
                if !item.domain.isEmpty { Text(item.domain).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1) }
            }
        }
    }
}

struct ProgressCapsule: View {
    var fraction: Double
    var tint: Color
    var height: CGFloat
    init(fraction: Double, tint: Color = Theme.blue, active: Bool = false, indeterminate: Bool = false, height: CGFloat = 6) {
        self.fraction = fraction
        self.tint = tint
        self.height = height
    }
    var body: some View { ThinProgress(fraction: fraction, tint: tint, height: height) }
}

extension Theme {
    static var violet: Color { Theme.upload }
    static var teal: Color { Color(nsColor: .systemTeal) }
    static var info: Color { Theme.blue }
    static func color(for state: TaskState) -> Color { StatusStyle.of(state).color }

    /// An SF Symbol for a file name, used for file badges.
    static func fileSymbol(name: String, kind: TaskKind = .http) -> String {
        let ext = (name as NSString).pathExtension.lowercased()
        if kind.isTorrent && !["iso", "img"].contains(ext) { return "point.3.filled.connected.trianglepath.dotted" }
        if kind == .hls { return "play.rectangle.fill" }
        switch ext {
        case "mp4", "mkv", "mov", "webm", "m4v", "avi", "m3u8", "ts": return "film.fill"
        case "mp3", "m4a", "flac", "wav", "aac", "ogg", "opus": return "waveform"
        case "jpg", "jpeg", "png", "gif", "heic", "webp", "svg", "tiff", "raw": return "photo.fill"
        case "zip", "rar", "7z", "gz", "tar", "bz2", "xz", "tgz": return "archivebox.fill"
        case "dmg", "iso", "img": return "opticaldiscdrive.fill"
        case "pkg", "app", "exe", "msi", "deb", "rpm", "apk": return "shippingbox.fill"
        case "pdf": return "doc.richtext.fill"
        case "txt", "md", "rtf", "doc", "docx", "pages", "odt": return "doc.text.fill"
        case "csv", "xls", "xlsx", "numbers", "json", "xml", "sql": return "tablecells.fill"
        case "key", "ppt", "pptx": return "rectangle.on.rectangle.angled.fill"
        case "swift", "py", "js", "rs", "go", "c", "h", "sh": return "chevron.left.forwardslash.chevron.right"
        default: return "doc.fill"
        }
    }

    static func fileTint(name: String, kind: TaskKind = .http) -> Color {
        switch fileSymbol(name: name, kind: kind) {
        case "film.fill", "play.rectangle.fill": return Color(light: 0x8E5BE8, dark: 0xB08CFF)
        case "waveform": return Color(light: 0xE0527A, dark: 0xFF7DA0)
        case "photo.fill": return Color(light: 0x1FA2A8, dark: 0x5AD6DB)
        case "archivebox.fill": return Color(light: 0xD98A1E, dark: 0xF5B04C)
        case "opticaldiscdrive.fill", "shippingbox.fill": return Color(light: 0x5566E0, dark: 0x8C98FF)
        case "doc.richtext.fill": return Color(light: 0xE0533F, dark: 0xFF7A66)
        case "tablecells.fill": return Color(light: 0x2E9E5B, dark: 0x5BD48C)
        case "point.3.filled.connected.trianglepath.dotted": return Color(light: 0x23A07A, dark: 0x4FD6A8)
        default: return Theme.blue
        }
    }
}
