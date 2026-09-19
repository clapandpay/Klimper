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
