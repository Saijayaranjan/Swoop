import AppKit
import OspreyKit
import SwiftUI

/// Bindings straight into the engine's settings document; every change is persisted.
@MainActor
struct SettingsBindings {
    let model: AppModel

    func bool(_ path: String, _ d: Bool = false) -> Binding<Bool> {
        Binding(get: { model.settings.bool(path, default: d) }, set: { model.setSetting(path, .bool($0)) })
    }
    func int(_ path: String, _ d: Int = 0) -> Binding<Int> {
        Binding(get: { model.settings.int(path, default: d) }, set: { model.setSetting(path, .number(Double($0))) })
    }
    func double(_ path: String, _ d: Double = 0) -> Binding<Double> {
        Binding(get: { model.settings.double(path, default: d) }, set: { model.setSetting(path, .number($0)) })
    }
    func string(_ path: String, _ d: String = "") -> Binding<String> {
        Binding(get: { model.settings.string(path, default: d) }, set: { model.setSetting(path, .string($0)) })
    }
    func optionalString(_ path: String) -> Binding<String> {
        Binding(get: { model.settings.string(path) }, set: { model.setSetting(path, $0.isEmpty ? .null : .string($0)) })
    }
    func bytes(_ path: String) -> Binding<UInt64> {
        Binding(get: { UInt64(max(0, model.settings.double(path))) }, set: { model.setSetting(path, .number(Double($0))) })
    }
    func list(_ path: String) -> Binding<String> {
        Binding(get: { model.settings.strings(path).joined(separator: ", ") },
                set: { model.setSetting(path, .array($0.split(separator: ",").map { .string($0.trimmingCharacters(in: .whitespaces)) }.filter { $0.string?.isEmpty == false })) })
    }
    func lines(_ path: String) -> Binding<String> {
        Binding(get: { model.settings.strings(path).joined(separator: "\n") },
                set: { model.setSetting(path, .array($0.split(whereSeparator: \.isNewline).map { .string($0.trimmingCharacters(in: .whitespaces)) }.filter { $0.string?.isEmpty == false })) })
    }
}

enum SettingsPane: String, CaseIterable, Identifiable {
    case general, downloads, queues, categories, rules, automations, recipes, network, bandwidth, torrents, browser, notifications, remote, devices, automation, privacy, updates, advanced
    var id: String { rawValue }
    var title: String {
        switch self {
        case .automations: return "Automation"
        case .automation: return "Plugins"
        default: return rawValue.capitalized
        }
    }
    var symbol: String {
        switch self {
        case .general: return "gearshape"
        case .downloads: return "arrow.down.circle"
        case .network: return "network"
        case .bandwidth: return "speedometer"
        case .torrents: return "point.3.connected.trianglepath.dotted"
        case .browser: return "safari"
        case .notifications: return "bell.badge"
        case .remote: return "iphone.and.arrow.forward"
        case .automation: return "puzzlepiece.extension"
        case .queues: return "tray.2"
        case .categories: return "square.grid.2x2"
        case .rules: return "arrow.triangle.branch"
        case .automations: return "gearshape.2"
        case .recipes: return "wand.and.stars"
        case .devices: return "iphone"
        case .privacy: return "hand.raised"
        case .updates: return "arrow.triangle.2.circlepath"
        case .advanced: return "wrench.and.screwdriver"
        }
    }
}

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("settingsPane") private var paneRaw = SettingsPane.general.rawValue

    var body: some View {
        NavigationSplitView {
            List(selection: Binding(get: { paneRaw }, set: { if let v = $0 { paneRaw = v } })) {
                ForEach(SettingsPane.allCases) { pane in
                    Label(LocalizedStringKey(pane.title), systemImage: pane.symbol).tag(pane.rawValue)
                }
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(190)
        } detail: {
            paneView(SettingsPane(rawValue: paneRaw) ?? .general)
                .navigationTitle(LocalizedStringKey((SettingsPane(rawValue: paneRaw) ?? .general).title))
        }
        .frame(width: 820, height: 600)
    }

    @ViewBuilder
    private func paneView(_ pane: SettingsPane) -> some View {
        let b = SettingsBindings(model: model)
        Group {
            switch pane {
            case .general: GeneralPane(b: b)
            case .downloads: DownloadsPane(b: b)
            case .network: NetworkPane(b: b)
            case .bandwidth: BandwidthPane(b: b)
            case .torrents: TorrentsPane(b: b)
            case .browser: BrowserPane(b: b)
            case .notifications: NotificationsPane(b: b)
            case .remote: RemotePane(b: b)
            case .automation: AutomationPane()
            case .queues: QueuesSettingsPane()
            case .categories: CategoriesView()
            case .rules: RulesView()
            case .automations: AutomationView()
            case .recipes: RecipesView()
            case .devices: DevicesView()
            case .privacy: PrivacyPane(b: b)
            case .updates: UpdatesPane(b: b)
            case .advanced: AdvancedPane()
            }
        }
        .formStyle(.grouped)
    }
}

