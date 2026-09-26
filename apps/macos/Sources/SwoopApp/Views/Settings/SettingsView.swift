import AppKit
import SwoopKit
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

/// Sidebar groups for the Settings window.
enum SettingsGroup: String, CaseIterable, Identifiable {
    case general, organise, network, integrations, privacy
    var id: String { rawValue }
    var title: String {
        switch self {
        case .general: return "General"
        case .organise: return "Downloads & Organisation"
        case .network: return "Network"
        case .integrations: return "Integrations"
        case .privacy: return "Privacy & Advanced"
        }
    }
    var panes: [SettingsPane] {
        switch self {
        case .general: return [.general, .notifications, .updates]
        case .organise: return [.downloads, .queues, .categories, .rules, .automations, .recipes]
        case .network: return [.network, .bandwidth, .torrents]
        case .integrations: return [.browser, .remote, .devices, .automation]
        case .privacy: return [.privacy, .advanced]
        }
    }
}

extension SettingsPane {
    var subtitle: String {
        switch self {
        case .general: return "Startup, appearance and language."
        case .downloads: return "Where files go and how they're checked."
        case .queues: return "Lanes that decide how many downloads run at once."
        case .categories: return "Sort files by type into their own folders."
        case .rules: return "Route new downloads by site, type, size or name."
        case .automations: return "React to download events with actions."
        case .recipes: return "One-click bundles of folder, queue, tags and actions."
        case .network: return "Connections, retries, timeouts and proxies."
        case .bandwidth: return "Speed modes, limits and how much runs in parallel."
        case .torrents: return "Peers, seeding and trackers."
        case .browser: return "Catch downloads straight from your browser."
        case .notifications: return "Choose what Swoop tells you about."
        case .remote: return "Control Swoop from other devices."
        case .devices: return "Pair and manage phones and computers."
        case .automation: return "Extensions that add abilities to Swoop."
        case .privacy: return "History, logs and saved sign-ins."
        case .updates: return "Stay on the latest version."
        case .advanced: return "Engine details and diagnostics."
        }
    }
}

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("settingsPane") private var paneRaw = SettingsPane.general.rawValue
    @Namespace private var selectionNS
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let pane = SettingsPane(rawValue: paneRaw) ?? .general
        let shape = RoundedRectangle(cornerRadius: 22, style: .continuous)
        ZStack {
            WindowWash()
            HStack(spacing: 0) {
                sidebar(pane)
                    .frame(width: 236)
                VStack(alignment: .leading, spacing: 0) {
                    HStack(spacing: 14) {
                        Image(systemName: pane.symbol)
                            .font(.system(size: 21, weight: .semibold))
                            .foregroundStyle(.white)
                            .frame(width: 46, height: 46)
                            .background(LinearGradient(colors: [Color(red: 0.25, green: 0.62, blue: 1.0), Color(red: 0.16, green: 0.36, blue: 0.93)],
                                                       startPoint: .topLeading, endPoint: .bottomTrailing),
                                        in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            .shadow(color: Theme.blue.opacity(0.3), radius: 8, y: 3)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(LocalizedStringKey(pane.title)).font(.system(size: 26, weight: .bold))
                            Text(LocalizedStringKey(pane.subtitle)).font(.system(size: 13)).foregroundStyle(.secondary)
                        }
                    }
                    .padding(.horizontal, 28)
                    .padding(.top, 24)
                    .padding(.bottom, 6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background { Color.clear.contentShape(Rectangle()).windowDraggable() }
                    paneView(pane)
                        .scrollContentBackground(.hidden)
                        .scrollIndicators(.never)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .id(pane)
                }
                .background(Theme.panel, in: shape)
                .clipShape(shape)
                .overlay(shape.strokeBorder(Theme.hairline, lineWidth: 1))
                .shadow(color: Color(red: 0.05, green: 0.1, blue: 0.3).opacity(scheme == .dark ? 0.45 : 0.10), radius: 24, y: 10)
                .padding(.vertical, 10)
                .padding(.trailing, 10)
            }
        }
        .ignoresSafeArea()
        .frame(width: 1000, height: 700)
        .transparentTitleBar()
    }

    private func sidebar(_ current: SettingsPane) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Settings")
                .font(.system(size: 22, weight: .bold, design: .rounded))
                .padding(.leading, 22)
                .padding(.top, 46)
                .padding(.bottom, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background { Color.clear.contentShape(Rectangle()).windowDraggable() }
            ScrollView {
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(SettingsGroup.allCases) { group in
                        Text(L10n.tr(group.title).uppercased())
                            .font(.system(size: 10, weight: .semibold))
                            .tracking(1.1)
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 12)
                            .padding(.top, group == .general ? 8 : 16)
                            .padding(.bottom, 4)
                        ForEach(group.panes) { p in
                            SettingsSidebarRow(pane: p, selected: current == p, namespace: selectionNS) {
                                withAnimation(.spring(response: 0.34, dampingFraction: 0.85)) { paneRaw = p.rawValue }
                            }
                        }
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 16)
            }
            .scrollIndicators(.never)
        }
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

