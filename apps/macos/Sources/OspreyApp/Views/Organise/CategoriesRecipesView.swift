import AppKit
import OspreyKit
import SwiftUI

struct CategoriesView: View {
    @Environment(AppModel.self) private var model
    @ViewState private var selection: String?
    @ViewState private var draft: CategoryData?

    var body: some View {
        MasterDetail(items: model.categories, selection: $selection,
                     onAdd: { draft = CategoryData(name: L10n.tr("New Category"), position: Int32(model.categories.count)); selection = nil },
                     onRemove: { id in
                         guard model.categories.first(where: { $0.id == id })?.builtin == false else {
                             model.toast(.info, "Built-in categories can't be removed")
                             return
                         }
                         Task {
                             await model.perform("Couldn't delete") { try await model.engine.deleteCategory(id) }
                             await model.reloadCategories()
                         }
                     }) { c in
            MasterRow(symbol: c.icon, tint: CategoryTint.of(c), title: c.name,
                      subtitle: c.extensions.isEmpty ? L10n.tr("Everything else") : c.extensions.prefix(6).joined(separator: ", "))
        } detail: {
            if let draft {
                CategoryEditor(category: draft) { self.draft = nil; selection = $0 }.id(draft.id)
            } else if let id = selection, let c = model.categories.first(where: { $0.id == id }) {
                CategoryEditor(category: c) { selection = $0 }.id(c.id + c.name + c.extensions.joined())
            } else {
                EmptyStateView("square.grid.2x2", title: "Categories", message: "Select a category to edit its file types and folder.")
            }
        }
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
                HStack(spacing: 14) {
                    Image(systemName: c.icon)
                        .font(.system(size: 22, weight: .semibold))
                        .foregroundStyle(.white)
                        .frame(width: 52, height: 52)
                        .background(CategoryTint.of(c).gradient, in: RoundedRectangle(cornerRadius: 15, style: .continuous))
                        .shadow(color: CategoryTint.of(c).opacity(0.35), radius: 8, y: 3)
                    VStack(alignment: .leading, spacing: 3) {
                        TextField("", text: $c.name, prompt: Text("Name")).labelsHidden().textFieldStyle(.plain).font(.system(size: 20, weight: .bold))
                        Text(c.builtin ? L10n.tr("Built-in category") : L10n.tr("Your category"))
                            .font(.system(size: 12)).foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 4)
                IconGrid(icons: Self.icons, selection: $c.icon, tint: CategoryTint.of(c))
                    .padding(.vertical, 2)
            }
            Section("Matches") {
                TextField("Extensions", text: Binding(
                    get: { c.extensions.joined(separator: ", ") },
                    set: { c.extensions = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces).lowercased().trimmingCharacters(in: CharacterSet(charactersIn: ".")) }.filter { !$0.isEmpty } }),
                    prompt: Text("pdf, zip, mp4"))
                TextField("MIME types", text: Binding(
                    get: { c.mimePrefixes.joined(separator: ", ") },
                    set: { c.mimePrefixes = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces).lowercased() }.filter { !$0.isEmpty } }),
                    prompt: Text("video/, audio/"))
                Text("Separate entries with commas.").font(.caption).foregroundStyle(.secondary)
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
                    .buttonStyle(ProminentCapsuleStyle())
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
                     onAdd: { draft = RecipeDoc(name: L10n.tr("New Recipe")); selection = nil },
                     onRemove: { id in
                         Task {
                             await model.perform("Couldn't delete recipe") { try await model.engine.deleteRecipe(id) }
                             await model.load("recipes")
                         }
                     }) { r in
            MasterRow(symbol: r.icon, tint: CategoryTint.of(symbol: r.icon), title: r.name,
                      subtitle: r.tags.isEmpty ? (model.queue(r.queueId)?.name ?? L10n.tr("Default queue")) : r.tags.joined(separator: ", "))
        } detail: {
            if let draft {
                RecipeEditor(recipe: draft) { self.draft = nil; selection = $0 }.id(draft.id)
            } else if let id = selection, let r = model.recipes.first(where: { $0.id == id }) {
                RecipeEditor(recipe: r) { selection = $0 }.id(r.id + String(r.updatedAt))
            } else {
                EmptyStateView("wand.and.stars", title: "No recipes yet", message: "A recipe bundles a folder, queue, category, tags and actions — like “PDFs → Documents, tagged Work”.") {
                    Button("New Recipe") { draft = RecipeDoc(name: L10n.tr("New Recipe")) }.buttonStyle(ProminentCapsuleStyle())
                }
            }
        }
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
        Form {
            Section {
                HStack(spacing: 14) {
                    Image(systemName: r.icon)
                        .font(.system(size: 22, weight: .semibold))
                        .foregroundStyle(.white)
                        .frame(width: 52, height: 52)
                        .background(CategoryTint.of(symbol: r.icon).gradient, in: RoundedRectangle(cornerRadius: 15, style: .continuous))
                    TextField("", text: $r.name, prompt: Text("Recipe name")).labelsHidden().textFieldStyle(.plain).font(.system(size: 20, weight: .bold))
                }
                .padding(.vertical, 4)
                IconGrid(icons: Self.icons, selection: $r.icon, tint: CategoryTint.of(symbol: r.icon))
            }
            Section("Defaults") {
                LabeledContent("Folder") {
                    HStack {
                        Text(r.directory?.abbreviatedPath ?? L10n.tr("Default")).lineLimit(1).foregroundStyle(.secondary)
                        Button("Choose…") {
                            let panel = NSOpenPanel()
                            panel.canChooseDirectories = true
                            panel.canChooseFiles = false
                            panel.canCreateDirectories = true
                            if panel.runModal() == .OK { r.directory = panel.url?.path }
                        }
                    }
                }
                Picker("Queue", selection: Binding(get: { r.queueId ?? "" }, set: { r.queueId = $0.isEmpty ? nil : $0 })) {
                    Text("Default").tag("")
                    ForEach(model.queues) { Text($0.name).tag($0.id) }
                }
                Picker("Category", selection: Binding(get: { r.categoryId ?? "" }, set: { r.categoryId = $0.isEmpty ? nil : $0 })) {
                    Text("Automatic").tag("")
                    ForEach(model.categories) { Text($0.name).tag($0.id) }
                }
                TextField("Tags", text: Binding(get: { r.tags.joined(separator: ", ") },
                                                set: { r.tags = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty } }),
                          prompt: Text("Comma-separated"))
                Picker("Automation", selection: Binding(get: { r.automationId ?? "" }, set: { r.automationId = $0.isEmpty ? nil : $0 })) {
                    Text("None").tag("")
                    ForEach(model.automations) { Text($0.name).tag($0.id) }
                }
                StepperRow("Connections", value: Binding(get: { r.options["max_connections"]?.int ?? 0 },
                                                         set: { r.options["max_connections"] = $0 == 0 ? .null : .number(Double($0)) }),
                           in: 0...64, display: (r.options["max_connections"]?.int).map { "\($0)" } ?? L10n.tr("Automatic"))
            }
            Section("Also apply these actions") {
                VariantListEditor(family: ActionSchemas.ruleAction, items: $r.ruleActions, addTitle: "Add Action", emptyText: "No extra actions.")
            }
            Section {
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
                        .buttonStyle(ProminentCapsuleStyle())
                        .disabled(r.name.isEmpty)
                }
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
    }
}
