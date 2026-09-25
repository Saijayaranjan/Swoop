import SwiftUI

// MARK: - Progress

/// A slim rounded progress bar: blue while transferring, state-tinted otherwise.
public struct ThinProgress: View {
    var fraction: Double
    var tint: Color
    var indeterminate: Bool
    var height: CGFloat

    public init(fraction: Double, tint: Color = Theme.blue, indeterminate: Bool = false, height: CGFloat = 6) {
        self.fraction = fraction
        self.tint = tint
        self.indeterminate = indeterminate
        self.height = height
    }

    public var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(Theme.well)
                if indeterminate {
                    IndeterminateSweep(tint: tint, width: geo.size.width)
                } else {
                    Capsule()
                        .fill(LinearGradient(colors: [tint.opacity(0.78), tint], startPoint: .leading, endPoint: .trailing))
                        .frame(width: max(fraction > 0 ? height : 0, geo.size.width * max(0, min(1, fraction))))
                        .animation(.smooth(duration: 0.5), value: fraction)
                }
            }
            .clipShape(Capsule())
        }
        .frame(height: height)
        .accessibilityElement()
        .accessibilityLabel(Text("Progress"))
        .accessibilityValue(Text(indeterminate ? "In progress" : Fmt.percent(fraction)))
    }
}

private struct IndeterminateSweep: View {
    var tint: Color
    var width: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.animation(minimumInterval: 1 / 30, paused: reduceMotion)) { ctx in
            let t = ctx.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1.6) / 1.6
            Capsule()
                .fill(tint)
                .frame(width: width * 0.3)
                .offset(x: -width * 0.3 + CGFloat(t) * width * 1.3)
        }
    }
}

/// A progress bar with its percentage alongside.
public struct ProgressWithPercent: View {
    var fraction: Double
    var tint: Color
    var indeterminate: Bool
    public init(fraction: Double, tint: Color = Theme.blue, indeterminate: Bool = false) {
        self.fraction = fraction
        self.tint = tint
        self.indeterminate = indeterminate
    }
    public var body: some View {
        HStack(spacing: 10) {
            ThinProgress(fraction: fraction, tint: tint, indeterminate: indeterminate, height: 6)
            Text(indeterminate ? "—" : Fmt.percent(fraction))
                .font(.system(size: 12, weight: .semibold, design: .rounded).monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: 40, alignment: .trailing)
                .contentTransition(.numericText(value: fraction))
        }
    }
}

/// A large circular progress ring with a centred label.
public struct ProgressRing<Label: View>: View {
    var fraction: Double
    var tint: Color
    var lineWidth: CGFloat
    var label: Label
    public init(fraction: Double, tint: Color = Theme.blue, lineWidth: CGFloat = 12, @ViewBuilder label: () -> Label) {
        self.fraction = fraction
        self.tint = tint
        self.lineWidth = lineWidth
        self.label = label()
    }
    public var body: some View {
        ZStack {
            Circle().stroke(Theme.well, lineWidth: lineWidth)
            Circle()
                .trim(from: 0, to: max(0.001, min(1, fraction)))
                .stroke(AngularGradient(colors: [tint.opacity(0.55), tint], center: .center,
                                        startAngle: .degrees(0), endAngle: .degrees(360 * max(0.01, fraction))),
                        style: StrokeStyle(lineWidth: lineWidth, lineCap: .round))
                .rotationEffect(.degrees(-90))
                .shadow(color: tint.opacity(0.35), radius: 6)
                .animation(.smooth(duration: 0.6), value: fraction)
            label
        }
        .accessibilityElement(children: .combine)
        .accessibilityValue(Text(Fmt.percent(fraction)))
    }
}

// MARK: - Status capsule

/// A coloured capsule naming a task's state.
public struct StatusCapsule: View {
    var state: TaskState
    var compact: Bool
    public init(_ state: TaskState, compact: Bool = false) {
        self.state = state
        self.compact = compact
    }
    public var body: some View {
        let style = StatusStyle.of(state)
        HStack(spacing: 5) {
            Image(systemName: style.symbol)
                .font(.system(size: 9, weight: .bold))
            Text(L10n.state(state))
                .font(.system(size: 12, weight: .semibold))
                .lineLimit(1)
        }
        .foregroundStyle(style.color)
        .padding(.horizontal, compact ? 8 : 10)
        .padding(.vertical, compact ? 3 : 5)
        .background(style.color.opacity(0.14), in: Capsule())
        .overlay(Capsule().strokeBorder(style.color.opacity(0.18), lineWidth: 0.5))
        .fixedSize()
        .animation(.smooth, value: state)
    }
}

// MARK: - Sparkline

/// A smooth area sparkline. Used in dashboard cards and the menu bar extra.
public struct Sparkline: View {
    var values: [Double]
    var color: Color
    var fill: Bool
    var lineWidth: CGFloat
    var ceiling: Double?

    public init(_ values: [Double], color: Color = Theme.blue, fill: Bool = true, lineWidth: CGFloat = 2, ceiling: Double? = nil) {
        self.values = values
        self.color = color
        self.fill = fill
        self.lineWidth = lineWidth
        self.ceiling = ceiling
    }

