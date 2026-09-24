import Charts
import OspreyKit
import SwiftUI

/// The Activity popover, opened from the toolbar speed readout: one throughput chart and three
/// plain figures.
struct ActivityPopover: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let torrentsActive = model.stats.seeding > 0 || model.tasks.count { $0.kind.isTorrent && $0.state.isActive } > 0
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 2) {
                LiveNumber(Fmt.speed(model.stats.downloadSpeed, zero: "0 B/s"), value: Double(model.stats.downloadSpeed), size: 28)
                Text(statusLine)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            SpeedChart(samples: Array(model.speedHistory.suffix(180)), showUpload: torrentsActive)
                .frame(height: 110)
            Divider()
            Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 4) {
                GridRow {
                    stat("\(model.stats.active)", "Active")
                    stat("\(model.stats.completedToday)", "Completed today")
                    stat(Fmt.bytes(model.stats.bytesToday), "Downloaded today")
                }
            }
            if let disk = model.disks.first, let free = disk.free {
                Text("\(Fmt.bytes(free)) available on \((disk.path as NSString).lastPathComponent)")
                    .font(.caption)
                    .foregroundStyle(disk.volumeAvailable ? AnyShapeStyle(.secondary) : AnyShapeStyle(Theme.warning))
            }
        }
        .padding(16)
        .frame(width: 340)
        .task { await model.refreshDashboard() }
    }

    private var statusLine: String {
        var parts: [String] = [model.networkAvailable ? L10n.tr("Online") : L10n.tr("Offline"), L10n.tr(model.stats.trafficMode.label)]
        if model.stats.uploadSpeed > 0 { parts.append("↑ \(Fmt.speed(model.stats.uploadSpeed))") }
        return parts.joined(separator: " · ")
    }

    private func stat(_ value: String, _ label: String) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(value).font(.headline.monospacedDigit())
            Text(LocalizedStringKey(label)).font(.caption).foregroundStyle(.secondary)
        }
    }
}

struct SpeedChart: View {
    let samples: [SpeedSampleData]
    var showUpload: Bool = false

    private var yMax: Double {
        let peak = samples.map { Double(max($0.download, showUpload ? $0.upload : 0)) }.max() ?? 0
        return max(peak * 1.15, 500_000)
    }

    var body: some View {
        if samples.count < 2 {
            Text("Throughput appears here while downloads run.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            Chart {
                ForEach(samples) { s in
                    AreaMark(x: .value("Time", Date(millis: s.at)), y: .value("Download", Double(s.download)), series: .value("Series", "down"))
                        .interpolationMethod(.monotone)
                        .foregroundStyle(Color.accentColor.opacity(0.25))
                    LineMark(x: .value("Time", Date(millis: s.at)), y: .value("Download", Double(s.download)), series: .value("Series", "down"))
                        .interpolationMethod(.monotone)
                        .foregroundStyle(Color.accentColor)
                        .lineStyle(StrokeStyle(lineWidth: 1.5))
                }
                if showUpload {
                    ForEach(samples) { s in
                        LineMark(x: .value("Time", Date(millis: s.at)), y: .value("Upload", Double(s.upload)), series: .value("Series", "up"))
                            .interpolationMethod(.monotone)
                            .foregroundStyle(Color.secondary)
                            .lineStyle(StrokeStyle(lineWidth: 1))
                    }
                }
            }
            .chartYScale(domain: 0...yMax)
            .chartYAxis {
                AxisMarks(position: .trailing, values: .automatic(desiredCount: 2)) { v in
                    AxisValueLabel { if let d = v.as(Double.self), d > 0 { Text(Fmt.speed(UInt64(d))).font(.caption2) } }
                }
            }
            .chartXAxis(.hidden)
            .accessibilityLabel(Text("Throughput over the last few minutes"))
        }
    }
}