struct SettingsSidebarRow: View {
    let pane: SettingsPane
    let selected: Bool
    let namespace: Namespace.ID
    let action: () -> Void
    @ViewState private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: pane.symbol)
                    .font(.system(size: 11.5, weight: .semibold))
                    .foregroundStyle(selected ? .white : Theme.blue)
                    .frame(width: 26, height: 26)
                    .background(selected ? AnyShapeStyle(Theme.blue.gradient) : AnyShapeStyle(Theme.blue.opacity(0.12)),
                                in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                Text(LocalizedStringKey(pane.title))
                    .font(.system(size: 14, weight: selected ? .semibold : .regular))
                    .foregroundStyle(.primary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 8)
            .frame(height: 36)
            .background {
                if selected {
                    Color.clear
                        .swoopGlass(.regular, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                        .matchedGeometryEffect(id: "settings-selection", in: namespace)
                } else if hovering {
                    RoundedRectangle(cornerRadius: 11, style: .continuous).fill(Color.primary.opacity(0.05))
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .accessibilityAddTraits(selected ? .isSelected : [])
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
                Toggle("Open Swoop at login", isOn: Binding(get: { LoginItem.isEnabled }, set: { on in
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
                Text("Takes effect the next time Swoop opens.").font(.caption).foregroundStyle(.secondary)
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
                StepperRow("Connections per download", value: b.int("network.connections_per_task", 8), in: 1...64, display: "\(model.settings.int("network.connections_per_task", default: 8))")
                StepperRow("Per server", value: b.int("network.max_connections_per_host", 16), in: 1...128, display: "\(model.settings.int("network.max_connections_per_host", default: 16))")
                StepperRow("In total", value: b.int("network.max_total_connections", 64), in: 1...512, display: "\(model.settings.int("network.max_total_connections", default: 64))")
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
                StepperRow("Retries", value: b.int("network.max_retries", 8), in: 0...50, display: "\(model.settings.int("network.max_retries", default: 8))")
                StepperRow("Connect timeout", value: b.int("network.connect_timeout_seconds", 20), in: 5...120, step: 5, display: "\(model.settings.int("network.connect_timeout_seconds", default: 20)) s")
                StepperRow("Read timeout", value: b.int("network.read_timeout_seconds", 45), in: 5...300, step: 5, display: "\(model.settings.int("network.read_timeout_seconds", default: 45)) s")
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
                    ForEach(TrafficMode.pickerModes, id: \.self) { Text(LocalizedStringKey($0.label)).tag($0) }
                }
                .pickerStyle(.segmented)
                Text(modeHelp).font(.caption).foregroundStyle(.secondary)
                LabeledContent("Custom download limit") { BytesField(bytes: b.bytes("bandwidth.custom_download_limit")) }
                LabeledContent("Custom upload limit") { BytesField(bytes: b.bytes("bandwidth.custom_upload_limit")) }
                let cap = UInt64(max(0, model.settings.double("bandwidth.measured_capacity")))
                LabeledContent("Measured connection", value: cap > 0 ? Fmt.speed(cap) : L10n.tr("Not measured yet"))
            }
            Section("Concurrency") {
                StepperRow("Active downloads", value: b.int("bandwidth.max_active_downloads", 5), in: 1...50, display: "\(model.settings.int("bandwidth.max_active_downloads", default: 5))")
                StepperRow("Active torrents", value: b.int("bandwidth.max_active_torrents", 5), in: 1...50, display: "\(model.settings.int("bandwidth.max_active_torrents", default: 5))")
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
                StepperRow("Peers per torrent", value: b.int("torrent.max_peers_per_torrent", 120), in: 10...1000, step: 10, display: "\(model.settings.int("torrent.max_peers_per_torrent", default: 120))")
                Toggle("Download pieces in order by default", isOn: b.bool("torrent.sequential_by_default"))
            }
            Section("Seeding") {
                Toggle("Seed after downloading", isOn: b.bool("torrent.seed_when_complete", true))
                LabeledContent("Stop at ratio") {
                    TextField("", value: b.double("torrent.seed_ratio_limit", 2), format: .number.precision(.fractionLength(1))).frame(width: 70)
                }
                StepperRow("Stop after", value: b.int("torrent.seed_time_limit_minutes"), in: 0...10080, step: 30, display: model.settings.int("torrent.seed_time_limit_minutes") == 0 ? L10n.tr("No limit") : "\(model.settings.int("torrent.seed_time_limit_minutes")) min")
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
    @ViewState private var status: [NativeMessagingInstaller.Status] = NativeMessagingInstaller.status()
    @ViewState private var errors: [NativeMessagingInstaller.Browser: String] = [:]
    @ViewState private var working = false

    var body: some View {
        Form {
            Section {
                if status.isEmpty {
                    Text("No supported browsers found.").foregroundStyle(.secondary)
                }
                ForEach(status) { s in
                    HStack {
                        Text(s.browser.name)
                        Spacer()
                        if let e = errors[s.browser] {
                            Text(e).font(.caption).foregroundStyle(Theme.danger).lineLimit(2)
                        } else if s.connected {
                            Label("Connected", systemImage: "checkmark.circle.fill")
                                .foregroundStyle(Theme.success)
                        } else {
                            Label("Not connected", systemImage: "circle").foregroundStyle(.secondary)
                        }
                    }
                }
                Button("Reconnect Browsers") { reconnect() }.disabled(working)
            } header: {
                Text("Browser integration")
            } footer: {
                Text("Swoop connects the browser extension automatically each time it opens.")
                    .font(.caption).foregroundStyle(.secondary)
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
        .onAppear { status = NativeMessagingInstaller.status() }
    }

    private func reconnect() {
        working = true
        Task {
            let result = await Task.detached(priority: .userInitiated) { NativeMessagingInstaller.registerAll() }.value
            errors = result
            status = NativeMessagingInstaller.status()
            working = false
            if result.isEmpty { model.toast(.success, "Browsers connected") }
        }
    }
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
                Toggle("Stay quiet while Swoop is in front", isOn: b.bool("notifications.quiet_when_active"))
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
                StepperRow("Sessions expire after", value: b.int("remote.session_ttl_hours", 720), in: 1...8760, step: 24, display: "\(model.settings.int("remote.session_ttl_hours", default: 720) / 24) days")
                StepperRow("Lock out after failed attempts", value: b.int("remote.max_failed_attempts", 5), in: 1...50, display: "\(model.settings.int("remote.max_failed_attempts", default: 5))")
                StepperRow("Lockout", value: b.int("remote.lockout_minutes", 15), in: 1...1440, display: "\(model.settings.int("remote.lockout_minutes", default: 15)) min")
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
                StepperRow("Keep for", value: b.int("privacy.history_retention_days"), in: 0...3650, step: 30, display: model.settings.int("privacy.history_retention_days") == 0 ? L10n.tr("Forever") : "\(model.settings.int("privacy.history_retention_days")) days")
            }
            Section("Logs") {
                Picker("Log detail", selection: b.string("privacy.log_level", "info")) {
                    ForEach(["error", "warn", "info", "debug", "trace"], id: \.self) { Text($0.capitalized).tag($0) }
                }
                StepperRow("Keep logs for", value: b.int("privacy.log_retention_days", 14), in: 1...365, display: "\(model.settings.int("privacy.log_retention_days", default: 14)) days")
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
