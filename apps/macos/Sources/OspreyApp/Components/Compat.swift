import OspreyKit
import SwiftUI

// Stock-component stand-ins for secondary screens that still use the earlier building blocks.
// Each maps to plain system styling (GroupBox, ContentUnavailableView, system colours).

struct GlassCard<Content: View>: View {
    var title: String?
    var symbol: String?
    var content: Content
    init(_ title: String? = nil, symbol: String? = nil, padding: CGFloat = 0, @ViewBuilder content: () -> Content) {
        self.title = title
        self.symbol = symbol
        self.content = content()
    }
    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) { content }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(4)
        } label: {
            if let title { Text(LocalizedStringKey(title)) }
        }
    }
}

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
        ContentUnavailableView {
            Label(LocalizedStringKey(title), systemImage: symbol)
        } description: {
            Text(LocalizedStringKey(message))
        } actions: {
            actions
        }
    }
}

extension EmptyStateView where Actions == EmptyView {
    init(_ symbol: String, title: String, message: String) {
        self.init(symbol, title: title, message: message) { EmptyView() }
    }
}

struct StatePill: View {
    var state: TaskState
    init(_ state: TaskState, compact: Bool = false) { self.state = state }
    var body: some View {
        Text(L10n.state(state))
            .font(.caption)
            .foregroundStyle(state == .failed ? AnyShapeStyle(Theme.danger) : AnyShapeStyle(.secondary))
    }
}

struct NameCell: View {
    let item: TaskItem
    var body: some View {
        HStack(spacing: 8) {
            Image(nsImage: FileIcons.icon(for: item.name, kind: item.kind)).resizable().frame(width: 20, height: 20)
            Text(item.name).lineLimit(1).truncationMode(.middle)
        }
    }
}

struct ProgressCapsule: View {
    var fraction: Double
    var tint: Color
    init(fraction: Double, tint: Color = .accentColor, active: Bool = false, indeterminate: Bool = false, height: CGFloat = 3) {
        self.fraction = fraction
        self.tint = tint
    }
    var body: some View { ThinProgress(fraction: fraction, tint: tint) }
}

extension Theme {
    static var violet: Color { .secondary }
    static var teal: Color { .accentColor }
    static var upload: Color { .secondary }
    static var info: Color { .secondary }
    static func color(for state: TaskState) -> Color { progressTint(state) }
    static func fileSymbol(name: String, kind: TaskKind = .http) -> String { "doc" }
    static func fileTint(name: String, kind: TaskKind = .http) -> Color { .secondary }
}
