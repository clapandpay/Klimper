// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 the Klimper authors
//
// Klimper's look: the Beidwerk UI v2 themes in src/themes/klimper-{light,dark} (named light and dark) and the rules in
// src/klimper.css that a theme.json cannot express (docs/design-guide.md is the source of truth).
// Imported once from App.tsx, before theme-o-rama's ThemeProvider reads localStorage.
import '../klimper.css';

const THEME_KEY = 'theme';
const LAST_NON_CORE_KEY = 'theme-o-rama-last-used-non-core-theme';
// Set once Klimper has chosen the theme on this install. Bump the suffix to apply a new default to
// every install again.
const SEEDED_KEY = 'klimper-theme-seeded-v2';

// The Klimper themes are named "light" and "dark": they replace Sage's core themes, so an install
// already on light or dark gets Klimper with nothing saved. Once per install, pick the one that
// matches the system appearance (theme-o-rama would otherwise always open in light); from then on
// the user's choice is kept.
function seedDefaultTheme() {
  try {
    if (localStorage.getItem(SEEDED_KEY)) return;
    const dark = window.matchMedia?.('(prefers-color-scheme: dark)').matches;
    const theme = dark ? 'dark' : 'light';
    localStorage.setItem(THEME_KEY, theme);
    // Earlier builds saved "klimper-light" / "klimper-dark", which no longer exist.
    if (localStorage.getItem(LAST_NON_CORE_KEY)?.startsWith('klimper-')) {
      localStorage.removeItem(LAST_NON_CORE_KEY);
    }
    localStorage.setItem(SEEDED_KEY, theme);
  } catch {
    // Storage can be unavailable; theme-o-rama then opens in light, which is Klimper Light.
  }
}

seedDefaultTheme();
document.documentElement.dataset.app = 'klimper';
