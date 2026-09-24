#if OSPREY_FFI
import Foundation

// Conversions between the generated UniFFI records (`Ffi*`) and the app's own model types.
// This file and `EngineClient.swift` are the only places that know the generated names.

enum Mapping {
    /// `dnsFailure` → `dns_failure` (UniFFI lowerCamelCases serde's snake_case variants).
    static func snake(_ value: Any) -> String {
        var s = String(describing: value)
        if let paren = s.firstIndex(of: "(") { s = String(s[..<paren]) }
        var out = ""
        for ch in s {
            if ch.isUppercase {
                if !out.isEmpty { out.append("_") }
                out.append(Character(ch.lowercased()))
            } else {
                out.append(ch)
            }
        }
        return out
    }

    // MARK: enums in

    static func state(_ s: FfiTaskState) -> TaskState { TaskState(rawValue: snake(s)) ?? .pending }
    static func kind(_ k: FfiTaskKind) -> TaskKind { TaskKind(rawValue: snake(k)) ?? .http }
    static func priority(_ p: FfiPriority) -> Priority { Priority(rawValue: snake(p)) ?? .normal }
    static func conflict(_ c: FfiConflictPolicy) -> ConflictPolicy { ConflictPolicy(rawValue: snake(c)) ?? .ask }
    static func traffic(_ t: FfiTrafficMode) -> TrafficMode { TrafficMode(rawValue: snake(t)) ?? .unlimited }
    static func level(_ l: FfiLogLevel) -> LogLevel { LogLevel(rawValue: snake(l)) ?? .info }
    static func scope(_ s: FfiScope) -> DeviceScope { DeviceScope(rawValue: snake(s)) ?? .read }

    // MARK: enums out

    static func ffi(_ p: Priority) -> FfiPriority {
        switch p { case .low: return .low; case .normal: return .normal; case .high: return .high; case .urgent: return .urgent }
    }
    static func ffi(_ c: ConflictPolicy) -> FfiConflictPolicy {
        switch c { case .ask: return .ask; case .replace: return .replace; case .rename: return .rename; case .skip: return .skip; case .keepBoth: return .keepBoth }
    }
    static func ffi(_ t: TrafficMode) -> FfiTrafficMode {
        switch t { case .unlimited: return .unlimited; case .fullSpeed: return .fullSpeed; case .balanced: return .balanced; case .browsing: return .browsing; case .custom: return .custom }
    }
    static func ffi(_ s: DeviceScope) -> FfiScope {
        switch s { case .read: return .read; case .add: return .add; case .control: return .control; case .admin: return .admin }
    }
    static func ffi(_ s: TaskState) -> FfiTaskState {
        switch s {
        case .pending: return .pending; case .queued: return .queued; case .scheduled: return .scheduled
        case .resolving: return .resolving; case .connecting: return .connecting; case .downloading: return .downloading
        case .paused: return .paused; case .retrying: return .retrying; case .verifying: return .verifying
        case .processing: return .processing; case .completed: return .completed; case .failed: return .failed
        case .cancelled: return .cancelled; case .seeding: return .seeding
        }
    }
    static func ffi(_ k: TaskKind) -> FfiTaskKind {
        switch k { case .http: return .http; case .ftp: return .ftp; case .torrent: return .torrent; case .magnet: return .magnet; case .metalink: return .metalink; case .hls: return .hls }
    }
    static func ffi(_ a: TaskAction) -> FfiTaskAction {
        switch a {
        case .start: return .start
        case .pause: return .pause
        case .resume: return .resume
        case .restart: return .restart
        case .retry: return .retry
        case .cancel: return .cancel
        case .redownload: return .redownload
        case .verify: return .verify(checksum: nil)
        case .retrySegments: return .retrySegments
        case .duplicate: return .duplicate
        }
    }
    static func historySort(_ s: String) -> FfiHistorySort {
        switch s {
        case "name": return .name
        case "size": return .size
        case "domain": return .domain
        case "duration": return .duration
        case "speed": return .speed
        default: return .finishedAt
        }
    }

    // MARK: records in