// MARK: - Panes

struct GeneralPane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model
    @ViewState private var loginError: String?

    var body: some View {
        Form {
            Section {
                Toggle("Open Osprey at login", isOn: Binding(get: { LoginItem.isEnabled }, set: { on in
                    loginError = LoginItem.set(on)
                    model.setSetting("appearance.launch_at_login", .bool(LoginItem.isEnabled))
                }))
                if let loginError { Text(loginError).font(.caption).foregroundStyle(Theme.danger) }
                Toggle("Show in the menu bar", isOn: b.bool("appearance.show_menu_bar_extra", true))
                Toggle("Show active downloads on the Dock icon", isOn: b.bool("appearance.show_dock_badge", true))
                Toggle("Keep the Mac awake while downloading", isOn: b.bool("appearance.prevent_sleep_while_active", true))
                Toggle("Ask before quitting with active downloads", isOn: b.bool("appearance.confirm_on_quit_with_active", true))
            }
            Section("Appearance") {
                Picker("Theme", selection: Binding(get: { model.settings.string("appearance.theme", default: "system") }, set: { v in
                    model.setSetting("appearance.theme", .string(v))
                    AppearanceApplier.apply(v)
                })) {
                    Text("Match System").tag("system")
                    Text("Light").tag("light")
                    Text("Dark").tag("dark")
                }
                .pickerStyle(.segmented)
                Toggle("Compact rows", isOn: b.bool("appearance.compact_rows"))
                Toggle("Show the throughput graph", isOn: b.bool("appearance.show_speed_graph", true))
            }
            Section("Language") {
                Picker("Language", selection: Binding(get: { model.settings.string("appearance.language", default: "en") }, set: { v in
                    model.setSetting("appearance.language", .string(v))
                    UserDefaults.standard.set([v], forKey: "AppleLanguages")
                })) {
                    Text("English").tag("en")
                    Text("हिन्दी").tag("hi")
                    Text("தமிழ்").tag("ta")
                }
                Text("Takes effect the next time Osprey opens.").font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

enum AppearanceApplier {
    @MainActor static func apply(_ theme: String) {
        switch theme {
        case "light": NSApp.appearance = NSAppearance(named: .aqua)
        case "dark": NSApp.appearance = NSAppearance(named: .darkAqua)
        default: NSApp.appearance = nil
        }
    }
}

struct DownloadsPane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model
    var body: some View {
        Form {
            Section("Location") {
                HStack {
                    Text("Download folder")
                    Spacer()
                    Text(model.settings.downloadDirectory.abbreviatedPath).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    Button("Choose…") {
                        let panel = NSOpenPanel()
                        panel.canChooseDirectories = true
                        panel.canChooseFiles = false
                        panel.canCreateDirectories = true
                        panel.directoryURL = URL(fileURLWithPath: model.settings.downloadDirectory)
                        if panel.runModal() == .OK, let url = panel.url { model.setSetting("storage.download_directory", .string(url.path)) }
                    }
                }
                Toggle("Organise into category folders", isOn: b.bool("storage.organise_by_category"))
            }
            Section("Files") {
                Toggle("Reserve disk space before downloading", isOn: b.bool("storage.preallocate", true))
                Toggle("Use sparse files", isOn: b.bool("storage.sparse_files"))
                LabeledContent("Keep free on disk") { BytesField(bytes: b.bytes("storage.reserved_free_space")) }
                LabeledContent("Warn before downloads larger than") { BytesField(bytes: b.bytes("storage.large_download_warning")) }
                Toggle("Mark downloaded files as from the internet (Gatekeeper)", isOn: b.bool("storage.quarantine_downloads", true))
            }
            Section("Integrity") {
                Toggle("Compute a checksum when downloads finish", isOn: b.bool("storage.verify_checksum_on_complete", true))
                Picker("Algorithm", selection: b.string("storage.default_checksum_algorithm", "sha256")) {
                    ForEach(ChecksumAlgorithm.allCases, id: \.self) { Text($0.label).tag($0.rawValue) }
                }
                Toggle("Detect duplicates", isOn: b.bool("storage.duplicate_detection", true))
                Picker("When a file already exists", selection: b.string("storage.default_conflict_policy", "ask")) {
                    Text("Ask").tag("ask")
                    Text("Keep both").tag("rename")
                    Text("Replace").tag("replace")
                    Text("Skip").tag("skip")
                }
            }
        }
    }
}

struct NetworkPane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model
    @ViewState private var optimizing = false
    @ViewState private var newProxy = ProxyDraft()

    struct ProxyDraft { var name = ""; var kind = "http"; var host = ""; var port = 8080; var user = ""; var password = "" }

    var body: some View {
        Form {
            Section("Connections") {
                Stepper(value: b.int("network.connections_per_task", 8), in: 1...64) {
                    LabeledContent("Connections per download", value: "\(model.settings.int("network.connections_per_task", default: 8))")
                }
                Stepper(value: b.int("network.max_connections_per_host", 16), in: 1...128) {
                    LabeledContent("Per server", value: "\(model.settings.int("network.max_connections_per_host", default: 16))")
                }
                Stepper(value: b.int("network.max_total_connections", 64), in: 1...512) {
                    LabeledContent("In total", value: "\(model.settings.int("network.max_total_connections", default: 64))")
                }
                Toggle("Adapt connections to the server", isOn: b.bool("network.adaptive_segmentation", true))
                LabeledContent("Smallest segment") { BytesField(bytes: b.bytes("network.min_segment_size")) }
                Toggle("Allow HTTP/2 multiplexing for segments", isOn: b.bool("network.allow_http2_for_segments"))
                HStack {
                    Button {
                        optimizing = true
                        Task {
                            if await model.perform("Couldn't optimise", { try await model.engine.optimize() }) != nil {
                                model.toast(.success, "Connection settings tuned for your network")
                            }
                            optimizing = false
                        }
                    } label: { Label("Optimise for My Network", systemImage: "wand.and.rays") }
                    .disabled(optimizing)
                    if optimizing { ProgressView().controlSize(.small) }
                }
            }
            Section("Reliability") {
                Stepper(value: b.int("network.max_retries", 8), in: 0...50) {
                    LabeledContent("Retries", value: "\(model.settings.int("network.max_retries", default: 8))")
                }
                Stepper(value: b.int("network.connect_timeout_seconds", 20), in: 5...120, step: 5) {
                    LabeledContent("Connect timeout", value: "\(model.settings.int("network.connect_timeout_seconds", default: 20)) s")
                }
                Stepper(value: b.int("network.read_timeout_seconds", 45), in: 5...300, step: 5) {
                    LabeledContent("Read timeout", value: "\(model.settings.int("network.read_timeout_seconds", default: 45)) s")
                }
                Toggle("Verify TLS certificates", isOn: b.bool("network.verify_tls", true))
                Toggle("IPv4 only", isOn: b.bool("network.ipv4_only"))
                Toggle("Prefer IPv6", isOn: b.bool("network.prefer_ipv6"))
                TextField("User agent", text: b.string("network.user_agent"))
            }
            Section("Proxy") {
                let proxies = model.settings["network.proxies"]?.array ?? []
                Picker("Use proxy", selection: b.optionalString("network.global_proxy")) {
                    Text("None (direct)").tag("")
                    ForEach(proxies.compactMap { $0["id"]?.string }, id: \.self) { id in
                        Text(proxies.first { $0["id"]?.string == id }?["name"]?.string ?? id).tag(id)
                    }
                }
                ForEach(proxies.compactMap { $0["id"]?.string }, id: \.self) { id in
                    let p = proxies.first { $0["id"]?.string == id }
                    HStack {
                        Text(p?["name"]?.string ?? id)
                        Text("\(p?["kind"]?.string ?? "") \(p?["host"]?.string ?? ""):\(p?["port"]?.int ?? 0)").font(.caption.monospaced()).foregroundStyle(.secondary)
                        Spacer()
                        Button("Remove") {
                            model.setSetting("network.proxies", .array(proxies.filter { $0["id"]?.string != id }))
                        }
                        .buttonStyle(.borderless)
                    }
                }
                DisclosureGroup("Add Proxy…") {
                    TextField("Name", text: $newProxy.name)
                    Picker("Type", selection: $newProxy.kind) {
                        Text("HTTP").tag("http"); Text("HTTPS").tag("https"); Text("SOCKS5").tag("socks5")
                    }
                    TextField("Host", text: $newProxy.host)
                    TextField("Port", value: $newProxy.port, format: .number.grouping(.never))
                    TextField("User name (optional)", text: $newProxy.user)
                    SecureField("Password (stored in Keychain)", text: $newProxy.password)
                    Button("Add Proxy") { Task { await addProxy(existing: proxies) } }
                        .disabled(newProxy.host.isEmpty || newProxy.name.isEmpty)
                }
            }
        }
    }

    private func addProxy(existing: [JSONValue]) async {
        var entry: [String: JSONValue] = [
            "id": .string(UUID().uuidString.lowercased()), "name": .string(newProxy.name), "kind": .string(newProxy.kind),
            "host": .string(newProxy.host), "port": .number(Double(newProxy.port)), "bypass": .array([]),
        ]
        if !newProxy.password.isEmpty {
            guard let cred = await model.perform("Couldn't store the proxy password", {
                try await model.engine.storeCredential(name: "Proxy \(newProxy.name)", username: newProxy.user.isEmpty ? nil : newProxy.user, secret: newProxy.password)
            }) else { return }
            entry["credential"] = .string(cred)
        }
        model.setSetting("network.proxies", .array(existing + [.object(entry)]))
        newProxy = ProxyDraft()
    }
}

struct BandwidthPane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model
    var body: some View {
        Form {
            Section("Speed mode") {
                Picker("Mode", selection: Binding(get: { model.stats.trafficMode }, set: { model.setTrafficMode($0) })) {
                    ForEach(TrafficMode.pickerModes, id: \.self) { Label($0.label, systemImage: $0.symbol).tag($0) }
                }
                .pickerStyle(.inline)
                Text(modeHelp).font(.caption).foregroundStyle(.secondary)
                LabeledContent("Custom download limit") { BytesField(bytes: b.bytes("bandwidth.custom_download_limit")) }
                LabeledContent("Custom upload limit") { BytesField(bytes: b.bytes("bandwidth.custom_upload_limit")) }
                let cap = UInt64(max(0, model.settings.double("bandwidth.measured_capacity")))
                LabeledContent("Measured connection", value: cap > 0 ? Fmt.speed(cap) : L10n.tr("Not measured yet"))
            }
            Section("Concurrency") {
                Stepper(value: b.int("bandwidth.max_active_downloads", 5), in: 1...50) {
                    LabeledContent("Active downloads", value: "\(model.settings.int("bandwidth.max_active_downloads", default: 5))")
                }
                Stepper(value: b.int("bandwidth.max_active_torrents", 5), in: 1...50) {
                    LabeledContent("Active torrents", value: "\(model.settings.int("bandwidth.max_active_torrents", default: 5))")
                }
            }
        }
    }

    private var modeHelp: String {
        switch model.stats.trafficMode {
        case .unlimited, .fullSpeed: return L10n.tr("Use all available bandwidth.")
        case .balanced: return L10n.tr("Leave about 30% of your connection for everything else.")
        case .browsing: return L10n.tr("Stay out of the way while you browse and call.")
        case .custom: return L10n.tr("Use the limits below.")
        }
    }
}

