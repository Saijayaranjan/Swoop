import Foundation

/// The single door to the Rust engine. Everything in the app talks to `EngineClient`; only this
/// file and `Mapping.swift` know about the generated UniFFI types.
public protocol EngineClient: AnyObject, Sendable {
    // lifecycle & events
    func info() -> EngineInfoData
    /// Registers a batch listener; the closure is called off the main thread.
    func addListener(_ handler: @escaping @Sendable ([EngineEvent]) -> Void) -> UInt64
    func removeListener(_ id: UInt64)
    func snapshot() async throws -> Snapshot
    func shutdown() async

    // tasks
    func taskDetail(_ id: String) async throws -> TaskDetailData
    func probe(_ request: NewTaskRequestData) async throws -> ProbeResultData
    func addTask(_ request: NewTaskRequestData) async throws -> AddResultData
    func addTasks(_ requests: [NewTaskRequestData]) async throws -> [AddResultData]
    func taskAction(_ id: String, _ action: TaskAction) async throws
    func tasksAction(_ ids: [String], _ action: TaskAction) async throws -> UInt32
    func retryFromSource(_ id: String, newURL: String?) async throws
    func verify(_ id: String, checksum: String?) async throws
    func setSeedingLimits(_ id: String, ratio: Double?, minutes: UInt32?) async throws
    func removeTasks(_ ids: [String], deleteFile: Bool) async throws -> UInt32
    func updateTask(_ id: String, patch: TaskPatchData) async throws
    func resolveDuplicate(_ id: String, policy: ConflictPolicy) async throws
    func reorderTasks(_ ids: [String], after: String?) async throws
    func setTaskLimit(_ id: String, download: UInt64?, upload: UInt64?) async throws
    func setTaskConnections(_ id: String, _ connections: UInt8) async throws
    func setTaskPriority(_ id: String, _ priority: Priority) async throws
    func taskLog(_ id: String, limit: UInt32) async throws -> [LogEntryData]
    func diagnosticsText(_ id: String) async throws -> String
    func pauseAll() async throws -> UInt32
    func resumeAll() async throws -> UInt32
    func retryAllFailed() async throws -> UInt32
    func clearCompleted() async throws -> UInt32

    // torrents
    func setTorrentFiles(_ id: String, _ files: [TorrentFileData]) async throws
    func setTorrentSequential(_ id: String, _ sequential: Bool) async throws
    func torrentPeers(_ id: String) async throws -> [PeerData]
    func addTrackers(_ id: String, _ trackers: [String]) async throws
    func removeTracker(_ id: String, _ tracker: String) async throws
    func setTrackerEnabled(_ id: String, _ tracker: String, _ enabled: Bool) async throws
    func reannounce(_ id: String) async throws
    func refreshTrackerList() async throws -> UInt32

    // media
    func detectMedia(url: String, pageUrl: String?) async throws -> DetectedMediaData

    // queues & categories
    func queues() async throws -> [QueueData]
    func saveQueue(_ queue: QueueData) async throws -> QueueData
    func deleteQueue(_ id: String, moveTo: String?) async throws
    func pauseQueue(_ id: String) async throws
    func resumeQueue(_ id: String) async throws
    func reorderQueues(_ ids: [String]) async throws
    func categories() async throws -> [CategoryData]
    func saveCategory(_ category: CategoryData) async throws -> CategoryData
    func deleteCategory(_ id: String) async throws

    // JSON-shaped documents
    func rulesJSON() async throws -> String
    func saveRuleJSON(_ json: String) async throws -> String
    func deleteRule(_ id: String) async throws
    func testRulesJSON(_ subjectJSON: String) async throws -> String
    func schedulesJSON() async throws -> String
    func saveScheduleJSON(_ json: String) async throws -> String
    func deleteSchedule(_ id: String) async throws
    func automationsJSON() async throws -> String
    func saveAutomationJSON(_ json: String) async throws -> String
    func deleteAutomation(_ id: String) async throws
    func grantAutomationConsent(_ id: String) async throws
    func automationRunsJSON(_ id: String?, limit: UInt32) async throws -> String
    func runAutomation(_ id: String, taskId: String) async throws
    func recipesJSON() async throws -> String
    func saveRecipeJSON(_ json: String) async throws -> String
    func deleteRecipe(_ id: String) async throws
    func applyRecipe(_ id: String, _ request: NewTaskRequestData) async throws -> AddResultData

