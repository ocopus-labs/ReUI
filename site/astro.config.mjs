// @ts-check
import { defineConfig } from 'astro/config';

// https://astro.build/config
export default defineConfig({
  site: 'https://re-ui.rohitk06.in',
  devToolbar: { enabled: false },
  vite: {
    // The site uses the app's own marks from design/assets (agents, devices), one level up.
    server: { fs: { allow: ['..'] } },
  },
});
