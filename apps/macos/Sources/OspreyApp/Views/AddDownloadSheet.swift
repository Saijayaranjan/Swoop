import AppKit
import OspreyKit
import SwiftUI
import UniformTypeIdentifiers

/// The Add sheet: detects links/magnets/torrents/metalinks as you type, probes the source, and
/// lets you pick destination, queue, category, recipe and advanced options before adding.
struct AddDownloadSheet: View {
    let prefill: AddPrefill
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @Environment(\.dismiss) private var dismiss

    @ViewState private var text = ""
    @ViewState private var torrentFile: URL?
    @ViewState private var torrentBase64: String?
    @ViewState private var metalinkFile: URL?
    @ViewState private var metalinkEntries: [MetalinkFile] = []
    @ViewState private var probe: ProbeResultData?
    @ViewState private var probing = false
    @ViewState private var probeError: String?
    @ViewState private var name = ""
    @ViewState private var directory = ""
    @ViewState private var queueId = ""
    @ViewState private var categoryId = ""
    @ViewState private var recipeId = ""
    @ViewState private var scheduleId = ""
    @ViewState private var priority: Priority = .normal
    @ViewState private var startNow = true
    @ViewState private var showAdvanced = false
    @ViewState private var options = TaskOptionsData()
    @ViewState private var connections = 0
    @ViewState private var downloadLimit = ""
    @ViewState private var uploadLimit = ""
    @ViewState private var headersText = ""
    @ViewState private var checksumText = ""
    @ViewState private var credentialId = ""
    @ViewState private var newCredUser = ""
    @ViewState private var newCredSecret = ""
    @ViewState private var files: [TorrentFileData] = []
    @ViewState private var variantId = ""
    @ViewState private var duplicateChoice: ConflictPolicy = .rename
    @ViewState private var adding = false
    @ViewState private var probeTask: Task<Void, Never>?

