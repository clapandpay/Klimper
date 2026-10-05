# Klimper design guide: Klimper Coin

The visual source of truth for the wallet. Where a screen and this file disagree, this file wins.
How it is wired into the code is at the end ([In this repository](#in-this-repository)).

- **Domain:** shared expenses, group money.
- **Character:** bright, modern and warm; money that moves. The colours and shapes come from the
  logo: a bird flying out of a red-orange sun, with teal and red coins in its trail.
- **It must not look like Sage.** The wallet engine is Sage's; the face is ours.

## History

The first draft was _Beidwerk UI v2_ (ink, wine, coral-red and brass on warm paper; ruled and
restrained). In the app it still read as Sage with a warmer paint, so the owner asked for a
brighter, more colourful direction built on the coin and the logo (2026-09-27). Beidwerk's
structure stays: neutral work surfaces, one strong anchor per screen, a restrained type system,
purposeful motion. The Beidwerk text is in the git history of this file.

## 1. Colour

| Role        | Light                 | Dark      | Use                                                    |
| ----------- | --------------------- | --------- | ------------------------------------------------------ |
| Canvas      | `#FFF7EC`             | `#0F1014` | page background, cream paper                           |
| Surface     | `#FFFFFF`             | `#1A1C22` | cards, dialogs, inputs                                 |
| Ink         | `#111217`             | `#FFF4E6` | text; the sidebar is ink in both themes                |
| Muted       | `#5C606B`             | `#A8ACB5` | secondary text                                         |
| Hairline    | `#EFDCC4`             | `#2C2F37` | borders                                                |
| Sun         | `#FF3B1F` → `#FF9A1F` | same      | the gradient: primary action, coins, the total balance |
| Sun red     | `#FF3B1F`             | same      | small marks only (dots, badges)                        |
| Sun orange  | `#FF7A1F`             | same      | active navigation, hover edges                         |
| Sun yellow  | `#FFC21A`             | same      | rims and highlights                                    |
| Teal        | `#0F7C87`             | `#3FC1CC` | second action (Receive), secondary buttons, focus ring |
| Primary     | `#C73515`             | `#FF6A3D` | text links and `text-primary`, readable on the canvas  |
| Destructive | `#B42318`             | `#F04438` | functional only, never branding                        |

Rules:

- **The sun is the one loud thing.** Use it for the primary action, coins and one anchor per
  screen (on the assets screen, the total balance band). Never as a page or card background.
- **Text on the sun is ink, not white.** White on orange fails contrast; ink holds it across the
  whole gradient.
- **Teal is the counterweight:** the second action next to a sun button, secondary buttons, focus.
- Work surfaces stay white or graphite. Colour lives in actions, coins and the backdrop.
- No neon, no purple, no startup blue, no rainbow charts. Charts: sun first, teal second, then
  muted greys.

## 2. The coin

The coin is Klimper's shape.

- An asset without its own icon is a **sun coin**: a circle filled with the sun gradient, an ink
  glyph, a light inner rim. An asset with an icon is round with a sun-yellow rim.
- Icon buttons are circles; buttons are pills; cards are rounded tiles (20px).
- The backdrop is the logo's motion: a sun rising from the bottom-right corner and a trail of two
  coins and three streaks. It sits in the corner of the canvas, behind the content, never behind a
  form or a table. On a phone it shrinks into the corner.

## 3. Components

- **Sidebar:** ink, in both themes. The active item is a tinted field with an orange icon.
- **Cards:** white (graphite in dark), 1px hairline, 20px radius, a warm glow; linked cards lift by
  2px on hover.
- **Primary button:** a pill filled with the sun, ink text, 700 weight, a soft sun shadow. No other
  gradient anywhere.
- **Secondary button:** a teal pill with white text.
- **Outline button:** a white pill with a hairline that turns orange on hover.
- **Inputs:** at least 44px high, white, hairline border, teal focus ring.
- **Tables:** mono uppercase column labels in muted grey, thin row rules.
- **Money:** balances in the display face, heavy, tabular figures.

## 4. Type

- **Display** (titles, balances): rounded grotesk, 800 weight, −0.03em tracking. `ui-rounded` (SF
  Pro Rounded on Apple), then Nunito, then Inter.
- **Body:** Inter or the system sans, sentence case.
- **Labels:** small mono uppercase (JetBrains Mono, SF Mono).
- Numbers: tabular figures everywhere.
- 8px spacing rhythm.

## 5. Motion

- 160–240ms for common transitions (the stylesheet uses 200ms with a soft ease-out).
- Movement explains: a card lifts when it can be opened, a button settles when pressed.
- No looping, pulsing or glowing in work areas. `prefers-reduced-motion` turns transitions off.

## 6. Avoid

- anything that makes it look like Sage again: grey sidebars, black primary buttons, square tiles
- white text on the sun
- the sun as a large surface behind content
- glassmorphism, neon, purple gradients, rainbow charts
- decoration behind forms, tables or transaction data

## In this repository

Sage styles everything through [theme-o-rama](https://www.npmjs.com/package/theme-o-rama)
themes, so the design is two themes plus one stylesheet, and no shared component is edited (see
the _Look_ table in `UPSTREAM.md`). The two themes are named `light` and `dark` and so replace
Sage's core Light and Dark: every install, and the quick picker in Settings, gets Klimper by
default. Sage's other themes remain selectable and are untouched by `klimper.css`.

| What                                                                             | Where                                                                                |
| -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Colours, radii, shadows, fonts, tables, switches, the backdrop (inline SVG)      | `src/themes/klimper-light/theme.json`, `src/themes/klimper-dark/theme.json`          |
| Sidebar, buttons, coins, total band, Send/Receive, focus, 44px controls, motion  | `src/klimper.css`, scoped to `html.theme-light` / `.theme-dark`                      |
| Default theme, once per install by system appearance (`klimper-theme-seeded-v2`) | `src/lib/klimperTheme.ts`, imported once from `src/App.tsx`                          |
| Logo                                                                             | `src/icon-{dark,light}.png` in the app; every app icon from `scripts/update-logo.sh` |

`klimper.css` finds Sage's elements by structure and by Sage's own class hooks (`btn-variant-*`,
`bg-card`, `shadow-card`), never by translated text. Three rules depend on a particular upstream
layout and are the first to check after an upstream merge: the sidebar
(`[style*='--sidebar-background']`), the total balance band (`.mt-6.pt-4.border-t` on the assets
screen) and Send/Receive (the first two outline buttons in the token card's action row).

Not done yet (TASK-004, TASK-008): a bundled display typeface (the rounded face is the system's
where it has one); the onboarding illustration; a Klimper home screen of our own, which needs a
screen rather than more CSS.

### Changing the logo

1. Replace `src/icon-light.png` (the logo on the light canvas; square, at least 1024px, opaque) and
   `src/icon-dark.png` (the same on the dark canvas). These two are the logo inside the app.
2. Run `scripts/update-logo.sh` from `klimper/`. It rebuilds every app icon from
   `src/icon-light.png`: full bleed for iOS and Android, the macOS rounded square for the desktop,
   and the Android launcher background from the logo's corner colour. Pass another file to use
   that instead: `scripts/update-logo.sh ~/Downloads/logo.png`.
3. Restart `pnpm tauri:dev`. macOS can keep showing the old Dock icon until the app restarts.
