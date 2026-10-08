# The Klimper sound

Klimper is what coins do. The app is the one place that jingle is heard (clapandpay
**decision-009**, **TASK-111**); the website stays silent
(`green-envelope/docs/clapandpay-brand.md`). Two moments:

- **Coins arrive**: one bright coin rings, the "money arrived" sound of the first mobile
  wallets, made ours. Richer the more arrived. Heard, not felt.
- **The phone is shaken**: every turn of the hand is a few coins landing, heard and felt, as heavy as the open page, for as long as the
  shaking lasts, with a vibration on every jolt, heavier the more they are worth. Stops when
  the hand does.

It is a small touch, built to cost nothing when not in use and to touch nothing that matters.

## Privacy and safety

- The only thing it reads is the balance Sage already shows on screen (the wallet store, and
  the dollar value a page already computed for display).
- It sends nothing anywhere: the XCH price is the one Sage's price display already fetched
  (`src/klimper/price.ts`: nodexch on mainnet, 0 on testnet).
- It touches no keys, no transactions, no database, nothing in the send or receive path, and
  cannot block or delay them: everything is asynchronous and fails quietly (one `console.warn`
  per run at most, never a toast).
- The tiers are coarse on purpose: a bystander hearing a shake learns at most "more than $200".
  Nothing about amounts is logged.
- The switch in Settings is the privacy setting: off means no sound and no haptics, and the
  accelerometer is stopped. The build flag removes the feature altogether.
- Nothing plays or vibrates while the app is in the background.

## Tilt, privacy mode, shake to undo

- **Tilt.** With the coins armed and the hand calm, tilting the phone by about 18° lets a few
  coins slide: one to three soft hits and gentle taps, quieter and slower than a shake. iOS
  reads the gravity vector of the same motion updates; Android a `TYPE_GRAVITY` sensor.
- **Privacy mode** (Settings, the Klimper card) keeps the coins quiet and still whatever the
  sound switch says, and hides amounts (`src/klimper/privacy.tsx`).
- **No "Undo Typing" on a shake.** iOS offers to undo typing when the phone is shaken; the
  plugin turns that off for Klimper (`applicationSupportsShakeToEdit = false`), so shaking is
  only ever the coins.

## Flag, switch, folder

| What                          | Where                                                                                                                                                                                                                                   |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Build flag                    | `VITE_ENABLE_KLIMPER_SOUND=true` (`.env`, `.env.production`)                                                                                                                                                                            |
| User switch                   | Settings → Klimper sound; `localStorage` key `klimper.sound`, on by default                                                                                                                                                             |
| Everything in the webview     | `src/klimper/sound/` (loaded lazily, only with the flag)                                                                                                                                                                                |
| What the open page is worth   | `src/klimper/pageValue.ts` (a few lines, outside the lazy chunk so pages can import it)                                                                                                                                                 |
| Native side (iOS, Android)    | `tauri-plugin-klimper-jingle/` (Rust + Swift + Kotlin)                                                                                                                                                                                  |
| The files and their generator | `src/klimper/sound/klimper-{ding-<tier>,grain-1..8}.wav`, `scripts/klimper-sound.py`                                                                                                                                                    |
| Hook points in Sage's code    | `KlimperRoot.tsx` (one lazy element), `pages/Settings.tsx` (one element), `pages/Token.tsx` and `pages/TokenList.tsx` (one `usePageValue` line each), `src-tauri/src/lib.rs` (one plugin), `capabilities/klimper.json` (one permission) |

Flag off: the chunk is never loaded, Settings has no card, the plugin sits unused, and
`usePageValue` publishes into a registry nobody reads. Switch off: the store subscriptions are
dropped and the native side is disarmed.

## Tiers

Four weights, by the US-dollar value of the coins involved. The table is
`src/klimper/sound/tiers.ts` (tests at the bounds).

| Tier      | Value          | Ding (coins arrive)                    | Each turn of a shaking hand           |
| --------- | -------------- | -------------------------------------- | ------------------------------------- |
| `coin`    | $0.01 to $1    | one tick                               | one coin (sometimes a faint second)   |
| `few`     | $1 to $20      | a clink and a tick                     | 2 to 4 coins                          |
| `handful` | $20 to $200    | four grains over a quarter second      | 3 to 6 coins, a little lower          |
| `pouch`   | $200 and above | a low body and eight grains over 0.4 s | 5 to 8 coins, lower, the heaviest tap |

