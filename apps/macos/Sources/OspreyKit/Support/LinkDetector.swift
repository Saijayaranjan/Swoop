import Foundation

/// Recognises downloadable links in pasted or dropped text.
public enum LinkDetector {
    public static let schemes = ["http://", "https://", "ftp://", "ftps://", "magnet:?"]

    public static func looksLikeLink(_ s: String) -> Bool {
        let t = s.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return schemes.contains { t.hasPrefix($0) }
    }

    /// Unique links in order of appearance.
    public static func links(in text: String) -> [String] {
        var out: [String] = []
        for raw in text.components(separatedBy: .whitespacesAndNewlines) {
            let t = raw.trimmingCharacters(in: CharacterSet(charactersIn: "<>\"'()[]"))
            if looksLikeLink(t), !out.contains(t) { out.append(t) }
        }
        return out
    }
}
