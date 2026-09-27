// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 the Klimper authors
//
// Klimper's look: the Beidwerk UI v2 themes in src/themes/klimper-{light,dark} and the rules in
// src/klimper.css that a theme.json cannot express (docs/design-guide.md is the source of truth).
// Imported once from App.tsx, before theme-o-rama's ThemeProvider reads localStorage.
import '../klimper.css';

const THEME_KEY = 'theme';
const LAST_NON_CORE_KEY = 'theme-o-rama-last-used-non-core-theme';
// Set once Klimper has chosen its theme on this install. Bump the suffix to apply a new default
// to every install again.
const SEEDED_KEY = 'klimper-theme-seeded-v1';

// theme-o-rama opens in Sage's plain "light" unless a theme is saved, and installs that ran
// before the Klimper themes existed may have a Sage theme saved. So once per install, pick the
// Klimper theme that matches the system appearance; from then on the user's choice is kept.
function seedDefaultTheme() {
  try {
    if (localStorage.getItem(SEEDED_KEY)) return;
    const dark = window.matchMedia?.('(prefers-color-scheme: dark)').matches;
    const theme = dark ? 'klimper-dark' : 'klimper-light';
    localStorage.setItem(THEME_KEY, theme);
    // The quick selector in Settings offers light, dark and the last non-core theme.
    localStorage.setItem(LAST_NON_CORE_KEY, theme);
    localStorage.setItem(SEEDED_KEY, theme);
  } catch {
    // Storage can be unavailable; the Sage default then applies.
  }
}

seedDefaultTheme();
document.documentElement.dataset.app = 'klimper';
