// SPDX-License-Identifier: AGPL-3.0-or-later
// Stand-in for Sage's biometric hook in the approvals preview (vite.preview.config.ts): the
// real one needs a BiometricProvider, whose module asks the Tauri OS plugin for the platform
// as it loads, and a plain browser has no Tauri. Approving in the preview asks nothing.
export function useBiometric() {
  return {
    enabled: false,
    available: false,
    promptIfEnabled: () => Promise.resolve(true),
    enableIfAvailable: () => Promise.resolve(),
    disable: () => Promise.resolve(),
  };
}
