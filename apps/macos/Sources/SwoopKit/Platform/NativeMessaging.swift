import Foundation
import os

/// Registers the browser Native Messaging host (`app.swoop.bridge`) with every installed browser.
///
/// The manifests point straight at the running bundle's `Contents/Helpers/swoop`, which recognises
/// a browser launch by its arguments and runs the relay. The extension has a fixed ID (its
/// manifest carries a public `key`), so nothing needs to be typed in: `registerAll()` runs on every
/// launch and also repairs the manifests after Swoop.app moves.
public enum NativeMessagingInstaller {
    public static let hostName = "app.swoop.bridge"
    /// The Chromium extension ID, derived from the `key` in
    /// extensions/browser/manifests/{chrome,edge}.json. Must match `CHROMIUM_EXTENSION_ID` in
    /// crates/swoop-cli/src/commands/native_host.rs.
    public static let chromiumExtensionId = "hbfgocpejejjhpigpanikchicoplcfjb"
    /// IDs assigned by an extension store, if a published build ever carries a different key.
    static let chromiumStoreExtensionIds: [String] = []
    /// Firefox's fixed gecko ID, from extensions/browser/manifests/firefox.json.
    public static let firefoxExtensionId = "swoop@swoop.app"

    private static let log = Logger(subsystem: "app.swoop.desktop", category: "browser-integration")

    public enum Browser: String, CaseIterable, Identifiable, Sendable {
        case chrome, chromeBeta, chromeCanary, chromium, brave, edge, vivaldi, arc, opera, firefox
        public var id: String { rawValue }
        public var name: String {
            switch self {
            case .chrome: return "Google Chrome"
            case .chromeBeta: return "Google Chrome Beta"
            case .chromeCanary: return "Google Chrome Canary"
            case .chromium: return "Chromium"
            case .brave: return "Brave"
            case .edge: return "Microsoft Edge"
            case .vivaldi: return "Vivaldi"
            case .arc: return "Arc"
            case .opera: return "Opera"
            case .firefox: return "Firefox"
            }
        }
        /// Profile folder in ~/Library/Application Support.
        var profileDirectory: String {
            switch self {
            case .chrome: return "Google/Chrome"
            case .chromeBeta: return "Google/Chrome Beta"
            case .chromeCanary: return "Google/Chrome Canary"
            case .chromium: return "Chromium"
            case .brave: return "BraveSoftware/Brave-Browser"
            case .edge: return "Microsoft Edge"
            case .vivaldi: return "Vivaldi"
            case .arc: return "Arc/User Data"
            case .opera: return "com.operasoftware.Opera"
            case .firefox: return "Mozilla"
            }
        }
        /// A file whose presence means the browser has been used. Not the profile folder itself:
        /// other tools create bare `NativeMessagingHosts` folders for browsers that aren't
        /// installed. Checked with a stat, never a directory listing, which macOS guards.
        var markerFile: String {
            self == .firefox ? "Firefox/profiles.ini" : "\(profileDirectory)/Local State"
        }
        /// Folder holding the browser's `NativeMessagingHosts` (Opera reads Chrome's).
        var hostsParentDirectory: String { self == .opera ? "Google/Chrome" : profileDirectory }
    }

    public struct Status: Identifiable, Sendable {
        public var browser: Browser
        /// The manifest on disk is the one this copy of Swoop would write.
        public var connected: Bool
        public var id: String { browser.rawValue }
    }

    public static var defaultAppSupport: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    }

    /// The embedded CLI inside the running bundle (in Helpers: `MacOS/swoop` would collide with
    /// `MacOS/Swoop` on case-insensitive volumes).
    public static var helperURL: URL {
        Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/swoop")
    }

    static func manifestURL(_ b: Browser, appSupport: URL) -> URL {
        appSupport.appendingPathComponent(b.hostsParentDirectory)
            .appendingPathComponent("NativeMessagingHosts")
            .appendingPathComponent("\(hostName).json")
    }

    static func detectedBrowsers(appSupport: URL) -> [Browser] {
        Browser.allCases.filter { b in
            FileManager.default.fileExists(atPath: appSupport.appendingPathComponent(b.markerFile).path)
        }
    }

    static func manifestData(for b: Browser, helper: URL) -> Data {
        var manifest: [String: Any] = [
            "name": hostName,
            "description": "Swoop native messaging host",
            "path": helper.path,
            "type": "stdio",
        ]
        if b == .firefox {
            manifest["allowed_extensions"] = [firefoxExtensionId]
        } else {
            manifest["allowed_origins"] = ([chromiumExtensionId] + chromiumStoreExtensionIds)
                .map { "chrome-extension://\($0)/" }
        }
        // Force-unwrap: the dictionary only holds strings and string arrays.
        return try! JSONSerialization.data(withJSONObject: manifest,
                                           options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
    }

    /// Detected browsers and whether each one's manifest is current.
    public static func status(appSupport: URL = defaultAppSupport, helper: URL = helperURL) -> [Status] {
        detectedBrowsers(appSupport: appSupport).map { b in
            let current = try? Data(contentsOf: manifestURL(b, appSupport: appSupport))
            return Status(browser: b, connected: current == manifestData(for: b, helper: helper))
        }
    }

    /// Writes the host manifest for every detected browser, touching only files whose contents
    /// differ. Best-effort: failures are logged and returned, never thrown.
    @discardableResult
    public static func registerAll(appSupport: URL = defaultAppSupport, helper: URL = helperURL) -> [Browser: String] {
        guard FileManager.default.isExecutableFile(atPath: helper.path) else {
            log.info("No bundled helper at \(helper.path, privacy: .public); skipping browser registration")
            return [:]
        }
        // Manifests used to point at a launcher script; the helper is now launched directly.
        try? FileManager.default.removeItem(at: appSupport.appendingPathComponent("Swoop/native-host"))

        var errors: [Browser: String] = [:]
        for b in detectedBrowsers(appSupport: appSupport) {
            let url = manifestURL(b, appSupport: appSupport)
            let data = manifestData(for: b, helper: helper)
            if (try? Data(contentsOf: url)) == data { continue }
            do {
                try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
                try data.write(to: url, options: .atomic)
                log.info("Registered the browser helper for \(b.name, privacy: .public)")
            } catch {
                errors[b] = error.localizedDescription
                log.error("Couldn't register the browser helper for \(b.name, privacy: .public): \(error.localizedDescription, privacy: .public)")
            }
        }
        return errors
    }
}
