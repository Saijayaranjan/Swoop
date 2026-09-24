import AppKit
import OspreyKit
import SwiftUI

/// Visual builder for a list of tagged values (conditions or actions): add, reorder, remove, and
/// edit each variant's fields with typed controls.
struct VariantListEditor: View {
    let family: VariantFamily
    @Binding var items: [TaggedValue]
    var addTitle: String = "Add"
    var emptyText: String = "Nothing here yet."

    var body: some View {
        Group {
            if items.isEmpty {
                Text(LocalizedStringKey(emptyText)).foregroundStyle(.secondary)
            }
            ForEach($items) { $item in
                VariantRow(family: family, value: $item) {
                    items.removeAll { $0.id == item.id }
                } move: { delta in
                    guard let i = items.firstIndex(where: { $0.id == item.id }) else { return }
                    let j = i + delta
                    guard j >= 0, j < items.count else { return }
                    items.swapAt(i, j)
                }
            }
            Menu {
                ForEach(family.variants) { v in
                    Button { items.append(family.make(v.tag)) } label: {
                        Label(v.label + (v.requiresConsent ? " ⚠︎" : ""), systemImage: v.symbol)
                    }
                }
            } label: {
                Label(addTitle, systemImage: "plus.circle.fill")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
    }
}

struct VariantRow: View {
    let family: VariantFamily
    @Binding var value: TaggedValue
    let remove: () -> Void
    let move: (Int) -> Void
    @Environment(AppModel.self) private var model

    var body: some View {
        let spec = family.variant(for: value)
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: spec?.symbol ?? "questionmark.circle")
                    .foregroundStyle(spec?.requiresConsent == true ? AnyShapeStyle(Theme.warning) : AnyShapeStyle(.secondary))
                    .frame(width: 20)
                Picker("", selection: Binding(get: { value.tag(family.tagKey) }, set: { newTag in
                    // Switching kind resets fields to the new variant's defaults.
                    let id = value.id
                    value = family.make(newTag)
                    value.id = id
                })) {
                    ForEach(family.variants) { v in Text(v.label).tag(v.tag) }
                    if spec == nil { Text(value.tag(family.tagKey)).tag(value.tag(family.tagKey)) }
                }
                .labelsHidden()
                .fixedSize()
                Spacer()
                Button { move(-1) } label: { Image(systemName: "chevron.up") }.buttonStyle(.borderless).accessibilityLabel(Text("Move up"))
                Button { move(1) } label: { Image(systemName: "chevron.down") }.buttonStyle(.borderless).accessibilityLabel(Text("Move down"))
                Button(role: .destructive, action: remove) { Image(systemName: "trash") }.buttonStyle(.borderless).accessibilityLabel(Text("Remove"))
            }
            if let spec {
                ForEach(spec.fields) { f in
                    FieldEditor(field: f, value: Binding(get: { value[f.key] ?? f.defaultValue }, set: { value[f.key] = $0 }))
                }
                if spec.requiresConsent {
                    Label("Runs code on your Mac. It only runs after you grant consent, and editing it revokes consent.", systemImage: "exclamationmark.shield.fill")
                        .font(.caption)
                        .foregroundStyle(Theme.warning)
                }
            }
        }
        .padding(.vertical, 4)
    }
}

