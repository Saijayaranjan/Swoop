import SwiftUI

/// Drop-in replacement for `@State`.
///
/// On the macOS 26+/27 SDK `@State` is implemented as a macro whose compiler plugin ships only with
/// Xcode, so it is unavailable to a Command Line Tools build. `@ViewState` wraps the underlying
/// `SwiftUICore.State` storage directly and behaves identically (view-owned, persisted across body
/// evaluations, `$binding` projection).
@propertyWrapper
public struct ViewState<Value>: DynamicProperty {
    private var storage: SwiftUICore.State<Value>

    public init(wrappedValue: Value) {
        storage = SwiftUICore.State(initialValue: wrappedValue)
    }

    public init(initialValue: Value) {
        storage = SwiftUICore.State(initialValue: initialValue)
    }

    public var wrappedValue: Value {
        get { storage.wrappedValue }
        nonmutating set { storage.wrappedValue = newValue }
    }

    public var projectedValue: Binding<Value> { storage.projectedValue }
}

public extension ViewState where Value: ExpressibleByNilLiteral {
    init() { self.init(wrappedValue: nil) }
}
