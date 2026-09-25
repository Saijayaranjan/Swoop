import Foundation
import Observation
import SwoopKit
import SwiftUI

/// Places in the main window. Configuration (rules, automation, categories, recipes, queues,
/// devices) lives in the Settings window.
enum SidebarItem: Hashable, Codable {
    case dashboard, downloads, completed, torrents, scheduled
    case history, grabber
    case queue(String)

    var title: String {
        switch self {
        case .dashboard: return "Dashboard"
        case .downloads: return "Downloads"
        case .completed: return "Completed"
        case .torrents: return "Torrents"
        case .scheduled: return "Scheduled"
        case .history: return "History"
        case .grabber: return "Site Grabber"
        case .queue: return "Queue"
        }
    }

    var symbol: String {
        switch self {
        case .dashboard: return "square.grid.2x2"
        case .downloads: return "arrow.down.circle"
        case .completed: return "checkmark.circle"
        case .torrents: return "point.3.connected.trianglepath.dotted"
        case .scheduled: return "calendar.badge.clock"
        case .history: return "clock.arrow.circlepath"
        case .grabber: return "globe.desk"
        case .queue: return "tray"
        }
    }

    /// ⌘1…⌘6 order.
    static let shortcutOrder: [SidebarItem] = [.dashboard, .downloads, .torrents, .scheduled, .history, .grabber]
}

enum SmartFilter: String, CaseIterable, Identifiable {
    case all, active, queued, paused, completed, failed, media
    var id: String { rawValue }
    var title: String {
        switch self {
        case .all: return "All"
        case .active: return "Active"
        case .queued: return "Waiting"
        case .paused: return "Paused"
        case .completed: return "Completed"
        case .failed: return "Failed"
        case .media: return "Media"
        }
    }
    @MainActor
    func matches(_ t: TaskItem) -> Bool {
        switch self {
        case .all: return true
        case .active: return t.state.isActive
        case .queued: return t.state == .queued || t.state == .pending || t.state == .scheduled
        case .paused: return t.state == .paused
        case .completed: return t.state == .completed || t.state == .seeding
        case .failed: return t.state == .failed || t.state == .cancelled
        case .media: return t.kind == .hls || Self.mediaExtensions.contains((t.name as NSString).pathExtension.lowercased())
        }
    }
    static let mediaExtensions: Set<String> = ["mp4", "mkv", "mov", "webm", "m4v", "avi", "mp3", "m4a", "flac", "wav", "aac", "ogg", "opus", "m3u8", "ts"]
}

/// Search tokens (queue, category, domain, date, size) shown in the toolbar search field.
struct FilterToken: Identifiable, Hashable {
    enum Kind: String { case queue, category, domain, tag, date, size }
    var kind: Kind
    var value: String
    var label: String
    var id: String { "\(kind.rawValue):\(value)" }
    var symbol: String {
        switch kind {
        case .queue: return "tray"
        case .category: return "square.grid.2x2"
        case .domain: return "globe"
        case .tag: return "tag"
        case .date: return "calendar"
        case .size: return "externaldrive"
        }
    }
}

/// Everything the Add sheet can be opened with.
struct AddPrefill: Identifiable, Equatable {
    let id = UUID()
    var text: String = ""
    var torrentFile: URL?
    var metalinkFile: URL?
    var refererPage: String?
    var autoPaste = false
}

@Observable
@MainActor
final class UIState {
    var sidebar: SidebarItem = .downloads
    var selection: Set<String> = []
    var showInspector = false
    var searchText = ""
    var tokens: [FilterToken] = []
    var smartFilter: SmartFilter = .all
    var addRequest: AddPrefill?
    var removeConfirmation: [String]?
    var editingQueue: QueueData?
    var windowActive = true
    var showActivity = false
    var showNotifications = false

    func openAdd(_ prefill: AddPrefill = AddPrefill()) {
        addRequest = prefill
    }

    func select(_ id: String) {
        sidebar = .downloads
        smartFilter = .all
        selection = [id]
        showInspector = true
    }
}
