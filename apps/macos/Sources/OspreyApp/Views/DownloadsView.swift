import AppKit
import OspreyKit
import SwiftUI

enum DownloadScope: Hashable {
    case active, completed, torrents
    case queue(String)
}

enum DownloadSort: String, CaseIterable, Identifiable {
    case manual, added, name, size, progress
    var id: String { rawValue }
    var title: String {
        switch self {
        case .manual: return "Queue Order"
        case .added: return "Date Added"
        case .name: return "Name"
        case .size: return "Size"
        case .progress: return "Progress"
        }
    }
}

struct DownloadsView: View {
    let scope: DownloadScope
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @Environment(\.appDelegate) private var delegate
    @AppStorage("downloadSort") private var sortRaw = DownloadSort.manual.rawValue

    private var sort: DownloadSort { DownloadSort(rawValue: sortRaw) ?? .manual }

    var body: some View {
        @Bindable var ui = ui
        let rows = visibleRows
        content(rows)
            .navigationTitle(LocalizedStringKey(title))
            .navigationSubtitle(subtitle(rows))
            .searchable(text: $ui.searchText, tokens: $ui.tokens, placement: .toolbar, prompt: Text("Search")) { token in
                Label(token.label, systemImage: token.symbol)
            }
            .searchSuggestions { suggestions }
            .toolbar {
                ToolbarItem(placement: .automatic) { filterMenu }
            }
    }

