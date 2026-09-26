import AppKit
import SwoopKit
import SwiftUI

// MARK: - Window wash

/// The static low-saturation gradient behind everything, with a faint glow.
struct WindowWash: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        ZStack {
            LinearGradient(colors: [Theme.washStart, Theme.washEnd], startPoint: .topLeading, endPoint: .bottomTrailing)
            RadialGradient(colors: [Theme.washGlow.opacity(scheme == .dark ? Theme.washGlowOpacity.dark : Theme.washGlowOpacity.light), .clear],
                           center: UnitPoint(x: 0.08, y: 0.0), startRadius: 0, endRadius: 520)
            RadialGradient(colors: [Theme.washGlow.opacity(scheme == .dark ? 0.16 : 0.35), .clear],
                           center: UnitPoint(x: 0.15, y: 1.0), startRadius: 0, endRadius: 420)
        }
        .ignoresSafeArea()
        .windowDraggable()
    }
}

extension View {
    /// Lets the window be dragged from this (otherwise empty) area.
    @ViewBuilder
    func windowDraggable() -> some View {
        if #available(macOS 15, *) { self.gesture(WindowDragGesture()) } else { self }
    }
}

// MARK: - Surfaces

extension View {
    /// A solid rounded card with a hairline edge and a soft lift.
    func cardSurface(cornerRadius: CGFloat = 20, padding: CGFloat? = 18) -> some View {
        modifier(CardSurface(cornerRadius: cornerRadius, padding: padding))
    }
}

private struct CardSurface: ViewModifier {
    var cornerRadius: CGFloat
    var padding: CGFloat?
    @Environment(\.colorScheme) private var scheme

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        content
            .padding(padding ?? 0)
            .background(Theme.card, in: shape)
            .overlay(shape.strokeBorder(Theme.hairline, lineWidth: 1))
            .shadow(color: Color(red: 0.1, green: 0.2, blue: 0.4).opacity(scheme == .dark ? 0 : 0.06), radius: 14, y: 6)
    }
}

// MARK: - Brand mark

/// The app mark: the diving-bird glyph (``SwoopGlyph``) on the app icon's indigo tile.
struct SwoopMark: View {
    var size: CGFloat = 30
    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
            .fill(LinearGradient(colors: [Color(red: 0.23, green: 0.21, blue: 0.60), Color(red: 0.12, green: 0.11, blue: 0.34)],
                                 startPoint: .top, endPoint: .bottom))
            .overlay {
                RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
                    .strokeBorder(LinearGradient(colors: [.white.opacity(0.45), .white.opacity(0.04)], startPoint: .top, endPoint: .bottom), lineWidth: 0.8)
            }
            .overlay {
                SwoopGlyph()
                    .fill(.white)
                    .frame(width: size * 0.7, height: size * 0.7)
                    .shadow(color: .black.opacity(0.25), radius: 1, y: 0.5)
            }
            .frame(width: size, height: size)
            .shadow(color: Color(red: 0.18, green: 0.16, blue: 0.49).opacity(0.35), radius: size * 0.2, y: size * 0.08)
            .accessibilityHidden(true)
    }
}

// MARK: - Empty-state illustration

/// A single asymmetric feather: vane, rachis, a notch and faint barbs.
struct FeatherShape: Shape {
    func path(in r: CGRect) -> Path {
        var p = Path()
        let w = r.width, h = r.height
        p.move(to: CGPoint(x: r.minX + w * 0.5, y: r.maxY))
        p.addCurve(to: CGPoint(x: r.minX + w * 0.58, y: r.minY),
                   control1: CGPoint(x: r.minX - w * 0.05, y: r.minY + h * 0.62),
                   control2: CGPoint(x: r.minX + w * 0.12, y: r.minY + h * 0.08))
        p.addCurve(to: CGPoint(x: r.minX + w * 0.86, y: r.minY + h * 0.42),
                   control1: CGPoint(x: r.minX + w * 0.86, y: r.minY + h * 0.08),
                   control2: CGPoint(x: r.minX + w * 0.98, y: r.minY + h * 0.26))
        // A small split in the vane.
        p.addLine(to: CGPoint(x: r.minX + w * 0.66, y: r.minY + h * 0.47))
        p.addLine(to: CGPoint(x: r.minX + w * 0.82, y: r.minY + h * 0.52))
        p.addCurve(to: CGPoint(x: r.minX + w * 0.5, y: r.maxY),
                   control1: CGPoint(x: r.minX + w * 0.76, y: r.minY + h * 0.75),
                   control2: CGPoint(x: r.minX + w * 0.58, y: r.minY + h * 0.86))
        p.closeSubpath()
        return p
    }
}

