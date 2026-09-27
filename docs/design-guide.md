# Klimper design guide: Beidwerk UI v2

The visual source of truth for the wallet. Where a screen and this file disagree, this file wins.
How it is wired into the code is at the end ([In this repository](#in-this-repository)).

- **Domain:** shared expenses, group money.
- **Character:** social finance with confidence. Ink, wine, coral-red and brass. Friendly, but
  not candy-coloured.
- **Primary identity hue:** Wine.

## 1. Beidwerk relationship

Klimper inherits the shared Beidwerk language: warm and cool material neutrals, black editorial
hierarchy, ruled layout, restrained print texture, orbital or architectural motifs and purposeful
motion traces. It must not look pastel, neon, or like a literal Bauhaus exercise.

## 2. Colour palette

| Role      | Hex       | Use                 |
| --------- | --------- | ------------------- |
| Canvas    | `#F2ECE5` | Warm paper          |
| Surface   | `#FCF8F2` | Surface             |
| Ink       | `#1B1A1E` | Text                |
| Graphite  | `#5B565D` | Muted text          |
| Wine      | `#7F3147` | Primary             |
| Coral red | `#B95445` | Payment / action    |
| Brass     | `#B38A3D` | Balance / highlight |
| Denim     | `#416179` | Secondary           |
| Stone     | `#847A74` | Neutral             |
| Hairline  | `#CDC1B7` | Borders             |

Colour behaviour:

- The primary hue is for the call to action, active navigation, focus emphasis and the first
  chart or data series.
- The secondary hue appears only where it adds domain meaning or visual separation.
- Large surfaces stay neutral. Never flood a whole screen with the accent.
- Material colours (steel, brass, copper, stone, concrete) are secondary identity cues.
- Semantic danger and warning colours are functional, not branding.

## 3. CSS tokens

```css
:root[data-app='klimper'] {
  --canvas: #f2ece5;
  --surface: #fcf8f2;
  --ink: #1b1a1e;
  --graphite: #5b565d;
  --wine: #7f3147;
  --coral-red: #b95445;
  --brass: #b38a3d;
  --denim: #416179;
  --stone: #847a74;
  --hairline: #cdc1b7;

  --bg: var(--canvas);
  --text: var(--ink);
  --text-muted: var(--graphite);
  --border: var(--hairline);
}

[data-app='klimper'][data-theme='dark'] {
  --bg: #171318;
  --surface: #231c22;
  --text: #f3eae1;
  --accent: #9b4059;
  --accent-secondary: #c26452;
  --border: color-mix(in srgb, #f3eae1 20%, #171318);
}
```

## 4. Component language

- **Navigation:** mostly neutral. The active item gets a narrow accent marker, a tinted field or
  strong text, never a big coloured pill.
- **Cards:** flat or lightly raised, 1px ruled borders, 10–14px radius.
- **Primary button:** solid identity colour with white or near-white text. No gradient.
- **Secondary button:** neutral surface, ink text, border.
- **Inputs:** at least 44px high, a clear focus ring in the identity colour, neutral at rest.
- **Tables:** strong typographic hierarchy, thin row rules, restrained zebra striping if needed.
- **Charts:** identity colour first, secondary colour second, then graphite and steel neutrals.
  No rainbow series.

```css
[data-app='klimper'] .button-primary {
  background: var(--accent, var(--wine));
  color: #fffaf2;
  border: 1px solid color-mix(in srgb, currentColor 0%, black 18%);
  border-radius: 10px;
  min-height: 44px;
  padding: 0 16px;
  font-weight: 700;
}

[data-app='klimper'] .card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: 0 8px 24px rgb(20 24 27 / 0.06);
}

[data-app='klimper'] :focus-visible {
  outline: 2px solid var(--accent, var(--wine));
  outline-offset: 2px;
}
```

## 5. Typography and layout

- Page title: heavy grotesk, tight tracking, one to three lines at most.
- Section labels: small mono or condensed uppercase.
- Body: ordinary sentence case.
- Numbers: tabular figures.
- An 8px spacing rhythm.
- Desktop layouts may be asymmetric, but functional alignment stays exact.
- One strong visual anchor per page rather than many decorative fragments.

## 6. Illustration and brand moments

Beidwerk shows through print grain, dither, technical linework, cropped circles and arcs,
bridge, planet and material imagery, and controlled collage. Use it in onboarding, empty states,
campaign panels and major section transitions. No heavy texture behind forms or transaction
data.

## 7. Motion

- 160–240ms for common UI transitions.
- Movement explains a change of state, a flow or a completion.
- No continuous pulsing, glowing or decorative looping in work areas.
- Respect `prefers-reduced-motion`.

## 8. Avoid

- pastel-first UI
- neon cyan and purple gradients
- generic startup blue everywhere
- glassmorphism
- primary red, blue or yellow as large default surfaces
- rainbow charts
- over-rounded "bubble" components
- poster texture over dense controls

## 9. Brief for agents

> Redesign Klimper using the Beidwerk UI v2 system. Keep the existing UX and information
> architecture unless a change is required for usability. Use the palette and CSS tokens in this
> file as the visual source of truth. The interface should feel editorial, material, technical and
> distinctive, but still production-grade. Keep most surfaces neutral, use Wine as the dominant
> interaction and identity colour, and use supporting colours only for clear data or semantic
> roles. Avoid pastel, neon, glassmorphism and generic SaaS gradients. Apply Beidwerk character
> through typography, ruled composition, asymmetry, print and material accents and restrained
> motion rather than through excessive decoration.

## In this repository

Sage styles everything through [theme-o-rama](https://www.npmjs.com/package/theme-o-rama)
themes, so the guide is two themes plus one stylesheet, and no shared component is edited (see
the _Look_ table in `UPSTREAM.md`). The two themes are named `light` and `dark` and so replace
Sage's core Light and Dark: every install, and the quick picker in Settings, gets Klimper by
default. Sage's other themes remain selectable and are untouched by `klimper.css`.

| What                                                                                                                     | Where                                                                                                      |
| ------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| Colours, radii, shadows, buttons, tables, switches                                                                       | `src/themes/klimper-light/theme.json`, `src/themes/klimper-dark/theme.json`                                |
| Guide tokens (`--wine`, `--brass` …), focus ring, 44px controls, tabular numbers, mono table headers, nav marker, motion | `src/klimper.css`, scoped to `html.theme-light` / `.theme-dark`                                            |
| Default theme, set once per install by system appearance (`klimper-theme-seeded-v2`), `data-app="klimper"`               | `src/lib/klimperTheme.ts`, imported once from `src/App.tsx`                                                |
| Logo                                                                                                                     | `src/icon-dark.png`, `src/icon-light.png`; app icons generated from it with `pnpm tauri icon` on `#F2ECE5` |

Mapping onto the shadcn roles: `primary` is wine, `background` canvas, `card` and `popover`
surface, `border` and `input` hairline, `muted-foreground` graphite, `ring` wine, `destructive`
a deeper red than coral so danger and payment never look alike.

Deliberate departures:

- **Dark primary text.** The guide's dark accent `#9B4059` reaches only 2.86:1 as text on
  `#171318`, below WCAG AA. Klimper Dark therefore uses `#C26A80` for `primary` (links, the nav
  marker, the focus ring) and keeps `#9B4059` as the primary button fill, where white text on it
  is 6.2:1.
- **Typeface.** No grotesk is bundled yet; titles use Inter or the system grotesk (SF Pro on
  Apple) at weight 800 with −0.02em tracking. Bundling a typeface is TASK-004.

Not done yet (TASK-004, TASK-008): coral-red and brass have no place in the current screens
because giving them one means editing shared components (send button, balance), which needs
hiding points upstream first; the onboarding illustration.
