// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Klimper sound (clapandpay TASK-111, decision-009): a coin "ding" when money arrives,
//! and coins rattling, felt as well as heard, for as long as the phone is shaken; heavier the
//! more the coins are worth.
//!
//! The sounds (a ding per tier, eight single coin hits) are baked into this library (`include_bytes!` of the files that
//! `scripts/klimper-sound.py` writes) and handed to the native side once per tier per run, so
//! nothing is fetched and no Xcode or Gradle resource plumbing is needed. On the phone the
//! native side plays them under an audio session that the silent switch mutes; while shaking is
//! on, every turn of the hand is a few coins, each heard and felt at the same moment, built at
//! random from the coin hits, so it stops when the hand does and no two turns are alike. Everywhere else the commands do nothing and say so: the desktop
//! plays the ding through the web audio API, and a desk cannot be shaken.

use serde::{Deserialize, Serialize, Serializer};
use tauri::{
    AppHandle, Manager, Runtime, State, command,
    ipc::Channel,
    webview::PageLoadEvent,
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

/// Watch for shaking, or stop. While on, every turn of the hand is a few coins, as many and as
/// heavy as `tier` says. Call again with a new tier when the wallet's value changes tier.
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

/// The system haptics a control asks for: a finger takes hold, passes a mark, finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Feel {
    Grab,
    Tick,
    Done,
}

/// One system haptic for a control (the slide to approve). The phone's own haptics setting
/// decides whether it is felt; the Klimper sound switch does not, since this is feedback for a
/// gesture rather than the coin sound.
#[command]
async fn feel<R: Runtime>(
    _app: AppHandle<R>,
    native: State<'_, Native<R>>,
    kind: Feel,
) -> Result<()> {
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || native.feel(kind))
        .await
        .map_err(|error| Error::Platform(error.to_string()))?
}

/// The coin pouch is open (TASK-124): the phone's tilt and the jolts of the hand go to
/// `channel` (`{ gx, gy }` in g, `{ jx, jy }` for a jolt), and the shake and tilt no longer sound
/// on their own: the pouch's coins knock and call [`clink`]. `open: false` closes it (the
/// channel is then ignored).
#[command]
async fn pouch<R: Runtime>(
    _app: AppHandle<R>,
    native: State<'_, Native<R>>,
    open: bool,
    channel: Channel<serde_json::Value>,
) -> Result<()> {
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || native.pouch(open.then_some(channel)))
        .await
        .map_err(|error| Error::Platform(error.to_string()))?
}

/// One knock between coins in the pouch: a coin hit and a tap, the same ones a shake is made
/// of. `size` and `force` are 0..1.
#[command]
async fn clink<R: Runtime>(
    _app: AppHandle<R>,
    native: State<'_, Native<R>>,
    size: f64,
    force: f64,
) -> Result<()> {
    let native = native.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        native.clink(size.clamp(0.0, 1.0), force.clamp(0.0, 1.0))
    })
    .await
    .map_err(|error| Error::Platform(error.to_string()))?
}

/// Single coin hits, one contact each, shared by every tier: the phone builds every turn of a
/// shaking hand out of these at random.
pub fn grains() -> [&'static [u8]; 8] {
    macro_rules! g {
        ($n:literal) => {
            include_bytes!(concat!(
                "../../src/klimper/sound/klimper-grain-",
                $n,
                ".wav"
            ))
        };
    }
    [
        g!("1"),
        g!("2"),
        g!("3"),
        g!("4"),
        g!("5"),
        g!("6"),
        g!("7"),
        g!("8"),
    ]
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-jingle")
        .invoke_handler(tauri::generate_handler![play, set_shake, feel, pouch, clink])
        .setup(|app, api| {
            let native = platform::init(app, api)?;
            app.manage(native);
            Ok(())
        })
        // A page that (re)loads never closed the pouch it had open: close it, or the motion
        // stream would run on for a channel nobody listens to, and the shake stay silent.
        .on_page_load(|webview, payload| {
            if payload.event() != PageLoadEvent::Started {
                return;
            }
            if let Some(native) = webview.try_state::<Native<R>>() {
                let native = native.inner().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _ = native.pouch(None);
                });
            }
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sounds_are_baked_in_and_are_wav_files() {
        let dings = Tier::ALL.map(Tier::ding);
        for wav in dings.iter().chain(grains().iter()) {
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..12], b"WAVE");
            assert!(wav.len() < 200 * 1024, "{} bytes", wav.len());
        }
    }

    #[test]
    fn feels_are_the_lowercase_names_the_webview_uses() {
        assert_eq!(
            serde_json::from_str::<Feel>("\"tick\"").unwrap(),
            Feel::Tick
        );
        assert_eq!(serde_json::to_string(&Feel::Done).unwrap(), "\"done\"");
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
