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

/// Which table columns fit the current width.
struct DownloadColumns: Equatable {
    var size = true
    var speed = true
    var eta = true
    var added = true
    var progress: CGFloat = 176

    static let status: CGFloat = 124
    static let sizeW: CGFloat = 104
    static let speedW: CGFloat = 92
    static let etaW: CGFloat = 72
    static let addedW: CGFloat = 78
    static let action: CGFloat = 34

    /// Keeps at least ~220 pt for names, adding columns in order of usefulness.
    static func fitting(_ width: CGFloat) -> DownloadColumns {
        var c = DownloadColumns(size: false, speed: false, eta: false, added: false, progress: width < 900 ? 140 : 176)
        var room = width - 60 - 52 - 220 - status - c.progress - action - 14 * 3
        func take(_ w: CGFloat) -> Bool {
            guard room >= w + 14 else { return false }
            room -= w + 14
            return true
        }
        c.speed = take(speedW)
        c.size = take(sizeW)
        c.eta = take(etaW)
        c.added = take(addedW)
        return c
    }
}

struct DownloadsView: View {
    let scope: DownloadScope
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @Environment(\.appDelegate) private var delegate
    @AppStorage("downloadSort") private var sortRaw = DownloadSort.manual.rawValue
    @ViewState private var anchor: String?
    @FocusState private var tableFocused: Bool

    private var sort: DownloadSort { DownloadSort(rawValue: sortRaw) ?? .manual }

