import AppKit
import SwoopKit
import SwiftUI

extension String {
    var abbreviatedPath: String { (self as NSString).abbreviatingWithTildeInPath }
}

private func copyToPasteboard(_ s: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(s, forType: .string)
}

// MARK: - Overview

struct OverviewTab: View {
    let item: TaskItem
    let detail: TaskDetailData?
    @Environment(AppModel.self) private var model
    @ViewState private var limitText = ""
    @ViewState private var tagText = ""

    var body: some View {
        let d = item.data
        if d.state == .failed, let kind = d.errorKind {
            Section("Problem") {
                Text(L10n.error(kind)).foregroundStyle(Theme.danger)
                if let m = d.errorMessage, !m.isEmpty {
                    Text(m).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                }
                HStack {
                    Button("Retry") { model.act(.retry, on: [d.id]) }
                    Button("Restart from Scratch") { model.act(.restart, on: [d.id]) }
                }
            }
        }
        if !d.blockedBy.isEmpty {
            Section("Waiting") {
                ForEach(d.blockedBy, id: \.self) { r in Text(L10n.pause(r)) }
            }
        }
        Section("Details") {
            LabeledContent("Where", value: d.targetPath.abbreviatedPath)
            LabeledContent("Added", value: Fmt.date(d.createdAt))
            if let s = detail?.startedAt { LabeledContent("Started", value: Fmt.date(s)) }
            if let c = d.completedAt { LabeledContent("Completed", value: Fmt.date(c)) }
            if d.kind.isTorrent {
                LabeledContent("Peers", value: "\(d.progress.peers) (\(d.progress.seeds) seeds)")
                LabeledContent("Uploaded", value: "\(Fmt.bytes(d.progress.uploaded)) · ratio \(Fmt.ratio(d.progress.ratio))")
            }
            if let stats = detail?.stats, stats.averageSpeed > 0 {
                LabeledContent("Average speed", value: Fmt.speed(stats.averageSpeed))
            }
            if let stats = detail?.stats, stats.activeSeconds > 0 {
                LabeledContent("Time active", value: Fmt.duration(stats.activeSeconds))
            }
            if let o = detail?.origin, !o.isEmpty { LabeledContent("Added from", value: o) }
            if let n = detail?.nextRetryAt { LabeledContent("Next retry", value: Fmt.relative(n)) }
        }
        .monospacedDigit()
        Section("Options") {
            Picker("Queue", selection: Binding(get: { d.queueId }, set: { v in patch { $0.queueId = v } })) {
                ForEach(model.queues) { q in Text(q.name).tag(q.id) }
            }
            Picker("Category", selection: Binding(get: { d.categoryId ?? "" }, set: { v in patch { $0.categoryId = .some(v.isEmpty ? nil : v) } })) {
                Text("None").tag("")
                ForEach(model.categories) { c in Text(c.name).tag(c.id) }
            }
            Picker("Priority", selection: Binding(get: { d.priority }, set: { p in
                Task { await model.perform("Couldn't change priority") { try await model.engine.setTaskPriority(d.id, p) } }
            })) {
                ForEach(Priority.allCases, id: \.self) { Text(LocalizedStringKey($0.label)).tag($0) }
            }
            LabeledContent("Speed limit") {
                TextField("", text: $limitText, prompt: Text(currentLimit))
                    .multilineTextAlignment(.trailing)
                    .onSubmit(applyLimit)
            }
            if !d.kind.isTorrent {
                Stepper(value: Binding(get: { Int(detail?.options.maxConnections ?? 0) }, set: { n in
                    Task { await model.perform("Couldn't change connections") { try await model.engine.setTaskConnections(d.id, UInt8(clamping: max(1, n))) } }
                }), in: 1...64) {
                    LabeledContent("Connections", value: detail?.options.maxConnections.map { "\($0)" } ?? L10n.tr("Automatic"))
                }
            }
            LabeledContent("Tags") {
                TextField("", text: $tagText, prompt: Text(d.tags.isEmpty ? L10n.tr("Add tag") : d.tags.joined(separator: ", ")))
                    .multilineTextAlignment(.trailing)
                    .onSubmit {
                        let t = tagText.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
                        guard !t.isEmpty else { return }
                        let next = Array(Set(d.tags + t)).sorted()
                        tagText = ""
                        patch { $0.tags = next }
                    }
            }
            if !d.tags.isEmpty {
                Button("Clear Tags") { patch { $0.tags = [] } }
            }
        }
        if let urls = detail?.urls, !urls.isEmpty {
            Section("Source") {
                ForEach(urls, id: \.self) { u in
                    Text(u).font(.caption.monospaced()).lineLimit(2).truncationMode(.middle).textSelection(.enabled)
                        .contextMenu { Button("Copy Link") { copyToPasteboard(u) } }
                }
            }
        }
    }

