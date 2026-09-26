import Foundation
import Testing
@testable import SwoopKit

@Suite("Formatting")
struct FormattingTests {
    @Test func bytes() {
        #expect(Fmt.bytes(nil) == "—")
        #expect(Fmt.bytes(UInt64(999)) == "999 B")
        #expect(Fmt.bytes(UInt64(1_536_000)) == "1.54 MB")
        #expect(Fmt.bytes(UInt64(98_700_000)) == "98.7 MB")
        #expect(Fmt.bytes(UInt64(987_000_000)) == "987 MB")
        #expect(Fmt.bytes(UInt64(4_700_000_000)) == "4.70 GB")
    }

    @Test func speedAndEta() {
        #expect(Fmt.speed(0) == "–")
        #expect(Fmt.speed(2_400_000) == "2.40 MB/s")
        #expect(Fmt.eta(nil) == "—")
        #expect(Fmt.eta(45) == "45s")
        #expect(Fmt.eta(185) == "3m 05s")
        #expect(Fmt.eta(7_620) == "2h 07m")
        #expect(Fmt.eta(3 * 86_400 + 4 * 3600) == "3d 4h")
    }

    @Test func percentAndParsing() {
        #expect(Fmt.percent(0) == "0%")
        #expect(Fmt.percent(0.054) == "5.4%")
        #expect(Fmt.percent(0.999) == "99%")
        #expect(Fmt.percent(1) == "100%")
        #expect(Fmt.parseBytes("2 MB") == 2_000_000)
        #expect(Fmt.parseBytes("512k") == 512_000)
        #expect(Fmt.parseBytes("1.5 G") == 1_500_000_000)
        #expect(Fmt.parseBytes("abc") == nil)
    }
}

@Suite("Task store merge")
@MainActor
struct TaskStoreTests {
    private func row(_ id: String, rev: UInt64, state: TaskState = .downloading, downloaded: UInt64 = 0) -> TaskRowData {
        var p = ProgressData()
        p.downloaded = downloaded
        p.total = 1000
        return TaskRowData(id: id, rev: rev, name: "\(id).zip", state: state, progress: p)
    }

    @Test func ignoresStaleRows() {
        let store = TaskStore()
        store.load([row("a", rev: 5)], rev: 1)
        store.upsert(row("a", rev: 4, state: .paused))
        #expect(store["a"]?.state == .downloading)
        store.upsert(row("a", rev: 6, state: .paused))
        #expect(store["a"]?.state == .paused)
        #expect(store.count == 1)
    }

    @Test func progressRespectsRevision() {
        let store = TaskStore()
        store.load([row("a", rev: 3)], rev: 1)
        var p = ProgressData()
        p.downloaded = 500
        p.total = 1000
        store.apply(progress: [ProgressUpdate(taskId: "a", rev: 2, progress: p)])
        #expect(store["a"]?.progress.downloaded == 0)
        store.apply(progress: [ProgressUpdate(taskId: "a", rev: 3, progress: p)])
        #expect(store["a"]?.progress.downloaded == 500)
        #expect(store["a"]?.fraction == 0.5)
    }

    @Test func removedTasksStayRemoved() {
        let store = TaskStore()
        store.load([row("a", rev: 1), row("b", rev: 1)], rev: 1)
        let itemB = store["b"]
        store.remove("a")
        store.upsert(row("a", rev: 2))
        #expect(store["a"] == nil)
        #expect(store.count == 1)
        // A snapshot reload keeps object identity for surviving rows.
        store.load([row("b", rev: 9), row("c", rev: 1)], rev: 2)
        #expect(store["b"] === itemB)
        #expect(store["b"]?.rev == 9)
        #expect(store.count == 2)
    }