    // history
    func history(_ query: HistoryQueryData) async throws -> [HistoryEntryData]
    func historyCount(_ query: HistoryQueryData) async throws -> UInt32
    func deleteHistory(_ ids: [String]) async throws -> UInt32
    func clearHistory() async throws -> UInt32

    // bandwidth & settings
    func setTrafficMode(_ mode: TrafficMode) async throws
    func setGlobalLimits(download: UInt64, upload: UInt64) async throws
    func optimize() async throws -> String
    func settingsJSON() -> String
    func updateSettingsJSON(_ json: String) async throws -> String
    func storeCredential(name: String, username: String?, secret: String) async throws -> String
    func credentials() async throws -> [CredentialData]
    func deleteCredential(_ id: String) async throws

    // stats
    func globalStats() -> GlobalStatsData
    func dashboard() async throws -> DashboardData
    func diskInfo(_ path: String?) async throws -> DiskInfoData

    // devices
    func devices() async throws -> [DeviceData]
    func startPairing(_ scopes: [DeviceScope]) async throws -> PairingData
    func cancelPairing() async throws
    func revokeDevice(_ id: String) async throws
    func renameDevice(_ id: String, name: String) async throws
    func auditLog(limit: UInt32) async throws -> [AuditEntryData]

    // grabber
    func grabberStart(_ options: GrabberOptionsData) async throws -> GrabberSessionData
    func grabberStatus(_ id: String) async throws -> GrabberSessionData
    func grabberCancel(_ id: String) async throws
    func grabberAdd(_ id: String, urls: [String], request: NewTaskRequestData) async throws -> [AddResultData]
    func grabberList() async throws -> [GrabberSessionData]

    // archives, import/export, updates, plugins, logs
    func archiveList(_ path: String) async throws -> ArchiveListingData
    func archiveExtract(_ path: String, entries: [String]?, destination: String) async throws -> UInt32
    func exportJSON(tasks: Bool, history: Bool) async throws -> String
    func importJSON(_ json: String, options: ImportOptionsData) async throws -> ImportReportData
    func checkForUpdates() async throws -> UpdateInfoData
    func downloadUpdate() async throws -> String
    func plugins() async throws -> [PluginData]
    func setPluginEnabled(_ id: String, _ enabled: Bool, granted: [String]) async throws
    func uninstallPlugin(_ id: String) async throws
    func recentLogs(limit: UInt32, level: String?) async throws -> [String]

    // platform inputs
    func updateEnvironment(_ env: EnvironmentData)
    func setActiveWindow(_ active: Bool)
}

/// Editable task fields (`FfiTaskPatch`); `nil` leaves a field unchanged.
public struct TaskPatchData: Equatable, Sendable {
    public var name: String?
    public var directory: String?
    public var queueId: String?
    /// `.some(nil)` clears the category.
    public var categoryId: String??
    public var scheduleId: String??
    public var priority: Priority?
    public var tags: [String]?
    public var options: TaskOptionsData?
    public var mirrors: [String]?
    public init() {}
}

public struct ImportOptionsData: Equatable, Sendable {
    public var settings = true, queues = true, categories = true, rules = true, schedules = true
    public var automations = true, tasks = true, history = true, recipes = true, overwrite = false
    public init() {}
}

/// Used when the engine library is not linked (UI development, unit tests) or failed to open.
/// Every call fails with a clear `unavailable` error — the UI shows it, never fake data.
public final class UnavailableEngineClient: EngineClient, @unchecked Sendable {
    public let reason: String
    public init(reason: String) { self.reason = reason }
    private func fail<T>() throws -> T { throw EngineError.unavailable(reason) }

