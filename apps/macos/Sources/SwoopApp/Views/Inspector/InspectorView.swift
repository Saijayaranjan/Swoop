import AppKit
import SwoopKit
import SwiftUI

enum InspectorTab: String, CaseIterable, Identifiable {
    case overview, connections, files, network, checksums, events, diagnostics
    var id: String { rawValue }
    var title: String {
        switch self {
        case .overview: return "Overview"
        case .connections: return "Connections"
        case .files: return "Files"
        case .network: return "Network"
        case .checksums: return "Checksums"
        case .events: return "Events"
        case .diagnostics: return "Diagnostics"
        }
    }
    var symbol: String {
        switch self {
        case .overview: return "info.circle"
        case .connections: return "point.3.connected.trianglepath.dotted"
        case .files: return "doc.on.doc"
        case .network: return "network"
        case .checksums: return "checkmark.seal"
        case .events: return "list.bullet"
        case .diagnostics: return "stethoscope"
        }
    }
}

/// Loads and periodically refreshes `task_detail` for the selected task.
@Observable
@MainActor
final class DetailLoader {
    var detail: TaskDetailData?
    var error: String?
    /// Per-segment speeds computed from committed-byte deltas between refreshes.
    var segmentSpeeds: [UInt32: UInt64] = [:]
    private var lastCommitted: [UInt32: (UInt64, Date)] = [:]
    private(set) var loadedId: String?

