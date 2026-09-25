import Foundation

/// Localisation. UI strings use English text as the key (SwiftUI `Text("…")` looks them up in
/// `Localizable.strings` automatically); engine keys (`state.*`, `error.*`, `health.*`,
/// `pause.*`) are mapped here so every front end shows the same wording.
public enum L10n {
    /// Looks a key up in the app bundle; falls back to the English default (never shows raw keys).
    public static func string(_ key: String, default fallback: String) -> String {
        let value = Bundle.main.localizedString(forKey: key, value: "\u{0}", table: nil)
        return value == "\u{0}" || value == key ? fallback : value
    }

    public static func tr(_ english: String) -> String { string(english, default: english) }

    public static func state(_ s: TaskState) -> String { string(s.labelKey, default: stateDefaults[s] ?? s.rawValue.capitalized) }

    /// `error.dns` → "The server name could not be resolved."; also accepts the snake_case kind.
    public static func error(_ keyOrKind: String) -> String {
        let key = keyOrKind.hasPrefix("error.") ? keyOrKind : (errorKindToKey[keyOrKind] ?? "error.\(keyOrKind)")
        return string(key, default: errorDefaults[key] ?? "Something went wrong.")
    }

    public static func health(_ key: String) -> String { string(key, default: healthDefaults[key] ?? key) }

    public static func healthLabel(_ score: UInt8) -> String {
        switch score {
        case 85...100: return health("health.excellent")
        case 65..<85: return health("health.good")
        case 40..<65: return health("health.fair")
        default: return health("health.poor")
        }
    }

    /// `pause.user` or a serialised reason (`user`, `schedule:night`, `{"reason":"queue",...}`).
    public static func pause(_ raw: String) -> String {
        var key = raw
        if !key.hasPrefix("pause.") {
            let base = raw.split(separator: ":").first.map(String.init) ?? raw
            key = pauseReasonToKey[base] ?? "pause.\(base)"
        }
        return string(key, default: pauseDefaults[key] ?? raw)
    }

    public static let stateDefaults: [TaskState: String] = [
        .pending: "Waiting", .queued: "Queued", .scheduled: "Scheduled", .resolving: "Resolving",
        .connecting: "Connecting", .downloading: "Downloading", .paused: "Paused", .retrying: "Retrying",
        .verifying: "Verifying", .processing: "Processing", .completed: "Completed", .failed: "Failed",
        .cancelled: "Cancelled", .seeding: "Seeding",
    ]

    static let errorKindToKey: [String: String] = [
        "invalid_url": "error.invalid_url", "unsupported_scheme": "error.unsupported_scheme",
        "dns_failure": "error.dns", "tls_failure": "error.tls", "certificate_invalid": "error.certificate",
        "connection_refused": "error.refused", "connection_timeout": "error.connect_timeout",
        "connection_reset": "error.reset", "read_timeout": "error.read_timeout", "proxy_error": "error.proxy",
        "authentication_required": "error.auth_required", "forbidden": "error.forbidden",
        "not_found": "error.not_found", "throttled": "error.throttled", "server_error": "error.server",
        "range_not_supported": "error.range_not_supported", "source_changed": "error.source_changed",
        "expired_url": "error.expired_url", "redirect_loop": "error.redirect_loop", "truncated": "error.truncated",
        "checksum_mismatch": "error.checksum", "disk_full": "error.disk_full", "disk_write_error": "error.disk_write",
        "disk_read_error": "error.disk_read", "permission_denied": "error.permission",
        "volume_unavailable": "error.volume", "invalid_filename": "error.filename",
        "path_traversal": "error.path_traversal", "file_exists": "error.file_exists",
        "invalid_torrent": "error.invalid_torrent", "tracker_failure": "error.tracker", "no_peers": "error.no_peers",
        "dht_unavailable": "error.dht", "parse_error": "error.parse", "protected_content": "error.protected",
        "mirror_exhausted": "error.mirrors_exhausted", "network_unavailable": "error.network_unavailable",
        "unexpected_content": "error.unexpected_content", "live_stream_unsupported": "error.live_stream",
        "quota_exceeded": "error.quota", "cancelled": "error.cancelled", "internal": "error.internal",
        "unknown": "error.unknown",
    ]

