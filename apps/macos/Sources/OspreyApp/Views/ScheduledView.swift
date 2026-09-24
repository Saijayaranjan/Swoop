import OspreyKit
import SwiftUI

struct ScheduledView: View {
    @Environment(AppModel.self) private var model
    @Environment(UIState.self) private var ui
    @ViewState private var editing: ScheduleDoc?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Scheduled").font(.largeTitle.weight(.semibold))
                        Text("Schedules open download windows (nights, weekends) and can wait for conditions like AC power or an unmetered network.")
                            .font(.callout).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button { editing = ScheduleDoc(name: L10n.tr("New Schedule")) } label: { Label("New Schedule", systemImage: "plus") }
                        .ospreyGlassButton(prominent: true)
                }

                if model.schedules.isEmpty {
                    GlassCard {
                        EmptyStateView("calendar.badge.clock", title: "No schedules", message: "Create a schedule, then attach it to a queue or to individual downloads.")
                            .frame(height: 260)
                    }
                } else {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 300), spacing: 14)], spacing: 14) {
                        ForEach(model.schedules) { s in ScheduleCard(schedule: s) { editing = s } }
                    }
                }

                let waiting = model.tasks.items.filter { $0.state == .scheduled || $0.data.scheduleId != nil && !$0.state.isTerminal }
                GlassCard("Waiting for a window", symbol: "hourglass") {
                    if waiting.isEmpty {
                        Text("No downloads are waiting for a schedule.").font(.callout).foregroundStyle(.secondary)
                    }
                    ForEach(waiting) { item in
                        Button { ui.select(item.id) } label: {
                            HStack(spacing: 12) {
                                NameCell(item: item).frame(maxWidth: .infinity, alignment: .leading)
                                if let sid = item.data.scheduleId, let s = model.schedules.first(where: { $0.id == sid }) {
                                    Label(s.name, systemImage: "calendar").font(.caption).foregroundStyle(.secondary)
                                }
                                StatePill(item.state)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .padding(20)
        }
        .ospreySoftScrollEdge()
        .navigationTitle("Scheduled")
        .task {
            await model.load("schedules")
            await model.refreshDashboard()
        }
        .sheet(item: $editing) { s in
            ScheduleEditor(schedule: s).environment(model)
        }
    }
}

struct ScheduleCard: View {
    let schedule: ScheduleDoc
    let onEdit: () -> Void
    @Environment(AppModel.self) private var model

    var body: some View {
        let next = model.scheduledNext.first { $0.scheduleId == schedule.id }
        let queues = model.queues.filter { $0.scheduleId == schedule.id }
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Image(systemName: schedule.enabled ? "calendar.badge.clock" : "calendar")
                    .font(.title2).symbolRenderingMode(.hierarchical)
                    .foregroundStyle(schedule.enabled ? Theme.violet : .secondary)
                VStack(alignment: .leading, spacing: 1) {
                    Text(schedule.name).font(.headline)
                    Text(schedule.recurrenceSummary).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Toggle("", isOn: Binding(get: { schedule.enabled }, set: { v in
                    var s = schedule
                    s.enabled = v
                    Task { _ = await model.saveSchedule(s) }
                }))
                .toggleStyle(.switch)
                .labelsHidden()
            }
            if !schedule.conditions.isEmpty {
                Text(schedule.conditions.map { ActionSchemas.scheduleCondition.summary($0) }.joined(separator: " · "))
                    .font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                if let next {
                    Label("Next change \(Fmt.relative(next.at))", systemImage: "clock").font(.caption)
                }
                Spacer()
                if !queues.isEmpty {
                    Text(queues.map(\.name).joined(separator: ", ")).font(.caption).foregroundStyle(.secondary)
                }
                Button("Edit", action: onEdit).controlSize(.small).ospreyGlassButton()
            }
        }
        .padding(16)
        .ospreyGlass(.regular, cornerRadius: 22)
    }
}

