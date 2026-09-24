import Foundation

// App-side value types. `Mapping.swift` converts the generated UniFFI records into these; nothing
// outside `Engine/` ever sees an `Ffi*` type.

public struct ProgressData: Equatable, Sendable {
    public var downloaded: UInt64 = 0
    public var uploaded: UInt64 = 0
    public var total: UInt64?
    public var speed: UInt64 = 0
    public var uploadSpeed: UInt64 = 0
    public var etaSeconds: UInt64?
    public var activeConnections: UInt32 = 0
    public var peers: UInt32 = 0
    public var seeds: UInt32 = 0
    public var ratio: Double = 0
    /// 0…1, derived by the engine when the total is unknown.
    public var fraction: Double = 0

    public init() {}

    public var effectiveFraction: Double {
        if let total, total > 0 { return min(1, Double(downloaded) / Double(total)) }
        return fraction
    }
}

/// A lightweight task row (`FfiTaskRow`).
public struct TaskRowData: Equatable, Sendable, Identifiable {
    public var id: String
    public var rev: UInt64
    public var name: String
    public var kind: TaskKind
    public var state: TaskState
    public var domain: String?
    public var progress: ProgressData
    public var queueId: String
    public var categoryId: String?
    public var priority: Priority
    public var position: Int64
    public var createdAt: Int64
    public var completedAt: Int64?
    public var errorKind: String?
    public var errorMessage: String?
    public var statusDetail: String?
    public var health: UInt8
    public var filePath: String?
    public var tags: [String]
    public var blockedBy: [String]
    public var scheduleId: String?
    public var url: String?
    public var directory: String

    public init(id: String, rev: UInt64 = 0, name: String, kind: TaskKind = .http, state: TaskState = .pending,
                domain: String? = nil, progress: ProgressData = ProgressData(), queueId: String = "queue-default",
                categoryId: String? = nil, priority: Priority = .normal, position: Int64 = 0, createdAt: Int64 = 0,
                completedAt: Int64? = nil, errorKind: String? = nil, errorMessage: String? = nil,
                statusDetail: String? = nil, health: UInt8 = 0, filePath: String? = nil, tags: [String] = [],
                blockedBy: [String] = [], scheduleId: String? = nil, url: String? = nil, directory: String = "") {
        self.id = id; self.rev = rev; self.name = name; self.kind = kind; self.state = state
        self.domain = domain; self.progress = progress; self.queueId = queueId; self.categoryId = categoryId
        self.priority = priority; self.position = position; self.createdAt = createdAt
        self.completedAt = completedAt; self.errorKind = errorKind; self.errorMessage = errorMessage
        self.statusDetail = statusDetail; self.health = health; self.filePath = filePath; self.tags = tags
        self.blockedBy = blockedBy; self.scheduleId = scheduleId; self.url = url; self.directory = directory
    }
}

public struct ProgressUpdate: Equatable, Sendable {
    public var taskId: String
    public var rev: UInt64
    public var progress: ProgressData
    public init(taskId: String, rev: UInt64, progress: ProgressData) {
        self.taskId = taskId; self.rev = rev; self.progress = progress
    }
}

public struct QueueData: Equatable, Sendable, Identifiable, Hashable {
    public var id: String
    public var name: String
    public var icon: String
    public var color: String?
    public var maxConcurrent: UInt32
    public var downloadLimit: UInt64
    public var uploadLimit: UInt64
    public var connectionsPerTask: UInt8
    public var scheduleId: String?
    public var directory: String?
    public var priority: Int32
    /// `nothing`, `notify`, `sleep`, `quit_application`, `run_automation:<id>`.
    public var completionAction: String
    public var paused: Bool
    public var builtin: Bool
    public var position: Int32

    public init(id: String = UUID().uuidString.lowercased(), name: String, icon: String = "tray.full", color: String? = nil,
                maxConcurrent: UInt32 = 3, downloadLimit: UInt64 = 0, uploadLimit: UInt64 = 0,
                connectionsPerTask: UInt8 = 0, scheduleId: String? = nil, directory: String? = nil,
                priority: Int32 = 0, completionAction: String = "nothing", paused: Bool = false,
                builtin: Bool = false, position: Int32 = 0) {
        self.id = id; self.name = name; self.icon = icon; self.color = color
        self.maxConcurrent = maxConcurrent; self.downloadLimit = downloadLimit; self.uploadLimit = uploadLimit
        self.connectionsPerTask = connectionsPerTask; self.scheduleId = scheduleId; self.directory = directory
        self.priority = priority; self.completionAction = completionAction; self.paused = paused
        self.builtin = builtin; self.position = position
    }
}