    private var currentLimit: String {
        if let l = detail?.options.downloadLimit, l > 0 { return Fmt.speed(l) }
        return L10n.tr("Unlimited")
    }

    private func applyLimit() {
        let text = limitText.trimmingCharacters(in: .whitespaces)
        let value: UInt64 = text.isEmpty || text.lowercased().hasPrefix("unl") ? 0 : (Fmt.parseBytes(text) ?? 0)
        let id = item.id
        Task { await model.perform("Couldn't set limit") { try await model.engine.setTaskLimit(id, download: value, upload: nil) } }
        limitText = ""
    }

    private func patch(_ edit: @escaping (inout TaskPatchData) -> Void) {
        var p = TaskPatchData()
        edit(&p)
        let patch = p
        let id = item.id
        Task { await model.perform("Couldn't update download") { try await model.engine.updateTask(id, patch: patch) } }
    }
}

// MARK: - Connections

struct ConnectionsTab: View {
    let item: TaskItem
    let detail: TaskDetailData?
    let speeds: [UInt32: UInt64]
    @Environment(AppModel.self) private var model

    var body: some View {
        if let detail, !detail.segments.isEmpty {
            Section {
                SegmentMap(segments: detail.segments, total: item.progress.total ?? detail.segments.last?.end ?? 1)
                    .frame(height: 14)
                LabeledContent("Segments", value: "\(detail.segments.count)")
                LabeledContent("Open connections", value: "\(item.progress.activeConnections)")
                Button("Retry Failed Segments") { model.act(.retrySegments, on: [item.id]) }
            }
            Section("Segments") {
                ForEach(detail.segments) { s in
                    HStack(spacing: 8) {
                        Text("\(s.index + 1)").foregroundStyle(.secondary).frame(width: 22, alignment: .leading)
                        ThinProgress(fraction: s.fraction, tint: s.fraction >= 1 ? Color(nsColor: .secondaryLabelColor) : .accentColor)
                        Text(Fmt.speed(speeds[s.index] ?? 0)).frame(width: 72, alignment: .trailing)
                        if detail.urls.count > 1 {
                            Text("M\(s.sourceIndex + 1)").foregroundStyle(.secondary)
                                .help(detail.urls[Int(s.sourceIndex) % detail.urls.count])
                        }
                    }
                    .font(.caption.monospacedDigit())
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(Text("Segment \(s.index + 1), \(Fmt.percent(s.fraction))"))
                }
            }
        } else if item.kind.isTorrent {
            PeersList(taskId: item.id)
        } else {
            Section {
                Text(item.state.isActive ? "Waiting for the first connection…" : "Connection details appear while the download runs.")
                    .foregroundStyle(.secondary)
            }
        }
    }
}

struct SegmentMap: View {
    let segments: [SegmentData]
    let total: UInt64
    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                RoundedRectangle(cornerRadius: 3).fill(Color(nsColor: .quaternaryLabelColor))
                ForEach(segments) { s in
                    let x = geo.size.width * CGFloat(Double(s.start) / Double(max(total, 1)))
                    let w = geo.size.width * CGFloat(Double(s.length) / Double(max(total, 1)))
                    Rectangle().fill(Color.accentColor)
                        .frame(width: max(0, w * CGFloat(s.fraction) - 1))
                        .offset(x: x)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 3))
        }
        .accessibilityLabel(Text("Segment map"))
    }
}