struct TorrentsPane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model
    var body: some View {
        Form {
            Section {
                Toggle("Enable BitTorrent", isOn: b.bool("torrent.enabled", true))
                TextField("Listening port (0 = automatic)", value: b.int("torrent.listen_port"), format: .number.grouping(.never))
                Toggle("Distributed hash table (DHT)", isOn: b.bool("torrent.dht", true))
                Toggle("Peer exchange (PEX)", isOn: b.bool("torrent.pex", true))
                Toggle("Require encrypted connections", isOn: b.bool("torrent.encryption_required"))
                Stepper(value: b.int("torrent.max_peers_per_torrent", 120), in: 10...1000, step: 10) {
                    LabeledContent("Peers per torrent", value: "\(model.settings.int("torrent.max_peers_per_torrent", default: 120))")
                }
                Toggle("Download pieces in order by default", isOn: b.bool("torrent.sequential_by_default"))
            }
            Section("Seeding") {
                Toggle("Seed after downloading", isOn: b.bool("torrent.seed_when_complete", true))
                LabeledContent("Stop at ratio") {
                    TextField("", value: b.double("torrent.seed_ratio_limit", 2), format: .number.precision(.fractionLength(1))).frame(width: 70)
                }
                Stepper(value: b.int("torrent.seed_time_limit_minutes"), in: 0...10080, step: 30) {
                    LabeledContent("Stop after", value: model.settings.int("torrent.seed_time_limit_minutes") == 0 ? L10n.tr("No limit") : "\(model.settings.int("torrent.seed_time_limit_minutes")) min")
                }
                LabeledContent("Upload limit") { BytesField(bytes: b.bytes("torrent.upload_limit")) }
            }
            Section("Trackers") {
                TextEditor(text: b.lines("torrent.additional_trackers"))
                    .font(.caption.monospaced())
                    .frame(height: 70)
                Text("Added to public torrents, one per line.").font(.caption).foregroundStyle(.secondary)
                TextField("Tracker list URL", text: b.optionalString("torrent.tracker_source_url"))
                Button("Refresh Tracker List Now") {
                    Task {
                        if let n = await model.perform("Couldn't refresh trackers", { try await model.engine.refreshTrackerList() }) {
                            model.toast(.success, "Updated \(n) torrents")
                        }
                    }
                }
            }
        }
    }
}