    @ViewBuilder
    private func content(_ rows: [TaskItem]) -> some View {
        if model.status == .starting {
            ProgressView().controlSize(.small).frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if rows.isEmpty {
            emptyState
        } else {
            list(rows)
        }
    }

    @ViewBuilder
    private var emptyState: some View {
        if !ui.searchText.isEmpty || !ui.tokens.isEmpty || ui.smartFilter != .all {
            ContentUnavailableView.search(text: ui.searchText)
        } else {
            switch scope {
            case .completed:
                ContentUnavailableView("No Completed Downloads", systemImage: "checkmark.circle",
                                       description: Text("Finished downloads appear here."))
            case .torrents:
                ContentUnavailableView {
                    Label("No Torrents", systemImage: "point.3.connected.trianglepath.dotted")
                } description: {
                    Text("Open a .torrent file or paste a magnet link.")
                } actions: {
                    Button("Add Torrent…") { ui.openAdd() }
                }
            default:
                ContentUnavailableView {
                    Label("No Downloads", systemImage: "arrow.down.circle")
                } description: {
                    Text("Paste a link, drop a file, or use the browser extension.")
                } actions: {
                    Button("Add Download…") { ui.openAdd() }
                }
            }
        }
    }

    private var title: String {
        switch scope {
        case .active: return "Downloads"
        case .completed: return "Completed"
        case .torrents: return "Torrents"
        case .queue(let id): return model.queue(id)?.name ?? "Queue"
        }
    }

    private func subtitle(_ rows: [TaskItem]) -> String {
        if case .queue(let id) = scope, let q = model.queue(id) {
            return q.paused ? L10n.tr("Paused") : String(format: L10n.tr("Up to %d at once"), Int(q.maxConcurrent))
        }
        return rows.count == 1 ? L10n.tr("1 item") : String(format: L10n.tr("%d items"), rows.count)
    }

    // MARK: filtering & sorting

    private var visibleRows: [TaskItem] {
        let text = ui.searchText.trimmingCharacters(in: .whitespaces).lowercased()
        var rows = model.tasks.items.filter { item in
            switch scope {
            case .active: if item.state == .completed { return false }
            case .completed: if item.state != .completed { return false }
            case .torrents: if !item.kind.isTorrent { return false }
            case .queue(let id): if item.data.queueId != id || item.state == .completed { return false }
            }
            if scope != .completed, !ui.smartFilter.matches(item) { return false }
            if !text.isEmpty {
                let d = item.data
                let hay = [d.name, d.domain ?? "", d.url ?? "", d.tags.joined(separator: " ")].joined(separator: " ").lowercased()
                if !hay.contains(text) { return false }
            }
            for token in ui.tokens where !matches(token, item) { return false }
            return true
        }
        switch scope == .completed && sort == .manual ? .added : sort {
        case .manual: rows.sort { $0.position < $1.position }
        case .added:
            if scope == .completed {
                rows.sort { ($0.data.completedAt ?? $0.createdAt) > ($1.data.completedAt ?? $1.createdAt) }
            } else {
                rows.sort { $0.createdAt > $1.createdAt }
            }
        case .name: rows.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        case .size: rows.sort { $0.sizeKey > $1.sizeKey }
        case .progress: rows.sort { $0.fraction > $1.fraction }
        }
        return rows
    }

    private func matches(_ token: FilterToken, _ item: TaskItem) -> Bool {
        let d = item.data
        switch token.kind {
        case .queue: return d.queueId == token.value
        case .category: return d.categoryId == token.value
        case .domain: return (d.domain ?? "").hasSuffix(token.value)
        case .tag: return d.tags.contains(token.value)
        case .date: return d.createdAt >= Date().millis - (Int64(token.value) ?? 1) * 86_400_000
        case .size: return (d.progress.total ?? 0) >= (UInt64(token.value) ?? 0)
        }
    }

    private var filterMenu: some View {
        @Bindable var ui = ui
        return Menu {
            if scope != .completed {
                Picker("Show", selection: $ui.smartFilter) {
                    ForEach(SmartFilter.allCases) { Text(LocalizedStringKey($0.title)).tag($0) }
                }
                .pickerStyle(.inline)
            }
            Picker("Sort By", selection: $sortRaw) {
                ForEach(DownloadSort.allCases) { Text(LocalizedStringKey($0.title)).tag($0.rawValue) }
            }
            .pickerStyle(.inline)
        } label: {
            Label("Filter", systemImage: ui.smartFilter == .all ? "line.3.horizontal.decrease.circle" : "line.3.horizontal.decrease.circle.fill")
        }
        .help("Filter and sort")
    }

    @ViewBuilder
    private var suggestions: some View {
        let text = ui.searchText.lowercased()
        let queues = model.queues.filter { text.isEmpty || $0.name.lowercased().contains(text) }.prefix(4)
        let cats = model.categories.filter { text.isEmpty || $0.name.lowercased().contains(text) }.prefix(4)
        let domains = Array(Set(model.tasks.items.compactMap(\.data.domain))).filter { !text.isEmpty && $0.contains(text) }.sorted().prefix(4)
        ForEach(Array(queues)) { q in
            Label(q.name, systemImage: "tray").searchCompletion(FilterToken(kind: .queue, value: q.id, label: q.name))
        }
        ForEach(Array(cats)) { c in
            Label(c.name, systemImage: "square.grid.2x2").searchCompletion(FilterToken(kind: .category, value: c.id, label: c.name))
        }
        ForEach(Array(domains), id: \.self) { d in
            Label(d, systemImage: "globe").searchCompletion(FilterToken(kind: .domain, value: d, label: d))
        }
        if text.isEmpty {
            Label("Added Today", systemImage: "calendar").searchCompletion(FilterToken(kind: .date, value: "1", label: "Today"))
            Label("Larger than 1 GB", systemImage: "externaldrive").searchCompletion(FilterToken(kind: .size, value: "1000000000", label: "> 1 GB"))
        }
    }

    // MARK: list

    private func list(_ rows: [TaskItem]) -> some View {
        @Bindable var ui = ui
        let reorderable = sort == .manual && scope != .completed && ui.searchText.isEmpty && ui.tokens.isEmpty && ui.smartFilter == .all
        return List(selection: $ui.selection) {
            ForEach(rows) { item in
                DownloadRow(item: item)
                    .tag(item.id)
                    .itemProvider { NSItemProvider(object: item.id as NSString) }
            }
            .onMove(perform: reorderable ? { from, to in move(rows, from: from, to: to) } : nil)
        }
        .listStyle(.inset)
        .alternatingRowBackgrounds(.disabled)
        .contextMenu(forSelectionType: String.self) { ids in
            TaskActionsMenu(ids: Array(ids))
        } primaryAction: { ids in
            openPrimary(Array(ids))
        }
        .onKeyPress(.space) {
            delegate?.quickLook.toggle(paths: paths(ui.selection))
            return .handled
        }
        .onDeleteCommand { if !ui.selection.isEmpty { ui.removeConfirmation = Array(ui.selection) } }
    }

    private func move(_ rows: [TaskItem], from: IndexSet, to: Int) {
        let moving = from.map { rows[$0].id }
        let remaining = rows.enumerated().filter { !from.contains($0.offset) }.map(\.element)
        let insertAt = to - from.filter { $0 < to }.count
        let after: String? = insertAt > 0 ? remaining[insertAt - 1].id : nil
        Task { await model.perform("Couldn't reorder") { try await model.engine.reorderTasks(moving, after: after) } }
    }

    private func paths(_ ids: Set<String>) -> [String] {
        ids.compactMap { model.tasks[$0]?.data }.map(\.targetPath)
    }

    private func openPrimary(_ ids: [String]) {
        guard let id = ids.first, let item = model.tasks[id] else { return }
        if item.state == .completed || item.state == .seeding {
            Finder.open(item.data.targetPath)
        } else {
            ui.selection = Set(ids)
            ui.showInspector = true
        }
    }
}

extension TaskRowData {
    /// Where the file is (or will be) on disk.
    var targetPath: String { filePath ?? (directory as NSString).appendingPathComponent(name) }
}

// MARK: - Row

struct DownloadRow: View {
    let item: TaskItem
    @Environment(AppModel.self) private var model
    @ViewState private var hovering = false

