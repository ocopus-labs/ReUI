// The README's badges (docs/images/badges/*.png), from /badges: each capsule photographed
// on a transparent background at 2×. Needs the dev server: `npm run dev`, then
// `npm run badges`.
import puppeteer from 'puppeteer-core';
import { mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';

const out = resolve(import.meta.dirname, '../../docs/images/badges');
mkdirSync(out, { recursive: true });
const browser = await puppeteer.launch({ executablePath: 'C:/Program Files/Google/Chrome/Application/chrome.exe', headless: true });
const page = await browser.newPage();
await page.setViewport({ width: 600, height: 800, deviceScaleFactor: 2 });
await page.goto('http://localhost:4321/badges', { waitUntil: 'networkidle0' });
await page.evaluate(() => document.fonts.ready);
for (const el of await page.$$('[data-badge]')) {
  const name = await el.evaluate((e) => e.dataset.badge);
  const box = await el.boundingBox();
  await el.screenshot({ path: join(out, `${name}.png`), omitBackground: true });
  console.log(`${name}.png`, `${Math.round(box.width)}×${Math.round(box.height)} (shown at that size)`);
}
await browser.close();