struct BrowserPane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model
    @AppStorage("chromiumExtensionId") private var extensionId = ""
    @ViewState private var status: [NativeMessagingInstaller.Status] = NativeMessagingInstaller.status()
    @ViewState private var errors: [NativeMessagingInstaller.Browser: String] = [:]

    var body: some View {
        Form {
            Section("Browser integration") {
                ForEach(status) { s in
                    HStack {
                        Image(systemName: s.manifestInstalled ? "checkmark.circle.fill" : "circle")
                            .foregroundStyle(s.manifestInstalled ? Theme.success : .secondary)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(s.browser.name)
                            if let e = errors[s.browser] { Text(e).font(.caption).foregroundStyle(Theme.danger) }
                            else if !s.browserInstalled { Text("Not installed").font(.caption).foregroundStyle(.secondary) }
                        }
                        Spacer()
                        if s.manifestInstalled {
                            Button("Remove") { NativeMessagingInstaller.uninstall([s.browser]); refresh() }
                        } else {
                            Button("Install") { install([s.browser]) }
                        }
                    }
                }
                TextField("Chromium extension ID", text: $extensionId, prompt: Text("From chrome://extensions"))
                    .font(.body.monospaced())
                Text("Chrome, Edge, Brave, Arc and Vivaldi need the ID of the Osprey extension you installed. Firefox is recognised automatically.")
                    .font(.caption).foregroundStyle(.secondary)
                Button("Install for All Browsers") { install(status.filter(\.browserInstalled).map(\.browser)) }
            }
            Section("Catching downloads") {
                Toggle("Take over downloads from the browser", isOn: b.bool("browser.intercept_downloads", true))
                LabeledContent("Only files larger than") { BytesField(bytes: b.bytes("browser.intercept_min_size")) }
                TextField("File types", text: b.list("browser.intercept_extensions"))
                TextField("Never on these sites", text: b.list("browser.excluded_domains"))
                Toggle("Detect videos and audio on pages", isOn: b.bool("browser.detect_media", true))
                Toggle("Confirm before downloading", isOn: b.bool("browser.show_confirmation", true))
            }
        }
    }

    private func install(_ browsers: [NativeMessagingInstaller.Browser]) {
        errors = NativeMessagingInstaller.install(browsers, chromiumExtensionId: extensionId)
        refresh()
        if errors.isEmpty { model.toast(.success, "Browser integration installed") }
    }

    private func refresh() { status = NativeMessagingInstaller.status() }
}