public struct QueueSummary: Equatable, Sendable {
    public var queueId: String
    public var active: UInt32 = 0
    public var waiting: UInt32 = 0
    public var completed: UInt32 = 0
    public var failed: UInt32 = 0
    public var downloadSpeed: UInt64 = 0
    public var uploadSpeed: UInt64 = 0
    public init(queueId: String) { self.queueId = queueId }
}

public struct CategoryData: Equatable, Sendable, Identifiable, Hashable {
    public var id: String
    public var name: String
    public var icon: String
    public var color: String?
    public var extensions: [String]
    public var mimePrefixes: [String]
    public var directory: String?
    public var builtin: Bool
    public var position: Int32

    public init(id: String = UUID().uuidString.lowercased(), name: String, icon: String = "folder", color: String? = nil,
                extensions: [String] = [], mimePrefixes: [String] = [], directory: String? = nil,
                builtin: Bool = false, position: Int32 = 0) {
        self.id = id; self.name = name; self.icon = icon; self.color = color; self.extensions = extensions
        self.mimePrefixes = mimePrefixes; self.directory = directory; self.builtin = builtin; self.position = position
    }
}

public struct GlobalStatsData: Equatable, Sendable {
    public var downloadSpeed: UInt64 = 0
    public var uploadSpeed: UInt64 = 0
    public var active: UInt32 = 0
    public var downloading: UInt32 = 0
    public var seeding: UInt32 = 0
    public var queued: UInt32 = 0
    public var scheduled: UInt32 = 0
    public var paused: UInt32 = 0
    public var completedToday: UInt32 = 0
    public var failedToday: UInt32 = 0
    public var totalTasks: UInt32 = 0
    public var bytesToday: UInt64 = 0
    public var freeSpace: UInt64?
    public var networkAvailable: Bool = true
    public var trafficMode: TrafficMode = .unlimited
    public var downloadLimit: UInt64 = 0
    public var uploadLimit: UInt64 = 0
    public var at: Int64 = 0
    public init() {}
}

public struct Snapshot: Sendable {
    public var rev: UInt64
    public var rows: [TaskRowData]
    public var queues: [QueueData]
    public var queueSummaries: [QueueSummary]
    public var categories: [CategoryData]
    public var stats: GlobalStatsData
    public init(rev: UInt64, rows: [TaskRowData], queues: [QueueData], queueSummaries: [QueueSummary] = [],
                categories: [CategoryData], stats: GlobalStatsData) {
        self.rev = rev; self.rows = rows; self.queues = queues; self.queueSummaries = queueSummaries
        self.categories = categories; self.stats = stats
    }
}

// MARK: - Task detail

public struct SegmentData: Equatable, Sendable, Identifiable {
    public var index: UInt32
    public var start: UInt64
    public var end: UInt64
    public var committed: UInt64
    public var speed: UInt64
    public var state: String
    public var sourceIndex: UInt32
    public var remoteAddr: String?
    public var httpVersion: String?
    public var retries: UInt32
    public var id: UInt32 { index }
    public var length: UInt64 { end > start ? end - start : 0 }
    public var fraction: Double { length == 0 ? 1 : Double(committed.clamped(start, end) - start) / Double(length) }
    public init(index: UInt32, start: UInt64, end: UInt64, committed: UInt64, speed: UInt64 = 0, state: String = "",
                sourceIndex: UInt32 = 0, remoteAddr: String? = nil, httpVersion: String? = nil, retries: UInt32 = 0) {
        self.index = index; self.start = start; self.end = end; self.committed = committed; self.speed = speed
        self.state = state; self.sourceIndex = sourceIndex; self.remoteAddr = remoteAddr
        self.httpVersion = httpVersion; self.retries = retries
    }
}

