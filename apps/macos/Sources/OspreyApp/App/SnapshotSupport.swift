import AppKit
import OspreyKit
import SwiftUI

/// Developer aid for headless visual checks (no Screen Recording permission needed): when
/// `OSPREY_SNAPSHOT` is set to a path prefix, Osprey renders its main window into PNGs for each
/// screen listed in `OSPREY_SNAPSHOT_SCREENS` (comma-separated: `downloads`, `downloads-empty`,
/// `dashboard`, `inspector`, `add`, `torrents`, `scheduled`, `history`, `grabber`, `queue`,
/// `settings`, `menubar`; default `downloads,dashboard`) and `OSPREY_SNAPSHOT_APPEARANCE` (`light`, `dark` or
/// both); `OSPREY_SNAPSHOT_SIZE=WxH` sets the main window size first. With `OSPREY_SNAPSHOT_SAMPLE=1` the in-memory model is filled with sample downloads,
/// live stats and activity so busy states can be checked; the engine's stored data is untouched.
/// Inert otherwise.
@MainActor
enum SnapshotSupport {
    static func runIfRequested(ui: UIState, model: AppModel, delegate: AppDelegate) {
        let env = ProcessInfo.processInfo.environment
        guard let prefix = env["OSPREY_SNAPSHOT"], !prefix.isEmpty else { return }
        let screens = (env["OSPREY_SNAPSHOT_SCREENS"] ?? "downloads,dashboard").split(separator: ",").map(String.init)
        let appearances = (env["OSPREY_SNAPSHOT_APPEARANCE"] ?? "light,dark").split(separator: ",").map(String.init)
        let sample = env["OSPREY_SNAPSHOT_SAMPLE"] == "1"
        Task { @MainActor in
            NSApp.activate(ignoringOtherApps: true)
            let main = NSApp.windows.first { $0.frame.height > 300 }
            main?.makeKeyAndOrderFront(nil)
            // OSPREY_SNAPSHOT_SIZE=WxH resizes the main window first (e.g. 1280x800).
            if let size = env["OSPREY_SNAPSHOT_SIZE"]?.split(separator: "x").compactMap({ Double($0) }), size.count == 2, let main {
                main.setFrame(NSRect(x: main.frame.minX, y: main.frame.maxY - size[1], width: size[0], height: size[1]), display: true)
            }
            try? await Task.sleep(nanoseconds: 2_500_000_000)
            if sample {
                SnapshotSample.install(into: model)
                // Keep the sample figures on screen while the live engine keeps reporting.
                Task { @MainActor in
                    while true {
                        SnapshotSample.refreshStats(into: model)
                        try? await Task.sleep(nanoseconds: 80_000_000)
                    }
                }
            }
            for appearance in appearances {
                NSApp.appearance = NSAppearance(named: appearance == "dark" ? .darkAqua : .aqua)
                for screen in screens {
                    if sample {
                        if screen == "downloads-empty" { model.tasks.load([], rev: model.tasks.snapshotRev) } else { SnapshotSample.loadTasks(into: model) }
                    }
                    ui.sidebar = item(screen, model: model)
                    ui.showInspector = screen == "inspector"
                    ui.selection = screen == "inspector" ? [SnapshotSample.inspectedId] : []
                    if screen == "add" { ui.openAdd() }
                    if screen == "settings" { delegate.openSettingsAction?() }
                    if screen == "menubar" {
                        await captureMenuBar(ui: ui, model: model, path: "\(prefix)-menubar-\(appearance).png", dark: appearance == "dark")
                        continue
                    }
                    try? await Task.sleep(nanoseconds: 1_600_000_000)
                    if sample { SnapshotSample.refreshHistory(into: model) }
                    try? await Task.sleep(nanoseconds: 350_000_000)
                    for (i, window) in NSApp.windows.enumerated() where window.isVisible && window.frame.height > 300 {
                        save(window, to: "\(prefix)-\(screen)-\(appearance)\(i == 0 ? "" : "-\(i)").png")
                    }
                    if screen == "add" { ui.addRequest = nil }
                    if screen == "settings" { NSApp.windows.filter { $0.identifier?.rawValue.contains("Settings") == true || $0.title.contains("Settings") }.forEach { $0.close() } }
                }
            }
            NSApp.appearance = nil
            if env["OSPREY_SNAPSHOT_QUIT"] == "1" {
                // Close windows first so no view (TimelineView etc.) renders during teardown.
                for w in NSApp.windows { w.orderOut(nil) }
                try? await Task.sleep(nanoseconds: 300_000_000)
                NSApp.terminate(nil)
            }
        }
    }

