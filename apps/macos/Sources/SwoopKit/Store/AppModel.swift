import Foundation
import Observation

/// A short-lived message shown as a glass HUD toast.
public struct Toast: Identifiable, Equatable, Sendable {
    public enum Style: Sendable { case info, success, error }
    public let id = UUID()
    public var style: Style
    public var title: String
    public var detail: String?
    public init(_ style: Style, _ title: String, detail: String? = nil) {
        self.style = style; self.title = title; self.detail = detail
    }
}

public enum EngineStatus: Equatable, Sendable {
    case starting
    case ready
    case unavailable(String)
}

/// The app's single source of truth for engine state. Lives on the main actor; the engine's
/// listener hops here asynchronously and never blocks the Rust forwarder.
@Observable
@MainActor
public final class AppModel {
    public let engine: EngineClient
    public let tasks = TaskStore()

    public private(set) var status: EngineStatus = .starting
    public private(set) var info = EngineInfoData()
    public private(set) var queues: [QueueData] = []
    public private(set) var queueSummaries: [String: QueueSummary] = [:]
    public private(set) var categories: [CategoryData] = []
    public private(set) var stats = GlobalStatsData()
    /// Rolling throughput samples (1 per stats event, ~1 Hz), newest last. Seeded from the dashboard.
    public private(set) var speedHistory: [SpeedSampleData] = []
    public private(set) var recentCompletions: [TaskRowData] = []
    public private(set) var settings = SettingsDoc(json: .object([:]))
    public private(set) var disks: [DiskInfoData] = []
    public private(set) var scheduledNext: [(scheduleId: String, at: Int64)] = []

    public private(set) var rules: [RuleDoc] = []
    public private(set) var schedules: [ScheduleDoc] = []
    public private(set) var automations: [AutomationDoc] = []
    public private(set) var recipes: [RecipeDoc] = []
    public private(set) var devices: [DeviceData] = []
    public private(set) var credentials: [CredentialData] = []
    public private(set) var pairing: PairingData?
    public private(set) var lastPairedDevice: String?
    public private(set) var grabberProgress: [String: (pages: UInt32, files: UInt32, done: Bool)] = [:]
    public private(set) var updateInfo: (available: Bool, version: String?, notes: String?)?
    public private(set) var networkAvailable = true
    /// Live log lines for tasks the inspector is watching (bounded per task).
    public private(set) var liveLogs: [String: [LogEntryData]] = [:]
    /// Revision counters views use to know when to refetch lazily loaded data.
    public private(set) var historyVersion = 0
    public private(set) var detailVersion: [String: Int] = [:]

    public var toasts: [Toast] = []

    /// Side-effect observers (notifications, Dock, platform actions). Called on the main actor.
    @ObservationIgnored private var observers: [(EngineEvent) -> Void] = []
    @ObservationIgnored private var listenerId: UInt64 = 0
    @ObservationIgnored private var loadedDocs: Set<String> = []
    @ObservationIgnored private var resyncInFlight = false

    public static let speedHistoryCapacity = 300

    /// When set, live stats from the engine are ignored and only `injectStats` updates them.
    /// Used by snapshot runs so sample figures are not interleaved with real readings.
    @ObservationIgnored public var liveStatsMuted = false

    /// Records a stats sample directly, bypassing `liveStatsMuted`.
    public func injectStats(_ s: GlobalStatsData) { record(s) }

    private func record(_ s: GlobalStatsData) {
        stats = s
        networkAvailable = s.networkAvailable
        speedHistory.append(SpeedSampleData(at: s.at > 0 ? s.at : Date().millis, download: s.downloadSpeed, upload: s.uploadSpeed))
        if speedHistory.count > Self.speedHistoryCapacity { speedHistory.removeFirst(speedHistory.count - Self.speedHistoryCapacity) }
    }

    public init(engine: EngineClient) {
        self.engine = engine
    }

    // MARK: lifecycle

