import Foundation

// Rules, schedules, automations and recipes cross the FFI as JSON (their shapes are rich and
// evolve with the domain). These Codable types mirror the serde representation exactly; the
// polymorphic parts (conditions / actions / recurrence) are kept as `TaggedValue` objects so the
// visual builder edits them field-by-field without losing unknown keys.

/// A serde internally-tagged enum value: `{"<tagKey>": "variant", ...fields}`.
public struct TaggedValue: Codable, Equatable, Sendable, Identifiable {
    public var fields: [String: JSONValue]
    /// Stable identity for SwiftUI lists (not serialised).
    public var id = UUID()

    public init(tagKey: String, tag: String, fields: [String: JSONValue] = [:]) {
        var f = fields
        f[tagKey] = .string(tag)
        self.fields = f
    }

    public init(from decoder: Decoder) throws {
        fields = try decoder.singleValueContainer().decode([String: JSONValue].self)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(fields)
    }

    public static func == (a: TaggedValue, b: TaggedValue) -> Bool { a.fields == b.fields }

    public func tag(_ key: String) -> String { fields[key]?.string ?? "" }

    public subscript(key: String) -> JSONValue? {
        get { fields[key] }
        set { fields[key] = newValue }
    }
}

public enum MatchMode: String, Codable, CaseIterable, Sendable { case all, any }

public struct RuleDoc: Codable, Equatable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var enabled: Bool
    public var priority: Int32
    public var matchMode: MatchMode
    public var conditions: [TaggedValue]
    public var actions: [TaggedValue]
    public var createdAt: Int64
    public var updatedAt: Int64
    public var hitCount: UInt64

    enum CodingKeys: String, CodingKey {
        case id, name, enabled, priority, conditions, actions
        case matchMode = "match_mode", createdAt = "created_at", updatedAt = "updated_at", hitCount = "hit_count"
    }

    public init(name: String) {
        let now = Date().millis
        id = UUID().uuidString.lowercased(); self.name = name; enabled = true; priority = 100; matchMode = .all
        conditions = []; actions = []; createdAt = now; updatedAt = now; hitCount = 0
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        enabled = try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        priority = try c.decodeIfPresent(Int32.self, forKey: .priority) ?? 100
        matchMode = try c.decodeIfPresent(MatchMode.self, forKey: .matchMode) ?? .all
        conditions = try c.decodeIfPresent([TaggedValue].self, forKey: .conditions) ?? []
        actions = try c.decodeIfPresent([TaggedValue].self, forKey: .actions) ?? []
        createdAt = try c.decodeIfPresent(Int64.self, forKey: .createdAt) ?? 0
        updatedAt = try c.decodeIfPresent(Int64.self, forKey: .updatedAt) ?? 0
        hitCount = try c.decodeIfPresent(UInt64.self, forKey: .hitCount) ?? 0
    }
}

public struct ScheduleDoc: Codable, Equatable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var enabled: Bool
    public var recurrence: TaggedValue
    public var conditions: [TaggedValue]
    public var onStart: [TaggedValue]
    public var onEnd: [TaggedValue]
    public var gateAttached: Bool
    public var lastFiredAt: Int64?
    public var createdAt: Int64
    public var updatedAt: Int64

    enum CodingKeys: String, CodingKey {
        case id, name, enabled, recurrence, conditions
        case onStart = "on_start", onEnd = "on_end", gateAttached = "gate_attached", lastFiredAt = "last_fired_at"
        case createdAt = "created_at", updatedAt = "updated_at"
    }

    public init(name: String) {
        let now = Date().millis
        id = UUID().uuidString.lowercased(); self.name = name; enabled = true
        recurrence = TaggedValue(tagKey: "type", tag: "daily", fields: ["start": .string("01:00"), "end": .string("07:00")])
        conditions = []; onStart = []; onEnd = []; gateAttached = true; createdAt = now; updatedAt = now
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        enabled = try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        recurrence = try c.decodeIfPresent(TaggedValue.self, forKey: .recurrence) ?? TaggedValue(tagKey: "type", tag: "always")
        conditions = try c.decodeIfPresent([TaggedValue].self, forKey: .conditions) ?? []
        onStart = try c.decodeIfPresent([TaggedValue].self, forKey: .onStart) ?? []
        onEnd = try c.decodeIfPresent([TaggedValue].self, forKey: .onEnd) ?? []
        gateAttached = try c.decodeIfPresent(Bool.self, forKey: .gateAttached) ?? true
        lastFiredAt = try c.decodeIfPresent(Int64.self, forKey: .lastFiredAt)
        createdAt = try c.decodeIfPresent(Int64.self, forKey: .createdAt) ?? 0
        updatedAt = try c.decodeIfPresent(Int64.self, forKey: .updatedAt) ?? 0
    }

    /// Human summary of the recurrence ("Daily 01:00–07:00", "Mon, Tue 22:00–06:00").
    public var recurrenceSummary: String {
        let r = recurrence
        let start = r["start"]?.string ?? "", end = r["end"]?.string ?? ""
        switch r.tag("type") {
        case "always": return "Always"
        case "daily": return "Daily \(start)–\(end)"
        case "weekly":
            let names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            let days = (r["days"]?.array ?? []).compactMap(\.int).filter { $0 >= 0 && $0 < 7 }.sorted().map { names[$0] }
            return "\(days.joined(separator: ", ")) \(start)–\(end)"
        case "once": return "Once at \(Fmt.date(r["at"]?.double.map { Int64($0) }))"
        case "range":
            return "\(Fmt.date(r["from"]?.double.map { Int64($0) })) – \(Fmt.date(r["to"]?.double.map { Int64($0) }))"
        default: return r.tag("type")
        }
    }
}

