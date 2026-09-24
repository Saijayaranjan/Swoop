import AppKit
import OspreyKit
import SwiftUI

struct CategoriesView: View {
    @Environment(AppModel.self) private var model
    @ViewState private var selection: String?
    @ViewState private var draft: CategoryData?

    var body: some View {
        MasterDetail(items: model.categories, selection: $selection,
                     onAdd: { draft = CategoryData(name: L10n.tr("New Category"), position: Int32(model.categories.count)); selection = nil }, onRemove: nil) { c in
            HStack {
                Image(systemName: c.icon).symbolRenderingMode(.hierarchical).foregroundStyle(Theme.accent).frame(width: 20)
                VStack(alignment: .leading, spacing: 1) {
                    Text(c.name)
                    Text(c.extensions.prefix(6).joined(separator: ", ")).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
        } detail: {
            if let draft {
                CategoryEditor(category: draft) { self.draft = nil; selection = $0 }.id(draft.id)
            } else if let id = selection, let c = model.categories.first(where: { $0.id == id }) {
                CategoryEditor(category: c) { selection = $0 }.id(c.id + c.name + c.extensions.joined())
            } else {
                EmptyStateView("square.grid.2x2", title: "Categories", message: "Select a category to edit its file types and folder.")
            }
        }
        .navigationTitle("Categories")
    }
}

struct CategoryEditor: View {
    @Environment(AppModel.self) private var model
    @ViewState private var c: CategoryData
    let onSaved: (String?) -> Void
    static let icons = ["folder", "doc.text", "photo", "film", "music.note", "archivebox", "app.badge", "network", "shippingbox", "book.closed", "gamecontroller", "paintpalette"]

    init(category: CategoryData, onSaved: @escaping (String?) -> Void) {
        _c = ViewState(wrappedValue: category)
        self.onSaved = onSaved
    }

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $c.name)
                Picker("Icon", selection: $c.icon) {
                    ForEach(Self.icons, id: \.self) { Image(systemName: $0).tag($0) }
                }
                .pickerStyle(.segmented)
            }
            Section("Matches") {
                TextField("Extensions (comma-separated)", text: Binding(
                    get: { c.extensions.joined(separator: ", ") },
                    set: { c.extensions = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces).lowercased().trimmingCharacters(in: CharacterSet(charactersIn: ".")) }.filter { !$0.isEmpty } }))
                TextField("MIME prefixes (comma-separated)", text: Binding(
                    get: { c.mimePrefixes.joined(separator: ", ") },
                    set: { c.mimePrefixes = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces).lowercased() }.filter { !$0.isEmpty } }))
            }
            Section("Folder") {
                HStack {
                    Text(c.directory.map { $0.isEmpty ? L10n.tr("Download folder") : $0.abbreviatedPath } ?? L10n.tr("Download folder"))
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Choose…") {
                        let panel = NSOpenPanel()
                        panel.canChooseDirectories = true
                        panel.canChooseFiles = false
                        panel.canCreateDirectories = true
                        if panel.runModal() == .OK { c.directory = panel.url?.path }
                    }
                }
                Text("Relative names (like “Videos”) are created inside your download folder when “Organise by category” is on.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section {
                HStack {
                    if !c.builtin, model.categories.contains(where: { $0.id == c.id }) {
                        Button("Delete Category", role: .destructive) {
                            Task {
                                await model.perform("Couldn't delete") { try await model.engine.deleteCategory(c.id) }
                                await model.reloadCategories()
                                onSaved(nil)
                            }
                        }
                    }
                    Spacer()
                    Button("Save") {
                        Task {
                            if let saved = await model.perform("Couldn't save category", { try await model.engine.saveCategory(c) }) {
                                await model.reloadCategories()
                                onSaved(saved.id)
                            }
                        }
                    }
                    .ospreyGlassButton(prominent: true)
                    .disabled(c.name.isEmpty)
                }
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
    }
}

// MARK: - Recipes

struct RecipesView: View {
    @Environment(AppModel.self) private var model
    @ViewState private var selection: String?
    @ViewState private var draft: RecipeDoc?