    public func info() -> EngineInfoData { EngineInfoData() }
    public func addListener(_ handler: @escaping @Sendable ([EngineEvent]) -> Void) -> UInt64 { 0 }
    public func removeListener(_ id: UInt64) {}
    public func snapshot() async throws -> Snapshot { try fail() }
    public func shutdown() async {}
    public func taskDetail(_ id: String) async throws -> TaskDetailData { try fail() }
    public func probe(_ request: NewTaskRequestData) async throws -> ProbeResultData { try fail() }
    public func addTask(_ request: NewTaskRequestData) async throws -> AddResultData { try fail() }
    public func addTasks(_ requests: [NewTaskRequestData]) async throws -> [AddResultData] { try fail() }
    public func taskAction(_ id: String, _ action: TaskAction) async throws { try fail() as Void }
    public func tasksAction(_ ids: [String], _ action: TaskAction) async throws -> UInt32 { try fail() }
    public func retryFromSource(_ id: String, newURL: String?) async throws { try fail() as Void }
    public func verify(_ id: String, checksum: String?) async throws { try fail() as Void }
    public func setSeedingLimits(_ id: String, ratio: Double?, minutes: UInt32?) async throws { try fail() as Void }
    public func removeTasks(_ ids: [String], deleteFile: Bool) async throws -> UInt32 { try fail() }
    public func updateTask(_ id: String, patch: TaskPatchData) async throws { try fail() as Void }
    public func resolveDuplicate(_ id: String, policy: ConflictPolicy) async throws { try fail() as Void }
    public func reorderTasks(_ ids: [String], after: String?) async throws { try fail() as Void }
    public func setTaskLimit(_ id: String, download: UInt64?, upload: UInt64?) async throws { try fail() as Void }
    public func setTaskConnections(_ id: String, _ connections: UInt8) async throws { try fail() as Void }
    public func setTaskPriority(_ id: String, _ priority: Priority) async throws { try fail() as Void }
    public func taskLog(_ id: String, limit: UInt32) async throws -> [LogEntryData] { try fail() }
    public func diagnosticsText(_ id: String) async throws -> String { try fail() }
    public func pauseAll() async throws -> UInt32 { try fail() }
    public func resumeAll() async throws -> UInt32 { try fail() }
    public func retryAllFailed() async throws -> UInt32 { try fail() }
    public func clearCompleted() async throws -> UInt32 { try fail() }
    public func setTorrentFiles(_ id: String, _ files: [TorrentFileData]) async throws { try fail() as Void }
    public func setTorrentSequential(_ id: String, _ sequential: Bool) async throws { try fail() as Void }
    public func torrentPeers(_ id: String) async throws -> [PeerData] { try fail() }
    public func addTrackers(_ id: String, _ trackers: [String]) async throws { try fail() as Void }
    public func removeTracker(_ id: String, _ tracker: String) async throws { try fail() as Void }
    public func setTrackerEnabled(_ id: String, _ tracker: String, _ enabled: Bool) async throws { try fail() as Void }
    public func reannounce(_ id: String) async throws { try fail() as Void }
    public func refreshTrackerList() async throws -> UInt32 { try fail() }
    public func detectMedia(url: String, pageUrl: String?) async throws -> DetectedMediaData { try fail() }
    public func queues() async throws -> [QueueData] { try fail() }
    public func saveQueue(_ queue: QueueData) async throws -> QueueData { try fail() }
    public func deleteQueue(_ id: String, moveTo: String?) async throws { try fail() as Void }
    public func pauseQueue(_ id: String) async throws { try fail() as Void }
    public func resumeQueue(_ id: String) async throws { try fail() as Void }
    public func reorderQueues(_ ids: [String]) async throws { try fail() as Void }
    public func categories() async throws -> [CategoryData] { try fail() }
    public func saveCategory(_ category: CategoryData) async throws -> CategoryData { try fail() }
    public func deleteCategory(_ id: String) async throws { try fail() as Void }
    public func rulesJSON() async throws -> String { try fail() }
    public func saveRuleJSON(_ json: String) async throws -> String { try fail() }
    public func deleteRule(_ id: String) async throws { try fail() as Void }
    public func testRulesJSON(_ subjectJSON: String) async throws -> String { try fail() }
    public func schedulesJSON() async throws -> String { try fail() }
    public func saveScheduleJSON(_ json: String) async throws -> String { try fail() }
    public func deleteSchedule(_ id: String) async throws { try fail() as Void }
    public func automationsJSON() async throws -> String { try fail() }
    public func saveAutomationJSON(_ json: String) async throws -> String { try fail() }
    public func deleteAutomation(_ id: String) async throws { try fail() as Void }
    public func grantAutomationConsent(_ id: String) async throws { try fail() as Void }
    public func automationRunsJSON(_ id: String?, limit: UInt32) async throws -> String { try fail() }
    public func runAutomation(_ id: String, taskId: String) async throws { try fail() as Void }
    public func recipesJSON() async throws -> String { try fail() }
    public func saveRecipeJSON(_ json: String) async throws -> String { try fail() }
    public func deleteRecipe(_ id: String) async throws { try fail() as Void }
    public func applyRecipe(_ id: String, _ request: NewTaskRequestData) async throws -> AddResultData { try fail() }
    public func history(_ query: HistoryQueryData) async throws -> [HistoryEntryData] { try fail() }
    public func historyCount(_ query: HistoryQueryData) async throws -> UInt32 { try fail() }
    public func deleteHistory(_ ids: [String]) async throws -> UInt32 { try fail() }
    public func clearHistory() async throws -> UInt32 { try fail() }
    public func setTrafficMode(_ mode: TrafficMode) async throws { try fail() as Void }
    public func setGlobalLimits(download: UInt64, upload: UInt64) async throws { try fail() as Void }
    public func optimize() async throws -> String { try fail() }
    public func settingsJSON() -> String { "{}" }
    public func updateSettingsJSON(_ json: String) async throws -> String { try fail() }
    public func storeCredential(name: String, username: String?, secret: String) async throws -> String { try fail() }
    public func credentials() async throws -> [CredentialData] { try fail() }
    public func deleteCredential(_ id: String) async throws { try fail() as Void }
    public func globalStats() -> GlobalStatsData { GlobalStatsData() }
    public func dashboard() async throws -> DashboardData { try fail() }
    public func diskInfo(_ path: String?) async throws -> DiskInfoData { try fail() }
    public func devices() async throws -> [DeviceData] { try fail() }
    public func startPairing(_ scopes: [DeviceScope]) async throws -> PairingData { try fail() }
    public func cancelPairing() async throws { try fail() as Void }
    public func revokeDevice(_ id: String) async throws { try fail() as Void }
    public func renameDevice(_ id: String, name: String) async throws { try fail() as Void }
    public func auditLog(limit: UInt32) async throws -> [AuditEntryData] { try fail() }
    public func grabberStart(_ options: GrabberOptionsData) async throws -> GrabberSessionData { try fail() }
    public func grabberStatus(_ id: String) async throws -> GrabberSessionData { try fail() }
    public func grabberCancel(_ id: String) async throws { try fail() as Void }
    public func grabberAdd(_ id: String, urls: [String], request: NewTaskRequestData) async throws -> [AddResultData] { try fail() }
    public func grabberList() async throws -> [GrabberSessionData] { try fail() }
    public func archiveList(_ path: String) async throws -> ArchiveListingData { try fail() }
    public func archiveExtract(_ path: String, entries: [String]?, destination: String) async throws -> UInt32 { try fail() }
    public func exportJSON(tasks: Bool, history: Bool) async throws -> String { try fail() }
    public func importJSON(_ json: String, options: ImportOptionsData) async throws -> ImportReportData { try fail() }
    public func checkForUpdates() async throws -> UpdateInfoData { try fail() }
    public func downloadUpdate() async throws -> String { try fail() }
    public func plugins() async throws -> [PluginData] { try fail() }
    public func setPluginEnabled(_ id: String, _ enabled: Bool, granted: [String]) async throws { try fail() as Void }
    public func uninstallPlugin(_ id: String) async throws { try fail() as Void }
    public func recentLogs(limit: UInt32, level: String?) async throws -> [String] { try fail() }
    public func updateEnvironment(_ env: EnvironmentData) {}
    public func setActiveWindow(_ active: Bool) {}
}