struct PeersList: View {
    let taskId: String
    @Environment(AppModel.self) private var model
    @ViewState private var peers: [PeerData] = []

    var body: some View {
        Section("Peers") {
            if peers.isEmpty {
                Text("No connected peers yet.").foregroundStyle(.secondary)
            }
            ForEach(peers) { p in
                LabeledContent {
                    Text("↓ \(Fmt.speed(p.downloadSpeed))  ↑ \(Fmt.speed(p.uploadSpeed))").monospacedDigit()
                } label: {
                    Text(p.address).font(.caption.monospaced())
                    Text(p.client ?? p.flags)
                }
            }
        }
        .task(id: taskId) {
            while !Task.isCancelled {
                if let p = try? await model.engine.torrentPeers(taskId) {
                    peers = p.sorted { $0.downloadSpeed > $1.downloadSpeed }
                }
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
        }
    }
}

// MARK: - Files

struct FilesTab: View {
    let item: TaskItem
    let detail: TaskDetailData?
    @Environment(AppModel.self) private var model
    @ViewState private var edited: [TorrentFileData]?
    @ViewState private var newTrackers = ""

    var body: some View {
        if let detail {
            if !detail.torrentFiles.isEmpty {
                torrentFiles(detail)
                trackers(detail)
            } else if !detail.mediaVariants.isEmpty || detail.mediaSegmentCount != nil {
                media(detail)
            } else {
                Section {
                    LabeledContent("File", value: item.name)
                    LabeledContent("Folder", value: detail.directory.abbreviatedPath)
                    LabeledContent("Type", value: detail.mime ?? "—")
                }
            }
        }
    }

