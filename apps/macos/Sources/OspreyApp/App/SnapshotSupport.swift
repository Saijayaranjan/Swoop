import AppKit
import OspreyKit

/// Developer aid for headless visual checks (no Screen Recording permission needed): when
/// `OSPREY_SNAPSHOT` is set to a path prefix, Osprey renders its main window into PNGs for each
/// screen listed in `OSPREY_SNAPSHOT_SCREENS` (comma-separated sidebar names, default
/// `downloads,dashboard`) and `OSPREY_SNAPSHOT_APPEARANCE` (`light`, `dark` or both). Inert otherwise.
@MainActor
enum SnapshotSupport {
    static func runIfRequested(ui: UIState) {
        let env = ProcessInfo.processInfo.environment
        guard let prefix = env["OSPREY_SNAPSHOT"], !prefix.isEmpty else { return }
        let screens = (env["OSPREY_SNAPSHOT_SCREENS"] ?? "downloads,dashboard").split(separator: ",").map(String.init)
        let appearances = (env["OSPREY_SNAPSHOT_APPEARANCE"] ?? "light,dark").split(separator: ",").map(String.init)
        Task { @MainActor in
            NSApp.activate(ignoringOtherApps: true)
            NSApp.windows.first { $0.frame.height > 300 }?.makeKeyAndOrderFront(nil)
            try? await Task.sleep(nanoseconds: 2_500_000_000)
            for appearance in appearances {
                NSApp.appearance = NSAppearance(named: appearance == "dark" ? .darkAqua : .aqua)
                for screen in screens {
                    ui.sidebar = item(screen)
                    if screen == "inspector" { ui.showInspector = true } else { ui.showInspector = false }
                    if screen == "add" { ui.openAdd() }
                    try? await Task.sleep(nanoseconds: 1_800_000_000)
                    for (i, window) in NSApp.windows.enumerated() where window.isVisible && window.frame.height > 300 {
                        save(window, to: "\(prefix)-\(screen)-\(appearance)\(i == 0 ? "" : "-\(i)").png")
                    }
                    if screen == "add" { ui.addRequest = nil }
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

    private static func item(_ name: String) -> SidebarItem {
        switch name {
        case "completed": return .completed
        case "torrents": return .torrents
        case "scheduled": return .scheduled
        case "grabber": return .grabber
        case "history": return .history
        default: return .downloads
        }
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
