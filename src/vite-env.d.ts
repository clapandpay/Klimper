/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_DISABLE_OFFERS?: string;
  readonly VITE_DISABLE_OPTIONS?: string;
  readonly VITE_DISABLE_SWAP?: string;
  readonly VITE_DISABLE_NFTS?: string;
  readonly VITE_DISABLE_NFT_MINTING?: string;
  readonly VITE_DISABLE_PROFILES?: string;
  readonly VITE_DISABLE_TOKEN_ISSUING?: string;
  readonly VITE_DISABLE_WALLETCONNECT?: string;
  readonly VITE_DISABLE_RPC_SETTINGS?: string;
  readonly VITE_DISABLE_THEME_GALLERY?: string;
  readonly VITE_ENABLE_HANDLES?: string;
  readonly VITE_DISABLE_CLAPANDPAY_CONNECT?: string;
  readonly VITE_CLAPANDPAY_ORIGIN?: string;
  readonly VITE_ENABLE_XCHONNECT?: string;
  readonly VITE_ENABLE_KLIMPER_COINS?: string;
  readonly VITE_DISABLE_SCAN?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

// Allow importing JSON files as modules
declare module '*.json' {
  const value: never;
  export default value;
}
