import SwoopKit
import SwiftUI

/// The sidebar: brand mark, places, queues, and notifications/settings pinned at the bottom. It
/// sits on the window wash; the selected row carries a Liquid Glass lozenge that glides between rows.
struct SidebarView: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @ViewState private var deletingQueue: QueueData?
    @Namespace private var selectionNS

    private static let places: [SidebarItem] = [.dashboard, .downloads, .torrents, .scheduled, .history, .grabber]

    var body: some View {
        @Bindable var ui = ui
        VStack(alignment: .leading, spacing: 0) {
            // Clear the traffic lights, then the brand.
            HStack(spacing: 10) {
                SwoopMark(size: 30)
                Text("Swoop")
                    .font(.system(size: 20, weight: .bold, design: .rounded))
            }
            .padding(.leading, 22)
            .padding(.top, 50)
            .padding(.bottom, 18)

            ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(Self.places, id: \.self) { item in
                            SidebarRow(title: item.title, symbol: item.symbol, badge: badge(item),
                                       selected: isSelected(item), namespace: selectionNS) { select(item) }
                        }

                        HStack {
                            Text(L10n.tr("Queues").uppercased())
                                .font(Theme.cardLabel)
                                .tracking(Theme.cardLabelTracking)
                                .foregroundStyle(.secondary)
                            Spacer()
                            Button {
                                ui.editingQueue = QueueData(name: L10n.tr("New Queue"), position: Int32(model.queues.count))
                            } label: {
                                Image(systemName: "plus").font(.system(size: 12, weight: .bold)).foregroundStyle(.secondary)
                                    .frame(width: 22, height: 22).contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .help("New Queue")
                            .accessibilityLabel(Text("New Queue"))
                        }
                        .padding(.horizontal, 14)
                        .padding(.top, 20)
                        .padding(.bottom, 6)

                        ForEach(model.queues) { q in queueRow(q) }
                    }
                    .padding(.horizontal, 12)
            }
            .scrollIndicators(.never)

            VStack(alignment: .leading, spacing: 2) {
                Rectangle().fill(Theme.hairline).frame(height: 1).padding(.horizontal, 14).padding(.bottom, 8)
                SidebarRow(title: "Notifications", symbol: model.recentCompletions.isEmpty ? "bell" : "bell.badge",
                           badge: 0, selected: false, namespace: selectionNS) { ui.showNotifications.toggle() }
                    .popover(isPresented: $ui.showNotifications, arrowEdge: .trailing) {
                        NotificationsPopover().environment(model).environment(ui)
                    }
                SettingsLink {
                    SidebarRowLabel(title: "Settings", symbol: "gearshape", badge: 0, selected: false)
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 14)
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

    private func isSelected(_ item: SidebarItem) -> Bool {
        ui.sidebar == item || (item == .downloads && ui.sidebar == .completed)
    }

    private func select(_ item: SidebarItem) {
        withAnimation(.spring(response: 0.36, dampingFraction: 0.84)) { ui.sidebar = item }
    }

    private func badge(_ item: SidebarItem) -> Int {
        switch item {
        case .downloads: return model.tasks.count { $0.state.isActive }
        case .torrents: return model.tasks.count { $0.kind.isTorrent && $0.state.isActive }
        case .scheduled: return model.tasks.count { $0.state == .scheduled }
        default: return 0
        }
    }

    private func queueRow(_ q: QueueData) -> some View {
        let s = model.queueSummaries[q.id]
        let count = Int((s?.active ?? 0) + (s?.waiting ?? 0))
        let item = SidebarItem.queue(q.id)
        return SidebarRow(title: q.name, symbol: q.icon.isEmpty ? "tray" : q.icon, badge: count,
                          selected: ui.sidebar == item, paused: q.paused, namespace: selectionNS) { select(item) }
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

/// One sidebar row: 17 pt label, symbol, optional count; the selected row wears the glass lozenge.
struct SidebarRow: View {
    var title: String
    var symbol: String
    var badge: Int
    var selected: Bool
    var paused: Bool = false
    var namespace: Namespace.ID
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            SidebarRowLabel(title: title, symbol: symbol, badge: badge, selected: selected, paused: paused)
                .background {
                    if selected {
                        Color.clear
                            .swoopGlass(.regular, in: RoundedRectangle(cornerRadius: 13, style: .continuous))
                            .matchedGeometryEffect(id: "sidebar-selection", in: namespace)
                    }
                }
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

struct SidebarRowLabel: View {
    var title: String
    var symbol: String
    var badge: Int
    var selected: Bool
    var paused: Bool = false
    @ViewState private var hovering = false

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 16, weight: .medium))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(selected ? AnyShapeStyle(Theme.blue) : AnyShapeStyle(.primary.opacity(0.75)))
                .frame(width: 24)
            Text(LocalizedStringKey(title))
                .font(Theme.sidebarRow.weight(selected ? .semibold : .regular))
                .lineLimit(1)
            if paused {
                Image(systemName: "pause.circle.fill").font(.system(size: 12)).foregroundStyle(Theme.warning)
            }
            Spacer(minLength: 4)
            if badge > 0 {
                Text("\(badge)")
                    .font(.system(size: 12, weight: .semibold, design: .rounded).monospacedDigit())
                    .foregroundStyle(selected ? AnyShapeStyle(.white) : AnyShapeStyle(.secondary))
                    .padding(.horizontal, 7)
                    .frame(minWidth: 22, minHeight: 20)
                    .background(selected ? AnyShapeStyle(Theme.blue) : AnyShapeStyle(Color.primary.opacity(0.08)), in: Capsule())
                    .contentTransition(.numericText())
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 44)
        .background {
            if hovering && !selected {
                RoundedRectangle(cornerRadius: 13, style: .continuous).fill(Color.primary.opacity(0.05))
            }
        }
        .contentShape(Rectangle())
        .onHover { h in withAnimation(.easeOut(duration: 0.12)) { hovering = h } }
    }
}

/// Recent finished and failed downloads, opened from the sidebar bell.
struct NotificationsPopover: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui

    var body: some View {
        let failed = model.tasks.items.filter { $0.state == .failed }.prefix(4)
        VStack(alignment: .leading, spacing: 12) {
            Text("Notifications").font(.system(size: 17, weight: .bold))
            if model.recentCompletions.isEmpty && failed.isEmpty {
                HStack(spacing: 10) {
                    Image(systemName: "bell.slash").foregroundStyle(.secondary)
                    Text("Nothing new. Finished and failed downloads show up here.")
                        .font(.callout).foregroundStyle(.secondary)
                }
                .padding(.vertical, 8)
            }
            ForEach(Array(failed)) { item in
                row(symbol: "exclamationmark.triangle.fill", tint: Theme.danger, title: item.name,
                    detail: TaskStatusText.detail(item.data)) { ui.select(item.id); ui.showNotifications = false }
            }
            ForEach(model.recentCompletions.prefix(6)) { r in
                row(symbol: "checkmark.circle.fill", tint: Theme.success, title: r.name,
                    detail: "\(Fmt.bytes(r.progress.total ?? r.progress.downloaded)) · \(ShortDate.string(r.completedAt))") {
                    Finder.reveal([r.targetPath])
                }
            }
            Divider()
            SettingsLink { Text("Notification Settings…") }
                .buttonStyle(.borderless)
        }
        .padding(16)
        .frame(width: 340)
    }

    private func row(symbol: String, tint: Color, title: String, detail: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: symbol).foregroundStyle(tint).font(.system(size: 15))
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(.system(size: 13, weight: .medium)).lineLimit(1).truncationMode(.middle)
                    Text(detail).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
