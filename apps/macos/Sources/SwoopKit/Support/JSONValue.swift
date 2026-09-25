import Foundation

/// A lossless JSON document used for the engine's opaque payloads (settings, platform-action
/// context). Editing through `JSONValue` keeps fields the app does not know about intact, so a
/// newer engine never loses data when an older UI saves settings.
public enum JSONValue: Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(parsing text: String) throws {
        let data = Data(text.utf8)
        let any = try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
        self = JSONValue(any: any)
    }

    public init(any: Any?) {
        switch any {
        case nil, is NSNull: self = .null
        case let n as NSNumber:
            // NSNumber bridges Bool; distinguish by objCType.
            if CFGetTypeID(n) == CFBooleanGetTypeID() { self = .bool(n.boolValue) } else { self = .number(n.doubleValue) }
        case let s as String: self = .string(s)
        case let a as [Any]: self = .array(a.map { JSONValue(any: $0) })
        case let d as [String: Any]: self = .object(d.mapValues { JSONValue(any: $0) })
        default: self = .null
        }
    }

    public var anyValue: Any {
        switch self {
        case .null: return NSNull()
        case .bool(let b): return b
        case .number(let n):
            if n.rounded() == n, abs(n) < 9.0e15 { return Int64(n) }
            return n
        case .string(let s): return s
        case .array(let a): return a.map(\.anyValue)
        case .object(let o): return o.mapValues(\.anyValue)
        }
    }

    public func serialized(pretty: Bool = false) -> String {
        var opts: JSONSerialization.WritingOptions = [.fragmentsAllowed, .sortedKeys, .withoutEscapingSlashes]
        if pretty { opts.insert(.prettyPrinted) }
        guard let data = try? JSONSerialization.data(withJSONObject: anyValue, options: opts) else { return "null" }
        return String(decoding: data, as: UTF8.self)
    }

    // MARK: typed accessors

    public var bool: Bool? { if case .bool(let b) = self { return b }; return nil }
    public var double: Double? { if case .number(let n) = self { return n }; return nil }
    public var int: Int? { double.map { Int($0) } }
    public var string: String? { if case .string(let s) = self { return s }; return nil }
    public var array: [JSONValue]? { if case .array(let a) = self { return a }; return nil }
    public var object: [String: JSONValue]? { if case .object(let o) = self { return o }; return nil }

    public subscript(key: String) -> JSONValue? {
        get { object?[key] }
        set {
            guard case .object(var o) = self else { return }
            o[key] = newValue
            self = .object(o)
        }
    }

    /// Dotted-path access: `settings[path: "network.connections_per_task"]`.
    public subscript(path path: String) -> JSONValue? {
        get {
            var cur: JSONValue? = self
            for part in path.split(separator: ".") { cur = cur?[String(part)] }
            return cur
        }
        set {
            let parts = path.split(separator: ".").map(String.init)
            self = JSONValue.setting(self, parts[...], newValue ?? .null)
        }
    }

    private static func setting(_ node: JSONValue, _ parts: ArraySlice<String>, _ value: JSONValue) -> JSONValue {
        guard let head = parts.first else { return value }
        var obj = node.object ?? [:]
        obj[head] = setting(obj[head] ?? .object([:]), parts.dropFirst(), value)
        return .object(obj)
    }
}

extension JSONValue: Codable {
    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([JSONValue].self) { self = .array(a) }
        else { self = .object(try c.decode([String: JSONValue].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n):
            if n.rounded() == n, abs(n) < 9.0e15 { try c.encode(Int64(n)) } else { try c.encode(n) }
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }
}

/// Shared JSON coders matching the engine's serde conventions (snake_case keys are spelled out
/// explicitly in `CodingKeys`, so no key strategy is applied).
public enum EngineJSON {
    public static func decode<T: Decodable>(_ type: T.Type, from text: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(text.utf8))
    }

    public static func encode<T: Encodable>(_ value: T) throws -> String {
        let enc = JSONEncoder()
        enc.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return String(decoding: try enc.encode(value), as: UTF8.self)
    }
}
