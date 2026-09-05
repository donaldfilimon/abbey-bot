// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "AbbeyAudioTap",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "abbey-audio-tap", targets: ["AbbeyAudioTap"])],
    targets: [
        .target(name: "AudioTapCore"),
        .target(name: "AudioTapRuntime", dependencies: ["AudioTapCore"]),
        .executableTarget(name: "AbbeyAudioTap", dependencies: ["AudioTapRuntime"]),
        .testTarget(name: "AudioTapCoreTests", dependencies: ["AudioTapCore"]),
        .testTarget(name: "AudioTapRuntimeTests", dependencies: ["AudioTapRuntime"]),
    ]
)