    private static func item(_ name: String, model: AppModel) -> SidebarItem {
        switch name {
        case "dashboard": return .dashboard
        case "completed": return .completed
        case "torrents": return .torrents
        case "scheduled": return .scheduled
        case "grabber": return .grabber
        case "history": return .history
        case "queue": return model.queues.first.map { .queue($0.id) } ?? .downloads
        default: return .downloads
        }
    }

    /// Hosts the menu bar extra's content in a temporary window and captures it.
    private static func captureMenuBar(ui: UIState, model: AppModel, path: String, dark: Bool) async {
        let host = NSHostingView(rootView: MenuBarView().environment(model).environment(ui))
        host.frame.size = host.fittingSize
        let window = NSWindow(contentRect: NSRect(origin: NSPoint(x: 200, y: 200), size: host.fittingSize),
                              styleMask: [.titled, .fullSizeContentView], backing: .buffered, defer: false)
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        window.contentView = NSVisualEffectView()
        (window.contentView as? NSVisualEffectView)?.material = .popover
        (window.contentView as? NSVisualEffectView)?.state = .active
        window.contentView?.addSubview(host)
        window.orderFront(nil)
        try? await Task.sleep(nanoseconds: 1_200_000_000)
        save(window, to: path)
        window.orderOut(nil)
    }

    private typealias WindowImageFn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?

    /// Compositor capture of our own window (includes glass, materials and Metal content). Looked
    /// up dynamically because the symbol is deprecated in newer SDKs; an app may always capture its
    /// own windows without Screen Recording permission.
    private static func compositedImage(_ window: NSWindow) -> CGImage? {
        guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else { return nil }
        let fn = unsafeBitCast(sym, to: WindowImageFn.self)
        // optionIncludingWindow = 1 << 3; boundsIgnoreFraming = 1 << 0, bestResolution = 1 << 3
        return fn(.null, 1 << 3, UInt32(window.windowNumber), (1 << 0) | (1 << 3))?.takeRetainedValue()
    }

    private static func save(_ window: NSWindow, to path: String) {
        if let cg = compositedImage(window), cg.width > 10 {
            let rep = NSBitmapImageRep(cgImage: cg)
            if let data = rep.representation(using: .png, properties: [:]) {
                try? data.write(to: URL(fileURLWithPath: path))
                return
            }
        }
        guard let view = window.contentView?.superview ?? window.contentView else { return }
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        if let data = rep.representation(using: .png, properties: [:]) {
            try? data.write(to: URL(fileURLWithPath: path))
        }
    }
}

/// Sample content for snapshot runs. Only ever installed when `OSPREY_SNAPSHOT_SAMPLE=1`.
@MainActor
enum SnapshotSample {
    static var active = false
    static var activity: [Date: DayActivity]?
    static var history: [HistoryEntryData]?
    static var allTimeBytes: UInt64 = 0
    static var allTimeFiles: UInt32 = 0
    static let inspectedId = "sample-1"

    private static let now = Date().millis