    static func row(_ r: FfiTaskRow) -> TaskRowData {
        var p = ProgressData()
        p.downloaded = r.downloaded; p.uploaded = r.uploaded; p.total = r.total; p.speed = r.speed
        p.uploadSpeed = r.uploadSpeed; p.etaSeconds = r.etaSeconds; p.activeConnections = r.activeConnections
        p.peers = r.peers; p.seeds = r.seeds; p.ratio = Double(r.ratio); p.fraction = Double(r.fraction)
        return TaskRowData(id: r.id, rev: r.rev, name: r.name, kind: kind(r.kind), state: state(r.state), domain: r.domain,
                           progress: p, queueId: r.queueId, categoryId: r.categoryId, priority: priority(r.priority),
                           position: r.position, createdAt: r.createdAt, completedAt: r.completedAt,
                           errorKind: r.errorKind.map { snake($0) }, errorMessage: r.errorMessage, statusDetail: r.statusDetail,
                           health: r.health, filePath: r.filePath, tags: r.tags, blockedBy: r.blockedBy,
                           scheduleId: r.scheduleId, url: r.url, directory: r.directory)
    }

    static func progress(_ u: FfiProgress) -> ProgressUpdate {
        var p = ProgressData()
        p.downloaded = u.downloaded; p.uploaded = u.uploaded; p.total = u.total; p.speed = u.speed
        p.uploadSpeed = u.uploadSpeed; p.etaSeconds = u.etaSeconds; p.activeConnections = u.activeConnections
        p.peers = u.peers; p.seeds = u.seeds; p.ratio = Double(u.ratio); p.fraction = Double(u.fraction)
        return ProgressUpdate(taskId: u.taskId, rev: u.rev, progress: p)
    }

    static func queue(_ q: FfiQueue) -> QueueData {
        let action: String
        switch q.completionAction {
        case .nothing: action = "nothing"
        case .notify: action = "notify"
        case .runAutomation(let id): action = "run_automation:\(id)"
        case .sleep: action = "sleep"
        case .quitApplication: action = "quit_application"
        }
        return QueueData(id: q.id, name: q.name, icon: q.icon, color: q.color, maxConcurrent: q.maxConcurrent,
                         downloadLimit: q.downloadLimit, uploadLimit: q.uploadLimit, connectionsPerTask: q.connectionsPerTask,
                         scheduleId: q.scheduleId, directory: q.directory, priority: q.priority, completionAction: action,
                         paused: q.paused, builtin: q.builtin, position: q.position)
    }

    static func summary(_ s: FfiQueueSummary) -> QueueSummary {
        var q = QueueSummary(queueId: s.queueId)
        q.active = s.active; q.waiting = s.waiting; q.completed = s.completed; q.failed = s.failed
        q.downloadSpeed = s.downloadSpeed; q.uploadSpeed = s.uploadSpeed
        return q
    }

    static func category(_ c: FfiCategory) -> CategoryData {
        CategoryData(id: c.id, name: c.name, icon: c.icon, color: c.color, extensions: c.extensions,
                     mimePrefixes: c.mimePrefixes, directory: c.directory, builtin: c.builtin, position: c.position)
    }

    static func stats(_ s: FfiGlobalStats) -> GlobalStatsData {
        var g = GlobalStatsData()
        g.downloadSpeed = s.downloadSpeed; g.uploadSpeed = s.uploadSpeed; g.active = s.active
        g.downloading = s.downloading; g.seeding = s.seeding; g.queued = s.queued; g.scheduled = s.scheduled
        g.paused = s.paused; g.completedToday = s.completedToday; g.failedToday = s.failedToday
        g.totalTasks = s.totalTasks; g.bytesToday = s.bytesToday; g.freeSpace = s.freeSpace
        g.networkAvailable = s.networkAvailable; g.trafficMode = traffic(s.trafficMode)
        g.downloadLimit = s.downloadLimit; g.uploadLimit = s.uploadLimit; g.at = s.at
        return g
    }

    static func log(_ e: FfiLogEntry) -> LogEntryData {
        LogEntryData(taskId: e.taskId, at: e.at, level: level(e.level), code: e.code, message: e.message)
    }

    static func torrentFile(_ f: FfiTorrentFile) -> TorrentFileData {
        TorrentFileData(index: f.index, path: f.path, size: f.size, downloaded: f.downloaded, selected: f.selected, priority: f.priority)
    }

    static func variant(_ v: FfiMediaVariant) -> MediaVariantData {
        MediaVariantData(id: v.id, label: v.label, url: v.url, width: v.width, height: v.height, bandwidth: v.bandwidth,
                         codecs: v.codecs, estimatedSize: v.estimatedSize, audioOnly: v.audioOnly)
    }

