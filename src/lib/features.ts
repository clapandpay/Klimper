// SPDX-License-Identifier: Apache-2.0
// Modified by Klimper (2026): see docs/UPSTREAM.md
export const offersEnabled = import.meta.env.VITE_DISABLE_OFFERS !== 'true';
export const optionsEnabled = import.meta.env.VITE_DISABLE_OPTIONS !== 'true';
export const swapEnabled = import.meta.env.VITE_DISABLE_SWAP !== 'true';
export const nftsEnabled = import.meta.env.VITE_DISABLE_NFTS !== 'true';
// Minting is separate from the NFT gallery (TASK-138): Klimper shows the user's NFTs and lets
// them send, hide and burn them, but makes none (decision-003). `VITE_DISABLE_NFT_MINTING`.
export const nftMintingEnabled =
  import.meta.env.VITE_DISABLE_NFT_MINTING !== 'true';
export const profilesEnabled = import.meta.env.VITE_DISABLE_PROFILES !== 'true';
export const tokenIssuingEnabled =
  import.meta.env.VITE_DISABLE_TOKEN_ISSUING !== 'true';
export const walletConnectEnabled =
  import.meta.env.VITE_DISABLE_WALLETCONNECT !== 'true';
export const rpcSettingsEnabled =
  import.meta.env.VITE_DISABLE_RPC_SETTINGS !== 'true';
export const themeGalleryEnabled =
  import.meta.env.VITE_DISABLE_THEME_GALLERY !== 'true';
// Opt-in, unlike the flags above: "@handle" in the send screen, resolved through XCHandles.
export const handlesEnabled = import.meta.env.VITE_ENABLE_HANDLES === 'true';
// Opt-in: dApp connections over Xchonnect (decision-008, milestone m-4). Off unless set to
// true, so a build that does not ask for it has no Xchonnect entry point at all — the
// routes redirect and the navigation entry is absent. The Rust plugin is always compiled
// in, but it only does anything once a screen calls it.
export const xchonnectEnabled =
  import.meta.env.VITE_ENABLE_XCHONNECT === 'true';
// Opt-in: the Klimper coins, everything that makes the money feel like coins: the coin sound
// and haptics (decision-009, TASK-111: a ding on receive, coins on shake and tilt) and the coin
// pouch on a token page (TASK-124). Off unless set to true; then the sound is also a switch in
// Settings. It all lives in src/klimper/sound/ and src/klimper/pouch/, loaded only when the flag
// is on, and the wallet works the same without it.
export const klimperCoinsEnabled =
  import.meta.env.VITE_ENABLE_KLIMPER_COINS === 'true';
// On by default: the Scan tab (src/klimper/Scanner.tsx), one camera for every code — a
// website's Xchonnect code, an offer, an address. `VITE_DISABLE_SCAN=true` turns it off.
export const scanEnabled = import.meta.env.VITE_DISABLE_SCAN !== 'true';
// Opt-in: partial offers (CHIP-0052, TASK-164): view a `partial1…`, take an amount of it,
// make one, list and cancel the wallet's own. Needs offers. Off unless set to true, and off
// in the release builds until it has been tried on testnet; it all lives in
// src/klimper/partial/ and tauri-plugin-klimper-partial-offers (docs/partial-offers.md).
export const partialOffersEnabled =
  offersEnabled && import.meta.env.VITE_ENABLE_PARTIAL_OFFERS === 'true';
