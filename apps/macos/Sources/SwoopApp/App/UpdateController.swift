import AppKit
import SwiftUI
import SwoopKit

/// Drives in-app updates from GitHub Releases.
///
/// The engine does the security-sensitive work: it checks the release feed, downloads the DMG and
/// verifies its Ed25519 signature, then (on "Install and Relaunch") re-verifies, mounts and
/// validates the app inside and stages it next to this bundle. This controller owns the schedule
/// and the UI, and after Swoop has quit cleanly it launches the bundled `swoop update apply`
/// helper, which swaps the bundles and relaunches.
@MainActor
@Observable
final class UpdateController {
    /// Set once by the app delegate; views read it directly (Observation tracks the reads).
    private(set) static var shared: UpdateController?

    enum Phase: Equatable {
        case idle
        case checking
        case upToDate
        case couldNotCheck(String)
        case available
        case downloading(received: UInt64, total: UInt64?)
        case verifying
        /// Downloaded and verified; waiting for the user.
        case ready
        case installing
        /// Staged in the background; installs when Swoop quits (automatic mode).
        case staged
        case failed(String)
    }

    private(set) var phase: Phase = .idle
    private(set) var info: UpdateInfoData?
    private(set) var lastChecked: Date?
    /// True while quitting to install, so the "downloads in progress" prompt is skipped: they
    /// pause and persist the normal way and resume after the relaunch.
    private(set) var isRelaunchingForUpdate = false

    @ObservationIgnored private let model: AppModel
    @ObservationIgnored private var pending: (staged: String, relaunch: Bool)?
    @ObservationIgnored private var timer: Timer?
    @ObservationIgnored private var window: NSWindow?
    @ObservationIgnored private var checking = false
    @ObservationIgnored private var downloading = false

    init(model: AppModel) {
        self.model = model
        Self.shared = self
    }

    // MARK: state for views

