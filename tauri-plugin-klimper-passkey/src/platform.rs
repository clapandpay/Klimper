// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Swift side (`ASAuthorization` with the PRF extension, iOS 18, and the backup flag);
//! elsewhere unsupported.

use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{AssertRequest, AssertResponse, Error, RegisterRequest, RegisterResponse, Result};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_klimper_passkey);

pub struct Passkeys<R: Runtime>(Option<PluginHandle<R>>);

impl<R: Runtime> Clone for Passkeys<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[cfg_attr(not(target_os = "ios"), allow(clippy::unnecessary_wraps))]
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> tauri::Result<Passkeys<R>> {
    #[cfg(target_os = "ios")]
    return Ok(Passkeys(Some(
        _api.register_ios_plugin(init_plugin_klimper_passkey)?,
    )));
    #[cfg(not(target_os = "ios"))]
    Ok(Passkeys(None))
}

impl<R: Runtime> Passkeys<R> {
    pub fn supported(&self) -> bool {
        self.0.is_some()
    }

    pub fn register(&self, request: RegisterRequest) -> Result<RegisterResponse> {
        self.run("register", request)
    }

    pub fn assert(&self, request: AssertRequest) -> Result<AssertResponse> {
        self.run("assert", request)
    }

    /// Leaves the directory and everything in it out of iCloud and device backups.
    pub fn exclude_from_backup(&self, dir: &Path) -> Result<()> {
        #[derive(Serialize)]
        struct Request {
            path: String,
        }
        self.run::<_, serde::de::IgnoredAny>(
            "excludeFromBackup",
            Request {
                path: dir.to_string_lossy().into_owned(),
            },
        )?;
        Ok(())
    }

    #[cfg_attr(not(mobile), allow(unused_variables, clippy::unused_self))]
    fn run<Q: serde::Serialize, S: DeserializeOwned>(&self, method: &str, request: Q) -> Result<S> {
        #[cfg(mobile)]
        if let Some(handle) = &self.0 {
            return handle
                .run_mobile_plugin(method, request)
                .map_err(|error| Error::Platform(error.to_string()));
        }
        Err(Error::Unsupported)
    }
}
