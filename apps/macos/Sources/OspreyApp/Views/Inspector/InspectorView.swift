import AppKit
import OspreyKit
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
                VStack(spacing: 0) {
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
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    Form {
                        InspectorHeader(item: item)
                        tabContent(item)
                    }
                    .formStyle(.grouped)
                }
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
                ContentUnavailableView("No Selection", systemImage: "sidebar.trailing",
                                       description: Text("Select a download to see its details."))
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    @ViewBuilder
    private func tabContent(_ item: TaskItem) -> some View {
        if let error = loader.error, loader.detail == nil {
            Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(Theme.warning) }
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

struct InspectorHeader: View {
    let item: TaskItem
    @Environment(AppModel.self) private var model

    var body: some View {
        let d = item.data
        Section {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .top, spacing: 10) {
                    Image(nsImage: FileIcons.icon(for: d.name, kind: d.kind))
                        .resizable()
                        .frame(width: 44, height: 44)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(d.name).font(.headline).lineLimit(3).textSelection(.enabled)
                        Text(TaskStatusText.detail(d))
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(d.state == .failed ? AnyShapeStyle(Theme.danger) : AnyShapeStyle(.secondary))
                    }
                }
                if d.state != .completed {
                    ThinProgress(fraction: d.progress.effectiveFraction, tint: Theme.progressTint(d.state), height: 4)
                }
                HStack(spacing: 8) {
                    if d.state.canPause {
                        Button("Pause") { model.act(.pause, on: [d.id]) }
                    } else if d.state == .failed || d.state == .cancelled {
                        Button("Retry") { model.act(.retry, on: [d.id]) }
                    } else if d.state.canResume {
                        Button("Resume") { model.act(.resume, on: [d.id]) }
                    }
                    if d.state == .completed || d.state == .seeding {
                        Button("Open") { Finder.open(d.targetPath) }
                    }
                    Button("Show in Finder") { Finder.reveal([d.targetPath]) }
                    Spacer()
                    Menu { TaskActionsMenu(ids: [d.id]) } label: { Image(systemName: "ellipsis.circle") }
                        .menuStyle(.borderlessButton)
                        .menuIndicator(.hidden)
                        .fixedSize()
                        .accessibilityLabel(Text("More actions"))
                }
                .controlSize(.small)
            }
            .padding(.vertical, 4)
        }
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
        Form {
            Section {
                LabeledContent("Selected", value: "\(items.count)")
                LabeledContent("Downloaded", value: "\(Fmt.bytes(done)) of \(Fmt.bytes(total))")
                LabeledContent("Combined speed", value: Fmt.speed(speed, zero: "—"))
                LabeledContent("Active", value: "\(items.filter { $0.state.isActive }.count)")
                LabeledContent("Failed", value: "\(items.filter { $0.state == .failed }.count)")
            }
            Section {
                HStack {
                    Button("Pause") { model.act(.pause, on: items.filter { $0.state.canPause }.map(\.id)) }
                    Button("Resume") { model.act(.resume, on: items.filter { $0.state.canResume }.map(\.id)) }
                    Spacer()
                    Button("Remove…", role: .destructive) { ui.removeConfirmation = ids }
                }
            }
        }
        .formStyle(.grouped)
    }
}
