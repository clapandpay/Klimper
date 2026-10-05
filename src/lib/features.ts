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
// (src/pages/ClapAndPayConnect.tsx). A passkey belongs to a website, so somebody who made their
// wallet by opening a Green Envelope has no other way to bring it here; leaving this off meant
// the one route from the page to the app was invisible in every build that did not ask for it.
// `VITE_DISABLE_CLAPANDPAY_CONNECT=true` turns it off.
export const clapAndPayConnectEnabled =
  import.meta.env.VITE_DISABLE_CLAPANDPAY_CONNECT !== 'true';
export const clapAndPayOrigin =
  import.meta.env.VITE_CLAPANDPAY_ORIGIN || 'https://clapandpay.com';
// Opt-in: dApp connections over Xchonnect (decision-008, milestone m-4). Off unless set to
// true, so a build that does not ask for it has no Xchonnect entry point at all — the
// routes redirect and the navigation entry is absent. The Rust plugin is always compiled
// in, but it only does anything once a screen calls it.
export const xchonnectEnabled =
  import.meta.env.VITE_ENABLE_XCHONNECT === 'true';
