import AppKit
import OspreyKit
import SwiftUI

/// Per-day totals for the activity heatmap.
struct DayActivity: Equatable {
    var files: Int = 0
    var bytes: UInt64 = 0
}

/// The overview page: live throughput, speed mode, counts, engine, activity and recent files.
struct DashboardView: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @ViewState private var activity: [Date: DayActivity] = [:]
    @ViewState private var allTimeBytes: UInt64 = 0
    @ViewState private var allTimeFiles: UInt32 = 0

    static let weeks = 12

    var body: some View {
        VStack(spacing: 0) {
            PageHeader("Dashboard", subtitle: greeting) {
                GlassControlGroup {
                    GroupIconButton(symbol: "plus", help: "Add Download", tint: Theme.blue) { ui.openAdd() }
                    GroupIconButton(symbol: "pause.fill", help: "Pause All") { model.pauseAll() }
                    GroupIconButton(symbol: "play.fill", help: "Resume All") { model.resumeAll() }
                }
            }
            ScrollView {
                VStack(spacing: 16) {
                    HStack(alignment: .top, spacing: 16) {
                        ThroughputCard()
                        SpeedModeCard().frame(width: 318)
                    }
                    .frame(height: 200)
                    HStack(spacing: 16) {
                        CountTile(label: "Active", value: Int(model.stats.active), symbol: "arrow.down", tint: Theme.blue,
                                  caption: "\(model.stats.downloading) downloading · \(model.stats.seeding) seeding")
                        CountTile(label: "Queued", value: Int(model.stats.queued + model.stats.scheduled), symbol: "hourglass", tint: Theme.neutral,
                                  caption: "\(model.stats.paused) paused · \(model.stats.scheduled) scheduled")
                        CountTile(label: "Completed", value: Int(model.stats.completedToday), symbol: "checkmark", tint: Theme.success,
                                  caption: model.stats.failedToday > 0 ? "today · \(model.stats.failedToday) failed" : "today")
                        EngineTile()
                    }
                    .frame(height: 118)
                    HStack(alignment: .top, spacing: 16) {
                        ActivityCard(activity: activity, allTimeBytes: allTimeBytes, allTimeFiles: allTimeFiles)
                        RecentCard().frame(width: 318)
                    }
                    .frame(height: 256)
                }
                .padding(.horizontal, 28)
                .padding(.top, 6)
                .padding(.bottom, 24)
            }
            .scrollIndicators(.never)
            .ospreySoftScrollEdge()
        }
        .task(id: model.historyVersion) {
            await model.refreshDashboard()
            await loadActivity()
        }
    }

    private var greeting: String {
        let h = Calendar.current.component(.hour, from: Date())
        let part = h < 5 ? "Working late" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening"
        let active = model.stats.active
        if active == 0 { return L10n.tr(part) + " — " + L10n.tr("all quiet on the wire.") }
        return L10n.tr(part) + " — " + String(format: L10n.tr("%d transfers in flight."), Int(active))
    }

    private func loadActivity() async {
        if let sample = SnapshotSample.activity {
            activity = sample
            allTimeBytes = SnapshotSample.allTimeBytes
            allTimeFiles = SnapshotSample.allTimeFiles
            return
        }
        let cal = Calendar.current
        let start = cal.date(byAdding: .day, value: -(Self.weeks * 7 + 7), to: cal.startOfDay(for: Date())) ?? Date()
        var q = HistoryQueryData()
        q.state = .completed
        q.since = start.millis
        q.limit = 5000
        let entries = (try? await model.engine.history(q)) ?? []
        var byDay: [Date: DayActivity] = [:]
        for e in entries {
            let day = cal.startOfDay(for: Date(millis: e.finishedAt))
            byDay[day, default: DayActivity()].files += 1
            byDay[day, default: DayActivity()].bytes += e.size ?? 0
        }
        activity = byDay
        var all = HistoryQueryData()
        all.state = .completed
        all.limit = 20000
        let everything = (try? await model.engine.history(all)) ?? []
        allTimeBytes = everything.reduce(0) { $0 + ($1.size ?? 0) }
        allTimeFiles = UInt32(everything.count)
    }
}

// MARK: - Throughput

