import AppKit
import OspreyKit
import SwiftUI
import UniformTypeIdentifiers

struct MainWindow: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @Environment(\.appDelegate) private var delegate
    @Environment(\.openWindow) private var openWindow
    @ViewState private var dropTargeted = false
    @ViewState private var columns: NavigationSplitViewVisibility = .all

    var body: some View {
        @Bindable var ui = ui
        NavigationSplitView(columnVisibility: $columns) {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 190, ideal: 220, max: 300)
        } detail: {
            DetailRouter()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .inspector(isPresented: $ui.showInspector) {
                    InspectorView()
                        .inspectorColumnWidth(min: 300, ideal: 340, max: 480)
                }
        }
        .toolbar { MainToolbar() }
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
                ContentUnavailableView {
                    Label("Osprey Can't Start", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(reason)
                } actions: {
                    Button("Try Again") { Task { await model.reloadSnapshot() } }
                }
            } else {
                switch ui.sidebar {
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

// MARK: - Toolbar

struct MainToolbar: ToolbarContent {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui

    var body: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            ActivityStatusButton()
        }
        ToolbarItemGroup(placement: .primaryAction) {
            AddToolbarButton()
            Button {
                if model.stats.active > 0 || model.stats.queued > 0 { model.pauseAll() } else { model.resumeAll() }
            } label: {
                if model.stats.active > 0 || model.stats.queued > 0 {
                    Label("Pause All", systemImage: "pause.fill")
                } else {
                    Label("Resume All", systemImage: "play.fill")
                }
            }
            .help(model.stats.active > 0 ? "Pause all downloads" : "Resume all downloads")
            Menu {
                Picker("Speed", selection: Binding(get: { model.stats.trafficMode == .fullSpeed ? .unlimited : model.stats.trafficMode },
                                                   set: { model.setTrafficMode($0) })) {
                    ForEach(TrafficMode.pickerModes, id: \.self) { Text(LocalizedStringKey($0.label)).tag($0) }
                }
                .pickerStyle(.inline)
                Divider()
                SettingsLink { Text("Speed Settings…") }
            } label: {
                Label("Speed", systemImage: "gauge.with.dots.needle.33percent")
            }
            .help("Speed mode: \(model.stats.trafficMode.label)")
            Button { ui.showInspector.toggle() } label: { Label("Inspector", systemImage: "sidebar.trailing") }
                .help("Show or hide the inspector (⌥⌘I)")
        }
    }
}

struct AddToolbarButton: View {
    @Environment(UIState.self) private var ui
    var body: some View {
        Button { ui.openAdd() } label: { Label("Add Download", systemImage: "plus") }
            .help("Add a download (⌘N)")
            .modifier(ProminentToolbarButton())
    }
}

private struct ProminentToolbarButton: ViewModifier {
    func body(content: Content) -> some View {
        if #available(macOS 26, *) {
            content.buttonStyle(.glassProminent)
        } else {
            content
        }
    }
}

/// "↓ 4.2 MB/s · 3 active" — the window's single live speed readout; opens the Activity popover.
struct ActivityStatusButton: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui

    var body: some View {
        @Bindable var ui = ui
        Button { ui.showActivity.toggle() } label: {
            HStack(spacing: 6) {
                Image(systemName: "arrow.down")
                    .imageScale(.small)
                Text(Fmt.speed(model.stats.downloadSpeed, zero: "0 B/s"))
                    .contentTransition(.numericText(value: Double(model.stats.downloadSpeed)))
                if model.stats.active > 0 {
                    Text("·")
                    Text("\(model.stats.active) active")
                }
            }
            .font(.callout.monospacedDigit())
            .foregroundStyle(.secondary)
            .animation(.smooth, value: model.stats.downloadSpeed)
            .padding(.horizontal, 6)
        }
        .buttonStyle(.borderless)
        .help("Show activity")
        .accessibilityLabel(Text("Download speed \(Fmt.speed(model.stats.downloadSpeed, zero: "0 B/s")), \(model.stats.active) active"))
        .popover(isPresented: $ui.showActivity, arrowEdge: .bottom) {
            ActivityPopover().environment(model)
        }
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