    static func rows() -> [TaskRowData] {
        func progress(_ done: Double, _ total: UInt64, speed: UInt64 = 0, eta: UInt64? = nil, conns: UInt32 = 0,
                      up: UInt64 = 0, peers: UInt32 = 0) -> ProgressData {
            var p = ProgressData()
            p.total = total
            p.downloaded = UInt64(Double(total) * done)
            p.fraction = done
            p.speed = speed
            p.etaSeconds = eta
            p.activeConnections = conns
            p.uploadSpeed = up
            p.peers = peers
            return p
        }
        let min: Int64 = 60_000
        return [
            TaskRowData(id: "sample-1", name: "field-recordings-vol2.flac.zip", state: .downloading, domain: "archive.lowtide.fm",
                        progress: progress(0.62, 2_310_000_000, speed: 18_400_000, eta: 48, conns: 8), position: 1,
                        createdAt: now - 12 * min, url: "https://archive.lowtide.fm/releases/field-recordings-vol2.flac.zip",
                        directory: NSHomeDirectory() + "/Downloads"),
            TaskRowData(id: "sample-2", name: "ubuntu-24.04.2-desktop-arm64.iso", kind: .torrent, state: .downloading,
                        progress: progress(0.81, 6_200_000_000, speed: 21_300_000, eta: 56, up: 1_150_000, peers: 42), position: 2,
                        createdAt: now - 45 * min, directory: NSHomeDirectory() + "/Downloads"),
            TaskRowData(id: "sample-3", name: "sunrise-timelapse-4k.mov", state: .downloading, domain: "media.kestrel.fm",
                        progress: progress(0.17, 3_400_000_000, speed: 2_600_000, eta: 1090, conns: 4), position: 3,
                        createdAt: now - 3 * min, directory: NSHomeDirectory() + "/Movies"),
            TaskRowData(id: "sample-4", name: "Keynote-Assets-2026.dmg", state: .paused, domain: "files.northwind.design",
                        progress: progress(0.34, 1_120_000_000), position: 4, createdAt: now - 140 * min,
                        blockedBy: ["user"], directory: NSHomeDirectory() + "/Downloads"),
            TaskRowData(id: "sample-5", name: "Blender-4.3-macOS-arm64.dmg", state: .queued, domain: "mirror.clarkson.edu",
                        progress: progress(0, 412_000_000), position: 5, createdAt: now - 2 * min, directory: NSHomeDirectory() + "/Downloads"),
            TaskRowData(id: "sample-6", name: "dataset-cities-2025.csv.gz", state: .failed, domain: "data.opencity.org",
                        progress: progress(0.08, 890_000_000), position: 6, createdAt: now - 26 * 60 * min,
                        errorMessage: "The server refused the request (403)", directory: NSHomeDirectory() + "/Downloads"),
            TaskRowData(id: "sample-7", name: "Quarterly-Report-Q3.pdf", state: .completed, domain: "docs.brightline.co",
                        progress: progress(1, 4_800_000), position: 7, createdAt: now - 90 * min, completedAt: now - 88 * min,
                        directory: NSHomeDirectory() + "/Downloads"),
            TaskRowData(id: "sample-8", name: "python-3.13.1-macos11.pkg", state: .completed, domain: "python.org",
                        progress: progress(1, 71_200_000), position: 8, createdAt: now - 200 * min, completedAt: now - 198 * min,
                        directory: NSHomeDirectory() + "/Downloads"),
            TaskRowData(id: "sample-9", name: "team-offsite-photos.zip", state: .completed, domain: "share.pinecone.photos",
                        progress: progress(1, 1_480_000_000), position: 9, createdAt: now - 30 * 60 * min, completedAt: now - 29 * 60 * min,
                        directory: NSHomeDirectory() + "/Downloads"),
            TaskRowData(id: "sample-10", name: "lecture-07-signals.mp4", state: .completed, domain: "courses.meridian.edu",
                        progress: progress(1, 612_000_000), position: 10, createdAt: now - 50 * 60 * min, completedAt: now - 49 * 60 * min,
                        directory: NSHomeDirectory() + "/Movies"),
        ]
    }

    static func loadTasks(into model: AppModel) {
        model.tasks.load(rows(), rev: model.tasks.snapshotRev)
    }

