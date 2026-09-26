import AppKit
import SwiftUI
import SwoopKit

/// The Software Update window: what's new, download and verification progress, and the choice
/// to install now, later, or skip the version. Also shows the result of a manual check.
struct UpdateWindowView: View {
    let updates: UpdateController
    @Environment(AppModel.self) private var model

    var body: some View {
        ZStack {
            WindowWash()
            content
                .padding(.horizontal, 30)
                .padding(.top, 38)
                .padding(.bottom, 26)
        }
        .frame(width: 600)
        .fixedSize(horizontal: false, vertical: true)
        .ignoresSafeArea()
        .animation(.smooth(duration: 0.25), value: updates.phase)
    }

    @ViewBuilder
    private var content: some View {
        switch updates.phase {
        case .checking where updates.info?.available != true, .idle where updates.info?.available != true:
            ResultCard(symbol: nil, tint: Theme.blue,
                       title: "Checking for updates…",
                       detail: "Looking for a newer version of Swoop on GitHub.") {
                Button("Cancel") { updates.later() }.buttonStyle(SecondaryCapsuleStyle())
            }
        case .upToDate:
            ResultCard(symbol: "checkmark.circle.fill", tint: Theme.success,
                       title: "You're up to date",
                       detail: String(format: L10n.tr("Swoop %@ is the newest version available."), updates.currentVersion)) {
                Button("OK") { updates.later() }.buttonStyle(ProminentCapsuleStyle()).keyboardShortcut(.defaultAction)
            }
        case .couldNotCheck(let message):
            ResultCard(symbol: "wifi.exclamationmark", tint: Theme.warning,
                       title: "Couldn't check for updates",
                       detail: message) {
                Button("Close") { updates.later() }.buttonStyle(SecondaryCapsuleStyle())
                Button("Try Again") { updates.checkNow(showingWindow: true) }
                    .buttonStyle(ProminentCapsuleStyle()).keyboardShortcut(.defaultAction)
            }
        default:
            if let info = updates.info {
                available(info)
            }
        }
    }

    // MARK: update available

    private func available(_ info: UpdateInfoData) -> some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(alignment: .center, spacing: 18) {
                ZStack {
                    Circle()
                        .fill(RadialGradient(colors: [Theme.blue.opacity(0.32), .clear], center: .center, startRadius: 2, endRadius: 58))
                        .frame(width: 116, height: 116)
                    Image(nsImage: NSApp.applicationIconImage)
                        .resizable()
                        .interpolation(.high)
                        .frame(width: 76, height: 76)
                        .shadow(color: .black.opacity(0.16), radius: 10, y: 6)
                }
                .frame(width: 84, height: 84)
                VStack(alignment: .leading, spacing: 6) {
                    Text("A new version of Swoop is ready")
                        .font(.system(size: 23, weight: .bold))
                    Text(String(format: L10n.tr("Swoop %@ is available. You have %@."), info.latestVersion ?? "", updates.currentVersion))
                        .font(.system(size: 13.5))
                        .foregroundStyle(.secondary)
                    HStack(spacing: 6) {
                        if let date = info.publishedAt {
                            MiniChip(symbol: "calendar", text: date.formatted(date: .abbreviated, time: .omitted))
                        }
                        if let size = info.size {
                            MiniChip(symbol: "arrow.down.circle", text: Fmt.bytes(size))
                        }
                        if info.prerelease {
                            MiniChip(symbol: "flask", text: L10n.tr("Beta"), tint: Theme.warning)
                        }
                    }
                    .padding(.top, 2)
                }
            }

            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text(L10n.tr("What's new").uppercased())
                        .font(Theme.cardLabel).tracking(Theme.cardLabelTracking)
                        .foregroundStyle(.secondary)
                    Spacer()
                    if let url = updates.releasesPageURL {
                        Link(destination: url) {
                            Label("View on GitHub", systemImage: "arrow.up.right").font(.system(size: 12, weight: .medium))
                        }
                    }
                }
                ScrollView {
                    ReleaseNotesView(markdown: info.notes?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
                                     ?? L10n.tr("This release has no notes."))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.trailing, 6)
                }
                .scrollIndicators(.automatic)
                .frame(minHeight: 110, maxHeight: 250)
            }
            .cardSurface(cornerRadius: 18, padding: 18)

            UpdateStatusLine(phase: updates.phase)

            if model.stats.active > 0 {
                Label(String(format: L10n.tr("%d active downloads will pause and pick up again after Swoop relaunches."), model.stats.active),
                      systemImage: "pause.circle")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 10) {
                Button("Skip This Version") { updates.skipThisVersion() }
                    .buttonStyle(.plain)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(.secondary)
                    .disabled(updates.phase == .installing)
                Spacer()
                Button("Later") { updates.later() }
                    .buttonStyle(SecondaryCapsuleStyle())
                    .keyboardShortcut(.cancelAction)
                Button(isFailed ? "Try Again" : "Install and Relaunch") { updates.installAndRelaunch() }
                    .buttonStyle(ProminentCapsuleStyle())
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canInstall)
            }
        }
    }

    private var isFailed: Bool {
        if case .failed = updates.phase { return true }
        return false
    }

    private var canInstall: Bool {
        switch updates.phase {
        case .ready, .staged, .failed: return true
        default: return false
        }
    }
}

