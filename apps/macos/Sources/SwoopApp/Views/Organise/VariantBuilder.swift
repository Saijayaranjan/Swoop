import AppKit
import SwoopKit
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
        TextField("", text: $text, prompt: Text("e.g. 100 MB"))
            .labelsHidden()
            .multilineTextAlignment(.trailing)
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

/// Shared list + detail layout for configuration panes: a card list with add/remove on the left
/// and the editor for the selected item on the right. The first item is selected automatically.
struct MasterDetail<Item: Identifiable, Row: View, Detail: View>: View where Item.ID == String {
    let items: [Item]
    @Binding var selection: String?
    let onAdd: () -> Void
    let onRemove: ((String) -> Void)?
    @ViewBuilder let row: (Item) -> Row
    @ViewBuilder let detail: () -> Detail

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            if !items.isEmpty { list }
            detail()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .padding(.leading, 20)
        .padding(.top, 10)
        .padding(.bottom, 16)
        .onAppear { if selection == nil { selection = items.first?.id } }
        .onChange(of: items.map(\.id)) { _, ids in
            if let s = selection, ids.contains(s) { return }
            if selection != nil || !ids.isEmpty { selection = ids.first }
        }
    }

    private var list: some View {
            VStack(spacing: 0) {
                HStack(spacing: 8) {
                    Text("\(items.count) \(L10n.tr(items.count == 1 ? "item" : "items"))")
                        .font(.system(size: 11, weight: .semibold))
                        .tracking(0.8)
                        .foregroundStyle(.secondary)
                        .textCase(.uppercase)
                    Spacer()
                    HStack(spacing: 0) {
                        Button(action: onAdd) {
                            Image(systemName: "plus").font(.system(size: 12, weight: .bold)).frame(width: 30, height: 26).contentShape(Rectangle())
                        }
                        .help("Add")
                        .accessibilityLabel(Text("Add"))
                        Rectangle().fill(Theme.hairline).frame(width: 1, height: 14)
                        Button { if let s = selection { onRemove?(s) } } label: {
                            Image(systemName: "minus").font(.system(size: 12, weight: .bold)).frame(width: 30, height: 26).contentShape(Rectangle())
                        }
                        .disabled(selection == nil || onRemove == nil)
                        .help("Remove")
                        .accessibilityLabel(Text("Remove"))
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(Theme.blue)
                    .background(Theme.well, in: Capsule())
                    .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                Rectangle().fill(Theme.hairline).frame(height: 1)
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(items) { item in
                            MasterListRow(selected: selection == item.id) { row(item) }
                                .onTapGesture { withAnimation(.easeOut(duration: 0.15)) { selection = item.id } }
                        }
                    }
                    .padding(8)
                }
                .scrollIndicators(.never)
            }
            .frame(width: 260)
            .frame(maxHeight: .infinity)
            .cardSurface(cornerRadius: 18, padding: 0)
    }
}

private struct MasterListRow<Content: View>: View {
    let selected: Bool
    @ViewBuilder let content: Content
    @ViewState private var hovering = false
    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        content
            .padding(.horizontal, 8)
            .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
            .background {
                if selected {
                    shape.fill(Theme.rowSelected).overlay(shape.strokeBorder(Theme.blue.opacity(0.3), lineWidth: 1))
                } else if hovering {
                    shape.fill(Theme.rowHover)
                }
            }
            .contentShape(shape)
            .onHover { hovering = $0 }
            .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// A list row with a tinted symbol tile, a title and a secondary line.
struct MasterRow: View {
    var symbol: String
    var tint: Color
    var title: String
    var subtitle: String?
    var subtitleTint: Color? = nil
    var dimmed = false
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 13, weight: .semibold))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(tint)
                .frame(width: 32, height: 32)
                .background(LinearGradient(colors: [tint.opacity(0.22), tint.opacity(0.10)], startPoint: .topLeading, endPoint: .bottomTrailing),
                            in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.system(size: 13.5, weight: .semibold)).lineLimit(1)
                    .foregroundStyle(dimmed ? .secondary : .primary)
                if let subtitle {
                    Text(subtitle).font(.system(size: 11.5)).lineLimit(1)
                        .foregroundStyle(subtitleTint.map { AnyShapeStyle($0) } ?? AnyShapeStyle(.secondary))
                }
            }
            Spacer(minLength: 0)
        }
        .opacity(dimmed ? 0.75 : 1)
    }
}

/// A palette of symbol tiles to pick an icon from.
struct IconGrid: View {
    let icons: [String]
    @Binding var selection: String
    var tint: Color = Theme.blue
    var body: some View {
        LazyVGrid(columns: Array(repeating: GridItem(.fixed(38), spacing: 8), count: 8), alignment: .leading, spacing: 8) {
            ForEach(icons, id: \.self) { icon in
                let on = selection == icon
                Button { selection = icon } label: {
                    Image(systemName: icon)
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(on ? .white : tint)
                        .frame(width: 38, height: 38)
                        .background(on ? AnyShapeStyle(tint.gradient) : AnyShapeStyle(tint.opacity(0.12)),
                                    in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(Text(icon))
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
    }
}

/// A colour for a category, from its stored hex colour or its symbol.
enum CategoryTint {
    static func of(_ c: CategoryData) -> Color {
        if let hex = c.color?.trimmingCharacters(in: CharacterSet(charactersIn: "#")), hex.count == 6, let v = UInt32(hex, radix: 16) {
            return Color(light: v, dark: v)
        }
        return of(symbol: c.icon)
    }
    static func of(symbol: String) -> Color {
        switch symbol {
        case let s where s.contains("film") || s.contains("video"): return Theme.fileTint(name: "x.mp4")
        case let s where s.contains("music") || s.contains("waveform"): return Theme.fileTint(name: "x.mp3")
        case let s where s.contains("photo") || s.contains("paintpalette"): return Theme.fileTint(name: "x.png")
        case let s where s.contains("archivebox"): return Theme.fileTint(name: "x.zip")
        case let s where s.contains("app") || s.contains("shippingbox") || s.contains("gamecontroller"): return Theme.fileTint(name: "x.dmg")
        case let s where s.contains("network") || s.contains("point.3"): return Theme.fileTint(name: "x", kind: .torrent)
        case let s where s.contains("book") || s.contains("richtext"): return Theme.fileTint(name: "x.pdf")
        default: return Theme.blue
        }
    }
}

/// A form row: title on the left, the current value and a stepper on the right.
struct StepperRow<V: Strideable>: View where V.Stride: ExpressibleByIntegerLiteral {
    let title: LocalizedStringKey
    @Binding var value: V
    let range: ClosedRange<V>
    let step: V.Stride
    let display: String

    init(_ title: LocalizedStringKey, value: Binding<V>, in range: ClosedRange<V>, step: V.Stride = 1, display: String) {
        self.title = title
        _value = value
        self.range = range
        self.step = step
        self.display = display
    }

    var body: some View {
        LabeledContent {
            HStack(spacing: 8) {
                Text(display)
                    .font(.system(.body, design: .rounded).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .contentTransition(.numericText())
                Stepper("", value: $value, in: range, step: step).labelsHidden()
            }
        } label: {
            Text(title)
        }
    }
}