public enum AutomationEventKind: String, CaseIterable, Codable, Sendable {
    case downloadStarted = "download_started", downloadPaused = "download_paused"
    case downloadResumed = "download_resumed", downloadFailed = "download_failed"
    case downloadCompleted = "download_completed", downloadVerified = "download_verified"
    case torrentStarted = "torrent_started", torrentFinished = "torrent_finished"
    case queueFinished = "queue_finished", scheduleFired = "schedule_fired"

    public var label: String {
        switch self {
        case .downloadStarted: return "Download started"
        case .downloadPaused: return "Download paused"
        case .downloadResumed: return "Download resumed"
        case .downloadFailed: return "Download failed"
        case .downloadCompleted: return "Download completed"
        case .downloadVerified: return "Download verified"
        case .torrentStarted: return "Torrent started"
        case .torrentFinished: return "Torrent finished"
        case .queueFinished: return "Queue finished"
        case .scheduleFired: return "Schedule fired"
        }
    }
}

public struct AutomationDoc: Codable, Equatable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var enabled: Bool
    public var events: [AutomationEventKind]
    public var conditions: [TaggedValue]
    public var matchMode: MatchMode
    public var actions: [TaggedValue]
    public var runCount: UInt64
    public var lastRunAt: Int64?
    public var lastError: String?
    public var createdAt: Int64
    public var updatedAt: Int64
    /// Present when the engine reports consent status alongside the rule (`consent_granted`).
    public var consentGranted: Bool?

    enum CodingKeys: String, CodingKey {
        case id, name, enabled, events, conditions, actions
        case matchMode = "match_mode", runCount = "run_count", lastRunAt = "last_run_at", lastError = "last_error"
        case createdAt = "created_at", updatedAt = "updated_at", consentGranted = "consent_granted"
    }

    public init(name: String) {
        let now = Date().millis
        id = UUID().uuidString.lowercased(); self.name = name; enabled = true; events = [.downloadCompleted]
        conditions = []; matchMode = .all; actions = []; runCount = 0; createdAt = now; updatedAt = now
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        enabled = try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        let rawEvents = try c.decodeIfPresent([String].self, forKey: .events) ?? []
        events = rawEvents.compactMap(AutomationEventKind.init(rawValue:))
        conditions = try c.decodeIfPresent([TaggedValue].self, forKey: .conditions) ?? []
        matchMode = try c.decodeIfPresent(MatchMode.self, forKey: .matchMode) ?? .all
        actions = try c.decodeIfPresent([TaggedValue].self, forKey: .actions) ?? []
        runCount = try c.decodeIfPresent(UInt64.self, forKey: .runCount) ?? 0
        lastRunAt = try c.decodeIfPresent(Int64.self, forKey: .lastRunAt)
        lastError = try c.decodeIfPresent(String.self, forKey: .lastError)
        createdAt = try c.decodeIfPresent(Int64.self, forKey: .createdAt) ?? 0
        updatedAt = try c.decodeIfPresent(Int64.self, forKey: .updatedAt) ?? 0
        consentGranted = try c.decodeIfPresent(Bool.self, forKey: .consentGranted)
    }

    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id); try c.encode(name, forKey: .name); try c.encode(enabled, forKey: .enabled)
        try c.encode(events, forKey: .events); try c.encode(conditions, forKey: .conditions)
        try c.encode(matchMode, forKey: .matchMode); try c.encode(actions, forKey: .actions)
        try c.encode(runCount, forKey: .runCount); try c.encodeIfPresent(lastRunAt, forKey: .lastRunAt)
        try c.encodeIfPresent(lastError, forKey: .lastError); try c.encode(createdAt, forKey: .createdAt)
        try c.encode(updatedAt, forKey: .updatedAt)
    }

    /// Whether any action executes code (shell, command, AppleScript, sandboxed script).
    public var executesCode: Bool {
        actions.contains { ActionSchemas.automationAction.variant(for: $0)?.requiresConsent == true }
    }
}