struct NotificationsPane: View {
    let b: SettingsBindings
    var body: some View {
        Form {
            Section("Notify me when") {
                Toggle("A download finishes", isOn: b.bool("notifications.completed", true))
                Toggle("A download fails", isOn: b.bool("notifications.failed", true))
                Toggle("A torrent finishes", isOn: b.bool("notifications.torrent_finished", true))
                Toggle("A checksum doesn't match", isOn: b.bool("notifications.checksum_mismatch", true))
                Toggle("Disk space runs low", isOn: b.bool("notifications.low_disk_space", true))
                Toggle("A download is queued", isOn: b.bool("notifications.queued"))
                Toggle("A download is scheduled", isOn: b.bool("notifications.scheduled", true))
                Toggle("A device pairs", isOn: b.bool("notifications.device_paired", true))
                Toggle("An automation fails", isOn: b.bool("notifications.automation_failure", true))
            }
            Section {
                Toggle("Play a sound", isOn: b.bool("notifications.sound", true))
                Toggle("Stay quiet while Osprey is in front", isOn: b.bool("notifications.quiet_when_active"))
                Button("Open Notification Settings…") {
                    NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension")!)
                }
            }
        }
    }
}

struct RemotePane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model
    var body: some View {
        Form {
            Section {
                Toggle("Allow remote control", isOn: b.bool("remote.enabled"))
                Text("Pair phones and other computers from Devices in the sidebar. Connections are encrypted with a certificate unique to this Mac.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("Listener") {
                TextField("Address", text: b.string("remote.bind_address", "0.0.0.0"))
                TextField("Port", value: b.int("remote.port", 41780), format: .number.grouping(.never))
                Toggle("Require TLS", isOn: b.bool("remote.tls", true))
                TextField("Local API port", value: b.int("remote.local_port", 41779), format: .number.grouping(.never))
            }
            Section("Security") {
                Stepper(value: b.int("remote.session_ttl_hours", 720), in: 1...8760, step: 24) {
                    LabeledContent("Sessions expire after", value: "\(model.settings.int("remote.session_ttl_hours", default: 720) / 24) days")
                }
                Stepper(value: b.int("remote.max_failed_attempts", 5), in: 1...50) {
                    LabeledContent("Lock out after failed attempts", value: "\(model.settings.int("remote.max_failed_attempts", default: 5))")
                }
                Stepper(value: b.int("remote.lockout_minutes", 15), in: 1...1440) {
                    LabeledContent("Lockout", value: "\(model.settings.int("remote.lockout_minutes", default: 15)) min")
                }
                TextField("Allowed web origins", text: b.list("remote.allowed_origins"))
            }
            if model.info.remoteEnabled, let port = model.info.remotePort {
                Section { LabeledContent("Listening on port", value: "\(port)") }
            }
        }
    }
}

