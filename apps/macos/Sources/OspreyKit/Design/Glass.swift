import SwiftUI

// Liquid Glass with graceful fallbacks. The deployment target is macOS 14, so every glass API is
// gated on macOS 26; older systems get `.ultraThinMaterial`. "Reduce transparency" always wins and
// renders opaque, high-legibility surfaces.

public enum GlassVariant: Sendable {
    case regular
    case clear
    /// Tinted glass for prominent surfaces (accent-coloured).
    case tinted(Color)
}

struct OspreyGlassModifier<S: Shape>: ViewModifier {
    var variant: GlassVariant
    var shape: S
    var interactive: Bool
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast

    func body(content: Content) -> some View {
        if reduceTransparency {
            content
                .background(Color(nsColor: .windowBackgroundColor), in: shape)
                .overlay(shape.outline(Color.primary.opacity(contrast == .increased ? 0.5 : 0.12), lineWidth: 1))
        } else if #available(macOS 26, *) {
            content.glassEffect(glass, in: shape)
                .overlay {
                    if contrast == .increased { shape.outline(Color.primary.opacity(0.45), lineWidth: 1) }
                }
        } else {
            content
                .background(.ultraThinMaterial, in: shape)
                .overlay(shape.outline(Color.white.opacity(0.12), lineWidth: 0.5))
        }
    }

    @available(macOS 26, *)
    private var glass: Glass {
        var g: Glass
        switch variant {
        case .regular: g = .regular
        case .clear: g = .clear
        case .tinted(let c): g = Glass.regular.tint(c.opacity(0.85))
        }
        if interactive { g = g.interactive() }
        return g
    }
}

private extension Shape {
    func outline(_ color: Color, lineWidth: CGFloat) -> some View {
        self.stroke(color, lineWidth: lineWidth)
    }
}

public extension View {
    /// Frosted, refractive Liquid Glass background in `shape`.
    func ospreyGlass<S: Shape>(_ variant: GlassVariant = .regular, in shape: S, interactive: Bool = false) -> some View {
        modifier(OspreyGlassModifier(variant: variant, shape: shape, interactive: interactive))
    }

    func ospreyGlass(_ variant: GlassVariant = .regular, cornerRadius: CGFloat = 18, interactive: Bool = false) -> some View {
        ospreyGlass(variant, in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous), interactive: interactive)
    }

    /// Glass button styling (`.glass` / `.glassProminent` on macOS 26, bordered before).
    @ViewBuilder
    func ospreyGlassButton(prominent: Bool = false) -> some View {
        if #available(macOS 26, *) {
            // Secondary glass buttons stay neutral; only prominent ones take the accent tint.
            if prominent { self.buttonStyle(.glassProminent) } else { self.buttonStyle(.glass).tint(nil) }
        } else {
            if prominent { self.buttonStyle(.borderedProminent) } else { self.buttonStyle(.bordered) }
        }
    }

    /// Morph identity for glass shapes that appear/disappear inside a `GlassGroup`.
    @ViewBuilder
    func ospreyGlassID<ID: Hashable & Sendable>(_ id: ID, in namespace: Namespace.ID) -> some View {
        if #available(macOS 26, *) {
            self.glassEffectID(id, in: namespace)
        } else {
            self
        }
    }

    /// Content extends beneath floating glass panes (sidebar / inspector).
    @ViewBuilder
    func ospreyBackgroundExtension() -> some View {
        if #available(macOS 26, *) { self.backgroundExtensionEffect() } else { self }
    }

    /// Soft scroll-edge fade under the toolbar.
    @ViewBuilder
    func ospreySoftScrollEdge() -> some View {
        if #available(macOS 26, *) { self.scrollEdgeEffectStyle(.soft, for: .top) } else { self }
    }
}

/// Groups glass shapes so they blend and morph together (`GlassEffectContainer`).
public struct GlassGroup<Content: View>: View {
    var spacing: CGFloat
    var content: Content
    public init(spacing: CGFloat = 12, @ViewBuilder content: () -> Content) {
        self.spacing = spacing
        self.content = content()
    }
    public var body: some View {
        if #available(macOS 26, *) {
            GlassEffectContainer(spacing: spacing) { content }
        } else {
            content
        }
    }
}
