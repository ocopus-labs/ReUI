// Screenshots of the running dev site for review:
//   node scripts/shoot.mjs <out-dir> "path#hash|WxH|waitMs|scrollScreens" ...
// Drives the installed Chrome (puppeteer-core); scrolls in steps so scroll-driven scenes
// see every position on the way, then waits for the motion to settle.
import puppeteer from 'puppeteer-core';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

const [out = 'shots', ...specs] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const browser = await puppeteer.launch({ executablePath: 'C:/Program Files/Google/Chrome/Application/chrome.exe', headless: true });
for (const spec of specs.length ? specs : ['/']) {
  // A fresh page each time: a change of hash alone wouldn't reload it.
  const page = await browser.newPage();
  const [path, size = '1440x900', wait = '2500', screens = '0'] = spec.split('|');
  const [width, height] = size.split('x').map(Number);
  await page.setViewport({ width, height, deviceScaleFactor: 1 });
  await page.goto(`http://localhost:4321${path}`, { waitUntil: 'networkidle0' });
  const target = Number(screens) * height;
  for (let y = 0; y <= target; y += height / 8) {
    await page.evaluate((y) => window.scrollTo(0, y), Math.min(y, target));
    await new Promise((r) => setTimeout(r, 30));
  }
  await page.evaluate((y) => window.scrollTo(0, y), target);
  await new Promise((r) => setTimeout(r, Number(wait)));
  const name = `${(path.replace(/[^\w-]+/g, '_') || 'home')}-${width}-${screens}.png`;
  await page.screenshot({ path: join(out, name) });
  console.log(name);
  await page.close();
}
await browser.close();
