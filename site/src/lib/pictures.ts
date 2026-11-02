// The README's pictures (docs/images, taken at 2× from this site's island, the clipboard's
// from the app itself), shown on the guide's pages and on the changelog. Each keeps the
// README's width and alt text, so the two say the same.
import type { ImageMetadata } from 'astro';

const files = import.meta.glob<{ default: ImageMetadata }>('../../../docs/images/*.png', { eager: true });

export interface Picture {
  src: ImageMetadata;
  alt: string;
  /** Its width on the page, in CSS pixels (half the file's). */
  width: number;
}

function picture(name: string, width: number, alt: string): Picture {
  const file = files[`../../../docs/images/${name}.png`];
  if (!file) throw new Error(`docs/images/${name}.png is missing`);
  return { src: file.default, alt, width };
}

const island = () => picture('now-playing', 212, 'The pill with music playing: the album cover and the level bars');
const nowPlaying = () => picture('hero', 960, 'The island open at the top of the screen, playing Blinding Lights by The Weeknd in Spotify, with the view switcher under it');
const clipboard = () => picture('clipboard', 896, 'Clipboard history open: type filters, a search field, the count, Clear and settings, over a row of cards for three files, a Wikipedia link, a photo and a note');

/** The picture at the top of a guide page, by its slug. */
export function forGuide(slug: string): Picture | undefined {
  switch (slug) {
    case 'the-island': return island();
    case 'now-playing': return nowPlaying();
    case 'clipboard-history': return clipboard();
    case 'volume-and-brightness': return picture('volume', 296, 'The volume level in the island, at 56 %');
    case 'devices': return picture('devices', 516, 'Galaxy Buds2 Pro just connected, at 82 % battery');
    case 'timers': return picture('timers', 516, 'A tea timer counting down, 3:42 left');
    case 'system-stats': return picture('system-stats', 576, 'System stats open: CPU at 8 % and memory at 42 % as rings, then GPU, disk and network');
    case 'coding-agents': return picture('agents', 576, 'Claude Code asking permission to run npm test, with Deny, Always and Allow');
    default: return undefined;
  }
}

/** The pictures of what a release brought, by version. */
export function forRelease(version: string): Picture[] {
  switch (version) {
    case '0.2.0': return [clipboard()];
    case '0.1.0': return [nowPlaying(), picture('agents', 576, 'Claude Code asking permission to run npm test, with Deny, Always and Allow')];
    default: return [];
  }
}