/// Opens the engine. Falls back to `UnavailableEngineClient` with the reason when the library is
/// not linked or `open` fails, so the UI can explain the problem instead of crashing.
public enum EngineFactory {
    public static func open(dataDirectory: URL, appVersion: String, bundleId: String) -> EngineClient {
        #if SWOOP_FFI
        do {
            return try LiveEngineClient(dataDirectory: nil, appVersion: appVersion, bundleId: bundleId)
        } catch {
            return UnavailableEngineClient(reason: "The engine could not start: \(error.localizedDescription)")
        }
        #else
        return UnavailableEngineClient(reason: "This build of Swoop was compiled without the engine library.")
        #endif
    }
}

#if SWOOP_FFI
// MARK: - Live client over the generated UniFFI bindings

/// Adapts the engine's batch callback to a Swift closure (called on the forwarder thread).
private final class ListenerBridge: EventListener, @unchecked Sendable {
    let handler: @Sendable ([EngineEvent]) -> Void
    init(_ handler: @escaping @Sendable ([EngineEvent]) -> Void) { self.handler = handler }
    func onEvents(events: [FfiEvent]) { handler(events.map(Mapping.event)) }
}

public final class LiveEngineClient: EngineClient, @unchecked Sendable {
    private let engine: SwoopEngine
    private let lock = NSLock()
    private var bridges: [UInt64: ListenerBridge] = [:]

