import AppKit
import Foundation
import IOKit.pwr_mgt
import IOKit.ps
import Network
import ServiceManagement
import UserNotifications

// MARK: - Power assertion

/// Keeps the Mac awake (idle system sleep only; the display may still sleep) while transfers are
/// active and the user enabled "Prevent sleep while downloading".
@MainActor
public final class SleepPreventer {
    private var assertion: IOPMAssertionID = 0
    private var held = false

    public init() {}

    public func update(active: Bool, enabled: Bool) {
        let want = active && enabled
        guard want != held else { return }
        if want {
            let reason = "Osprey is downloading" as CFString
            let r = IOPMAssertionCreateWithName(kIOPMAssertPreventUserIdleSystemSleep as CFString,
                                                IOPMAssertionLevel(kIOPMAssertionLevelOn), reason, &assertion)
            held = r == kIOReturnSuccess
        } else {
            IOPMAssertionRelease(assertion)
            assertion = 0
            held = false
        }
    }

    public var isHeld: Bool { held }
}

// MARK: - Environment probe

/// Observes the network path and power source and pushes `EnvironmentData` to the engine's
/// scheduler conditions (metered/battery/VPN). SSID is omitted (needs location permission).
@MainActor
public final class EnvironmentMonitor {
    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "app.osprey.path-monitor")
    private var powerSource: CFRunLoopSource?
    private var timer: Timer?
    private var current = EnvironmentData()
    private let push: (EnvironmentData) -> Void

    public init(push: @escaping (EnvironmentData) -> Void) {
        self.push = push
    }

    public func start() {
        monitor.pathUpdateHandler = { [weak self] path in
            let available = path.status == .satisfied
            let metered = path.isExpensive || path.isConstrained
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.current.networkAvailable = available
                self.current.metered = metered
                self.refreshAndPush()
            }
        }
        monitor.start(queue: queue)

        let context = Unmanaged.passUnretained(self).toOpaque()
        if let src = IOPSNotificationCreateRunLoopSource({ ctx in
            guard let ctx else { return }
            let me = Unmanaged<EnvironmentMonitor>.fromOpaque(ctx).takeUnretainedValue()
            MainActor.assumeIsolated { me.refreshAndPush() }
        }, context)?.takeRetainedValue() {
            powerSource = src
            CFRunLoopAddSource(CFRunLoopGetMain(), src, .defaultMode)
        }
        // VPN interfaces appear/disappear without a path change on some setups; poll gently.
        timer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refreshAndPush() }
        }
        refreshAndPush()
    }

    public func stop() {
        monitor.cancel()
        timer?.invalidate()
        if let src = powerSource { CFRunLoopRemoveSource(CFRunLoopGetMain(), src, .defaultMode) }
    }

    private func refreshAndPush() {
        let power = Self.powerState()
        current.onAcPower = power.onAC
        current.batteryPercent = power.percent
        current.vpnActive = Self.vpnActive()
        current.ssid = nil
        push(current)
    }

    public static func powerState() -> (onAC: Bool, percent: UInt8?) {
        guard let info = IOPSCopyPowerSourcesInfo()?.takeRetainedValue() else { return (true, nil) }
        let type = IOPSGetProvidingPowerSourceType(info)?.takeUnretainedValue() as String?
        let onAC = type != kIOPSBatteryPowerValue
        var percent: UInt8?
        if let list = IOPSCopyPowerSourcesList(info)?.takeRetainedValue() as? [CFTypeRef] {
            for ps in list {
                guard let desc = IOPSGetPowerSourceDescription(info, ps)?.takeUnretainedValue() as? [String: Any] else { continue }
                if let cur = desc[kIOPSCurrentCapacityKey] as? Int, let max = desc[kIOPSMaxCapacityKey] as? Int, max > 0 {
                    percent = UInt8(clamping: cur * 100 / max)
                }
            }
        }
        return (onAC, percent)
    }

    /// A VPN is considered active when a point-to-point tunnel interface (utun/ipsec/ppp) carries
    /// an IPv4 address or a routable IPv6 address.
    public static func vpnActive() -> Bool {
        var ifaddr: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifaddr) == 0, let first = ifaddr else { return false }
        defer { freeifaddrs(ifaddr) }
        var found = false
        var ptr: UnsafeMutablePointer<ifaddrs>? = first
        while let p = ptr {
            let name = String(cString: p.pointee.ifa_name)
            let flags = Int32(p.pointee.ifa_flags)
            if (name.hasPrefix("utun") || name.hasPrefix("ipsec") || name.hasPrefix("ppp")),
               flags & IFF_UP != 0, let addr = p.pointee.ifa_addr {
                let family = addr.pointee.sa_family
                if family == UInt8(AF_INET) {
                    found = true
                } else if family == UInt8(AF_INET6) {
                    var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                    getnameinfo(addr, socklen_t(addr.pointee.sa_len), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST)
                    let s = String(cString: host)
                    if !s.lowercased().hasPrefix("fe80") { found = true }
                }
            }
            ptr = p.pointee.ifa_next
        }
        return found
    }
}

