import Foundation

/// Installs the browser Native Messaging host manifests (`app.swoop.bridge`).
///
/// Browsers launch the manifest's `path` with the extension origin as the only argument and cannot
/// pass `native-host`, so the manifests point at a tiny launcher script (kept in Application
/// Support and rewritten on every install, so moving Swoop.app just needs a re-install) that execs
/// `Swoop.app/Contents/Helpers/swoop native-host`.
public enum NativeMessagingInstaller {
    public static let hostName = "app.swoop.bridge"
    /// Chromium-family browsers identify the extension by the id the store (or "Load unpacked")
    /// assigned; the user pastes it from chrome://extensions. Firefox uses the manifest's gecko id.
    public static let firefoxExtensionId = "swoop@swoop.app"

    public enum Browser: String, CaseIterable, Identifiable, Sendable {
        case chrome, chromium, edge, brave, arc, vivaldi, firefox
        public var id: String { rawValue }
        public var name: String {
            switch self {
            case .chrome: return "Google Chrome"
            case .chromium: return "Chromium"
            case .edge: return "Microsoft Edge"
            case .brave: return "Brave"
            case .arc: return "Arc"
            case .vivaldi: return "Vivaldi"
            case .firefox: return "Firefox"
            }
        }
        /// Directory (relative to ~/Library/Application Support) holding NativeMessagingHosts.
        var supportDirectory: String {
            switch self {
            case .chrome: return "Google/Chrome"
            case .chromium: return "Chromium"
            case .edge: return "Microsoft Edge"
            case .brave: return "BraveSoftware/Brave-Browser"
            case .arc: return "Arc/User Data"
            case .vivaldi: return "Vivaldi"
            case .firefox: return "Mozilla"
            }
        }
        var bundleIds: [String] {
            switch self {
            case .chrome: return ["com.google.Chrome"]
            case .chromium: return ["org.chromium.Chromium"]
            case .edge: return ["com.microsoft.edgemac"]
            case .brave: return ["com.brave.Browser"]
            case .arc: return ["company.thebrowser.Browser"]
            case .vivaldi: return ["com.vivaldi.Vivaldi"]
            case .firefox: return ["org.mozilla.firefox", "org.mozilla.firefoxdeveloperedition", "org.mozilla.nightly"]
            }
        }
    }

    public struct Status: Identifiable, Sendable {
        public var browser: Browser
        public var browserInstalled: Bool
        public var manifestInstalled: Bool
        public var manifestPath: String
        public var id: String { browser.rawValue }
    }

    static var appSupport: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    }

    public static func manifestURL(_ b: Browser) -> URL {
        appSupport.appendingPathComponent(b.supportDirectory).appendingPathComponent("NativeMessagingHosts")
            .appendingPathComponent("\(hostName).json")
    }

    public static var launcherURL: URL {
        appSupport.appendingPathComponent("Swoop/native-host/swoop-native-host")
    }

    /// The embedded CLI inside the running bundle (in Helpers: `MacOS/swoop` would collide with
    /// `MacOS/Swoop` on case-insensitive volumes).
    public static var cliURL: URL {
        Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/swoop")
    }

    public static func status() -> [Status] {
        Browser.allCases.map { b in
            let installed = b.bundleIds.contains { id in
                LSCopyApplicationURLsForBundleIdentifier(id as CFString, nil)?.takeRetainedValue() != nil
            } || FileManager.default.fileExists(atPath: appSupport.appendingPathComponent(b.supportDirectory).path)
            let url = manifestURL(b)
            return Status(browser: b, browserInstalled: installed,
                          manifestInstalled: FileManager.default.fileExists(atPath: url.path), manifestPath: url.path)
        }
    }

    /// Writes the launcher and the manifest for `browsers`. Returns per-browser errors.
    @discardableResult
    public static func install(_ browsers: [Browser], chromiumExtensionId: String) -> [Browser: String] {
        var errors: [Browser: String] = [:]
        let chromiumId = chromiumExtensionId.trimmingCharacters(in: .whitespacesAndNewlines)
        let validId = chromiumId.count == 32 && chromiumId.allSatisfy { ("a"..."p").contains($0) }
        do {
            try writeLauncher()
        } catch {
            for b in browsers { errors[b] = "Couldn't write the launcher: \(error.localizedDescription)" }
            return errors
        }
        for b in browsers {
            var manifest: [String: Any] = [
                "name": hostName,
                "description": "Swoop native messaging host",
                "path": launcherURL.path,
                "type": "stdio",
            ]
            if b == .firefox {
                manifest["allowed_extensions"] = [firefoxExtensionId]
            } else if validId {
                manifest["allowed_origins"] = ["chrome-extension://\(chromiumId)/"]
            } else {
                errors[b] = "Enter the extension ID shown on the browser's Extensions page (32 letters a–p)."
                continue
            }
            do {
                let url = manifestURL(b)
                try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
                let data = try JSONSerialization.data(withJSONObject: manifest, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
                try data.write(to: url, options: .atomic)
            } catch {
                errors[b] = error.localizedDescription
            }
        }
        return errors
    }

    public static func uninstall(_ browsers: [Browser]) {
        for b in browsers { try? FileManager.default.removeItem(at: manifestURL(b)) }
    }

    static func writeLauncher() throws {
        let url = launcherURL
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let cli = cliURL.path.replacingOccurrences(of: "'", with: "'\\''")
        let script = """
        #!/bin/sh
        # Generated by Swoop. Browsers start this with the extension origin as argument; the
        # relay itself ignores it and speaks Native Messaging on stdin/stdout.
        exec '\(cli)' native-host
        """
        try script.write(to: url, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
    }
}