extension UInt64 {
    func clamped(_ lo: UInt64, _ hi: UInt64) -> UInt64 { Swift.min(Swift.max(self, lo), hi) }
}

public struct TorrentFileData: Equatable, Sendable, Identifiable {
    public var index: UInt32
    public var path: String
    public var size: UInt64
    public var downloaded: UInt64
    public var selected: Bool
    /// 0 low, 1 normal, 2 high.
    public var priority: UInt8
    public var id: UInt32 { index }
    public init(index: UInt32, path: String, size: UInt64, downloaded: UInt64 = 0, selected: Bool = true, priority: UInt8 = 1) {
        self.index = index; self.path = path; self.size = size; self.downloaded = downloaded
        self.selected = selected; self.priority = priority
    }
}

public struct TrackerData: Equatable, Sendable, Identifiable {
    public var url: String
    public var tier: UInt32
    public var enabled: Bool
    public var seeders: UInt32?
    public var leechers: UInt32?
    public var lastAnnounceAt: Int64?
    public var lastError: String?
    public var health: String
    public var id: String { url }
    public init(url: String, tier: UInt32 = 0, enabled: Bool = true, seeders: UInt32? = nil, leechers: UInt32? = nil,
                lastAnnounceAt: Int64? = nil, lastError: String? = nil, health: String = "") {
        self.url = url; self.tier = tier; self.enabled = enabled; self.seeders = seeders; self.leechers = leechers
        self.lastAnnounceAt = lastAnnounceAt; self.lastError = lastError; self.health = health
    }
}

public struct PeerData: Equatable, Sendable, Identifiable {
    public var address: String
    public var client: String?
    public var downloadSpeed: UInt64
    public var uploadSpeed: UInt64
    public var progress: Double
    public var flags: String
    public var id: String { address }
    public init(address: String, client: String?, downloadSpeed: UInt64, uploadSpeed: UInt64, progress: Double, flags: String) {
        self.address = address; self.client = client; self.downloadSpeed = downloadSpeed
        self.uploadSpeed = uploadSpeed; self.progress = progress; self.flags = flags
    }
}

public struct MediaVariantData: Equatable, Sendable, Identifiable, Hashable {
    public var id: String
    public var label: String
    public var url: String
    public var width: UInt32?
    public var height: UInt32?
    public var bandwidth: UInt64?
    public var codecs: String?
    public var estimatedSize: UInt64?
    public var audioOnly: Bool
    public init(id: String, label: String, url: String, width: UInt32? = nil, height: UInt32? = nil,
                bandwidth: UInt64? = nil, codecs: String? = nil, estimatedSize: UInt64? = nil, audioOnly: Bool = false) {
        self.id = id; self.label = label; self.url = url; self.width = width; self.height = height
        self.bandwidth = bandwidth; self.codecs = codecs; self.estimatedSize = estimatedSize; self.audioOnly = audioOnly
    }
    public var resolution: String? {
        guard let w = width, let h = height else { return nil }
        return "\(w)×\(h)"
    }
}

public struct HealthData: Equatable, Sendable {
    public var score: UInt8 = 0
    public var sourceStability: UInt8 = 0
    public var throughputConsistency: UInt8 = 0
    public var connectionQuality: UInt8 = 0
    public var retryPressure: UInt8 = 0
    public var remainingRisk: UInt8 = 0
    /// `health.*` keys.
    public var notes: [String] = []
    public init() {}
}

public struct TaskStatsData: Equatable, Sendable {
    public var averageSpeed: UInt64 = 0
    public var peakSpeed: UInt64 = 0
    public var retries: UInt32 = 0
    public var failedConnections: UInt32 = 0
    public var segmentsReassigned: UInt32 = 0
    public var mirrorsSwitched: UInt32 = 0
    public var activeSeconds: UInt64 = 0
    public var rangeSupported: Bool?
    public var httpVersion: String?
    public var finalUrl: String?
    public var server: String?
    public var contentType: String?
    public var remoteAddr: String?
    public var etag: String?
    public var lastModified: String?
    public init() {}
}

