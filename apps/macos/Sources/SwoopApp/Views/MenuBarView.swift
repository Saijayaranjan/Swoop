import AppKit
import SwoopKit
import SwiftUI

/// The menu bar extra (window style): live speeds, sparkline, speed modes, recent completions and
/// quick actions — all on glass.
struct MenuBarView: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @Environment(\.appDelegate) private var delegate
    @ViewState private var quickURL = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                SwoopMark(size: 26)
                Text("Swoop").font(.system(size: 16, weight: .bold, design: .rounded))
                Spacer()
                HStack(spacing: 4) {
                    StatusDot(color: model.networkAvailable ? Theme.success : Theme.warning)
                    Text(model.networkAvailable ? "Online" : "Offline").font(.system(size: 11, weight: .medium)).foregroundStyle(.secondary)
                }
            }

            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .firstTextBaseline) {
                    LiveNumber(Fmt.speed(model.stats.downloadSpeed, zero: "0 B/s"), value: Double(model.stats.downloadSpeed), size: 30)
                    Spacer()
                    Label(Fmt.speed(model.stats.uploadSpeed, zero: "0 B/s"), systemImage: "arrow.up")
                        .font(.system(size: 12, weight: .semibold).monospacedDigit())
                        .foregroundStyle(Theme.upload)
                }
                let samples = model.speedHistory.suffix(90)
                let ceiling = max(Double(samples.map { max($0.download, $0.upload) }.max() ?? 0) * 1.15, 250_000)
                ZStack {
                    Sparkline(samples.map { Double($0.download) }, color: Theme.blue, fill: true, lineWidth: 1.8, ceiling: ceiling)
                    Sparkline(samples.map { Double($0.upload) }, color: Theme.upload, fill: false, lineWidth: 1.2, ceiling: ceiling)
                }
                .frame(height: 44)
            }
            .cardSurface(cornerRadius: 16, padding: 14)

            HStack(spacing: 8) {
                miniStat("Active", "\(model.stats.active)")
                miniStat("Queued", "\(model.stats.queued + model.stats.scheduled)")
                miniStat("Today", Fmt.bytes(model.stats.bytesToday))
            }

            HStack(spacing: 4) {
                ForEach(TrafficMode.quickModes, id: \.self) { mode in
                    let current = model.stats.trafficMode == .fullSpeed ? TrafficMode.unlimited : model.stats.trafficMode
                    let selected = current == mode
                    Button { model.setTrafficMode(mode) } label: {
                        HStack(spacing: 5) {
                            Image(systemName: mode.symbol).font(.system(size: 11, weight: .semibold))
                            Text(LocalizedStringKey(mode.label)).font(.system(size: 12, weight: .medium))
                        }
                        .frame(maxWidth: .infinity, minHeight: 30)
                        .foregroundStyle(selected ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
                        .background {
                            if selected { Capsule().fill(Theme.blue.gradient) }
                        }
                        .contentShape(Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(selected ? .isSelected : [])
                }
            }
            .padding(3)
            .background(Theme.well, in: Capsule())

            HStack(spacing: 8) {
                Image(systemName: "link").foregroundStyle(.secondary)
                TextField("Paste a link and press Return", text: $quickURL)
                    .textFieldStyle(.plain)
                    .onSubmit(addQuick)
                Button { if let s = NSPasteboard.general.string(forType: .string) { quickURL = s.trimmingCharacters(in: .whitespacesAndNewlines); addQuick() } } label: {
                    Image(systemName: "doc.on.clipboard")
                }
                .buttonStyle(.borderless)
                .help("Download the link on the clipboard")
            }
            .padding(.horizontal, 12)
            .frame(height: 36)
            .swoopGlass(.regular, in: Capsule())

            if !model.recentCompletions.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    CardLabel("Recently finished")
                    ForEach(model.recentCompletions.prefix(4)) { row in
                        Button { Finder.reveal([row.filePath ?? (row.directory as NSString).appendingPathComponent(row.name)]) } label: {
                            HStack(spacing: 8) {
                                FileBadge(name: row.name, kind: row.kind, size: 24)
                                Text(row.name).font(.system(size: 12, weight: .medium)).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                Text(Fmt.bytes(row.progress.total)).font(.system(size: 11).monospacedDigit()).foregroundStyle(.secondary)
                            }
                            .padding(.vertical, 2)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }

            HStack(spacing: 6) {
                Button { model.pauseAll() } label: { Label("Pause All", systemImage: "pause.fill").frame(maxWidth: .infinity) }
                Button { model.resumeAll() } label: { Label("Resume All", systemImage: "play.fill").frame(maxWidth: .infinity) }
                Button { model.retryFailed() } label: { Image(systemName: "arrow.clockwise") }
                    .help("Retry Failed")
            }
            .controlSize(.regular)
            .swoopGlassButton()
            Rectangle().fill(Theme.hairline).frame(height: 1)
            HStack(spacing: 14) {
                Button("Open Swoop") { delegate?.showMainWindow() }
                Button("Add…") { delegate?.showMainWindow(); ui.openAdd() }
                Button("Downloads Folder") { NSWorkspace.shared.open(URL(fileURLWithPath: model.settings.downloadDirectory)) }
                Spacer()
                Button { NSApp.terminate(nil) } label: { Image(systemName: "power") }
                    .help("Quit Swoop")
                    .accessibilityLabel(Text("Quit Swoop"))
            }
            .buttonStyle(.borderless)
            .font(.system(size: 12, weight: .medium))
        }
        .padding(16)
        .frame(width: 350)
    }

    private func miniStat(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(L10n.tr(label).uppercased()).font(.system(size: 9, weight: .semibold)).tracking(0.9).foregroundStyle(.secondary)
            Text(value).font(.system(size: 17, weight: .semibold, design: .rounded).monospacedDigit()).lineLimit(1).minimumScaleFactor(0.7)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Theme.well, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }

    private func addQuick() {
        let links = LinkDetector.links(in: quickURL)
        guard !links.isEmpty else { return }
        quickURL = ""
        Task {
            var added = 0
            for link in links {
                var req = NewTaskRequestData()
                if link.lowercased().hasPrefix("magnet:") { req.magnet = link } else { req.url = link }
                req.origin = "menu_bar"
                if await model.perform("Couldn't add download", { try await model.engine.addTask(req) }) != nil { added += 1 }
            }
            if added > 0 { model.toast(.success, "Added \(added) download\(added == 1 ? "" : "s")") }
        }
    }
}
