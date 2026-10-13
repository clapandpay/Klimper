// SPDX-License-Identifier: AGPL-3.0-or-later
//! Klimper keeps the wallet's keys exactly as upstream Sage does (`keys.bin` in the app data
//! directory, Sage's own password handling, Require biometrics in Settings). The one deliberate
//! difference (clapandpay TASK-057, TASK-126): that directory never goes into a phone backup,
//! because `keys.bin` under Sage's password protects nothing once it leaves the phone, and an
//! iCloud Backup without Advanced Data Protection is readable by Apple.
//!
//! The flag sits on the directory, so it also covers what Sage creates there later (each
//! wallet's database, a `keys.bin` written for the first time). A new phone restores from the
//! recovery words, or imports the Clap and Pay money again. Android needs nothing here: the
//! manifest turns backup and device transfer off (`gen/android`, `data_extraction_rules.xml`).

use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_klimper_storage);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-storage")
        .setup(|app, api| {
            let dir = app.path().app_data_dir()?;
            #[cfg(target_os = "ios")]
            {
                let handle = api.register_ios_plugin(init_plugin_klimper_storage)?;
                keep_out_of_backups(handle, dir);
            }
            #[cfg(not(target_os = "ios"))]
            let _ = (api, dir);
            Ok(())
        })
        .build()
}

/// Long enough for Sage's logging to be up (`Sage::initialize`, called by the webview at start).
#[cfg(target_os = "ios")]
const LOG_AFTER_SECS: u64 = 15;

#[cfg(target_os = "ios")]
fn keep_out_of_backups<R: Runtime>(
    handle: tauri::plugin::PluginHandle<R>,
    dir: std::path::PathBuf,
) {
    #[derive(serde::Serialize)]
    struct Request {
        path: String,
    }
    // Off the main thread, where setup runs and the Swift side answers. The result is logged a
    // little later: Sage installs its log subscriber when the webview initializes it, after
    // plugin setup, and an event sent before that goes nowhere. It is retried every launch.
    tauri::async_runtime::spawn_blocking(move || {
        let result = std::fs::create_dir_all(&dir)
            .map_err(|error| error.to_string())
            .and_then(|()| {
                handle
                    .run_mobile_plugin::<serde::de::IgnoredAny>(
                        "excludeFromBackup",
                        Request {
                            path: dir.to_string_lossy().into_owned(),
                        },
                    )
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            });
        std::thread::sleep(std::time::Duration::from_secs(LOG_AFTER_SECS));
        match result {
            Ok(()) => tracing::info!("{} is kept out of backups", dir.display()),
            Err(error) => {
                tracing::error!("could not keep {} out of backups: {error}", dir.display());
            }
        }
    });
}
