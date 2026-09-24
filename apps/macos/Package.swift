// swift-tools-version: 5.9
// Osprey for macOS — built with Command Line Tools only (no Xcode project).
// `scripts/build-macos.sh` produces the Rust static library (build/lib/libosprey_ffi.a), the UniFFI
// header/modulemap (Sources/OspreyFFI/include) and the generated Swift bindings
// (Sources/OspreyKit/Generated/OspreyFFI.swift) before invoking `swift build`.
import PackageDescription
import Foundation

let packageDir = Context.packageDirectory
let repoRoot = URL(fileURLWithPath: packageDir).deletingLastPathComponent().deletingLastPathComponent().path
let rustLibDir = "\(repoRoot)/build/lib"

// The FFI artefacts exist once the build script has run. Without them the package still compiles
// (EngineClient reports "engine unavailable") so UI work and unit tests never depend on Rust.
let hasFFI = FileManager.default.fileExists(atPath: "\(packageDir)/Sources/OspreyFFI/include/osprey_ffiFFI.h")
    && FileManager.default.fileExists(atPath: "\(packageDir)/Sources/OspreyKit/Generated/OspreyFFI.swift")
    && FileManager.default.fileExists(atPath: "\(rustLibDir)/libosprey_ffi.a")

let systemFrameworks: [LinkerSetting] = [
    .linkedFramework("Security"),
    .linkedFramework("SystemConfiguration"),
    .linkedFramework("CoreFoundation"),
    .linkedFramework("CoreServices"),
    .linkedFramework("IOKit"),
    .linkedLibrary("resolv"),
    .linkedLibrary("c++"),
]

// Command Line Tools ship the Swift Testing macro plugin outside the default search path.
let cltTestingPlugins = "/Library/Developer/CommandLineTools/usr/lib/swift/host/plugins/testing"
let testingPluginSettings: [SwiftSetting] = FileManager.default.fileExists(atPath: cltTestingPlugins)
    ? [.unsafeFlags(["-plugin-path", cltTestingPlugins])] : []

var kitDependencies: [Target.Dependency] = []
var kitSwiftSettings: [SwiftSetting] = []
var kitLinker: [LinkerSetting] = []
var kitExclude: [String] = []
if hasFFI {
    kitDependencies.append("OspreyFFI")
    kitSwiftSettings.append(.define("OSPREY_FFI"))
    kitLinker = [.unsafeFlags(["-L\(rustLibDir)"]), .linkedLibrary("osprey_ffi")] + systemFrameworks
} else {
    kitExclude.append("Generated")
}

var targets: [Target] = [
    .target(
        name: "OspreyKit",
        dependencies: kitDependencies,
        path: "Sources/OspreyKit",
        exclude: kitExclude,
        swiftSettings: kitSwiftSettings,
        linkerSettings: kitLinker + [
            .linkedFramework("AppKit"),
            .linkedFramework("UserNotifications"),
            .linkedFramework("ServiceManagement"),
            .linkedFramework("Network"),
            .linkedFramework("IOKit"),
        ]
    ),
    .executableTarget(
        name: "OspreyApp",
        dependencies: ["OspreyKit"],
        path: "Sources/OspreyApp",
        linkerSettings: [
            .linkedFramework("Quartz"),
            .linkedFramework("CoreImage"),
            .linkedFramework("Charts"),
        ]
    ),
    .testTarget(
        name: "OspreyKitTests",
        dependencies: ["OspreyKit"],
        path: "Tests/OspreyKitTests",
        swiftSettings: testingPluginSettings
    ),
]
if hasFFI {
    targets.append(.target(name: "OspreyFFI", path: "Sources/OspreyFFI"))
}

let package = Package(
    name: "Osprey",
    defaultLocalization: "en",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "OspreyApp", targets: ["OspreyApp"]),
        .library(name: "OspreyKit", targets: ["OspreyKit"]),
    ],
    targets: targets
)