    public init(dataDirectory: URL?, appVersion: String, bundleId: String) throws {
        var config = FfiEngineConfig()
        config.dataDir = dataDirectory?.path
        config.appVersion = appVersion
        config.bundleId = bundleId
        config.headless = false
        do {
            engine = try SwoopEngine.open(config: config)
        } catch {
            throw Mapping.error(error)
        }
    }

    /// Runs an engine call, translating `FfiError` into `EngineError`.
    @inline(__always)
    private func call<T>(_ op: () async throws -> T) async throws -> T {
        do { return try await op() } catch { throw Mapping.error(error) }
    }

    public func info() -> EngineInfoData {
        let i = engine.info()
        var d = EngineInfoData()
        d.version = i.version; d.build = i.build; d.os = i.os; d.arch = i.arch; d.dataDir = i.dataDir
        d.localApiPort = i.localApiPort; d.remoteEnabled = i.remoteEnabled; d.remotePort = i.remotePort
        d.uptimeSeconds = i.uptimeSeconds; d.ffmpegAvailable = i.ffmpegAvailable
        return d
    }

    public func addListener(_ handler: @escaping @Sendable ([EngineEvent]) -> Void) -> UInt64 {
        let bridge = ListenerBridge(handler)
        let id = engine.addListener(listener: bridge)
        lock.lock(); bridges[id] = bridge; lock.unlock()
        return id
    }

    public func removeListener(_ id: UInt64) {
        engine.removeListener(id: id)
        lock.lock(); bridges[id] = nil; lock.unlock()
    }

    public func snapshot() async throws -> Snapshot {
        let s = try await call { try await engine.snapshot() }
        return Snapshot(rev: s.rev, rows: s.rows.map(Mapping.row), queues: s.queues.map(Mapping.queue),
                        queueSummaries: s.queueSummaries.map(Mapping.summary),
                        categories: s.categories.map(Mapping.category), stats: Mapping.stats(s.stats))
    }

    public func shutdown() async { try? await engine.shutdown() }

    // tasks
    public func taskDetail(_ id: String) async throws -> TaskDetailData { Mapping.detail(try await call { try await engine.taskDetail(id: id) }) }
    public func probe(_ request: NewTaskRequestData) async throws -> ProbeResultData { Mapping.probe(try await call { try await engine.probe(request: Mapping.ffi(request)) }) }
    public func addTask(_ request: NewTaskRequestData) async throws -> AddResultData { Mapping.add(try await call { try await engine.addTask(request: Mapping.ffi(request)) }) }
    public func addTasks(_ requests: [NewTaskRequestData]) async throws -> [AddResultData] {
        try await call { try await engine.addTasks(requests: requests.map(Mapping.ffi)) }.map(Mapping.add)
    }
    public func taskAction(_ id: String, _ action: TaskAction) async throws { _ = try await call { try await engine.taskAction(id: id, action: Mapping.ffi(action)) } }
    public func tasksAction(_ ids: [String], _ action: TaskAction) async throws -> UInt32 { try await call { try await engine.tasksAction(ids: ids, action: Mapping.ffi(action)) } }
    public func retryFromSource(_ id: String, newURL: String?) async throws { _ = try await call { try await engine.taskAction(id: id, action: .retryFromSource(newUrl: newURL)) } }
    public func verify(_ id: String, checksum: String?) async throws { _ = try await call { try await engine.taskAction(id: id, action: .verify(checksum: checksum)) } }
    public func setSeedingLimits(_ id: String, ratio: Double?, minutes: UInt32?) async throws {
        _ = try await call { try await engine.setSeedingLimits(id: id, limits: FfiSeedingLimits(ratioLimit: ratio.map(Float.init), timeLimitMinutes: minutes)) }
    }
    public func removeTasks(_ ids: [String], deleteFile: Bool) async throws -> UInt32 { try await call { try await engine.removeTasks(ids: ids, deleteFile: deleteFile) } }
    public func updateTask(_ id: String, patch: TaskPatchData) async throws { _ = try await call { try await engine.updateTask(id: id, patch: Mapping.ffi(patch)) } }
    public func resolveDuplicate(_ id: String, policy: ConflictPolicy) async throws { _ = try await call { try await engine.resolveDuplicate(id: id, policy: Mapping.ffi(policy)) } }
    public func reorderTasks(_ ids: [String], after: String?) async throws { try await call { try await engine.reorderTasks(ids: ids, after: after) } }
    public func setTaskLimit(_ id: String, download: UInt64?, upload: UInt64?) async throws { _ = try await call { try await engine.setTaskLimit(id: id, download: download, upload: upload) } }
    public func setTaskConnections(_ id: String, _ connections: UInt8) async throws { _ = try await call { try await engine.setTaskConnections(id: id, connections: connections) } }
    public func setTaskPriority(_ id: String, _ priority: Priority) async throws { _ = try await call { try await engine.setTaskPriority(id: id, priority: Mapping.ffi(priority)) } }
    public func taskLog(_ id: String, limit: UInt32) async throws -> [LogEntryData] { try await call { try await engine.taskLog(id: id, limit: limit) }.map(Mapping.log) }
    public func diagnosticsText(_ id: String) async throws -> String { try await call { try await engine.diagnosticsText(id: id) } }
    public func pauseAll() async throws -> UInt32 { try await call { try await engine.pauseAll() } }
    public func resumeAll() async throws -> UInt32 { try await call { try await engine.resumeAll() } }
    public func retryAllFailed() async throws -> UInt32 { try await call { try await engine.retryAllFailed() } }
    public func clearCompleted() async throws -> UInt32 { try await call { try await engine.clearCompleted() } }

