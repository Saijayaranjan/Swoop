import AppKit
import Quartz

/// Data source for the shared Quick Look panel (space bar on the downloads table).
@MainActor
final class QuickLookController: NSObject, QLPreviewPanelDataSource, QLPreviewPanelDelegate {
    private(set) var urls: [URL] = []

    /// Shows (or hides, if already visible) Quick Look for existing files among `paths`.
    func toggle(paths: [String]) {
        urls = paths.map { URL(fileURLWithPath: $0) }.filter { FileManager.default.fileExists(atPath: $0.path) }
        guard let panel = QLPreviewPanel.shared() else { return }
        if panel.isVisible {
            panel.orderOut(nil)
            return
        }
        guard !urls.isEmpty else {
            NSSound.beep()
            return
        }
        panel.makeKeyAndOrderFront(nil)
        // If no responder claimed control (e.g. focus in a text field), drive it directly.
        if panel.dataSource == nil {
            panel.dataSource = self
            panel.delegate = self
        }
        panel.reloadData()
    }

    nonisolated func numberOfPreviewItems(in panel: QLPreviewPanel!) -> Int {
        MainActor.assumeIsolated { urls.count }
    }

    nonisolated func previewPanel(_ panel: QLPreviewPanel!, previewItemAt index: Int) -> (any QLPreviewItem)! {
        MainActor.assumeIsolated { urls[index] as NSURL }
    }

    nonisolated func previewPanel(_ panel: QLPreviewPanel!, handle event: NSEvent!) -> Bool {
        // Space closes the panel again.
        if event.type == .keyDown, event.charactersIgnoringModifiers == " " {
            MainActor.assumeIsolated { panel.orderOut(nil) }
            return true
        }
        return false
    }
}