// MARK: - Pieces

/// Download → verify → install progress, one line.
private struct UpdateStatusLine: View {
    let phase: UpdateController.Phase

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 9) {
                icon
                Text(title)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                if case .downloading(let received, let total) = phase, let total, total > 0 {
                    Text("\(Fmt.bytes(received)) of \(Fmt.bytes(total))")
                        .font(.system(size: 12).monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
            if case .downloading(let received, let total) = phase {
                ProgressView(value: Double(received), total: Double(max(total ?? 0, max(received, 1))))
                    .progressViewStyle(.linear)
                    .tint(Theme.blue)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 11)
        .background(Theme.well.opacity(0.7), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Theme.hairline, lineWidth: 1))
    }

    @ViewBuilder private var icon: some View {
        switch phase {
        case .ready, .staged:
            Image(systemName: "checkmark.seal.fill").foregroundStyle(Theme.success)
        case .failed:
            Image(systemName: "xmark.octagon.fill").foregroundStyle(Theme.danger)
        case .downloading:
            Image(systemName: "arrow.down.circle.fill").foregroundStyle(Theme.blue)
        default:
            ProgressView().controlSize(.small)
        }
    }

    private var title: String {
        switch phase {
        case .downloading: return L10n.tr("Downloading…")
        case .verifying: return L10n.tr("Verifying the signature…")
        case .ready: return L10n.tr("Downloaded and verified: signed with Swoop's release key.")
        case .installing: return L10n.tr("Checking and preparing the new version…")
        case .staged: return L10n.tr("Verified and ready. It installs when you quit Swoop.")
        case .failed(let message): return String(format: L10n.tr("The update wasn't installed: %@"), message)
        default: return L10n.tr("Preparing the download…")
        }
    }
}

/// Centered icon, title, detail and buttons for the "checking", "up to date" and "couldn't
/// check" states.
private struct ResultCard<Buttons: View>: View {
    let symbol: String?
    let tint: Color
    let title: LocalizedStringKey
    let detail: String
    @ViewBuilder var buttons: Buttons

    var body: some View {
        VStack(spacing: 14) {
            ZStack {
                Circle()
                    .fill(RadialGradient(colors: [tint.opacity(0.28), .clear], center: .center, startRadius: 2, endRadius: 64))
                    .frame(width: 128, height: 128)
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .interpolation(.high)
                    .frame(width: 80, height: 80)
                    .shadow(color: .black.opacity(0.16), radius: 10, y: 6)
                    .overlay(alignment: .bottomTrailing) {
                        if let symbol {
                            Image(systemName: symbol)
                                .font(.system(size: 24, weight: .semibold))
                                .foregroundStyle(.white, tint)
                                .background(Circle().fill(Theme.card).padding(2))
                                .offset(x: 6, y: 6)
                        } else {
                            ProgressView().controlSize(.small)
                                .padding(6)
                                .background(Circle().fill(Theme.card))
                                .offset(x: 6, y: 6)
                        }
                    }
            }
            .frame(height: 104)
            Text(title)
                .font(.system(size: 22, weight: .bold))
            Text(detail)
                .font(.system(size: 13.5))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 420)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 10) { buttons }
                .padding(.top, 8)
        }
        .frame(maxWidth: .infinity)
    }
}

private struct MiniChip: View {
    let symbol: String
    let text: String
    var tint: Color = .secondary

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: symbol).font(.system(size: 10, weight: .semibold)).foregroundStyle(tint)
            Text(text).font(.system(size: 11.5, weight: .medium))
        }
        .padding(.horizontal, 9)
        .frame(height: 22)
        .background(Theme.well, in: Capsule())
        .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
    }
}

