import AppKit
import OspreyKit
import SwiftUI
import UniformTypeIdentifiers

struct MainWindow: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @Environment(\.appDelegate) private var delegate
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    @ViewState private var dropTargeted = false

    var body: some View {
        @Bindable var ui = ui
        ZStack {
            WindowWash()
            HStack(spacing: 0) {
                SidebarView()
                    .frame(width: 252)
                ContentPanel()
                    .padding(.vertical, 10)
                    .padding(.trailing, 10)
            }
        }
        .ignoresSafeArea()
        .sheet(item: $ui.addRequest) { prefill in
            AddDownloadSheet(prefill: prefill)
                .environment(model)
                .environment(ui)
        }
        .sheet(item: $ui.editingQueue) { q in
            QueueEditor(queue: q).environment(model)
        }
        .confirmationDialog(removeTitle, isPresented: Binding(get: { ui.removeConfirmation != nil },
                                                              set: { if !$0 { ui.removeConfirmation = nil } }),
                            titleVisibility: .visible) {
            Button("Remove from List") { remove(deleteFiles: false) }
            Button("Remove and Delete Files", role: .destructive) { remove(deleteFiles: true) }
            Button("Cancel", role: .cancel) { ui.removeConfirmation = nil }
        } message: {
            Text("Removing a download from the list keeps its file unless you choose to delete it.")
        }
        .overlay(alignment: .bottom) { ToastOverlay() }
        .overlay {
            if dropTargeted {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .stroke(Color.accentColor, lineWidth: 3)
                    .padding(4)
                    .allowsHitTesting(false)
            }
        }
        .onDrop(of: [.fileURL, .url, .plainText], isTargeted: $dropTargeted) { providers in
            handleDrop(providers)
        }
        .onAppear {
            delegate?.openMainWindowAction = { openWindow(id: "main") }
            delegate?.openSettingsAction = { openSettings() }
        }
    }

    private var removeTitle: String {
        let n = ui.removeConfirmation?.count ?? 0
        return n == 1 ? L10n.tr("Remove this download?") : String(format: L10n.tr("Remove %d downloads?"), n)
    }

    private func remove(deleteFiles: Bool) {
        guard let ids = ui.removeConfirmation else { return }
        model.remove(ids, deleteFiles: deleteFiles)
        ui.selection.subtract(ids)
        ui.removeConfirmation = nil
    }

    private func handleDrop(_ providers: [NSItemProvider]) -> Bool {
        var handled = false
        for p in providers {
            if p.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
                handled = true
                _ = p.loadObject(ofClass: URL.self) { url, _ in
                    guard let url else { return }
                    Task { @MainActor in
                        switch url.pathExtension.lowercased() {
                        case "torrent": ui.openAdd(AddPrefill(torrentFile: url))
                        case "metalink", "meta4": ui.openAdd(AddPrefill(metalinkFile: url))
                        default:
                            if let text = try? String(contentsOf: url, encoding: .utf8) { ui.openAdd(AddPrefill(text: text)) }
                        }
                    }
                }
            } else if p.canLoadObject(ofClass: URL.self) {
                handled = true
                _ = p.loadObject(ofClass: URL.self) { url, _ in
                    guard let url else { return }
                    Task { @MainActor in ui.openAdd(AddPrefill(text: url.absoluteString)) }
                }
            } else if p.canLoadObject(ofClass: String.self) {
                handled = true
                _ = p.loadObject(ofClass: String.self) { text, _ in
                    guard let text else { return }
                    Task { @MainActor in ui.openAdd(AddPrefill(text: text)) }
                }
            }
        }
        return handled
    }
}

struct DetailRouter: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui

    var body: some View {
        Group {
            if case .unavailable(let reason) = model.status {
                EmptyStateView("exclamationmark.triangle", title: "Osprey Can't Start", message: reason) {
                    Button("Try Again") { Task { await model.reloadSnapshot() } }
                        .buttonStyle(ProminentCapsuleStyle())
                }
            } else {
                switch ui.sidebar {
                case .dashboard: DashboardView()
                case .downloads: DownloadsView(scope: .active)
                case .completed: DownloadsView(scope: .completed)
                case .torrents: DownloadsView(scope: .torrents)
                case .queue(let id): DownloadsView(scope: .queue(id))
                case .scheduled: ScheduledView()
                case .grabber: GrabberView()
                case .history: HistoryView()
                }
            }
        }
        .id(ui.sidebar)
    }
}

// MARK: - Content panel

/// The large floating panel: the current page, the inspector beside it and the status bar.
struct ContentPanel: View {
    @Environment(UIState.self) private var ui
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 22, style: .continuous)
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                DetailRouter()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                if ui.showInspector {
                    InspectorView()
                        .frame(width: 340)
                        .frame(maxHeight: .infinity)
                        .background(Theme.card.opacity(scheme == .dark ? 0.7 : 0.9), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(Theme.hairline, lineWidth: 1))
                        .padding(.top, 12)
                        .padding(.trailing, 12)
                        .transition(.move(edge: .trailing).combined(with: .opacity))
                }
            }
            .animation(.spring(response: 0.4, dampingFraction: 0.88), value: ui.showInspector)
            StatusBar()
        }
        .background(Theme.panel, in: shape)
        .clipShape(shape)
        .overlay(shape.strokeBorder(Theme.hairline, lineWidth: 1))
        .shadow(color: Color(red: 0.05, green: 0.1, blue: 0.3).opacity(scheme == .dark ? 0.45 : 0.10), radius: 24, y: 10)
        .transaction { if SnapshotSample.active { $0.disablesAnimations = true } }
    }
}

// MARK: - Toasts

struct ToastOverlay: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 8) {
            ForEach(model.toasts) { t in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    if t.style == .error {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Theme.danger)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(t.title).font(.callout.weight(.medium))
                        if let d = t.detail { Text(d).font(.caption).foregroundStyle(.secondary).lineLimit(3) }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .frame(maxWidth: 440)
                .ospreyGlass(.regular, in: Capsule())
                .transition(reduceMotion ? .opacity : .move(edge: .bottom).combined(with: .opacity))
                .onTapGesture { model.toasts.removeAll { $0.id == t.id } }
                .accessibilityElement(children: .combine)
            }
        }
        .padding(.bottom, 20)
        .animation(reduceMotion ? nil : .smooth, value: model.toasts)
    }
}
