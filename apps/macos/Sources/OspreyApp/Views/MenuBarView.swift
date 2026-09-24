import AppKit
import OspreyKit
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
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 0) {
                    LiveNumber(Fmt.speed(model.stats.downloadSpeed, zero: "0 B/s"), value: Double(model.stats.downloadSpeed), size: 26)
                        .foregroundStyle(Theme.accent)
                    Text("↑ \(Fmt.speed(model.stats.uploadSpeed, zero: "0 B/s")) · \(model.stats.active) active")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Circle().fill(model.networkAvailable ? Theme.success : Theme.danger).frame(width: 8, height: 8)
                    .accessibilityLabel(Text(model.networkAvailable ? "Online" : "Offline"))
            }
            Sparkline(model.speedHistory.suffix(90).map { Double($0.download) })
                .frame(height: 38)

            GlassGroup(spacing: 4) {
                HStack(spacing: 4) {
                    ForEach(TrafficMode.pickerModes, id: \.self) { mode in
                        let selected = model.stats.trafficMode == mode
                        Button { model.setTrafficMode(mode) } label: {
                            VStack(spacing: 2) {
                                Image(systemName: mode.symbol).font(.system(size: 13))
                                Text(mode.label).font(.system(size: 9, weight: .medium))
                            }
                            .frame(maxWidth: .infinity, minHeight: 38)
                            .background { if selected { RoundedRectangle(cornerRadius: 10).fill(Theme.accent.opacity(0.25)) } }
                            .contentShape(RoundedRectangle(cornerRadius: 10))
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(selected ? Theme.accent : .primary)
                        .accessibilityAddTraits(selected ? .isSelected : [])
                    }
                }
                .padding(4)
                .ospreyGlass(.regular, cornerRadius: 14)
            }

            HStack(spacing: 6) {
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
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .ospreyGlass(.regular, in: Capsule())

            if !model.recentCompletions.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Recently finished").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    ForEach(model.recentCompletions.prefix(5)) { row in
                        Button { Finder.reveal([row.filePath ?? (row.directory as NSString).appendingPathComponent(row.name)]) } label: {
                            HStack(spacing: 8) {
                                Image(systemName: Theme.fileSymbol(name: row.name, kind: row.kind))
                                    .foregroundStyle(Theme.fileTint(name: row.name, kind: row.kind)).frame(width: 16)
                                Text(row.name).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                Text(Fmt.bytes(row.progress.total)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .font(.callout)
                    }
                }
            }

            Divider()
            HStack(spacing: 6) {
                Button { model.pauseAll() } label: { Label("Pause All", systemImage: "pause.fill") }
                Button { model.resumeAll() } label: { Label("Resume All", systemImage: "play.fill") }
                Button { model.retryFailed() } label: { Label("Retry Failed", systemImage: "arrow.clockwise") }
            }
            .controlSize(.small)
            .ospreyGlassButton()
            HStack {
                Button("Open Osprey") { delegate?.showMainWindow() }
                Button("Add…") { delegate?.showMainWindow(); ui.openAdd() }
                Button("Downloads Folder") { NSWorkspace.shared.open(URL(fileURLWithPath: model.settings.downloadDirectory)) }
                Spacer()
                Button { NSApp.terminate(nil) } label: { Image(systemName: "power") }
                    .help("Quit Osprey")
                    .accessibilityLabel(Text("Quit Osprey"))
            }
            .buttonStyle(.borderless)
            .font(.callout)
        }
        .padding(14)
        .frame(width: 340)
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
