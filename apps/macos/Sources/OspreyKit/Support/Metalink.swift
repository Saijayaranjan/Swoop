import Foundation

/// Minimal Metalink (v3 and v4) reader for local `.metalink`/`.meta4` files. Remote Metalink URLs
/// are resolved by the engine; a local document is turned into one request per file with the
/// mirrors and checksum it declares.
public struct MetalinkFile: Equatable, Sendable {
    public var name: String
    public var size: UInt64?
    public var urls: [String]
    /// `algo:hex` using the engine's algorithm names.
    public var checksum: String?
}

public enum MetalinkParser {
    public static func parse(_ data: Data) -> [MetalinkFile] {
        let delegate = Delegate()
        let parser = XMLParser(data: data)
        parser.delegate = delegate
        parser.shouldProcessNamespaces = true
        parser.parse()
        return delegate.files.filter { !$0.urls.isEmpty }
    }

    /// Converts files into add requests (primary URL + mirrors).
    public static func requests(from files: [MetalinkFile]) -> [NewTaskRequestData] {
        files.map { f in
            var r = NewTaskRequestData()
            r.url = f.urls.first
            r.mirrors = Array(f.urls.dropFirst())
            r.name = (f.name as NSString).lastPathComponent
            r.options.checksum = f.checksum
            r.origin = "metalink"
            return r
        }
    }

    private final class Delegate: NSObject, XMLParserDelegate {
        var files: [MetalinkFile] = []
        private var current: MetalinkFile?
        private var text = ""
        private var hashType: String?
        private var urlPriority: [(Int, String)] = []
        private var urlPriorityCurrent = 999999

        func parser(_ parser: XMLParser, didStartElement name: String, namespaceURI: String?, qualifiedName: String?, attributes: [String: String] = [:]) {
            text = ""
            switch name {
            case "file":
                current = MetalinkFile(name: attributes["name"] ?? "download", size: nil, urls: [], checksum: nil)
                urlPriority = []
            case "hash":
                hashType = attributes["type"]
            case "url":
                // v4 uses `priority` (1 = best), v3 uses `preference` (100 = best).
                if let p = attributes["priority"].flatMap(Int.init) { urlPriorityCurrent = p }
                else if let p = attributes["preference"].flatMap(Int.init) { urlPriorityCurrent = 1000 - p }
                else { urlPriorityCurrent = 999999 }
            default: break
            }
        }

        func parser(_ parser: XMLParser, foundCharacters string: String) { text += string }

        func parser(_ parser: XMLParser, didEndElement name: String, namespaceURI: String?, qualifiedName: String?) {
            let value = text.trimmingCharacters(in: .whitespacesAndNewlines)
            switch name {
            case "size": current?.size = UInt64(value)
            case "hash":
                if let type = hashType, current?.checksum == nil || type.contains("256") {
                    let algo = type.lowercased().replacingOccurrences(of: "-", with: "")
                    if ["md5", "sha1", "sha256", "sha512"].contains(algo) { current?.checksum = "\(algo):\(value.lowercased())" }
                }
                hashType = nil
            case "url":
                let lower = value.lowercased()
                if lower.hasPrefix("http://") || lower.hasPrefix("https://") || lower.hasPrefix("ftp://") || lower.hasPrefix("ftps://") {
                    urlPriority.append((urlPriorityCurrent, value))
                }
            case "file":
                if var f = current {
                    f.urls = urlPriority.sorted { $0.0 < $1.0 }.map(\.1)
                    files.append(f)
                }
                current = nil
            default: break
            }
            text = ""
        }
    }
}
