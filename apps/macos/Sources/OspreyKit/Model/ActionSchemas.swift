import Foundation

/// Describes the variants of each polymorphic engine type (rule conditions/actions, automation
/// actions, schedule conditions/actions) so one visual builder can edit them all.
public struct FieldSpec: Sendable, Identifiable {
    public enum Kind: Sendable {
        case text, multiline, stringList, bytes, integer, path, queue, category, automation, priority
        case trafficMode, headers, url, toggle
    }
    public var key: String
    public var label: String
    public var kind: Kind
    public var placeholder: String
    public var id: String { key }
    public init(_ key: String, _ label: String, _ kind: Kind, placeholder: String = "") {
        self.key = key; self.label = label; self.kind = kind; self.placeholder = placeholder
    }

    /// Default JSON value for a fresh field.
    public var defaultValue: JSONValue {
        switch kind {
        case .stringList: return .array([])
        case .bytes, .integer: return .number(0)
        case .headers: return .object([:])
        case .priority: return .string("normal")
        case .trafficMode: return .string("balanced")
        case .toggle: return .bool(true)
        default: return .string("")
        }
    }
}

public struct VariantSpec: Sendable, Identifiable {
    public var tag: String
    public var label: String
    public var symbol: String
    public var fields: [FieldSpec]
    /// Code-executing actions need explicit, hash-bound consent.
    public var requiresConsent: Bool
    public var id: String { tag }
    public init(_ tag: String, _ label: String, _ symbol: String, _ fields: [FieldSpec] = [], consent: Bool = false) {
        self.tag = tag; self.label = label; self.symbol = symbol; self.fields = fields; self.requiresConsent = consent
    }
}

public struct VariantFamily: Sendable {
    public var tagKey: String
    public var variants: [VariantSpec]

    public func variant(for value: TaggedValue) -> VariantSpec? {
        let t = value.tag(tagKey)
        return variants.first { $0.tag == t }
    }

    public func make(_ tag: String) -> TaggedValue {
        let spec = variants.first { $0.tag == tag }
        var fields: [String: JSONValue] = [:]
        for f in spec?.fields ?? [] { fields[f.key] = f.defaultValue }
        return TaggedValue(tagKey: tagKey, tag: tag, fields: fields)
    }

    /// One-line human description of a value ("Extension is zip, rar").
    public func summary(_ value: TaggedValue) -> String {
        guard let spec = variant(for: value) else { return value.tag(tagKey) }
        let parts: [String] = spec.fields.compactMap { f in
            guard let v = value[f.key] else { return nil }
            switch f.kind {
            case .stringList: return (v.array ?? []).compactMap(\.string).joined(separator: ", ")
            case .bytes: return Fmt.bytes(UInt64(max(0, v.double ?? 0)))
            case .integer: return v.int.map(String.init)
            case .headers: return nil
            case .toggle: return v.bool == true ? "yes" : "no"
            default:
                let s = v.string ?? ""
                return s.count > 40 ? String(s.prefix(40)) + "…" : s
            }
        }.filter { !$0.isEmpty }
        return parts.isEmpty ? spec.label : "\(spec.label): \(parts.joined(separator: " · "))"
    }
}

public enum ActionSchemas {
    public static let ruleCondition = VariantFamily(tagKey: "field", variants: [
        VariantSpec("extension", "File extension", "doc", [FieldSpec("any_of", "Any of", .stringList, placeholder: "zip, dmg, iso")]),
        VariantSpec("mime", "MIME type", "tag", [FieldSpec("prefix", "Starts with", .text, placeholder: "video/")]),
        VariantSpec("filename", "File name", "textformat", [FieldSpec("glob", "Matches", .text, placeholder: "*.part?.rar")]),
        VariantSpec("url", "URL", "link", [FieldSpec("contains", "Contains", .text, placeholder: "/releases/")]),
        VariantSpec("domain", "Domain", "globe", [FieldSpec("any_of", "Any of", .stringList, placeholder: "example.com")]),
        VariantSpec("regex", "URL regex", "chevron.left.forwardslash.chevron.right", [FieldSpec("pattern", "Pattern", .text, placeholder: "^https://cdn\\.")]),
        VariantSpec("size_greater_than", "Size larger than", "arrow.up.right", [FieldSpec("bytes", "Size", .bytes)]),
        VariantSpec("size_less_than", "Size smaller than", "arrow.down.right", [FieldSpec("bytes", "Size", .bytes)]),
        VariantSpec("origin", "Added from", "arrow.down.app", [FieldSpec("equals", "Origin", .text, placeholder: "browser")]),
        VariantSpec("kind", "Download type", "square.stack.3d.up", [FieldSpec("equals", "Type", .text, placeholder: "torrent")]),
    ])