    // torrents
    public func setTorrentFiles(_ id: String, _ files: [TorrentFileData]) async throws {
        let sel = files.map { FfiFileSelection(index: $0.index, selected: $0.selected, priority: $0.priority) }
        _ = try await call { try await engine.setTorrentFiles(id: id, selection: sel) }
    }
    public func setTorrentSequential(_ id: String, _ sequential: Bool) async throws { _ = try await call { try await engine.setTorrentSequential(id: id, sequential: sequential) } }
    public func torrentPeers(_ id: String) async throws -> [PeerData] {
        try await call { try await engine.torrentPeers(id: id) }.map {
            PeerData(address: $0.address, client: $0.client, downloadSpeed: $0.downloadSpeed, uploadSpeed: $0.uploadSpeed,
                     progress: Double($0.progress), flags: $0.flags)
        }
    }
    public func addTrackers(_ id: String, _ trackers: [String]) async throws { _ = try await call { try await engine.addTrackers(id: id, trackers: trackers) } }
    public func removeTracker(_ id: String, _ tracker: String) async throws { _ = try await call { try await engine.removeTracker(id: id, tracker: tracker) } }
    public func setTrackerEnabled(_ id: String, _ tracker: String, _ enabled: Bool) async throws { _ = try await call { try await engine.setTrackerEnabled(id: id, tracker: tracker, enabled: enabled) } }
    public func reannounce(_ id: String) async throws { try await call { try await engine.reannounce(id: id) } }
    public func refreshTrackerList() async throws -> UInt32 { try await call { try await engine.refreshTrackerList() } }

    // media
    public func detectMedia(url: String, pageUrl: String?) async throws -> DetectedMediaData {
        let m = try await call { try await engine.detectMedia(url: url, pageUrl: pageUrl) }
        return DetectedMediaData(url: m.url, kind: Mapping.snake(m.kind), title: m.title, size: m.size,
                                 variants: m.variants.map(Mapping.variant), protected: m.protected)
    }

    // queues & categories
    public func queues() async throws -> [QueueData] { try await call { try await engine.queues() }.map(Mapping.queue) }
    public func saveQueue(_ queue: QueueData) async throws -> QueueData { Mapping.queue(try await call { try await engine.saveQueue(queue: Mapping.ffi(queue)) }) }
    public func deleteQueue(_ id: String, moveTo: String?) async throws { try await call { try await engine.deleteQueue(id: id, moveTo: moveTo) } }
    public func pauseQueue(_ id: String) async throws { _ = try await call { try await engine.pauseQueue(id: id) } }
    public func resumeQueue(_ id: String) async throws { _ = try await call { try await engine.resumeQueue(id: id) } }
    public func reorderQueues(_ ids: [String]) async throws { try await call { try await engine.reorderQueues(ids: ids) } }
    public func categories() async throws -> [CategoryData] { try await call { try await engine.categories() }.map(Mapping.category) }
    public func saveCategory(_ category: CategoryData) async throws -> CategoryData { Mapping.category(try await call { try await engine.saveCategory(category: Mapping.ffi(category)) }) }
    public func deleteCategory(_ id: String) async throws { try await call { try await engine.deleteCategory(id: id) } }

