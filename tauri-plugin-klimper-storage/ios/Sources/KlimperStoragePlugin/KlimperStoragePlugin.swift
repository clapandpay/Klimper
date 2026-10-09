// SPDX-License-Identifier: AGPL-3.0-or-later
// Keeps a directory out of iCloud and device backups (clapandpay TASK-057, TASK-126).

import SwiftRs
import Tauri
import UIKit

struct BackupArgs: Decodable {
  let path: String
}

class KlimperStoragePlugin: Plugin {
  // Set on the directory, so it covers files Sage replaces or creates later.
  @objc public func excludeFromBackup(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(BackupArgs.self)
    var url = URL(fileURLWithPath: args.path, isDirectory: true)
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try url.setResourceValues(values)
    // Read back: a flag that did not stick must fail loudly, not look done.
    url.removeAllCachedResourceValues()
    let check = try url.resourceValues(forKeys: [.isExcludedFromBackupKey])
    guard check.isExcludedFromBackup == true else {
      invoke.reject("the backup exclusion did not stick")
      return
    }
    invoke.resolve()
  }
}

@_cdecl("init_plugin_klimper_storage")
func initPlugin() -> Plugin {
  return KlimperStoragePlugin()
}
