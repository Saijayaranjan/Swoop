import OspreyKit
import SwiftUI

struct RulesView: View {
    @Environment(AppModel.self) private var model
    @ViewState private var selection: String?
    @ViewState private var draft: RuleDoc?

    var body: some View {
        MasterDetail(items: model.rules, selection: $selection,
                     onAdd: { draft = RuleDoc(name: L10n.tr("New Rule")); selection = nil },
                     onRemove: { id in
                         Task {
                             await model.perform("Couldn't delete rule") { try await model.engine.deleteRule(id) }
                             await model.load("rules")
                             selection = nil
                         }
                     }) { rule in
            VStack(alignment: .leading, spacing: 1) {
                Text(rule.name).foregroundStyle(rule.enabled ? .primary : .secondary)
                Text(rule.enabled ? "Used \(rule.hitCount) times" : "Off").font(.caption).foregroundStyle(.secondary)
            }
        } detail: {
            if let draft {
                RuleEditor(rule: draft) { self.draft = nil; selection = $0 }.id(draft.id)
            } else if let id = selection, let rule = model.rules.first(where: { $0.id == id }) {
                RuleEditor(rule: rule) { selection = $0 }.id(rule.id + String(rule.updatedAt))
            } else {
                ContentUnavailableView {
                    Label("No Rule Selected", systemImage: "arrow.triangle.branch")
                } description: {
                    Text("Rules sort new downloads by type, site, size or name — where they're saved and how they run.")
                } actions: {
                    Button("New Rule") { draft = RuleDoc(name: L10n.tr("New Rule")) }
                }
            }
        }
        .task { await model.load("rules"); await model.load("automations") }
    }
}

struct RuleEditor: View {
    @Environment(AppModel.self) private var model
    @ViewState private var rule: RuleDoc
    @ViewState private var testURL = ""
    @ViewState private var testSize = UInt64(0)
    @ViewState private var testResults: [RuleTestResult]?
    let onSaved: (String?) -> Void

    init(rule: RuleDoc, onSaved: @escaping (String?) -> Void) {
        _rule = ViewState(wrappedValue: rule)
        self.onSaved = onSaved
    }

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $rule.name)
                Toggle("Enabled", isOn: $rule.enabled)
                Stepper(value: $rule.priority, in: 0...1000, step: 10) {
                    LabeledContent("Order", value: "\(rule.priority)")
                }
                .help("Lower numbers run first")
            }
            Section {
                VariantListEditor(family: ActionSchemas.ruleCondition, items: $rule.conditions, addTitle: "Add Condition",
                                  emptyText: "Rules without conditions never match.")
            } header: {
                HStack {
                    Text("If")
                    Picker("", selection: $rule.matchMode) {
                        Text("all").tag(MatchMode.all)
                        Text("any").tag(MatchMode.any)
                    }
                    .labelsHidden()
                    .fixedSize()
                    Text("of these match")
                }
            }
            Section("Then") {
                VariantListEditor(family: ActionSchemas.ruleAction, items: $rule.actions, addTitle: "Add Action",
                                  emptyText: "Choose what happens to matching downloads.")
            }
            Section("Try It") {
                TextField("Link", text: $testURL, prompt: Text("https://example.com/file.zip"))
                LabeledContent("Size") { BytesField(bytes: $testSize) }
                Button("Test Saved Rules") { Task { await runTest() } }
                if let testResults {
                    if testResults.isEmpty { Text("No saved rule matches.").foregroundStyle(.secondary) }
                    ForEach(testResults) { r in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(r.ruleName)
                            ForEach(r.actions) { a in
                                Text(ActionSchemas.ruleAction.summary(a)).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            Section {
                HStack {
                    Spacer()
                    Button("Save") { Task { if await model.saveRule(rule) { onSaved(rule.id) } } }
                        .keyboardShortcut("s", modifiers: .command)
                        .disabled(rule.name.isEmpty)
                }
            }
        }
        .formStyle(.grouped)
    }

    private func runTest() async {
        var subject = RuleSubjectDoc()
        subject.url = testURL
        subject.domain = URL(string: testURL)?.host ?? ""
        subject.name = URL(string: testURL)?.lastPathComponent ?? ""
        subject.size = testSize == 0 ? nil : testSize
        guard let json = try? EngineJSON.encode(subject),
              let out = await model.perform("Couldn't test rules", { try await model.engine.testRulesJSON(json) }) else { return }
        testResults = RuleTestParser.parse(out)
    }
}

/// `test_rules_json` returns `[[rule, [actions]], …]` (serde tuple encoding) or
/// `[{"rule": …, "actions": …}]`; accept both.
enum RuleTestParser {
    static func parse(_ json: String) -> [RuleTestResult] {
        guard let v = try? JSONValue(parsing: json), let arr = v.array else { return [] }
        return arr.compactMap { entry in
            let rule: JSONValue?
            let actions: JSONValue?
            if let pair = entry.array, pair.count == 2 {
                rule = pair[0]; actions = pair[1]
            } else {
                rule = entry["rule"]; actions = entry["actions"]
            }
            guard let name = rule?["name"]?.string else { return nil }
            let acts: [TaggedValue] = (actions?.array ?? []).compactMap { a in
                guard let o = a.object else { return nil }
                var t = TaggedValue(tagKey: "action", tag: o["action"]?.string ?? "")
                t.fields = o
                return t
            }
            return RuleTestResult(ruleName: name, actions: acts)
        }
    }
}