    // documents
    public func rulesJSON() async throws -> String { try await call { try await engine.rulesJson() } }
    public func saveRuleJSON(_ json: String) async throws -> String { try await call { try await engine.saveRuleJson(json: json) } }
    public func deleteRule(_ id: String) async throws { try await call { try await engine.deleteRule(id: id) } }
    public func testRulesJSON(_ subjectJSON: String) async throws -> String { try await call { try await engine.testRulesJson(subjectJson: subjectJSON) } }
    public func schedulesJSON() async throws -> String { try await call { try await engine.schedulesJson() } }
    public func saveScheduleJSON(_ json: String) async throws -> String { try await call { try await engine.saveScheduleJson(json: json) } }
    public func deleteSchedule(_ id: String) async throws { try await call { try await engine.deleteSchedule(id: id) } }
    public func automationsJSON() async throws -> String { try await call { try await engine.automationsJson() } }
    public func saveAutomationJSON(_ json: String) async throws -> String { try await call { try await engine.saveAutomationJson(json: json) } }
    public func deleteAutomation(_ id: String) async throws { try await call { try await engine.deleteAutomation(id: id) } }
    public func grantAutomationConsent(_ id: String) async throws { _ = try await call { try await engine.grantAutomationConsent(id: id) } }
    public func automationRunsJSON(_ id: String?, limit: UInt32) async throws -> String { try await call { try await engine.automationRunsJson(id: id, limit: limit) } }
    public func runAutomation(_ id: String, taskId: String) async throws { _ = try await call { try await engine.runAutomation(id: id, taskId: taskId) } }
    public func recipesJSON() async throws -> String { try await call { try await engine.recipesJson() } }
    public func saveRecipeJSON(_ json: String) async throws -> String { try await call { try await engine.saveRecipeJson(json: json) } }
    public func deleteRecipe(_ id: String) async throws { try await call { try await engine.deleteRecipe(id: id) } }
    public func applyRecipe(_ id: String, _ request: NewTaskRequestData) async throws -> AddResultData {
        Mapping.add(try await call { try await engine.applyRecipe(id: id, request: Mapping.ffi(request)) })
    }

    // history
    public func history(_ query: HistoryQueryData) async throws -> [HistoryEntryData] { try await call { try await engine.history(query: Mapping.ffi(query)) }.map(Mapping.history) }
    public func historyCount(_ query: HistoryQueryData) async throws -> UInt32 { try await call { try await engine.historyCount(query: Mapping.ffi(query)) } }
    public func deleteHistory(_ ids: [String]) async throws -> UInt32 { try await call { try await engine.deleteHistory(ids: ids) } }
    public func clearHistory() async throws -> UInt32 { try await call { try await engine.clearHistory() } }

    // bandwidth & settings
    public func setTrafficMode(_ mode: TrafficMode) async throws { try await call { try await engine.setTrafficMode(mode: Mapping.ffi(mode)) } }
    public func setGlobalLimits(download: UInt64, upload: UInt64) async throws { try await call { try await engine.setGlobalLimits(download: download, upload: upload) } }
    public func optimize() async throws -> String { try await call { try await engine.optimize() } }
    public func settingsJSON() -> String { engine.settingsJson() }
    public func updateSettingsJSON(_ json: String) async throws -> String { try await call { try await engine.updateSettingsJson(json: json) } }
    public func storeCredential(name: String, username: String?, secret: String) async throws -> String {
        try await call { try await engine.storeCredential(name: name, username: username, secret: secret) }
    }
    public func credentials() async throws -> [CredentialData] {
        try await call { try await engine.credentials() }.map { CredentialData(id: $0.id, name: $0.name, username: $0.username) }
    }
    public func deleteCredential(_ id: String) async throws { try await call { try await engine.deleteCredential(id: id) } }

    // stats
    public func globalStats() -> GlobalStatsData { Mapping.stats(engine.globalStats()) }
    public func dashboard() async throws -> DashboardData {
        let d = try await call { try await engine.dashboard() }
        return DashboardData(stats: Mapping.stats(d.stats), queues: d.queues.map(Mapping.summary), recent: d.recent.map(Mapping.row),
                             speedHistory: d.speedHistory.map { SpeedSampleData(at: $0.at, download: $0.download, upload: $0.upload) },
                             disks: d.disks.map(Mapping.disk), scheduledNext: d.scheduledNext.map { ($0.scheduleId, $0.at) })
    }
    public func diskInfo(_ path: String?) async throws -> DiskInfoData { Mapping.disk(try await call { try await engine.diskInfo(path: path) }) }

