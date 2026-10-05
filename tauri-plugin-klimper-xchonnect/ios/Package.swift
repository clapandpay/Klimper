// swift-tools-version:5.9
// SPDX-License-Identifier: AGPL-3.0-or-later

import PackageDescription

let package = Package(
  name: "tauri-plugin-klimper-xchonnect",
  platforms: [
    .iOS(.v14)
  ],
  products: [
    .library(
      name: "tauri-plugin-klimper-xchonnect",
      type: .static,
      targets: ["tauri-plugin-klimper-xchonnect"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-klimper-xchonnect",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
