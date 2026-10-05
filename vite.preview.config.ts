import { lingui } from '@lingui/vite-plugin';
import react from '@vitejs/plugin-react-swc';
import path from 'path';
import { defineConfig } from 'vite';
export default defineConfig({
  plugins: [react({ plugins: [['@lingui/swc-plugin', {}]] }), lingui()],
  server: { port: 5199, strictPort: true },
  resolve: {
    alias: [
      { find: '@/components/Layout', replacement: path.resolve(__dirname, './src/preview/Layout.tsx') },
      { find: '@/components/Header', replacement: path.resolve(__dirname, './src/preview/Header.tsx') },
      { find: '@', replacement: path.resolve(__dirname, './src') },
    ],
  },
});