struct FeatherRachis: Shape {
    func path(in r: CGRect) -> Path {
        var p = Path()
        p.move(to: CGPoint(x: r.minX + r.width * 0.5, y: r.maxY + r.height * 0.06))
        p.addQuadCurve(to: CGPoint(x: r.minX + r.width * 0.57, y: r.minY + r.height * 0.06),
                       control: CGPoint(x: r.minX + r.width * 0.46, y: r.minY + r.height * 0.4))
        for i in 1...6 {
            let t = CGFloat(i) / 7.5
            let y = r.maxY - r.height * t * 0.95
            let x = r.minX + r.width * (0.49 + 0.02 * t)
            p.move(to: CGPoint(x: x, y: y))
            p.addLine(to: CGPoint(x: x + r.width * 0.22, y: y - r.height * 0.07))
            p.move(to: CGPoint(x: x, y: y))
            p.addLine(to: CGPoint(x: x - r.width * 0.22, y: y - r.height * 0.06))
        }
        return p
    }
}

/// A fan of feathers rising from a soft glow — the empty-state artwork.
struct FeatherIllustration: View {
    var size: CGFloat = 220
    /// When set, a smaller fan frames a tile carrying this symbol.
    var glyph: String? = nil
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @ViewState private var lifted = false

    private var feathers: [(angle: Double, scale: CGFloat, hue: Double)] {
        if glyph != nil { return [(-58, 0.62, 0.60), (-30, 0.78, 0.63), (30, 0.78, 0.70), (58, 0.62, 0.73)] }
        return [(-52, 0.62, 0.60), (-27, 0.80, 0.63), (0, 1.0, 0.66), (27, 0.80, 0.70), (52, 0.62, 0.73)]
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(RadialGradient(colors: [Theme.blue.opacity(scheme == .dark ? 0.38 : 0.26), Theme.upload.opacity(0.10), .clear],
                                     center: .center, startRadius: 4, endRadius: size * 0.55))
                .frame(width: size * 1.2, height: size * 1.2)
                .blur(radius: 8)
            ForEach(Array(feathers.enumerated()), id: \.offset) { _, f in
                let h = size * 0.56 * f.scale
                let w = h * 0.36
                ZStack {
                    FeatherShape()
                        .fill(LinearGradient(colors: [Color(hue: f.hue, saturation: 0.55, brightness: 1.0),
                                                      Color(hue: f.hue + 0.02, saturation: 0.75, brightness: scheme == .dark ? 0.85 : 0.92)],
                                             startPoint: .top, endPoint: .bottom))
                    FeatherShape()
                        .fill(LinearGradient(colors: [.white.opacity(0.45), .clear], startPoint: .topLeading, endPoint: .center))
                    FeatherRachis()
                        .stroke(.white.opacity(0.55), style: StrokeStyle(lineWidth: 1.1, lineCap: .round))
                }
                .frame(width: w, height: h)
                .shadow(color: Color(hue: f.hue, saturation: 0.7, brightness: 0.8).opacity(0.35), radius: 10, y: 6)
                .offset(y: -h / 2)
                .rotationEffect(.degrees(f.angle), anchor: .bottom)
                .offset(y: size * 0.18)
            }
            if let glyph {
                // A luminous tile at the heart of the fan.
                RoundedRectangle(cornerRadius: size * 0.1, style: .continuous)
                    .fill(LinearGradient(colors: [Color(red: 0.30, green: 0.64, blue: 1.0), Color(red: 0.36, green: 0.30, blue: 0.92)],
                                         startPoint: .topLeading, endPoint: .bottomTrailing))
                    .overlay {
                        RoundedRectangle(cornerRadius: size * 0.1, style: .continuous)
                            .strokeBorder(LinearGradient(colors: [.white.opacity(0.6), .white.opacity(0.05)], startPoint: .top, endPoint: .bottom), lineWidth: 1)
                    }
                    .overlay {
                        Image(systemName: glyph)
                            .font(.system(size: size * 0.15, weight: .semibold))
                            .foregroundStyle(.white)
                            .shadow(color: .black.opacity(0.15), radius: 2, y: 1)
                    }
                    .frame(width: size * 0.34, height: size * 0.34)
                    .shadow(color: Theme.blue.opacity(0.45), radius: 14, y: 6)
                    .offset(y: size * 0.02)
            } else {
                // A small quill cap where the fan meets.
                Circle()
                    .fill(LinearGradient(colors: [.white, Color(hue: 0.64, saturation: 0.25, brightness: 1)], startPoint: .top, endPoint: .bottom))
                    .frame(width: size * 0.07, height: size * 0.07)
                    .shadow(color: Theme.blue.opacity(0.5), radius: 6)
                    .offset(y: size * 0.18)
            }
            // Drifting motes.
            ForEach(0..<5) { i in
                let a = Double(i) * 1.3 + 0.4
                Circle()
                    .fill(Color.white.opacity(scheme == .dark ? 0.55 : 0.9))
                    .frame(width: CGFloat(3 + i % 3), height: CGFloat(3 + i % 3))
                    .shadow(color: Theme.blue.opacity(0.6), radius: 3)
                    .offset(x: cos(a) * size * 0.44, y: sin(a) * size * 0.30 - size * 0.12)
            }
        }
        .frame(width: size, height: size * 0.8)
        .offset(y: lifted ? -4 : 2)
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.easeInOut(duration: 3.2).repeatForever(autoreverses: true)) { lifted = true }
        }
        .accessibilityHidden(true)
    }
}