// MARK: - Login item

public enum LoginItem {
    public static var isEnabled: Bool { SMAppService.mainApp.status == .enabled }

    /// Returns an error message when the system refused.
    @discardableResult
    public static func set(_ enabled: Bool) -> String? {
        do {
            if enabled { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            return nil
        } catch {
            return error.localizedDescription
        }
    }
}

// MARK: - Finder

public enum Finder {
    public static func reveal(_ paths: [String]) {
        let urls = paths.map { URL(fileURLWithPath: $0) }.filter { FileManager.default.fileExists(atPath: $0.path) }
        if urls.isEmpty, let first = paths.first {
            // The file may not exist yet (in progress): reveal its folder.
            NSWorkspace.shared.open(URL(fileURLWithPath: first).deletingLastPathComponent())
        } else {
            NSWorkspace.shared.activateFileViewerSelecting(urls)
        }
    }

    @discardableResult
    public static func open(_ path: String) -> Bool {
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    /// Adds Finder tags (keeps existing ones).
    public static func addTags(_ tags: [String], to path: String) throws {
        let url = URL(fileURLWithPath: path)
        let existing = (try? url.resourceValues(forKeys: [.tagNamesKey]).tagNames) ?? []
        let merged = Array(Set(existing + tags)).sorted()
        try (url as NSURL).setResourceValue(merged, forKey: .tagNamesKey)
    }

    public static func tags(of path: String) -> [String] {
        (try? URL(fileURLWithPath: path).resourceValues(forKeys: [.tagNamesKey]).tagNames) ?? []
    }
}

// MARK: - Platform actions from the engine

/// Executes `FfiEvent::PlatformAction` payloads: open, reveal, Finder tags and AppleScript. Code-
/// executing actions only ever reach the app after the engine verified the user's consent hash.
@MainActor
public enum PlatformActionRunner {
    public static func run(actionJSON: String, contextJSON: String) -> String? {
        guard let action = try? JSONValue(parsing: actionJSON) else { return "Unreadable action" }
        let context = (try? JSONValue(parsing: contextJSON)) ?? .object([:])
        let path = context["file_path"]?.string ?? ""
        switch action["type"]?.string ?? "" {
        case "open":
            return Finder.open(path) ? nil : "Couldn't open \(path)"
        case "reveal_in_finder":
            Finder.reveal([path])
            return nil
        case "finder_tag":
            let tags = (action["tags"]?.array ?? []).compactMap(\.string)
            do { try Finder.addTags(tags, to: path); return nil } catch { return error.localizedDescription }
        case "run_apple_script":
            let script = action["script"]?.string ?? ""
            return runAppleScript(script, context: context)
        default:
            return nil
        }
    }

    /// Variables are passed as AppleScript properties via a prelude, never spliced into the body.
    static func runAppleScript(_ body: String, context: JSONValue) -> String? {
        var prelude = ""
        for (key, value) in (context.object ?? [:]).sorted(by: { $0.key < $1.key }) {
            guard let s = value.string ?? value.double.map({ String(Int64($0)) }) else { continue }
            let escaped = s.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
            prelude += "set osprey_\(key) to \"\(escaped)\"\n"
        }
        var error: NSDictionary?
        let script = NSAppleScript(source: prelude + body)
        script?.executeAndReturnError(&error)
        if let error { return error[NSAppleScript.errorMessage] as? String ?? "AppleScript failed" }
        return nil
    }
}

// MARK: - Sleep / quit on ReadyForSleep

@MainActor
public enum SystemPower {
    /// `reason` is `quit:<name>` (quit Osprey) or `queue:<name>` / `schedule:<name>` (sleep the Mac).
    public static func handleReadyForSleep(_ reason: String) {
        if reason.hasPrefix("quit:") {
            NSApp.terminate(nil)
        } else {
            sleepMac()
        }
    }

    public static func sleepMac() {
        let port = IOPMFindPowerManagement(mach_port_t(MACH_PORT_NULL))
        if port != 0 {
            let r = IOPMSleepSystem(port)
            IOServiceClose(port)
            if r == kIOReturnSuccess { return }
        }
        var err: NSDictionary?
        NSAppleScript(source: "tell application \"System Events\" to sleep")?.executeAndReturnError(&err)
    }
}
