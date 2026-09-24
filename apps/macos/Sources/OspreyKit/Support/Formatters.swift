import Foundation

/// Deterministic, allocation-light formatting for the numbers the UI shows many times per second.
/// Units are decimal (1 kB = 1000 B) to match Finder.
public enum Fmt {
    private static let units = ["B", "kB", "MB", "GB", "TB", "PB"]

    /// `1_536_000` → `"1.5 MB"`; `nil` → `"—"`.
    public static func bytes(_ value: UInt64?) -> String {
        guard let value else { return "—" }
        return bytes(Double(value))
    }

    public static func bytes(_ value: Double) -> String {
        if value < 1000 { return "\(Int(value)) B" }
        var v = value
        var i = 0
        while v >= 1000, i < units.count - 1 {
            v /= 1000
            i += 1
        }
        // Three significant digits: 9.87 MB, 98.7 MB, 987 MB.
        let s: String
        if v >= 100 { s = String(format: "%.0f", v) }
        else if v >= 10 { s = String(format: "%.1f", v) }
        else { s = String(format: "%.2f", v) }
        return "\(s) \(units[i])"
    }

    /// Bytes per second → `"2.4 MB/s"`; zero renders as an en dash so idle rows stay quiet.
    public static func speed(_ bytesPerSecond: UInt64, zero: String = "–") -> String {
        bytesPerSecond == 0 ? zero : bytes(Double(bytesPerSecond)) + "/s"
    }

    /// Seconds → `"45s"`, `"3m 05s"`, `"2h 07m"`, `"3d 4h"`; nil → `"—"`, `∞` beyond a year.
    public static func eta(_ seconds: UInt64?) -> String {
        guard let s = seconds else { return "—" }
        if s >= 365 * 86_400 { return "∞" }
        if s < 60 { return "\(s)s" }
        if s < 3600 { return String(format: "%dm %02ds", s / 60, s % 60) }
        if s < 86_400 { return String(format: "%dh %02dm", s / 3600, (s % 3600) / 60) }
        return "\(s / 86_400)d \((s % 86_400) / 3600)h"
    }

    /// Duration in seconds for history ("1h 02m 03s").
    public static func duration(_ seconds: UInt64) -> String {
        if seconds < 60 { return "\(seconds)s" }
        if seconds < 3600 { return String(format: "%dm %02ds", seconds / 60, seconds % 60) }
        return String(format: "%dh %02dm %02ds", seconds / 3600, (seconds % 3600) / 60, seconds % 60)
    }

    /// 0…1 → `"42%"`, with one decimal below 10 % so early progress is visible.
    public static func percent(_ fraction: Double) -> String {
        let p = max(0, min(1, fraction)) * 100
        if p > 0, p < 10 { return String(format: "%.1f%%", p) }
        return String(format: "%.0f%%", p.rounded(.down))
    }

    public static func ratio(_ r: Double) -> String { String(format: "%.2f", r) }

    /// Compact count: 1234 → "1.2k".
    public static func count(_ n: Int) -> String {
        if n < 1000 { return "\(n)" }
        if n < 1_000_000 { return String(format: "%.1fk", Double(n) / 1000) }
        return String(format: "%.1fM", Double(n) / 1_000_000)
    }

    public static func date(_ millis: Int64?) -> String {
        guard let millis, millis > 0 else { return "—" }
        return dateFormatter.string(from: Date(millis: millis))
    }

    public static func relative(_ millis: Int64?, now: Date = Date()) -> String {
        guard let millis, millis > 0 else { return "—" }
        return relativeFormatter.localizedString(for: Date(millis: millis), relativeTo: now)
    }

    public static func time(_ millis: Int64?) -> String {
        guard let millis, millis > 0 else { return "—" }
        return timeFormatter.string(from: Date(millis: millis))
    }

    /// Parse user input like "2 MB", "512k", "1.5 G" into bytes. Bare numbers are bytes.
    public static func parseBytes(_ text: String) -> UInt64? {
        let t = text.trimmingCharacters(in: .whitespaces).lowercased()
        guard !t.isEmpty else { return nil }
        let numberPart = t.prefix { $0.isNumber || $0 == "." }
        guard let n = Double(numberPart) else { return nil }
        let unit = t.dropFirst(numberPart.count).trimmingCharacters(in: .whitespaces)
        let mult: Double
        switch unit.first {
        case nil, "b": mult = 1
        case "k": mult = 1e3
        case "m": mult = 1e6
        case "g": mult = 1e9
        case "t": mult = 1e12
        default: return nil
        }
        return UInt64(n * mult)
    }

    private static let dateFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .medium
        f.timeStyle = .short
        return f
    }()

    private static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .none
        f.timeStyle = .short
        return f
    }()

    private static let relativeFormatter: RelativeDateTimeFormatter = {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        return f
    }()
}

public extension Date {
    init(millis: Int64) { self.init(timeIntervalSince1970: TimeInterval(millis) / 1000) }
    var millis: Int64 { Int64((timeIntervalSince1970 * 1000).rounded()) }
}
