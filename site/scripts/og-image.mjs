// The link-preview card (public/og.jpg, 1200×630): the home page's hero as it first loads.
// Without one, chat apps and social sites pick any image off the page (the album cover).
// Needs the dev server: `npm run dev`, then `npm run og-image`.
import puppeteer from 'puppeteer-core';
import { resolve } from 'node:path';
import sharp from 'sharp';

const out = resolve(import.meta.dirname, '../public/og.jpg');
const browser = await puppeteer.launch({ executablePath: 'C:/Program Files/Google/Chrome/Application/chrome.exe', headless: true });
const page = await browser.newPage();
// 1440 wide lays the hero out as on a desktop; 1.905:1 is the card's shape. Rendered at 2×
// and scaled down so the type stays crisp.
await page.setViewport({ width: 1440, height: 756, deviceScaleFactor: 2 });
await page.goto('http://localhost:4321/', { waitUntil: 'networkidle0' });
await page.evaluate(() => document.fonts.ready);
// Only the hero: the next section's top edge peeks into the first screen, and the film grain
// turns to noise when scaled down.
await page.addStyleTag({ content: 'main > .hero ~ *, footer, .grain { display: none !important; }' });
await new Promise((r) => setTimeout(r, 2500));
const shot = await page.screenshot();
await sharp(shot).resize(1200, 630).jpeg({ quality: 86, mozjpeg: true }).toFile(out);
console.log(out);
await browser.close();
