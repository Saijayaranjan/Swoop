import AppKit
import SwoopKit
import SwiftUI

struct HistoryView: View {
    @Environment(AppModel.self) private var model
    @ViewState private var entries: [HistoryEntryData] = []
    @ViewState private var total: UInt32 = 0
    @ViewState private var search = ""
    @ViewState private var stateFilter = "all"
    @ViewState private var selection: Set<String> = []
    @ViewState private var sortOrder: [KeyPathComparator<HistoryEntryData>] = [KeyPathComparator(\HistoryEntryData.finishedAt, order: .reverse)]
    @ViewState private var loading = false
    @ViewState private var confirmClear = false

    var body: some View {
        VStack(spacing: 0) {
            PageHeader("History", count: "\(total)", subtitle: L10n.tr("Everything Swoop has finished, failed or cancelled — searchable and ready to fetch again.")) {
                HStack(spacing: 10) {
                    GlassControlGroup {
                        GroupIconMenu(symbol: "square.and.arrow.up", help: "Export") {
                            Button("Export as CSV…") { export(csv: true) }
                            Button("Export as JSON…") { export(csv: false) }
                        }
                        GroupIconButton(symbol: "trash", help: "Clear History") { confirmClear = true }
                    }
                    SearchCapsule(text: $search, prompt: "Search history")
                }
            } below: {
                HStack(spacing: 8) {
                    ForEach([("all", "All"), ("completed", "Completed"), ("failed", "Failed"), ("cancelled", "Cancelled")], id: \.0) { key, title in
                        Button { withAnimation(.smooth(duration: 0.25)) { stateFilter = key } } label: {
                            Chip(selected: stateFilter == key) { Text(LocalizedStringKey(title)) }
                        }
                        .buttonStyle(.plain)
                    }
                    Spacer()
                    Text("Showing \(entries.count) of \(total)").font(.system(size: 12).monospacedDigit()).foregroundStyle(.secondary)
                }
            }
            Group {
                if entries.isEmpty && !loading {
                    EmptyStateView("clock.arrow.circlepath", title: search.isEmpty ? "No history yet" : "No matches",
                                   message: search.isEmpty ? "Finished and failed downloads are recorded here so you can find or fetch them again." : "Try a different search.")
                } else {
                    table
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .cardSurface(cornerRadius: 20, padding: 0)
            .padding(.horizontal, 28)
            .padding(.bottom, 20)
        }
        .task(id: "\(search)|\(stateFilter)|\(sortKey)|\(model.historyVersion)") {
            try? await Task.sleep(nanoseconds: 200_000_000)
            await load()
        }
        .confirmationDialog("Clear all history?", isPresented: $confirmClear, titleVisibility: .visible) {
            Button("Clear History", role: .destructive) {
                Task {
                    await model.perform("Couldn't clear history") { _ = try await model.engine.clearHistory() }
                    await load()
                }
            }
        } message: {
            Text("Downloaded files are not affected.")
        }
    }

    private var sortKey: String {
        guard let first = sortOrder.first else { return "finished_at" }
        let dir = first.order == .reverse ? "-" : "+"
        switch first.keyPath {
        case \HistoryEntryData.name: return dir + "name"
        case \HistoryEntryData.sizeKey: return dir + "size"
        case \HistoryEntryData.domain: return dir + "domain"
        case \HistoryEntryData.durationSeconds: return dir + "duration"
        case \HistoryEntryData.averageSpeed: return dir + "speed"
        default: return dir + "finished_at"
        }
    }

    private var table: some View {
        Table(entries, selection: $selection, sortOrder: $sortOrder) {
            TableColumn("Name", value: \.name) { e in
                HStack(spacing: 10) {
                    FileBadge(name: e.name, kind: e.kind, size: 28)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(e.name).lineLimit(1).truncationMode(.middle)
                        if let err = e.error { Text(err).font(.caption).foregroundStyle(Theme.danger).lineLimit(1) }
                    }
                }
                .padding(.vertical, 5)
            }
            .width(min: 220, ideal: 320)
            TableColumn("Site", value: \.domain) { e in Text(e.domain).foregroundStyle(.secondary) }.width(min: 90, ideal: 140)
            TableColumn("Size", value: \.sizeKey) { e in Text(Fmt.bytes(e.size)).monospacedDigit() }.width(min: 60, ideal: 80)
            TableColumn("Finished", value: \.finishedAt) { e in Text(Fmt.date(e.finishedAt)).monospacedDigit() }.width(min: 110, ideal: 150)
            TableColumn("Duration", value: \.durationSeconds) { e in Text(Fmt.duration(e.durationSeconds)).monospacedDigit().foregroundStyle(.secondary) }.width(min: 60, ideal: 80)
            TableColumn("Avg speed", value: \.averageSpeed) { e in Text(Fmt.speed(e.averageSpeed)).monospacedDigit().foregroundStyle(.secondary) }.width(min: 70, ideal: 90)
            TableColumn("Result") { e in StatePill(e.state) }.width(min: 90, ideal: 110)
        }
        .scrollContentBackground(.hidden)
        .alternatingRowBackgrounds(.disabled)
        .contextMenu(forSelectionType: String.self) { ids in
            let items = entries.filter { ids.contains($0.taskId) }
            Button("Download Again") { redownload(items) }
            Button("Show in Finder") { Finder.reveal(items.map(\.destination)) }
            Button("Copy Link") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(items.map(\.originalUrl).joined(separator: "\n"), forType: .string)
            }
            Divider()
            Button("Delete from History", role: .destructive) {
                Task {
                    await model.perform("Couldn't delete") { _ = try await model.engine.deleteHistory(items.map(\.taskId)) }
                    await load()
                }
            }
        } primaryAction: { ids in
            let items = entries.filter { ids.contains($0.taskId) }
            if let first = items.first, FileManager.default.fileExists(atPath: first.destination) { Finder.open(first.destination) }
        }
        .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
    }

