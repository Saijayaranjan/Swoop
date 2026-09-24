import AppKit
import CoreImage
import CoreImage.CIFilterBuiltins
import OspreyKit
import SwiftUI

struct DevicesView: View {
    @Environment(AppModel.self) private var model
    @ViewState private var audit: [AuditEntryData] = []
    @ViewState private var scopes: Set<DeviceScope> = [.read, .add, .control]
    @ViewState private var renaming: DeviceData?
    @ViewState private var newName = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Control Osprey from your phone or another computer. Remote access is encrypted and every device can be revoked.")
                    .font(.system(size: 13)).foregroundStyle(.secondary)
                if !model.settings.bool("remote.enabled") {
                    HStack {
                        Label("Remote access is off", systemImage: "wifi.slash").font(.callout.weight(.semibold))
                        Spacer()
                        Button("Turn On Remote Access") { model.setSetting("remote.enabled", .bool(true)) }
                            .ospreyGlassButton(prominent: true)
                    }
                    .cardSurface(cornerRadius: 18, padding: 14)
                }
                HStack(alignment: .top, spacing: 14) {
                    pairingCard.frame(maxWidth: 420)
                    devicesCard.frame(maxWidth: .infinity)
                }
                auditCard
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 16)
        }
        .scrollIndicators(.never)
        .ospreySoftScrollEdge()
        .task {
            await model.load("devices")
            await loadAudit()
        }
        .onChange(of: model.devices) { _, _ in Task { await loadAudit() } }
        .alert("Rename Device", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("Name", text: $newName)
            Button("Rename") {
                if let d = renaming {
                    let name = newName
                    Task {
                        await model.perform("Couldn't rename") { try await model.engine.renameDevice(d.id, name: name) }
                        await model.load("devices")
                    }
                }
            }
            Button("Cancel", role: .cancel) {}
        }
    }

    private var pairingCard: some View {
        GlassCard("Pair a device", symbol: "qrcode") {
            if let p = model.pairing {
                VStack(alignment: .center, spacing: 12) {
                    QRCodeView(text: p.url)
                        .frame(width: 200, height: 200)
                        .padding(12)
                        .background(.white, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                        .accessibilityLabel(Text("Pairing QR code"))
                    Text("Scan with your phone's camera, then enter this code:").font(.caption).foregroundStyle(.secondary)
                    Text(formatCode(p.code))
                        .font(.system(size: 30, weight: .semibold, design: .monospaced))
                        .kerning(3)
                        .textSelection(.enabled)
                    TimelineView(.periodic(from: .now, by: 1)) { ctx in
                        let left = max(0, Int((p.expiresAt - ctx.date.millis) / 1000))
                        Text(left > 0 ? "Expires in \(left / 60):\(String(format: "%02d", left % 60))" : "Expired")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(left > 20 ? AnyShapeStyle(.secondary) : AnyShapeStyle(Theme.warning))
                    }
                    Text(p.url).font(.caption2.monospaced()).foregroundStyle(.tertiary).lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                    if let fp = p.tlsFingerprint {
                        Text("Certificate \(fp.prefix(23))…").font(.caption2.monospaced()).foregroundStyle(.tertiary)
                    }
                    Button("Cancel Pairing") { Task { await model.cancelPairing() } }.ospreyGlassButton()
                }
                .frame(maxWidth: .infinity)
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Allow the new device to").font(.callout)
                    ForEach(DeviceScope.allCases, id: \.self) { s in
                        Toggle(isOn: Binding(get: { scopes.contains(s) }, set: { if $0 { scopes.insert(s) } else { scopes.remove(s) } })) {
                            VStack(alignment: .leading, spacing: 0) {
                                Text(s.label)
                                Text(scopeHelp(s)).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        .toggleStyle(.checkbox)
                    }
                    Button { Task { await model.startPairing(Array(scopes).sorted { $0.rawValue < $1.rawValue }) } } label: {
                        Label("Show Pairing Code", systemImage: "qrcode")
                    }
                    .ospreyGlassButton(prominent: true)
                    .disabled(scopes.isEmpty)
                    if let name = model.lastPairedDevice {
                        Label("\(name) was paired", systemImage: "checkmark.circle.fill").foregroundStyle(Theme.success).font(.caption)
                    }
                }
            }
        }
    }

    private var devicesCard: some View {
        GlassCard("Paired devices", symbol: "iphone.and.arrow.forward") {
            if model.devices.isEmpty {
                Text("No devices yet.").font(.callout).foregroundStyle(.secondary)
            }
            ForEach(model.devices) { d in
                HStack(spacing: 12) {
                    Image(systemName: symbol(d.kind)).font(.title2).symbolRenderingMode(.hierarchical)
                        .foregroundStyle(d.revoked ? AnyShapeStyle(.secondary) : AnyShapeStyle(Theme.accent))
                        .frame(width: 32)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(d.name).strikethrough(d.revoked)
                        Text(details(d)).font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    if !d.revoked {
                        Button("Rename") { newName = d.name; renaming = d }.buttonStyle(.borderless)
                        Button("Revoke", role: .destructive) {
                            Task {
                                await model.perform("Couldn't revoke") { try await model.engine.revokeDevice(d.id) }
                                await model.load("devices")
                            }
                        }
                        .ospreyGlassButton()
                    } else {
                        Text("Revoked").font(.caption).foregroundStyle(.secondary)
                    }
                }
                .controlSize(.small)
            }
        }
    }

    private var auditCard: some View {
        GlassCard("Activity", symbol: "list.bullet.rectangle.portrait") {
            if audit.isEmpty {
                Text("Remote requests and pairing attempts are logged here.").font(.callout).foregroundStyle(.secondary)
            }
            ForEach(audit.prefix(60)) { a in
                HStack(spacing: 10) {
                    Image(systemName: a.success ? "checkmark.circle" : "xmark.octagon").foregroundStyle(a.success ? Theme.success : Theme.danger)
                    Text(a.action).font(.caption.monospaced())
                    if let t = a.target { Text(t).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                    Spacer()
                    Text(model.devices.first { $0.id == a.deviceId }?.name ?? a.ip).font(.caption).foregroundStyle(.secondary)
                    Text(Fmt.date(a.at)).font(.caption2.monospacedDigit()).foregroundStyle(.tertiary)
                }
                .help(a.detail ?? "")
            }
        }
    }

    private func loadAudit() async {
        audit = (try? await model.engine.auditLog(limit: 200)) ?? []
    }

    private func formatCode(_ c: String) -> String {
        c.count == 8 ? "\(c.prefix(4))-\(c.suffix(4))" : c
    }

    private func scopeHelp(_ s: DeviceScope) -> String {
        switch s {
        case .read: return L10n.tr("See downloads and progress")
        case .add: return L10n.tr("Add new downloads")
        case .control: return L10n.tr("Pause, resume and remove downloads")
        case .admin: return L10n.tr("Change settings and manage devices")
        }
    }

    private func symbol(_ kind: String) -> String {
        switch kind.lowercased() {
        case let k where k.contains("phone") || k.contains("ios") || k.contains("android"): return "iphone"
        case let k where k.contains("tablet") || k.contains("ipad"): return "ipad"
        case let k where k.contains("browser") || k.contains("web"): return "safari"
        default: return "laptopcomputer"
        }
    }

    private func details(_ d: DeviceData) -> String {
        var parts = [d.scopes.map(\.label).joined(separator: ", ")]
        if let seen = d.lastSeenAt { parts.append("seen \(Fmt.relative(seen))") }
        if let ip = d.lastIp { parts.append(ip) }
        return parts.joined(separator: " · ")
    }
}

/// QR code rendered with Core Image's `CIQRCodeGenerator`, scaled without smoothing.
struct QRCodeView: View {
    let text: String
    var body: some View {
        if let image = Self.render(text) {
            Image(nsImage: image).interpolation(.none).resizable().aspectRatio(1, contentMode: .fit)
        } else {
            Image(systemName: "qrcode").resizable().aspectRatio(contentMode: .fit).foregroundStyle(.secondary)
        }
    }

    static func render(_ text: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 10, y: 10)) else { return nil }
        let rep = NSCIImageRep(ciImage: output)
        let image = NSImage(size: rep.size)
        image.addRepresentation(rep)
        return image
    }
}