    // devices
    public func devices() async throws -> [DeviceData] { try await call { try await engine.devices() }.map(Mapping.device) }
    public func startPairing(_ scopes: [DeviceScope]) async throws -> PairingData {
        let p = try await call { try await engine.startPairing(scopes: scopes.map(Mapping.ffi)) }
        return PairingData(code: p.code, expiresAt: p.expiresAt, url: p.url, tlsFingerprint: p.tlsFingerprint)
    }
    public func cancelPairing() async throws { try await call { try await engine.cancelPairing() } }
    public func revokeDevice(_ id: String) async throws { try await call { try await engine.revokeDevice(id: id) } }
    public func renameDevice(_ id: String, name: String) async throws { _ = try await call { try await engine.renameDevice(id: id, name: name) } }
    public func auditLog(limit: UInt32) async throws -> [AuditEntryData] {
        try await call { try await engine.auditLog(limit: limit) }.map {
            AuditEntryData(at: $0.at, deviceId: $0.deviceId, ip: $0.ip, action: $0.action, target: $0.target, success: $0.success, detail: $0.detail)
        }
    }

    // grabber
    public func grabberStart(_ options: GrabberOptionsData) async throws -> GrabberSessionData { Mapping.grabber(try await call { try await engine.grabberStart(options: Mapping.ffi(options)) }) }
    public func grabberStatus(_ id: String) async throws -> GrabberSessionData { Mapping.grabber(try await call { try await engine.grabberStatus(id: id) }) }
    public func grabberCancel(_ id: String) async throws { try await call { try await engine.grabberCancel(id: id) } }
    public func grabberAdd(_ id: String, urls: [String], request: NewTaskRequestData) async throws -> [AddResultData] {
        try await call { try await engine.grabberAdd(id: id, urls: urls, request: Mapping.ffi(request)) }.map(Mapping.add)
    }
    public func grabberList() async throws -> [GrabberSessionData] { try await call { try await engine.grabberList() }.map(Mapping.grabber) }

    // archives, import/export, updates, plugins, logs
    public func archiveList(_ path: String) async throws -> ArchiveListingData {
        let l = try await call { try await engine.archiveList(path: path) }
        return ArchiveListingData(format: l.format, entries: l.entries.map { ArchiveEntryData(path: $0.path, size: $0.size, isDir: $0.isDir) },
                                  truncated: l.truncated, intact: l.intact)
    }
    public func archiveExtract(_ path: String, entries: [String]?, destination: String) async throws -> UInt32 {
        try await call { try await engine.archiveExtract(path: path, entries: entries, destination: destination) }
    }
    public func exportJSON(tasks: Bool, history: Bool) async throws -> String { try await call { try await engine.exportJson(includeTasks: tasks, includeHistory: history) } }
    public func importJSON(_ json: String, options: ImportOptionsData) async throws -> ImportReportData {
        let r = try await call { try await engine.importJson(json: json, options: Mapping.ffi(options)) }
        return ImportReportData(imported: r.imported, skipped: r.skipped, errors: r.errors)
    }
    public func checkForUpdates() async throws -> UpdateInfoData {
        let u = try await call { try await engine.checkForUpdates() }
        return UpdateInfoData(currentVersion: u.currentVersion, available: u.available, latestVersion: u.latestVersion,
                              notes: u.notes, signatureValid: u.signatureValid, checkedAt: u.checkedAt)
    }
    public func downloadUpdate() async throws -> String { try await call { try await engine.downloadUpdate() } }
    public func plugins() async throws -> [PluginData] {
        try await call { try await engine.plugins() }.map {
            PluginData(id: $0.id, name: $0.name, version: $0.version, description: $0.description, author: $0.author,
                       permissions: $0.permissions, grantedPermissions: $0.grantedPermissions, enabled: $0.enabled)
        }
    }
    public func setPluginEnabled(_ id: String, _ enabled: Bool, granted: [String]) async throws {
        _ = try await call { try await engine.setPluginEnabled(id: id, enabled: enabled, grantedPermissions: granted) }
    }
    public func uninstallPlugin(_ id: String) async throws { try await call { try await engine.uninstallPlugin(id: id) } }
    public func recentLogs(limit: UInt32, level: String?) async throws -> [String] { try await call { try await engine.recentLogs(limit: limit, level: level) } }

    // platform inputs
    public func updateEnvironment(_ env: EnvironmentData) { engine.updateEnvironment(env: Mapping.ffi(env)) }
    public func setActiveWindow(_ active: Bool) { engine.setActiveWindow(active: active) }
}
#endif
