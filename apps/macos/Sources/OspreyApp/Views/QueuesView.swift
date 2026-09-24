import AppKit
import OspreyKit
import SwiftUI

/// Settings → Queues: the list of queues with edit/delete (per-queue downloads live in the sidebar).
struct QueuesSettingsPane: View {
    @Environment(AppModel.self) private var model
    @ViewState private var editing: QueueData?

    var body: some View {
        Form {
            Section {
                ForEach(model.queues) { q in
                    let s = model.queueSummaries[q.id]
                    LabeledContent {
                        HStack {
                            Text("\(s?.active ?? 0) active · \(s?.waiting ?? 0) waiting").foregroundStyle(.secondary).monospacedDigit()
                            Button("Edit…") { editing = q }
                        }
                    } label: {
                        Label(q.name, systemImage: q.icon.isEmpty ? "tray" : q.icon)
                        Text(q.paused ? "Paused" : "Up to \(q.maxConcurrent) at once")
                    }
                }
            } footer: {
                HStack {
                    Spacer()
                    Button("New Queue…") { editing = QueueData(name: L10n.tr("New Queue"), position: Int32(model.queues.count)) }
                }
            }
        }
        .formStyle(.grouped)
        .sheet(item: $editing) { q in QueueEditor(queue: q).environment(model) }
    }
}

struct QueueEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @ViewState private var q: QueueData
    @ViewState private var downLimit: String
    @ViewState private var upLimit: String

    init(queue: QueueData) {
        _q = ViewState(wrappedValue: queue)
        _downLimit = ViewState(wrappedValue: queue.downloadLimit > 0 ? Fmt.bytes(queue.downloadLimit) : "")
        _upLimit = ViewState(wrappedValue: queue.uploadLimit > 0 ? Fmt.bytes(queue.uploadLimit) : "")
    }

    static let icons = ["tray.full", "bolt", "film", "network", "moon.stars", "music.note", "doc.text", "shippingbox", "gamecontroller", "graduationcap", "briefcase", "star"]

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Section {
                    TextField("Name", text: $q.name)
                    Picker("Icon", selection: $q.icon) {
                        ForEach(Self.icons, id: \.self) { Image(systemName: $0).tag($0) }
                    }
                    .pickerStyle(.segmented)
                }
                Section("Limits") {
                    Stepper(value: Binding(get: { Int(q.maxConcurrent) }, set: { q.maxConcurrent = UInt32(max(1, $0)) }), in: 1...32) {
                        LabeledContent("Downloads at once", value: "\(q.maxConcurrent)")
                    }
                    TextField("Download limit (e.g. 5 MB, blank = unlimited)", text: $downLimit)
                    TextField("Upload limit", text: $upLimit)
                    Stepper(value: Binding(get: { Int(q.connectionsPerTask) }, set: { q.connectionsPerTask = UInt8(clamping: max(0, $0)) }), in: 0...64) {
                        LabeledContent("Connections per download", value: q.connectionsPerTask == 0 ? L10n.tr("Default") : "\(q.connectionsPerTask)")
                    }
                }
                Section("Behaviour") {
                    Picker("Schedule", selection: Binding(get: { q.scheduleId ?? "" }, set: { q.scheduleId = $0.isEmpty ? nil : $0 })) {
                        Text("Always").tag("")
                        ForEach(model.schedules) { Text($0.name).tag($0.id) }
                    }
                    HStack {
                        Text("Save to")
                        Spacer()
                        Text(q.directory?.abbreviatedPath ?? L10n.tr("Default folder")).foregroundStyle(.secondary).lineLimit(1)
                        Button("Choose…") {
                            let panel = NSOpenPanel()
                            panel.canChooseDirectories = true
                            panel.canChooseFiles = false
                            panel.canCreateDirectories = true
                            if panel.runModal() == .OK { q.directory = panel.url?.path }
                        }
                        if q.directory != nil { Button("Reset") { q.directory = nil } }
                    }
                    Picker("When the queue finishes", selection: $q.completionAction) {
                        Text("Do nothing").tag("nothing")
                        Text("Notify me").tag("notify")
                        Text("Sleep the Mac").tag("sleep")
                        Text("Quit Osprey").tag("quit_application")
                        ForEach(model.automations) { a in Text("Run “\(a.name)”").tag("run_automation:\(a.id)") }
                    }
                }
            }
            .formStyle(.grouped)
            .scrollContentBackground(.hidden)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Save") {
                    var saved = q
                    saved.downloadLimit = Fmt.parseBytes(downLimit) ?? 0
                    saved.uploadLimit = Fmt.parseBytes(upLimit) ?? 0
                    Task {
                        if await model.perform("Couldn't save queue", { try await model.engine.saveQueue(saved) }) != nil {
                            await model.reloadQueues()
                            dismiss()
                        }
                    }
                }
                .keyboardShortcut(.defaultAction)
                .ospreyGlassButton(prominent: true)
                .disabled(q.name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            .padding(16)
        }
        .frame(width: 520, height: 600)
        .task {
            await model.load("schedules")
            await model.load("automations")
        }
    }
}