// MARK: - Empty state

struct IllustratedEmptyState<Actions: View>: View {
    var title: String
    var message: String
    var glyph: String?
    var actions: Actions
    init(title: String, message: String, glyph: String? = nil, @ViewBuilder actions: () -> Actions) {
        self.glyph = glyph
        self.title = title
        self.message = message
        self.actions = actions()
    }
    var body: some View {
        VStack(spacing: 18) {
            FeatherIllustration(size: 230, glyph: glyph)
            VStack(spacing: 8) {
                Text(LocalizedStringKey(title))
                    .font(.system(size: 26, weight: .bold))
                Text(LocalizedStringKey(message))
                    .font(.system(size: 15))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 420)
            }
            actions.padding(.top, 6)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(24)
    }
}

// MARK: - Page header

/// "Downloads (12)" — the big page title, a count pill and page controls.
struct PageHeader<Controls: View, Below: View>: View {
    var title: String
    var count: String?
    var subtitle: String?
    var controls: Controls
    var below: Below

    init(_ title: String, count: String? = nil, subtitle: String? = nil,
         @ViewBuilder controls: () -> Controls, @ViewBuilder below: () -> Below) {
        self.title = title
        self.count = count
        self.subtitle = subtitle
        self.controls = controls()
        self.below = below()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .center, spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(alignment: .center, spacing: 12) {
                        Text(LocalizedStringKey(title))
                            .font(Theme.pageTitle)
                            .lineLimit(1)
                        if let count { CountPill(count) }
                    }
                    if let subtitle {
                        Text(subtitle).font(.system(size: 13)).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                Spacer(minLength: 16)
                controls
            }
            below
        }
        .padding(.horizontal, 28)
        .padding(.top, 22)
        .padding(.bottom, 12)
        .background { Color.clear.contentShape(Rectangle()).windowDraggable() }
    }
}

extension PageHeader where Below == EmptyView {
    init(_ title: String, count: String? = nil, subtitle: String? = nil, @ViewBuilder controls: () -> Controls) {
        self.init(title, count: count, subtitle: subtitle, controls: controls) { EmptyView() }
    }
}

extension PageHeader where Below == EmptyView, Controls == EmptyView {
    init(_ title: String, count: String? = nil, subtitle: String? = nil) {
        self.init(title, count: count, subtitle: subtitle, controls: { EmptyView() }) { EmptyView() }
    }
}

