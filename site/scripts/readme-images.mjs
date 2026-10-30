// The README's pictures (docs/images), taken from the site's own island at 2×, on the sky
// of the hero photograph (Super Visuals, CC BY 4.0). Real content: a real track and cover,
// real logos. Needs the dev server: `npm run dev`, then `npm run readme-images`.
// Each picture is the island with room round it: the drawn shape (and the switcher under
// a place's card) plus a margin, as at the top of a screen.
import puppeteer from 'puppeteer-core';
import { join, resolve } from 'node:path';
import sharp from 'sharp';

const out = resolve(import.meta.dirname, '../../docs/images');
const shots = [
  // name, island state, options; `wide` frames it like a screen's top centre.
  ['hero', 'now-playing', { wide: true }],
  ['now-playing', 'music'],
  ['clipboard', 'clipboard'],
  ['volume', 'volume'],
  ['devices', 'earbuds-card'],
  ['system-stats', 'stats-card'],
  ['timers', 'timer-card'],
  ['agents', 'permission'],
  ['appearance-light', 'timer-card', { theme: 'light' }],
  ['appearance-notch', 'timer-card', { style: 'notch' }],
];

const browser = await puppeteer.launch({ executablePath: 'C:/Program Files/Google/Chrome/Application/chrome.exe', headless: true });
for (const [name, moment, o = {}] of shots) {
  const page = await browser.newPage();
  await page.setViewport({ width: 1400, height: 700, deviceScaleFactor: 2 });
  const hash = new URLSearchParams({ island: moment, ...(o.theme && { theme: o.theme }), ...(o.style && { style: o.style }) });
  await page.goto(`http://localhost:4321/shots#${hash}`, { waitUntil: 'networkidle0' });
  await page.evaluate(() => document.fonts.ready);
  await new Promise((r) => setTimeout(r, 1500));
  const box = await page.evaluate(() => {
    const parts = ['[data-pill]', '[data-switcher].shown'].map((s) => document.querySelector(s)).filter(Boolean);
    const rects = parts.map((el) => el.getBoundingClientRect());
    return { left: Math.min(...rects.map((r) => r.left)), right: Math.max(...rects.map((r) => r.right)), bottom: Math.max(...rects.map((r) => r.bottom)) };
  });
  const margin = 48;
  const width = o.wide ? 960 : Math.round(box.right - box.left + margin * 2);
  const clip = { x: Math.round(700 - width / 2), y: 0, width, height: Math.round(Math.max(o.wide ? 256 : 0, box.bottom + 24)) };
  // Rounded corners baked in (GitHub can't round an image), so a picture sits on the
  // page as a card rather than a slab, on the light theme and the dark one.
  const shot = await page.screenshot({ clip });
  const [w, h, r] = [clip.width * 2, clip.height * 2, 32];
  const mask = Buffer.from(`<svg width="${w}" height="${h}"><rect width="${w}" height="${h}" rx="${r}" ry="${r}"/></svg>`);
  await sharp(shot).composite([{ input: mask, blend: 'dest-in' }]).png().toFile(join(out, `${name}.png`));
  console.log(`${name}.png`, `${clip.width * 2}×${clip.height * 2}`);
  await page.close();
}
await browser.close();