struct AutomationPane: View {
    @Environment(AppModel.self) private var model
    @ViewState private var plugins: [PluginData] = []
    var body: some View {
        Form {
            Section("Automations") {
                LabeledContent("Automations", value: "\(model.automations.count)")
                LabeledContent("That run code", value: "\(model.automations.filter(\.executesCode).count)")
                Text("Code-running actions only execute after you grant consent for the exact script. Manage them in Automation in the sidebar.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("Plugins") {
                if plugins.isEmpty { Text("No plugins installed.").foregroundStyle(.secondary) }
                ForEach(plugins) { p in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Toggle(isOn: Binding(get: { p.enabled }, set: { on in
                                Task {
                                    await model.perform("Couldn't update plugin") { try await model.engine.setPluginEnabled(p.id, on, granted: on ? p.permissions : []) }
                                    plugins = (try? await model.engine.plugins()) ?? plugins
                                }
                            })) {
                                Text("\(p.name) \(p.version)").font(.callout.weight(.medium))
                            }
                            Spacer()
                            Button("Uninstall", role: .destructive) {
                                Task {
                                    await model.perform("Couldn't uninstall") { try await model.engine.uninstallPlugin(p.id) }
                                    plugins = (try? await model.engine.plugins()) ?? []
                                }
                            }
                            .buttonStyle(.borderless)
                        }
                        Text(p.description).font(.caption).foregroundStyle(.secondary)
                        if !p.permissions.isEmpty {
                            Text("Needs: \(p.permissions.joined(separator: ", "))").font(.caption2).foregroundStyle(Theme.warning)
                        }
                    }
                }
            }
        }
        .task {
            await model.load("automations")
            plugins = (try? await model.engine.plugins()) ?? []
        }
    }
}

