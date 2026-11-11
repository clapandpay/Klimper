// SPDX-License-Identifier: AGPL-3.0-or-later
// Tests run on Sage's Vite config, with two Klimper additions.
//
// From Node 25, Node has a `localStorage` of its own, which is undefined unless Node is given a
// storage file. It shadows jsdom's, so every test that touches storage fails before it starts
// ("Cannot read properties of undefined (reading 'clear')"). The test workers run without it.
//
// The app SDK's manifest test is a `node --test` file (its package's `pnpm test`), not Vitest's.
import { configDefaults, defineConfig, mergeConfig } from 'vitest/config';
import viteConfig from './vite.config';

const nodeMajor = Number(process.versions.node.split('.')[0]);

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      execArgv: nodeMajor >= 25 ? ['--no-experimental-webstorage'] : [],
      exclude: [...configDefaults.exclude, 'packages/sage-app-sdk/test/**'],
    },
  }),
);