    static func install(into model: AppModel) {
        active = true
        // Only sample files appear in the recent list.
        let sampleIds = Set(rows().map(\.id))
        model.apply(model.recentCompletions.filter { !sampleIds.contains($0.id) }.map { .taskRemoved(id: $0.id) })
        loadTasks(into: model)
        for id in ["sample-10", "sample-9", "sample-8", "sample-7"] {
            model.apply([.taskStateChanged(id: id, from: .downloading, to: .completed)])
        }
        refreshHistory(into: model)
        refreshStats(into: model)

        var days: [Date: DayActivity] = [:]
        let cal = Calendar.current
        let today = cal.startOfDay(for: Date())
        for back in 0..<84 {
            guard let day = cal.date(byAdding: .day, value: -back, to: today) else { continue }
            let seed = (back * 7919 + 13) % 23
            if seed < 8 { continue }
            let weekend = cal.isDateInWeekend(day)
            let bytes = UInt64(seed * (weekend ? 900_000_000 : 380_000_000))
            days[day] = DayActivity(files: seed / 3 + 1, bytes: bytes)
        }
        activity = days
        let items: [(String, String, UInt64, TaskState, Int64, String?)] = [
            ("Quarterly-Report-Q3.pdf", "docs.brightline.co", 4_800_000, .completed, 88, nil),
            ("python-3.13.1-macos11.pkg", "python.org", 71_200_000, .completed, 198, nil),
            ("team-offsite-photos.zip", "share.pinecone.photos", 1_480_000_000, .completed, 1740, nil),
            ("lecture-07-signals.mp4", "courses.meridian.edu", 612_000_000, .completed, 2940, nil),
            ("dataset-cities-2025.csv.gz", "data.opencity.org", 890_000_000, .failed, 1560, "The server refused the request (403)"),
            ("night-drive-mix.flac", "archive.lowtide.fm", 412_000_000, .completed, 4320, nil),
            ("figma-exports-v3.zip", "files.northwind.design", 238_000_000, .cancelled, 5100, nil),
            ("debian-12.8.0-arm64-netinst.iso", "cdimage.debian.org", 654_000_000, .completed, 7200, nil),
            ("coastline-drone-4k.mov", "media.kestrel.fm", 3_900_000_000, .completed, 9800, nil),
        ]
        history = items.enumerated().map { i, e in
            HistoryEntryData(taskId: "history-\(i)", kind: .http, name: e.0, originalUrl: "https://\(e.1)/\(e.0)", domain: e.1,
                             size: e.2, state: e.3, destination: NSHomeDirectory() + "/Downloads/" + e.0,
                             finishedAt: now - e.4 * 60_000, durationSeconds: UInt64(20 + i * 37),
                             averageSpeed: UInt64(6_000_000 + i * 1_300_000), error: e.5, checksum: nil)
        }
        allTimeBytes = 1_842_000_000_000
        allTimeFiles = 2_317
    }

    /// Appends two minutes of varied throughput so the charts show a live trace.
    static func refreshHistory(into model: AppModel) {
        var events: [EngineEvent] = []
        let start = Date().millis
        for i in 0..<120 {
            var s = baseStats
            let t = Double(i)
            let wave = 0.55 + 0.25 * sin(t / 9) + 0.12 * sin(t / 3.1) + 0.08 * cos(t / 1.7)
            s.downloadSpeed = UInt64(max(0, 42_300_000 * wave * min(1, t / 20)))
            s.uploadSpeed = UInt64(max(0, 1_400_000 * (0.7 + 0.3 * sin(t / 5))))
            s.at = start - Int64(120 - i) * 1000
            events.append(.globalStats(s))
        }
        model.apply(events)
    }

    private static var baseStats: GlobalStatsData {
        var s = GlobalStatsData()
        s.active = 4
        s.downloading = 3
        s.seeding = 1
        s.queued = 1
        s.paused = 1
        s.completedToday = 6
        s.failedToday = 1
        s.totalTasks = 10
        s.bytesToday = 18_400_000_000
        s.trafficMode = .unlimited
        return s
    }

    static func refreshStats(into model: AppModel) {
        var s = baseStats
        s.downloadSpeed = 42_300_000
        s.uploadSpeed = 1_150_000
        s.at = Date().millis
        model.apply([.globalStats(s)])
    }
}
