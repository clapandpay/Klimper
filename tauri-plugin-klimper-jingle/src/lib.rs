// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Klimper sound (clapandpay TASK-111, decision-009): a coin "ding" when money arrives,
//! and coins rattling, felt as well as heard, for as long as the phone is shaken; heavier the
//! more the coins are worth.
//!
//! The sixteen sounds are baked into this library (`include_bytes!` of the files that
//! `scripts/klimper-sound.py` writes) and handed to the native side once per tier per run, so
//! nothing is fetched and no Xcode or Gradle resource plumbing is needed. On the phone the
//! native side plays them under an audio session that the silent switch mutes, loops the
//! rattle while the accelerometer keeps jolting and taps the haptics with every jolt, and
//! stops when the hand does. Everywhere else the commands do nothing and say so: the desktop
//! plays the ding through the web audio API, and a desk cannot be shaken.

use serde::{Deserialize, Serialize, Serializer};
use tauri::{
    AppHandle, Manager, Runtime, State, command,
    plugin::{Builder, TauriPlugin},
};

mod platform;

use platform::Native;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the Klimper sound plays on the phone only")]
    Unsupported,
    #[error("jingle: {0}")]
    Platform(String),
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

/// The weight of the coins involved, by their value (`src/klimper/sound/tiers.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Coin,
    Few,
    Handful,
    Pouch,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Tier::Coin, Tier::Few, Tier::Handful, Tier::Pouch];

    /// Coins arrive: one shot.
    pub fn ding(self) -> &'static [u8] {
        match self {
            Tier::Coin => include_bytes!("../../src/klimper/sound/klimper-ding-coin.wav"),
            Tier::Few => include_bytes!("../../src/klimper/sound/klimper-ding-few.wav"),
            Tier::Handful => include_bytes!("../../src/klimper/sound/klimper-ding-handful.wav"),
            Tier::Pouch => include_bytes!("../../src/klimper/sound/klimper-ding-pouch.wav"),
        }
    }

    /// The phone is shaken: two seamless loops; the phone picks one at random each shake.
    pub fn rattles(self) -> [&'static [u8]; 2] {
        match self {
            Tier::Coin => [
                include_bytes!("../../src/klimper/sound/klimper-rattle-coin-a.wav"),
                include_bytes!("../../src/klimper/sound/klimper-rattle-coin-b.wav"),
            ],
            Tier::Few => [
                include_bytes!("../../src/klimper/sound/klimper-rattle-few-a.wav"),
                include_bytes!("../../src/klimper/sound/klimper-rattle-few-b.wav"),
            ],
            Tier::Handful => [
                include_bytes!("../../src/klimper/sound/klimper-rattle-handful-a.wav"),
                include_bytes!("../../src/klimper/sound/klimper-rattle-handful-b.wav"),
            ],
            Tier::Pouch => [
                include_bytes!("../../src/klimper/sound/klimper-rattle-pouch-a.wav"),
                include_bytes!("../../src/klimper/sound/klimper-rattle-pouch-b.wav"),
            ],
        }
    }

    /// One jolt: a couple of coins knocking, played over the loop, louder the harder the jolt.
    pub fn knock(self) -> &'static [u8] {
        match self {
            Tier::Coin => include_bytes!("../../src/klimper/sound/klimper-knock-coin.wav"),
            Tier::Few => include_bytes!("../../src/klimper/sound/klimper-knock-few.wav"),
            Tier::Handful => include_bytes!("../../src/klimper/sound/klimper-knock-handful.wav"),
            Tier::Pouch => include_bytes!("../../src/klimper/sound/klimper-knock-pouch.wav"),
        }
    }
}

/// Play the ding of a tier once. Nothing plays while the app is not in the foreground.
#[command]
async fn play<R: Runtime>(
    _app: AppHandle<R>,
    native: State<'_, Native<R>>,
    tier: Tier,
) -> Result<()> {
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || native.play(tier))
        .await
        .map_err(|error| Error::Platform(error.to_string()))?
}

/// Watch for shaking, or stop. While on, shaking plays the rattle of `tier` with haptics for
/// as long as it lasts. Call again with a new tier when the wallet's value changes tier.
#[command]
async fn set_shake<R: Runtime>(
    _app: AppHandle<R>,
    native: State<'_, Native<R>>,
    enabled: bool,
    tier: Tier,
) -> Result<()> {
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || native.set_shake(enabled, tier))
        .await
        .map_err(|error| Error::Platform(error.to_string()))?
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-jingle")
        .invoke_handler(tauri::generate_handler![play, set_shake])
        .setup(|app, api| {
            let native = platform::init(app, api)?;
            app.manage(native);
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sixteen_sounds_are_baked_in_and_are_wav_files() {
        for tier in Tier::ALL {
            let [a, b] = tier.rattles();
            for wav in [tier.ding(), a, b, tier.knock()] {
                assert_eq!(&wav[..4], b"RIFF", "{tier:?}");
                assert_eq!(&wav[8..12], b"WAVE", "{tier:?}");
                assert!(wav.len() < 200 * 1024, "{tier:?} is {} bytes", wav.len());
            }
        }
    }

    #[test]
    fn tiers_are_the_lowercase_names_the_webview_uses() {
        assert_eq!(
            serde_json::from_str::<Tier>("\"pouch\"").unwrap(),
            Tier::Pouch
        );
        assert_eq!(
            serde_json::to_string(&Tier::Handful).unwrap(),
            "\"handful\""
        );
        assert!(serde_json::from_str::<Tier>("\"bag\"").is_err());
    }
}
