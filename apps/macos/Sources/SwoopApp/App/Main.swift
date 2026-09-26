import AppKit
import SwoopKit
import SwiftUI

@main
struct SwoopApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        Window("Swoop", id: "main") {
            MainWindow()
                .environment(delegate.model)
                .environment(delegate.ui)
                .environment(\.appDelegate, delegate)
                .frame(minWidth: 960, minHeight: 600)
        }
        .defaultSize(width: 1280, height: 800)
        .windowStyle(.hiddenTitleBar)
        .commands { SwoopCommands(model: delegate.model, ui: delegate.ui, delegate: delegate) }

        Window("About Swoop", id: "about") {
            AboutView()
                .environment(delegate.model)
        }
        .windowStyle(.hiddenTitleBar)
        .windowResizability(.contentSize)
        .defaultPosition(.center)

        Settings {
            SettingsView()
                .environment(delegate.model)
                .environment(delegate.ui)
        }
        .windowStyle(.hiddenTitleBar)
        .windowResizability(.contentSize)

        MenuBarExtra(isInserted: Binding(
            get: { delegate.model.settings.bool("appearance.show_menu_bar_extra", default: true) },
            set: { delegate.model.setSetting("appearance.show_menu_bar_extra", .bool($0)) }
        )) {
            MenuBarView()
                .environment(delegate.model)
                .environment(delegate.ui)
                .environment(\.appDelegate, delegate)
        } label: {
            MenuBarLabel(model: delegate.model)
        }
        .menuBarExtraStyle(.window)
    }
}

private struct AppDelegateKey: EnvironmentKey {
    static let defaultValue: AppDelegate? = nil
}

extension EnvironmentValues {
    var appDelegate: AppDelegate? {
        get { self[AppDelegateKey.self] }
        set { self[AppDelegateKey.self] = newValue }
    }
}

struct MenuBarLabel: View {
    let model: AppModel
    var body: some View {
        let active = model.stats.downloading
        HStack(spacing: 3) {
            Image(systemName: active > 0 ? "arrow.down.circle.fill" : "arrow.down.circle")
            if active > 0 {
                Text(Fmt.speed(model.stats.downloadSpeed, zero: ""))
                    .monospacedDigit()
            }
        }
        .accessibilityLabel(Text("Swoop"))
    }
}

struct SwoopCommands: Commands {
    let model: AppModel
    let ui: UIState
    let delegate: AppDelegate

    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .appInfo) {
            Button("About Swoop") { openWindow(id: "about") }
            Button("Check for Updates…") { delegate.updates.checkNow(showingWindow: true) }
        }
        CommandGroup(replacing: .newItem) {
            Button("New Download…") { delegate.showMainWindow(); ui.openAdd() }
                .keyboardShortcut("n", modifiers: .command)
            Button("Add from Clipboard") { delegate.showMainWindow(); ui.openAdd(AddPrefill(autoPaste: true)) }
                .keyboardShortcut("v", modifiers: [.command, .shift])
            Button("Open Torrent or Metalink…") { openFile() }
                .keyboardShortcut("o", modifiers: .command)
        }
        CommandGroup(after: .importExport) {
            Button("Export Downloads & Settings…") { exportAll() }
            Button("Import…") { importAll() }
        }
        CommandMenu("Downloads") {
            Button("Pause / Resume") { model.togglePause(Array(ui.selection)) }
                .keyboardShortcut(".", modifiers: .command)
                .disabled(ui.selection.isEmpty)
            Button("Show in Finder") { revealSelection() }
                .keyboardShortcut("r", modifiers: .command)
                .disabled(ui.selection.isEmpty)
            Button("Quick Look") { quickLookSelection() }
                .keyboardShortcut("y", modifiers: .command)
                .disabled(ui.selection.isEmpty)
            Button("Remove") { ui.removeConfirmation = Array(ui.selection) }
                .keyboardShortcut(.delete, modifiers: .command)
                .disabled(ui.selection.isEmpty)
            Divider()
            Button("Pause All") { model.pauseAll() }
                .keyboardShortcut("p", modifiers: [.command, .option])
            Button("Resume All") { model.resumeAll() }
                .keyboardShortcut("r", modifiers: [.command, .option])
            Button("Retry Failed") { model.retryFailed() }
            Button("Clear Completed") { model.clearCompleted() }
            Divider()
            Menu("Speed Mode") {
                ForEach(TrafficMode.pickerModes, id: \.self) { mode in
                    Button { model.setTrafficMode(mode) } label: {
                        if model.stats.trafficMode == mode { Label(mode.label, systemImage: "checkmark") } else { Text(mode.label) }
                    }
                }
            }
        }
        CommandGroup(after: .sidebar) {
            Button(ui.showInspector ? "Hide Inspector" : "Show Inspector") { ui.showInspector.toggle() }
                .keyboardShortcut("i", modifiers: [.command, .option])
            Divider()
            ForEach(Array(SidebarItem.shortcutOrder.enumerated()), id: \.offset) { i, item in
                Button(LocalizedStringKey(item.title)) { ui.sidebar = item }
                    .keyboardShortcut(KeyEquivalent(Character("\(i + 1)")), modifiers: .command)
            }
        }
    }

    private func selectedPaths() -> [String] {
        ui.selection.compactMap { model.tasks[$0]?.data }.map { $0.filePath ?? ($0.directory as NSString).appendingPathComponent($0.name) }
    }

    private func revealSelection() { Finder.reveal(selectedPaths()) }
    private func quickLookSelection() { delegate.quickLook.toggle(paths: selectedPaths()) }

    private func openFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.init(filenameExtension: "torrent")!, .init(filenameExtension: "metalink")!, .init(filenameExtension: "meta4")!]
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        delegate.showMainWindow()
        if url.pathExtension.lowercased() == "torrent" {
            ui.openAdd(AddPrefill(torrentFile: url))
        } else {
            ui.openAdd(AddPrefill(metalinkFile: url))
        }
    }

    private func exportAll() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "Swoop Export.json"
        panel.allowedContentTypes = [.json]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task {
            if let json = await model.perform("Export failed", { try await model.engine.exportJSON(tasks: true, history: true) }) {
                do {
                    try json.write(to: url, atomically: true, encoding: .utf8)
                    model.toast(.success, "Exported", detail: url.lastPathComponent)
                } catch {
                    model.toast(.error, "Export failed", detail: error.localizedDescription)
                }
            }
        }
    }

    private func importAll() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        guard panel.runModal() == .OK, let url = panel.url, let json = try? String(contentsOf: url, encoding: .utf8) else { return }
        Task {
            if let report = await model.perform("Import failed", { try await model.engine.importJSON(json, options: ImportOptionsData()) }) {
                let n = report.imported.values.reduce(0, +)
                model.toast(report.errors.isEmpty ? .success : .error, "Imported \(n) items",
                            detail: report.errors.isEmpty ? nil : report.errors.prefix(3).joined(separator: "\n"))
                await model.reloadSnapshot()
            }
        }
    }
}