struct ScheduleEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @ViewState private var s: ScheduleDoc

    init(schedule: ScheduleDoc) { _s = ViewState(wrappedValue: schedule) }

    private static let dayNames = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    HStack {
                        TextField("Schedule name", text: $s.name).textFieldStyle(.plain).font(.title2.weight(.semibold))
                        Toggle("Enabled", isOn: $s.enabled).toggleStyle(.switch)
                    }
                    GlassCard("When", symbol: "clock") {
                        Picker("Repeat", selection: Binding(get: { s.recurrence.tag("type") }, set: { setType($0) })) {
                            Text("Every day").tag("daily")
                            Text("On certain days").tag("weekly")
                            Text("Once").tag("once")
                            Text("Between two dates").tag("range")
                            Text("Always").tag("always")
                        }
                        .pickerStyle(.menu)
                        recurrenceControls
                    }
                    GlassCard("Only while", symbol: "checklist") {
                        VariantListEditor(family: ActionSchemas.scheduleCondition, items: $s.conditions, addTitle: "Add Condition",
                                          emptyText: "No conditions — the window opens on time.")
                    }
                    GlassCard("When the window opens", symbol: "play.circle") {
                        VariantListEditor(family: ActionSchemas.scheduleAction, items: $s.onStart, addTitle: "Add Action", emptyText: "Nothing extra.")
                    }
                    GlassCard("When the window closes", symbol: "stop.circle") {
                        VariantListEditor(family: ActionSchemas.scheduleAction, items: $s.onEnd, addTitle: "Add Action", emptyText: "Nothing extra.")
                    }
                    Toggle("Pause attached downloads and queues outside this window", isOn: $s.gateAttached)
                }
                .padding(22)
            }
            HStack {
                if model.schedules.contains(where: { $0.id == s.id }) {
                    Button("Delete", role: .destructive) {
                        Task {
                            await model.perform("Couldn't delete schedule") { try await model.engine.deleteSchedule(s.id) }
                            await model.load("schedules")
                            dismiss()
                        }
                    }
                }
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Save") { Task { if await model.saveSchedule(s) { dismiss() } } }
                    .keyboardShortcut(.defaultAction)
                    .ospreyGlassButton(prominent: true)
                    .disabled(s.name.isEmpty)
            }
            .padding(16)
            .background(.bar)
        }
        .frame(width: 620, height: 680)
        .task { await model.load("automations") }
    }

    @ViewBuilder
    private var recurrenceControls: some View {
        switch s.recurrence.tag("type") {
        case "daily", "weekly":
            if s.recurrence.tag("type") == "weekly" {
                HStack(spacing: 6) {
                    ForEach(0..<7, id: \.self) { d in
                        let on = days.contains(d)
                        Button(Self.dayNames[d]) {
                            var set = days
                            if on { set.removeAll { $0 == d } } else { set.append(d) }
                            s.recurrence["days"] = .array(set.sorted().map { .number(Double($0)) })
                        }
                        .buttonStyle(.plain)
                        .font(.caption.weight(.semibold))
                        .frame(width: 44, height: 28)
                        .background(on ? Theme.accent.opacity(0.3) : Color.primary.opacity(0.06), in: Capsule())
                        .accessibilityAddTraits(on ? .isSelected : [])
                    }
                }
            }
            HStack {
                DatePicker("From", selection: timeBinding("start"), displayedComponents: .hourAndMinute)
                DatePicker("to", selection: timeBinding("end"), displayedComponents: .hourAndMinute)
            }
            Text("Windows that end before they start run overnight. Same start and end means all day.")
                .font(.caption).foregroundStyle(.secondary)
        case "once":
            DatePicker("At", selection: millisBinding("at"))
            Text("The window stays open for 24 hours.").font(.caption).foregroundStyle(.secondary)
        case "range":
            DatePicker("From", selection: millisBinding("from"))
            DatePicker("Until", selection: millisBinding("to"))
        default:
            Text("The schedule is always open; use conditions to gate it.").font(.caption).foregroundStyle(.secondary)
        }
    }

    private var days: [Int] { (s.recurrence["days"]?.array ?? []).compactMap(\.int) }

    private func setType(_ type: String) {
        let now = Date().millis
        switch type {
        case "daily": s.recurrence = TaggedValue(tagKey: "type", tag: "daily", fields: ["start": .string("01:00"), "end": .string("07:00")])
        case "weekly": s.recurrence = TaggedValue(tagKey: "type", tag: "weekly", fields: ["days": .array([5, 6].map { .number($0) }), "start": .string("00:00"), "end": .string("00:00")])
        case "once": s.recurrence = TaggedValue(tagKey: "type", tag: "once", fields: ["at": .number(Double(now + 3_600_000))])
        case "range": s.recurrence = TaggedValue(tagKey: "type", tag: "range", fields: ["from": .number(Double(now)), "to": .number(Double(now + 86_400_000))])
        default: s.recurrence = TaggedValue(tagKey: "type", tag: "always")
        }
    }

    private func timeBinding(_ key: String) -> Binding<Date> {
        Binding(get: {
            let parts = (s.recurrence[key]?.string ?? "00:00").split(separator: ":").compactMap { Int($0) }
            return Calendar.current.date(bySettingHour: parts.first ?? 0, minute: parts.count > 1 ? parts[1] : 0, second: 0, of: Date()) ?? Date()
        }, set: { d in
            let c = Calendar.current.dateComponents([.hour, .minute], from: d)
            s.recurrence[key] = .string(String(format: "%02d:%02d", c.hour ?? 0, c.minute ?? 0))
        })
    }

    private func millisBinding(_ key: String) -> Binding<Date> {
        Binding(get: { Date(millis: Int64(s.recurrence[key]?.double ?? Double(Date().millis))) },
                set: { s.recurrence[key] = .number(Double($0.millis)) })
    }
}