public struct LogEntryData: Equatable, Sendable, Identifiable {
    public var taskId: String
    public var at: Int64
    public var level: LogLevel
    public var code: String
    public var message: String
    public var id: String { "\(taskId)-\(at)-\(code)-\(message.hashValue)" }
    public init(taskId: String, at: Int64, level: LogLevel, code: String, message: String) {
        self.taskId = taskId; self.at = at; self.level = level; self.code = code; self.message = message
    }
}

public struct TaskDetailData: Equatable, Sendable {
    public var row: TaskRowData
    public var urls: [String]
    public var directory: String
    public var origin: String
    public var mime: String?
    public var referer: String?
    public var startedAt: Int64?
    public var updatedAt: Int64?
    public var options: TaskOptionsData
    public var segments: [SegmentData]
    public var torrentFiles: [TorrentFileData]
    public var trackers: [TrackerData]
    public var infoHash: String?
    public var torrentPrivate: Bool
    public var availability: Double
    public var mediaVariants: [MediaVariantData]
    public var selectedVariant: String?
    public var mediaSegmentCount: UInt32?
    public var mediaSegmentsDone: UInt32
    public var stats: TaskStatsData
    public var health: HealthData
    public var log: [LogEntryData]
    public var expectedChecksum: String?
    public var verifiedChecksum: String?
    public var attempt: UInt32
    public var nextRetryAt: Int64?

    public init(row: TaskRowData) {
        self.row = row; urls = []; directory = ""; origin = ""; options = TaskOptionsData(); segments = []
        torrentFiles = []; trackers = []; torrentPrivate = false; availability = 0; mediaVariants = []
        mediaSegmentsDone = 0; stats = TaskStatsData(); health = HealthData(); log = []; attempt = 0
    }
}

/// Editable per-task options (`TaskOptions`). `nil` = inherit.
public struct TaskOptionsData: Equatable, Sendable {
    public var maxConnections: UInt8?
    public var downloadLimit: UInt64?
    public var uploadLimit: UInt64?
    public var headers: [String: String] = [:]
    public var userAgent: String?
    public var referer: String?
    public var cookies: String?
    public var credentialId: String?
    public var proxyId: String?
    public var directConnection: Bool = false
    /// `algo:hex`.
    public var checksum: String?
    public var conflictPolicy: ConflictPolicy = .ask
    public var sequential: Bool?
    public var seedRatioLimit: Double?
    public var mediaVariant: String?
    public var openWhenDone: Bool = false
    public init() {}
}

// MARK: - Add flow

public struct NewTaskRequestData: Equatable, Sendable {
    public var url: String?
    public var mirrors: [String] = []
    public var magnet: String?
    public var torrentBase64: String?
    public var metalinkUrl: String?
    public var hlsPlaylistUrl: String?
    public var name: String?
    public var directory: String?
    public var queueId: String?
    public var categoryId: String?
    public var scheduleId: String?
    public var priority: Priority?
    public var tags: [String] = []
    public var options = TaskOptionsData()
    public var start: Bool = true
    public var origin: String = "app"
    public var selectedFiles: [UInt32]?
    public var refererPage: String?
    public init() {}
}

public struct DuplicateData: Equatable, Sendable {
    /// `path`, `filename`, `size`, `checksum`, `url_history`.
    public var matchedBy: String
    public var existingPath: String?
    public var existingTaskId: String?
    public var existingSize: UInt64?
    public var existingCompletedAt: Int64?
    public init(matchedBy: String, existingPath: String? = nil, existingTaskId: String? = nil,
                existingSize: UInt64? = nil, existingCompletedAt: Int64? = nil) {
        self.matchedBy = matchedBy; self.existingPath = existingPath; self.existingTaskId = existingTaskId
        self.existingSize = existingSize; self.existingCompletedAt = existingCompletedAt
    }
}

public struct ProbeResultData: Equatable, Sendable {
    public var kind: TaskKind
    public var suggestedName: String
    public var suggestedDirectory: String
    public var suggestedQueue: String
    public var suggestedCategory: String?
    public var size: UInt64?
    public var mime: String?
    public var resumable: Bool?
    public var finalUrl: String?
    public var server: String?
    public var freeSpace: UInt64?
    public var duplicate: DuplicateData?
    public var applicableRules: [String]
    public var warnings: [String]
    public var torrentFiles: [TorrentFileData]
    public var torrentName: String?
    public var mediaVariants: [MediaVariantData]
    public var mediaTitle: String?
    public var mediaDuration: Double?
    public init(kind: TaskKind, suggestedName: String, suggestedDirectory: String, suggestedQueue: String) {
        self.kind = kind; self.suggestedName = suggestedName; self.suggestedDirectory = suggestedDirectory
        self.suggestedQueue = suggestedQueue; applicableRules = []; warnings = []; torrentFiles = []; mediaVariants = []
    }
}

