import OspreyKit
import SwiftUI

/// Stock sidebar (a floating Liquid Glass pane on macOS 26): places, library and queues.
struct SidebarView: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @ViewState private var deletingQueue: QueueData?

    var body: some View {
        List(selection: Binding(get: { ui.sidebar }, set: { if let v = $0 { ui.sidebar = v } })) {
            Section {
                row(.downloads, badge: model.tasks.count { $0.state.isActive })
                row(.completed, badge: 0)
                row(.torrents, badge: model.tasks.count { $0.kind.isTorrent && $0.state.isActive })
                row(.scheduled, badge: model.tasks.count { $0.state == .scheduled })
            }
            Section("Library") {
                row(.history, badge: 0)
                row(.grabber, badge: 0)
            }
            Section("Queues") {
                ForEach(model.queues) { q in queueRow(q) }
            }
        }
        .listStyle(.sidebar)
        .contextMenu {
            Button("New Queue…") { ui.editingQueue = QueueData(name: L10n.tr("New Queue"), position: Int32(model.queues.count)) }
        }
        .confirmationDialog("Delete queue?", isPresented: Binding(get: { deletingQueue != nil }, set: { if !$0 { deletingQueue = nil } }),
                            presenting: deletingQueue) { q in
            ForEach(model.queues.filter { $0.id != q.id }) { target in
                Button("Move Downloads to \(target.name)") {
                    Task { await model.perform("Couldn't delete queue") { try await model.engine.deleteQueue(q.id, moveTo: target.id) } }
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: { q in
            Text("Downloads in “\(q.name)” move to the queue you choose.")
        }
    }

    private func row(_ item: SidebarItem, badge: Int) -> some View {
        Label(LocalizedStringKey(item.title), systemImage: item.symbol)
            .badge(badge)
            .tag(item)
    }

    private func queueRow(_ q: QueueData) -> some View {
        let s = model.queueSummaries[q.id]
        let count = Int((s?.active ?? 0) + (s?.waiting ?? 0))
        return Label {
            HStack(spacing: 4) {
                Text(q.name)
                if q.paused { Image(systemName: "pause.circle").foregroundStyle(.secondary).imageScale(.small) }
            }
        } icon: {
            Image(systemName: q.icon.isEmpty ? "tray" : q.icon)
        }
        .badge(count)
        .tag(SidebarItem.queue(q.id))
        .dropDestination(for: String.self) { ids, _ in
            Task {
                for id in ids where model.tasks[id] != nil {
                    var patch = TaskPatchData()
                    patch.queueId = q.id
                    let p = patch
                    await model.perform("Couldn't move to \(q.name)") { try await model.engine.updateTask(id, patch: p) }
                }
            }
            return true
        }
        .contextMenu {
            if q.paused {
                Button("Resume Queue") { Task { await model.perform("Couldn't resume queue") { try await model.engine.resumeQueue(q.id) } } }
            } else {
                Button("Pause Queue") { Task { await model.perform("Couldn't pause queue") { try await model.engine.pauseQueue(q.id) } } }
            }
            Button("Edit Queue…") { ui.editingQueue = q }
            Button("New Queue…") { ui.editingQueue = QueueData(name: L10n.tr("New Queue"), position: Int32(model.queues.count)) }
            if !q.builtin {
                Divider()
                Button("Delete Queue…", role: .destructive) { deletingQueue = q }
            }
        }
    }
}
