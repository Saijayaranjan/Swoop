import SwiftUI

// MARK: - Progress

/// A thin, calm progress line: system accent while transferring, grey when stopped, red on failure.
public struct ThinProgress: View {
    var fraction: Double
    var tint: Color
    var indeterminate: Bool
    var height: CGFloat

    public init(fraction: Double, tint: Color = .accentColor, indeterminate: Bool = false, height: CGFloat = 3) {
        self.fraction = fraction
        self.tint = tint
        self.indeterminate = indeterminate
        self.height = height
    }

    public var body: some View {
        Group {
            if indeterminate {
                ProgressView()
                    .progressViewStyle(.linear)
                    .tint(tint)
                    .frame(height: height)
                    .scaleEffect(x: 1, y: height / 4, anchor: .center)
            } else {
                GeometryReader { geo in
                    ZStack(alignment: .leading) {
                        Capsule().fill(Color(nsColor: .quaternaryLabelColor))
                        Capsule().fill(tint)
                            .frame(width: max(fraction > 0 ? height : 0, geo.size.width * max(0, min(1, fraction))))
                            .animation(.smooth, value: fraction)
                    }
                }
                .frame(height: height)
            }
        }
        .accessibilityElement()
        .accessibilityLabel(Text("Progress"))
        .accessibilityValue(Text(indeterminate ? "In progress" : Fmt.percent(fraction)))
    }
}

// MARK: - Sparkline (menu bar extra)

public struct Sparkline: View {
    var values: [Double]
    public init(_ values: [Double]) { self.values = values }

    public var body: some View {
        GeometryReader { geo in
            let maxV = max(values.max() ?? 1, 1)
            let pts: [CGPoint] = values.enumerated().map { i, v in
                CGPoint(x: values.count < 2 ? 0 : geo.size.width * CGFloat(i) / CGFloat(values.count - 1),
                        y: geo.size.height * (1 - CGFloat(v / maxV) * 0.9))
            }
            ZStack {
                Path { p in
                    guard let first = pts.first else { return }
                    p.move(to: CGPoint(x: first.x, y: geo.size.height))
                    for pt in pts { p.addLine(to: pt) }
                    p.addLine(to: CGPoint(x: pts.last?.x ?? 0, y: geo.size.height))
                    p.closeSubpath()
                }
                .fill(Color.accentColor.opacity(0.18))
                Path { p in
                    guard let first = pts.first else { return }
                    p.move(to: first)
                    for pt in pts.dropFirst() { p.addLine(to: pt) }
                }
                .stroke(Color.accentColor, style: StrokeStyle(lineWidth: 1.25, lineCap: .round, lineJoin: .round))
            }
        }
        .accessibilityHidden(true)
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
        Text(text)
            .font(Theme.numeral(size))
            .contentTransition(.numericText(value: value))
            .animation(.smooth, value: text)
    }
}
