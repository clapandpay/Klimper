// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Swift side (`AVAudioPlayer`, `UIImpactFeedbackGenerator`, `CMMotionManager`); elsewhere
//! unsupported. The jingle bytes cross the bridge base64-encoded, once per tier per run (Swift
//! keeps the decoded player).

use std::sync::{Arc, Mutex};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde::{Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    ipc::{Channel, InvokeResponseBody},
    plugin::{PluginApi, PluginHandle},
};

use crate::{Error, Impact, Result, Shake, Tier};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_klimper_jingle);

pub struct Native<R: Runtime> {
    handle: Option<PluginHandle<R>>,
    /// Tiers whose bytes Swift already holds, so they cross the bridge once.
    sent: Arc<Mutex<Vec<Tier>>>,
}

impl<R: Runtime> Clone for Native<R> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            sent: self.sent.clone(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayArgs<'a> {
    tier: Tier,
    /// The WAV, base64; `None` once Swift has it.
    wav: Option<String>,
    impacts: &'a [Impact],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShakeArgs {
    enabled: bool,
    channel: Option<Channel<InvokeResponseBody>>,
}

#[cfg_attr(not(target_os = "ios"), allow(clippy::unnecessary_wraps))]
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> std::result::Result<Native<R>, Box<dyn std::error::Error>> {
    #[cfg(target_os = "ios")]
    let handle = Some(_api.register_ios_plugin(init_plugin_klimper_jingle)?);
    #[cfg(not(target_os = "ios"))]
    let handle = None;
    Ok(Native {
        handle,
        sent: Arc::default(),
    })
}

impl<R: Runtime> Native<R> {
    pub fn play(&self, tier: Tier, impacts: &[Impact]) -> Result<()> {
        let wav = {
            let mut sent = self
                .sent
                .lock()
                .map_err(|_| Error::Platform("lock".into()))?;
            if sent.contains(&tier) {
                None
            } else {
                sent.push(tier);
                Some(B64.encode(tier.wav()))
            }
        };
        let result = self.run::<_, serde::de::IgnoredAny>("play", PlayArgs { tier, wav, impacts });
        if result.is_err()
            && let Ok(mut sent) = self.sent.lock()
        {
            // Swift may not have kept it: send it again next time.
            sent.retain(|t| *t != tier);
        }
        result.map(|_| ())
    }

    pub fn set_shake_enabled(
        &self,
        enabled: bool,
        on_shake: impl Fn(Shake) + Send + Sync + 'static,
    ) -> Result<()> {
        let channel = enabled.then(|| {
            Channel::new(move |body| {
                if let InvokeResponseBody::Json(json) = body
                    && let Ok(shake) = serde_json::from_str::<Shake>(&json)
                {
                    on_shake(shake);
                }
                Ok(())
            })
        });
        self.run::<_, serde::de::IgnoredAny>("setShakeEnabled", ShakeArgs { enabled, channel })
            .map(|_| ())
    }

    #[cfg_attr(not(mobile), allow(unused_variables, clippy::unused_self))]
    fn run<Q: Serialize, S: DeserializeOwned>(&self, method: &str, args: Q) -> Result<S> {
        #[cfg(mobile)]
        if let Some(handle) = &self.handle {
            return handle
                .run_mobile_plugin(method, args)
                .map_err(|error| Error::Platform(error.to_string()));
        }
        Err(Error::Unsupported)
    }
}