    var body: some View {
        @Bindable var ui = ui
        let rows = visibleRows
        VStack(spacing: 0) {
            header(rows)
            content(rows)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    // MARK: header

    private func header(_ rows: [TaskItem]) -> some View {
        @Bindable var ui = ui
        return PageHeader(title, count: "\(scopeRows.count)", subtitle: subtitle) {
            GlassGroup(spacing: 10) {
                HStack(spacing: 10) {
                    GlassControlGroup {
                        GroupIconButton(symbol: "plus", help: "Add Download", tint: Theme.blue) { ui.openAdd() }
                        GroupIconButton(symbol: "pause.fill", help: "Pause All") { model.pauseAll() }
                        GroupIconButton(symbol: "play.fill", help: "Resume All") { model.resumeAll() }
                        filterMenu
                        GroupIconButton(symbol: "sidebar.trailing", help: "Show or hide the inspector (⌥⌘I)",
                                        tint: ui.showInspector ? Theme.blue : nil) { ui.showInspector.toggle() }
                    }
                    SearchCapsule(text: $ui.searchText, prompt: "Search downloads")
                }
            }
        } below: {
            if scope != .completed {
                filterChips
            }
        }
    }

    private var filterChips: some View {
        @Bindable var ui = ui
        let base = scopeRows
        return ScrollView(.horizontal) {
            HStack(spacing: 8) {
                ForEach(SmartFilter.allCases) { f in
                    let n = base.filter { f.matches($0) }.count
                    if f == .all || n > 0 || ui.smartFilter == f {
                        Button {
                            withAnimation(.smooth(duration: 0.25)) { ui.smartFilter = f }
                        } label: {
                            Chip(selected: ui.smartFilter == f) {
                                Text(LocalizedStringKey(f.title))
                                Text("\(n)")
                                    .font(.system(size: 12, weight: .semibold, design: .rounded).monospacedDigit())
                                    .opacity(0.7)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
                ForEach(ui.tokens) { token in
                    Button { ui.tokens.removeAll { $0.id == token.id } } label: {
                        Chip {
                            Image(systemName: token.symbol).font(.system(size: 11)).foregroundStyle(Theme.blue)
                            Text(token.label)
                            Image(systemName: "xmark").font(.system(size: 9, weight: .bold)).foregroundStyle(.secondary)
                        }
                    }
                    .buttonStyle(.plain)
                    .help("Remove filter")
                }
            }
        }
        .scrollIndicators(.never)
    }

    @ViewBuilder
    private func content(_ rows: [TaskItem]) -> some View {
        if model.status == .starting {
            ProgressView().controlSize(.small)
        } else if rows.isEmpty {
            emptyState
        } else {
            table(rows)
        }
    }

    @ViewBuilder
    private var emptyState: some View {
        @Bindable var ui = ui
        if !ui.searchText.isEmpty || !ui.tokens.isEmpty || (ui.smartFilter != .all && !scopeRows.isEmpty) {
            EmptyStateView("magnifyingglass", title: "No matches",
                           message: ui.searchText.isEmpty ? "Nothing fits this filter right now." : "Nothing matches “\(ui.searchText)”.") {
                Button("Clear Filters") {
                    ui.searchText = ""
                    ui.tokens = []
                    ui.smartFilter = .all
                }
                .ospreyGlassButton()
            }
        } else {
            switch scope {
            case .completed:
                IllustratedEmptyState(title: "Nothing finished yet", message: "Completed downloads gather here, ready to open or reveal in Finder.") {
                    EmptyView()
                }
            case .torrents:
                IllustratedEmptyState(title: "No torrents in the air",
                                      message: "Open a .torrent file or paste a magnet link. Osprey seeds politely and stops when you say so.") {
                    emptyActions(addTitle: "Add Torrent")
                }
            case .queue:
                IllustratedEmptyState(title: "This queue is clear", message: "Drag downloads onto the queue in the sidebar, or choose it when adding.") {
                    emptyActions(addTitle: "Add Download")
                }
            default:
                IllustratedEmptyState(title: "Ready when you are",
                                      message: "Paste a link, drop a file or torrent anywhere in this window, or send one from your browser.") {
                    emptyActions(addTitle: "Add Download")
                }
            }
        }
    }

    private func emptyActions(addTitle: String) -> some View {
        VStack(spacing: 14) {
            HStack(spacing: 12) {
                Button { ui.openAdd() } label: {
                    Label(LocalizedStringKey(addTitle), systemImage: "plus")
                }
                .buttonStyle(ProminentCapsuleStyle())
                Button { ui.openAdd(AddPrefill(autoPaste: true)) } label: {
                    Label("Paste Link", systemImage: "doc.on.clipboard")
                }
                .buttonStyle(SecondaryCapsuleStyle())
            }
            HStack(spacing: 14) {
                KeyHint(keys: "⌘N", text: "New download")
                KeyHint(keys: "⇧⌘V", text: "Add from clipboard")
                KeyHint(keys: "⌘O", text: "Open torrent")
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

    private var subtitle: String? {
        if case .queue(let id) = scope, let q = model.queue(id) {
            return q.paused ? L10n.tr("Paused") : String(format: L10n.tr("Up to %d at once"), Int(q.maxConcurrent))
        }
        let rows = scopeRows
        let moving = rows.filter { $0.state == .downloading }
        if moving.isEmpty {
            let done = rows.filter { $0.state == .completed }.count
            return rows.isEmpty ? nil : String(format: L10n.tr("%d finished · nothing transferring"), done)
        }
        let left = moving.reduce(UInt64(0)) { acc, t in
            let p = t.progress
            return acc + ((p.total ?? 0) > p.downloaded ? (p.total ?? 0) - p.downloaded : 0)
        }
        return String(format: L10n.tr("%d downloading · %@ to go"), moving.count, Fmt.bytes(left))
    }

    // MARK: filtering & sorting

    /// Rows in scope before the chip filter, search and tokens.
    private var scopeRows: [TaskItem] {
        model.tasks.items.filter { item in
            switch scope {
            case .active: return true
            case .completed: return item.state == .completed
            case .torrents: return item.kind.isTorrent
            case .queue(let id): return item.data.queueId == id
            }
        }
    }

    private var visibleRows: [TaskItem] {
        let text = ui.searchText.trimmingCharacters(in: .whitespaces).lowercased()
        var rows = scopeRows.filter { item in
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
        return GroupIconMenu(symbol: ui.smartFilter == .all && ui.tokens.isEmpty ? "line.3.horizontal.decrease" : "line.3.horizontal.decrease.circle.fill",
                             help: "Filter and sort") {
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
            Divider()
            Menu("Narrow To") {
                if !model.queues.isEmpty {
                    Section("Queue") {
                        ForEach(model.queues) { q in Button(q.name) { addToken(FilterToken(kind: .queue, value: q.id, label: q.name)) } }
                    }
                }
                if !model.categories.isEmpty {
                    Section("Category") {
                        ForEach(model.categories) { c in Button(c.name) { addToken(FilterToken(kind: .category, value: c.id, label: c.name)) } }
                    }
                }
                let domains = Array(Set(model.tasks.items.compactMap(\.data.domain))).sorted().prefix(8)
                if !domains.isEmpty {
                    Section("Site") {
                        ForEach(Array(domains), id: \.self) { d in Button(d) { addToken(FilterToken(kind: .domain, value: d, label: d)) } }
                    }
                }
                Section {
                    Button("Added Today") { addToken(FilterToken(kind: .date, value: "1", label: "Today")) }
                    Button("Larger than 1 GB") { addToken(FilterToken(kind: .size, value: "1000000000", label: "> 1 GB")) }
                }
            }
            if !ui.tokens.isEmpty {
                Button("Clear Narrowing") { ui.tokens = [] }
            }
        }
    }

    private func addToken(_ t: FilterToken) {
        if !ui.tokens.contains(t) { ui.tokens.append(t) }
    }

    // MARK: table

    private func table(_ rows: [TaskItem]) -> some View {
        let reorderable = sort == .manual && scope != .completed && ui.searchText.isEmpty && ui.tokens.isEmpty && ui.smartFilter == .all
        return GeometryReader { geo in
            let cols = DownloadColumns.fitting(geo.size.width)
            VStack(alignment: .leading, spacing: 0) {
                columnHeader(cols)
                ScrollView {
                    LazyVStack(spacing: 4) {
                        ForEach(rows) { item in
                            DownloadTableRow(item: item, columns: cols, selected: ui.selection.contains(item.id))
                                .onTapGesture { click(item, rows: rows) }
                                .contextMenu { TaskActionsMenu(ids: menuIds(item)) }
                                .onDrag { NSItemProvider(object: item.id as NSString) }
                                .dropDestination(for: String.self) { ids, _ in
                                    guard reorderable else { return false }
                                    move(ids, before: item, rows: rows)
                                    return true
                                }
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.bottom, 16)
                }
                .scrollIndicators(.automatic)
                .frame(maxHeight: .infinity, alignment: .top)
            }
            .frame(width: geo.size.width, height: geo.size.height, alignment: .top)
        }
        .focusable()
        .focused($tableFocused)
        .focusEffectDisabled()
        .onKeyPress(.space) {
            delegate?.quickLook.toggle(paths: paths(ui.selection))
            return .handled
        }
        .onKeyPress(.upArrow) { step(-1, rows: rows); return .handled }
        .onKeyPress(.downArrow) { step(1, rows: rows); return .handled }
        .onKeyPress(.return) { openPrimary(Array(ui.selection)); return .handled }
        .onKeyPress(characters: .init(charactersIn: "a"), phases: .down) { press in
            guard press.modifiers.contains(.command) else { return .ignored }
            ui.selection = Set(rows.map(\.id))
            return .handled
        }
        .onDeleteCommand { if !ui.selection.isEmpty { ui.removeConfirmation = Array(ui.selection) } }
    }

    private func columnHeader(_ cols: DownloadColumns) -> some View {
        HStack(spacing: 14) {
            headerLabel("Name", sort: .name).frame(maxWidth: .infinity, alignment: .leading).padding(.leading, 48)
            headerLabel("Status", sort: nil).frame(width: DownloadColumns.status, alignment: .leading)
            headerLabel("Progress", sort: .progress).frame(width: cols.progress, alignment: .leading)
            if cols.size { headerLabel("Size", sort: .size).frame(width: DownloadColumns.sizeW, alignment: .trailing) }
            if cols.speed { headerLabel("Speed", sort: nil).frame(width: DownloadColumns.speedW, alignment: .trailing) }
            if cols.eta { headerLabel("ETA", sort: nil).frame(width: DownloadColumns.etaW, alignment: .trailing) }
            if cols.added { headerLabel("Added", sort: .added).frame(width: DownloadColumns.addedW, alignment: .trailing) }
            Color.clear.frame(width: DownloadColumns.action, height: 1)
        }
        .padding(.horizontal, 30)
        .padding(.vertical, 8)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.hairline).frame(height: 1).padding(.horizontal, 28) }
    }

    @ViewBuilder
    private func headerLabel(_ text: String, sort key: DownloadSort?) -> some View {
        let active = key != nil && sort == key
        let label = HStack(spacing: 4) {
            Text(L10n.tr(text).uppercased())
                .font(.system(size: 10.5, weight: .semibold))
                .tracking(1)
            if active { Image(systemName: "chevron.down").font(.system(size: 8, weight: .bold)) }
        }
        .foregroundStyle(active ? AnyShapeStyle(Theme.blue) : AnyShapeStyle(.secondary))
        if let key {
            Button { sortRaw = (sort == key ? DownloadSort.manual : key).rawValue } label: { label }
                .buttonStyle(.plain)
                .help(Text("Sort by \(L10n.tr(key.title))"))
        } else {
            label
        }
    }

    // MARK: selection

    private func click(_ item: TaskItem, rows: [TaskItem]) {
        tableFocused = true
        if (NSApp.currentEvent?.clickCount ?? 1) >= 2 {
            openPrimary([item.id])
            return
        }
        let flags = NSEvent.modifierFlags
        if flags.contains(.command) {
            if ui.selection.contains(item.id) { ui.selection.remove(item.id) } else { ui.selection.insert(item.id) }
            anchor = item.id
        } else if flags.contains(.shift), let a = anchor, let i = rows.firstIndex(where: { $0.id == a }),
                  let j = rows.firstIndex(where: { $0.id == item.id }) {
            ui.selection = Set(rows[min(i, j)...max(i, j)].map(\.id))
        } else {
            ui.selection = [item.id]
            anchor = item.id
        }
    }

    private func step(_ delta: Int, rows: [TaskItem]) {
        guard !rows.isEmpty else { return }
        let current = rows.firstIndex { $0.id == anchor } ?? (delta > 0 ? -1 : rows.count)
        let next = max(0, min(rows.count - 1, current + delta))
        ui.selection = [rows[next].id]
        anchor = rows[next].id
    }

    private func menuIds(_ item: TaskItem) -> [String] {
        ui.selection.contains(item.id) ? Array(ui.selection) : [item.id]
    }

    private func move(_ ids: [String], before target: TaskItem, rows: [TaskItem]) {
        let moving = ids.filter { $0 != target.id && model.tasks[$0] != nil }
        guard !moving.isEmpty else { return }
        let remaining = rows.filter { !moving.contains($0.id) }
        guard let idx = remaining.firstIndex(where: { $0.id == target.id }) else { return }
        let after: String? = idx > 0 ? remaining[idx - 1].id : nil
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

struct KeyHint: View {
    var keys: String
    var text: String
    var body: some View {
        HStack(spacing: 6) {
            Text(keys)
                .font(.system(size: 11, weight: .semibold, design: .rounded))
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Theme.well, in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous).strokeBorder(Theme.hairline, lineWidth: 1))
            Text(LocalizedStringKey(text)).font(.system(size: 12))
        }
        .foregroundStyle(.secondary)
    }
}

// MARK: - Row

struct DownloadTableRow: View {
    let item: TaskItem
    let columns: DownloadColumns
    let selected: Bool
    @Environment(AppModel.self) private var model
    @ViewState private var hovering = false

    var body: some View {
        let d = item.data
        let p = d.progress
        let shape = RoundedRectangle(cornerRadius: 14, style: .continuous)
        HStack(spacing: 14) {
            HStack(spacing: 12) {
                FileBadge(name: d.name, kind: d.kind, size: 38)
                VStack(alignment: .leading, spacing: 2) {
                    Text(d.name)
                        .font(.system(size: 14, weight: .semibold))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(secondaryLine(d))
                        .font(.system(size: 12))
                        .foregroundStyle(d.state == .failed ? AnyShapeStyle(Theme.danger) : AnyShapeStyle(.secondary))
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            StatusCapsule(d.state)
                .frame(width: DownloadColumns.status, alignment: .leading)

            ProgressWithPercent(fraction: d.state == .completed ? 1 : p.effectiveFraction,
                                tint: Theme.progressTint(d.state),
                                indeterminate: (d.state == .resolving || d.state == .connecting || d.state == .downloading)
                                    && p.total == nil && p.fraction == 0)
                .frame(width: columns.progress)

            if columns.size {
                VStack(alignment: .trailing, spacing: 1) {
                    Text(Fmt.bytes(p.total ?? (d.state == .completed ? p.downloaded : nil)))
                        .font(.system(size: 13, weight: .medium).monospacedDigit())
                    if d.state != .completed, p.downloaded > 0 {
                        Text(Fmt.bytes(p.downloaded))
                            .font(.system(size: 11).monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }
                .frame(width: DownloadColumns.sizeW, alignment: .trailing)
            }
            if columns.speed {
                Text(speedText(d))
                    .font(.system(size: 13, weight: .medium).monospacedDigit())
                    .foregroundStyle(p.speed > 0 || p.uploadSpeed > 0 ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
                    .contentTransition(.numericText(value: Double(p.speed)))
                    .frame(width: DownloadColumns.speedW, alignment: .trailing)
            }
            if columns.eta {
                Text(d.state == .downloading ? Fmt.eta(p.etaSeconds) : "—")
                    .font(.system(size: 13).monospacedDigit())
                    .foregroundStyle(d.state == .downloading ? AnyShapeStyle(.secondary) : AnyShapeStyle(.tertiary))
                    .frame(width: DownloadColumns.etaW, alignment: .trailing)
            }
            if columns.added {
                Text(ShortDate.string(d.createdAt))
                    .font(.system(size: 12).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(width: DownloadColumns.addedW, alignment: .trailing)
            }
            actionButton(d)
                .frame(width: DownloadColumns.action)
                .opacity(hovering || selected ? 1 : 0)
        }
        .padding(.horizontal, 14)
        .frame(height: 62)
        .background {
            if selected {
                shape.fill(Theme.rowSelected)
                    .overlay(shape.strokeBorder(Theme.blue.opacity(0.35), lineWidth: 1))
            } else if hovering {
                shape.fill(Theme.rowHover)
            }
        }
        .contentShape(shape)
        .onHover { h in withAnimation(.easeOut(duration: 0.12)) { hovering = h } }
        .animation(.easeOut(duration: 0.15), value: selected)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(d.name))
        .accessibilityValue(Text("\(L10n.state(d.state)), \(TaskStatusText.detail(d))"))
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func secondaryLine(_ d: TaskRowData) -> String {
        if d.state == .failed { return TaskStatusText.detail(d) }
        if let domain = d.domain, !domain.isEmpty { return domain }
        return d.kind.isTorrent ? L10n.tr("Torrent") : d.kind.label
    }

    private func speedText(_ d: TaskRowData) -> String {
        if d.state == .seeding { return "↑ " + Fmt.speed(d.progress.uploadSpeed, zero: "0 B/s") }
        return d.state == .downloading ? Fmt.speed(d.progress.speed, zero: "—") : "—"
    }

    @ViewBuilder
    private func actionButton(_ d: TaskRowData) -> some View {
        let (symbol, label, action): (String, String, () -> Void) = {
            switch d.state {
            case .completed: return ("magnifyingglass", "Show in Finder", { Finder.reveal([d.targetPath]) })
            case .failed, .cancelled: return ("arrow.clockwise", "Retry", { model.act(.retry, on: [d.id]) })
            case .paused, .pending: return ("play.fill", "Resume", { model.act(.resume, on: [d.id]) })
            default: return ("pause.fill", "Pause", { model.act(.pause, on: [d.id]) })
            }
        }()
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(Theme.blue)
                .frame(width: 30, height: 30)
                .background(Theme.blue.opacity(0.12), in: Circle())
        }
        .buttonStyle(.plain)
        .help(Text(LocalizedStringKey(label)))
        .accessibilityLabel(Text(LocalizedStringKey(label)))
    }
}
