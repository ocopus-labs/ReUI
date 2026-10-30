# ReUI site

The landing page (Astro, static). Not deployed yet.

```
npm run dev       # http://localhost:4321
npm run build     # into dist/
npm run images          # re-render the app's stages into src/assets/island (needs cargo)
npm run readme-images   # the README's pictures into docs/images, from this island (dev server running)
```

The island pictures are drawn by Settings' shot mode, not captured from a screen: each stage is
rendered on black and on white, and the pair gives a transparent PNG, so the page can put
them on any wallpaper (`src/assets/wallpaper.svg`, a stand-in; swap for a photo). Copy
follows the README; keep the two in step.