    public func start() async {
        if let unavailable = engine as? UnavailableEngineClient {
            status = .unavailable(unavailable.reason)
            return
        }
        info = engine.info()
        settings = SettingsDoc(text: engine.settingsJSON())
        listenerId = engine.addListener { [weak self] events in
            // Called on the engine's forwarder thread: hop, never block.
            Task { @MainActor [weak self] in self?.apply(events) }
        }
        await reloadSnapshot()
        await refreshDashboard()
    }

    public func stop() async {
        if listenerId != 0 { engine.removeListener(listenerId) }
        await engine.shutdown()
    }

    public func observe(_ handler: @escaping (EngineEvent) -> Void) {
        observers.append(handler)
    }

    public func reloadSnapshot() async {
        do {
            let snap = try await engine.snapshot()
            tasks.load(snap.rows, rev: snap.rev)
            queues = snap.queues.sorted { $0.position < $1.position }
            queueSummaries = Dictionary(snap.queueSummaries.map { ($0.queueId, $0) }, uniquingKeysWith: { $1 })
            categories = snap.categories.sorted { $0.position < $1.position }
            stats = snap.stats
            networkAvailable = snap.stats.networkAvailable
            status = .ready
        } catch {
            status = .unavailable(error.localizedDescription)
        }
    }

    public func refreshDashboard() async {
        guard let dash = try? await engine.dashboard() else { return }
        stats = dash.stats
        for s in dash.queues { queueSummaries[s.queueId] = s }
        if speedHistory.count < 2 { speedHistory = Array(dash.speedHistory.suffix(Self.speedHistoryCapacity)) }
        disks = dash.disks
        scheduledNext = dash.scheduledNext
        if recentCompletions.isEmpty {
            recentCompletions = dash.recent.filter { $0.state == .completed || $0.state == .seeding }
        }
    }

    // MARK: event application

    /// Applies one batch from the engine. Public for tests.
    public func apply(_ events: [EngineEvent]) {
        for event in events {
            applyOne(event)
            for o in observers { o(event) }
        }
    }

    private func applyOne(_ event: EngineEvent) {
        switch event {
        case .taskAdded(let row), .taskUpdated(let row):
            tasks.upsert(row)
            bumpDetail(row.id)
        case .taskRemoved(let id):
            tasks.remove(id)
            recentCompletions.removeAll { $0.id == id }
        case .taskStateChanged(let id, _, let to):
            tasks.applyState(id, to: to)
            bumpDetail(id)
            if to == .completed || to == .failed { historyVersion &+= 1 }
            if to == .completed, let row = tasks[id]?.data {
                recentCompletions.removeAll { $0.id == id }
                recentCompletions.insert(row, at: 0)
                if recentCompletions.count > 12 { recentCompletions.removeLast() }
            }
        case .progress(let updates):
            tasks.apply(progress: updates)
        case .taskLog(let entry):
            if liveLogs[entry.taskId] != nil {
                var lines = liveLogs[entry.taskId] ?? []
                lines.append(entry)
                if lines.count > 500 { lines.removeFirst(lines.count - 500) }
                liveLogs[entry.taskId] = lines
            }
        case .queueUpdated(let q):
            if let i = queues.firstIndex(where: { $0.id == q.id }) { queues[i] = q } else { queues.append(q) }
            queues.sort { $0.position < $1.position }
        case .queueRemoved(let id):
            queues.removeAll { $0.id == id }
            queueSummaries[id] = nil
        case .queueSummaries(let list):
            for s in list { queueSummaries[s.queueId] = s }
        case .categoriesChanged:
            Task { await reloadCategories() }
        case .rulesChanged: reloadIfLoaded("rules")
        case .schedulesChanged: reloadIfLoaded("schedules")
        case .automationsChanged: reloadIfLoaded("automations")
        case .recipesChanged: reloadIfLoaded("recipes")
        case .settingsChanged:
            settings = SettingsDoc(text: engine.settingsJSON())
        case .globalStats(let s):
            if !liveStatsMuted { record(s) }
        case .devicesChanged:
            reloadIfLoaded("devices")
        case .pairingStarted(let code, let expiresAt, let url):
            pairing = PairingData(code: code, expiresAt: expiresAt, url: url, tlsFingerprint: pairing?.tlsFingerprint)
        case .pairingCompleted(let name):
            pairing = nil
            lastPairedDevice = name
            toast(.success, "Paired with \(name)")
            reloadIfLoaded("devices")
        case .diskSpace(let path, let free):
            if let i = disks.firstIndex(where: { $0.path == path }) { disks[i].free = free }
        case .networkChanged(let available, _):
            networkAvailable = available
        case .grabberProgress(let id, let pages, let files, let done):
            grabberProgress[id] = (pages, files, done)
        case .updateCheck(let available, let version, let notes):
            updateInfo = (available, version, notes)
        case .resync:
            guard !resyncInFlight else { return }
            resyncInFlight = true
            Task {
                await reloadSnapshot()
                resyncInFlight = false
            }
        case .engineStopping:
            status = .unavailable("The engine is shutting down.")
        case .automationRan(let id, _, let success, let message):
            if !success { toast(.error, "Automation failed", detail: message) }
            reloadIfLoaded("automations")
            _ = id
        case .notification, .platformAction, .readyForSleep, .custom:
            break // handled by platform observers
        }
    }