    @ViewBuilder
    private func torrentFiles(_ detail: TaskDetailData) -> some View {
        let files = edited ?? detail.torrentFiles
        Section {
            ForEach(files) { f in
                HStack(spacing: 8) {
                    Toggle(isOn: Binding(get: { f.selected }, set: { v in edited = update(files, f.index) { $0.selected = v } })) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(f.path).lineLimit(1).truncationMode(.middle)
                            Text("\(Fmt.bytes(f.downloaded)) of \(Fmt.bytes(f.size))").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                        }
                    }
                    .toggleStyle(.checkbox)
                    Spacer()
                    Picker("", selection: Binding(get: { f.priority }, set: { v in edited = update(files, f.index) { $0.priority = v } })) {
                        Text("Low").tag(UInt8(0)); Text("Normal").tag(UInt8(1)); Text("High").tag(UInt8(2))
                    }
                    .labelsHidden()
                    .fixedSize()
                    .controlSize(.small)
                }
            }
            if let edited, edited != detail.torrentFiles {
                HStack {
                    Spacer()
                    Button("Revert") { self.edited = nil }
                    Button("Apply") {
                        let files = edited
                        Task {
                            await model.perform("Couldn't change files") { try await model.engine.setTorrentFiles(item.id, files) }
                            self.edited = nil
                        }
                    }
                    .keyboardShortcut(.defaultAction)
                }
            }
        } header: {
            HStack {
                Text("\(files.filter(\.selected).count) of \(files.count) files")
                Spacer()
                Button("All") { edited = files.map { var f = $0; f.selected = true; return f } }.buttonStyle(.link)
                Button("None") { edited = files.map { var f = $0; f.selected = false; return f } }.buttonStyle(.link)
            }
        }
    }

    private func update(_ files: [TorrentFileData], _ index: UInt32, _ edit: (inout TorrentFileData) -> Void) -> [TorrentFileData] {
        files.map { f in
            guard f.index == index else { return f }
            var c = f
            edit(&c)
            return c
        }
    }

    @ViewBuilder
    private func trackers(_ detail: TaskDetailData) -> some View {
        Section("Trackers") {
            ForEach(detail.trackers) { t in
                Toggle(isOn: Binding(get: { t.enabled }, set: { v in
                    Task { await model.perform("Couldn't update tracker") { try await model.engine.setTrackerEnabled(item.id, t.url, v) } }
                })) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(t.url).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
                        Text(trackerStatus(t)).font(.caption).foregroundStyle(t.lastError == nil ? AnyShapeStyle(.secondary) : AnyShapeStyle(Theme.warning))
                    }
                }
                .toggleStyle(.checkbox)
                .contextMenu {
                    Button("Remove Tracker") { Task { await model.perform("Couldn't remove tracker") { try await model.engine.removeTracker(item.id, t.url) } } }
                }
            }
            TextField("Add trackers", text: $newTrackers, prompt: Text("udp://… (separate with spaces)"))
                .onSubmit {
                    let list = newTrackers.split(whereSeparator: { $0.isWhitespace }).map(String.init)
                    guard !list.isEmpty else { return }
                    newTrackers = ""
                    Task { await model.perform("Couldn't add trackers") { try await model.engine.addTrackers(item.id, list) } }
                }
            Button("Ask Trackers for Peers") { Task { await model.perform("Couldn't reannounce") { try await model.engine.reannounce(item.id) } } }
        }
        if let hash = detail.infoHash {
            Section {
                LabeledContent("Info hash") { Text(hash).font(.caption.monospaced()).textSelection(.enabled) }
                LabeledContent("Private", value: detail.torrentPrivate ? L10n.tr("Yes") : L10n.tr("No"))
                LabeledContent("Availability", value: String(format: "%.2f", detail.availability))
            }
        }
    }

    private func trackerStatus(_ t: TrackerData) -> String {
        if let e = t.lastError { return e }
        var parts: [String] = []
        if let s = t.seeders { parts.append("\(s) seeds") }
        if let l = t.leechers { parts.append("\(l) peers") }
        if let at = t.lastAnnounceAt { parts.append("announced \(Fmt.relative(at))") }
        return parts.isEmpty ? t.health : parts.joined(separator: " · ")
    }

    @ViewBuilder
    private func media(_ detail: TaskDetailData) -> some View {
        if let count = detail.mediaSegmentCount, count > 0 {
            Section("Stream") {
                LabeledContent("Segments", value: "\(detail.mediaSegmentsDone) of \(count)")
                ThinProgress(fraction: Double(detail.mediaSegmentsDone) / Double(count))
            }
        }
        if !detail.mediaVariants.isEmpty {
            Section("Quality") {
                ForEach(detail.mediaVariants) { v in
                    LabeledContent {
                        Text([v.resolution, v.bandwidth.map { Fmt.speed($0 / 8) }].compactMap { $0 }.joined(separator: " · "))
                            .monospacedDigit()
                    } label: {
                        HStack(spacing: 4) {
                            Text(v.label)
                            if v.id == detail.selectedVariant { Image(systemName: "checkmark").foregroundStyle(.secondary) }
                        }
                    }
                }
            }
        }
    }
}

// MARK: - Network

struct NetworkTab: View {
    let item: TaskItem
    let detail: TaskDetailData?
    @Environment(AppModel.self) private var model
    @ViewState private var mirrorsText = ""
    @ViewState private var editingMirrors = false