private struct ThroughputCard: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let samples = Array(model.speedHistory.suffix(120))
        let down = samples.map { Double($0.download) }
        let up = samples.map { Double($0.upload) }
        let peak = max(down.max() ?? 0, up.max() ?? 0)
        let ceiling = max(peak * 1.15, 250_000)
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                CardLabel("Throughput", symbol: "waveform.path.ecg")
                Spacer()
                LegendDot(color: Theme.blue, text: "Download")
                LegendDot(color: Theme.upload, text: "Upload")
            }
            HStack(alignment: .firstTextBaseline, spacing: 28) {
                VStack(alignment: .leading, spacing: 2) {
                    LiveNumber(Fmt.speed(model.stats.downloadSpeed, zero: "0 B/s"), value: Double(model.stats.downloadSpeed), size: 46)
                    Label("Download", systemImage: "arrow.down")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Theme.blue)
                }
                VStack(alignment: .leading, spacing: 2) {
                    LiveNumber(Fmt.speed(model.stats.uploadSpeed, zero: "0 B/s"), value: Double(model.stats.uploadSpeed), size: 28)
                    Label("Upload", systemImage: "arrow.up")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Theme.upload)
                }
                Spacer()
                if peak > 0 {
                    VStack(alignment: .trailing, spacing: 2) {
                        Text(Fmt.speed(UInt64(peak)))
                            .font(.system(size: 15, weight: .semibold, design: .rounded).monospacedDigit())
                        Text("peak").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
            }
            .padding(.top, 10)
            ZStack(alignment: .bottom) {
                VStack(spacing: 0) {
                    ForEach(0..<3) { _ in
                        Rectangle().fill(Theme.hairline).frame(height: 1)
                        Spacer(minLength: 0)
                    }
                    Rectangle().fill(Theme.hairline).frame(height: 1)
                }
                if samples.count > 1 {
                    Sparkline(down, color: Theme.blue, fill: true, lineWidth: 2.2, ceiling: ceiling)
                    Sparkline(up, color: Theme.upload, fill: false, lineWidth: 1.6, ceiling: ceiling)
                } else {
                    Text("A live trace appears here while downloads run.")
                        .font(.system(size: 12))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .padding(.top, 14)
            .frame(maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .cardSurface()
    }
}

private struct LegendDot: View {
    var color: Color
    var text: String
    var body: some View {
        HStack(spacing: 5) {
            Circle().fill(color).frame(width: 7, height: 7)
            Text(LocalizedStringKey(text)).font(.system(size: 11, weight: .medium)).foregroundStyle(.secondary)
        }
        .padding(.leading, 8)
    }
}

// MARK: - Speed mode

private struct SpeedModeCard: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let current = model.stats.trafficMode == .fullSpeed ? TrafficMode.unlimited : model.stats.trafficMode
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                CardLabel("Speed mode", symbol: "speedometer")
                Spacer()
                SettingsLink {
                    Image(systemName: "slider.horizontal.3").font(.system(size: 12, weight: .semibold)).foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .help("Speed Settings…")
            }
            VStack(spacing: 6) {
                ForEach(TrafficMode.quickModes, id: \.self) { mode in
                    let selected = current == mode
                    Button { withAnimation(.smooth(duration: 0.25)) { model.setTrafficMode(mode) } } label: {
                        HStack(spacing: 12) {
                            Image(systemName: mode.symbol)
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(selected ? .white : Theme.blue)
                                .frame(width: 34, height: 34)
                                .background(selected ? AnyShapeStyle(Theme.blue.gradient) : AnyShapeStyle(Theme.blue.opacity(0.12)),
                                            in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                            VStack(alignment: .leading, spacing: 1) {
                                Text(LocalizedStringKey(mode.label)).font(.system(size: 14, weight: .semibold))
                                Text(LocalizedStringKey(mode.blurb)).font(.system(size: 11.5)).foregroundStyle(.secondary).lineLimit(1)
                            }
                            Spacer(minLength: 0)
                            if selected {
                                Image(systemName: "checkmark.circle.fill").foregroundStyle(Theme.blue).font(.system(size: 16))
                                    .transition(.scale.combined(with: .opacity))
                            }
                        }
                        .padding(6)
                        .background(selected ? Theme.blue.opacity(0.08) : .clear, in: RoundedRectangle(cornerRadius: 13, style: .continuous))
                        .contentShape(RoundedRectangle(cornerRadius: 13, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(selected ? .isSelected : [])
                }
            }
            if current == .custom {
                Text("Custom limits are active.").font(.system(size: 11)).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .cardSurface(padding: 16)
    }
}

// MARK: - Tiles

private struct CountTile: View {
    var label: String
    var value: Int
    var symbol: String
    var tint: Color
    var caption: String

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                CardLabel(label)
                Spacer()
                Image(systemName: symbol)
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(tint)
                    .frame(width: 26, height: 26)
                    .background(tint.opacity(0.14), in: Circle())
            }
            Spacer(minLength: 0)
            Text("\(value)")
                .font(Theme.numeral(44))
                .contentTransition(.numericText(value: Double(value)))
                .animation(.smooth, value: value)
            Text(LocalizedStringKey(caption))
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .cardSurface(padding: 16)
    }
}

private struct EngineTile: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let (text, color): (String, Color) = {
            switch model.status {
            case .ready: return (model.networkAvailable ? "Running" : "Offline", model.networkAvailable ? Theme.success : Theme.warning)
            case .starting: return ("Starting", Theme.warning)
            case .unavailable: return ("Stopped", Theme.danger)
            }
        }()
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                CardLabel("Engine")
                Spacer()
                StatusDot(color: color, pulse: model.status == .ready)
            }
            Spacer(minLength: 0)
            Text(LocalizedStringKey(text))
                .font(.system(size: 30, weight: .semibold, design: .rounded))
            Text(detail)
                .font(.system(size: 12).monospacedDigit())
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .cardSurface(padding: 16)
    }

    private var detail: String {
        var parts: [String] = []
        if !model.info.version.isEmpty { parts.append("v\(model.info.version)") }
        if let disk = model.disks.first, let free = disk.free { parts.append("\(Fmt.bytes(free)) free") }
        return parts.isEmpty ? L10n.tr("Local engine") : parts.joined(separator: " · ")
    }
}

