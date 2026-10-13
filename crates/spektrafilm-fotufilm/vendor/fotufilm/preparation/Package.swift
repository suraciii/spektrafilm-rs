// swift-tools-version: 5.9
import PackageDescription
let package = Package(name: "FotufilmPreparation", products: [.executable(name: "fotufilm-web-profile", targets: ["ProfileClient"])], targets: [
.target(name: "FotufilmHalide", path: "FotufilmHalide", publicHeadersPath: "include", cxxSettings: [.unsafeFlags(["-std=c++17"])]),
.target(name: "NativeMath", path: "NativeMath"),
.target(name: "FotufilmCore", dependencies: ["FotufilmHalide"], path: "FotufilmCore", resources: [.process("Resources"), .process("CameraProfiles"), .copy("Stocks")]),
.target(name: "FotufilmEditModel", dependencies: ["FotufilmCore"], path: "FotufilmEditModel"),
.executableTarget(name: "ProfileClient", dependencies: ["FotufilmCore", "FotufilmEditModel", "NativeMath"], path: "ProfileClient", linkerSettings: [.unsafeFlags(["-Xlinker", "--wrap=expf", "-Xlinker", "--wrap=exp2f", "-Xlinker", "--wrap=logf", "-Xlinker", "--wrap=log1pf", "-Xlinker", "--wrap=log2f", "-Xlinker", "--wrap=log10f", "-Xlinker", "--wrap=tanf", "-Xlinker", "--wrap=powf", "-Xlinker", "--wrap=fotufilm_film_tile_build"])])])
