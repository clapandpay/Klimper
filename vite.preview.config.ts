// SPDX-License-Identifier: AGPL-3.0-or-later
import { lingui } from '@lingui/vite-plugin';
import react from '@vitejs/plugin-react-swc';
import path from 'path';
import { defineConfig } from 'vite';
export default defineConfig({
  plugins: [react({ plugins: [['@lingui/swc-plugin', {}]] }), lingui()],
  server: { port: 5199, strictPort: true },
  // The preview is of the Xchonnect and card screens: on whatever the shell's environment says.
  define: {
    'import.meta.env.VITE_ENABLE_XCHONNECT': JSON.stringify('true'),
    'import.meta.env.VITE_ENABLE_GNOSIS_PAY': JSON.stringify('true'),
  },
  resolve: {
    alias: [
      {
        find: '@/components/Layout',
        replacement: path.resolve(__dirname, './src/preview/Layout.tsx'),
      },
      {
        find: '@/components/Header',
        replacement: path.resolve(__dirname, './src/preview/Header.tsx'),
      },
      {
        find: '@/hooks/useBiometric',
        replacement: path.resolve(__dirname, './src/preview/useBiometric.ts'),
      },
      { find: '@', replacement: path.resolve(__dirname, './src') },
    ],
  },
});
