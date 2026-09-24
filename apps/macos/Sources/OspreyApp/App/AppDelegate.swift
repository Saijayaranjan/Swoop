import AppKit
import OspreyKit
import Quartz
import SwiftUI

/// Owns the engine, the app model and every native integration (Dock, notifications, power,
/// environment, URL scheme, Services, Quick Look).
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let model: AppModel
    let ui = UIState()
    let notifications = NotificationService()
    private let sleepPreventer = SleepPreventer()
    private var environment: EnvironmentMonitor?
    private var dock: DockTileController?
    private var pendingURLs: [URL] = []
    private var launched = false
    let quickLook = QuickLookController()

    override init() {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.1.0"
        let bundleId = Bundle.main.bundleIdentifier ?? "app.osprey.desktop"
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Osprey", isDirectory: true)
        let engine = EngineFactory.open(dataDirectory: support, appVersion: version, bundleId: bundleId)
        model = AppModel(engine: engine)
        super.init()
    }

    // MARK: lifecycle

    func applicationWillFinishLaunching(_ notification: Notification) {
        NSAppleEventManager.shared().setEventHandler(self, andSelector: #selector(handleGetURL(_:reply:)),
                                                     forEventClass: AEEventClass(kInternetEventClass),
                                                     andEventID: AEEventID(kAEGetURL))
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.servicesProvider = self
        NSUpdateDynamicServices()
        notifications.configure()
        notifications.onAction = { [weak self] action, taskId, path in self?.handleNotificationAction(action, taskId: taskId, path: path) }
        dock = DockTileController(model: model)

        model.observe { [weak self] event in self?.handlePlatformEvent(event) }

        Task {
            await model.start()
            AppearanceApplier.apply(model.settings.string("appearance.theme", default: "system"))
            launched = true
            let engine = model.engine
            environment = EnvironmentMonitor { env in engine.updateEnvironment(env) }
            environment?.start()
            startPeriodicWork()
            let urls = pendingURLs
            pendingURLs.removeAll()
            if !urls.isEmpty { self.application(NSApp, open: urls) }
            SnapshotSupport.runIfRequested(ui: ui, model: model, delegate: self)
        }

        NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.ui.windowActive = true
                self?.model.engine.setActiveWindow(true)
            }
        }
        NotificationCenter.default.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.ui.windowActive = false
                self?.model.engine.setActiveWindow(false)
            }
        }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        let confirm = model.settings.bool("appearance.confirm_on_quit_with_active", default: true)
        let active = model.stats.downloading
        if confirm, active > 0 {
            let alert = NSAlert()
            alert.messageText = L10n.tr("Quit Osprey?")
            alert.informativeText = String(format: L10n.tr("%d downloads are in progress. They'll resume next time you open Osprey."), active)
            alert.addButton(withTitle: L10n.tr("Quit"))
            alert.addButton(withTitle: L10n.tr("Cancel"))
            if alert.runModal() != .alertFirstButtonReturn { return .terminateCancel }
        }
        Task {
            environment?.stop()
            sleepPreventer.update(active: false, enabled: false)
            await model.stop()
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { showMainWindow() }
        return true
    }

    func applicationDockMenu(_ sender: NSApplication) -> NSMenu? {
        let menu = NSMenu()
        let items: [(String, Selector)] = [
            ("Add Download…", #selector(dockAdd)),
            ("Pause All", #selector(dockPauseAll)),
            ("Resume All", #selector(dockResumeAll)),
        ]
        for (title, sel) in items {
            let item = NSMenuItem(title: L10n.tr(title), action: sel, keyEquivalent: "")
            item.target = self
            menu.addItem(item)
        }
        return menu
    }

    @objc private func dockAdd() { showMainWindow(); ui.openAdd() }
    @objc private func dockPauseAll() { model.pauseAll() }
    @objc private func dockResumeAll() { model.resumeAll() }

    func showMainWindow() {
        NSApp.activate(ignoringOtherApps: true)
        if let w = NSApp.windows.first(where: { $0.identifier?.rawValue.contains("main") == true || $0.title == "Osprey" }) {
            w.makeKeyAndOrderFront(nil)
        } else {
            openMainWindowAction?()
        }
    }

    /// Set by the main scene so AppKit entry points can reopen the SwiftUI window.
    var openMainWindowAction: (() -> Void)?
    var openSettingsAction: (() -> Void)?

    // MARK: periodic work (power assertion, dashboard refresh)

    private func startPeriodicWork() {
        Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let enabled = self.model.settings.bool("appearance.prevent_sleep_while_active", default: true)
                self.sleepPreventer.update(active: self.model.stats.active > 0, enabled: enabled)
                self.dock?.refresh()
            }
        }
        Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                Task { await self.model.refreshDashboard() }
            }
        }
    }

    // MARK: engine → platform

    private func handlePlatformEvent(_ event: EngineEvent) {
        switch event {
        case .notification(let n):
            let quiet = model.settings.bool("notifications.quiet_when_active") && NSApp.isActive
            if !quiet { notifications.post(n, sound: model.settings.bool("notifications.sound", default: true)) }
            if n.kind == "completed", model.settings.bool("appearance.show_dock_badge", default: true) {
                NSApp.requestUserAttention(.informationalRequest)
            }
        case .platformAction(_, let actionJSON, let contextJSON):
            if let error = PlatformActionRunner.run(actionJSON: actionJSON, contextJSON: contextJSON) {
                model.toast(.error, "Automation action failed", detail: error)
            }
        case .readyForSleep(let reason):
            SystemPower.handleReadyForSleep(reason)
        case .custom(let name, let payload):
            if name == "schedule.launch_application",
               let path = (try? JSONValue(parsing: payload))?["path"]?.string {
                NSWorkspace.shared.openApplication(at: URL(fileURLWithPath: path), configuration: .init()) { _, _ in }
            }
        case .taskStateChanged, .taskAdded, .taskRemoved:
            dock?.refresh()
        default:
            break
        }
    }

    private func handleNotificationAction(_ action: NotificationService.Action, taskId: String?, path: String?) {
        switch action {
        case .open:
            if let path { Finder.open(path) }
        case .reveal:
            if let path { Finder.reveal([path]) }
        case .retry:
            if let taskId { model.act(.retry, on: [taskId]) }
        case .show:
            showMainWindow()
            if let taskId { ui.select(taskId) }
        }
    }

    // MARK: URL scheme, documents, Services

    @objc private func handleGetURL(_ event: NSAppleEventDescriptor, reply: NSAppleEventDescriptor) {
        guard let s = event.paramDescriptor(forKeyword: keyDirectObject)?.stringValue, let url = URL(string: s) else { return }
        application(NSApp, open: [url])
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        guard launched else {
            pendingURLs.append(contentsOf: urls)
            return
        }
        for url in urls { route(url) }
    }

    private func route(_ url: URL) {
        showMainWindow()
        switch url.scheme?.lowercased() {
        case "magnet":
            ui.openAdd(AddPrefill(text: url.absoluteString))
        case "osprey":
            let comps = URLComponents(url: url, resolvingAgainstBaseURL: false)
            let target = comps?.queryItems?.first { $0.name == "url" }?.value ?? ""
            let referer = comps?.queryItems?.first { $0.name == "referer" }?.value
            if url.host == "add" || url.path.contains("add") {
                ui.openAdd(AddPrefill(text: target, refererPage: referer))
            } else if url.host == "show", let id = comps?.queryItems?.first(where: { $0.name == "id" })?.value {
                ui.select(id)
            }
        case "file":
            let ext = url.pathExtension.lowercased()
            if ext == "torrent" {
                ui.openAdd(AddPrefill(torrentFile: url))
            } else if ext == "metalink" || ext == "meta4" {
                ui.openAdd(AddPrefill(metalinkFile: url))
            }
        case "http", "https", "ftp", "ftps":
            ui.openAdd(AddPrefill(text: url.absoluteString))
        default:
            break
        }
    }

    /// Services menu → "Download with Osprey".
    @objc func downloadURL(_ pboard: NSPasteboard, userData: String?, error: AutoreleasingUnsafeMutablePointer<NSString?>) {
        let text: String
        if let urls = pboard.readObjects(forClasses: [NSURL.self]) as? [URL], !urls.isEmpty {
            text = urls.map(\.absoluteString).joined(separator: "\n")
        } else if let s = pboard.string(forType: .string), !s.isEmpty {
            text = s
        } else {
            error.pointee = L10n.tr("No link was selected.") as NSString
            return
        }
        showMainWindow()
        ui.openAdd(AddPrefill(text: text))
    }

    // MARK: Quick Look (responder-chain entry points)

    override func acceptsPreviewPanelControl(_ panel: QLPreviewPanel!) -> Bool { true }
    override func beginPreviewPanelControl(_ panel: QLPreviewPanel!) {
        panel.dataSource = quickLook
        panel.delegate = quickLook
    }
    override func endPreviewPanelControl(_ panel: QLPreviewPanel!) {
        panel.dataSource = nil
        panel.delegate = nil
    }
}
