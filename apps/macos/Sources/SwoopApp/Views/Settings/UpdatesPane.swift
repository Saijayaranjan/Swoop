import AppKit
import SwiftUI
import SwoopKit

/// Settings → Updates: status, "Check Now", and the three update preferences.
struct UpdatesPane: View {
    let b: SettingsBindings
    @Environment(AppModel.self) private var model

    var body: some View {
        Form {
            if let updates = UpdateController.shared {
                Section { hero(updates) }
            }
            Section {
                Toggle("Automatically check for updates", isOn: b.bool("updates.check_automatically", true))
                Toggle("Automatically download and install", isOn: b.bool("updates.install_automatically"))
                    .disabled(!model.settings.bool("updates.check_automatically", default: true))
                Toggle("Include beta releases", isOn: Binding(
                    get: { model.settings.string("updates.channel", default: "stable") == "beta" },
                    set: { model.setSetting("updates.channel", .string($0 ? "beta" : "stable")) }
                ))
            } footer: {
                Text(model.settings.bool("updates.install_automatically")
                     ? "New versions download in the background and install the next time you quit Swoop."
                     : "New versions download in the background, then Swoop asks before installing.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section {
                LabeledContent("Current version", value: UpdateController.shared?.currentVersion ?? model.info.version)
                LabeledContent("Last checked", value: lastCheckedText)
                if let url = UpdateController.shared?.releasesPageURL {
                    LabeledContent("Release notes") {
                        Link("GitHub Releases", destination: url)
                    }
                }
                Label("Every update is checked against Swoop's Ed25519 release key before anything is installed.",
                      systemImage: "checkmark.shield")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var lastCheckedText: String {
        guard let date = UpdateController.shared?.lastCheckDate else { return L10n.tr("Never") }
        if Date().timeIntervalSince(date) < 60 { return L10n.tr("Just now") }
        return RelativeDateTimeFormatter().localizedString(for: date, relativeTo: Date())
    }

    private func hero(_ updates: UpdateController) -> some View {
        HStack(spacing: 16) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .interpolation(.high)
                .frame(width: 64, height: 64)
                .shadow(color: .black.opacity(0.12), radius: 6, y: 3)
            VStack(alignment: .leading, spacing: 4) {
                Text("Swoop \(updates.currentVersion)")
                    .font(.system(size: 20, weight: .bold))
                status(updates)
                    .font(.system(size: 13))
            }
            Spacer(minLength: 12)
            if updates.info?.available == true, updates.phase != .checking {
                Button("View Update…") { updates.showWindow() }
                    .swoopGlassButton(prominent: true)
            } else {
                Button("Check Now") { updates.checkNow(showingWindow: false) }
                    .swoopGlassButton()
                    .disabled(updates.phase == .checking)
            }
        }
        .padding(.vertical, 6)
    }

    @ViewBuilder
    private func status(_ updates: UpdateController) -> some View {
        switch updates.phase {
        case .checking:
            HStack(spacing: 6) {
                ProgressView().controlSize(.mini)
                Text("Checking for updates…").foregroundStyle(.secondary)
            }
        case .upToDate:
            Label("You're up to date", systemImage: "checkmark.circle.fill")
                .foregroundStyle(Theme.success)
        case .couldNotCheck(let message):
            Label(String(format: L10n.tr("Couldn't check: %@"), message), systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(Theme.warning)
                .lineLimit(2)
        case .downloading(let received, let total):
            Label(String(format: L10n.tr("Downloading Swoop %@… %@"), updates.info?.latestVersion ?? "",
                         total.map { "\(Fmt.bytes(received)) / \(Fmt.bytes($0))" } ?? Fmt.bytes(received)),
                  systemImage: "arrow.down.circle.fill")
                .foregroundStyle(Theme.blue)
        case .ready, .staged, .installing, .verifying, .available:
            Label(String(format: L10n.tr("Swoop %@ is available"), updates.info?.latestVersion ?? ""),
                  systemImage: "sparkles")
                .foregroundStyle(Theme.blue)
        case .failed(let message):
            Label(message, systemImage: "xmark.octagon.fill")
                .foregroundStyle(Theme.danger)
                .lineLimit(2)
        case .idle:
            Text("Updates are checked quietly in the background.")
                .foregroundStyle(.secondary)
        }
    }
}