struct CountPill: View {
    var text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text)
            .font(.system(size: 13, weight: .semibold, design: .rounded).monospacedDigit())
            .foregroundStyle(.secondary)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(Theme.well, in: Capsule())
            .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
            .contentTransition(.numericText())
            .animation(.smooth, value: text)
    }
}

/// A row of icon buttons sharing one glass capsule.
struct GlassControlGroup<Content: View>: View {
    var content: Content
    init(@ViewBuilder content: () -> Content) { self.content = content() }
    var body: some View {
        HStack(spacing: 2) { content }
            .padding(.horizontal, 5)
            .padding(.vertical, 4)
            .swoopGlass(.regular, in: Capsule())
    }
}

/// An icon button used inside a `GlassControlGroup`.
struct GroupIconButton: View {
    var symbol: String
    var help: String
    var tint: Color? = nil
    var action: () -> Void
    @ViewState private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(tint.map { AnyShapeStyle($0) } ?? AnyShapeStyle(.primary.opacity(0.8)))
                .frame(width: 36, height: 32)
                .background(Circle().fill(Color.primary.opacity(hovering ? 0.08 : 0)).frame(width: 32, height: 32))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { h in withAnimation(.easeOut(duration: 0.12)) { hovering = h } }
        .help(Text(LocalizedStringKey(help)))
        .accessibilityLabel(Text(LocalizedStringKey(help)))
    }
}

/// A menu that looks like a `GroupIconButton`.
struct GroupIconMenu<Items: View>: View {
    var symbol: String
    var help: String
    var items: Items
    init(symbol: String, help: String, @ViewBuilder items: () -> Items) {
        self.symbol = symbol
        self.help = help
        self.items = items()
    }
    var body: some View {
        Menu { items } label: {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(.primary.opacity(0.8))
                .frame(width: 36, height: 32)
                .contentShape(Rectangle())
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help(Text(LocalizedStringKey(help)))
        .accessibilityLabel(Text(LocalizedStringKey(help)))
    }
}

/// A glass search capsule that grows from a round button into a field.
struct SearchCapsule: View {
    @Binding var text: String
    var prompt: String = "Search"
    var width: CGFloat = 240
    @ViewState private var expanded = false
    @FocusState private var focused: Bool

    var body: some View {
        let open = expanded || !text.isEmpty
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(.primary.opacity(0.8))
            if open {
                TextField(LocalizedStringKey(prompt), text: $text)
                    .textFieldStyle(.plain)
                    .font(.system(size: 14))
                    .focused($focused)
                    .onSubmit { if text.isEmpty { collapse() } }
                    .onExitCommand { text = ""; collapse() }
                    .transition(.opacity.combined(with: .move(edge: .leading)))
                if !text.isEmpty {
                    Button { text = "" } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("Clear search"))
                }
            }
        }
        .padding(.horizontal, open ? 14 : 0)
        .frame(width: open ? width : 42, height: 42)
        .contentShape(Capsule())
        .swoopGlass(.regular, in: Capsule(), interactive: !open)
        .onTapGesture {
            guard !open else { return }
            withAnimation(.spring(response: 0.38, dampingFraction: 0.82)) { expanded = true }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { focused = true }
        }
        .onChange(of: focused) { _, f in if !f && text.isEmpty { collapse() } }
        .help(Text("Search"))
    }

    private func collapse() {
        withAnimation(.spring(response: 0.34, dampingFraction: 0.86)) { expanded = false }
    }
}

// MARK: - Chips

/// A small capsule used in the status bar and filter rows.
struct Chip<Content: View>: View {
    var selected: Bool = false
    var content: Content
    init(selected: Bool = false, @ViewBuilder content: () -> Content) {
        self.selected = selected
        self.content = content()
    }
    var body: some View {
        HStack(spacing: 6) { content }
            .font(.system(size: 13, weight: .medium))
            .padding(.horizontal, 12)
            .frame(height: 30)
            .background(selected ? AnyShapeStyle(Theme.blue) : AnyShapeStyle(Theme.well), in: Capsule())
            .overlay(Capsule().strokeBorder(selected ? Color.clear : Theme.hairline, lineWidth: 1))
            .foregroundStyle(selected ? AnyShapeStyle(Color.white) : AnyShapeStyle(.primary))
    }
}

