import Foundation

// Swift mirrors of the engine enums. Raw values equal the serde snake_case names, so the same
// types decode the engine's JSON documents and map from the UniFFI enums.

public enum TaskState: String, CaseIterable, Codable, Sendable {
    case pending, queued, scheduled, resolving, connecting, downloading, paused, retrying
    case verifying, processing, completed, failed, cancelled, seeding

    /// Holds a live transfer (mirrors `TaskState::is_active`).
    public var isActive: Bool {
        switch self {
        case .resolving, .connecting, .downloading, .retrying, .verifying, .processing, .seeding: return true
        default: return false
        }
    }
    public var isTerminal: Bool { self == .completed || self == .failed || self == .cancelled }
    public var isWaiting: Bool { self == .pending || self == .queued || self == .scheduled }
    public var canPause: Bool {
        switch self {
        case .queued, .scheduled, .resolving, .connecting, .downloading, .retrying, .seeding: return true
        default: return false
        }
    }
    public var canResume: Bool {
        switch self {
        case .paused, .failed, .cancelled, .pending: return true
        default: return false
        }
    }
    public var labelKey: String { "state.\(rawValue)" }
}

public enum TaskKind: String, CaseIterable, Codable, Sendable {
    case http, ftp, torrent, magnet, metalink, hls
    public var isTorrent: Bool { self == .torrent || self == .magnet }
    public var symbol: String {
        switch self {
        case .http: return "arrow.down.circle"
        case .ftp: return "server.rack"
        case .torrent, .magnet: return "point.3.connected.trianglepath.dotted"
        case .metalink: return "link.circle"
        case .hls: return "play.rectangle"
        }
    }
    public var label: String {
        switch self {
        case .http: return "HTTP"
        case .ftp: return "FTP"
        case .torrent: return "Torrent"
        case .magnet: return "Magnet"
        case .metalink: return "Metalink"
        case .hls: return "HLS"
        }
    }
}

public enum Priority: String, CaseIterable, Codable, Sendable, Comparable {
    case low, normal, high, urgent
    public var rank: Int { Self.allCases.firstIndex(of: self) ?? 1 }
    public static func < (a: Priority, b: Priority) -> Bool { a.rank < b.rank }
    public var label: String { rawValue.capitalized }
}

public enum ConflictPolicy: String, CaseIterable, Codable, Sendable {
    case ask, replace, rename, skip, keepBoth = "keep_both"
}

public enum TrafficMode: String, CaseIterable, Codable, Sendable {
    case unlimited, fullSpeed = "full_speed", balanced, browsing, custom
    public var label: String {
        switch self {
        case .unlimited: return "Unlimited"
        case .fullSpeed: return "Full speed"
        case .balanced: return "Balanced"
        case .browsing: return "Quiet"
        case .custom: return "Custom"
        }
    }
    public var symbol: String {
        switch self {
        case .unlimited: return "infinity"
        case .fullSpeed: return "hare"
        case .balanced: return "gauge.with.dots.needle.50percent"
        case .browsing: return "leaf"
        case .custom: return "slider.horizontal.3"
        }
    }
    /// The four modes the speed picker offers (full speed is an engine alias of unlimited).
    public static let pickerModes: [TrafficMode] = [.unlimited, .balanced, .browsing, .custom]
}

public enum ChecksumAlgorithm: String, CaseIterable, Codable, Sendable {
    case md5, sha1, sha256, sha512, blake3
    public var label: String {
        switch self {
        case .md5: return "MD5"
        case .sha1: return "SHA-1"
        case .sha256: return "SHA-256"
        case .sha512: return "SHA-512"
        case .blake3: return "BLAKE3"
        }
    }
    public var hexLength: Int {
        switch self {
        case .md5: return 32
        case .sha1: return 40
        case .sha256, .blake3: return 64
        case .sha512: return 128
        }
    }
}

public enum LogLevel: String, CaseIterable, Codable, Sendable {
    case trace, debug, info, warn, error
}

public enum DeviceScope: String, CaseIterable, Codable, Sendable {
    case read, add, control, admin
    public var label: String { rawValue.capitalized }
}

public enum TaskAction: String, CaseIterable, Sendable {
    case start, pause, resume, restart, retry, cancel, redownload, verify, retrySegments, duplicate
    public var label: String {
        switch self {
        case .start: return "Start"
        case .pause: return "Pause"
        case .resume: return "Resume"
        case .restart: return "Restart from Scratch"
        case .retry: return "Retry"
        case .cancel: return "Cancel"
        case .redownload: return "Download Again"
        case .verify: return "Verify Checksum"
        case .retrySegments: return "Retry Failed Segments"
        case .duplicate: return "Duplicate"
        }
    }
    public var symbol: String {
        switch self {
        case .start: return "play.fill"
        case .pause: return "pause.fill"
        case .resume: return "play.fill"
        case .restart: return "arrow.counterclockwise"
        case .retry: return "arrow.clockwise"
        case .cancel: return "xmark.circle"
        case .redownload: return "arrow.down.circle"
        case .verify: return "checkmark.seal"
        case .retrySegments: return "square.split.2x1"
        case .duplicate: return "plus.square.on.square"
        }
    }
}

/// Engine error categories (the `FfiError` cases).
public enum EngineErrorKind: String, Sendable {
    case notFound, validation, conflict, permissionDenied, storage, engine, unavailable, internalError
}

public struct EngineError: Error, LocalizedError, Sendable, Equatable {
    public var kind: EngineErrorKind
    public var message: String
    public init(_ kind: EngineErrorKind, _ message: String) {
        self.kind = kind
        self.message = message
    }
    public var errorDescription: String? { message }

    public static func unavailable(_ why: String = "The Osprey engine is not available.") -> EngineError {
        EngineError(.unavailable, why)
    }
}
