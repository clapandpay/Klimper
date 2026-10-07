// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Swift side (`AVAudioPlayer`, `UIImpactFeedbackGenerator`, `CMMotionManager`) and the
//! Kotlin side (`SoundPool`, `Vibrator`, `SensorManager`); elsewhere unsupported. Sound bytes
//! cross the bridge base64-encoded, once per sound per run (the native side keeps the player).

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde::{Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{Error, Result, Tier};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_klimper_jingle);

/// The Kotlin plugin (`android/`): its package is the Gradle namespace.
#[cfg(target_os = "android")]
const ANDROID_PLUGIN: &str = "com.clapandpay.klimper.jingle";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Sound {
    Ding(Tier),
    Rattle(Tier),
}

impl Sound {
    fn encoded(self) -> Vec<String> {
        match self {
            Sound::Ding(tier) => vec![B64.encode(tier.ding())],
            Sound::Rattle(tier) => {
                let [a, b] = tier.rattles();
                vec![B64.encode(a), B64.encode(b), B64.encode(tier.knock())]
            }
        }
    }
}

pub struct Native<R: Runtime> {
    handle: Option<PluginHandle<R>>,
    /// Sounds the native side already holds, so each crosses the bridge once.
    sent: Arc<Mutex<HashSet<Sound>>>,
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
struct PlayArgs {
    tier: Tier,
    /// The WAV, base64; `None` once the native side has it.
    wav: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShakeArgs {
    enabled: bool,
    tier: Tier,
    /// The two rattle loops of `tier`, base64; `None` once the native side has them.
    loops: Option<Vec<String>>,
    /// The knock of `tier`, base64; `None` once the native side has it.
    knock: Option<String>,
}

#[cfg_attr(not(mobile), allow(clippy::unnecessary_wraps))]
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> std::result::Result<Native<R>, Box<dyn std::error::Error>> {
    #[cfg(target_os = "ios")]
    let handle = Some(_api.register_ios_plugin(init_plugin_klimper_jingle)?);
    #[cfg(target_os = "android")]
    let handle = Some(_api.register_android_plugin(ANDROID_PLUGIN, "KlimperJinglePlugin")?);
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let handle = None;
    Ok(Native {
        handle,
        sent: Arc::default(),
    })
}

impl<R: Runtime> Native<R> {
    pub fn play(&self, tier: Tier) -> Result<()> {
        let sound = Sound::Ding(tier);
        let wav = self.unless_sent(sound)?.map(|mut v| v.remove(0));
        let result = self.run("play", PlayArgs { tier, wav });
        self.forget_if_failed(sound, &result);
        result
    }

    pub fn set_shake(&self, enabled: bool, tier: Tier) -> Result<()> {
        let sound = Sound::Rattle(tier);
        let fresh = if enabled {
            self.unless_sent(sound)?
        } else {
            None
        };
        let (loops, knock) = match fresh {
            Some(mut v) => {
                let knock = v.pop();
                (Some(v), knock)
            }
            None => (None, None),
        };
        let result = self.run(
            "setShake",
            ShakeArgs {
                enabled,
                tier,
                loops,
                knock,
            },
        );
        self.forget_if_failed(sound, &result);
        result
    }

    fn unless_sent(&self, sound: Sound) -> Result<Option<Vec<String>>> {
        let mut sent = self
            .sent
            .lock()
            .map_err(|_| Error::Platform("lock".into()))?;
        Ok(sent.insert(sound).then(|| sound.encoded()))
    }

    /// The native side may not have kept the bytes: send them again next time.
    fn forget_if_failed(&self, sound: Sound, result: &Result<()>) {
        if result.is_err()
            && let Ok(mut sent) = self.sent.lock()
        {
            sent.remove(&sound);
        }
    }

    #[cfg_attr(not(mobile), allow(unused_variables, clippy::unused_self))]
    fn run<Q: Serialize>(&self, method: &str, args: Q) -> Result<()> {
        #[cfg(mobile)]
        if let Some(handle) = &self.handle {
            return handle
                .run_mobile_plugin::<serde::de::IgnoredAny>(method, args)
                .map(|_| ())
                .map_err(|error| Error::Platform(error.to_string()));
        }
        Err(Error::Unsupported)
    }
}
