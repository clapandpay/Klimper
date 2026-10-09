export const offersEnabled = import.meta.env.VITE_DISABLE_OFFERS !== 'true';
export const optionsEnabled = import.meta.env.VITE_DISABLE_OPTIONS !== 'true';
export const swapEnabled = import.meta.env.VITE_DISABLE_SWAP !== 'true';
export const nftsEnabled = import.meta.env.VITE_DISABLE_NFTS !== 'true';
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
// On by default, like the flags above: "Use my Clap and Pay money" on the start screen
// (src/klimper/ClapAndPayConnect.tsx), release builds included (TASK-130). A passkey belongs to a website, so somebody who made their
// wallet by opening a Green Envelope has no other way to bring it here; leaving this off meant
// the one route from the page to the app was invisible in every build that did not ask for it.
// `VITE_DISABLE_CLAPANDPAY_CONNECT=true` turns it off.
export const clapAndPayConnectEnabled =
  import.meta.env.VITE_DISABLE_CLAPANDPAY_CONNECT !== 'true';
// A release build only ever talks to an https origin: a local development page left in a
// developer's .env must not reach a phone.
const configuredOrigin = import.meta.env.VITE_CLAPANDPAY_ORIGIN;
export const clapAndPayOrigin =
  configuredOrigin &&
  (!import.meta.env.PROD || configuredOrigin.startsWith('https://'))
    ? configuredOrigin
    : 'https://envelope.clapandpay.com';
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