struct StatusDot: View {
    var color: Color
    var pulse: Bool = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @ViewState private var on = false
    var body: some View {
        ZStack {
            if pulse && !reduceMotion {
                Circle().fill(color.opacity(0.35)).frame(width: 14, height: 14).scaleEffect(on ? 1 : 0.5).opacity(on ? 0 : 1)
            }
            Circle().fill(color).frame(width: 8, height: 8)
                .shadow(color: color.opacity(0.7), radius: 3)
        }
        .frame(width: 14, height: 14)
        .onAppear {
            guard pulse, !reduceMotion else { return }
            withAnimation(.easeOut(duration: 1.6).repeatForever(autoreverses: false)) { on = true }
        }
    }
}

extension TrafficMode {
    /// The modes the speed selectors offer.
    static let quickModes: [TrafficMode] = [.unlimited, .balanced, .browsing]

    var blurb: String {
        switch self {
        case .unlimited, .fullSpeed: return "Everything the connection has"
        case .balanced: return "Leaves headroom for other apps"
        case .browsing: return "Stays out of the way"
        case .custom: return "Your own limits"
        }
    }
}

// MARK: - Status bar

/// The bottom row of chips: speed mode, live speeds, active count and engine status.
struct StatusBar: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui

    var body: some View {
        @Bindable var ui = ui
        let mode = model.stats.trafficMode == .fullSpeed ? TrafficMode.unlimited : model.stats.trafficMode
        HStack(spacing: 8) {
            Menu {
                Picker("Speed", selection: Binding(get: { mode }, set: { model.setTrafficMode($0) })) {
                    ForEach(TrafficMode.pickerModes, id: \.self) { m in
                        Label(LocalizedStringKey(m.label), systemImage: m.symbol).tag(m)
                    }
                }
                .pickerStyle(.inline)
                Divider()
                SettingsLink { Text("Speed Settings…") }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: mode.symbol)
                    Text(LocalizedStringKey(mode.label))
                }
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .font(.system(size: 13, weight: .medium))
            .padding(.horizontal, 12)
            .frame(height: 30)
            .background(Theme.well, in: Capsule())
            .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
            .help("Speed mode")

            Button { ui.showActivity.toggle() } label: {
                Chip {
                    Image(systemName: "arrow.down").font(.system(size: 11, weight: .bold)).foregroundStyle(Theme.blue)
                    Text(Fmt.speed(model.stats.downloadSpeed, zero: "0 B/s"))
                        .monospacedDigit()
                        .contentTransition(.numericText(value: Double(model.stats.downloadSpeed)))
                    Rectangle().fill(Theme.hairline).frame(width: 1, height: 14)
                    Image(systemName: "arrow.up").font(.system(size: 11, weight: .bold)).foregroundStyle(Theme.upload)
                    Text(Fmt.speed(model.stats.uploadSpeed, zero: "0 B/s"))
                        .monospacedDigit()
                        .contentTransition(.numericText(value: Double(model.stats.uploadSpeed)))
                }
                .animation(.smooth, value: model.stats.downloadSpeed)
            }
            .buttonStyle(.plain)
            .help("Show activity")
            .popover(isPresented: $ui.showActivity, arrowEdge: .top) {
                ActivityPopover().environment(model)
            }

            Chip {
                Image(systemName: "bolt.horizontal.fill").font(.system(size: 11)).foregroundStyle(model.stats.active > 0 ? Theme.blue : Color.secondary)
                Text("\(model.stats.active) active").monospacedDigit()
            }

            Spacer(minLength: 8)

            if let updates = UpdateController.shared, updates.updateReady {
                Button { updates.showWindow() } label: {
                    Chip {
                        Image(systemName: "sparkles").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.blue)
                        Text(updates.phase == .staged ? "Update on quit" : "Update ready")
                    }
                }
                .buttonStyle(.plain)
                .help(String(format: L10n.tr("Swoop %@ is downloaded and verified"), updates.info?.latestVersion ?? ""))
                .transition(.opacity.combined(with: .scale(scale: 0.9)))
            }

            if let disk = model.disks.first, let free = disk.free {
                Chip {
                    Image(systemName: "internaldrive").font(.system(size: 11)).foregroundStyle(.secondary)
                    Text("\(Fmt.bytes(free)) free").monospacedDigit()
                }
                .help("Free space on \((disk.path as NSString).lastPathComponent)")
            }
            if !model.networkAvailable {
                Chip {
                    StatusDot(color: Theme.warning)
                    Text("Offline")
                }
            }
            Chip {
                StatusDot(color: engineColor, pulse: model.status == .ready && model.stats.active > 0)
                Text(LocalizedStringKey(engineText))
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private var engineColor: Color {
        switch model.status {
        case .ready: return Theme.success
        case .starting: return Theme.warning
        case .unavailable: return Theme.danger
        }
    }

    private var engineText: String {
        switch model.status {
        case .ready: return "Engine ready"
        case .starting: return "Engine starting"
        case .unavailable: return "Engine stopped"
        }
    }
}