    static func options(_ o: FfiTaskOptions) -> TaskOptionsData {
        var t = TaskOptionsData()
        t.maxConnections = o.maxConnections; t.downloadLimit = o.downloadLimit; t.uploadLimit = o.uploadLimit
        t.headers = o.headers; t.userAgent = o.userAgent; t.referer = o.referer; t.cookies = o.cookies
        t.credentialId = o.credentialId; t.proxyId = o.proxyId; t.directConnection = o.directConnection
        t.checksum = o.checksum; t.conflictPolicy = conflict(o.conflictPolicy); t.sequential = o.sequential
        t.seedRatioLimit = o.seedRatioLimit.map(Double.init); t.mediaVariant = o.mediaVariant; t.openWhenDone = o.openWhenDone
        return t
    }

    static func detail(_ d: FfiTaskDetail) -> TaskDetailData {
        var out = TaskDetailData(row: row(d.row))
        out.urls = d.urls
        out.directory = d.row.directory
        out.origin = d.origin
        out.mime = d.mime
        out.startedAt = d.startedAt
        out.updatedAt = d.updatedAt
        out.options = options(d.options)
        out.referer = d.options.referer
        out.segments = d.segments.map { SegmentData(index: $0.index, start: $0.start, end: $0.end, committed: $0.committed, sourceIndex: $0.sourceIndex) }
        if let t = d.torrent {
            out.torrentFiles = t.files.map(torrentFile)
            out.trackers = t.trackers.map {
                TrackerData(url: $0.url, tier: $0.tier, enabled: $0.enabled, seeders: $0.seeders, leechers: $0.leechers,
                            lastAnnounceAt: $0.lastAnnounceAt, lastError: $0.lastError, health: $0.health)
            }
            out.infoHash = t.infoHash
            out.torrentPrivate = t.`private`
            out.availability = Double(t.availability)
        }
        if let m = d.media {
            out.mediaVariants = m.variants.map(variant)
            out.selectedVariant = m.selectedVariant
            out.mediaSegmentCount = m.segmentCount
            out.mediaSegmentsDone = m.segmentsDone
        }
        let s = d.stats
        var st = TaskStatsData()
        st.averageSpeed = s.averageSpeed; st.peakSpeed = s.peakSpeed; st.retries = s.retries
        st.failedConnections = s.failedConnections; st.segmentsReassigned = s.segmentsReassigned
        st.mirrorsSwitched = s.mirrorsSwitched; st.activeSeconds = s.activeSeconds; st.rangeSupported = s.rangeSupported
        st.httpVersion = s.httpVersion; st.finalUrl = s.finalUrl; st.server = s.server; st.contentType = s.contentType
        st.remoteAddr = s.remoteAddr; st.etag = s.etag; st.lastModified = s.lastModified
        out.stats = st
        var h = HealthData()
        h.score = d.health.score; h.sourceStability = d.health.sourceStability
        h.throughputConsistency = d.health.throughputConsistency; h.connectionQuality = d.health.connectionQuality
        h.retryPressure = d.health.retryPressure; h.remainingRisk = d.health.remainingRisk; h.notes = d.health.notes
        out.health = h
        out.log = d.logTail.map(log)
        out.expectedChecksum = d.options.checksum
        out.verifiedChecksum = d.verifiedChecksum
        out.attempt = d.attempt
        out.nextRetryAt = d.nextRetryAt
        if let e = d.error {
            out.row.errorKind = snake(e.kind)
            out.row.errorMessage = e.message
        }
        return out
    }

    static func duplicate(_ d: FfiDuplicateInfo) -> DuplicateData {
        DuplicateData(matchedBy: d.matchedBy, existingPath: d.existingPath, existingTaskId: d.existingTaskId,
                      existingSize: d.existingSize, existingCompletedAt: d.existingCompletedAt)
    }

    static func probe(_ p: FfiProbeResult) -> ProbeResultData {
        var out = ProbeResultData(kind: kind(p.kind), suggestedName: p.suggestedName,
                                  suggestedDirectory: p.suggestedDirectory, suggestedQueue: p.suggestedQueue)
        out.suggestedCategory = p.suggestedCategory
        out.size = p.total ?? p.torrent.map(\.totalSize)
        out.mime = p.mime
        out.resumable = p.resumable
        out.finalUrl = p.finalUrl
        out.server = p.server
        out.freeSpace = p.freeSpace
        out.duplicate = p.duplicate.map(duplicate)
        out.applicableRules = p.applicableRules
        out.warnings = p.warnings
        out.torrentFiles = p.torrent?.files.map(torrentFile) ?? []
        out.torrentName = p.torrent?.name
        out.mediaVariants = p.media?.variants.map(variant) ?? []
        out.mediaTitle = p.media?.title
        out.mediaDuration = p.media?.durationSeconds
        return out
    }

