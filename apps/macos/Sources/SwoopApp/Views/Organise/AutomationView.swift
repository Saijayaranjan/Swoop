import SwoopKit
import SwiftUI

struct AutomationView: View {
    @Environment(AppModel.self) private var model
    @ViewState private var selection: String?
    @ViewState private var draft: AutomationDoc?

    var body: some View {
        MasterDetail(items: model.automations, selection: $selection,
                     onAdd: { draft = AutomationDoc(name: L10n.tr("New Automation")); selection = nil },
                     onRemove: { id in
                         Task {
                             await model.perform("Couldn't delete automation") { try await model.engine.deleteAutomation(id) }
                             await model.load("automations")
                         }
                     }) { a in
            MasterRow(symbol: a.executesCode ? "exclamationmark.shield" : "gearshape.2",
                      tint: a.executesCode ? Theme.warning : (a.enabled ? Theme.blue : Theme.neutral),
                      title: a.name, subtitle: a.lastError ?? "Ran \(a.runCount)×",
                      subtitleTint: a.lastError == nil ? nil : Theme.danger, dimmed: !a.enabled)
        } detail: {
            if let draft {
                AutomationEditor(automation: draft) { self.draft = nil; selection = $0 }.id(draft.id)
            } else if let id = selection, let a = model.automations.first(where: { $0.id == id }) {
                AutomationEditor(automation: a) { selection = $0 }.id(a.id + String(a.updatedAt))
            } else {
                EmptyStateView("gearshape.2", title: "Let downloads finish themselves",
                               message: "Automations react to download events. Built-in actions are safe; scripts need your explicit consent.") {
                    Button("New Automation") { draft = AutomationDoc(name: L10n.tr("New Automation")) }.buttonStyle(ProminentCapsuleStyle())
                }
            }
        }
        .task { await model.load("automations") }
    }
}

struct AutomationEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @ViewState private var a: AutomationDoc
    @ViewState private var runs: [AutomationRunDoc] = []
    @ViewState private var consentJustGranted = false
    let onSaved: (String?) -> Void

    init(automation: AutomationDoc, onSaved: @escaping (String?) -> Void) {
        _a = ViewState(wrappedValue: automation)
        self.onSaved = onSaved
    }

    private var saved: AutomationDoc? { model.automations.first { $0.id == a.id } }
    private var dirty: Bool { saved.map { $0.actions != a.actions || $0.name != a.name || $0.events != a.events || $0.conditions != a.conditions || $0.enabled != a.enabled || $0.matchMode != a.matchMode } ?? true }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack {
                    TextField("Automation name", text: $a.name).textFieldStyle(.plain).font(.title2.weight(.semibold))
                    Toggle("Enabled", isOn: $a.enabled).toggleStyle(.switch)
                }
                if a.executesCode { consentBanner }
                VStack(alignment: .leading, spacing: 8) {
                    Text("When").font(.headline)
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 170), alignment: .leading)], alignment: .leading, spacing: 6) {
                        ForEach(AutomationEventKind.allCases, id: \.self) { e in
                            Toggle(e.label, isOn: Binding(get: { a.events.contains(e) }, set: { on in
                                if on { a.events.append(e) } else { a.events.removeAll { $0 == e } }
                            }))
                            .toggleStyle(.checkbox)
                        }
                    }
                }
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text("Only if").font(.headline)
                        Picker("", selection: $a.matchMode) {
                            Text("all match").tag(MatchMode.all)
                            Text("any match").tag(MatchMode.any)
                        }
                        .labelsHidden().fixedSize()
                    }
                    VariantListEditor(family: ActionSchemas.ruleCondition, items: $a.conditions, addTitle: "Add Condition",
                                      emptyText: "No conditions — runs for every download.")
                }
                VStack(alignment: .leading, spacing: 8) {
                    Text("Do").font(.headline)
                    Text("Variables: {name} {stem} {ext} {file_path} {directory} {url} {domain} {date} {size} {checksum}")
                        .font(.caption.monospaced()).foregroundStyle(.secondary)
                    VariantListEditor(family: ActionSchemas.automationAction, items: $a.actions, addTitle: "Add Action",
                                      emptyText: "Add the actions to perform.")
                }
                HStack {
                    if saved != nil {
                        Button("Delete", role: .destructive) {
                            Task {
                                await model.perform("Couldn't delete automation") { try await model.engine.deleteAutomation(a.id) }
                                await model.load("automations")
                                onSaved(nil)
                            }
                        }
                        Menu("Run Now On…") {
                            ForEach(model.tasks.items.filter { $0.state == .completed }.prefix(20)) { t in
                                Button(t.name) {
                                    Task {
                                        await model.perform("Automation failed") { try await model.engine.runAutomation(a.id, taskId: t.id) }
                                        await loadRuns()
                                    }
                                }
                            }
                        }
                        .fixedSize()
                        .disabled(dirty)
                    }
                    Spacer()
                    Button("Save") { Task { if await model.saveAutomation(a) { onSaved(a.id); await loadRuns() } } }
                        .swoopGlassButton(prominent: true)
                        .keyboardShortcut("s", modifiers: .command)
                        .disabled(a.name.isEmpty || a.events.isEmpty)
                }
                if !runs.isEmpty {
                    Divider()
                    Text("Recent runs").font(.headline)
                    ForEach(runs) { r in
                        HStack(alignment: .top) {
                            Image(systemName: r.success ? "checkmark.circle.fill" : "xmark.octagon.fill")
                                .foregroundStyle(r.success ? Theme.success : Theme.danger)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(r.message).font(.caption).textSelection(.enabled)
                                Text("\(r.event.replacingOccurrences(of: "_", with: " ")) · \(Fmt.relative(r.at))").font(.caption2).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .padding(20)
        }
        .task(id: a.id) { await loadRuns() }
    }

    private var consentBanner: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("This automation runs code on your Mac", systemImage: "exclamationmark.shield.fill")
                .font(.callout.weight(.semibold))
                .foregroundStyle(Theme.warning)
            Text("Shell commands, programs and scripts run with your permissions. Swoop records consent for the exact code you approve; changing any script revokes it until you grant consent again.")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                if a.consentGranted == true || consentJustGranted {
                    Label("Consent granted for the saved version", systemImage: "checkmark.seal.fill")
                        .font(.caption).foregroundStyle(Theme.success)
                }
                Spacer()
                Button {
                    Task {
                        // Consent binds to the saved code, so save first.
                        if dirty { guard await model.saveAutomation(a) else { return } }
                        if await model.perform("Couldn't record consent", { try await model.engine.grantAutomationConsent(a.id) }) != nil {
                            consentJustGranted = true
                            model.toast(.success, "Consent granted", detail: a.name)
                            await model.load("automations")
                        }
                    }
                } label: {
                    Label(dirty ? "Save & Grant Consent" : "Grant Consent", systemImage: "hand.raised.fill")
                }
                .swoopGlassButton(prominent: true)
            }
        }
        .padding(14)
        .background(Theme.warning.opacity(0.1), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(Theme.warning.opacity(0.35), lineWidth: 0.5))
        .onChange(of: a.actions) { _, _ in consentJustGranted = false }
    }

    private func loadRuns() async {
        guard saved != nil, let text = try? await model.engine.automationRunsJSON(a.id, limit: 20) else { return }
        runs = (try? EngineJSON.decode([AutomationRunDoc].self, from: text)) ?? []
    }
}