    var body: some View {
        if let detail {
            Section("Server") {
                LabeledContent("Final URL") { Text(detail.stats.finalUrl ?? detail.urls.first ?? "—").lineLimit(2).truncationMode(.middle).textSelection(.enabled) }
                LabeledContent("Address", value: detail.stats.remoteAddr ?? "—")
                LabeledContent("HTTP version", value: detail.stats.httpVersion ?? "—")
                LabeledContent("Server", value: detail.stats.server ?? "—")
                LabeledContent("Content type", value: detail.stats.contentType ?? detail.mime ?? "—")
                LabeledContent("Resumable", value: detail.stats.rangeSupported.map { $0 ? L10n.tr("Yes") : L10n.tr("No") } ?? L10n.tr("Unknown"))
                if let etag = detail.stats.etag { LabeledContent("ETag", value: etag) }
                if let lm = detail.stats.lastModified { LabeledContent("Last modified", value: lm) }
            }
            Section("Request") {
                LabeledContent("Proxy", value: detail.options.directConnection ? L10n.tr("Direct") : (detail.options.proxyId ?? L10n.tr("Default")))
                LabeledContent("User agent", value: detail.options.userAgent ?? L10n.tr("Default"))
                if let r = detail.options.referer { LabeledContent("Referer", value: r) }
                ForEach(detail.options.headers.sorted { $0.key < $1.key }, id: \.key) { h in
                    LabeledContent(h.key, value: redact(h.key, h.value))
                }
            }
            Section("Mirrors") {
                if editingMirrors {
                    TextEditor(text: $mirrorsText)
                        .font(.caption.monospaced())
                        .frame(minHeight: 80)
                    HStack {
                        Spacer()
                        Button("Cancel") { editingMirrors = false }
                        Button("Save") {
                            var p = TaskPatchData()
                            p.mirrors = mirrorsText.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
                            let patch = p
                            editingMirrors = false
                            Task { await model.perform("Couldn't save mirrors") { try await model.engine.updateTask(item.id, patch: patch) } }
                        }
                        .keyboardShortcut(.defaultAction)
                    }
                } else {
                    ForEach(Array(detail.urls.enumerated()), id: \.offset) { _, u in
                        Text(u).font(.caption.monospaced()).lineLimit(2).truncationMode(.middle).textSelection(.enabled)
                    }
                    if detail.stats.mirrorsSwitched > 0 {
                        LabeledContent("Mirror switches", value: "\(detail.stats.mirrorsSwitched)")
                    }
                    Button("Edit Mirrors…") {
                        mirrorsText = detail.urls.joined(separator: "\n")
                        editingMirrors = true
                    }
                }
            }
        }
    }

    private func redact(_ key: String, _ value: String) -> String {
        let k = key.lowercased()
        return k.contains("auth") || k.contains("cookie") || k.contains("token") ? "••••••" : value
    }
}

// MARK: - Checksums

struct ChecksumsTab: View {
    let item: TaskItem
    let detail: TaskDetailData?
    @Environment(AppModel.self) private var model
    @ViewState private var expected = ""
    @ViewState private var algorithm: ChecksumAlgorithm = .sha256

    var body: some View {
        Section("Result") {
            if let v = detail?.verifiedChecksum {
                LabeledContent("Computed") { Text(v).font(.caption.monospaced()).textSelection(.enabled).lineLimit(3) }
            } else {
                Text(item.state == .completed ? "No checksum computed yet." : "Computed when the download finishes.").foregroundStyle(.secondary)
            }
            if let e = detail?.expectedChecksum {
                LabeledContent("Expected") { Text(e).font(.caption.monospaced()).textSelection(.enabled).lineLimit(3) }
                if let v = detail?.verifiedChecksum {
                    let match = v.lowercased().hasSuffix(e.split(separator: ":").last.map { String($0).lowercased() } ?? "")
                    Label(match ? "Matches" : "Does not match", systemImage: match ? "checkmark.circle.fill" : "xmark.octagon.fill")
                        .foregroundStyle(match ? Theme.success : Theme.danger)
                }
            }
        }
        Section("Verify") {
            Picker("Algorithm", selection: $algorithm) {
                ForEach(ChecksumAlgorithm.allCases, id: \.self) { Text($0.label).tag($0) }
            }
            TextField("Checksum", text: $expected, prompt: Text("Paste the published value"))
                .font(.caption.monospaced())
            if !expected.isEmpty && !isWellFormed {
                Text("A \(algorithm.label) checksum has \(algorithm.hexLength) hex characters.").font(.caption).foregroundStyle(Theme.warning)
            }
            HStack {
                Button("Save as Expected") {
                    var opts = detail?.options ?? TaskOptionsData()
                    opts.checksum = "\(algorithm.rawValue):\(cleaned)"
                    var p = TaskPatchData()
                    p.options = opts
                    let patch = p
                    Task { await model.perform("Couldn't save checksum") { try await model.engine.updateTask(item.id, patch: patch) } }
                }
                .disabled(!isWellFormed)
                Spacer()
                Button("Verify Now") {
                    let sum = expected.isEmpty ? nil : "\(algorithm.rawValue):\(cleaned)"
                    Task { await model.perform("Couldn't verify") { try await model.engine.verify(item.id, checksum: sum) } }
                }
                .disabled(item.state != .completed || (!expected.isEmpty && !isWellFormed))
            }
        }
        .onChange(of: expected) { _, v in
            let hex = v.trimmingCharacters(in: .whitespaces)
            if let a = ChecksumAlgorithm.allCases.first(where: { $0.hexLength == hex.count && $0 != .blake3 }) { algorithm = a }
        }
    }