What is weighed: for the ding, what arrived (several coins landing within 1.5 s are one ding,
for their sum). For the shake, **what the open page shows**: a token page rattles with that
token's value, the wallet page with the total, any other page with the whole wallet in XCH.
**No price (testnet, offline, not fetched yet), or less than a cent, means silence**: no ding,
no rattle, no taps. A wallet with nothing in it makes no sound (owner's rule, 2026-10-07).

**The shake is one rule**: every turn of the hand is a few coins landing in it, heard and felt
at the same instant, and built fresh every time. A turn is a jolt of 1.3 g after the hand has come
back below 0.6 g (once per reversal), at most every 0.3 s. Each coin is one of eight single coin
hits, drawn without repeats and never the same opener twice, spaced like coins settling (gaps of
8 to 60 ms, mostly short, wider the harder the hand), at its own pitch and loudness (the first
loudest, the rest 20 to 70 % of it and fading). The hand feels them as taps where they are at
least 45 ms apart, four at most: the first is the wallet's weight, dull for a pouch and brighter
for one coin, the rest sharp, lighter clicks of coin on coin; a quarter of the turns end with one
coin sliding off. A new turn cuts the last one's settle. On iOS one Core Haptics pattern carries
the sounds and the taps together, so they cannot drift apart; Android schedules them the same
way with `SoundPool` and one `VibrationEffect` waveform.

## The sounds

Real coins. The files are cut by `scripts/klimper-sound.py` (pure Python, no dependencies,
fixed seeds) from six **CC0** recordings by Kenney (kenney.nl: RPG Audio `handleCoins`,
`handleCoins2`; Impact Sounds `impactMetal_light_000/002`, `_medium_001`, `_heavy_000`), kept
in `src/klimper/sound/sources/` as mono 44.1 kHz WAV with their provenance in `SOURCES.md`.
CC0 is public domain: free to use, change and ship, no attribution required; the file is the
attribution anyway. Running the script twice gives byte-identical files, so a change to the
sound is a change to the script, reviewed like code. Sixteen files, 10 to 67 kB, 0.8 MB in all,
baked into the binary.

The recordings are single gestures (a clink of 0.1 to 0.25 s, then silence), so they are used
as **grains**: the clink of `handleCoins` (0.10 to 0.45 s) and the tick of `handleCoins2`
(0.15 to 0.34 s), copied many times at seeded offsets, speeds (0.82 to 1.2, lower is bigger
coins) and gains. A hand full of coins is that anyway.

- **Arrive** (`ding`): a small rattle dying away. One tick for a coin; a clink and a tick for a
  few; four grains over a quarter second for a handful; the pouch's low body plus eight grains
  over 0.4 s. 0.16 to 0.83 s.
- **Coin hits** (eight, shared by every tier): single contacts cut at their own onset from the
  two coin recordings (found by an onset detector, picked by hand), 0.05 to 0.14 s, raised-cosine
  edges, equal peak, no slowing. The phone sets their pitch and loudness. 72 kB in all.

The first two versions were synthesised (inharmonic disc partials); the owner heard the
second on a phone and asked for real coins. Listening notes for this one: to be judged on the
device.

## How it works

**Coins arrive** (`receive.ts`, pure and tested as a table). `KlimperSound` subscribes to
Sage's wallet store (`useWalletState`, refreshed on every `coin_state` event):

- the first synced sample is the baseline and makes no sound: coins that came while the app
  was closed are not announced;
- Sage's default state (`receive_address === 'Unknown'`: logged out, switching) drops the
  baseline;
- a sample of a wallet that is catching up (`synced_coins < total_coins`) is **ignored**: a
  new coin is unsynced until Sage has its puzzle, and the balance and the flag do not change
  in the same store update. So the baseline is only ever a synced sample and the delta is
  taken between synced samples (the first version followed the balance during the unsynced
  moment and never heard a receive; found on the device);
- a decrease is a spend;
- an increase of a synced wallet is a receive. Increases within 1.5 s are one ding of their
  sum. Before it rings, the wallet's fingerprint is checked against the one recorded at the
  baseline, so a switch to a richer wallet is never a ding.

**The phone is shaken.** The webview arms the native side with the tier of what the page shows
(`setShake(true, tier)`, whenever the tier changes); off disarms it. Everything else is native, in real time:

- iOS: `CMMotionManager` user acceleration at 40 Hz while armed and active. `CHHapticEngine` on
  the shared ambient session, the coin hits registered as its audio resources; the engine is
  restarted after a reset and stopped with the motion when the app resigns active or the switch
  goes off. The audio session category is set only once the feature is used.
- Android: `TYPE_LINEAR_ACCELERATION` at game rate while armed and resumed, the same rule;
  `SoundPool` plays a sound only once it has loaded, and is released with the activity.
  Written to the same shape as the Swift, **not compiled**: no Android SDK here.

**Playback.** The sixteen WAVs are baked into the Rust library (`include_bytes!`) and handed to
the native side base64 once per sound per run, which keeps Xcode and Gradle resource plumbing
out of it. iOS plays under the `.ambient` session with `mixWithOthers`: the silent switch mutes
it, music keeps playing. Desktop: the ding through the web audio API, decoded once per tier
(Vite resolves `./klimper-ding-${tier}.wav`); a desk cannot be shaken.

## Checks

Unit: `tiers.test.ts` (bounds, mojos to USD, file names), `receive.test.ts` (the detector's
rules including the unsynced moment, the coalescer), `KlimperSound.test.tsx` (the running
component with the store, the price, the fingerprint, the page value and the switch mocked:
the ding of a summed receive, quiet on spend/catch-up/switch/fingerprint change, a coin
without a price, the shake armed with the page's tier or the wallet's, nothing when off).
Plugin: `cargo test -p tauri-plugin-klimper-jingle` (the WAVs are baked in and valid, tiers
parse). Swift compiles in the TestFlight build; the ding, the coins and the haptics on a
device are the owner's check.
