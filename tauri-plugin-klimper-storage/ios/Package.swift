// swift-tools-version:5.9
// SPDX-License-Identifier: AGPL-3.0-or-later

import PackageDescription

let package = Package(
  name: "tauri-plugin-klimper-storage",
  platforms: [
    .iOS(.v14)
  ],
  products: [
    .library(
      name: "tauri-plugin-klimper-storage",
      type: .static,
      targets: ["tauri-plugin-klimper-storage"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-klimper-storage",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