public struct AddResultData: Equatable, Sendable {
    public var row: TaskRowData
    public var duplicate: DuplicateData?
    public init(row: TaskRowData, duplicate: DuplicateData?) { self.row = row; self.duplicate = duplicate }
}

public struct DetectedMediaData: Equatable, Sendable {
    public var url: String
    public var kind: String
    public var title: String?
    public var size: UInt64?
    public var variants: [MediaVariantData]
    public var protected: Bool
    public init(url: String, kind: String, title: String?, size: UInt64?, variants: [MediaVariantData], protected: Bool) {
        self.url = url; self.kind = kind; self.title = title; self.size = size; self.variants = variants; self.protected = protected
    }
}

// MARK: - History, dashboard, disks

public struct HistoryEntryData: Equatable, Sendable, Identifiable {
    public var taskId: String
    public var kind: TaskKind
    public var name: String
    public var originalUrl: String
    public var domain: String
    public var size: UInt64?
    public var state: TaskState
    public var destination: String
    public var finishedAt: Int64
    public var durationSeconds: UInt64
    public var averageSpeed: UInt64
    public var error: String?
    public var checksum: String?
    public var id: String { taskId }
    public init(taskId: String, kind: TaskKind, name: String, originalUrl: String, domain: String, size: UInt64?,
                state: TaskState, destination: String, finishedAt: Int64, durationSeconds: UInt64,
                averageSpeed: UInt64, error: String?, checksum: String?) {
        self.taskId = taskId; self.kind = kind; self.name = name; self.originalUrl = originalUrl; self.domain = domain
        self.size = size; self.state = state; self.destination = destination; self.finishedAt = finishedAt
        self.durationSeconds = durationSeconds; self.averageSpeed = averageSpeed; self.error = error; self.checksum = checksum
    }
    // Sort helpers (Table comparators need non-optional keys).
    public var sizeKey: UInt64 { size ?? 0 }
}

public struct HistoryQueryData: Equatable, Sendable {
    public var text: String?
    public var domain: String?
    public var state: TaskState?
    public var kind: TaskKind?
    public var since: Int64?
    /// `finished_at`, `name`, `size`, `domain`, `duration`, `speed`.
    public var sort: String = "finished_at"
    public var descending = true
    public var limit: UInt32 = 500
    public var offset: UInt32 = 0
    public init() {}
}

public struct SpeedSampleData: Equatable, Sendable, Identifiable {
    public var at: Int64
    public var download: UInt64
    public var upload: UInt64
    public var id: Int64 { at }
    public init(at: Int64, download: UInt64, upload: UInt64) { self.at = at; self.download = download; self.upload = upload }
}

public struct DiskInfoData: Equatable, Sendable, Identifiable {
    public var path: String
    public var free: UInt64?
    public var total: UInt64?
    public var reserved: UInt64
    public var requiredByActive: UInt64
    public var volumeAvailable: Bool
    public var id: String { path }
    public init(path: String, free: UInt64?, total: UInt64?, reserved: UInt64 = 0, requiredByActive: UInt64 = 0, volumeAvailable: Bool = true) {
        self.path = path; self.free = free; self.total = total; self.reserved = reserved
        self.requiredByActive = requiredByActive; self.volumeAvailable = volumeAvailable
    }
    public var usedFraction: Double {
        guard let total, total > 0, let free else { return 0 }
        return 1 - Double(free) / Double(total)
    }
}