    var currentVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? model.info.version
    }

    /// The chip in the status bar shows while an update waits to be installed.
    var updateReady: Bool { phase == .ready || phase == .staged }

    var lastCheckDate: Date? {
        if let lastChecked { return lastChecked }
        let ms = model.settings.double("updates.last_check_at")
        return ms > 0 ? Date(millis: Int64(ms)) : nil
    }

    var releasesPageURL: URL? {
        let s = info?.notesURL ?? model.engine.releasesPageURL()
        return s.isEmpty ? nil : URL(string: s)
    }

    private var busyInstalling: Bool {
        switch phase {
        case .downloading, .verifying, .ready, .installing, .staged: return true
        default: return false
        }
    }

    // MARK: schedule

    /// First check ~10 s after launch, then whenever 24 h have passed since the last one.
    func start() {
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(10))
            await self?.automaticCheck()
        }
        timer = Timer.scheduledTimer(withTimeInterval: 60 * 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let last = self.lastCheckDate ?? .distantPast
                if Date().timeIntervalSince(last) >= 24 * 60 * 60 {
                    Task { await self.automaticCheck() }
                }
            }
        }
    }

    /// Quiet: nothing is shown unless an update is found (and then only once it's verified).
    private func automaticCheck() async {
        guard model.settings.bool("updates.check_automatically", default: true), !busyInstalling else { return }
        await runCheck(manual: false)
        guard info?.available == true else {
            if case .couldNotCheck = phase { phase = .idle }
            return
        }
        await download()
        guard phase == .ready else { return }
        #if DEBUG
        // End-to-end testing only (compiled out of release builds): take the "Install and
        // Relaunch" path as soon as the update is verified.
        if ProcessInfo.processInfo.environment["SWOOP_UPDATE_TEST_INSTALL"] == "1" {
            installAndRelaunch()
            return
        }
        #endif
        if model.settings.bool("updates.install_automatically") {
            await stage(relaunch: false)
        } else {
            showWindow()
        }
    }

    /// "Check for Updates…" (menu) and "Check Now" (Settings). A found update starts downloading
    /// right away so "Install and Relaunch" is quick.
    func checkNow(showingWindow: Bool) {
        if showingWindow { showWindow() }
        Task {
            if !busyInstalling { await runCheck(manual: true) }
            guard info?.available == true else { return }
            if !showingWindow { showWindow() }
            if phase == .available || isFailed { await download() }
        }
    }

    private var isFailed: Bool {
        if case .failed = phase { return true }
        return false
    }

    private func runCheck(manual: Bool) async {
        guard !checking else { return }
        checking = true
        defer { checking = false }
        let readyVersion = updateReady ? info?.latestVersion : nil
        phase = .checking
        do {
            let result = try await model.engine.checkAppUpdate(manual: manual)
            info = result
            lastChecked = Date()
            if result.available {
                phase = readyVersion != nil && readyVersion == result.latestVersion ? .ready : .available
            } else if result.isQuietlyCurrent || result.skipped {
                phase = .upToDate
            } else {
                phase = .couldNotCheck(Self.sentence(result.message ?? L10n.tr("GitHub couldn't be reached.")))
            }
        } catch {
            phase = .couldNotCheck(Self.sentence(error.localizedDescription))
        }
    }

    // MARK: download → verify

    func download() async {
        guard !downloading, info?.available == true else { return }
        downloading = true
        defer { downloading = false }
        phase = .downloading(received: 0, total: info?.size)
        let engine = model.engine
        let poll = Task { [weak self] in
            while !Task.isCancelled {
                let p = engine.updateProgress()
                await MainActor.run { self?.apply(p) }
                try? await Task.sleep(for: .milliseconds(150))
            }
        }
        defer { poll.cancel() }
        do {
            _ = try await engine.downloadUpdate()
            phase = .ready
        } catch {
            phase = .failed(Self.sentence(error.localizedDescription))
        }
    }

    private func apply(_ p: UpdateProgressData) {
        switch p.phase {
        case "downloading": if case .downloading = phase { phase = .downloading(received: p.received, total: p.total) }
        case "verifying": if case .downloading = phase { phase = .verifying }
        default: break
        }
    }

    // MARK: install

    func installAndRelaunch() {
        switch phase {
        case .staged:
            pending?.relaunch = true
            quitToInstall()
        case .ready:
            Task { await stage(relaunch: true) }
        case .failed:
            Task {
                await download()
                if phase == .ready { await stage(relaunch: true) }
            }
        default:
            break
        }
    }

    private func stage(relaunch: Bool) async {
        phase = .installing
        do {
            let staged = try await model.engine.stageUpdate(bundlePath: Bundle.main.bundlePath)
            pending = (staged, relaunch)
            if relaunch { quitToInstall() } else { phase = .staged }
        } catch {
            phase = .failed(Self.sentence(error.localizedDescription))
            if !relaunch { showWindow() }
        }
    }

    private func quitToInstall() {
        isRelaunchingForUpdate = true
        closeWindow()
        // Not `NSApp.terminate(nil)` directly: this often runs inside a main-actor job, and
        // `applicationShouldTerminate` replies from another main-actor task, which can't run while
        // this job is blocked in AppKit's terminate loop. A run-loop perform starts it cleanly.
        NSApp.perform(#selector(NSApplication.terminate(_:)), with: nil, afterDelay: 0)
    }

    /// Called by the app delegate once the engine has stopped (downloads paused and persisted).
    /// Hands the staged bundle to the detached helper, which waits for this process to exit.
    func launchPendingInstall() {
        guard let pending else { return }
        let helper = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/swoop")
        let process = Process()
        process.executableURL = helper
        var args = ["update", "apply",
                    "--pid", String(ProcessInfo.processInfo.processIdentifier),
                    "--staged", pending.staged,
                    "--target", Bundle.main.bundlePath]
        if pending.relaunch { args.append("--relaunch") }
        process.arguments = args
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
        } catch {
            NSLog("Swoop: couldn't start the update helper: \(error.localizedDescription)")
        }
    }

    /// Engine messages start lower-case ("the update's signature…"); show them as sentences.
    private static func sentence(_ s: String) -> String {
        s.prefix(1).uppercased() + s.dropFirst()
    }

    func later() {
        closeWindow()
    }

    func skipThisVersion() {
        if let v = info?.latestVersion {
            model.setSetting("updates.skipped_version", .string(v))
        }
        info?.available = false
        pending = nil
        phase = .idle
        closeWindow()
    }

    // MARK: window

    func showWindow() {
        if window == nil {
            let host = NSHostingController(rootView: UpdateWindowView(updates: self).environment(model))
            host.sizingOptions = [.preferredContentSize]
            let w = NSWindow(contentViewController: host)
            w.styleMask = [.titled, .closable, .fullSizeContentView]
            w.titlebarAppearsTransparent = true
            w.titleVisibility = .hidden
            w.title = L10n.tr("Software Update")
            w.isMovableByWindowBackground = true
            w.isReleasedWhenClosed = false
            w.center()
            window = w
        }
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }

    func closeWindow() {
        window?.close()
    }

    /// Snapshot tooling only (`SWOOP_SNAPSHOT_SCREENS=update-ready,…`): shows the window in a given
    /// state with sample release data. Nothing is checked, downloaded or installed.
    func showSnapshotState(_ state: String) -> NSWindow? {
        let notes = """
        ## Highlights
        - **Updates install themselves**: verified with Swoop's release key, then swapped in on relaunch.
        - Smoother segment scheduling on unreliable connections.

        ## Fixes
        1. Emptying a paused queue no longer crashes.
        2. The menu bar speed stays steady.
        """
        var sample = UpdateInfoData(currentVersion: currentVersion, available: true, latestVersion: "0.2.0",
                                    notes: notes, signatureValid: nil, checkedAt: 0)
        sample.status = "available"
        sample.publishedAt = Date()
        sample.size = 46_012_714
        switch state {
        case "checking": info = nil; phase = .checking
        case "uptodate": info = nil; phase = .upToDate
        case "offline": info = nil; phase = .couldNotCheck(L10n.tr("Couldn't reach GitHub: the Internet connection appears to be offline."))
        case "downloading": info = sample; phase = .downloading(received: 18_874_368, total: 46_012_714)
        case "failed": info = sample; phase = .failed(Self.sentence(L10n.tr("the update's signature doesn't match Swoop's release key")))
        default: info = sample; phase = .ready
        }
        showWindow()
        return window
    }
}