    private func load() async {
        if let sample = SnapshotSample.history {
            entries = sample
            total = UInt32(sample.count)
            return
        }
        loading = true
        defer { loading = false }
        var q = HistoryQueryData()
        q.text = search.isEmpty ? nil : search
        q.state = stateFilter == "all" ? nil : TaskState(rawValue: stateFilter)
        let key = sortKey
        q.sort = String(key.dropFirst())
        q.descending = key.hasPrefix("-")
        q.limit = 1000
        if let list = await model.perform("Couldn't load history", { try await model.engine.history(q) }) { entries = list }
        var countQuery = q
        countQuery.limit = 0
        total = (try? await model.engine.historyCount(countQuery)) ?? UInt32(entries.count)
    }

    private func redownload(_ items: [HistoryEntryData]) {
        Task {
            var added = 0
            for e in items {
                var req = NewTaskRequestData()
                if e.originalUrl.lowercased().hasPrefix("magnet:") { req.magnet = e.originalUrl } else { req.url = e.originalUrl }
                req.origin = "history"
                if await model.perform("Couldn't download again", { try await model.engine.addTask(req) }) != nil { added += 1 }
            }
            if added > 0 { model.toast(.success, "Added \(added) download\(added == 1 ? "" : "s")") }
        }
    }

    private func export(csv: Bool) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = csv ? "Swoop History.csv" : "Swoop History.json"
        panel.allowedContentTypes = [csv ? .commaSeparatedText : .json]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task {
            do {
                if csv {
                    func esc(_ s: String) -> String { "\"" + s.replacingOccurrences(of: "\"", with: "\"\"") + "\"" }
                    var out = "name,url,domain,size,state,finished,duration_seconds,average_speed,destination,checksum\n"
                    for e in entries {
                        out += [esc(e.name), esc(e.originalUrl), esc(e.domain), e.size.map(String.init) ?? "", e.state.rawValue,
                                ISO8601DateFormatter().string(from: Date(millis: e.finishedAt)), "\(e.durationSeconds)",
                                "\(e.averageSpeed)", esc(e.destination), esc(e.checksum ?? "")].joined(separator: ",") + "\n"
                    }
                    try out.write(to: url, atomically: true, encoding: .utf8)
                } else {
                    guard let json = await model.perform("Export failed", { try await model.engine.exportJSON(tasks: false, history: true) }) else { return }
                    try json.write(to: url, atomically: true, encoding: .utf8)
                }
                model.toast(.success, "History exported", detail: url.lastPathComponent)
            } catch {
                model.toast(.error, "Export failed", detail: error.localizedDescription)
            }
        }
    }
}