    private func bumpDetail(_ id: String) {
        detailVersion[id, default: 0] &+= 1
    }

    // MARK: derived

    public func queue(_ id: String?) -> QueueData? { queues.first { $0.id == id } }
    public func category(_ id: String?) -> CategoryData? { categories.first { $0.id == id } }

    public var activeCount: Int { tasks.count { $0.state.isActive } }

    // MARK: actions (all errors surface as toasts)

    public func toast(_ style: Toast.Style, _ title: String, detail: String? = nil) {
        let t = Toast(style, title, detail: detail)
        toasts.append(t)
        let id = t.id
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: style == .error ? 6_000_000_000 : 3_000_000_000)
            self.toasts.removeAll { $0.id == id }
        }
    }

    /// Runs an engine call, reporting failures to the user. Returns the value or nil.
    @discardableResult
    public func perform<T>(_ what: String, _ op: () async throws -> T) async -> T? {
        do {
            return try await op()
        } catch {
            toast(.error, what, detail: error.localizedDescription)
            return nil
        }
    }

    public func act(_ action: TaskAction, on ids: [String]) {
        guard !ids.isEmpty else { return }
        Task {
            if ids.count == 1 {
                await perform("Couldn't \(action.label.lowercased())") { try await engine.taskAction(ids[0], action) }
            } else {
                await perform("Couldn't \(action.label.lowercased())") { _ = try await engine.tasksAction(ids, action) }
            }
        }
    }

    /// Pause if any selected task is running, otherwise resume (⌘.).
    public func togglePause(_ ids: [String]) {
        let items = ids.compactMap { tasks[$0] }
        if items.contains(where: { $0.state.canPause }) {
            act(.pause, on: items.filter { $0.state.canPause }.map(\.id))
        } else {
            act(.resume, on: items.filter { $0.state.canResume }.map(\.id))
        }
    }

    public func remove(_ ids: [String], deleteFiles: Bool) {
        Task { await perform("Couldn't remove") { _ = try await engine.removeTasks(ids, deleteFile: deleteFiles) } }
    }

    public func pauseAll() { Task { await perform("Couldn't pause all") { _ = try await engine.pauseAll() } } }
    public func resumeAll() { Task { await perform("Couldn't resume all") { _ = try await engine.resumeAll() } } }
    public func retryFailed() {
        Task {
            if let n = await perform("Couldn't retry", { try await engine.retryAllFailed() }) {
                toast(.info, n == 0 ? "Nothing to retry" : "Retrying \(n) download\(n == 1 ? "" : "s")")
            }
        }
    }
    public func clearCompleted() {
        Task { await perform("Couldn't clear") { _ = try await engine.clearCompleted() } }
    }

    public func setTrafficMode(_ mode: TrafficMode) {
        stats.trafficMode = mode
        Task { await perform("Couldn't change speed mode") { try await engine.setTrafficMode(mode) } }
    }

    public func saveSettings(_ doc: SettingsDoc) async -> Bool {
        let previous = settings
        settings = doc
        do {
            let saved = try await engine.updateSettingsJSON(doc.json.serialized())
            settings = SettingsDoc(text: saved)
            return true
        } catch {
            settings = previous
            toast(.error, "Settings not saved", detail: error.localizedDescription)
            return false
        }
    }

    /// Edit one settings value and persist.
    public func setSetting(_ path: String, _ value: JSONValue) {
        var doc = settings
        doc[path] = value
        Task { _ = await saveSettings(doc) }
    }

    public func reloadCategories() async {
        if let c = try? await engine.categories() { categories = c.sorted { $0.position < $1.position } }
    }

    public func reloadQueues() async {
        if let q = try? await engine.queues() { queues = q.sorted { $0.position < $1.position } }
    }

    // MARK: lazily loaded documents

    public func load(_ doc: String) async {
        loadedDocs.insert(doc)
        switch doc {
        case "rules":
            if let text = await perform("Couldn't load rules", { try await engine.rulesJSON() }) {
                rules = (try? EngineJSON.decode([RuleDoc].self, from: text))?.sorted { $0.priority < $1.priority } ?? []
            }
        case "schedules":
            if let text = await perform("Couldn't load schedules", { try await engine.schedulesJSON() }) {
                schedules = (try? EngineJSON.decode([ScheduleDoc].self, from: text)) ?? []
            }
        case "automations":
            if let text = await perform("Couldn't load automations", { try await engine.automationsJSON() }) {
                automations = (try? EngineJSON.decode([AutomationDoc].self, from: text)) ?? []
            }
        case "recipes":
            if let text = await perform("Couldn't load recipes", { try await engine.recipesJSON() }) {
                recipes = (try? EngineJSON.decode([RecipeDoc].self, from: text)) ?? []
            }
        case "devices":
            if let d = await perform("Couldn't load devices", { try await engine.devices() }) { devices = d }
        case "credentials":
            if let c = await perform("Couldn't load credentials", { try await engine.credentials() }) { credentials = c }
        default: break
        }
    }

    private func reloadIfLoaded(_ doc: String) {
        guard loadedDocs.contains(doc) else { return }
        Task { await load(doc) }
    }

    public func saveRule(_ rule: RuleDoc) async -> Bool {
        var r = rule
        r.updatedAt = Date().millis
        guard let json = try? EngineJSON.encode(r) else { return false }
        let ok = await perform("Couldn't save rule") { _ = try await engine.saveRuleJSON(json) } != nil
        if ok { await load("rules") }
        return ok
    }

    public func saveSchedule(_ s: ScheduleDoc) async -> Bool {
        var d = s
        d.updatedAt = Date().millis
        guard let json = try? EngineJSON.encode(d) else { return false }
        let ok = await perform("Couldn't save schedule") { _ = try await engine.saveScheduleJSON(json) } != nil
        if ok { await load("schedules") }
        return ok
    }

    public func saveAutomation(_ a: AutomationDoc) async -> Bool {
        var d = a
        d.updatedAt = Date().millis
        guard let json = try? EngineJSON.encode(d) else { return false }
        let ok = await perform("Couldn't save automation") { _ = try await engine.saveAutomationJSON(json) } != nil
        if ok { await load("automations") }
        return ok
    }

    public func saveRecipe(_ r: RecipeDoc) async -> Bool {
        var d = r
        d.updatedAt = Date().millis
        guard let json = try? EngineJSON.encode(d) else { return false }
        let ok = await perform("Couldn't save recipe") { _ = try await engine.saveRecipeJSON(json) } != nil
        if ok { await load("recipes") }
        return ok
    }

    // MARK: devices

    public func startPairing(_ scopes: [DeviceScope]) async {
        if let p = await perform("Couldn't start pairing", { try await engine.startPairing(scopes) }) { pairing = p }
    }

    public func cancelPairing() async {
        await perform("Couldn't cancel pairing") { try await engine.cancelPairing() }
        pairing = nil
    }

    // MARK: live logs

    public func watchLog(_ id: String) async {
        if liveLogs[id] == nil { liveLogs[id] = [] }
        if let entries = try? await engine.taskLog(id, limit: 300) { liveLogs[id] = entries }
    }

    public func unwatchLog(_ id: String) { liveLogs[id] = nil }
}