    public static let ruleAction = VariantFamily(tagKey: "action", variants: [
        VariantSpec("save_to", "Save to folder", "folder", [FieldSpec("directory", "Folder", .path)]),
        VariantSpec("rename", "Rename", "pencil", [FieldSpec("template", "Template", .text, placeholder: "{date}-{name}")]),
        VariantSpec("assign_queue", "Put in queue", "tray.full", [FieldSpec("queue_id", "Queue", .queue)]),
        VariantSpec("assign_category", "Set category", "square.grid.2x2", [FieldSpec("category_id", "Category", .category)]),
        VariantSpec("add_tags", "Add tags", "tag", [FieldSpec("tags", "Tags", .stringList)]),
        VariantSpec("set_priority", "Set priority", "flag", [FieldSpec("priority", "Priority", .priority)]),
        VariantSpec("date_folder", "Date subfolder", "calendar"),
        VariantSpec("domain_folder", "Domain subfolder", "globe"),
        VariantSpec("move_after_completion", "Move when done", "arrow.right.doc.on.clipboard", [FieldSpec("directory", "Folder", .path)]),
        VariantSpec("finder_tags", "Finder tags", "tag.circle", [FieldSpec("tags", "Tags", .stringList, placeholder: "Red, Work")]),
        VariantSpec("reveal_in_finder", "Reveal in Finder", "magnifyingglass"),
        VariantSpec("run_automation", "Run automation", "gearshape.2", [FieldSpec("automation_id", "Automation", .automation)]),
        VariantSpec("set_connection_limit", "Connections", "point.3.filled.connected.trianglepath.dotted", [FieldSpec("connections", "Connections", .integer)]),
        VariantSpec("set_speed_limit", "Speed limit", "speedometer", [FieldSpec("bytes_per_second", "Per second", .bytes)]),
        VariantSpec("stop_processing", "Stop processing rules", "hand.raised"),
    ])

    public static let automationAction = VariantFamily(tagKey: "type", variants: [
        VariantSpec("move", "Move file", "folder", [FieldSpec("directory", "To folder", .path)]),
        VariantSpec("copy", "Copy file", "doc.on.doc", [FieldSpec("directory", "To folder", .path)]),
        VariantSpec("rename", "Rename file", "pencil", [FieldSpec("template", "Template", .text, placeholder: "{stem}-{date}.{ext}")]),
        VariantSpec("open", "Open file", "arrow.up.forward.app"),
        VariantSpec("reveal_in_finder", "Reveal in Finder", "magnifyingglass"),
        VariantSpec("finder_tag", "Finder tags", "tag.circle", [FieldSpec("tags", "Tags", .stringList)]),
        VariantSpec("add_tag", "Add Osprey tags", "tag", [FieldSpec("tags", "Tags", .stringList)]),
        VariantSpec("notify", "Show notification", "bell", [FieldSpec("title", "Title", .text), FieldSpec("body", "Body", .text, placeholder: "{name} is ready")]),
        VariantSpec("webhook", "Call webhook", "network", [FieldSpec("url", "URL", .url, placeholder: "https://"), FieldSpec("headers", "Headers", .headers)]),
        VariantSpec("emit_event", "Emit event", "dot.radiowaves.left.and.right", [FieldSpec("name", "Event name", .text), FieldSpec("payload", "Payload", .headers)]),
        VariantSpec("run_command", "Run program", "terminal", [FieldSpec("program", "Program", .path), FieldSpec("args", "Arguments", .stringList, placeholder: "{file_path}")], consent: true),
        VariantSpec("run_shell", "Run shell script", "apple.terminal", [FieldSpec("script", "Script", .multiline)], consent: true),
        VariantSpec("run_apple_script", "Run AppleScript", "applescript", [FieldSpec("script", "Script", .multiline)], consent: true),
        VariantSpec("run_sandboxed_script", "Run sandboxed script", "curlybraces", [FieldSpec("script", "Script", .multiline), FieldSpec("max_ms", "Time limit (ms)", .integer)], consent: true),
    ])

    public static let scheduleCondition = VariantFamily(tagKey: "type", variants: [
        VariantSpec("network_available", "Network available", "wifi"),
        VariantSpec("not_metered", "Not on a metered network", "antenna.radiowaves.left.and.right.slash"),
        VariantSpec("on_ac_power", "On AC power", "powerplug"),
        VariantSpec("battery_above", "Battery above", "battery.75percent", [FieldSpec("percent", "Percent", .integer)]),
        VariantSpec("bandwidth_below", "Other traffic below", "speedometer", [FieldSpec("bytes_per_second", "Per second", .bytes)]),
        VariantSpec("vpn_active", "VPN connected", "lock.shield", [FieldSpec("active", "Connected", .toggle)]),
        VariantSpec("active_transfers_below", "Active transfers below", "arrow.down.circle", [FieldSpec("count", "Count", .integer)]),
        VariantSpec("network_named", "Wi-Fi network", "wifi.router", [FieldSpec("ssid", "SSID", .text)]),
    ])

    public static let scheduleAction = VariantFamily(tagKey: "type", variants: [
        VariantSpec("start_queue", "Start queue", "play", [FieldSpec("queue_id", "Queue", .queue)]),
        VariantSpec("pause_queue", "Pause queue", "pause", [FieldSpec("queue_id", "Queue", .queue)]),
        VariantSpec("set_speed_limit", "Set speed limit", "speedometer", [FieldSpec("download", "Download", .bytes), FieldSpec("upload", "Upload", .bytes)]),
        VariantSpec("set_connection_limit", "Connections per task", "point.3.filled.connected.trianglepath.dotted", [FieldSpec("per_task", "Per task", .integer)]),
        VariantSpec("set_traffic_mode", "Set speed mode", "gauge.with.dots.needle.50percent", [FieldSpec("mode", "Mode", .trafficMode)]),
        VariantSpec("launch_application", "Launch application", "app", [FieldSpec("path", "Application", .path)]),
        VariantSpec("run_automation", "Run automation", "gearshape.2", [FieldSpec("automation_id", "Automation", .automation)]),
        VariantSpec("notify", "Notify", "bell", [FieldSpec("message", "Message", .text)]),
        VariantSpec("sleep_computer", "Sleep the Mac", "moon.zzz"),
        VariantSpec("quit_application", "Quit Osprey", "power"),
    ])
}
