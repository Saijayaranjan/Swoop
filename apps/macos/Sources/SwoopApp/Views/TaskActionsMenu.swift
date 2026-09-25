import AppKit
import SwoopKit
import SwiftUI

/// Every task action, used by the table's context menu and the inspector's action menu.
struct TaskActionsMenu: View {
    let ids: [String]
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @Environment(\.appDelegate) private var delegate

    private var items: [TaskItem] { ids.compactMap { model.tasks[$0] } }

    var body: some View {
        let states = Set(items.map(\.state))
        let single = items.count == 1 ? items.first : nil

        if items.isEmpty {
            Button("Add Download…") { ui.openAdd() }
        } else {
            // Primary transfer control
            if states.contains(where: \.canPause) {
                action(.pause, ids: items.filter { $0.state.canPause }.map(\.id))
            }
            if states.contains(where: { $0 == .paused || $0 == .pending }) {
                action(.resume, ids: items.filter { $0.state == .paused || $0.state == .pending }.map(\.id))
            }
            if states.contains(where: { $0 == .pending || $0 == .queued || $0 == .scheduled }) {
                action(.start, ids: items.filter { $0.state.isWaiting }.map(\.id))
            }
            if states.contains(where: { $0 == .failed || $0 == .cancelled }) {
                action(.retry, ids: items.filter { $0.state == .failed || $0.state == .cancelled }.map(\.id))
                if let s = single, s.state == .failed {
                    Button("Retry from New Link…") { retryFromSource(s) }
                }
            }
            if states.contains(where: { !$0.isTerminal }) {
                action(.cancel, ids: items.filter { !$0.state.isTerminal }.map(\.id))
            }
            Divider()

            if let s = single {
                Button("Open") { Finder.open(path(s)) }.disabled(s.state != .completed && s.state != .seeding)
            }
            Button("Show in Finder") { Finder.reveal(items.map(path)) }
            Button("Quick Look") { delegate?.quickLook.toggle(paths: items.map(path)) }
            Button("Copy Link") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(items.compactMap(\.data.url).joined(separator: "\n"), forType: .string)
            }
            if let s = single {
                Button("Show Details") { ui.selection = [s.id]; ui.showInspector = true }
                Button("Rename…") { rename(s) }
            }
            Divider()

            Menu("Move to Queue") {
                ForEach(model.queues) { q in
                    Button(q.name) { patch { $0.queueId = q.id } }
                }
            }
            Menu("Category") {
                Button("None") { patch { $0.categoryId = .some(nil) } }
                ForEach(model.categories) { c in
                    Button(c.name) { patch { $0.categoryId = .some(c.id) } }
                }
            }
            Menu("Priority") {
                ForEach(Priority.allCases, id: \.self) { p in
                    Button(p.label) { forEach { try await model.engine.setTaskPriority($0, p) } }
                }
            }
            Menu("Speed Limit") {
                Button("Unlimited") { forEach { try await model.engine.setTaskLimit($0, download: 0, upload: nil) } }
                ForEach([256_000, 1_000_000, 5_000_000, 20_000_000] as [UInt64], id: \.self) { l in
                    Button(Fmt.speed(l)) { forEach { try await model.engine.setTaskLimit($0, download: l, upload: nil) } }
                }
            }
            Menu("Connections") {
                ForEach([1, 2, 4, 8, 16, 32] as [UInt8], id: \.self) { n in
                    Button("\(n)") { forEach { try await model.engine.setTaskConnections($0, n) } }
                }
            }
            Button("Move to Top of Queue") {
                Task { await model.perform("Couldn't reorder") { try await model.engine.reorderTasks(ids, after: nil) } }
            }
            Divider()

            if states.contains(.completed) {
                action(.verify, ids: items.filter { $0.state == .completed }.map(\.id))
                action(.redownload, ids: items.filter { $0.state == .completed }.map(\.id))
            }
            if items.contains(where: { !$0.kind.isTorrent && $0.state.isActive }) {
                action(.retrySegments, ids: items.filter { !$0.kind.isTorrent }.map(\.id))
            }
            action(.restart, ids: ids)
            action(.duplicate, ids: ids)
            if let s = single, s.kind.isTorrent {
                Divider()
                Button("Ask Trackers for Peers") { Task { await model.perform("Couldn't reannounce") { try await model.engine.reannounce(s.id) } } }
                Button("Download Sequentially") { Task { await model.perform("Couldn't change order") { try await model.engine.setTorrentSequential(s.id, true) } } }
            }
            if let s = single {
                Button("Copy Diagnostics") {
                    Task {
                        if let text = await model.perform("Couldn't collect diagnostics", { try await model.engine.diagnosticsText(s.id) }) {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(text, forType: .string)
                            model.toast(.success, "Diagnostics copied")
                        }
                    }
                }
            }
            Divider()
            Button("Remove…", role: .destructive) { ui.removeConfirmation = ids }
        }
    }

    private func action(_ a: TaskAction, ids: [String]) -> some View {
        Button { model.act(a, on: ids) } label: { Label(a.label, systemImage: a.symbol) }
            .disabled(ids.isEmpty)
    }

    private func path(_ t: TaskItem) -> String {
        t.data.filePath ?? (t.data.directory as NSString).appendingPathComponent(t.data.name)
    }

    private func patch(_ edit: @escaping (inout TaskPatchData) -> Void) {
        var p = TaskPatchData()
        edit(&p)
        let patch = p
        forEach { try await model.engine.updateTask($0, patch: patch) }
    }

    private func forEach(_ op: @escaping (String) async throws -> Void) {
        let ids = self.ids
        Task {
            for id in ids { await model.perform("Couldn't update") { try await op(id) } }
        }
    }

    private func rename(_ item: TaskItem) {
        let alert = NSAlert()
        alert.messageText = L10n.tr("Rename Download")
        let field = NSTextField(string: item.name)
        field.frame = NSRect(x: 0, y: 0, width: 320, height: 24)
        alert.accessoryView = field
        alert.addButton(withTitle: L10n.tr("Rename"))
        alert.addButton(withTitle: L10n.tr("Cancel"))
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let name = field.stringValue.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty, name != item.name else { return }
        var p = TaskPatchData()
        p.name = name
        let patch = p
        Task { await model.perform("Couldn't rename") { try await model.engine.updateTask(item.id, patch: patch) } }
    }

    private func retryFromSource(_ item: TaskItem) {
        let alert = NSAlert()
        alert.messageText = L10n.tr("Retry from a New Link")
        alert.informativeText = L10n.tr("Paste a fresh link to the same file. Downloaded data is kept when the file hasn't changed.")
        let field = NSTextField(string: item.data.url ?? "")
        field.frame = NSRect(x: 0, y: 0, width: 360, height: 24)
        alert.accessoryView = field
        alert.addButton(withTitle: L10n.tr("Retry"))
        alert.addButton(withTitle: L10n.tr("Cancel"))
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let url = field.stringValue.trimmingCharacters(in: .whitespaces)
        Task { await model.perform("Couldn't retry") { try await model.engine.retryFromSource(item.id, newURL: url.isEmpty ? nil : url) } }
    }
}
