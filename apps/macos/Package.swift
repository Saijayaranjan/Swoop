// swift-tools-version: 5.9
// Swoop for macOS — built with Command Line Tools only (no Xcode project).
// `scripts/build-macos.sh` produces the Rust static library (build/lib/libswoop_ffi.a), the UniFFI
// header/modulemap (Sources/SwoopFFI/include) and the generated Swift bindings
// (Sources/SwoopKit/Generated/SwoopFFI.swift) before invoking `swift build`.
import PackageDescription
import Foundation

let packageDir = Context.packageDirectory
let repoRoot = URL(fileURLWithPath: packageDir).deletingLastPathComponent().deletingLastPathComponent().path
let rustLibDir = "\(repoRoot)/build/lib"

// The FFI artefacts exist once the build script has run. Without them the package still compiles
// (EngineClient reports "engine unavailable") so UI work and unit tests never depend on Rust.
let hasFFI = FileManager.default.fileExists(atPath: "\(packageDir)/Sources/SwoopFFI/include/swoop_ffiFFI.h")
    && FileManager.default.fileExists(atPath: "\(packageDir)/Sources/SwoopKit/Generated/SwoopFFI.swift")
    && FileManager.default.fileExists(atPath: "\(rustLibDir)/libswoop_ffi.a")

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
    kitDependencies.append("SwoopFFI")
    kitSwiftSettings.append(.define("SWOOP_FFI"))
    kitLinker = [.unsafeFlags(["-L\(rustLibDir)"]), .linkedLibrary("swoop_ffi")] + systemFrameworks
} else {
    kitExclude.append("Generated")
}

var targets: [Target] = [
    .target(
        name: "SwoopKit",
        dependencies: kitDependencies,
        path: "Sources/SwoopKit",
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
        name: "SwoopApp",
        dependencies: ["SwoopKit"],
        path: "Sources/SwoopApp",
        linkerSettings: [
            .linkedFramework("Quartz"),
            .linkedFramework("CoreImage"),
            .linkedFramework("Charts"),
        ]
    ),
    .testTarget(
        name: "SwoopKitTests",
        dependencies: ["SwoopKit"],
        path: "Tests/SwoopKitTests",
        swiftSettings: testingPluginSettings
    ),
]
if hasFFI {
    targets.append(.target(name: "SwoopFFI", path: "Sources/SwoopFFI"))
}

let package = Package(
    name: "Swoop",
    defaultLocalization: "en",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "SwoopApp", targets: ["SwoopApp"]),
        .library(name: "SwoopKit", targets: ["SwoopKit"]),
    ],
    targets: targets
)