    var body: some View {
        let d = item.data
        let active = d.state == .downloading || d.state == .seeding
        HStack(spacing: 12) {
            Image(nsImage: FileIcons.icon(for: d.name, kind: d.kind))
                .resizable()
                .interpolation(.high)
                .frame(width: 32, height: 32)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 5) {
                    Text(d.name)
                        .font(.body.weight(active ? .semibold : .regular))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if d.state == .completed {
                        Image(systemName: "checkmark.circle.fill")
                            .foregroundStyle(Theme.success)
                            .imageScale(.small)
                            .symbolEffect(.bounce, value: d.completedAt)
                    }
                }
                Text(TaskStatusText.detail(d))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(d.state == .failed ? AnyShapeStyle(Theme.danger) : AnyShapeStyle(.secondary))
                    .lineLimit(1)
                    .contentTransition(.numericText())
                if d.state != .completed {
                    ThinProgress(fraction: d.progress.effectiveFraction,
                                 tint: Theme.progressTint(d.state),
                                 indeterminate: (d.state == .resolving || d.state == .connecting || d.state == .downloading)
                                     && d.progress.total == nil && d.progress.fraction == 0)
                        .padding(.top, 2)
                }
            }
            Spacer(minLength: 8)
            actionButton(d)
                .opacity(hovering ? 1 : 0)
        }
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(d.name))
        .accessibilityValue(Text("\(L10n.state(d.state)), \(TaskStatusText.detail(d))"))
    }

    @ViewBuilder
    private func actionButton(_ d: TaskRowData) -> some View {
        let (symbol, label, action): (String, String, () -> Void) = {
            switch d.state {
            case .completed: return ("magnifyingglass.circle", "Show in Finder", { Finder.reveal([d.targetPath]) })
            case .failed, .cancelled: return ("arrow.clockwise.circle", "Retry", { model.act(.retry, on: [d.id]) })
            case .paused, .pending: return ("play.circle", "Resume", { model.act(.resume, on: [d.id]) })
            default: return ("pause.circle", "Pause", { model.act(.pause, on: [d.id]) })
            }
        }()
        Button(action: action) {
            Image(systemName: symbol).font(.title2).fontWeight(.light)
        }
        .buttonStyle(.borderless)
        .foregroundStyle(.secondary)
        .help(Text(LocalizedStringKey(label)))
        .accessibilityLabel(Text(LocalizedStringKey(label)))
    }
}
