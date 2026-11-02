// The clipboard picture for the README and the website: the panel as the island itself
// draws it (offscreen, `cargo test -p reui-island clipboard_panel_sheet -- --ignored`
// writes target/clipboard-panel/panel-200.png), set on the site's wallpaper the way
// readme-images.mjs frames the others: at the top as on a screen, 48 px either side, 24
// below, rounded corners baked in. The site's own island has no clipboard of this design,
// so this one comes from the app.
//   node scripts/clipboard-image.mjs
import sharp from 'sharp';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '../..');
const panel = resolve(root, 'target/clipboard-panel/panel-200.png');
const out = resolve(root, 'docs/images/clipboard.png');

// Device pixels: the README pictures are taken at 2×.
const scale = 2;
const [top, side, bottom] = [11, 48, 24].map((d) => d * scale);
const { width: pw, height: ph } = await sharp(panel).metadata();
const [w, h] = [pw + side * 2, top + ph + bottom];

// The wallpaper as the site's page shows it behind the island: its top, across a 1400 px
// wide window, centred.
const window = 1400 * scale;
const wall = await sharp(resolve(root, 'site/src/assets/hero.jpg')).resize({ width: window }).toBuffer();
const backdrop = await sharp(wall).extract({ left: Math.round((window - w) / 2), top: 0, width: w, height: h }).toBuffer();

const mask = Buffer.from(`<svg width="${w}" height="${h}"><rect width="${w}" height="${h}" rx="32" ry="32"/></svg>`);
const framed = await sharp(backdrop).composite([{ input: panel, left: side, top }]).png().toBuffer();
await sharp(framed).composite([{ input: mask, blend: 'dest-in' }]).png().toFile(out);
console.log(`clipboard.png ${w}×${h}`);