public struct AutomationRunDoc: Codable, Equatable, Sendable, Identifiable {
    public var automationId: String
    public var taskId: String?
    public var event: String
    public var at: Int64
    public var success: Bool
    public var message: String
    public var id: String { "\(automationId)-\(at)-\(taskId ?? "")" }
    enum CodingKeys: String, CodingKey {
        case event, at, success, message
        case automationId = "automation_id", taskId = "task_id"
    }
}

public struct RecipeDoc: Codable, Equatable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var icon: String
    public var queueId: String?
    public var categoryId: String?
    public var directory: String?
    public var options: JSONValue
    public var tags: [String]
    public var ruleActions: [TaggedValue]
    public var automationId: String?
    public var createdAt: Int64
    public var updatedAt: Int64

    enum CodingKeys: String, CodingKey {
        case id, name, icon, directory, options, tags
        case queueId = "queue_id", categoryId = "category_id", ruleActions = "rule_actions"
        case automationId = "automation_id", createdAt = "created_at", updatedAt = "updated_at"
    }

    public init(name: String) {
        let now = Date().millis
        id = UUID().uuidString.lowercased(); self.name = name; icon = "wand.and.stars"; options = .object([:])
        tags = []; ruleActions = []; createdAt = now; updatedAt = now
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        icon = try c.decodeIfPresent(String.self, forKey: .icon) ?? "wand.and.stars"
        queueId = try c.decodeIfPresent(String.self, forKey: .queueId)
        categoryId = try c.decodeIfPresent(String.self, forKey: .categoryId)
        directory = try c.decodeIfPresent(String.self, forKey: .directory)
        options = try c.decodeIfPresent(JSONValue.self, forKey: .options) ?? .object([:])
        tags = try c.decodeIfPresent([String].self, forKey: .tags) ?? []
        ruleActions = try c.decodeIfPresent([TaggedValue].self, forKey: .ruleActions) ?? []
        automationId = try c.decodeIfPresent(String.self, forKey: .automationId)
        createdAt = try c.decodeIfPresent(Int64.self, forKey: .createdAt) ?? 0
        updatedAt = try c.decodeIfPresent(Int64.self, forKey: .updatedAt) ?? 0
    }
}

/// Result of `test_rules_json`: which rules match a subject and what they would do.
public struct RuleTestResult: Equatable, Sendable, Identifiable {
    public var ruleName: String
    public var actions: [TaggedValue]
    public var id: String { ruleName }
    public init(ruleName: String, actions: [TaggedValue]) {
        self.ruleName = ruleName
        self.actions = actions
    }
}

public struct RuleSubjectDoc: Codable, Equatable, Sendable {
    public var name = ""
    public var url = ""
    public var domain = ""
    public var mime: String?
    public var size: UInt64?
    public var origin = "app"
    public var kind = "http"
    public init() {}
}

/// Settings are edited as a lossless JSON document.
public struct SettingsDoc: Equatable, Sendable {
    public var json: JSONValue
    public init(json: JSONValue) { self.json = json }
    public init(text: String) { json = (try? JSONValue(parsing: text)) ?? .object([:]) }

    public subscript(path: String) -> JSONValue? {
        get { json[path: path] }
        set { json[path: path] = newValue }
    }
    public func bool(_ path: String, default d: Bool = false) -> Bool { json[path: path]?.bool ?? d }
    public func int(_ path: String, default d: Int = 0) -> Int { json[path: path]?.int ?? d }
    public func double(_ path: String, default d: Double = 0) -> Double { json[path: path]?.double ?? d }
    public func string(_ path: String, default d: String = "") -> String { json[path: path]?.string ?? d }
    public func strings(_ path: String) -> [String] { (json[path: path]?.array ?? []).compactMap(\.string) }

    public var downloadDirectory: String {
        (string("storage.download_directory", default: "~/Downloads") as NSString).expandingTildeInPath
    }
}