public struct DashboardData: Equatable, Sendable {
    public var stats: GlobalStatsData
    public var queues: [QueueSummary]
    public var recent: [TaskRowData]
    public var speedHistory: [SpeedSampleData]
    public var disks: [DiskInfoData]
    public var scheduledNext: [(scheduleId: String, at: Int64)]
    public init(stats: GlobalStatsData, queues: [QueueSummary], recent: [TaskRowData], speedHistory: [SpeedSampleData],
                disks: [DiskInfoData], scheduledNext: [(scheduleId: String, at: Int64)]) {
        self.stats = stats; self.queues = queues; self.recent = recent; self.speedHistory = speedHistory
        self.disks = disks; self.scheduledNext = scheduledNext
    }
    public static func == (a: DashboardData, b: DashboardData) -> Bool {
        a.stats == b.stats && a.queues == b.queues && a.recent == b.recent && a.speedHistory == b.speedHistory
            && a.disks == b.disks && a.scheduledNext.map(\.scheduleId) == b.scheduledNext.map(\.scheduleId)
    }
}

// MARK: - Devices

public struct DeviceData: Equatable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var kind: String
    public var scopes: [DeviceScope]
    public var createdAt: Int64
    public var lastSeenAt: Int64?
    public var lastIp: String?
    public var expiresAt: Int64?
    public var revoked: Bool
    public init(id: String, name: String, kind: String, scopes: [DeviceScope], createdAt: Int64, lastSeenAt: Int64?,
                lastIp: String?, expiresAt: Int64?, revoked: Bool) {
        self.id = id; self.name = name; self.kind = kind; self.scopes = scopes; self.createdAt = createdAt
        self.lastSeenAt = lastSeenAt; self.lastIp = lastIp; self.expiresAt = expiresAt; self.revoked = revoked
    }
}

public struct PairingData: Equatable, Sendable {
    public var code: String
    public var expiresAt: Int64
    public var url: String
    public var tlsFingerprint: String?
    public init(code: String, expiresAt: Int64, url: String, tlsFingerprint: String?) {
        self.code = code; self.expiresAt = expiresAt; self.url = url; self.tlsFingerprint = tlsFingerprint
    }
}

public struct AuditEntryData: Equatable, Sendable, Identifiable {
    public var at: Int64
    public var deviceId: String?
    public var ip: String
    public var action: String
    public var target: String?
    public var success: Bool
    public var detail: String?
    public var id: String { "\(at)-\(action)-\(ip)-\(target ?? "")" }
    public init(at: Int64, deviceId: String?, ip: String, action: String, target: String?, success: Bool, detail: String?) {
        self.at = at; self.deviceId = deviceId; self.ip = ip; self.action = action; self.target = target
        self.success = success; self.detail = detail
    }
}

// MARK: - Grabber

public struct GrabberOptionsData: Equatable, Sendable {
    public var url: String = ""
    public var maxDepth: UInt8 = 1
    /// `same_domain`, `subdomains`, `external`.
    public var scope: String = "same_domain"
    public var respectRobots = true
    public var concurrency: UInt8 = 4
    public var maxPages: UInt32 = 200
    public var includeExtensions: [String] = []
    public var excludePatterns: [String] = []
    public var includeRegex: String?
    public var minSize: UInt64?
    public var maxSize: UInt64?
    public var probeFiles = true
    public var followIframes = false
    public init() {}
}

public struct GrabberFileData: Equatable, Sendable, Identifiable, Hashable {
    public var url: String
    public var name: String
    public var ext: String
    public var domain: String
    public var foundOn: String
    public var size: UInt64?
    public var mime: String?
    public var kind: String
    public var depth: UInt8
    public var id: String { url }
    public init(url: String, name: String, ext: String, domain: String, foundOn: String, size: UInt64?, mime: String?, kind: String, depth: UInt8) {
        self.url = url; self.name = name; self.ext = ext; self.domain = domain; self.foundOn = foundOn
        self.size = size; self.mime = mime; self.kind = kind; self.depth = depth
    }
}

