import SwoopKit
import SwiftUI

/// A soft rounded tile with a symbol for the file's kind (video, archive, disk image…).
struct FileBadge: View {
    var name: String
    var kind: TaskKind = .http
    var size: CGFloat = 36

    var body: some View {
        let tint = Theme.fileTint(name: name, kind: kind)
        let shape = RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
        Image(systemName: Theme.fileSymbol(name: name, kind: kind))
            .font(.system(size: size * 0.42, weight: .semibold))
            .symbolRenderingMode(.hierarchical)
            .foregroundStyle(tint)
            .frame(width: size, height: size)
            .background(LinearGradient(colors: [tint.opacity(0.22), tint.opacity(0.10)], startPoint: .topLeading, endPoint: .bottomTrailing),
                        in: shape)
            .overlay(shape.strokeBorder(tint.opacity(0.18), lineWidth: 0.75))
            .accessibilityHidden(true)
    }
}