    private var links: [String] { LinkDetector.links(in: text) }
    private var isBatch: Bool { (links.count > 1 && torrentFile == nil && metalinkFile == nil) || metalinkEntries.count > 1 }
    private var canAdd: Bool { !adding && (torrentBase64 != nil || !metalinkEntries.isEmpty || !links.isEmpty) }

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    header
                    sourceField
                    if isBatch { batchSummary } else { probeSection }
                    destinationSection
                    if !files.isEmpty { torrentFilesSection }
                    if let probe, !probe.mediaVariants.isEmpty { variantSection(probe) }
                    advancedSection
                }
                .padding(26)
            }
            .scrollIndicators(.never)
            footer
        }
        .background {
            ZStack(alignment: .top) {
                Theme.panel
                LinearGradient(colors: [Theme.washEnd.opacity(0.9), Theme.washStart.opacity(0)], startPoint: .top, endPoint: .bottom)
                    .frame(height: 180)
            }
            .ignoresSafeArea()
        }
        .frame(width: 660, height: 720)
        .onAppear(perform: setup)
        .onChange(of: text) { _, _ in scheduleProbe() }
    }

    // MARK: sections

    private var header: some View {
        HStack(spacing: 14) {
            Image(systemName: kindSymbol)
                .font(.system(size: 24, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 56, height: 56)
                .background(LinearGradient(colors: [Color(red: 0.25, green: 0.62, blue: 1.0), Color(red: 0.16, green: 0.36, blue: 0.93)],
                                           startPoint: .topLeading, endPoint: .bottomTrailing),
                            in: RoundedRectangle(cornerRadius: 17, style: .continuous))
                .shadow(color: Theme.blue.opacity(0.35), radius: 10, y: 4)
                .contentTransition(.symbolEffect(.replace))
            VStack(alignment: .leading, spacing: 3) {
                Text("Add Download").font(.system(size: 26, weight: .bold))
                Text(detectedKindLabel).font(.system(size: 13)).foregroundStyle(.secondary)
            }
            Spacer()
        }
    }

    private var sourceField: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack(alignment: .topLeading) {
                TextEditor(text: $text)
                    .font(.system(size: 13, design: .monospaced))
                    .scrollContentBackground(.hidden)
                    .frame(minHeight: 70, maxHeight: 120)
                    .padding(10)
                if text.isEmpty {
                    Text("Paste links, a magnet link, or choose a .torrent / .metalink file")
                        .foregroundStyle(.tertiary)
                        .padding(.horizontal, 15)
                        .padding(.vertical, 10)
                        .allowsHitTesting(false)
                }
            }
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Theme.hairline, lineWidth: 1))
            .shadow(color: .black.opacity(0.04), radius: 8, y: 3)
            HStack {
                Button { paste() } label: { Label("Paste", systemImage: "doc.on.clipboard") }
                    .ospreyGlassButton()
                Button { chooseFile() } label: { Label("Choose File…", systemImage: "folder") }
                    .ospreyGlassButton()
                if let f = torrentFile ?? metalinkFile {
                    Label(f.lastPathComponent, systemImage: "doc.fill").font(.caption).foregroundStyle(.secondary)
                    Button { clearFile() } label: { Image(systemName: "xmark.circle.fill") }.buttonStyle(.plain).foregroundStyle(.secondary)
                }
                Spacer()
                if probing { ProgressView().controlSize(.small) }
            }
            .controlSize(.small)
        }
    }

    private var batchSummary: some View {
        let shown = metalinkEntries.isEmpty ? links : metalinkEntries.map { "\($0.name) — \($0.urls.count) mirror(s)" }
        return GlassCard("\(shown.count) downloads", symbol: "list.bullet") {
            ForEach(shown.prefix(8), id: \.self) { l in
                Text(l).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
            }
            if shown.count > 8 { Text("and \(shown.count - 8) more").font(.caption).foregroundStyle(.secondary) }
        }
    }

    @ViewBuilder
    private var probeSection: some View {
        if let probeError {
            Label(probeError, systemImage: "exclamationmark.triangle.fill")
                .font(.callout)
                .foregroundStyle(Theme.warning)
        }
        if let p = probe {
            GlassCard {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 12) {
                        FileBadge(name: p.suggestedName, kind: p.kind, size: 40)
                        TextField("Name", text: $name).textFieldStyle(.plain).font(.system(size: 16, weight: .semibold))
                    }
                    HStack(spacing: 8) {
                        chip("externaldrive", Fmt.bytes(p.size))
                        chip("tag", p.mime ?? p.kind.label)
                        if let r = p.resumable {
                            chip(r ? "arrow.triangle.2.circlepath" : "exclamationmark.arrow.triangle.2.circlepath", r ? "Resumable" : "Can't resume")
                        }
                        if let free = p.freeSpace { chip("internaldrive", "\(Fmt.bytes(free)) free") }
                    }
                    if let size = p.size, let free = p.freeSpace, size > free {
                        Label("Not enough free space on the destination disk.", systemImage: "exclamationmark.triangle.fill")
                            .font(.caption).foregroundStyle(Theme.danger)
                    }
                    ForEach(p.warnings, id: \.self) { w in
                        Label(w, systemImage: "exclamationmark.circle").font(.caption).foregroundStyle(Theme.warning)
                    }
                    if !p.applicableRules.isEmpty {
                        Label("Rules: \(p.applicableRules.joined(separator: ", "))", systemImage: "arrow.triangle.branch")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            if let dup = p.duplicate { duplicateCard(dup) }
        }
    }

    private func duplicateCard(_ dup: DuplicateData) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("You already have this file", systemImage: "doc.on.doc.fill")
                .font(.callout.weight(.semibold))
                .foregroundStyle(Theme.warning)
            Text(duplicateDescription(dup)).font(.caption).foregroundStyle(.secondary)
            Picker("", selection: $duplicateChoice) {
                Text("Keep Both (Rename)").tag(ConflictPolicy.rename)
                Text("Replace").tag(ConflictPolicy.replace)
                Text("Skip").tag(ConflictPolicy.skip)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            if let path = dup.existingPath {
                Button("Show Existing File") { Finder.reveal([path]) }.buttonStyle(.link).font(.caption)
            }
        }
        .padding(14)
        .background(Theme.warning.opacity(0.1), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).stroke(Theme.warning.opacity(0.3), lineWidth: 0.5))
    }

    private func duplicateDescription(_ d: DuplicateData) -> String {
        var s = "Matched by \(d.matchedBy.replacingOccurrences(of: "_", with: " "))"
        if let p = d.existingPath { s += " · \(p.abbreviatedPath)" }
        if let size = d.existingSize { s += " · \(Fmt.bytes(size))" }
        if let at = d.existingCompletedAt { s += " · \(Fmt.relative(at))" }
        return s
    }

    private var destinationSection: some View {
        GlassCard("Destination", symbol: "folder") {
        Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 10) {
            GridRow {
                Text("Save to").foregroundStyle(.secondary)
                HStack {
                    Text(directory.isEmpty ? "Default folder" : directory.abbreviatedPath)
                        .lineLimit(1).truncationMode(.middle)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Button("Choose…") { chooseDirectory() }.controlSize(.small)
                }
            }
            GridRow {
                Text("Queue").foregroundStyle(.secondary)
                Picker("", selection: $queueId) {
                    ForEach(model.queues) { q in Label(q.name, systemImage: q.icon).tag(q.id) }
                }
                .labelsHidden()
            }
            GridRow {
                Text("Category").foregroundStyle(.secondary)
                Picker("", selection: $categoryId) {
                    Text("Automatic").tag("")
                    ForEach(model.categories) { c in Label(c.name, systemImage: c.icon).tag(c.id) }
                }
                .labelsHidden()
            }
            if !model.recipes.isEmpty {
                GridRow {
                    Text("Recipe").foregroundStyle(.secondary)
                    Picker("", selection: $recipeId) {
                        Text("None").tag("")
                        ForEach(model.recipes) { r in Label(r.name, systemImage: r.icon).tag(r.id) }
                    }
                    .labelsHidden()
                }
            }
            GridRow {
                Text("Start").foregroundStyle(.secondary)
                Picker("", selection: $startNow) {
                    Text("Now").tag(true)
                    Text("Later (paused)").tag(false)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(width: 240)
            }
        }
        .font(.callout)
        }
    }

    private var torrentFilesSection: some View {
        GlassCard("Files", symbol: "doc.on.doc") {
            HStack {
                Text("\(files.filter(\.selected).count) of \(files.count) selected · \(Fmt.bytes(files.filter(\.selected).reduce(0) { $0 + $1.size }))")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("All") { files = files.map { var f = $0; f.selected = true; return f } }.buttonStyle(.borderless)
                Button("None") { files = files.map { var f = $0; f.selected = false; return f } }.buttonStyle(.borderless)
            }
            .font(.caption)
            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach($files) { $f in
                        Toggle(isOn: $f.selected) {
                            HStack {
                                Text(f.path).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                Text(Fmt.bytes(f.size)).monospacedDigit().foregroundStyle(.secondary)
                            }
                            .font(.caption)
                        }
                        .toggleStyle(.checkbox)
                    }
                }
            }
            .frame(maxHeight: 180)
        }
    }

    private func variantSection(_ p: ProbeResultData) -> some View {
        GlassCard("Quality", symbol: "film") {
            Picker("", selection: $variantId) {
                Text("Best available").tag("")
                ForEach(p.mediaVariants) { v in
                    Text([v.label, v.resolution, v.estimatedSize.map { Fmt.bytes($0) }].compactMap { $0 }.joined(separator: " · ")).tag(v.id)
                }
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
        }
    }

    private var advancedSection: some View {
        GlassCard {
        DisclosureGroup(isExpanded: $showAdvanced) {
            Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 10) {
                GridRow {
                    Text("Connections").foregroundStyle(.secondary)
                    Stepper(value: $connections, in: 0...64) {
                        Text(connections == 0 ? "Automatic" : "\(connections)").monospacedDigit()
                    }
                }
                GridRow {
                    Text("Speed limit").foregroundStyle(.secondary)
                    HStack {
                        TextField("↓ unlimited", text: $downloadLimit).frame(width: 110)
                        TextField("↑ unlimited", text: $uploadLimit).frame(width: 110)
                        Text("e.g. 2 MB").font(.caption).foregroundStyle(.secondary)
                    }
                    .textFieldStyle(.roundedBorder)
                }
                GridRow {
                    Text("Priority").foregroundStyle(.secondary)
                    Picker("", selection: $priority) {
                        ForEach(Priority.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                }
                GridRow {
                    Text("Schedule").foregroundStyle(.secondary)
                    Picker("", selection: $scheduleId) {
                        Text("None").tag("")
                        ForEach(model.schedules) { s in Text(s.name).tag(s.id) }
                    }
                    .labelsHidden()
                }
                GridRow {
                    Text("Checksum").foregroundStyle(.secondary)
                    TextField("sha256:… (verified when complete)", text: $checksumText)
                        .textFieldStyle(.roundedBorder)
                        .font(.caption.monospaced())
                }
                GridRow {
                    Text("Referer").foregroundStyle(.secondary)
                    TextField("https://", text: Binding(get: { options.referer ?? "" }, set: { options.referer = $0.isEmpty ? nil : $0 }))
                        .textFieldStyle(.roundedBorder)
                }
                GridRow {
                    Text("User agent").foregroundStyle(.secondary)
                    TextField("Default", text: Binding(get: { options.userAgent ?? "" }, set: { options.userAgent = $0.isEmpty ? nil : $0 }))
                        .textFieldStyle(.roundedBorder)
                }
                GridRow {
                    Text("Headers").foregroundStyle(.secondary)
                    TextEditor(text: $headersText)
                        .font(.caption.monospaced())
                        .frame(height: 54)
                        .scrollContentBackground(.hidden)
                        .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))
                        .overlay(alignment: .topLeading) {
                            if headersText.isEmpty {
                                Text("Name: value, one per line").font(.caption).foregroundStyle(.tertiary).padding(4).allowsHitTesting(false)
                            }
                        }
                }
                GridRow {
                    Text("Cookies").foregroundStyle(.secondary)
                    TextField("name=value; …", text: Binding(get: { options.cookies ?? "" }, set: { options.cookies = $0.isEmpty ? nil : $0 }))
                        .textFieldStyle(.roundedBorder)
                }
                GridRow {
                    Text("Sign in").foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 6) {
                        Picker("", selection: $credentialId) {
                            Text("None").tag("")
                            ForEach(model.credentials) { c in Text(c.username.map { "\(c.name) (\($0))" } ?? c.name).tag(c.id) }
                            Text("New credential…").tag("__new")
                        }
                        .labelsHidden()
                        if credentialId == "__new" {
                            HStack {
                                TextField("User name", text: $newCredUser).textFieldStyle(.roundedBorder)
                                SecureField("Password", text: $newCredSecret).textFieldStyle(.roundedBorder)
                            }
                            Text("Stored in your Keychain.").font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                }
                GridRow {
                    Text("Proxy").foregroundStyle(.secondary)
                    Picker("", selection: Binding(get: { options.directConnection ? "__direct" : (options.proxyId ?? "") },
                                                  set: { v in
                                                      options.directConnection = v == "__direct"
                                                      options.proxyId = (v.isEmpty || v == "__direct") ? nil : v
                                                  })) {
                        Text("Use global setting").tag("")
                        Text("Direct connection").tag("__direct")
                        ForEach(proxies, id: \.id) { p in Text(p.name).tag(p.id) }
                    }
                    .labelsHidden()
                }
                if probe?.kind.isTorrent == true || torrentBase64 != nil || text.lowercased().hasPrefix("magnet:") {
                    GridRow {
                        Text("Torrent").foregroundStyle(.secondary)
                        Toggle("Download pieces in order (for previewing)", isOn: Binding(get: { options.sequential ?? false }, set: { options.sequential = $0 }))
                    }
                }
                GridRow {
                    Text("When done").foregroundStyle(.secondary)
                    Toggle("Open the file", isOn: $options.openWhenDone)
                }
            }
            .font(.callout)
            .padding(.top, 8)
        } label: {
            CardLabel("Advanced", symbol: "slider.horizontal.3")
        }
        }
    }

    private var footer: some View {
        HStack {
            if isBatch {
                Text("\(max(links.count, metalinkEntries.count)) downloads will be added").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Cancel") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .buttonStyle(SecondaryCapsuleStyle())
            Button {
                Task { await add() }
            } label: {
                if adding {
                    ProgressView().controlSize(.small).tint(.white)
                } else {
                    Label(isBatch ? "Add All" : "Download", systemImage: "arrow.down")
                }
            }
            .keyboardShortcut(.defaultAction)
            .buttonStyle(ProminentCapsuleStyle())
            .disabled(!canAdd || (probe?.duplicate != nil && duplicateChoice == .skip))
        }
        .padding(.horizontal, 22)
        .padding(.vertical, 14)
        .background(Theme.card.opacity(0.6))
        .overlay(alignment: .top) { Rectangle().fill(Theme.hairline).frame(height: 1) }
    }

    private func chip(_ symbol: String, _ text: String) -> some View {
        Label(text, systemImage: symbol)
            .font(.system(size: 12, weight: .medium).monospacedDigit())
            .foregroundStyle(.secondary)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Theme.well, in: Capsule())
    }

    // MARK: detection

    private var kindSymbol: String {
        if torrentFile != nil || text.lowercased().hasPrefix("magnet:") { return "point.3.connected.trianglepath.dotted" }
        if metalinkFile != nil { return "link.circle" }
        if isBatch { return "square.stack.3d.down.forward" }
        if let p = probe { return Theme.fileSymbol(name: p.suggestedName, kind: p.kind) }
        return "arrow.down.circle"
    }

    private var detectedKindLabel: String {
        if torrentFile != nil { return L10n.tr("Torrent file") }
        if metalinkFile != nil { return L10n.tr("Metalink") }
        if isBatch { return String(format: L10n.tr("%d links"), max(links.count, metalinkEntries.count)) }
        let t = text.lowercased()
        if t.hasPrefix("magnet:") { return L10n.tr("Magnet link") }
        if t.contains(".m3u8") { return L10n.tr("HLS stream") }
        if t.hasPrefix("ftp") { return L10n.tr("FTP") }
        if let p = probe { return p.kind.label }
        return links.isEmpty ? L10n.tr("Paste a link to begin") : L10n.tr("Link")
    }

    private var proxies: [(id: String, name: String)] {
        (model.settings["network.proxies"]?.array ?? []).compactMap { p in
            guard let id = p["id"]?.string else { return nil }
            return (id, p["name"]?.string ?? id)
        }
    }

    // MARK: actions

    private func setup() {
        text = prefill.text
        queueId = model.queues.first?.id ?? "queue-default"
        if prefill.autoPaste { paste() }
        if let f = prefill.torrentFile { loadTorrent(f) }
        if let m = prefill.metalinkFile { loadMetalink(m) }
        Task {
            await model.load("recipes")
            await model.load("schedules")
            await model.load("credentials")
        }
        if !text.isEmpty { scheduleProbe() }
    }

    private func paste() {
        if let urls = NSPasteboard.general.readObjects(forClasses: [NSURL.self]) as? [URL],
           let file = urls.first(where: { $0.isFileURL && $0.pathExtension.lowercased() == "torrent" }) {
            loadTorrent(file)
        } else if let s = NSPasteboard.general.string(forType: .string) {
            text = s.trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = ["torrent", "metalink", "meta4"].compactMap { UTType(filenameExtension: $0) }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        if url.pathExtension.lowercased() == "torrent" { loadTorrent(url) } else { loadMetalink(url) }
    }

    private func clearFile() {
        torrentFile = nil
        torrentBase64 = nil
        metalinkFile = nil
        metalinkEntries = []
        files = []
        probe = nil
    }

    private func loadTorrent(_ url: URL) {
        let access = url.startAccessingSecurityScopedResource()
        defer { if access { url.stopAccessingSecurityScopedResource() } }
        guard let data = try? Data(contentsOf: url) else {
            probeError = L10n.tr("The torrent file couldn't be read.")
            return
        }
        torrentFile = url
        torrentBase64 = data.base64EncodedString()
        scheduleProbe()
    }

    private func loadMetalink(_ url: URL) {
        let access = url.startAccessingSecurityScopedResource()
        defer { if access { url.stopAccessingSecurityScopedResource() } }
        guard let data = try? Data(contentsOf: url) else {
            probeError = L10n.tr("The Metalink file couldn't be read.")
            return
        }
        let entries = MetalinkParser.parse(data)
        guard !entries.isEmpty else {
            probeError = L10n.tr("The Metalink file doesn't list any downloadable mirrors.")
            return
        }
        metalinkFile = url
        metalinkEntries = entries
        if entries.count == 1 { name = (entries[0].name as NSString).lastPathComponent }
        scheduleProbe()
    }

    private func chooseDirectory() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.directoryURL = URL(fileURLWithPath: directory.isEmpty ? model.settings.downloadDirectory : directory)
        if panel.runModal() == .OK, let url = panel.url { directory = url.path }
    }

    private func scheduleProbe() {
        probeTask?.cancel()
        guard !isBatch, torrentBase64 != nil || metalinkEntries.count == 1 || links.count == 1 else {
            probe = nil
            probeError = nil
            return
        }
        probeTask = Task {
            try? await Task.sleep(nanoseconds: 450_000_000)
            guard !Task.isCancelled else { return }
            probing = true
            defer { probing = false }
            do {
                let p = try await model.engine.probe(baseRequest())
                guard !Task.isCancelled else { return }
                probe = p
                probeError = nil
                if name.isEmpty || name == probe?.suggestedName { name = p.suggestedName }
                if directory.isEmpty { directory = p.suggestedDirectory }
                queueId = p.suggestedQueue.isEmpty ? queueId : p.suggestedQueue
                if categoryId.isEmpty, let c = p.suggestedCategory { categoryId = c }
                if !p.torrentFiles.isEmpty { files = p.torrentFiles }
            } catch {
                guard !Task.isCancelled else { return }
                probe = nil
                probeError = error.localizedDescription
            }
        }
    }

    /// The source-only request used for probing.
    private func baseRequest(for link: String? = nil) -> NewTaskRequestData {
        var r = NewTaskRequestData()
        if let b = torrentBase64 {
            r.torrentBase64 = b
        } else if let entry = metalinkEntries.first(where: { $0.urls.first == link }) ?? (link == nil ? metalinkEntries.first : nil) {
            r = MetalinkParser.requests(from: [entry])[0]
        } else if let l = link ?? links.first {
            let lower = l.lowercased()
            let path = URL(string: l)?.path.lowercased() ?? lower
            if lower.hasPrefix("magnet:") { r.magnet = l }
            else if path.hasSuffix(".metalink") || path.hasSuffix(".meta4") { r.metalinkUrl = l }
            else if path.hasSuffix(".m3u8") { r.hlsPlaylistUrl = l }
            else { r.url = l }
        }
        r.refererPage = prefill.refererPage
        r.origin = "app"
        return r
    }

    private func fullRequest(for link: String? = nil) async -> NewTaskRequestData? {
        var r = baseRequest(for: link)
        if !isBatch, !name.isEmpty, name != probe?.suggestedName { r.name = name }
        if !directory.isEmpty { r.directory = directory }
        r.queueId = queueId.isEmpty ? nil : queueId
        r.categoryId = categoryId.isEmpty ? nil : categoryId
        r.scheduleId = scheduleId.isEmpty ? nil : scheduleId
        r.priority = priority
        r.start = startNow
        var o = options
        o.maxConnections = connections == 0 ? nil : UInt8(connections)
        o.downloadLimit = Fmt.parseBytes(downloadLimit)
        o.uploadLimit = Fmt.parseBytes(uploadLimit)
        o.headers = parseHeaders(headersText)
        let sum = checksumText.trimmingCharacters(in: .whitespaces)
        o.checksum = sum.isEmpty ? r.options.checksum : sum
        o.mediaVariant = variantId.isEmpty ? nil : variantId
        if probe?.duplicate != nil { o.conflictPolicy = duplicateChoice }
        if credentialId == "__new" {
            guard !newCredSecret.isEmpty else {
                model.toast(.error, "Enter a password for the new credential")
                return nil
            }
            let host = URL(string: links.first ?? "")?.host ?? "Download"
            guard let id = await model.perform("Couldn't save the credential", {
                try await model.engine.storeCredential(name: host, username: newCredUser.isEmpty ? nil : newCredUser, secret: newCredSecret)
            }) else { return nil }
            o.credentialId = id
        } else {
            o.credentialId = credentialId.isEmpty ? nil : credentialId
        }
        r.options = o
        if !files.isEmpty, files.contains(where: { !$0.selected }) {
            r.selectedFiles = files.filter(\.selected).map(\.index)
        }
        return r
    }

    private func parseHeaders(_ text: String) -> [String: String] {
        var h: [String: String] = [:]
        for line in text.split(whereSeparator: \.isNewline) {
            let parts = line.split(separator: ":", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            if parts.count == 2, !parts[0].isEmpty { h[parts[0]] = parts[1] }
        }
        return h
    }

    private func add() async {
        adding = true
        defer { adding = false }
        if isBatch {
            let sources = metalinkEntries.isEmpty ? links : metalinkEntries.compactMap(\.urls.first)
            var requests: [NewTaskRequestData] = []
            for l in sources { if let r = await fullRequest(for: l) { requests.append(r) } }
            guard requests.count == sources.count else { return }
            if let results = await model.perform("Couldn't add downloads", { try await model.engine.addTasks(requests) }) {
                model.toast(.success, "Added \(results.count) downloads")
                dismiss()
            }
            return
        }
        guard let req = await fullRequest() else { return }
        let result: AddResultData?
        if !recipeId.isEmpty {
            result = await model.perform("Couldn't add download") { try await model.engine.applyRecipe(recipeId, req) }
        } else {
            result = await model.perform("Couldn't add download") { try await model.engine.addTask(req) }
        }
        guard let result else { return }
        if result.duplicate != nil, probe?.duplicate == nil {
            // The engine found a duplicate the probe didn't: apply the user's policy now.
            await model.perform("Couldn't resolve duplicate") { try await model.engine.resolveDuplicate(result.row.id, policy: duplicateChoice) }
        }
        model.toast(.success, "Added \(result.row.name)")
        ui.selection = [result.row.id]
        dismiss()
    }
}
