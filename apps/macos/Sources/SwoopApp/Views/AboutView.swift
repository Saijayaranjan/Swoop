import AppKit
import SwoopKit
import SwiftUI

/// The About window: icon, version and a few engine facts on the window wash.
struct AboutView: View {
    @Environment(AppModel.self) private var model

    private var version: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? model.info.version }
    private var build: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? model.info.build }

    var body: some View {
        ZStack {
            WindowWash()
            VStack(spacing: 0) {
                ZStack {
                    Circle()
                        .fill(RadialGradient(colors: [Theme.blue.opacity(0.35), .clear], center: .center, startRadius: 4, endRadius: 110))
                        .frame(width: 220, height: 220)
                    Image(nsImage: NSApp.applicationIconImage)
                        .resizable()
                        .interpolation(.high)
                        .frame(width: 118, height: 118)
                        .shadow(color: .black.opacity(0.18), radius: 14, y: 8)
                }
                .frame(height: 170)
                .padding(.top, 34)
                Text("Swoop")
                    .font(.system(size: 32, weight: .bold, design: .rounded))
                Text("Version \(version) (\(build))")
                    .font(.system(size: 13).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .padding(.top, 2)
                Text("Fast, careful downloads for your Mac.")
                    .font(.system(size: 14))
                    .padding(.top, 12)
                VStack(spacing: 0) {
                    row("Engine", model.info.version.isEmpty ? "—" : "v\(model.info.version)")
                    Rectangle().fill(Theme.hairline).frame(height: 1)
                    row("Platform", model.info.arch.isEmpty ? "—" : "\(model.info.os) \(model.info.arch)")
                    Rectangle().fill(Theme.hairline).frame(height: 1)
                    row("Media tools", model.info.ffmpegAvailable ? L10n.tr("Available") : L10n.tr("Not installed"))
                }
                .cardSurface(cornerRadius: 16, padding: 0)
                .padding(.horizontal, 32)
                .padding(.top, 22)
                Spacer(minLength: 12)
                Text("© 2026 Swoop")
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
                    .padding(.bottom, 18)
            }
        }
        .ignoresSafeArea()
        .frame(width: 380, height: 520)
        .transparentTitleBar()
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack {
            Text(LocalizedStringKey(label)).foregroundStyle(.secondary)
            Spacer()
            Text(value).font(.system(size: 13, weight: .medium).monospacedDigit())
        }
        .font(.system(size: 13))
        .padding(.horizontal, 16)
        .frame(height: 40)
    }
}