    func load(_ id: String, engine: EngineClient) async {
        if loadedId != id {
            detail = nil
            segmentSpeeds = [:]
            lastCommitted = [:]
        }
        loadedId = id
        do {
            let d = try await engine.taskDetail(id)
            guard loadedId == id else { return }
            let now = Date()
            var speeds: [UInt32: UInt64] = [:]
            for s in d.segments {
                if let (prev, at) = lastCommitted[s.index], now.timeIntervalSince(at) > 0.2, s.committed >= prev {
                    speeds[s.index] = UInt64(Double(s.committed - prev) / now.timeIntervalSince(at))
                }
                lastCommitted[s.index] = (s.committed, now)
            }
            segmentSpeeds = speeds
            detail = d
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

struct InspectorView: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @AppStorage("inspectorTab") private var tabRaw = InspectorTab.overview.rawValue
    @ViewState private var loader = DetailLoader()

    private var tab: InspectorTab { InspectorTab(rawValue: tabRaw) ?? .overview }

    var body: some View {
        Group {
            if ui.selection.count == 1, let id = ui.selection.first, let item = model.tasks[id] {
                Form {
                    Section {
                        InspectorHero(item: item, detail: loader.detail)
                    }
                    Section {
                        Picker("Section", selection: $tabRaw) {
                            ForEach(InspectorTab.allCases) { t in
                                Image(systemName: t.symbol)
                                    .help(Text(LocalizedStringKey(t.title)))
                                    .accessibilityLabel(Text(LocalizedStringKey(t.title)))
                                    .tag(t.rawValue)
                            }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                    } footer: {
                        Text(LocalizedStringKey(tab.title)).font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
                    }
                    tabContent(item)
                }
                .formStyle(.grouped)
                .scrollContentBackground(.hidden)
                .task(id: "\(id)#\(model.detailVersion[id] ?? 0)") {
                    await loader.load(id, engine: model.engine)
                }
                .task(id: id) {
                    while !Task.isCancelled {
                        try? await Task.sleep(nanoseconds: 1_000_000_000)
                        if let s = model.tasks[id]?.state, s.isActive { await loader.load(id, engine: model.engine) }
                    }
                }
            } else if ui.selection.count > 1 {
                MultiSelectionSummary(ids: Array(ui.selection))
            } else {
                EmptyStateView("sidebar.trailing", title: "Nothing selected",
                               message: "Select a download to see its progress, source and options.")
            }
        }
        .overlay(alignment: .topTrailing) {
            Button { ui.showInspector = false } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(.secondary)
                    .frame(width: 24, height: 24)
                    .background(Theme.well, in: Circle())
            }
            .buttonStyle(.plain)
            .padding(12)
            .help("Hide Inspector")
            .accessibilityLabel(Text("Hide Inspector"))
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    @ViewBuilder
    private func tabContent(_ item: TaskItem) -> some View {
        if loader.detail == nil {
            Section {
                HStack(spacing: 8) {
                    if loader.error == nil { ProgressView().controlSize(.small) }
                    Text(loader.error == nil ? "Loading details…" : "More details appear once the engine reports on this download.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        switch tab {
        case .overview: OverviewTab(item: item, detail: loader.detail)
        case .connections: ConnectionsTab(item: item, detail: loader.detail, speeds: loader.segmentSpeeds)
        case .files: FilesTab(item: item, detail: loader.detail)
        case .network: NetworkTab(item: item, detail: loader.detail)
        case .checksums: ChecksumsTab(item: item, detail: loader.detail)
        case .events: EventsTab(item: item)
        case .diagnostics: DiagnosticsTab(item: item, detail: loader.detail)
        }
    }
}

/// The top of the inspector: a progress ring around the file icon, the name, metric tiles and the
/// main actions.
struct InspectorHero: View {
    let item: TaskItem
    let detail: TaskDetailData?
    @Environment(AppModel.self) private var model

    var body: some View {
        let d = item.data
        let p = d.progress
        let fraction = d.state == .completed ? 1 : p.effectiveFraction
        VStack(spacing: 16) {
            ProgressRing(fraction: fraction, tint: Theme.progressTint(d.state), lineWidth: 9) {
                FileBadge(name: d.name, kind: d.kind, size: 62)
            }
            .frame(width: 118, height: 118)
            .padding(.top, 8)

            VStack(spacing: 6) {
                Text(d.name)
                    .font(.system(size: 16, weight: .semibold))
                    .multilineTextAlignment(.center)
                    .lineLimit(3)
                    .textSelection(.enabled)
                HStack(spacing: 8) {
                    StatusCapsule(d.state, compact: true)
                    if let domain = d.domain, !domain.isEmpty {
                        Text(domain).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
            }

            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(Fmt.percent(fraction))
                    .font(Theme.numeral(34))
                    .contentTransition(.numericText(value: fraction))
                    .animation(.smooth, value: fraction)
                Text(d.state == .completed ? Fmt.bytes(p.total ?? p.downloaded) : "\(Fmt.bytes(p.downloaded)) of \(Fmt.bytes(p.total))")
                    .font(.system(size: 12, weight: .medium).monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            if d.state == .failed {
                Text(TaskStatusText.detail(d)).font(.system(size: 12)).foregroundStyle(Theme.danger).multilineTextAlignment(.center)
            }

            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 2), spacing: 8) {
                MetricTile(label: "Speed", value: speedValue(d), symbol: "speedometer")
                MetricTile(label: "ETA", value: d.state == .downloading ? Fmt.eta(p.etaSeconds) : "—", symbol: "timer")
                MetricTile(label: "Size", value: Fmt.bytes(p.total ?? (d.state == .completed ? p.downloaded : nil)), symbol: "externaldrive")
                MetricTile(label: d.kind.isTorrent ? "Peers" : "Connections", value: connectionsValue(d), symbol: "point.3.connected.trianglepath.dotted")
                MetricTile(label: "Source", value: sourceValue(d), symbol: "globe")
                MetricTile(label: "Saved to", value: savedTo(d), symbol: "folder")
            }

            VStack(spacing: 8) {
                primaryButton(d)
                HStack(spacing: 8) {
                    Button { Finder.reveal([d.targetPath]) } label: {
                        Label("Reveal", systemImage: "magnifyingglass").frame(maxWidth: .infinity)
                    }
                    .swoopGlassButton()
                    Button { copyLink(d) } label: {
                        Label("Copy Link", systemImage: "link").frame(maxWidth: .infinity)
                    }
                    .swoopGlassButton()
                    .disabled(link(d) == nil)
                    Menu { TaskActionsMenu(ids: [d.id]) } label: { Image(systemName: "ellipsis") }
                        .menuStyle(.button)
                        .menuIndicator(.hidden)
                        .fixedSize()
                        .swoopGlassButton()
                        .accessibilityLabel(Text("More actions"))
                }
                .controlSize(.large)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 6)
    }

    @ViewBuilder
    private func primaryButton(_ d: TaskRowData) -> some View {
        let primary: (title: String, symbol: String, action: () -> Void)? = {
            if d.state.canPause { return ("Pause", "pause.fill", { model.act(.pause, on: [d.id]) }) }
            if d.state == .failed || d.state == .cancelled { return ("Retry", "arrow.clockwise", { model.act(.retry, on: [d.id]) }) }
            if d.state.canResume { return ("Resume", "play.fill", { model.act(.resume, on: [d.id]) }) }
            if d.state == .completed || d.state == .seeding { return ("Open", "arrow.up.forward.app", { Finder.open(d.targetPath) }) }
            return nil
        }()
        if let primary {
            Button(action: primary.action) {
                Label(LocalizedStringKey(primary.title), systemImage: primary.symbol)
                    .font(.system(size: 14, weight: .semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 5)
            }
            .controlSize(.extraLarge)
            .swoopGlassButton(prominent: true)
        }
    }

    private func speedValue(_ d: TaskRowData) -> String {
        if d.state == .seeding { return "↑ " + Fmt.speed(d.progress.uploadSpeed, zero: "0 B/s") }
        return d.state == .downloading ? Fmt.speed(d.progress.speed, zero: "—") : "—"
    }

    private func connectionsValue(_ d: TaskRowData) -> String {
        if d.kind.isTorrent { return "\(d.progress.peers)" }
        let segments = detail?.segments.count ?? 0
        if segments > 0 { return "\(d.progress.activeConnections) / \(segments)" }
        return d.progress.activeConnections > 0 ? "\(d.progress.activeConnections)" : "—"
    }

    private func sourceValue(_ d: TaskRowData) -> String {
        if let domain = d.domain, !domain.isEmpty { return domain }
        if let u = link(d), let host = URL(string: u)?.host { return host }
        return d.kind.label
    }

    private func savedTo(_ d: TaskRowData) -> String {
        let dir = (d.targetPath as NSString).deletingLastPathComponent
        return dir.isEmpty ? "—" : (dir as NSString).lastPathComponent
    }

    private func link(_ d: TaskRowData) -> String? { d.url ?? detail?.urls.first }

    private func copyLink(_ d: TaskRowData) {
        guard let u = link(d) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(u, forType: .string)
        model.toast(.success, "Link copied")
    }
}

struct MetricTile: View {
    var label: String
    var value: String
    var symbol: String
    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 5) {
                Image(systemName: symbol).font(.system(size: 9, weight: .bold))
                Text(L10n.tr(label).uppercased()).font(.system(size: 9.5, weight: .semibold)).tracking(0.9)
            }
            .foregroundStyle(.secondary)
            .lineLimit(1)
            Text(value)
                .font(.system(size: 15, weight: .semibold, design: .rounded).monospacedDigit())
                .lineLimit(1)
                .truncationMode(.middle)
                .contentTransition(.numericText())
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(Theme.well, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

struct MultiSelectionSummary: View {
    let ids: [String]
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    var body: some View {
        let items = ids.compactMap { model.tasks[$0] }
        let total = items.reduce(UInt64(0)) { $0 + ($1.progress.total ?? 0) }
        let done = items.reduce(UInt64(0)) { $0 + $1.progress.downloaded }
        let speed = items.reduce(UInt64(0)) { $0 + $1.progress.speed }
        VStack(spacing: 18) {
            ZStack {
                ForEach(Array(items.prefix(3).enumerated()), id: \.offset) { i, item in
                    FileBadge(name: item.name, kind: item.kind, size: 58)
                        .background(Theme.card, in: RoundedRectangle(cornerRadius: 17, style: .continuous))
                        .rotationEffect(.degrees(Double(i - 1) * 12))
                        .offset(x: CGFloat(i - 1) * 22)
                }
            }
            .frame(height: 80)
            .padding(.top, 36)
            VStack(spacing: 2) {
                Text("\(items.count)").font(Theme.numeral(40))
                Text("downloads selected").font(.system(size: 13)).foregroundStyle(.secondary)
            }
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 2), spacing: 8) {
                MetricTile(label: "Downloaded", value: "\(Fmt.bytes(done)) of \(Fmt.bytes(total))", symbol: "arrow.down.circle")
                MetricTile(label: "Combined speed", value: Fmt.speed(speed, zero: "—"), symbol: "speedometer")
                MetricTile(label: "Active", value: "\(items.filter { $0.state.isActive }.count)", symbol: "bolt.horizontal")
                MetricTile(label: "Failed", value: "\(items.filter { $0.state == .failed }.count)", symbol: "exclamationmark.triangle")
            }
            HStack(spacing: 8) {
                Button { model.act(.pause, on: items.filter { $0.state.canPause }.map(\.id)) } label: {
                    Label("Pause", systemImage: "pause.fill").frame(maxWidth: .infinity)
                }
                .swoopGlassButton(prominent: true)
                Button { model.act(.resume, on: items.filter { $0.state.canResume }.map(\.id)) } label: {
                    Label("Resume", systemImage: "play.fill").frame(maxWidth: .infinity)
                }
                .swoopGlassButton()
            }
            .controlSize(.large)
            Button("Remove…", role: .destructive) { ui.removeConfirmation = ids }
                .buttonStyle(.borderless)
                .foregroundStyle(Theme.danger)
            Spacer()
        }
        .padding(18)
    }
}
