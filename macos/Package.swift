// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "P1505Duplex",
    defaultLocalization: "zh-Hant",
    platforms: [.macOS("27.0")],
    targets: [
        // Models, API client, event stream and notification rules; no UI.
        .target(name: "DuplexKit"),
        // The menu bar app.
        .executableTarget(name: "P1505Duplex", dependencies: ["DuplexKit"]),
        .testTarget(name: "DuplexKitTests", dependencies: ["DuplexKit"]),
    ]
)