    private var cleaned: String { expected.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
    private var isWellFormed: Bool { cleaned.count == algorithm.hexLength && cleaned.allSatisfy(\.isHexDigit) }
}

// MARK: - Events

struct EventsTab: View {
    let item: TaskItem
    @Environment(AppModel.self) private var model

    var body: some View {
        let entries = Array((model.liveLogs[item.id] ?? []).reversed())
        Section("Events") {
            if entries.isEmpty {
                Text("No events yet.").foregroundStyle(.secondary)
            }
            ForEach(entries) { e in
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        if e.level == .error || e.level == .warn {
                            Image(systemName: e.level == .error ? "xmark.octagon.fill" : "exclamationmark.triangle.fill")
                                .foregroundStyle(e.level == .error ? Theme.danger : Theme.warning)
                                .imageScale(.small)
                        }
                        Text(e.code).font(.caption.monospaced())
                        Spacer()
                        Text(Fmt.time(e.at)).font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
                    }
                    Text(e.message).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                }
            }
        }
        .task(id: item.id) { await model.watchLog(item.id) }
        .onDisappear { model.unwatchLog(item.id) }
    }
}

// MARK: - Diagnostics

struct DiagnosticsTab: View {
    let item: TaskItem
    let detail: TaskDetailData?
    @Environment(AppModel.self) private var model

    var body: some View {
        if let detail {
            let h = detail.health
            Section("Health") {
                LabeledContent("Overall") {
                    Text("\(h.score) — \(L10n.healthLabel(h.score))").foregroundStyle(Theme.health(h.score))
                }
                HealthRow(title: "Source stability", value: h.sourceStability)
                HealthRow(title: "Throughput consistency", value: h.throughputConsistency)
                HealthRow(title: "Connection quality", value: h.connectionQuality)
                HealthRow(title: "Retry pressure", value: h.retryPressure)
                HealthRow(title: "Remaining risk", value: h.remainingRisk)
                ForEach(h.notes, id: \.self) { n in
                    Text(L10n.health(n)).font(.caption).foregroundStyle(.secondary)
                }
            }
            Section("Counters") {
                LabeledContent("Retries", value: "\(detail.stats.retries)")
                LabeledContent("Failed connections", value: "\(detail.stats.failedConnections)")
                LabeledContent("Segments reassigned", value: "\(detail.stats.segmentsReassigned)")
                LabeledContent("Mirror switches", value: "\(detail.stats.mirrorsSwitched)")
                LabeledContent("Attempt", value: "\(detail.attempt)")
            }
            .monospacedDigit()
            Section {
                Button("Copy Diagnostics") {
                    Task {
                        if let text = await model.perform("Couldn't collect diagnostics", { try await model.engine.diagnosticsText(item.id) }) {
                            copyToPasteboard(text)
                            model.toast(.success, "Diagnostics copied", detail: "Secrets are redacted.")
                        }
                    }
                }
            }
        }
    }
}

struct HealthRow: View {
    let title: String
    let value: UInt8
    var body: some View {
        LabeledContent {
            HStack(spacing: 8) {
                ThinProgress(fraction: Double(value) / 100, tint: Color(nsColor: .secondaryLabelColor)).frame(width: 90)
                Text("\(value)").monospacedDigit().frame(width: 26, alignment: .trailing)
            }
        } label: {
            Text(LocalizedStringKey(title))
        }
        .accessibilityElement(children: .combine)
    }
}
