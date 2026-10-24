// Modified by Klimper (2026): see docs/UPSTREAM.md
import { createRoot } from 'react-dom/client';

import '@fontsource/roboto/300.css';
import '@fontsource/roboto/400.css';
import '@fontsource/roboto/500.css';
import '@fontsource/roboto/700.css';

import './setup.ts';

import App from './App.tsx';
import { AppRecovery } from './klimper/KlimperErrorBoundary'; // Klimper: never a black screen

const element = document.getElementById('root') as HTMLElement;

createRoot(element).render(
  <AppRecovery>
    <App />
  </AppRecovery>,
);
