# The Klimper sound

Klimper is what coins do. The app is the one place that jingle is heard (clapandpay
**decision-009**, **TASK-111**); the website stays silent
(`green-envelope/docs/clapandpay-brand.md`). Two moments:

- **Coins arrive**: one bright coin rings, the "money arrived" sound of the first mobile
  wallets, made ours. Richer the more arrived. Heard, not felt.
- **The phone is shaken**: the coins on the open page rattle in the hand for as long as the
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

## Flag, switch, folder

| What                          | Where                                                                                                                                                                                                                                   |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Build flag                    | `VITE_ENABLE_KLIMPER_SOUND=true` (`.env`, `.env.production`)                                                                                                                                                                            |
| User switch                   | Settings → Klimper sound; `localStorage` key `klimper.sound`, on by default                                                                                                                                                             |
| Everything in the webview     | `src/klimper/sound/` (loaded lazily, only with the flag)                                                                                                                                                                                |
| What the open page is worth   | `src/klimper/pageValue.ts` (a few lines, outside the lazy chunk so pages can import it)                                                                                                                                                 |
| Native side (iOS, Android)    | `tauri-plugin-klimper-jingle/` (Rust + Swift + Kotlin)                                                                                                                                                                                  |
| The files and their generator | `src/klimper/sound/klimper-{ding,rattle-a,rattle-b,knock}-<tier>.wav`, `scripts/klimper-sound.py`                                                                                                                                       |
| Hook points in Sage's code    | `KlimperRoot.tsx` (one lazy element), `pages/Settings.tsx` (one element), `pages/Token.tsx` and `pages/TokenList.tsx` (one `usePageValue` line each), `src-tauri/src/lib.rs` (one plugin), `capabilities/klimper.json` (one permission) |

Flag off: the chunk is never loaded, Settings has no card, the plugin sits unused, and
`usePageValue` publishes into a registry nobody reads. Switch off: the store subscriptions are
dropped and the native side is disarmed.

## Tiers

Four weights, by the US-dollar value of the coins involved. The table is
`src/klimper/sound/tiers.ts` (tests at the bounds).

| Tier      | Value          | Ding (coins arrive)                    | Rattle (shake)                            |
| --------- | -------------- | -------------------------------------- | ----------------------------------------- |
| `coin`    | $0.01 to $1    | one tick                               | one coin knocking about (3 grains a loop) |
| `few`     | $1 to $20      | a clink and a tick                     | 8 grains a loop                           |
| `handful` | $20 to $200    | four grains over a quarter second      | 16 grains a loop                          |
| `pouch`   | $200 and above | a low body and eight grains over 0.4 s | 28 grains and five low thuds a loop       |

What is weighed: for the ding, what arrived (several coins landing within 1.5 s are one ding,
for their sum). For the shake, **what the open page shows**: a token page rattles with that
token's value, the wallet page with the total, any other page with the whole wallet in XCH.
**No price (testnet, offline, not fetched yet), or less than a cent, means silence**: no ding,
no rattle, no taps. A wallet with nothing in it makes no sound (owner's rule, 2026-10-07).

**Harder shaking sounds harder, and no two shakes are alike.** Each rattle starts from one of
the tier's two loop variants at a slightly random rate (0.96 to 1.04); each jolt adds a knock
(a couple of coins, 0.14 s) on top at a random pitch (rate 0.85 to 1.25), louder the harder the
jolt; and the loop's volume (0.55 to 1) and rate (0.92 to 1.25) follow a smoothed measure of
how hard the hand is shaking, so the coins swell and settle with the hand.

Haptics on a shake: one tap per turn of the hand (no closer than 160 ms; faster is a buzz, not
coins), with the knock sound riding on the tap, `rigid` for the light tiers,
`medium` for a handful, `heavy` for a pouch, intensity from the tier's weight and the jolt's
force (never below what a hand notices). Android: one `VibrationEffect` pulse per jolt, 18 to
48 ms, amplitude from the same strength.

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
- **Rattle** (two loops per tier, 0.78 s, seamless): the grains fall in two bursts a loop with
  a pause after each (a hand turns about 2.4 times a second, and the coins knock when it turns):
  2 grains a loop for a coin (one coin knocking about), 5 for a few, 9 for a handful, 16 plus
  two low-passed thuds for the pouch.
  The loop's last 60 ms are overlapped onto its first and dropped, so it runs into itself.
- **Knock** (one per jolt): a dull light impact for a coin, two ticks for a few, a light impact
  with a clink for a handful, the medium impact with the body for a pouch. 0.12 to 0.21 s.

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
(`setShake(true, tier)`, at once with `coin` so the first shake answers before any price, then
with the real tier, and again whenever it changes); off disarms it. Everything else is native,
in real time:

- iOS: `CMMotionManager` user acceleration at 40 Hz (gravity removed, no permission) while
  the app is active. A jolt is 1.3 g or more; two jolts within 0.45 s start one of the tier's
  rattle loops (`AVAudioPlayer`, `numberOfLoops = -1`, `enableRate`); every jolt while it runs
  is a haptic tap, a knock (three players per tier so knocks overlap), a nudge to the loop's
  volume and rate, and pushes the stop back; 0.35 s without a jolt fades the loop out in 120 ms. Resigning active
  stops both. Motion state lives on the motion queue, players and haptics on main.
- Android: `TYPE_LINEAR_ACCELERATION` (or the raw accelerometer with 1 g removed) at game
  rate while resumed, the same thresholds; `SoundPool` loop (`-1`), `stop` on stillness; a
  `VibrationEffect` pulse per jolt. Skipped when the ringer is silent or vibrate (the hand still
  feels it). Written to the same shape as the Swift, **not compiled**: no Android SDK here.

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
parse). Swift compiles in the TestFlight build; the ding, the rattle and the haptics on a
device are the owner's check.
