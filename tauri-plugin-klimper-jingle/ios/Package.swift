// swift-tools-version:5.9
// SPDX-License-Identifier: AGPL-3.0-or-later

import PackageDescription

let package = Package(
  name: "tauri-plugin-klimper-jingle",
  platforms: [
    .iOS(.v14)
  ],
  products: [
    .library(
      name: "tauri-plugin-klimper-jingle",
      type: .static,
      targets: ["tauri-plugin-klimper-jingle"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-klimper-jingle",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
