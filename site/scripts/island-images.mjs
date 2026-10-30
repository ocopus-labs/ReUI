// Renders the island pictures for the site into src/assets/island, with a transparent
// background so a page can put them on any wallpaper. Settings' shot mode
// (`reui-settings --shots <dir> --stages`) draws each stage twice, on black and on white;
// a pixel's coverage is how little the backdrop shows through it, so the pair gives the
// island's exact colour and alpha (edges included). Runs Settings sandboxed
// (REUI_CONFIG_DIR in the temp folder): default settings, no real files touched.
//   npm run images
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import sharp from 'sharp';

const root = resolve(import.meta.dirname, '../..');
const out = resolve(import.meta.dirname, '../src/assets/island');

execFileSync('cargo', ['build', '-p', 'reui-settings', '--manifest-path', join(root, 'Cargo.toml')], { stdio: 'inherit' });
const exe = join(root, 'target', 'debug', 'reui-settings.exe');

const sandbox = mkdtempSync(join(tmpdir(), 'reui-site-'));
const shoot = (color) => {
  const dir = join(sandbox, color.slice(1));
  execFileSync(exe, ['--shots', dir, '--stages', '--backdrop-color', color], { env: { ...process.env, REUI_CONFIG_DIR: join(sandbox, 'config') } });
  return dir;
};
const [onBlack, onWhite] = [shoot('#000000'), shoot('#FFFFFF')];

const raw = async (path) => sharp(path).ensureAlpha().raw().toBuffer({ resolveWithObject: true });

mkdirSync(out, { recursive: true });
for (const name of readdirSync(onBlack).filter((f) => f.startsWith('stage-') && f.endsWith('.png'))) {
  const [b, w] = await Promise.all([raw(join(onBlack, name)), raw(join(onWhite, name))]);
  const { width, height } = b.info;
  // The stage dims its backdrop, so "nothing drawn here" on white is the bottom-left
  // pixel's value, not 255.
  const corner = (height - 1) * width * 4;
  const span = (c) => w.data[corner + c] - b.data[corner + c];
  const spans = [span(0), span(1), span(2)];
  const pixels = Buffer.alloc(width * height * 4);
  for (let i = 0; i < pixels.length; i += 4) {
    let shown = 0;
    for (let c = 0; c < 3; c++) shown += (w.data[i + c] - b.data[i + c]) / spans[c];
    const a = Math.min(1, Math.max(0, 1 - shown / 3));
    for (let c = 0; c < 3; c++) pixels[i + c] = a > 0 ? Math.min(255, Math.round(b.data[i + c] / a)) : 0;
    pixels[i + 3] = Math.round(a * 255);
  }
  await sharp(pixels, { raw: { width, height, channels: 4 } }).png({ compressionLevel: 9 }).toFile(join(out, name.replace(/^stage-/, '')));
  console.log(name.replace(/^stage-/, ''), `${width}×${height}`);
}
rmSync(sandbox, { recursive: true, force: true });