    static func add(_ r: FfiAddTaskResult) -> AddResultData {
        AddResultData(row: row(r.row), duplicate: r.duplicate.map(duplicate))
    }

    static func history(_ h: FfiHistoryEntry) -> HistoryEntryData {
        HistoryEntryData(taskId: h.taskId, kind: kind(h.kind), name: h.name, originalUrl: h.originalUrl, domain: h.domain,
                         size: h.size, state: state(h.state), destination: h.destination, finishedAt: h.finishedAt,
                         durationSeconds: h.durationSeconds, averageSpeed: h.averageSpeed, error: h.error, checksum: h.checksum)
    }

    static func disk(_ d: FfiDiskInfo) -> DiskInfoData {
        DiskInfoData(path: d.path, free: d.free, total: d.total, reserved: d.reserved,
                     requiredByActive: d.requiredByActive, volumeAvailable: d.volumeAvailable)
    }

    static func device(_ d: FfiDevice) -> DeviceData {
        DeviceData(id: d.id, name: d.name, kind: d.kind, scopes: d.scopes.map(scope), createdAt: d.createdAt,
                   lastSeenAt: d.lastSeenAt, lastIp: d.lastIp, expiresAt: d.expiresAt, revoked: d.revoked)
    }

    static func grabberOptions(_ o: FfiGrabberOptions) -> GrabberOptionsData {
        var g = GrabberOptionsData()
        g.url = o.url; g.maxDepth = o.maxDepth; g.scope = o.scope; g.respectRobots = o.respectRobots
        g.concurrency = o.concurrency; g.maxPages = o.maxPages; g.includeExtensions = o.includeExtensions
        g.excludePatterns = o.excludePatterns; g.includeRegex = o.includeRegex; g.minSize = o.minSize
        g.maxSize = o.maxSize; g.probeFiles = o.probeFiles; g.followIframes = o.followIframes
        return g
    }

    static func grabber(_ s: FfiGrabberSession) -> GrabberSessionData {
        GrabberSessionData(id: s.id, options: grabberOptions(s.options), pagesCrawled: s.pagesCrawled, pagesQueued: s.pagesQueued,
                           files: s.files.map {
                               GrabberFileData(url: $0.url, name: $0.name, ext: $0.extension, domain: $0.domain, foundOn: $0.foundOn,
                                               size: $0.size, mime: $0.mime, kind: $0.kind, depth: $0.depth)
                           },
                           done: s.done, cancelled: s.cancelled, error: s.error, startedAt: s.startedAt,
                           finishedAt: s.finishedAt, robotsBlocked: s.robotsBlocked)
    }

    static func notification(_ n: FfiNotification) -> EngineNotification {
        func t(_ s: String) -> String { L10n.tr(s) }
        switch n {
        case .completed(let id, let name, let path):
            return EngineNotification(kind: "completed", taskId: id, title: t("Download complete"), body: name, path: path)
        case .failed(let id, let name, let reason):
            return EngineNotification(kind: "failed", taskId: id, title: t("Download failed"), body: "\(name) — \(reason)", path: nil)
        case .queued(let id, let name):
            return EngineNotification(kind: "queued", taskId: id, title: t("Download queued"), body: name, path: nil)
        case .scheduled(let id, let name, let at):
            return EngineNotification(kind: "scheduled", taskId: id, title: t("Download scheduled"), body: "\(name) · \(Fmt.date(at))", path: nil)
        case .checksumMismatch(let id, let name):
            return EngineNotification(kind: "checksum_mismatch", taskId: id, title: t("Checksum mismatch"), body: name, path: nil)
        case .lowDiskSpace(let path, let free, let required):
            return EngineNotification(kind: "low_disk_space", taskId: nil, title: t("Low disk space"),
                                      body: "\(Fmt.bytes(free)) free, \(Fmt.bytes(required)) needed on \(path)", path: path)
        case .torrentFinished(let id, let name):
            return EngineNotification(kind: "torrent_finished", taskId: id, title: t("Torrent finished"), body: name, path: nil)
        case .devicePaired(_, let name):
            return EngineNotification(kind: "device_paired", taskId: nil, title: t("Device paired"), body: name, path: nil)
        case .automationFailed(_, let name, let error):
            return EngineNotification(kind: "automation_failed", taskId: nil, title: t("Automation failed"), body: "\(name): \(error)", path: nil)
        case .duplicateDetected(let id, let name, let existing):
            return EngineNotification(kind: "duplicate_detected", taskId: id, title: t("Already downloaded"), body: "\(name) — \(existing)", path: existing)
        case .updateAvailable(let version, _):
            return EngineNotification(kind: "update_available", taskId: nil, title: t("Update available"), body: "Osprey \(version)", path: nil)
        case .queueFinished(_, let name):
            return EngineNotification(kind: "queue_finished", taskId: nil, title: t("Queue finished"), body: name, path: nil)
        case .custom(let title, let body, let id):
            return EngineNotification(kind: "custom", taskId: id, title: title, body: body, path: nil)
        }
    }