// MARK: - Release notes

/// Renders a GitHub release body: headings, bullet and numbered lists, quotes, code blocks and
/// paragraphs as blocks, with inline Markdown (bold, italics, code, links) through
/// `AttributedString`.
struct ReleaseNotesView: View {
    let markdown: String

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            ForEach(Array(ReleaseNotes.parse(markdown).enumerated()), id: \.offset) { _, block in
                view(for: block)
            }
        }
        .textSelection(.enabled)
    }

    @ViewBuilder
    private func view(for block: ReleaseNotes.Block) -> some View {
        switch block {
        case .heading(let level, let text):
            Text(text)
                .font(.system(size: level <= 1 ? 17 : level == 2 ? 15 : 13.5, weight: .semibold))
                .padding(.top, 5)
        case .bullet(let text, let depth):
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Circle().fill(Theme.blue.opacity(depth == 0 ? 0.85 : 0.5)).frame(width: 5, height: 5)
                    .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                Text(text).font(.system(size: 13)).fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, CGFloat(depth) * 16)
        case .numbered(let marker, let text, let depth):
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(marker).font(.system(size: 13).monospacedDigit()).foregroundStyle(.secondary)
                Text(text).font(.system(size: 13)).fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, CGFloat(depth) * 16)
        case .quote(let text):
            Text(text)
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .padding(.leading, 10)
                .overlay(alignment: .leading) { Rectangle().fill(Theme.blue.opacity(0.4)).frame(width: 3) }
        case .code(let code):
            Text(code)
                .font(.system(size: 11.5, design: .monospaced))
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Theme.well, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        case .rule:
            Rectangle().fill(Theme.hairline).frame(height: 1).padding(.vertical, 4)
        case .paragraph(let text):
            Text(text).font(.system(size: 13)).fixedSize(horizontal: false, vertical: true)
        }
    }
}

enum ReleaseNotes {
    enum Block {
        case heading(Int, AttributedString)
        case bullet(AttributedString, depth: Int)
        case numbered(String, AttributedString, depth: Int)
        case quote(AttributedString)
        case code(String)
        case rule
        case paragraph(AttributedString)
    }

    static func inline(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(s)
    }

    static func parse(_ markdown: String) -> [Block] {
        var blocks: [Block] = []
        var paragraph: [String] = []
        var code: [String]?
        func flush() {
            if !paragraph.isEmpty {
                blocks.append(.paragraph(inline(paragraph.joined(separator: " "))))
                paragraph.removeAll()
            }
        }
        for raw in markdown.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n") {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("```") {
                if let c = code {
                    blocks.append(.code(c.joined(separator: "\n")))
                    code = nil
                } else {
                    flush()
                    code = []
                }
                continue
            }
            if code != nil { code?.append(raw); continue }
            let indent = raw.prefix { $0 == " " || $0 == "\t" }.count
            let depth = min(indent / 2, 3)
            if line.isEmpty { flush(); continue }
            if let hashes = line.firstIndex(where: { $0 != "#" }), line.hasPrefix("#"), line[hashes] == " " {
                flush()
                let level = line.distance(from: line.startIndex, to: hashes)
                blocks.append(.heading(level, inline(String(line[hashes...]).trimmingCharacters(in: .whitespaces))))
            } else if line == "---" || line == "***" || line == "___" {
                flush(); blocks.append(.rule)
            } else if line.hasPrefix("- ") || line.hasPrefix("* ") || line.hasPrefix("+ ") {
                flush(); blocks.append(.bullet(inline(String(line.dropFirst(2))), depth: depth))
            } else if let dot = line.firstIndex(of: "."), line[..<dot].allSatisfy(\.isNumber), !line[..<dot].isEmpty,
                      line[line.index(after: dot)...].hasPrefix(" ") {
                flush()
                blocks.append(.numbered(String(line[...dot]), inline(String(line[line.index(dot, offsetBy: 2)...])), depth: depth))
            } else if line.hasPrefix(">") {
                flush(); blocks.append(.quote(inline(String(line.dropFirst()).trimmingCharacters(in: .whitespaces))))
            } else {
                paragraph.append(line)
            }
        }
        if let c = code { blocks.append(.code(c.joined(separator: "\n"))) }
        flush()
        return blocks
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