    var body: some View {
        MasterDetail(items: model.recipes, selection: $selection,
                     onAdd: { draft = RecipeDoc(name: L10n.tr("New Recipe")); selection = nil }, onRemove: nil) { r in
            Label(r.name, systemImage: r.icon)
        } detail: {
            if let draft {
                RecipeEditor(recipe: draft) { self.draft = nil; selection = $0 }.id(draft.id)
            } else if let id = selection, let r = model.recipes.first(where: { $0.id == id }) {
                RecipeEditor(recipe: r) { selection = $0 }.id(r.id + String(r.updatedAt))
            } else {
                EmptyStateView("wand.and.stars", title: "Recipes", message: "A recipe bundles a folder, queue, category, tags and actions — like “PDFs → Documents, tagged Work”.") {
                    Button("New Recipe") { draft = RecipeDoc(name: L10n.tr("New Recipe")) }.ospreyGlassButton(prominent: true)
                }
            }
        }
        .navigationTitle("Recipes")
        .task { await model.load("recipes"); await model.load("automations") }
    }
}

struct RecipeEditor: View {
    @Environment(AppModel.self) private var model
    @ViewState private var r: RecipeDoc
    let onSaved: (String?) -> Void
    static let icons = ["wand.and.stars", "doc.richtext", "film", "music.note", "photo", "archivebox", "briefcase", "graduationcap", "gamecontroller", "star"]

    init(recipe: RecipeDoc, onSaved: @escaping (String?) -> Void) {
        _r = ViewState(wrappedValue: recipe)
        self.onSaved = onSaved
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                TextField("Recipe name", text: $r.name).textFieldStyle(.plain).font(.title2.weight(.semibold))
                Picker("Icon", selection: $r.icon) {
                    ForEach(Self.icons, id: \.self) { Image(systemName: $0).tag($0) }
                }
                .pickerStyle(.segmented)
                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 10) {
                    GridRow {
                        Text("Folder").foregroundStyle(.secondary)
                        HStack {
                            Text(r.directory?.abbreviatedPath ?? L10n.tr("Default")).lineLimit(1)
                            Spacer()
                            Button("Choose…") {
                                let panel = NSOpenPanel()
                                panel.canChooseDirectories = true
                                panel.canChooseFiles = false
                                panel.canCreateDirectories = true
                                if panel.runModal() == .OK { r.directory = panel.url?.path }
                            }
                        }
                    }
                    GridRow {
                        Text("Queue").foregroundStyle(.secondary)
                        Picker("", selection: Binding(get: { r.queueId ?? "" }, set: { r.queueId = $0.isEmpty ? nil : $0 })) {
                            Text("Default").tag("")
                            ForEach(model.queues) { Text($0.name).tag($0.id) }
                        }.labelsHidden()
                    }
                    GridRow {
                        Text("Category").foregroundStyle(.secondary)
                        Picker("", selection: Binding(get: { r.categoryId ?? "" }, set: { r.categoryId = $0.isEmpty ? nil : $0 })) {
                            Text("Automatic").tag("")
                            ForEach(model.categories) { Text($0.name).tag($0.id) }
                        }.labelsHidden()
                    }
                    GridRow {
                        Text("Tags").foregroundStyle(.secondary)
                        TextField("Comma-separated", text: Binding(get: { r.tags.joined(separator: ", ") },
                                                                   set: { r.tags = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty } }))
                            .textFieldStyle(.roundedBorder)
                    }
                    GridRow {
                        Text("Automation").foregroundStyle(.secondary)
                        Picker("", selection: Binding(get: { r.automationId ?? "" }, set: { r.automationId = $0.isEmpty ? nil : $0 })) {
                            Text("None").tag("")
                            ForEach(model.automations) { Text($0.name).tag($0.id) }
                        }.labelsHidden()
                    }
                    GridRow {
                        Text("Connections").foregroundStyle(.secondary)
                        Stepper(value: Binding(get: { r.options["max_connections"]?.int ?? 0 },
                                               set: { r.options["max_connections"] = $0 == 0 ? .null : .number(Double($0)) }), in: 0...64) {
                            Text((r.options["max_connections"]?.int).map { "\($0)" } ?? L10n.tr("Automatic"))
                        }
                    }
                }
                Text("Also apply these actions").font(.headline)
                VariantListEditor(family: ActionSchemas.ruleAction, items: $r.ruleActions, addTitle: "Add Action", emptyText: "No extra actions.")
                HStack {
                    if model.recipes.contains(where: { $0.id == r.id }) {
                        Button("Delete", role: .destructive) {
                            Task {
                                await model.perform("Couldn't delete recipe") { try await model.engine.deleteRecipe(r.id) }
                                await model.load("recipes")
                                onSaved(nil)
                            }
                        }
                    }
                    Spacer()
                    Button("Save") { Task { if await model.saveRecipe(r) { onSaved(r.id) } } }
                        .ospreyGlassButton(prominent: true)
                        .disabled(r.name.isEmpty)
                }
            }
            .padding(20)
        }
    }
}
