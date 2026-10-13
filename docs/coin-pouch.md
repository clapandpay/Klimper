# The coin pouch (TASK-124)

A token page shows the asset's coins as discs in a pouch, above Sage's coin list. They lie,
roll and knock as the phone tilts; tapping a coin selects it, and the selection is the list's
own, so **Split** and **Combine** below work on it unchanged. It shows Chia's coin set model as
it is: a balance is a handful of coins, a payment spends whole coins and gives change, dust is
many tiny ones (beyond 120 coins the smallest are counted, not drawn, with a "Select N tiny
coins" button: the nudge to combine them).

## The coins

Each disc is a coin: a milled rim, a raised ring and the asset's ticker struck into the face
(from 11 px radius up; smaller coins are plain), spelled as the asset spells it. A token whose
ticker would read as XCH gets a plain coin, and no token gets XCH's green: a ticker and an
asset id are the token's own choice, and a coin must not pass for Chia's. A coin **locked in an open offer** carries a
lock and can still be selected. A coin **already being spent** by a pending transaction is
faded and cannot (a second spend of it could only fail; Sage's spendable check refuses it too).

## Coins in an offer

Combining or splitting a coin that is locked in an offer is how that offer is cancelled: Sage's
own **Cancel offer** spends one of the offer's coins back to the wallet in the same way. So the
wallet core needs no change for it, and nothing about the coins is lost
(`test_combine_coins_locked_in_an_offer` in `crates/sage-wallet/src/wallet/coin_management.rs`):

- Sage never picks such a coin for a payment (`selectable_coins`), but it is spendable when
  chosen on purpose (`spendable_coins`), so **Split** and **Combine** accept it.
- Once the spend confirms, the offer queue marks the offer **cancelled**; its other coins are
  free again, and the new coin is an ordinary one.
- If someone takes the offer first, the combine fails at the mempool and Sage removes it, as
  with any failed transaction; the coins went to the taker as the offer said.

Before that happens the coin list asks (`src/klimper/OfferCoinsWarning.tsx`): submitting
**Combine** or **Split** with offer coins selected opens "Combine coins locked in an offer?",
and nothing is built before **Cancel offers and combine**. The coins are read fresh at that
moment, so an offer made after the selection is asked about too, and a failed read spends
nothing. This guard holds for the coin list as well, with or without the pouch. "Select N tiny
coins" leaves offer coins out, so one tap never cancels an offer.

## When the coins change

The pouch reads the coin set, not the buttons, so it shows what the chain did, whatever asked
for it (`src/klimper/pouch/changes.ts`):

- **Pending**: the coins one transaction spends lean together and dim, about to become one.
- **Merge** (combine, or a payment with change): when it confirms, the spent coins pull into
  their middle, shrink and fade, and the new coin pops out there with one knock.
- **Split**: the spent coin pulls in and the new coins burst out of its place in all
  directions, knocking into the others.
- **Sent away** (nothing came back): the spent coins rise out of the pouch.
- **Received**: the new coins drop in from the top, one after another.

Spent coins of a pending transaction that go before anything new arrives wait 1.5 s for what
they became, so a combine whose new coin comes with the next sync update is still one movement.
The first fill of a pouch, and a different asset, just drop the coins in.

## Behind the Klimper coins flag

`VITE_ENABLE_KLIMPER_COINS=true` turns on everything that makes money feel like coins: the
coin sound and haptics (`docs/sound.md`) and this pouch. Off, the wallet is Sage's: the coin
card's one hook (`KlimperCoinPouch` in `src/components/OwnedCoinsCard.tsx`) renders nothing,
the pouch's code is not in the build (a lazy chunk), and nothing calls the native plugin, so
no motion sensor runs. The slide to approve keeps its own haptic: that is a control's feedback,
not the coins.

## The app stays upright

On a phone the app is portrait only (iPhone: `UISupportedInterfaceOrientations` in
`gen/apple/project.yml` and `Info.plist`; Android: `screenOrientation` in the manifest), so a
tilt moves the coins and never turns the screen. Klimper is an iPhone app (an iPad runs it as
one, upright), Android 16 ignores the lock on large screens by design, and the manifest marks
portrait as not required so a TV can still install the app.

## Pieces

| File                                           | What                                                                                                                                       |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `src/klimper/pouch/physics.ts`                 | Discs in a box: gravity, walls, knocks (sort and sweep), damping; asleep once nothing moved for half a second. Pure, tested.               |
| `src/klimper/pouch/layout.ts`                  | Disc size from the amount on a log scale, scaled to fill at most 42 % of the box; dust beyond 120; privacy mode makes every disc the same. |
| `src/klimper/pouch/changes.ts`                 | When the coins change: merges, splits, sends and receipts as movements; pure, tested.                                                      |
| `src/klimper/pouch/sprite.ts`                  | One coin drawn once per size and state: rim, ring, ticker, lock badge, faded when pending.                                                 |
| `src/klimper/pouch/CoinPouch.tsx`              | The canvas, the loop, tap and throw, the selected amount, the hint.                                                                        |
| `src/klimper/OfferCoinsWarning.tsx`            | The question before a combine or split cancels offers (not behind the flag: it guards Sage's coin list too).                               |
| `src/klimper/sound/player.ts`                  | `openPouch` (tilt and jolts from the phone) and `clink` (one knock), next to the ding and the shake.                                       |
| `tauri-plugin-klimper-jingle` `pouch`, `clink` | The same motion stream and the same coin hits and taps as the shake (`coin`, `perform` in Swift; `coin`, `vibrate` in Kotlin).             |

## Cost

Nothing while the coins rest: no animation frame. The frame loop runs only while something
moves, the tilt stream only while the pouch is on screen and the app in front, and the phone
sends gravity only when it turned by about 1.5° (a phone lying still sends nothing). Coins are
drawn from sprites made once per size and state (the rim, ticker and lock are drawn then, never
per frame). Knocks sound at most every 45 ms, and only with the coin
sound switched on and privacy mode off; while the pouch is open the shake and tilt do not sound
on their own, the pouch's coins do.
