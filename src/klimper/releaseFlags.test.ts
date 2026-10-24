// SPDX-License-Identifier: AGPL-3.0-or-later
// What each release build turns on (TASK-160). Desktop (.env.desktop, `vite build --mode
// desktop`): everything, the Sage app bridge included. Phones (.env.production): everything
// except offers; the app bridge is off there by Sage's own platform check in App.tsx.
import { readFileSync } from 'node:fs';
import { loadEnv } from 'vite';
import { describe, expect, it } from 'vitest';

const root = `${__dirname}/../..`;
const flags = (mode: string) => loadEnv(mode, root, 'VITE_');
const json = (file: string) =>
  JSON.parse(readFileSync(`${root}/src-tauri/${file}`, 'utf8')) as {
    build?: { beforeBuildCommand?: string };
  };

const SWITCHES = [
  'OFFERS',
  'OPTIONS',
  'SWAP',
  'NFTS',
  'NFT_MINTING',
  'PROFILES',
  'TOKEN_ISSUING',
  'WALLETCONNECT',
  'RPC_SETTINGS',
  'THEME_GALLERY',
  'SCAN',
];
const OPT_IN = ['HANDLES', 'XCHONNECT', 'KLIMPER_COINS'];

describe('release flags', () => {
  it('turn everything on for the desktop', () => {
    const env = flags('desktop');
    for (const name of SWITCHES) {
      expect(env[`VITE_DISABLE_${name}`], name).toBe('false');
    }
    for (const name of OPT_IN) {
      expect(env[`VITE_ENABLE_${name}`], name).toBe('true');
    }
  });

  it('turn everything on for the phones except offers', () => {
    const env = flags('production');
    for (const name of SWITCHES) {
      expect(env[`VITE_DISABLE_${name}`], name).toBe(
        name === 'OFFERS' ? 'true' : 'false',
      );
    }
    for (const name of OPT_IN) {
      expect(env[`VITE_ENABLE_${name}`], name).toBe('true');
    }
  });

  it('are picked per platform by the Tauri configs', () => {
    for (const p of ['macos', 'linux', 'windows']) {
      expect(json(`tauri.${p}.conf.json`).build?.beforeBuildCommand, p).toBe(
        'pnpm run build:desktop',
      );
    }
    for (const p of ['ios', 'android']) {
      expect(json(`tauri.${p}.conf.json`).build?.beforeBuildCommand, p).toBe(
        'pnpm run build:frontend',
      );
    }
  });

  it('keep the app bridge off on phones (Sage’s own check)', () => {
    const app = readFileSync(`${root}/src/App.tsx`, 'utf8');
    expect(app).toMatch(
      /supportsSageApps =\s*currentPlatform !== 'android' && currentPlatform !== 'ios'/,
    );
  });
});