// MARK: - Activity heatmap

private struct ActivityCard: View {
    var activity: [Date: DayActivity]
    var allTimeBytes: UInt64
    var allTimeFiles: UInt32
    @Environment(AppModel.self) private var model

    private let cell: CGFloat = 17
    private let gap: CGFloat = 5

    var body: some View {
        let cal = Calendar.current
        let today = cal.startOfDay(for: Date())
        let weekStart = cal.dateInterval(of: .weekOfYear, for: today)?.start ?? today
        let firstWeek = cal.date(byAdding: .weekOfYear, value: -(DashboardView.weeks - 1), to: weekStart) ?? weekStart
        let weeks: [[Date]] = (0..<DashboardView.weeks).map { w in
            (0..<7).compactMap { d in cal.date(byAdding: .day, value: w * 7 + d, to: firstWeek) }
        }
        let windowDays = activity.filter { $0.key >= firstWeek }
        let maxBytes = max(windowDays.values.map(\.bytes).max() ?? 0, 1)
        let files = windowDays.values.reduce(0) { $0 + $1.files }
        let busiest = windowDays.max { $0.value.bytes < $1.value.bytes }

        VStack(alignment: .leading, spacing: 10) {
            HStack {
                CardLabel("Activity", symbol: "calendar")
                Spacer()
                Text("Last \(DashboardView.weeks) weeks").font(.system(size: 11, weight: .medium)).foregroundStyle(.secondary)
            }
            HStack(alignment: .top, spacing: 26) {
                VStack(alignment: .leading, spacing: 6) {
                    // Month labels.
                    HStack(spacing: gap) {
                        Color.clear.frame(width: 26, height: 12)
                        ForEach(Array(weeks.enumerated()), id: \.offset) { i, week in
                            let first = week.first ?? today
                            let show = i == 0 || cal.component(.month, from: first) != cal.component(.month, from: weeks[i - 1].first ?? first)
                            Text(show ? first.formatted(.dateTime.month(.abbreviated)) : "")
                                .font(.system(size: 10, weight: .medium))
                                .foregroundStyle(.secondary)
                                .fixedSize()
                                .frame(width: cell, alignment: .leading)
                        }
                    }
                    HStack(alignment: .top, spacing: gap) {
                        VStack(alignment: .leading, spacing: gap) {
                            ForEach(0..<7) { d in
                                let label = d % 2 == 0 ? (weeks.first?[d].formatted(.dateTime.weekday(.abbreviated)) ?? "") : ""
                                Text(label)
                                    .font(.system(size: 10, weight: .medium))
                                    .foregroundStyle(.secondary)
                                    .frame(width: 26, height: cell, alignment: .leading)
                            }
                        }
                        ForEach(Array(weeks.enumerated()), id: \.offset) { _, week in
                            VStack(spacing: gap) {
                                ForEach(week, id: \.self) { day in
                                    HeatCell(level: level(activity[day]?.bytes ?? 0, max: maxBytes, files: activity[day]?.files ?? 0),
                                             isToday: day == today, isFuture: day > today, size: cell)
                                        .help(tooltip(day, activity[day]))
                                }
                            }
                        }
                    }
                    HStack(spacing: 5) {
                        Color.clear.frame(width: 26, height: 1)
                        Text("Quiet").font(.system(size: 10)).foregroundStyle(.secondary)
                        ForEach(0..<5) { l in HeatCell(level: l, isToday: false, isFuture: false, size: 10) }
                        Text("Busy").font(.system(size: 10)).foregroundStyle(.secondary)
                    }
                    .padding(.top, 4)
                }
                .fixedSize()

                VStack(alignment: .leading, spacing: 14) {
                    stat("Transferred today", Fmt.bytes(model.stats.bytesToday), big: true)
                    stat("All time", allTimeBytes > 0 ? Fmt.bytes(allTimeBytes) : "0 B", big: false,
                         caption: allTimeFiles > 0 ? "\(allTimeFiles) files" : nil)
                    stat("Busiest day", busiest.map { $0.key.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day()) } ?? "—",
                         big: false, caption: busiest.map { "\(Fmt.bytes($0.value.bytes)) · \(files) files in \(DashboardView.weeks) weeks" })
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .cardSurface()
    }

    private func stat(_ label: String, _ value: String, big: Bool, caption: String? = nil) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(L10n.tr(label).uppercased())
                .font(.system(size: 10, weight: .semibold))
                .tracking(1)
                .foregroundStyle(.secondary)
            LiveNumber(value, value: 0, size: big ? 32 : 20)
            if let caption {
                Text(caption).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
            }
        }
    }

