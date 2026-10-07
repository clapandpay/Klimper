# The Klimper sound

Klimper is what coins do. The app is the one place that jingle is heard (clapandpay
**decision-009**, **TASK-111**): when coins arrive, and when the phone is shaken, with a
vibration on the shake, heavier the more the coins are worth. The website stays silent
(`green-envelope/docs/clapandpay-brand.md`).

It is a small touch, and it is built to cost nothing when it is not in use: a feature flag,
a switch in Settings, everything in one folder, and a native side that is idle until asked.

## Flag, switch, folder

| What                          | Where                                                                                                                                                       |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Build flag                    | `VITE_ENABLE_KLIMPER_SOUND=true` (`.env`, `.env.production`)                                                                                                |
| User switch                   | Settings → Klimper sound; `localStorage` key `klimper.sound`, on by default                                                                                 |
| Everything in the webview     | `src/klimper/sound/` (loaded lazily, only with the flag)                                                                                                    |
| Native side (iOS)             | `tauri-plugin-klimper-jingle/` (Rust + Swift)                                                                                                               |
| The files and their generator | `src/klimper/sound/klimper-*.wav`, `scripts/klimper-sound.py`                                                                                               |
| Hook points in Sage's code    | `KlimperRoot.tsx` (one lazy element), `pages/Settings.tsx` (one element), `src-tauri/src/lib.rs` (one plugin), `capabilities/klimper.json` (one permission) |

Flag off: the chunk is never loaded, Settings has no card, and the plugin sits unused. Switch
off: the store subscription is dropped and the accelerometer is stopped.

## Tiers

Four weights, by the US-dollar value of the coins involved: what arrived, or the whole
wallet on a shake. Coarse on purpose: the sound is for the holder of the phone, not a
readout. The table is `src/klimper/sound/tiers.ts` (tests at the bounds).

| Tier      | Value                 | Sound                                                                 | Haptics (shake)                       |
| --------- | --------------------- | --------------------------------------------------------------------- | ------------------------------------- |
| `coin`    | under $1, or no price | one bright coin, two touches, 0.8 s                                   | one light tap                         |
| `few`     | $1 to $20             | six touches scattered over a third of a second                        | three taps, light to medium           |
| `handful` | $20 to $200           | thirteen touches, lower, settling over two thirds of a second         | five impacts, medium, closing in      |
| `pouch`   | $200 and above        | a thud as the mass lands, then two dozen coins rolling to rest, 1.5 s | seven impacts, heavy to light, 650 ms |

No price (testnet, offline, not fetched yet) means `coin`. The XCH price comes from Sage's
`get_xch_usd_price`, asked for at most once in ten minutes and only when something is about to
play.

## The sound itself

A struck coin rings with a handful of partials at non-integer ratios (a circular plate: 1,
1.59, 2.14, 2.30, 2.65, 2.92), the high ones dying first, behind one or two milliseconds of
noise where the metal meets. Two coins touching are two such strikes a few milliseconds apart
at different pitches. A jingle is many strikes whose density thins as the coins come to rest.

The signature is the tail: every tier ends on the same three strikes, low, high, middle
("klim-per"), a touch louder and cleaner than the rest, so the four files are one sound at four
weights. The pouch adds a thud (a pitch drop from 95 to 48 Hz with a little low noise) before
the coins.

`scripts/klimper-sound.py` renders all four from fixed seeds: pure Python, no dependencies,
44.1 kHz 16-bit mono, peak at -1 dBFS, tails cut at -48 dBFS. Running it twice gives
byte-identical files, so a change to the sound is a change to the script, reviewed like code.
Sizes: 68 to 131 kB.

Listening notes (2026-10-07, first version): the coin is dry and short; the few and handful
have the signature as their loudest moment, which is the brand showing; the pouch's thud is
felt more than heard on a phone speaker, which is what the haptics are for. To be adjusted
after listening on a device.

## How it plays

**Receive.** `KlimperSound` subscribes to Sage's wallet store (`useWalletState`, which Sage
refreshes on every `coin_state` event) and feeds each state to a pure detector
(`receive.ts`, tested as a table):

- the first real sample is the baseline and makes no sound: coins that came while the app
  was closed are not announced;
- Sage's default state (`receive_address === 'Unknown'`: logged out, switching) drops the
  baseline;
- while the wallet is catching up (`synced_coins < total_coins`) the baseline follows the
  balance silently;
- a decrease is a spend;
- an increase of a synced wallet is a receive. Increases within 1.5 s are one jingle of
  their sum. Before it plays, the wallet's fingerprint is checked against the one recorded at
  the baseline, so a switch to a richer wallet is never a jingle.

A receive is heard, not felt: haptics are for the shake, where the hand is already on the
phone.

**Shake.** On the phone the plugin watches `CMMotionManager` user acceleration at 30 Hz
(gravity removed, no permission needed) while the app is active. Three jolts above 1.6 g
within 0.7 s are a shake; the next counts after 1.5 s. Swift reports it over a channel, Rust
emits `klimper-jingle://shake`, and the webview plays the tier of the whole balance with the
tier's haptic pattern. The stream stops when the app resigns active and when the switch is
off.

**Playback.** iOS: `AVAudioPlayer` under the `.ambient` audio session with `mixWithOthers`,
so the silent switch mutes it and music keeps playing; the WAV bytes are baked into the Rust
library (`include_bytes!`) and handed to Swift once per tier per run, which keeps Xcode
resource plumbing out of it. Haptics: `UIImpactFeedbackGenerator`, light under 0.4, medium
under 0.7, heavy above, with `intensity`. Nothing plays or vibrates unless the app is active.
Desktop: the web audio API, decoded once per tier (Vite resolves `./klimper-${tier}.wav`).
Android: the plugin has no Kotlin yet; the commands fail quietly and nothing plays.

Every failure is quiet: one `console.warn` per run at most, never a toast. A sound that did
not play is not an error the user needs.

## Checks

Unit: `tiers.test.ts` (bounds, mojos to USD, haptic weights ascend), `receive.test.ts`
(the detector's rules, the coalescer), `KlimperSound.test.tsx` (the running component with
the store, the price, the fingerprint, the shake event and the switch mocked). Plugin:
`cargo test -p tauri-plugin-klimper-jingle` (the WAVs are baked in and valid, tiers parse,
haptic patterns are bounded). Swift compiles in the TestFlight build; the sound on a device
is the owner's check.