struct PrivacyPane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model
    var body: some View {
        Form {
            Section("History") {
                Toggle("Keep download history", isOn: b.bool("privacy.keep_history", true))
                Stepper(value: b.int("privacy.history_retention_days"), in: 0...3650, step: 30) {
                    LabeledContent("Keep for", value: model.settings.int("privacy.history_retention_days") == 0 ? L10n.tr("Forever") : "\(model.settings.int("privacy.history_retention_days")) days")
                }
            }
            Section("Logs") {
                Picker("Log detail", selection: b.string("privacy.log_level", "info")) {
                    ForEach(["error", "warn", "info", "debug", "trace"], id: \.self) { Text($0.capitalized).tag($0) }
                }
                Stepper(value: b.int("privacy.log_retention_days", 14), in: 1...365) {
                    LabeledContent("Keep logs for", value: "\(model.settings.int("privacy.log_retention_days", default: 14)) days")
                }
                Text("Logs never contain passwords, cookies or tokens.").font(.caption).foregroundStyle(.secondary)
            }
            Section("Saved sign-ins (Keychain)") {
                if model.credentials.isEmpty { Text("None").foregroundStyle(.secondary) }
                ForEach(model.credentials) { c in
                    HStack {
                        Image(systemName: "key.fill").foregroundStyle(.secondary)
                        Text(c.name)
                        if let u = c.username { Text(u).foregroundStyle(.secondary) }
                        Spacer()
                        Button("Delete", role: .destructive) {
                            Task {
                                await model.perform("Couldn't delete") { try await model.engine.deleteCredential(c.id) }
                                await model.load("credentials")
                            }
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
            Section {
                Toggle("Share anonymous usage statistics", isOn: b.bool("privacy.analytics_opt_in"))
            }
        }
        .task { await model.load("credentials") }
    }
}

struct UpdatesPane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model
    @ViewState private var info: UpdateInfoData?
    @ViewState private var busy = false
    @ViewState private var downloadedPath: String?

    var body: some View {
        Form {
            Section {
                Toggle("Check for updates automatically", isOn: b.bool("updates.check_automatically", true))
                Toggle("Download and install automatically", isOn: b.bool("updates.install_automatically"))
                Picker("Channel", selection: b.string("updates.channel", "stable")) {
                    Text("Stable").tag("stable")
                    Text("Beta").tag("beta")
                }
            }
            Section {
                LabeledContent("Current version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? model.info.version)
                if let info {
                    if info.available {
                        LabeledContent("Available", value: info.latestVersion ?? "")
                        if let notes = info.notes { Text(notes).font(.caption).textSelection(.enabled) }
                        if info.signatureValid == false { Label("The update's signature is invalid; it won't be installed.", systemImage: "xmark.shield").foregroundStyle(Theme.danger) }
                    } else {
                        Label("Osprey is up to date", systemImage: "checkmark.circle.fill").foregroundStyle(Theme.success)
                    }
                }
                HStack {
                    Button("Check Now") {
                        busy = true
                        Task { info = await model.perform("Update check failed", { try await model.engine.checkForUpdates() }); busy = false }
                    }
                    if info?.available == true, info?.signatureValid != false {
                        Button("Download Update") {
                            busy = true
                            Task { downloadedPath = await model.perform("Update download failed", { try await model.engine.downloadUpdate() }); busy = false }
                        }
                    }
                    if let path = downloadedPath {
                        Button("Install and Relaunch") {
                            if let error = UpdateInstaller.install(archive: path) { model.toast(.error, "Couldn't install the update", detail: error) }
                        }
                        .ospreyGlassButton(prominent: true)
                    }
                    if busy { ProgressView().controlSize(.small) }
                }
            }
        }
    }
}

/// Installs a verified update archive (the engine checked its Ed25519 signature): unpacks it next
/// to the running app, keeps the previous bundle in the Trash for rollback, and relaunches.
enum UpdateInstaller {
    @MainActor
    static func install(archive: String) -> String? {
        let fm = FileManager.default
        let current = Bundle.main.bundleURL
        let staging = fm.temporaryDirectory.appendingPathComponent("osprey-update-\(UUID().uuidString)")
        do {
            try fm.createDirectory(at: staging, withIntermediateDirectories: true)
            let ditto = Process()
            ditto.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
            ditto.arguments = ["-x", "-k", archive, staging.path]
            try ditto.run()
            ditto.waitUntilExit()
            guard ditto.terminationStatus == 0 else { return "The update archive couldn't be unpacked." }
            guard let newApp = try fm.contentsOfDirectory(at: staging, includingPropertiesForKeys: nil).first(where: { $0.pathExtension == "app" }) else {
                return "The update archive doesn't contain Osprey.app."
            }
            var trashed: NSURL?
            try fm.trashItem(at: current, resultingItemURL: &trashed)
            try fm.moveItem(at: newApp, to: current)
            let open = Process()
            open.executableURL = URL(fileURLWithPath: "/usr/bin/open")
            open.arguments = ["-n", current.path]
            try open.run()
            NSApp.terminate(nil)
            return nil
        } catch {
            return error.localizedDescription
        }
    }
}

struct AdvancedPane: View {
    @Environment(AppModel.self) private var model
    @ViewState private var logs: [String] = []
    @ViewState private var level = "info"

    var body: some View {
        Form {
            Section("Engine") {
                LabeledContent("Version", value: "\(model.info.version) (\(model.info.build))")
                LabeledContent("Platform", value: "\(model.info.os) \(model.info.arch)")
                LabeledContent("Data folder", value: model.info.dataDir.abbreviatedPath)
                LabeledContent("Local API port", value: model.info.localApiPort == 0 ? "—" : "\(model.info.localApiPort)")
                LabeledContent("ffmpeg", value: model.info.ffmpegAvailable ? L10n.tr("Available") : L10n.tr("Not found (optional)"))
                HStack {
                    Button("Show Data Folder") { NSWorkspace.shared.open(URL(fileURLWithPath: model.info.dataDir)) }
                    Button("Reload State") { Task { await model.reloadSnapshot() } }
                }
            }
            Section("Recent log") {
                Picker("Level", selection: $level) {
                    ForEach(["error", "warn", "info", "debug"], id: \.self) { Text($0.capitalized).tag($0) }
                }
                .pickerStyle(.segmented)
                ScrollView {
                    Text(logs.isEmpty ? L10n.tr("No log lines.") : logs.joined(separator: "\n"))
                        .font(.system(size: 10, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(height: 180)
                Button("Copy Log") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(logs.joined(separator: "\n"), forType: .string)
                }
            }
        }
        .task(id: level) { logs = (try? await model.engine.recentLogs(limit: 300, level: level)) ?? [] }
    }
}
