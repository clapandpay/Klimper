# NFT auctions

The owner asked for "dutch offers from upstream" (2026-11-07). Upstream has no dutch (falling
price) auction anywhere; chia-wallet-sdk has an English one (xch-dev/chia-wallet-sdk #507,
merged 2026-10-27, also in Chia-Network's cni-wallet-sdk 0.39.0 of 2026-10-29): one NFT sold to
rising bids. That is what Klimper uses, with the owner's scope for a first version: start an
auction for one of the wallet's NFTs, open any auction by its id (a link, a QR code
`auction:0x<id>`, or pasted), bid in XCH, end it, cancel the wallet's own before a bid. Auctions
priced in a CAT open read-only. There is no list of other people's auctions: nothing indexes
them yet (nodexch could).

## How an auction works (the SDK's)

- A singleton on the action layer, with three actions: bid, end, cancel. It locks the NFT and a
  reserve coin at a lock puzzle (`auction_lock_p2_puzzle_hash(launcher id)`, a MIPS puzzle whose
  only member is the auction).
- Settings, fixed at launch: minimum bid and increment (flat or percent), end time and grace
  period, buyer's premium and commission (basis points, to puzzle hashes), payout puzzle hash
  (the seller). The NFT's royalty comes from the NFT.
- A bid locks the bid plus premium plus royalty in the reserve and refunds the previous bidder
  in full. Before the end time a bid asserts it is before it; after it, a grace bid must come
  within the grace period of the last spend.
- Ending (anyone, after the end time and the grace period since the last spend) sends the NFT to
  the winner (back to the seller with no bid) and pays out. The seller can cancel before the
  first bid; the NFT comes back.

## Layout

| Part                | Where                                                                                                            | Replaced by                                                   |
| ------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| Driver              | `crates/chia-sdk-next/src/auction/` (the SDK's files, copied onto 0.37.1; the SDK's 91 auction tests pass there) | chia-wallet-sdk's own, once Sage depends on a release with it |
| Wallet and commands | `tauri-plugin-klimper-auctions` (reuses the partial offers plugin's wallet context)                              | Sage's endpoints, if it ships auctions                        |
| Screens             | `src/klimper/auctions/` (`api.ts` is the only file naming the commands)                                          | kept, with a new `api.ts`                                     |

Commands: `view_auction`, `start_auction`, `bid_auction`, `end_auction`, `cancel_auction`,
`get_auctions`, `delete_auction`. The wallet's auctions (started or bid on) are kept in
`<network>-klimper-auctions.json` next to Sage's wallet database; losing it loses nothing (the
launcher id is also in the memo of the NFT's send to the auction and in a remark of each bid).

## What the wallet adds to the driver (challenger review, 2026-11-07)

- **The NFT is checked on the chain before an auction is biddable**: the coin it names exists,
  is unspent, is a standard NFT, and is locked by this auction's lock. Otherwise the auction can
  never end and a winning bid stays locked. The royalty is taken from that NFT.
- **Timing bounds**: no start or bid when the end is more than 90 days away or the grace period
  is over a day; a start must run at least ten minutes. The grace rule is the puzzles' (above);
  block times lag, so a margin of a minute applies. The last spend's time comes from its block
  header, only for auctions with a grace period (without it no grace bid is offered, and an end
  waits out the whole grace period after the end time).
- **Every wallet spend is tied to the auction spend it pays for**: a bid's coins assert its
  announcement; an end's or cancel's fee spend asserts the auction coin's spend. A bid is
  checked before signing: the driver runs it as the puzzle will, the reserve amount matches
  what was shown, and inputs less outputs equal the network fee.
- **Start**: the launching coin is picked first (the launcher's id and the lock follow from
  it) and never used for anything else; the NFT goes to the lock through Sage's spends; the
  launching coin's spend makes the launcher, the empty reserve and change, and asserts the NFT's
  spend. No buyer's premium, no commission: Klimper takes nothing.
- **Cancel**: the auction accepts a cancel only from a coin at the seller's puzzle hash; the
  wallet makes one of nothing there and spends it in the same bundle.
- **Reading**: only confirmed spends are followed; an `Nft` is re-read into the context that
  spends it (its metadata lives in the allocator that read it); "cancelled" and "ended without a
  bid" are told apart by the action that ran.
- The screens show the NFT by its id (names and pictures can be copied), what a bid locks up and
  where each part goes, and the premium and commission of another seller's auction.

## Tests

`tauri-plugin-klimper-auctions/src/tests.rs`, on Sage wallets and a simulated chain: an NFT
minted, auctioned, bid on by two wallets (the first refunded in full; each bidder's balance down
by exactly bid, royalty and fee), ended after the end time (the seller paid, the NFT in the
winner's Sage database); a cancel before a bid (the NFT back in the seller's database, nobody
else can cancel); an id that is not an auction refused. Frontend: `math.test.ts`, the scan target.

Not yet: a run on testnet11 (then turn `VITE_ENABLE_AUCTIONS` on in the release files), bids in
a CAT, an index of auctions.
