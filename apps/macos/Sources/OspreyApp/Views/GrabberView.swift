import AppKit
import OspreyKit
import SwiftUI

struct GrabberView: View {
    @Environment(AppModel.self) private var model
    @ViewState private var options = GrabberOptionsData()
    @ViewState private var extensionsText = ""
    @ViewState private var excludeText = ""
    @ViewState private var session: GrabberSessionData?
    @ViewState private var past: [GrabberSessionData] = []
    @ViewState private var starting = false

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    optionsForm
                    if !past.isEmpty { pastSessions }
                }
                .padding(16)
            }
            .frame(width: 330)
            .ospreyGlass(.regular, cornerRadius: 22)

            Group {
                if let session {
                    GrabberResults(session: session) { self.session = $0 }
                } else {
                    EmptyStateView("square.stack.3d.down.right", title: "Site Grabber",
                                   message: "Crawl a page (and pages it links to) to collect every downloadable file, then pick what to download.")
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .ospreyGlass(.regular, cornerRadius: 22)
        }
        .padding(16)
        .navigationTitle("Site Grabber")
        .task { past = (try? await model.engine.grabberList()) ?? [] }
        .task(id: session?.id) {
            // Poll the live session until it finishes (progress events trigger faster refreshes).
            guard let id = session?.id else { return }
            while !Task.isCancelled {
                if let s = try? await model.engine.grabberStatus(id) {
                    session = s
                    if s.done || s.cancelled { break }
                }
                try? await Task.sleep(nanoseconds: 800_000_000)
            }
            past = (try? await model.engine.grabberList()) ?? past
        }
    }

    private var optionsForm: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Crawl").font(.headline)
            TextField("https://example.com/downloads/", text: $options.url)
                .textFieldStyle(.roundedBorder)
            Stepper(value: Binding(get: { Int(options.maxDepth) }, set: { options.maxDepth = UInt8($0) }), in: 0...5) {
                LabeledContent("Link depth", value: options.maxDepth == 0 ? L10n.tr("This page only") : "\(options.maxDepth)")
            }
            Picker("Follow links", selection: $options.scope) {
                Text("Same site").tag("same_domain")
                Text("Including subdomains").tag("subdomains")
                Text("Anywhere").tag("external")
            }
            Stepper(value: Binding(get: { Int(options.maxPages) }, set: { options.maxPages = UInt32($0) }), in: 1...5000, step: 50) {
                LabeledContent("Max pages", value: "\(options.maxPages)")
            }
            Stepper(value: Binding(get: { Int(options.concurrency) }, set: { options.concurrency = UInt8($0) }), in: 1...16) {
                LabeledContent("Parallel requests", value: "\(options.concurrency)")
            }
            TextField("File types (blank = common downloads)", text: $extensionsText)
                .textFieldStyle(.roundedBorder)
                .help("Comma-separated extensions, e.g. pdf, zip, mp4")
            TextField("Exclude URLs matching (e.g. *thumb*)", text: $excludeText)
                .textFieldStyle(.roundedBorder)
            HStack {
                Text("Size").foregroundStyle(.secondary)
                BytesField(bytes: Binding(get: { options.minSize ?? 0 }, set: { options.minSize = $0 == 0 ? nil : $0 }))
                Text("–")
                BytesField(bytes: Binding(get: { options.maxSize ?? 0 }, set: { options.maxSize = $0 == 0 ? nil : $0 }))
            }
            Toggle("Respect robots.txt", isOn: $options.respectRobots)
            Toggle("Check each file's size and type", isOn: $options.probeFiles)
            Toggle("Follow links inside frames", isOn: $options.followIframes)
            HStack {
                if let s = session, !s.done, !s.cancelled {
                    Button("Stop") { Task { await model.perform("Couldn't stop") { try await model.engine.grabberCancel(s.id) } } }
                        .ospreyGlassButton()
                }
                Spacer()
                Button {
                    Task { await start() }
                } label: {
                    if starting { ProgressView().controlSize(.small) } else { Label("Start Crawl", systemImage: "play.fill") }
                }
                .ospreyGlassButton(prominent: true)
                .disabled(!LinkDetector.looksLikeLink(options.url) || starting)
            }
        }
        .font(.callout)
    }

    private var pastSessions: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Recent crawls").font(.headline)
            ForEach(past.sorted { $0.startedAt > $1.startedAt }.prefix(8)) { s in
                Button { session = s } label: {
                    HStack {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(URL(string: s.options.url)?.host ?? s.options.url).lineLimit(1)
                            Text("\(s.files.count) files · \(Fmt.relative(s.startedAt))").font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if !s.done && !s.cancelled { ProgressView().controlSize(.mini) }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func start() async {
        starting = true
        defer { starting = false }
        var o = options
        o.includeExtensions = extensionsText.split(separator: ",").map { $0.trimmingCharacters(in: CharacterSet(charactersIn: " .")).lowercased() }.filter { !$0.isEmpty }
        o.excludePatterns = excludeText.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        if let s = await model.perform("Couldn't start the crawl", { try await model.engine.grabberStart(o) }) {
            session = s
        }
    }
}

struct GrabberResults: View {
    let session: GrabberSessionData
    let update: (GrabberSessionData) -> Void
    @Environment(AppModel.self) private var model
    @ViewState private var selected: Set<String> = []
    @ViewState private var filter = ""
    @ViewState private var grouping = 0
    @ViewState private var directory = ""
    @ViewState private var queueId = ""
    @ViewState private var adding = false

    private var files: [GrabberFileData] {
        let f = filter.lowercased()
        return session.files.filter { f.isEmpty || $0.url.lowercased().contains(f) || $0.name.lowercased().contains(f) }
    }

    private var groups: [(key: String, files: [GrabberFileData])] {
        let dict = Dictionary(grouping: files) { grouping == 0 ? $0.kind : ($0.domain.isEmpty ? "—" : $0.domain) }
        return dict.keys.sorted().map { ($0, dict[$0]!.sorted { $0.name < $1.name }) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            HStack {
                TextField("Filter results", text: $filter).textFieldStyle(.roundedBorder).frame(maxWidth: 260)
                Picker("", selection: $grouping) {
                    Text("By type").tag(0)
                    Text("By site").tag(1)
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 170)
                Spacer()
                Menu("Select") {
                    Button("All") { selected = Set(files.map(\.url)) }
                    Button("None") { selected = [] }
                    Divider()
                    Menu("By Type") {
                        ForEach(Array(Set(session.files.map(\.kind))).sorted(), id: \.self) { k in
                            Button(k.capitalized) { selected.formUnion(session.files.filter { $0.kind == k }.map(\.url)) }
                        }
                    }
                    Menu("By Extension") {
                        ForEach(Array(Set(session.files.map(\.ext))).filter { !$0.isEmpty }.sorted(), id: \.self) { e in
                            Button(".\(e)") { selected.formUnion(session.files.filter { $0.ext == e }.map(\.url)) }
                        }
                    }
                    Menu("By Site") {
                        ForEach(Array(Set(session.files.map(\.domain))).sorted(), id: \.self) { d in
                            Button(d) { selected.formUnion(session.files.filter { $0.domain == d }.map(\.url)) }
                        }
                    }
                    Divider()
                    Button("Exclude Filtered") { selected.subtract(files.map(\.url)) }
                }
                .fixedSize()
            }
            List {
                ForEach(groups, id: \.key) { g in
                    Section {
                        ForEach(g.files) { f in fileRow(f) }
                    } header: {
                        HStack {
                            Toggle(isOn: Binding(get: { g.files.allSatisfy { selected.contains($0.url) } }, set: { on in
                                if on { selected.formUnion(g.files.map(\.url)) } else { selected.subtract(g.files.map(\.url)) }
                            })) { Text(g.key.capitalized).font(.subheadline.weight(.semibold)) }
                            .toggleStyle(.checkbox)
                            Spacer()
                            Text("\(g.files.count)").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            .listStyle(.inset)
            .scrollContentBackground(.hidden)
            footer
        }
        .padding(16)
        .onAppear { queueId = model.queues.first?.id ?? "" }
    }

    private var header: some View {
        HStack(spacing: 16) {
            VStack(alignment: .leading, spacing: 2) {
                Text(URL(string: session.options.url)?.host ?? session.options.url).font(.title3.weight(.semibold))
                Text(session.options.url).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            let live = model.grabberProgress[session.id]
            metric("Pages", "\(max(session.pagesCrawled, live?.pages ?? 0))")
            metric("Files", "\(max(UInt32(session.files.count), live?.files ?? 0))")
            if session.robotsBlocked > 0 { metric("Blocked by robots", "\(session.robotsBlocked)") }
            if !session.done && !session.cancelled {
                ProgressView().controlSize(.small)
            } else if session.cancelled {
                Label("Stopped", systemImage: "stop.circle").foregroundStyle(.secondary)
            } else {
                Label("Done", systemImage: "checkmark.circle.fill").foregroundStyle(Theme.success)
            }
        }
        .overlay(alignment: .bottomLeading) {
            if let e = session.error { Text(e).font(.caption).foregroundStyle(Theme.danger).offset(y: 16) }
        }
    }

    private func metric(_ t: String, _ v: String) -> some View {
        VStack(alignment: .trailing, spacing: 0) {
            Text(v).font(Theme.numeral(18))
            Text(LocalizedStringKey(t)).font(.caption2).foregroundStyle(.secondary)
        }
    }

    private func fileRow(_ f: GrabberFileData) -> some View {
        Toggle(isOn: Binding(get: { selected.contains(f.url) }, set: { if $0 { selected.insert(f.url) } else { selected.remove(f.url) } })) {
            HStack(spacing: 8) {
                Image(systemName: Theme.fileSymbol(name: f.name)).foregroundStyle(Theme.fileTint(name: f.name)).frame(width: 18)
                VStack(alignment: .leading, spacing: 1) {
                    Text(f.name).lineLimit(1).truncationMode(.middle)
                    Text(f.url).font(.caption2).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                }
                Spacer()
                Text(Fmt.bytes(f.size)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
        }
        .toggleStyle(.checkbox)
    }

    private var footer: some View {
        HStack {
            Text("\(selected.count) selected · \(Fmt.bytes(session.files.filter { selected.contains($0.url) }.reduce(0) { $0 + ($1.size ?? 0) }))")
                .font(.caption).foregroundStyle(.secondary)
            Spacer()
            Picker("Queue", selection: $queueId) {
                ForEach(model.queues) { Text($0.name).tag($0.id) }
            }
            .fixedSize()
            Button(directory.isEmpty ? "Default Folder" : (directory as NSString).lastPathComponent) {
                let panel = NSOpenPanel()
                panel.canChooseDirectories = true
                panel.canChooseFiles = false
                panel.canCreateDirectories = true
                if panel.runModal() == .OK { directory = panel.url?.path ?? "" }
            }
            Button {
                Task { await addSelected() }
            } label: {
                if adding { ProgressView().controlSize(.small) } else { Label("Add Selected", systemImage: "arrow.down.circle.fill") }
            }
            .ospreyGlassButton(prominent: true)
            .disabled(selected.isEmpty || adding)
        }
    }

    private func addSelected() async {
        adding = true
        defer { adding = false }
        var req = NewTaskRequestData()
        req.queueId = queueId.isEmpty ? nil : queueId
        req.directory = directory.isEmpty ? nil : directory
        req.origin = "grabber"
        req.refererPage = session.options.url
        let urls = session.files.map(\.url).filter { selected.contains($0) }
        if let results = await model.perform("Couldn't add files", { try await model.engine.grabberAdd(session.id, urls: urls, request: req) }) {
            model.toast(.success, "Added \(results.count) downloads")
            selected = []
        }
    }
}