struct FieldEditor: View {
    let field: FieldSpec
    @Binding var value: JSONValue
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(alignment: field.kind == .multiline || field.kind == .headers ? .top : .firstTextBaseline) {
            Text(LocalizedStringKey(field.label)).font(.caption).foregroundStyle(.secondary).frame(width: 96, alignment: .trailing)
            control
        }
    }

    @ViewBuilder
    private var control: some View {
        switch field.kind {
        case .text, .url:
            TextField(field.placeholder, text: stringBinding).textFieldStyle(.roundedBorder)
        case .multiline:
            TextEditor(text: stringBinding)
                .font(.system(size: 12, design: .monospaced))
                .frame(minHeight: 70)
                .scrollContentBackground(.hidden)
                .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))
        case .stringList:
            TextField(field.placeholder.isEmpty ? "Comma-separated" : field.placeholder, text: Binding(
                get: { (value.array ?? []).compactMap(\.string).joined(separator: ", ") },
                set: { value = .array($0.split(separator: ",").map { .string($0.trimmingCharacters(in: .whitespaces)) }.filter { $0.string?.isEmpty == false }) }
            ))
            .textFieldStyle(.roundedBorder)
        case .bytes:
            BytesField(bytes: Binding(get: { UInt64(max(0, value.double ?? 0)) }, set: { value = .number(Double($0)) }))
        case .integer:
            TextField("0", value: Binding(get: { value.int ?? 0 }, set: { value = .number(Double($0)) }), format: .number)
                .textFieldStyle(.roundedBorder)
                .frame(width: 100)
        case .toggle:
            Toggle("", isOn: Binding(get: { value.bool ?? true }, set: { value = .bool($0) })).labelsHidden()
        case .path:
            HStack {
                TextField("Folder or file", text: stringBinding).textFieldStyle(.roundedBorder)
                Button("Choose…") {
                    let panel = NSOpenPanel()
                    panel.canChooseDirectories = true
                    panel.canChooseFiles = field.key == "program" || field.key == "path"
                    panel.canCreateDirectories = true
                    if panel.runModal() == .OK, let url = panel.url { value = .string(url.path) }
                }
                .controlSize(.small)
            }
        case .queue:
            Picker("", selection: stringBinding) {
                Text("Choose…").tag("")
                ForEach(model.queues) { Text($0.name).tag($0.id) }
            }
            .labelsHidden()
        case .category:
            Picker("", selection: stringBinding) {
                Text("Choose…").tag("")
                ForEach(model.categories) { Text($0.name).tag($0.id) }
            }
            .labelsHidden()
        case .automation:
            Picker("", selection: stringBinding) {
                Text("Choose…").tag("")
                ForEach(model.automations) { Text($0.name).tag($0.id) }
            }
            .labelsHidden()
            .task { if model.automations.isEmpty { await model.load("automations") } }
        case .priority:
            Picker("", selection: stringBinding) {
                ForEach(Priority.allCases, id: \.self) { Text($0.label).tag($0.rawValue) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
        case .trafficMode:
            Picker("", selection: stringBinding) {
                ForEach(TrafficMode.allCases, id: \.self) { Text($0.label).tag($0.rawValue) }
            }
            .labelsHidden()
        case .headers:
            TextEditor(text: Binding(
                get: { (value.object ?? [:]).sorted { $0.key < $1.key }.map { "\($0.key): \($0.value.string ?? "")" }.joined(separator: "\n") },
                set: { text in
                    var o: [String: JSONValue] = [:]
                    for line in text.split(whereSeparator: \.isNewline) {
                        let parts = line.split(separator: ":", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
                        if parts.count == 2, !parts[0].isEmpty { o[parts[0]] = .string(parts[1]) }
                    }
                    value = .object(o)
                }
            ))
            .font(.system(size: 12, design: .monospaced))
            .frame(minHeight: 44)
            .scrollContentBackground(.hidden)
            .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))
        }
    }

    private var stringBinding: Binding<String> {
        Binding(get: { value.string ?? "" }, set: { value = .string($0) })
    }
}

/// A human-friendly byte-size field ("2 MB").
struct BytesField: View {
    @Binding var bytes: UInt64
    @ViewState private var text = ""
    @FocusState private var focused: Bool
    var body: some View {
        TextField("e.g. 100 MB", text: $text)
            .textFieldStyle(.roundedBorder)
            .frame(width: 140)
            .focused($focused)
            .onAppear { text = bytes == 0 ? "" : Fmt.bytes(bytes) }
            .onSubmit(commit)
            .onChange(of: focused) { _, f in if !f { commit() } }
    }
    private func commit() {
        bytes = Fmt.parseBytes(text) ?? 0
        text = bytes == 0 ? "" : Fmt.bytes(bytes)
    }
}

/// Shared list + detail layout for configuration panes (Mail-rules style: list with +/− below).
struct MasterDetail<Item: Identifiable, Row: View, Detail: View>: View where Item.ID == String {
    let items: [Item]
    @Binding var selection: String?
    let onAdd: () -> Void
    let onRemove: ((String) -> Void)?
    @ViewBuilder let row: (Item) -> Row
    @ViewBuilder let detail: () -> Detail

    var body: some View {
        HStack(spacing: 0) {
            VStack(spacing: 0) {
                List(selection: $selection) {
                    ForEach(items) { item in row(item).tag(item.id) }
                }
                .listStyle(.inset)
                Divider()
                HStack(spacing: 0) {
                    Button(action: onAdd) { Image(systemName: "plus").frame(width: 24, height: 20) }
                        .accessibilityLabel(Text("Add"))
                    Divider().frame(height: 16)
                    Button { if let s = selection { onRemove?(s) } } label: { Image(systemName: "minus").frame(width: 24, height: 20) }
                        .disabled(selection == nil || onRemove == nil)
                        .accessibilityLabel(Text("Remove"))
                    Spacer()
                }
                .buttonStyle(.borderless)
                .padding(4)
            }
            .frame(width: 220)
            Divider()
            detail()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}