    static func event(_ e: FfiEvent) -> EngineEvent {
        switch e {
        case .taskAdded(let r): return .taskAdded(row(r))
        case .taskUpdated(let r): return .taskUpdated(row(r))
        case .taskRemoved(let id, _): return .taskRemoved(id: id)
        case .taskStateChanged(let id, let from, let to, _): return .taskStateChanged(id: id, from: state(from), to: state(to))
        case .progress(let updates): return .progress(updates.map(progress))
        case .taskLog(let entry): return .taskLog(log(entry))
        case .queueUpdated(let q): return .queueUpdated(queue(q))
        case .queueRemoved(let id): return .queueRemoved(id: id)
        case .queueSummaries(let s): return .queueSummaries(s.map(summary))
        case .categoriesChanged: return .categoriesChanged
        case .rulesChanged: return .rulesChanged
        case .schedulesChanged: return .schedulesChanged
        case .automationsChanged: return .automationsChanged
        case .automationRan(let aid, let tid, let success, let message):
            return .automationRan(automationId: aid, taskId: tid, success: success, message: message)
        case .recipesChanged: return .recipesChanged
        case .settingsChanged: return .settingsChanged
        case .globalStats(let s): return .globalStats(stats(s))
        case .notification(let n): return .notification(notification(n))
        case .devicesChanged: return .devicesChanged
        case .pairingStarted(let code, let expiresAt, let url): return .pairingStarted(code: code, expiresAt: expiresAt, url: url)
        case .pairingCompleted(let name): return .pairingCompleted(deviceName: name)
        case .diskSpace(let path, let free): return .diskSpace(path: path, free: free)
        case .networkChanged(let available, let metered): return .networkChanged(available: available, metered: metered)
        case .platformAction(let tid, let action, let context): return .platformAction(taskId: tid, actionJSON: action, contextJSON: context)
        case .grabberProgress(let sid, let pages, let files, let done): return .grabberProgress(sessionId: sid, pages: pages, files: files, done: done)
        case .updateCheck(let available, let version, let notes): return .updateCheck(available: available, version: version, notes: notes)
        case .readyForSleep(let reason): return .readyForSleep(reason: reason)
        case .custom(let name, let payload): return .custom(name: name, payloadJSON: payload)
        case .resync: return .resync
        case .engineStopping: return .engineStopping
        }
    }

    // MARK: records out

    static func ffi(_ o: TaskOptionsData) -> FfiTaskOptions {
        var f = FfiTaskOptions(headers: o.headers, conflictPolicy: ffi(o.conflictPolicy))
        f.maxConnections = o.maxConnections; f.downloadLimit = o.downloadLimit; f.uploadLimit = o.uploadLimit
        f.userAgent = o.userAgent; f.referer = o.referer; f.cookies = o.cookies; f.credentialId = o.credentialId
        f.proxyId = o.proxyId; f.directConnection = o.directConnection; f.checksum = o.checksum
        f.sequential = o.sequential; f.seedRatioLimit = o.seedRatioLimit.map(Float.init)
        f.mediaVariant = o.mediaVariant; f.openWhenDone = o.openWhenDone
        return f
    }

    static func ffi(_ r: NewTaskRequestData) -> FfiNewTaskRequest {
        var f = FfiNewTaskRequest()
        f.url = r.url; f.mirrors = r.mirrors; f.magnet = r.magnet; f.torrentBase64 = r.torrentBase64
        f.metalinkUrl = r.metalinkUrl; f.hlsPlaylistUrl = r.hlsPlaylistUrl; f.name = r.name; f.directory = r.directory
        f.queueId = r.queueId; f.categoryId = r.categoryId; f.scheduleId = r.scheduleId; f.priority = r.priority.map(ffi)
        f.tags = r.tags; f.options = ffi(r.options); f.start = r.start; f.origin = r.origin
        f.selectedFiles = r.selectedFiles; f.refererPage = r.refererPage
        return f
    }