    public static let errorDefaults: [String: String] = [
        "error.invalid_url": "The link is not a valid URL.",
        "error.unsupported_scheme": "This kind of link is not supported.",
        "error.dns": "The server name could not be found. Check your connection or the address.",
        "error.tls": "A secure connection could not be established.",
        "error.certificate": "The server's certificate is not trusted.",
        "error.refused": "The server refused the connection.",
        "error.connect_timeout": "The server took too long to respond.",
        "error.reset": "The connection was interrupted.",
        "error.read_timeout": "The server stopped sending data.",
        "error.proxy": "The proxy server reported an error.",
        "error.auth_required": "The server needs a user name and password.",
        "error.forbidden": "The server denied access to this file.",
        "error.not_found": "The file no longer exists on the server.",
        "error.throttled": "The server is limiting requests. Swoop will slow down and retry.",
        "error.server": "The server had a problem. Swoop will retry.",
        "error.range_not_supported": "The server can't resume; the download will use one connection.",
        "error.source_changed": "The file changed on the server, so the download restarted.",
        "error.expired_url": "The link has expired. Get a fresh link and retry from source.",
        "error.redirect_loop": "The server redirected too many times.",
        "error.truncated": "The server sent less data than expected.",
        "error.checksum": "The file does not match its checksum.",
        "error.disk_full": "There isn't enough free space on the destination disk.",
        "error.disk_write": "Swoop couldn't write to the destination.",
        "error.disk_read": "Swoop couldn't read the partial file.",
        "error.permission": "Swoop doesn't have permission to save here.",
        "error.volume": "The destination disk is not connected.",
        "error.filename": "The file name is not valid on this Mac.",
        "error.path_traversal": "The file name tried to escape the download folder and was blocked.",
        "error.file_exists": "A file with this name already exists.",
        "error.invalid_torrent": "The torrent file is damaged or invalid.",
        "error.tracker": "The trackers could not be reached.",
        "error.no_peers": "No peers are sharing this torrent right now.",
        "error.dht": "The distributed hash table is unavailable.",
        "error.parse": "The response could not be understood.",
        "error.protected": "This media is DRM-protected and can't be downloaded.",
        "error.mirrors_exhausted": "Every mirror failed.",
        "error.network_unavailable": "You're offline. Swoop will continue when the network returns.",
        "error.unexpected_content": "The server returned a web page instead of the file.",
        "error.live_stream": "Live streams can't be downloaded.",
        "error.quota": "The server's download quota was reached.",
        "error.cancelled": "The download was cancelled.",
        "error.internal": "Swoop hit an internal error.",
        "error.unknown": "Something went wrong.",
    ]

    public static let healthDefaults: [String: String] = [
        "health.excellent": "Excellent", "health.good": "Good", "health.fair": "Fair", "health.poor": "Poor",
        "health.connections_failing": "Connections are failing",
        "health.many_retries": "Many retries",
        "health.throttled": "The server is throttling",
        "health.no_range": "The server can't resume",
    ]

    static let pauseReasonToKey: [String: String] = [
        "user": "pause.user", "queue": "pause.queue", "schedule": "pause.schedule", "condition": "pause.condition",
        "shutdown": "pause.shutdown", "disk_space": "pause.disk_space", "network_unavailable": "pause.network",
        "network": "pause.network", "volume_unavailable": "pause.volume", "volume": "pause.volume",
    ]

    public static let pauseDefaults: [String: String] = [
        "pause.user": "Paused by you", "pause.queue": "Queue paused", "pause.schedule": "Waiting for schedule",
        "pause.condition": "Waiting for conditions", "pause.shutdown": "Paused at quit",
        "pause.disk_space": "Waiting for disk space", "pause.network": "Waiting for network",
        "pause.volume": "Waiting for disk",
    ]
}