// MARK: - Dates

enum ShortDate {
    private static let time: DateFormatter = {
        let f = DateFormatter()
        f.timeStyle = .short
        f.dateStyle = .none
        return f
    }()
    private static let day: DateFormatter = {
        let f = DateFormatter()
        f.setLocalizedDateFormatFromTemplate("MMM d")
        return f
    }()

    /// "14:02", "Yesterday", "Sep 12".
    static func string(_ millis: Int64?) -> String {
        guard let millis, millis > 0 else { return "—" }
        let date = Date(millis: millis)
        let cal = Calendar.current
        if cal.isDateInToday(date) { return time.string(from: date) }
        if cal.isDateInYesterday(date) { return L10n.tr("Yesterday") }
        return day.string(from: date)
    }
}

// MARK: - Prominent call to action

/// A large blue capsule for the one primary action on a page.
struct ProminentCapsuleStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, 22)
            .frame(height: 40)
            .background(
                Capsule().fill(LinearGradient(colors: [Color(red: 0.24, green: 0.58, blue: 1.0), Color(red: 0.05, green: 0.42, blue: 0.98)],
                                              startPoint: .top, endPoint: .bottom))
            )
            .overlay(Capsule().strokeBorder(.white.opacity(0.25), lineWidth: 0.75))
            .shadow(color: Theme.blue.opacity(configuration.isPressed ? 0.15 : 0.35), radius: configuration.isPressed ? 4 : 10, y: 4)
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .opacity(enabled ? 1 : 0.5)
            .animation(.spring(response: 0.25, dampingFraction: 0.8), value: configuration.isPressed)
    }
}

/// A quiet capsule that pairs with `ProminentCapsuleStyle`.
struct SecondaryCapsuleStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 14, weight: .medium))
            .padding(.horizontal, 20)
            .frame(height: 40)
            .background(Theme.card, in: Capsule())
            .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
            .shadow(color: .black.opacity(0.06), radius: 6, y: 2)
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .animation(.spring(response: 0.25, dampingFraction: 0.8), value: configuration.isPressed)
    }
}

// MARK: - Window chrome

/// Hides the title bar of the hosting window and lets content run underneath it.
struct TransparentTitleBar: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { Probe() }
    func updateNSView(_ nsView: NSView, context: Context) {}

    final class Probe: NSView {
        private var observations: [NSKeyValueObservation] = []

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            observations.removeAll()
            guard let window else { return }
            apply(window)
            // SwiftUI may restore the standard title bar while it updates the scene; put ours back.
            observations = [
                window.observe(\.titlebarAppearsTransparent) { [weak self] w, _ in self?.reapply(w) },
                window.observe(\.titleVisibility) { [weak self] w, _ in self?.reapply(w) },
                window.observe(\.styleMask) { [weak self] w, _ in self?.reapply(w) },
            ]
        }

        private func reapply(_ window: NSWindow) {
            DispatchQueue.main.async { [weak self] in self?.apply(window) }
        }

        private func apply(_ window: NSWindow) {
            if !window.titlebarAppearsTransparent { window.titlebarAppearsTransparent = true }
            if window.titleVisibility != .hidden { window.titleVisibility = .hidden }
            if !window.styleMask.contains(.fullSizeContentView) { window.styleMask.insert(.fullSizeContentView) }
            if window.toolbar != nil { window.toolbar = nil }
        }
    }
}

extension View {
    func transparentTitleBar() -> some View { background(TransparentTitleBar().frame(width: 0, height: 0)) }
}