    @Test func appModelAppliesEventBatches() {
        let model = AppModel(engine: UnavailableEngineClient(reason: "test"))
        model.apply([.taskAdded(row("x", rev: 1, state: .queued))])
        model.apply([.taskStateChanged(id: "x", from: .queued, to: .completed)])
        #expect(model.tasks["x"]?.state == .completed)
        #expect(model.recentCompletions.first?.id == "x")
        model.apply([.taskRemoved(id: "x")])
        #expect(model.tasks.count == 0)
        #expect(model.recentCompletions.isEmpty)
    }
}

@Suite("Documents")
struct DocumentTests {
    @Test func ruleRoundTripKeepsUnknownFields() throws {
        let json = #"{"id":"r1","name":"Zips","enabled":true,"priority":10,"match_mode":"any","conditions":[{"field":"extension","any_of":["zip"],"future":1}],"actions":[{"action":"date_folder"}],"created_at":1,"updated_at":2,"hit_count":3}"#
        let rule = try EngineJSON.decode(RuleDoc.self, from: json)
        #expect(rule.matchMode == .any)
        #expect(ActionSchemas.ruleCondition.variant(for: rule.conditions[0])?.tag == "extension")
        let back = try EngineJSON.encode(rule)
        #expect(back.contains(#""future":1"#))
        #expect(back.contains(#""match_mode":"any""#))
    }

    @Test func settingsPathEditing() {
        var doc = SettingsDoc(text: #"{"network":{"connections_per_task":8},"plugins":{"x":{"a":1}}}"#)
        doc["network.connections_per_task"] = .number(16)
        doc["storage.download_directory"] = .string("/tmp")
        #expect(doc.int("network.connections_per_task") == 16)
        #expect(doc.string("storage.download_directory") == "/tmp")
        #expect(doc["plugins.x.a"]?.int == 1)
    }
}

@Suite("Browser integration")
struct NativeMessagingTests {
    @Test func registersDetectedBrowsersOnly() throws {
        let fm = FileManager.default
        let root = fm.temporaryDirectory.appendingPathComponent("swoop-nm-\(UUID().uuidString)")
        defer { try? fm.removeItem(at: root) }
        // Chromium only has a bare hosts folder (as other tools leave behind): not detected.
        for dir in ["BraveSoftware/Brave-Browser", "Firefox", "Chromium/NativeMessagingHosts", "Swoop/native-host"] {
            try fm.createDirectory(at: root.appendingPathComponent(dir), withIntermediateDirectories: true)
        }
        try Data().write(to: root.appendingPathComponent("BraveSoftware/Brave-Browser/Local State"))
        try Data().write(to: root.appendingPathComponent("Firefox/profiles.ini"))
        let helper = root.appendingPathComponent("swoop-helper")
        try Data().write(to: helper)
        try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: helper.path)

        #expect(NativeMessagingInstaller.registerAll(appSupport: root, helper: helper).isEmpty)

        let brave = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent(
            "BraveSoftware/Brave-Browser/NativeMessagingHosts/app.swoop.bridge.json"))) as? [String: Any]
        #expect(brave?["path"] as? String == helper.path)
        #expect(brave?["allowed_origins"] as? [String] == ["chrome-extension://\(NativeMessagingInstaller.chromiumExtensionId)/"])
        let firefox = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent(
            "Mozilla/NativeMessagingHosts/app.swoop.bridge.json"))) as? [String: Any]
        #expect(firefox?["allowed_extensions"] as? [String] == ["swoop@swoop.app"])
        #expect(!fm.fileExists(atPath: root.appendingPathComponent("Google/Chrome").path))
        #expect(!fm.fileExists(atPath: root.appendingPathComponent("Chromium/NativeMessagingHosts/app.swoop.bridge.json").path))
        #expect(!fm.fileExists(atPath: root.appendingPathComponent("Swoop/native-host").path))

        let status = NativeMessagingInstaller.status(appSupport: root, helper: helper)
        #expect(status.map(\.browser) == [.brave, .firefox])
        let allConnected = status.allSatisfy { $0.connected }
        #expect(allConnected)
    }
}
