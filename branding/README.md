# Klimper logo

The vector sources of the logo (1024 viewBox, exact geometry): `klimper-logo-light.svg` on the
light canvas, `klimper-logo-dark.svg` on the dark canvas, `klimper-logo-mark.svg` without a
canvas. Every raster in the app is rendered from them:

- `src/icon-light.png`, `src/icon-dark.png`: the light and dark SVG at 1024px, opaque.
- the app icons: `scripts/update-logo.sh` with the light SVG rendered at 2048px.
- the backdrops in `src/themes/klimper-light/` and `klimper-dark/` embed the mark's shapes as an
  SVG data URI (scaled 0.75 at 1000,440 on a 1600×1000 canvas), so they stay sharp at any size.