public struct GrabberSessionData: Equatable, Sendable, Identifiable {
    public var id: String
    public var options: GrabberOptionsData
    public var pagesCrawled: UInt32
    public var pagesQueued: UInt32
    public var files: [GrabberFileData]
    public var done: Bool
    public var cancelled: Bool
    public var error: String?
    public var startedAt: Int64
    public var finishedAt: Int64?
    public var robotsBlocked: UInt32
    public init(id: String, options: GrabberOptionsData, pagesCrawled: UInt32, pagesQueued: UInt32, files: [GrabberFileData],
                done: Bool, cancelled: Bool, error: String?, startedAt: Int64, finishedAt: Int64?, robotsBlocked: UInt32) {
        self.id = id; self.options = options; self.pagesCrawled = pagesCrawled; self.pagesQueued = pagesQueued
        self.files = files; self.done = done; self.cancelled = cancelled; self.error = error
        self.startedAt = startedAt; self.finishedAt = finishedAt; self.robotsBlocked = robotsBlocked
    }
}

// MARK: - Misc

public struct CredentialData: Equatable, Sendable, Identifiable, Hashable {
    public var id: String
    public var name: String
    public var username: String?
    public init(id: String, name: String, username: String?) { self.id = id; self.name = name; self.username = username }
}

public struct EngineInfoData: Equatable, Sendable {
    public var version: String = ""
    public var build: String = ""
    public var os: String = ""
    public var arch: String = ""
    public var dataDir: String = ""
    public var localApiPort: UInt16 = 0
    public var remoteEnabled = false
    public var remotePort: UInt16?
    public var uptimeSeconds: UInt64 = 0
    public var ffmpegAvailable = false
    public init() {}
}

public struct UpdateInfoData: Equatable, Sendable {
    public var currentVersion: String
    public var available: Bool
    public var latestVersion: String?
    public var notes: String?
    public var signatureValid: Bool?
    public var checkedAt: Int64
    public init(currentVersion: String, available: Bool, latestVersion: String?, notes: String?, signatureValid: Bool?, checkedAt: Int64) {
        self.currentVersion = currentVersion; self.available = available; self.latestVersion = latestVersion
        self.notes = notes; self.signatureValid = signatureValid; self.checkedAt = checkedAt
    }
}

public struct PluginData: Equatable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var version: String
    public var description: String
    public var author: String
    public var permissions: [String]
    public var grantedPermissions: [String]
    public var enabled: Bool
    public init(id: String, name: String, version: String, description: String, author: String,
                permissions: [String], grantedPermissions: [String], enabled: Bool) {
        self.id = id; self.name = name; self.version = version; self.description = description; self.author = author
        self.permissions = permissions; self.grantedPermissions = grantedPermissions; self.enabled = enabled
    }
}

public struct ArchiveEntryData: Equatable, Sendable, Identifiable {
    public var path: String
    public var size: UInt64
    public var isDir: Bool
    public var id: String { path }
    public init(path: String, size: UInt64, isDir: Bool) { self.path = path; self.size = size; self.isDir = isDir }
}

public struct ArchiveListingData: Equatable, Sendable {
    public var format: String
    public var entries: [ArchiveEntryData]
    public var truncated: Bool
    public var intact: Bool?
    public init(format: String, entries: [ArchiveEntryData], truncated: Bool, intact: Bool?) {
        self.format = format; self.entries = entries; self.truncated = truncated; self.intact = intact
    }
}

public struct ImportReportData: Equatable, Sendable {
    public var imported: [String: UInt32]
    public var skipped: [String: UInt32]
    public var errors: [String]
    public init(imported: [String: UInt32], skipped: [String: UInt32], errors: [String]) {
        self.imported = imported; self.skipped = skipped; self.errors = errors
    }
}

/// Engine notifications (`FfiNotification`), already localised-ready.
public struct EngineNotification: Equatable, Sendable {
    /// `completed`, `failed`, `queued`, `scheduled`, `checksum_mismatch`, `low_disk_space`,
    /// `torrent_finished`, `device_paired`, `automation_failed`, `duplicate_detected`,
    /// `update_available`, `queue_finished`.
    public var kind: String
    public var taskId: String?
    public var title: String
    public var body: String
    public var path: String?
    public init(kind: String, taskId: String?, title: String, body: String, path: String?) {
        self.kind = kind; self.taskId = taskId; self.title = title; self.body = body; self.path = path
    }
}

/// Environment measurements pushed to the scheduler.
public struct EnvironmentData: Equatable, Sendable {
    public var networkAvailable = true
    public var metered = false
    public var onAcPower = true
    public var batteryPercent: UInt8?
    public var vpnActive = false
    public var ssid: String?
    public init() {}
}