    private func level(_ bytes: UInt64, max: UInt64, files: Int) -> Int {
        guard bytes > 0 || files > 0 else { return 0 }
        let f = Double(bytes) / Double(max)
        return f > 0.75 ? 4 : f > 0.45 ? 3 : f > 0.18 ? 2 : 1
    }

    private func tooltip(_ day: Date, _ a: DayActivity?) -> String {
        let d = day.formatted(date: .abbreviated, time: .omitted)
        guard let a, a.files > 0 else { return "\(d): nothing downloaded" }
        return "\(d): \(a.files) file\(a.files == 1 ? "" : "s"), \(Fmt.bytes(a.bytes))"
    }
}

private struct HeatCell: View {
    var level: Int
    var isToday: Bool
    var isFuture: Bool
    var size: CGFloat

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
            .fill(fill)
            .frame(width: size, height: size)
            .overlay {
                if isToday {
                    RoundedRectangle(cornerRadius: size * 0.28, style: .continuous).strokeBorder(Theme.blue, lineWidth: 1.5)
                }
            }
            .opacity(isFuture ? 0.35 : 1)
    }

    private var fill: Color {
        switch level {
        case 0: return Theme.well
        case 1: return Theme.blue.opacity(0.28)
        case 2: return Theme.blue.opacity(0.5)
        case 3: return Theme.blue.opacity(0.75)
        default: return Theme.blue
        }
    }
}

// MARK: - Recent

private struct RecentCard: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            CardLabel("Recently finished", symbol: "checkmark.seal")
            if model.recentCompletions.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "tray")
                        .font(.system(size: 26, weight: .light))
                        .foregroundStyle(.tertiary)
                    Text("Finished files land here.")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                VStack(spacing: 2) {
                    ForEach(model.recentCompletions.prefix(5)) { r in RecentRow(row: r) }
                }
                Spacer(minLength: 0)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .cardSurface(padding: 16)
    }
}

private struct RecentRow: View {
    let row: TaskRowData
    @ViewState private var hovering = false

    var body: some View {
        Button { Finder.reveal([row.targetPath]) } label: {
            HStack(spacing: 10) {
                FileBadge(name: row.name, kind: row.kind, size: 32)
                VStack(alignment: .leading, spacing: 1) {
                    Text(row.name).font(.system(size: 13, weight: .medium)).lineLimit(1).truncationMode(.middle)
                    Text("\(Fmt.bytes(row.progress.total ?? row.progress.downloaded)) · \(ShortDate.string(row.completedAt))")
                        .font(.system(size: 11).monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(Theme.blue)
                    .opacity(hovering ? 1 : 0)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(hovering ? Theme.rowHover : .clear, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help("Show in Finder")
    }
}
