// The repo's own documents, read at build time so the site never drifts from them:
// CHANGELOG.md (each release, newest first) and docs/guide.md (one page per section).
// They're Markdown written for GitHub; links to other files in the repo go to GitHub.
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { Marked } from 'marked';

const repo = 'https://github.com/ocopus-labs/ReUI';

/** The repo's root: the nearest folder up from where the build runs that has CHANGELOG.md. */
function root(): string {
  let dir = process.cwd();
  while (!existsSync(join(dir, 'CHANGELOG.md'))) {
    const up = dirname(dir);
    if (up === dir) throw new Error('CHANGELOG.md not found above ' + process.cwd());
    dir = up;
  }
  return dir;
}

function read(path: string): string {
  return readFileSync(join(root(), path), 'utf8').replace(/\r\n/g, '\n');
}

/** "The island" → "the-island". */
export function slug(text: string): string {
  return text.toLowerCase().replace(/&/g, 'and').replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
}

/** Markdown → HTML; `from` is the document's folder in the repo, for its relative links. */
function html(markdown: string, from: string): string {
  const marked = new Marked({ gfm: true, async: false });
  marked.use({
    walkTokens(token) {
      if (token.type === 'link' && !/^(https?:|mailto:|#|\/)/.test(token.href)) {
        token.href = `${repo}/blob/main/${from ? from + '/' : ''}${token.href}`;
      }
    },
    renderer: {
      // Headings inside a section get ids, so a part of a page can be linked to.
      heading({ tokens, depth }) {
        const text = this.parser.parseInline(tokens);
        const id = slug(text.replace(/<[^>]+>/g, ''));
        return `<h${depth} id="${id}">${text}</h${depth}>\n`;
      },
    },
  });
  return marked.parse(markdown) as string;
}

export interface Release {
  version: string;
  /** "30 October 2026", or "unreleased". */
  when: string;
  /** The entry's first line, for the banner and the list. */
  summary: string;
  html: string;
}

export function releases(): Release[] {
  const text = read('CHANGELOG.md');
  return text
    .split(/^## /m)
    .slice(1)
    .map((part) => {
      const [head, ...rest] = part.split('\n');
      const [, version = head.trim(), when = ''] = head.match(/^v?(\S+)\s*(?:\((.*)\))?/) ?? [];
      const body = rest.join('\n').trim();
      const first = body.split('\n\n')[0] ?? '';
      const summary = first.startsWith('-') ? '' : first.replace(/\s+/g, ' ').trim();
      return { version, when, summary, html: html(body, '') };
    });
}

export interface Section {
  title: string;
  slug: string;
  /** The section's first paragraph, plain, for the index. */
  lead: string;
  html: string;
}

export interface Guide {
  intro: string;
  sections: Section[];
}

export function guide(): Guide {
  const text = read('docs/guide.md');
  const [head, ...parts] = text.split(/^## /m);
  const intro = head.replace(/^# .*\n/, '').trim();
  const sections = parts.map((part) => {
    const [title, ...rest] = part.split('\n');
    const body = rest.join('\n').trim();
    const firstPara = body.split('\n\n').find((p) => !p.startsWith('|') && !p.startsWith('<')) ?? '';
    const lead = firstPara.replace(/\*\*|`/g, '').replace(/\[([^\]]+)\]\([^)]+\)/g, '$1').replace(/\s+/g, ' ').trim();
    return { title: title.trim(), slug: slug(title), lead, html: html(body, 'docs') };
  });
  return { intro: html(intro, 'docs'), sections };
}