    public var body: some View {
        GeometryReader { geo in
            let maxV = max(ceiling ?? 0, values.max() ?? 0, 1)
            let pts: [CGPoint] = values.enumerated().map { i, v in
                CGPoint(x: values.count < 2 ? 0 : geo.size.width * CGFloat(i) / CGFloat(values.count - 1),
                        y: geo.size.height - lineWidth - (geo.size.height - lineWidth * 2) * CGFloat(v / maxV))
            }
            ZStack {
                if fill {
                    Self.smooth(pts, closeTo: geo.size.height)
                        .fill(LinearGradient(colors: [color.opacity(0.32), color.opacity(0.0)], startPoint: .top, endPoint: .bottom))
                }
                Self.smooth(pts, closeTo: nil)
                    .stroke(color, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round, lineJoin: .round))
            }
        }
        .accessibilityHidden(true)
    }

    /// Catmull-Rom style smoothing through the points.
    static func smooth(_ pts: [CGPoint], closeTo baseline: CGFloat?) -> Path {
        Path { p in
            guard let first = pts.first else { return }
            if let baseline {
                p.move(to: CGPoint(x: first.x, y: baseline))
                p.addLine(to: first)
            } else {
                p.move(to: first)
            }
            if pts.count > 1 {
                for i in 1..<pts.count {
                    let p0 = pts[max(i - 2, 0)], p1 = pts[i - 1], p2 = pts[i], p3 = pts[min(i + 1, pts.count - 1)]
                    let c1 = CGPoint(x: p1.x + (p2.x - p0.x) / 6, y: p1.y + (p2.y - p0.y) / 6)
                    let c2 = CGPoint(x: p2.x - (p3.x - p1.x) / 6, y: p2.y - (p3.y - p1.y) / 6)
                    p.addCurve(to: p2, control1: c1, control2: c2)
                }
            }
            if let baseline, let last = pts.last {
                p.addLine(to: CGPoint(x: last.x, y: baseline))
                p.closeSubpath()
            }
        }
    }
}

// MARK: - Card label

/// Small uppercase tracked label used at the top of cards.
public struct CardLabel: View {
    var text: String
    var symbol: String?
    public init(_ text: String, symbol: String? = nil) {
        self.text = text
        self.symbol = symbol
    }
    public var body: some View {
        HStack(spacing: 6) {
            if let symbol {
                Image(systemName: symbol).font(.system(size: 10, weight: .bold))
            }
            Text(L10n.tr(text).uppercased())
                .font(Theme.cardLabel)
                .tracking(Theme.cardLabelTracking)
        }
        .foregroundStyle(.secondary)
    }
}

// MARK: - Status text for a task

public enum TaskStatusText {
    /// The detail line under a download's name ("1.2 GB of 3.4 GB — 4.2 MB/s — 6 min left").
    public static func detail(_ d: TaskRowData) -> String {
        let p = d.progress
        switch d.state {
        case .downloading:
            var parts = ["\(Fmt.bytes(p.downloaded)) of \(Fmt.bytes(p.total))"]
            if p.speed > 0 { parts.append(Fmt.speed(p.speed)) }
            if let eta = p.etaSeconds { parts.append("\(Fmt.eta(eta)) left") }
            if d.kind.isTorrent { parts.append("\(p.peers) peers") }
            return parts.joined(separator: " — ")
        case .seeding:
            return "Seeding — ↑ \(Fmt.speed(p.uploadSpeed, zero: "0 B/s")) — ratio \(Fmt.ratio(p.ratio))"
        case .completed:
            return "\(Fmt.bytes(p.total ?? p.downloaded)) — \(Fmt.date(d.completedAt ?? d.createdAt))"
        case .failed:
            return d.errorKind.map { L10n.error($0) } ?? (d.errorMessage ?? L10n.state(.failed))
        case .paused:
            let reason = d.blockedBy.first.map { L10n.pause($0) } ?? L10n.state(.paused)
            return p.downloaded > 0 ? "\(reason) — \(Fmt.bytes(p.downloaded)) of \(Fmt.bytes(p.total))" : reason
        case .resolving, .connecting, .retrying, .verifying, .processing:
            if let s = d.statusDetail, !s.isEmpty { return "\(L10n.state(d.state)) — \(s)" }
            return L10n.state(d.state) + "…"
        default:
            return L10n.state(d.state)
        }
    }
}

// MARK: - Big live number

/// A large rounded figure that rolls between values, with an optional smaller unit.
public struct LiveNumber: View {
    var text: String
    var size: CGFloat
    var value: Double
    public init(_ text: String, value: Double, size: CGFloat = 34) {
        self.text = text
        self.value = value
        self.size = size
    }
    public var body: some View {
        let (number, unit) = Self.split(text)
        HStack(alignment: .firstTextBaseline, spacing: size * 0.1) {
            Text(number)
                .font(Theme.numeral(size))
                .contentTransition(.numericText(value: value))
            if let unit {
                Text(unit)
                    .font(.system(size: max(12, size * 0.36), weight: .semibold, design: .rounded))
                    .foregroundStyle(.secondary)
            }
        }
        .animation(.smooth, value: text)
    }

    /// "4.2 MB/s" → ("4.2", "MB/s"); a bare number has no unit.
    static func split(_ text: String) -> (String, String?) {
        guard let space = text.lastIndex(of: " ") else { return (text, nil) }
        let head = String(text[..<space])
        guard head.last?.isNumber == true else { return (text, nil) }
        return (head, String(text[text.index(after: space)...]))
    }
}
