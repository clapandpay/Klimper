// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Klimper sound (clapandpay TASK-111, decision-009): coins colliding when money arrives
//! or the phone is shaken, felt as well as heard, heavier the more the coins are worth.
//!
//! The four jingles are baked into this library (`include_bytes!` of the files that
//! `scripts/klimper-sound.py` writes) and handed to the Swift side once per tier per run, so
//! nothing is fetched and no Xcode resource plumbing is needed. On iOS, Swift plays them with
//! `AVAudioPlayer` under the ambient audio session (the silent switch mutes them, other audio
//! keeps playing), fires the haptic pattern the webview sent, and watches the accelerometer
//! for a shake. Everywhere else the commands do nothing and say so: the desktop plays the same
//! files through the web audio API, and a desk cannot be shaken.

use serde::{Deserialize, Serialize, Serializer};
use tauri::{
    AppHandle, Emitter, Manager, Runtime, State, command,
    plugin::{Builder, TauriPlugin},
};

mod platform;

use platform::Native;

/// Event to the webview: the phone was shaken. Payload: [`Shake`].
pub const EVENT_SHAKE: &str = "klimper-jingle://shake";

/// Most impacts one haptic pattern may have, and the latest one may land, in ms.
const MAX_IMPACTS: usize = 16;
const MAX_IMPACT_MS: u32 = 2_000;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "the haptic pattern is out of bounds (at most {MAX_IMPACTS} impacts within {MAX_IMPACT_MS} ms, strengths 0 to 1)"
    )]
    BadPattern,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Coin,
    Few,
    Handful,
    Pouch,
}

impl Tier {
    /// The jingle, as `scripts/klimper-sound.py` wrote it.
    pub fn wav(self) -> &'static [u8] {
        match self {
            Tier::Coin => include_bytes!("../../src/klimper/sound/klimper-coin.wav"),
            Tier::Few => include_bytes!("../../src/klimper/sound/klimper-few.wav"),
            Tier::Handful => include_bytes!("../../src/klimper/sound/klimper-handful.wav"),
            Tier::Pouch => include_bytes!("../../src/klimper/sound/klimper-pouch.wav"),
        }
    }
}

/// One haptic impact: when, and how hard (0 to 1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Impact {
    pub at_ms: u32,
    pub strength: f32,
}

/// A shake, as the webview receives it. `intensity` is 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shake {
    pub intensity: f32,
}

fn check_pattern(impacts: &[Impact]) -> Result<()> {
    let ok = impacts.len() <= MAX_IMPACTS
        && impacts
            .iter()
            .all(|i| i.at_ms <= MAX_IMPACT_MS && (0.0..=1.0).contains(&i.strength));
    if ok { Ok(()) } else { Err(Error::BadPattern) }
}

/// Play the jingle of a tier and, with `impacts`, the haptic pattern that goes with it.
/// Nothing plays while the app is not in the foreground; Swift checks that too.
#[command]
async fn play<R: Runtime>(
    _app: AppHandle<R>,
    native: State<'_, Native<R>>,
    tier: Tier,
    impacts: Vec<Impact>,
) -> Result<()> {
    check_pattern(&impacts)?;
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || native.play(tier, &impacts))
        .await
        .map_err(|error| Error::Platform(error.to_string()))?
}

/// Start or stop watching the accelerometer. While on, a shake raises [`EVENT_SHAKE`].
#[command]
async fn set_shake_enabled<R: Runtime>(
    app: AppHandle<R>,
    native: State<'_, Native<R>>,
    enabled: bool,
) -> Result<()> {
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        native.set_shake_enabled(enabled, move |shake| {
            if let Err(error) = app.emit(EVENT_SHAKE, shake) {
                tracing::warn!("jingle: shake event not delivered: {error}");
            }
        })
    })
    .await
    .map_err(|error| Error::Platform(error.to_string()))?
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-jingle")
        .invoke_handler(tauri::generate_handler![play, set_shake_enabled])
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
    fn the_four_jingles_are_baked_in_and_are_wav_files() {
        for tier in [Tier::Coin, Tier::Few, Tier::Handful, Tier::Pouch] {
            let wav = tier.wav();
            assert_eq!(&wav[..4], b"RIFF", "{tier:?}");
            assert_eq!(&wav[8..12], b"WAVE", "{tier:?}");
            assert!(wav.len() < 200 * 1024, "{tier:?} is {} bytes", wav.len());
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

    #[test]
    fn a_haptic_pattern_is_bounded() {
        let ok = vec![
            Impact {
                at_ms: 0,
                strength: 1.0,
            },
            Impact {
                at_ms: 650,
                strength: 0.3,
            },
        ];
        assert!(check_pattern(&ok).is_ok());
        assert!(check_pattern(&[]).is_ok());
        assert!(
            check_pattern(&[Impact {
                at_ms: 0,
                strength: 1.5
            }])
            .is_err()
        );
        assert!(
            check_pattern(&[Impact {
                at_ms: 0,
                strength: -0.1
            }])
            .is_err()
        );
        assert!(
            check_pattern(&[Impact {
                at_ms: 2_001,
                strength: 0.5
            }])
            .is_err()
        );
        let many = vec![
            Impact {
                at_ms: 0,
                strength: 0.5
            };
            MAX_IMPACTS + 1
        ];
        assert!(check_pattern(&many).is_err());
    }
}