    static func ffi(_ p: TaskPatchData) -> FfiTaskPatch {
        var f = FfiTaskPatch()
        f.name = p.name; f.directory = p.directory; f.queueId = p.queueId
        if let c = p.categoryId { if let c { f.categoryId = c } else { f.clearCategory = true } }
        if let s = p.scheduleId { if let s { f.scheduleId = s } else { f.clearSchedule = true } }
        f.priority = p.priority.map(ffi); f.tags = p.tags; f.options = p.options.map(ffi); f.mirrors = p.mirrors
        return f
    }

    static func ffi(_ q: QueueData) -> FfiQueue {
        let action: FfiQueueCompletionAction
        switch q.completionAction {
        case "notify": action = .notify
        case "sleep": action = .sleep
        case "quit_application": action = .quitApplication
        case let s where s.hasPrefix("run_automation:"): action = .runAutomation(automationId: String(s.dropFirst("run_automation:".count)))
        default: action = .nothing
        }
        let now = Date().millis
        return FfiQueue(id: q.id, name: q.name, icon: q.icon, color: q.color, maxConcurrent: q.maxConcurrent,
                        downloadLimit: q.downloadLimit, uploadLimit: q.uploadLimit, connectionsPerTask: q.connectionsPerTask,
                        scheduleId: q.scheduleId, directory: q.directory, priority: q.priority, completionAction: action,
                        paused: q.paused, builtin: q.builtin, position: q.position, createdAt: now, updatedAt: now)
    }

    static func ffi(_ c: CategoryData) -> FfiCategory {
        let now = Date().millis
        return FfiCategory(id: c.id, name: c.name, icon: c.icon, color: c.color, extensions: c.extensions,
                           mimePrefixes: c.mimePrefixes, directory: c.directory, builtin: c.builtin, position: c.position,
                           createdAt: now, updatedAt: now)
    }

    static func ffi(_ q: HistoryQueryData) -> FfiHistoryQuery {
        var f = FfiHistoryQuery()
        f.text = q.text; f.domain = q.domain; f.state = q.state.map(ffi); f.kind = q.kind.map(ffi); f.since = q.since
        f.sort = historySort(q.sort); f.descending = q.descending; f.limit = q.limit; f.offset = q.offset
        return f
    }

    static func ffi(_ o: GrabberOptionsData) -> FfiGrabberOptions {
        var f = FfiGrabberOptions(url: o.url)
        f.maxDepth = o.maxDepth; f.scope = o.scope; f.respectRobots = o.respectRobots; f.concurrency = o.concurrency
        f.maxPages = o.maxPages; f.includeExtensions = o.includeExtensions; f.excludePatterns = o.excludePatterns
        f.includeRegex = o.includeRegex; f.minSize = o.minSize; f.maxSize = o.maxSize; f.probeFiles = o.probeFiles
        f.followIframes = o.followIframes
        return f
    }

    static func ffi(_ o: ImportOptionsData) -> FfiImportOptions {
        FfiImportOptions(settings: o.settings, queues: o.queues, categories: o.categories, rules: o.rules,
                         schedules: o.schedules, automations: o.automations, tasks: o.tasks, history: o.history,
                         recipes: o.recipes, overwrite: o.overwrite)
    }

    static func ffi(_ e: EnvironmentData) -> FfiEnvironment {
        FfiEnvironment(networkAvailable: e.networkAvailable, metered: e.metered, onAcPower: e.onAcPower,
                       batteryPercent: e.batteryPercent, vpnActive: e.vpnActive, ssid: e.ssid)
    }

    static func error(_ e: Error) -> EngineError {
        guard let f = e as? FfiError else { return EngineError(.internalError, e.localizedDescription) }
        switch f {
        case .NotFound(let m): return EngineError(.notFound, m)
        case .Validation(let m): return EngineError(.validation, m)
        case .Conflict(let m): return EngineError(.conflict, m)
        case .PermissionDenied(let m): return EngineError(.permissionDenied, m)
        case .Storage(let m): return EngineError(.storage, m)
        case .Engine(let m): return EngineError(.engine, m)
        case .Unavailable(let m): return EngineError(.unavailable, m)
        case .Internal(let m): return EngineError(.internalError, m)
        }
    }
}
#endif
