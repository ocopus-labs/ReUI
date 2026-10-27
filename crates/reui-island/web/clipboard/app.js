/* ReUI clipboard window: web UI. See docs/clipboard-window-contract.md.
   No framework, no network. Renders only from the latest `view`; local UI state
   (selection by id, scroll, menus) survives every state. */
(() => {
  'use strict';

  /* ---------------- Bridge ---------------- */

  const wv = window.chrome && window.chrome.webview;
  const send = (msg) => { try { if (wv) wv.postMessage(msg); } catch (_) { /* host gone */ } };
  const dev = window.ReuiDev || null;
  // Image ids are opaque decimal strings (64-bit); never convert them to numbers.
  const imgUrl = (id) => (id == null || id === '' ? '' : dev && dev.image ? dev.image(String(id)) : `https://reui.local/image/${encodeURIComponent(String(id))}`);

  const WIDTH = 760;
  const QUERY_DEBOUNCE = 60;
  const UNDO_MS = 5000;

  /* ---------------- Icons (16 grid, stroke) ---------------- */

  const sv = (body, vb = '0 0 16 16', extra = '') =>
    `<svg viewBox="${vb}" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"${extra}>${body}</svg>`;
  const I = {
    search: sv('<circle cx="7" cy="7" r="4.75"/><path d="M10.5 10.5 14 14"/>'),
    x: sv('<path d="M4 4l8 8M12 4l-8 8"/>', '0 0 16 16', ' stroke-width="2"'),
    chev: sv('<path d="M4 6l4 4 4-4"/>', '0 0 16 16', ' stroke-width="2"'),
    chevR: sv('<path d="M6 4l4 4-4 4"/>', '0 0 16 16', ' stroke-width="2"'),
    back: sv('<path d="M10 4L6 8l4 4"/>', '0 0 16 16', ' stroke-width="2"'),
    grid: sv('<rect x="2.25" y="2.25" width="4.75" height="4.75" rx="1.25"/><rect x="9" y="2.25" width="4.75" height="4.75" rx="1.25"/><rect x="2.25" y="9" width="4.75" height="4.75" rx="1.25"/><rect x="9" y="9" width="4.75" height="4.75" rx="1.25"/>'),
    strip: sv('<rect x="1.75" y="4" width="5.5" height="8" rx="1.5"/><rect x="8.75" y="4" width="5.5" height="8" rx="1.5"/>'),
    plus: sv('<path d="M8 3v10M3 8h10"/>', '0 0 16 16', ' stroke-width="2"'),
    check: sv('<path d="M3.5 8.5l3 3 6-7"/>', '0 0 16 16', ' stroke-width="2.25"'),
    starFill: '<svg viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 1.6l1.95 4 4.4.62-3.18 3.1.75 4.38L8 11.65 4.08 13.7l.75-4.38L1.65 6.22l4.4-.62z"/></svg>',
    star: sv('<path d="M8 1.9l1.85 3.8 4.2.6-3.03 2.95.72 4.18L8 11.45l-3.74 1.98.72-4.18L1.95 6.3l4.2-.6z"/>'),
    globe: sv('<circle cx="8" cy="8" r="6"/><path d="M2 8h12M8 2c1.7 1.7 2.5 3.7 2.5 6S9.7 12.3 8 14C6.3 12.3 5.5 10.3 5.5 8S6.3 3.7 8 2z"/>'),
    text: sv('<path d="M3 4h10M3 8h10M3 12h6"/>'),
    rich: sv('<path d="M3.5 12.5L6.5 3.5l3 9M4.5 9.5h4M11 7.5v5M11 7.5c.4-.7 1-1 1.6-1"/>'),
    code: sv('<path d="M5.5 4.5L2 8l3.5 3.5M10.5 4.5L14 8l-3.5 3.5"/>'),
    link: sv('<path d="M7 9a2.5 2.5 0 003.5 0l2.25-2.25a2.5 2.5 0 00-3.5-3.5L8.5 4M9 7a2.5 2.5 0 00-3.5 0L3.25 9.25a2.5 2.5 0 003.5 3.5L7.5 12"/>'),
    image: sv('<rect x="2" y="2.75" width="12" height="10.5" rx="2"/><circle cx="5.75" cy="6.25" r="1.25"/><path d="M2.5 12l3.75-3.5 2.5 2.25L11 8.5l2.5 2.5"/>'),
    color: sv('<path d="M8 2.2c2.2 2.6 4.25 4.9 4.25 7.3a4.25 4.25 0 01-8.5 0C3.75 7.1 5.8 4.8 8 2.2z"/>'),
    files: sv('<path d="M4 1.75h5l3 3v9.5H4z"/><path d="M9 1.75v3h3"/>'),
    doc: '<svg viewBox="0 0 30 36" aria-hidden="true"><path d="M4 1.5h15l9 9V32a2.5 2.5 0 01-2.5 2.5h-21A2.5 2.5 0 012 32V4A2.5 2.5 0 014.5 1.5z" fill="rgb(118 118 128 / .32)"/><path d="M19 1.5V8a2.5 2.5 0 002.5 2.5H28" fill="rgb(255 255 255 / .16)"/></svg>',
    folder: '<svg viewBox="0 0 30 36" aria-hidden="true"><path d="M2 9.5A2.5 2.5 0 014.5 7h7l3 3h11a2.5 2.5 0 012.5 2.5V28a2.5 2.5 0 01-2.5 2.5h-21A2.5 2.5 0 012 28z" fill="#5AA9F5"/><path d="M2 13.5h26V28a2.5 2.5 0 01-2.5 2.5h-21A2.5 2.5 0 012 28z" fill="#7DBDFA"/></svg>',
    fileSm: sv('<path d="M3.5 1.75h5l3 3v9.5h-8z"/><path d="M8.5 1.75v3h3"/>'),
    paste: sv('<rect x="3" y="2.75" width="10" height="11.5" rx="2"/><path d="M6 2.75h4v2H6z"/>'),
    plain: sv('<path d="M3 4h10M8 4v9"/>'),
    copy: sv('<rect x="5" y="5" width="8.5" height="8.5" rx="2"/><path d="M11 5V3.5A1.5 1.5 0 009.5 2h-6A1.5 1.5 0 002 3.5v6A1.5 1.5 0 003.5 11H5"/>'),
    stack: sv('<rect x="2.5" y="6" width="11" height="7.5" rx="2"/><path d="M4.5 3.75h7M6 1.75h4"/>'),
    edit: sv('<path d="M10.5 2.75l2.75 2.75L6 12.75l-3.5.75.75-3.5z"/>'),
    folderSm: sv('<path d="M2 4.25A1.25 1.25 0 013.25 3h3l1.5 1.5h5A1.25 1.25 0 0114 5.75v6A1.25 1.25 0 0112.75 13h-9.5A1.25 1.25 0 012 11.75z"/>'),
    remove: sv('<path d="M2 4.25A1.25 1.25 0 013.25 3h3l1.5 1.5h5A1.25 1.25 0 0114 5.75v6A1.25 1.25 0 0112.75 13h-9.5A1.25 1.25 0 012 11.75zM6 8.75h4"/>'),
    ocr: sv('<path d="M2 5V3.25C2 2.56 2.56 2 3.25 2H5M11 2h1.75c.69 0 1.25.56 1.25 1.25V5M14 11v1.75c0 .69-.56 1.25-1.25 1.25H11M5 14H3.25C2.56 14 2 13.44 2 12.75V11M5 6h6M8 6v5"/>'),
    trash: sv('<path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.4c.05.6.55 1.1 1.15 1.1h3.5c.6 0 1.1-.5 1.15-1.1l.6-8.4"/>'),
    pause: sv('<path d="M6 3.5v9M10 3.5v9"/>', '0 0 16 16', ' stroke-width="2"'),
    play: '<svg viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M5 3.2v9.6c0 .5.55.8.97.53l7.2-4.8a.63.63 0 000-1.06l-7.2-4.8A.63.63 0 005 3.2z"/></svg>',
    case: sv('<path d="M2 12l3-8 3 8M3 9.5h4M10 7.5c.5-.6 1.1-.9 1.9-.9 1.2 0 1.85.7 1.85 1.8V12M13.75 9.6c-2.7 0-3.9.5-3.9 1.4 0 .65.55 1.1 1.4 1.1 1.3 0 2.5-.9 2.5-2.5"/>'),
    trim: sv('<path d="M2 4h12M4 8h8M6 12h4"/>'),
    line: sv('<path d="M2 8h12M11 5l3 3-3 3"/>'),
    braces: sv('<path d="M6 2.5c-1.5 0-2 .6-2 2v1.5c0 1-.5 2-1.5 2 1 0 1.5 1 1.5 2v1.5c0 1.4.5 2 2 2M10 2.5c1.5 0 2 .6 2 2v1.5c0 1 .5 2 1.5 2-1 0-1.5 1-1.5 2v1.5c0 1.4-.5 2-2 2"/>'),
    md: sv('<rect x="1.75" y="3.5" width="12.5" height="9" rx="2"/><path d="M4.25 10.25v-4.5l1.75 2 1.75-2v4.5M11 5.75v4.5M9.5 8.75l1.5 1.5 1.5-1.5"/>'),
    rename: sv('<path d="M2.5 12.5h3l7-7-3-3-7 7zM11 2.5l2.5 2.5"/>'),
    palette: sv('<path d="M8 2a6 6 0 100 12c.9 0 1.25-.6 1.25-1.2 0-.9-.75-1.1-.75-1.9 0-.7.55-1.15 1.25-1.15H11.5A2.5 2.5 0 0014 7.25C14 4.3 11.3 2 8 2z"/><circle cx="5" cy="7.5" r=".6" fill="currentColor"/><circle cx="7.5" cy="5" r=".6" fill="currentColor"/><circle cx="10.5" cy="5.5" r=".6" fill="currentColor"/>'),
  };

  const KIND = { text: 'Text', code: 'Code', link: 'Link', image: 'Image', color: 'Color', files: 'Files' };
  const TYPE_ICON = {
    text: I.text, code: I.code, link: I.link, links: I.link, image: I.image, images: I.image,
    color: I.color, colors: I.color, files: I.files,
  };
  const SWATCHES = [
    ['Red', '#FF453A'], ['Orange', '#FF9F0A'], ['Yellow', '#FFD60A'], ['Green', '#30D158'],
    ['Teal', '#40C8E0'], ['Blue', '#0A84FF'], ['Indigo', '#5E5CE6'], ['Purple', '#BF5AF2'], ['Pink', '#FF375F'],
  ];
  const TRANSFORMS = [
    ['upper', 'Paste as UPPERCASE', I.case], ['lower', 'Paste as lowercase', I.case],
    ['title', 'Paste as Title Case', I.case], ['trim', 'Paste trimmed', I.trim],
    ['single-line', 'Paste as single line', I.line], ['json', 'Paste as JSON', I.braces],
    ['markdown', 'Paste as Markdown', I.md],
  ];

  /* ---------------- Utilities ---------------- */

  const $ = (s, r = document) => r.querySelector(s);
  const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ESC[c]);
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const reduced = () => !!(window.ReuiDev && window.ReuiDev.still) || window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const now = () => performance.now();

  function hexRgb(hex) {
    let h = String(hex || '').replace('#', '').trim();
    if (h.length === 3 || h.length === 4) h = h.split('').map((c) => c + c).join('');
    const n = parseInt(h.slice(0, 6), 16);
    if (Number.isNaN(n)) return null;
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  function luminance([r, g, b]) {
    const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); };
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  }
  const inkFor = (hex) => { const rgb = hexRgb(hex); return rgb && luminance(rgb) > 0.36 ? 'ink-dark' : 'ink-light'; };
  function hsl([r, g, b]) {
    r /= 255; g /= 255; b /= 255;
    const mx = Math.max(r, g, b), mn = Math.min(r, g, b), l = (mx + mn) / 2;
    let h = 0, s = 0;
    if (mx !== mn) {
      const d = mx - mn;
      s = l > 0.5 ? d / (2 - mx - mn) : d / (mx + mn);
      h = mx === r ? (g - b) / d + (g < b ? 6 : 0) : mx === g ? (b - r) / d + 2 : (r - g) / d + 4;
      h *= 60;
    }
    return [Math.round(h), Math.round(s * 100), Math.round(l * 100)];
  }

  const AGE_UNITS = { s: 'second', m: 'minute', h: 'hour', d: 'day', w: 'week', mo: 'month', y: 'year' };
  function ageLong(age) {
    const m = /^(\d+)\s*(mo|s|m|h|d|w|y)$/.exec(String(age || '').trim());
    if (!m) return age === 'now' ? 'just now' : age || '';
    const n = +m[1];
    return `${n} ${AGE_UNITS[m[2]]}${n === 1 ? '' : 's'} ago`;
  }

  function splitText(t) {
    const lines = String(t || '').replace(/\r\n?/g, '\n').split('\n');
    let i = 0;
    while (i < lines.length && !lines[i].trim()) i++;
    const first = (lines[i] || '').trim();
    const rest = lines.slice(i + 1).join('\n').replace(/\n{3,}/g, '\n\n').replace(/^\s*\n/, '').trimEnd();
    return [first, rest];
  }

  function dedent(t, max) {
    let lines = String(t || '').replace(/\r\n?/g, '\n').replace(/\t/g, '  ').split('\n');
    while (lines.length && !lines[0].trim()) lines.shift();
    if (max) lines = lines.slice(0, max);
    let min = Infinity;
    for (const l of lines) if (l.trim()) min = Math.min(min, l.length - l.trimStart().length);
    if (!Number.isFinite(min)) min = 0;
    return lines.map((l) => l.slice(min)).join('\n');
  }

  // Tiny highlighter: comments, strings, numbers, keywords. Light tint only.
  const HL = /(\/\/[^\n]*|\/\*[\s\S]*?\*\/|(?<=^|\s)#\s[^\n]*|<!--[\s\S]*?-->)|("(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|`(?:\\.|[^`\\])*`)|\b(\d+(?:\.\d+)?)\b|\b(fn|let|const|var|function|return|if|else|elif|for|while|loop|break|continue|import|from|export|default|class|def|pub|struct|enum|impl|trait|use|mod|async|await|new|true|false|null|undefined|None|True|False|self|this|type|interface|match|in|of|mut|ref|static|public|private|protected|void|int|bool|string|try|catch|finally|throw|switch|case|package|func|go|select|SELECT|FROM|WHERE|JOIN|INSERT|UPDATE|DELETE|AND|OR|NOT)\b/g;
  function hl(src, limit = 20000) {
    const s = String(src || '');
    const head = s.length > limit ? s.slice(0, limit) : s;
    let out = '', last = 0, m;
    HL.lastIndex = 0;
    while ((m = HL.exec(head))) {
      if (m.index === HL.lastIndex && m[0] === '') { HL.lastIndex++; continue; }
      out += esc(head.slice(last, m.index));
      const cls = m[1] ? 'hc' : m[2] ? 'hs' : m[3] ? 'hn' : 'hk';
      out += `<span class="${cls}">${esc(m[0])}</span>`;
      last = m.index + m[0].length;
    }
    out += esc(head.slice(last));
    if (head.length < s.length) out += esc(s.slice(limit));
    return out;
  }

  function fileExt(name) {
    const m = /\.([A-Za-z0-9]{1,5})$/.exec(String(name || ''));
    return m ? m[1].toUpperCase() : '';
  }
  const fileIcon = (name) => {
    const ext = fileExt(name);
    return ext ? `${I.doc}<span class="ext">${esc(ext)}</span>` : I.folder;
  };

  function hostPath(url) {
    try {
      const u = new URL(url);
      const path = (u.pathname + u.search).replace(/\/$/, '');
      return [u.host.replace(/^www\./, ''), path && path !== '/' ? decodeURIComponent(path) : ''];
    } catch (_) {
      return [String(url || ''), ''];
    }
  }

  /* ---------------- State ---------------- */

  const S = {
    view: null,
    q: { query: '', collection: 'history', filter: { type: null, app: null } },
    layout: 'strip',
    sel: [],          // ordered selection (ids)
    cur: null,        // cursor id
    focus: 'search',  // 'search' | 'cards'
    lastLocal: 0,     // last time the page changed the query/layout itself
    previews: new Map(),
    previewAsked: new Map(),
    overlay: null,    // null | 'preview' | 'edit'
    editId: null,
    menu: null,
    tabEdit: null,    // { id|null, el }
    tabDrag: null,
    drag: null,
    undoUntil: 0,
    undoTimer: 0,
    stackTotal: 0,
    lastH: 0,
    cols: 5,
  };
  const cardEls = new Map(); // id -> { el, sig }

  /* ---------------- Skeleton ---------------- */

  const root = document.getElementById('root') || document.body;
  root.innerHTML = `
<div class="win" id="win">
  <header class="top">
    <div class="search" id="searchBox">
      <span class="mag" aria-hidden="true">${I.search}</span>
      <input id="q" type="text" placeholder="Search" autocomplete="off" spellcheck="false" role="combobox"
        aria-label="Search clips" aria-controls="cards" aria-expanded="true" aria-autocomplete="list">
      <button class="clear" id="clear" type="button" tabindex="-1" aria-label="Clear search" hidden>${I.x}</button>
    </div>
    <div class="status" id="status" role="status"></div>
    <button class="chip" id="typeChip" type="button" tabindex="-1" aria-haspopup="menu" aria-expanded="false"></button>
    <button class="chip" id="appChip" type="button" tabindex="-1" aria-haspopup="menu" aria-expanded="false"></button>
    <button class="round" id="layoutBtn" type="button" tabindex="-1"></button>
  </header>
  <div class="tabs" id="tabs" role="tablist" aria-label="Collections"></div>
  <div class="stage" id="stage">
    <div class="cards strip" id="cards" role="listbox" aria-label="Clips" aria-multiselectable="true" aria-orientation="horizontal" tabindex="-1"></div>
    <p class="empty" id="empty" hidden></p>
  </div>
  <section class="preview none" id="preview" aria-label="Preview"></section>
  <div class="overlay" id="overlay" hidden></div>
  <div class="pop" id="pop" hidden></div>
  <div class="sr" id="live" aria-live="polite"></div>
</div>`;

  const $win = $('#win'), $q = $('#q'), $clear = $('#clear'), $status = $('#status');
  const $typeChip = $('#typeChip'), $appChip = $('#appChip'), $layoutBtn = $('#layoutBtn');
  const $tabs = $('#tabs'), $cards = $('#cards'), $empty = $('#empty');
  const $preview = $('#preview'), $overlay = $('#overlay'), $pop = $('#pop'), $live = $('#live');

  /* ---------------- View helpers ---------------- */

  const items = () => (S.view && S.view.items) || [];
  const byId = (id) => items().find((i) => i.id === id) || null;
  const indexOf = (id) => items().findIndex((i) => i.id === id);
  const curItem = () => byId(S.cur);
  const selIds = () => {
    const ids = S.sel.filter((id) => byId(id));
    return ids.length ? ids : (S.cur != null ? [S.cur] : []);
  };
  const collections = () => (S.view && S.view.collections) || [];
  const collection = (id) => collections().find((c) => c.id === id) || null;

  /* ---------------- Messages in ---------------- */

  if (wv) {
    wv.addEventListener('message', (e) => {
      let m = e.data;
      if (typeof m === 'string') { try { m = JSON.parse(m); } catch (_) { return; } }
      if (m && typeof m === 'object') onMessage(m);
    });
  }

  function onMessage(m) {
    switch (m.type) {
      case 'open': adopt(m.view, true); shown(); break;
      case 'state': adopt(m.view, false); break;
      case 'preview': onPreview(m); break;
      case 'host.shown': shown(); break;
      case 'host.hidden': case 'close': hidden(); break;
      default: break;
    }
  }

  function adopt(view, reset) {
    if (!view) return;
    const prev = S.view;
    S.view = view;
    const f = view.filter || {};
    const ctxChanged = reset || !prev ||
      prev.collection !== view.collection || prev.query !== view.query ||
      (prev.filter || {}).type !== f.type || (prev.filter || {}).app !== f.app;

    const settled = now() - S.lastLocal > 300;
    if (reset || !prev) {
      S.q = { query: view.query || '', collection: view.collection || 'history', filter: { type: f.type || null, app: f.app || null } };
      if ($q.value !== S.q.query) $q.value = S.q.query;
    } else if (settled) {
      S.q.collection = view.collection || 'history';
      S.q.filter = { type: f.type || null, app: f.app || null };
    }
    if (reset || !prev || settled) S.layout = view.layout === 'grid' ? 'grid' : 'strip';

    // Selection by id (W-10: new clips slide in, selection stays).
    const list = items();
    if (ctxChanged) {
      S.cur = list.length ? list[0].id : null;
      S.sel = S.cur != null ? [S.cur] : [];
      $cards.scrollLeft = 0; $cards.scrollTop = 0; scroller.stop();
    } else {
      const prevIdx = prev ? prev.items.findIndex((i) => i.id === S.cur) : -1;
      S.sel = S.sel.filter((id) => list.some((i) => i.id === id));
      if (!list.some((i) => i.id === S.cur)) {
        S.cur = list.length ? list[clamp(prevIdx, 0, list.length - 1)].id : null;
      }
      if (!S.sel.length && S.cur != null) S.sel = [S.cur];
    }

    // Undo shows for 5 s from when it appears.
    if (view.undo && !(prev && prev.undo)) {
      S.undoUntil = now() + UNDO_MS;
      clearTimeout(S.undoTimer);
      S.undoTimer = setTimeout(renderTop, UNDO_MS + 20);
    } else if (!view.undo) S.undoUntil = 0;

    if (!view.stack || !view.stack.active) S.stackTotal = 0;
    else if (!S.stackTotal) S.stackTotal = view.stack.remaining || 0;

    if (S.editId != null && !byId(S.editId)) closeOverlay();

    render(!ctxChanged && !!prev);
    if (ctxChanged) $live.textContent = view.empty || '';
  }

  /* ---------------- Messages out ---------------- */

  let queryTimer = 0;
  function sendQuery(immediate) {
    S.lastLocal = now();
    clearTimeout(queryTimer);
    const go = () => send({ type: 'query', query: S.q.query, collection: S.q.collection, filter: { type: S.q.filter.type, app: S.q.filter.app } });
    if (immediate) go(); else queryTimer = setTimeout(go, QUERY_DEBOUNCE);
  }

  function setCollection(id) {
    if (!id || id === S.q.collection) return;
    S.q.collection = id;
    renderTabs(true);
    sendQuery(true);
  }
  function setFilter(key, val) {
    S.q.filter[key] = val;
    renderTop();
    sendQuery(true);
  }
  function setLayout(layout) {
    S.layout = layout;
    S.lastLocal = now();
    send({ type: 'layout', layout });
    applyLayout();
    postSize();
    revealCur(false);
  }

  const paste = (ids, mode = 'normal') => ids.length && send({ type: 'paste', ids, mode });
  const copy = (ids, mode = 'normal') => ids.length && send({ type: 'copy', ids, mode });
  function stack(ids) {
    if (!ids.length) return;
    S.stackTotal = ids.length;
    send({ type: 'stack', ids });
  }
  function togglePin(ids) {
    const list = ids.map(byId).filter(Boolean);
    const allPinned = list.length && list.every((i) => i.pinned);
    for (const it of list) send({ type: allPinned ? 'unpin' : 'pin', id: it.id });
  }
  const del = (ids) => ids.length && send({ type: 'delete', ids });
  function undo() { send({ type: 'undo' }); }
  function requestPreview(id) {
    if (id == null || S.previews.has(id)) return;
    const t = S.previewAsked.get(id);
    if (t && now() - t < 1500) return;
    S.previewAsked.set(id, now());
    send({ type: 'preview', id });
  }
  let previewTimer = 0;
  function schedulePreview() {
    clearTimeout(previewTimer);
    const id = S.cur;
    previewTimer = setTimeout(() => requestPreview(id), 70);
  }
  function onPreview(m) {
    S.previews.set(m.id, m);
    if (S.previews.size > 40) S.previews.delete(S.previews.keys().next().value);
    if (m.id === S.cur) { renderPreview(true); if (S.overlay === 'preview') renderOverlay(); }
    if (S.overlay === 'edit' && S.editId === m.id && S.editPristine && typeof m.text === 'string') {
      const ta = $('textarea', $overlay);
      if (ta) { ta.value = m.text; }
    }
  }

  /* ---------------- Lifecycle ---------------- */

  function shown() {
    closeMenu();
    S.focus = 'search';
    $q.focus({ preventScroll: true });
    $q.select();
    applyFocus();
    if (!reduced()) {
      $win.animate([{ opacity: 0, transform: 'scale(.985)' }, { opacity: 1, transform: 'none' }],
        { duration: 160, easing: 'cubic-bezier(.2,.8,.2,1)' });
    }
    postSize(true);
  }
  function hidden() {
    closeMenu();
    if (S.tabEdit) endTabEdit(false);
    closeOverlay();
    endCardDrag(true);
    $win.classList.remove('ctrl');
  }

  /* ---------------- Rendering ---------------- */

  function render(animate) {
    applyLayout();
    renderTop();
    renderTabs();
    renderCards(animate);
    renderEmpty();
    updateSel(false);
    renderPreview();
    if (S.overlay === 'preview') renderOverlay();
    if (S.menu && S.menu.live) refreshMenu();
    postSize();
  }

  function applyLayout() {
    const grid = S.layout === 'grid';
    $win.classList.toggle('grid', grid);
    $cards.classList.toggle('grid', grid);
    $cards.classList.toggle('strip', !grid);
    $cards.setAttribute('aria-orientation', grid ? 'vertical' : 'horizontal');
    const icon = grid ? I.strip : I.grid;
    if ($layoutBtn._icon !== icon) { $layoutBtn.innerHTML = icon; $layoutBtn._icon = icon; }
    $layoutBtn.setAttribute('aria-label', grid ? 'Strip' : 'Grid');
    $layoutBtn.title = grid ? 'Strip' : 'Grid';
    requestAnimationFrame(updateFades);
  }

  // Content height comes from fixed component heights, so one read after a render is enough.
  function postSize(force) {
    const h = Math.ceil($win.offsetHeight);
    if (!h || (!force && h === S.lastH)) return;
    S.lastH = h;
    send({ type: 'host.resize', w: WIDTH, h });
  }

  function setHTML(el, html) { if (el._html !== html) { el.innerHTML = html; el._html = html; } }

  function renderTop() {
    const v = S.view || {};
    $clear.hidden = !$q.value;

    let st = '';
    if (v.stack && v.stack.active) {
      const total = Math.max(S.stackTotal, v.stack.remaining || 0);
      const n = total ? clamp(total - (v.stack.remaining || 0) + 1, 1, total) : 0;
      st += `<span class="pill accent" aria-label="Paste stack ${n} of ${total}">${I.stack.replace('<svg', '<svg width="14" height="14"')} Stack ${total ? `${n}/${total}` : ''}</span>`;
    }
    if (v.undo && now() < S.undoUntil) st += `<button class="pill" type="button" tabindex="-1" data-act="undo">Undo</button>`;
    if (v.paused) st += `<button class="pill quiet" type="button" tabindex="-1" data-act="resume" aria-label="Paused. Resume">${'<span class="pd"></span>'}Paused</button>`;
    setHTML($status, st);

    const t = S.q.filter.type;
    const tInfo = t ? ((v.types || []).find((x) => x.id === t) || { id: t, label: t[0].toUpperCase() + t.slice(1) }) : null;
    setHTML($typeChip, tInfo
      ? `<span class="cg">${TYPE_ICON[t] || I.text}</span><span class="cl">${esc(tInfo.label)}</span><span class="cv">${I.chev}</span>`
      : `<span class="cl">Type</span><span class="cv">${I.chev}</span>`);
    $typeChip.classList.toggle('on', !!t);
    $typeChip.setAttribute('aria-label', tInfo ? `Type: ${tInfo.label}` : 'Type');

    const a = S.q.filter.app;
    const aInfo = a ? ((v.apps || []).find((x) => x.id === a) || { id: a, name: a.replace(/\.exe$/i, '') }) : null;
    setHTML($appChip, aInfo
      ? `${aInfo.icon != null ? `<img class="ci" src="${esc(imgUrl(aInfo.icon))}" alt="" decoding="async">` : ''}<span class="cl">${esc(aInfo.name)}</span><span class="cv">${I.chev}</span>`
      : `<span class="cl">App</span><span class="cv">${I.chev}</span>`);
    $appChip.classList.toggle('on', !!a);
    $appChip.setAttribute('aria-label', aInfo ? `App: ${aInfo.name}` : 'App');
  }

  function renderTabs(force) {
    if (S.tabEdit || S.tabDrag) return;
    const cols = collections();
    const sig = JSON.stringify([cols, S.q.collection]);
    if (!force && $tabs._sig === sig) return;
    $tabs._sig = sig;
    $tabs.innerHTML = cols.map((c) => tabHTML(c, c.id === S.q.collection)).join('') +
      `<button class="tab add" type="button" tabindex="-1" data-add="1" aria-label="New collection" title="New collection">${I.plus}</button>`;
    const sel = $('.tab[aria-selected="true"]', $tabs);
    if (sel) {
      const l = sel.offsetLeft, r = l + sel.offsetWidth;
      if (l < $tabs.scrollLeft + 16) $tabs.scrollLeft = l - 16;
      else if (r > $tabs.scrollLeft + $tabs.clientWidth - 16) $tabs.scrollLeft = r - $tabs.clientWidth + 16;
    }
  }
  function tabHTML(c, selected) {
    const dot = c.color ? `<span class="dot" style="--c:${esc(c.color)}"></span>` : '';
    return `<button class="tab" type="button" role="tab" tabindex="-1" aria-selected="${selected}" data-id="${esc(c.id)}"${c.builtin ? ' data-builtin="1"' : ''} aria-label="${esc(c.name)}, ${c.count}">${dot}<span class="nm">${esc(c.name)}</span><span class="n">${esc(fmtCount(c.count))}</span></button>`;
  }
  const fmtCount = (n) => (n == null ? '' : n >= 10000 ? `${Math.floor(n / 1000)}k` : String(n));

  /* ----- Cards ----- */

  function cardSig(it) {
    return JSON.stringify([it.kind, it.text, it.rich, it.title, it.site, it.url, it.image, it.icon, it.app, it.age,
      it.meta, it.color, it.files, it.pinned, it.missing, it.cut, it.lines]);
  }

  function footHTML(it, cls) {
    const icon = it.icon != null ? `<img class="appi" src="${esc(imgUrl(it.icon))}" alt="" decoding="async">` : '';
    return `<div class="foot${cls ? ' ' + cls : ''}"><span class="qk"></span>${icon}<span class="age">${esc(it.age || '')}</span><span class="meta">${esc(it.meta || '')}</span>${it.pinned ? `<span class="pin">${I.starFill}</span>` : ''}</div>`;
  }
  const lazyImg = (id, cls = '') => `<img${cls ? ` class="${cls}"` : ''} src="${esc(imgUrl(id))}" alt="" loading="lazy" decoding="async" draggable="false">`;

  function cardHTML(it) {
    const chk = `<span class="chk">${I.check}</span>`;
    switch (it.kind) {
      case 'image':
        return `<div class="media">${it.image != null ? lazyImg(it.image) : `<span class="ph">${I.image}</span>`}</div>${footHTML(it, 'on-media')}${chk}`;
      case 'link': {
        const [host, path] = hostPath(it.url);
        if (it.image != null) {
          return `<div class="lk-img">${lazyImg(it.image)}</div><p class="lk-t">${esc(it.title || host + path)}</p>${footHTML(it)}${chk}`;
        }
        const hero = it.title || host;
        const detail = it.title ? host + path : path;
        return `<div class="lk-plain"><span class="gl">${I.globe}</span><p class="lh">${esc(hero)}</p>${detail ? `<p class="lp">${esc(detail)}</p>` : ''}</div>${footHTML(it)}${chk}`;
      }
      case 'color': {
        const ink = inkFor(it.color);
        return `<div class="sw ${ink}" style="--c:${esc(it.color)}"><span class="hex">${esc(String(it.color || '').toUpperCase())}</span></div>${footHTML(it, ink)}${chk}`;
      }
      case 'files': {
        const f = it.files || [];
        return `<div class="fl"><span class="fico">${fileIcon(f[0])}</span><p class="fn">${esc(f[0] || '')}</p>${f.length > 1 ? `<p class="fm">+${f.length - 1}</p>` : ''}</div>${footHTML(it)}${chk}`;
      }
      case 'code':
        return `<pre class="cd">${hl(dedent(it.text, 9), 1200)}</pre><span class="badge" aria-hidden="true">${I.code}</span>${footHTML(it)}${chk}`;
      default: {
        const [first, rest] = splitText(it.text);
        const longFirst = first.length > 16; // about one line of 13 px semibold in a card
        return `<div class="tx"><p class="t1${rest ? '' : ' solo'}">${esc(first)}</p>${rest ? `<p class="t2${longFirst ? '' : ' more'}">${esc(rest.slice(0, 600))}</p>` : ''}</div><span class="badge" aria-hidden="true">${it.rich ? I.rich : I.text}</span>${footHTML(it)}${chk}`;
      }
    }
  }

  function cardLabel(it) {
    const parts = [KIND[it.kind] || 'Clip'];
    if (it.kind === 'text' || it.kind === 'code') parts.push(splitText(it.text)[0].slice(0, 80));
    else if (it.kind === 'link') parts.push(it.title || it.url || '');
    else if (it.kind === 'color') parts.push(it.color || '');
    else if (it.kind === 'files') parts.push((it.files || []).join(', ').slice(0, 120));
    if (it.meta && it.meta !== it.color) parts.push(it.meta);
    if (it.app) parts.push(it.app);
    if (it.age) parts.push(ageLong(it.age));
    if (it.pinned) parts.push('pinned');
    if (it.missing) parts.push('missing');
    return parts.filter(Boolean).join(', ');
  }

  function buildCard(it) {
    const el = document.createElement('div');
    el.className = `card k-${it.kind}${it.kind === 'link' && it.image != null ? ' has-img' : ''}${it.missing ? ' missing' : ''}`;
    el.id = `clip-${it.id}`;
    el.dataset.id = it.id;
    el.setAttribute('role', 'option');
    el.setAttribute('aria-selected', 'false');
    el.setAttribute('aria-label', cardLabel(it));
    el.innerHTML = cardHTML(it);
    el._qk = el.querySelector('.qk');
    for (const img of el.querySelectorAll('img')) img.addEventListener('error', () => img.classList.add('broken'), { once: true });
    return el;
  }

  function renderCards(animate) {
    const list = items();
    const doFlip = animate && !reduced() && list.length && cardEls.size;
    const first = new Map();
    if (doFlip) {
      for (const [id, rec] of cardEls) if (rec.el.isConnected) first.set(id, rec.el.getBoundingClientRect());
    }
    const seen = new Set();
    const fresh = [];
    let prevEl = null;
    list.forEach((it, i) => {
      seen.add(it.id);
      const sig = cardSig(it);
      let rec = cardEls.get(it.id);
      if (!rec) {
        rec = { el: buildCard(it), sig };
        cardEls.set(it.id, rec);
        fresh.push(rec.el);
      } else if (rec.sig !== sig) {
        const el = buildCard(it);
        if (rec.el.parentNode) rec.el.replaceWith(el);
        if (S.previews.has(it.id) && rec.sigText !== it.text) S.previews.delete(it.id);
        rec.el = el; rec.sig = sig;
      }
      rec.sigText = it.text;
      const want = prevEl ? prevEl.nextSibling : $cards.firstChild;
      if (rec.el !== want) $cards.insertBefore(rec.el, want);
      const qk = i < 9 ? String(i + 1) : '';
      if (rec.el._qk && rec.el._qk.textContent !== qk) rec.el._qk.textContent = qk;
      prevEl = rec.el;
    });
    for (const [id, rec] of cardEls) {
      if (!seen.has(id)) { rec.el.remove(); cardEls.delete(id); }
    }
    if (doFlip) {
      const ease = 'cubic-bezier(.2,.8,.2,1)';
      for (const [id, r0] of first) {
        const rec = cardEls.get(id);
        if (!rec) continue;
        const r1 = rec.el.getBoundingClientRect();
        const dx = r0.left - r1.left, dy = r0.top - r1.top;
        if (Math.abs(dx) > 0.5 || Math.abs(dy) > 0.5) {
          rec.el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], { duration: 240, easing: ease });
        }
      }
      for (const el of fresh) {
        el.animate([{ opacity: 0, transform: 'translateX(-14px) scale(.94)' }, { opacity: 1, transform: 'none' }],
          { duration: 260, easing: ease });
      }
    }
    measureCols();
    updateFades();
  }

  function measureCols() {
    if (S.layout !== 'grid') { S.cols = 1; return; }
    const els = $cards.children;
    if (!els.length) return;
    const top = els[0].offsetTop;
    let n = 0;
    while (n < els.length && els[n].offsetTop === top) n++;
    S.cols = n || 5;
  }

  function renderEmpty() {
    const v = S.view;
    const text = v && v.empty && !items().length ? v.empty : '';
    $empty.hidden = !text;
    if ($empty.textContent !== text) $empty.textContent = text;
  }

  function updateSel(reveal = true) {
    const set = new Set(S.sel);
    const multi = set.size > 1;
    $cards.classList.toggle('multi', multi);
    for (const [id, rec] of cardEls) {
      const isCur = id === S.cur, picked = set.has(id);
      rec.el.classList.toggle('cur', isCur);
      rec.el.classList.toggle('pick', picked && multi);
      const aria = String(picked || isCur);
      if (rec.el.getAttribute('aria-selected') !== aria) rec.el.setAttribute('aria-selected', aria);
    }
    const active = S.cur != null ? `clip-${S.cur}` : '';
    for (const el of [$cards, $q]) {
      if (active) el.setAttribute('aria-activedescendant', active); else el.removeAttribute('aria-activedescendant');
    }
    if (reveal) revealCur(true);
    renderPreview();
    if (S.overlay === 'preview') renderOverlay();
  }

  function setCursor(id, { extend = false, toggle = false, keep = false } = {}) {
    if (id == null) return;
    if (toggle) {
      const i = S.sel.indexOf(id);
      if (i >= 0 && S.sel.length > 1) S.sel.splice(i, 1);
      else if (i < 0) S.sel.push(id);
      S.cur = id;
    } else if (extend) {
      if (!S.sel.includes(S.cur) && S.cur != null) S.sel.push(S.cur);
      if (!S.sel.includes(id)) S.sel.push(id);
      S.cur = id;
    } else {
      S.cur = id;
      if (!keep) S.sel = [id];
    }
    updateSel(true);
  }

  function move(delta, opts) {
    const list = items();
    if (!list.length) return;
    let i = indexOf(S.cur);
    i = i < 0 ? 0 : clamp(i + delta, 0, list.length - 1);
    if (opts && opts.extend && delta) {
      // Shift+arrow: extend (or shrink back) the run of selected cards.
      const next = list[i].id;
      if (S.sel.includes(next) && next !== S.cur && S.sel.length > 1) {
        S.sel = S.sel.filter((x) => x !== S.cur);
        S.cur = next;
        updateSel(true);
        return;
      }
    }
    setCursor(list[i].id, opts);
  }

  /* ----- Scrolling: eased, timer stops when settled ----- */

  const scroller = (() => {
    let raf = 0, target = 0, axis = 'x', last = 0;
    const get = () => (axis === 'x' ? $cards.scrollLeft : $cards.scrollTop);
    const set = (v) => { if (axis === 'x') $cards.scrollLeft = v; else $cards.scrollTop = v; };
    function step(t) {
      const dt = Math.min(48, t - last || 16); last = t;
      const curv = get();
      const next = curv + (target - curv) * (1 - Math.exp(-dt / 45));
      if (Math.abs(target - next) < 0.6) { set(target); raf = 0; return; }
      set(next);
      if (Math.abs(get() - curv) < 0.01 && Math.abs(target - curv) > 1) { set(target); raf = 0; return; }
      raf = requestAnimationFrame(step);
    }
    return {
      to(v, ax) {
        axis = ax;
        const max = ax === 'x' ? $cards.scrollWidth - $cards.clientWidth : $cards.scrollHeight - $cards.clientHeight;
        target = clamp(v, 0, Math.max(0, max));
        if (reduced()) { set(target); return; }
        if (!raf) { last = performance.now(); raf = requestAnimationFrame(step); }
      },
      by(d, ax) { this.to((raf && axis === ax ? target : (ax === 'x' ? $cards.scrollLeft : $cards.scrollTop)) + d, ax); },
      stop() { cancelAnimationFrame(raf); raf = 0; },
    };
  })();

  function revealCur(smooth) {
    const rec = cardEls.get(S.cur);
    if (!rec) return;
    const el = rec.el;
    if (S.layout === 'grid') {
      const t = el.offsetTop, b = t + el.offsetHeight;
      const st = $cards.scrollTop, h = $cards.clientHeight;
      let to = null;
      if (t - 4 < st) to = t - (t <= 4 ? 4 : 18);
      else if (b + 24 > st + h) to = b + 24 - h;
      if (to != null) smooth ? scroller.to(to, 'y') : ($cards.scrollTop = to);
    } else {
      const l = el.offsetLeft, r = l + el.offsetWidth;
      const sl = $cards.scrollLeft, w = $cards.clientWidth;
      let to = null;
      if (l - 44 < sl) to = l - (l <= 16 ? 16 : 44);
      else if (r + 68 > sl + w) to = r + 68 - w;
      if (to != null) smooth ? scroller.to(to, 'x') : ($cards.scrollLeft = to);
    }
  }

  let fadeRaf = 0;
  function updateFades() {
    fadeRaf = 0;
    if (S.layout === 'grid') {
      const st = $cards.scrollTop, max = $cards.scrollHeight - $cards.clientHeight;
      $cards.classList.toggle('fl', st > 2);
      $cards.classList.toggle('fr', st < max - 2);
    } else {
      const sl = $cards.scrollLeft, max = $cards.scrollWidth - $cards.clientWidth;
      $cards.classList.toggle('fl', sl > 2);
      $cards.classList.toggle('fr', sl < max - 2);
    }
  }
  $cards.addEventListener('scroll', () => { if (!fadeRaf) fadeRaf = requestAnimationFrame(updateFades); }, { passive: true });
  $cards.addEventListener('wheel', (e) => {
    if (S.layout === 'grid') return;
    const d = Math.abs(e.deltaY) > Math.abs(e.deltaX) ? e.deltaY : e.deltaX;
    if (!d) return;
    e.preventDefault();
    const px = e.deltaMode === 1 ? d * 40 : e.deltaMode === 2 ? d * $cards.clientWidth : d;
    scroller.by(px * 1.2, 'x');
  }, { passive: false });

  /* ----- Preview strip and large preview ----- */

  function previewHTML(it, big) {
    const p = S.previews.get(it.id);
    const full = p && typeof p.text === 'string' ? p.text : it.text;
    const facts = [];
    if (it.age) facts.push(`<span class="pf">${esc(it.age)}</span>`);
    if (it.meta && it.kind !== 'color') facts.push(`<span class="pf">${esc(it.meta)}</span>`);
    const cc = (it.collections || []).map(collection).filter(Boolean);
    const head = `<div class="ph">${it.icon != null ? `<img class="appi" src="${esc(imgUrl(it.icon))}" alt="" decoding="async">` : ''}<span class="pa">${esc(it.app || KIND[it.kind] || '')}</span>${facts.join('')}${cc.length ? `<span class="cc">${cc.map((c) => `<span><i style="--c:${esc(c.color || 'rgb(235 235 245 / .3)')}"></i>${esc(c.name)}</span>`).join('')}</span>` : ''}</div>`;
    const H = big ? 278 : 126;
    let media = '', body = '';
    switch (it.kind) {
      case 'image': {
        const id = p && p.image != null ? p.image : it.image;
        const dims = /(\d+)\s*[×x]\s*(\d+)/.exec(it.meta || '');
        const ratio = dims ? clamp(+dims[1] / +dims[2], 0.5, 3) : 1.6;
        const maxW = big ? 440 : 300;
        const w = Math.round(Math.min(H * ratio, maxW)), h = Math.round(w / ratio);
        media = id != null ? `<div class="pm fit" style="width:${w}px;height:${Math.min(h, H)}px">${lazyImg(id)}</div>` : '';
        if (p && p.text) body = `<div class="scroll"><p class="pquiet">${esc(p.text)}</p></div>`;
        break;
      }
      case 'link': {
        if (it.image != null) media = `<div class="pm" style="width:${Math.round(H * 1.78)}px">${lazyImg(it.image)}</div>`;
        body = `${it.title ? `<p class="ptitle">${esc(it.title)}</p>` : ''}<p class="purl">${esc(it.url || full || '')}</p>`;
        break;
      }
      case 'color': {
        const rgb = hexRgb(it.color) || [0, 0, 0];
        const [h, s, l] = hsl(rgb);
        media = `<div class="pm swatch" style="--c:${esc(it.color)}"></div>`;
        body = `<dl class="vals"><dt>HEX</dt><dd>${esc(String(it.color).toUpperCase())}</dd><dt>RGB</dt><dd>${rgb.join(', ')}</dd><dt>HSL</dt><dd>${h}°, ${s}%, ${l}%</dd></dl>`;
        break;
      }
      case 'files': {
        const f = it.files || [];
        media = `<div class="pm fileic"><span class="fico">${fileIcon(f[0])}</span></div>`;
        body = `<div class="scroll"><ul class="pfiles">${f.map((n) => `<li class="${it.missing ? 'gone' : ''}">${fileExt(n) ? I.fileSm : I.folderSm}<span>${esc(n)}</span></li>`).join('')}</ul></div>`;
        break;
      }
      case 'code':
        body = `<div class="scroll"><pre class="pcode">${hl(dedent(full))}</pre></div>`;
        break;
      default: {
        const t = String(full || '');
        const [first] = splitText(t);
        const at = first ? t.indexOf(first) : -1;
        body = at >= 0
          ? `<div class="scroll"><p class="pt">${esc(t.slice(0, at))}<span class="first">${esc(first)}</span>${esc(t.slice(at + first.length))}</p></div>`
          : `<div class="scroll"><p class="pt">${esc(t)}</p></div>`;
      }
    }
    return `<div class="pv">${media}<div class="pc">${head}${body}</div></div>`;
  }

  function renderPreview(force) {
    const it = curItem();
    if (!it) {
      if ($preview._key !== '') { $preview.innerHTML = ''; $preview._key = ''; }
      $preview.classList.add('none');
      return;
    }
    $preview.classList.remove('none');
    const rec = cardEls.get(it.id);
    const key = `${it.id}|${rec ? rec.sig : ''}|${S.previews.has(it.id) ? 1 : 0}|${collections().length}`;
    if (!force && $preview._key === key) return;
    $preview._key = key;
    $preview.innerHTML = previewHTML(it, false);
    schedulePreview();
  }

  function openPreview() {
    if (!curItem()) return;
    S.overlay = 'preview';
    renderOverlay();
    showOverlay();
    requestPreview(S.cur);
  }
  function showOverlay() {
    const top = $('#stage').offsetTop;
    $overlay.style.top = `${top}px`;
    const was = $overlay.hidden;
    $overlay.hidden = false;
    $win.classList.add('ov');
    if (was && !reduced()) {
      $overlay.animate([{ opacity: 0, transform: 'scale(.97) translateY(6px)' }, { opacity: 1, transform: 'none' }],
        { duration: 160, easing: 'cubic-bezier(.2,.8,.2,1)' });
    }
  }
  function renderOverlay() {
    if (S.overlay !== 'preview') return;
    const it = curItem();
    if (!it) { closeOverlay(); return; }
    const key = `${it.id}|${S.previews.has(it.id) ? 1 : 0}`;
    if ($overlay._key === key) return;
    $overlay._key = key;
    $overlay.setAttribute('role', 'dialog');
    $overlay.setAttribute('aria-label', `Preview: ${cardLabel(it)}`);
    $overlay.innerHTML = previewHTML(it, true);
  }
  function closeOverlay() {
    if (!S.overlay) return;
    S.overlay = null; S.editId = null;
    $overlay._key = '';
    $overlay.hidden = true;
    $win.classList.remove('ov');
    $overlay.innerHTML = '';
    if (S.focus === 'search') $q.focus({ preventScroll: true }); else $cards.focus({ preventScroll: true });
  }

  function openEditor(it) {
    if (!it || (it.kind !== 'text' && it.kind !== 'code')) return;
    closeMenu();
    S.overlay = 'edit';
    S.editId = it.id;
    S.editPristine = true;
    const p = S.previews.get(it.id);
    const text = p && typeof p.text === 'string' ? p.text : it.text || '';
    $overlay._key = '';
    $overlay.setAttribute('role', 'dialog');
    $overlay.setAttribute('aria-label', 'Edit clip');
    $overlay.innerHTML = `<div class="editor"><textarea class="${it.kind === 'code' ? 'mono' : ''}" spellcheck="false" aria-label="Clip text"></textarea><div class="row"><button class="btn" type="button" data-ed="cancel">Cancel</button><button class="btn primary" type="button" data-ed="save">Save</button></div></div>`;
    const ta = $('textarea', $overlay);
    ta.value = text;
    ta.addEventListener('input', () => { S.editPristine = false; }, { once: true });
    showOverlay();
    ta.focus();
    ta.setSelectionRange(0, 0);
    ta.scrollTop = 0;
    requestPreview(it.id);
  }
  function saveEditor() {
    const ta = $('textarea', $overlay);
    if (!ta || S.editId == null) return;
    send({ type: 'edit', id: S.editId, text: ta.value });
    S.previews.delete(S.editId);
    closeOverlay();
  }
  $overlay.addEventListener('click', (e) => {
    const b = e.target.closest('[data-ed]');
    if (!b) return;
    if (b.dataset.ed === 'save') saveEditor(); else closeOverlay();
  });

  /* ---------------- Menus (dropdowns, context menu, actions palette) ---------------- */

  function openMenu(opts) {
    closeMenu();
    S.menu = { opts, levels: [{ title: null, items: resolveItems(opts.items) }], q: '', active: -1, live: !!opts.live };
    if (opts.anchor) { opts.anchor.setAttribute('aria-expanded', 'true'); }
    $pop.hidden = false;
    $pop.classList.toggle('scrim', !!opts.palette);
    $pop.innerHTML = `<div class="menu${opts.palette ? ' palette' : ''}" role="menu" aria-label="${esc(opts.label || 'Menu')}">${opts.filter ? `<input class="mf" type="text" placeholder="${esc(opts.placeholder || 'Search')}" spellcheck="false" autocomplete="off" aria-label="Filter">` : ''}<div class="mhw"></div><div class="ml"></div></div>`;
    const menu = $('.menu', $pop);
    renderMenuList();
    positionMenu(menu, opts);
    if (!reduced()) {
      menu.animate([{ opacity: 0, transform: 'scale(.97)' }, { opacity: 1, transform: 'none' }], { duration: 120, easing: 'cubic-bezier(.2,.8,.2,1)' });
    }
    const f = $('.mf', menu);
    if (f) {
      f.focus();
      f.addEventListener('input', () => { S.menu.q = f.value; S.menu.active = -1; renderMenuList(); });
    }
  }
  function resolveItems(it) { return typeof it === 'function' ? it() : it || []; }
  function refreshMenu() {
    if (!S.menu) return;
    const top = S.menu.levels[0];
    top.items = resolveItems(S.menu.opts.items);
    renderMenuList();
  }
  function closeMenu() {
    if (!S.menu) return;
    const { opts } = S.menu;
    if (opts.anchor) opts.anchor.setAttribute('aria-expanded', 'false');
    S.menu = null;
    $pop.hidden = true;
    $pop.innerHTML = '';
    if (!S.overlay && !S.tabEdit) {
      if (S.focus === 'search') $q.focus({ preventScroll: true }); else $cards.focus({ preventScroll: true });
    }
  }
  function visibleMenuItems() {
    const M = S.menu;
    const lvl = M.levels[M.levels.length - 1];
    let list = lvl.items;
    const q = M.q.trim().toLowerCase();
    if (q && M.levels.length === 1) {
      // Filtering searches one level deep, so "prompts" finds "Add to collection ▸ Prompts".
      const flat = [];
      for (const it of list) {
        if (it.sep) continue;
        if (it.label.toLowerCase().includes(q)) flat.push(it);
        else if (it.sub) for (const s of resolveItems(it.sub)) if (!s.sep && (`${it.label} ${s.label}`).toLowerCase().includes(q)) flat.push({ ...s, label: `${it.subPrefix || it.label}: ${s.label}` });
      }
      list = flat;
    }
    return list;
  }
  function renderMenuList() {
    const M = S.menu;
    const menu = $('.menu', $pop);
    if (!menu) return;
    const lvl = M.levels[M.levels.length - 1];
    const list = visibleMenuItems();
    M.visible = list;
    if (M.active < 0 || M.active >= list.length || (list[M.active] && list[M.active].sep)) {
      M.active = list.findIndex((x) => !x.sep && !x.disabled);
    }
    $('.mhw', menu).innerHTML = lvl.title ? `<div class="mh"><button type="button" tabindex="-1" data-back="1" aria-label="Back">${I.back}</button>${esc(lvl.title)}</div>` : '';
    const html = list.map((it, i) => {
      if (it.sep) return '<div class="msep" role="separator"></div>';
      const ic = it.img != null ? `<img src="${esc(imgUrl(it.img))}" alt="">` : it.swatch !== undefined ? `<i class="${it.swatch ? '' : 'none'}" style="--c:${esc(it.swatch || 'transparent')}"></i>` : it.icon || '';
      return `<button class="mi${i === M.active ? ' act' : ''}${it.danger ? ' danger' : ''}" type="button" tabindex="-1" role="${it.checked !== undefined ? 'menuitemradio' : 'menuitem'}"${it.checked !== undefined ? ` aria-checked="${!!it.checked}"` : ''}${it.sub ? ' aria-haspopup="menu"' : ''} data-i="${i}" id="mi-${i}"><span class="mic">${ic}</span><span class="ml-t">${esc(it.label)}</span>${it.count != null ? `<span class="mc">${esc(fmtCount(it.count))}</span>` : ''}${it.checked ? `<span class="mx">${I.check}</span>` : ''}${it.sub ? `<span class="ms">${I.chevR}</span>` : ''}</button>`;
    }).join('') || `<div class="mnone">${esc(M.q ? 'No matches' : '')}</div>`;
    const ml = $('.ml', menu);
    ml.innerHTML = html;
    if (M.active >= 0) {
      menu.setAttribute('aria-activedescendant', `mi-${M.active}`);
      const a = $(`#mi-${M.active}`, ml);
      if (a) a.scrollIntoView({ block: 'nearest' });
    }
  }
  function setMenuActive(i) {
    const M = S.menu;
    if (!M || i === M.active) return;
    const ml = $('.ml', $pop);
    const old = $(`#mi-${M.active}`, ml);
    if (old) old.classList.remove('act');
    M.active = i;
    const el = $(`#mi-${i}`, ml);
    if (el) { el.classList.add('act'); el.scrollIntoView({ block: 'nearest' }); }
  }
  function moveMenu(d) {
    const M = S.menu, list = M.visible || [];
    if (!list.length) return;
    let i = M.active;
    for (let n = 0; n < list.length; n++) {
      i = (i + d + list.length) % list.length;
      if (!list[i].sep && !list[i].disabled) break;
    }
    setMenuActive(i);
  }
  function activateMenu(i, viaKey) {
    const M = S.menu;
    const it = M && M.visible && M.visible[i];
    if (!it || it.sep || it.disabled) return;
    if (it.sub) {
      M.levels.push({ title: it.subTitle || it.label, items: resolveItems(it.sub) });
      M.q = '';
      const f = $('.mf', $pop); if (f) f.value = '';
      M.active = -1;
      renderMenuList();
      return;
    }
    const run = it.run;
    if (!it.keep) closeMenu();
    if (run) run(viaKey);
    if (it.keep && S.menu) refreshMenu();
  }
  function menuBack() {
    const M = S.menu;
    if (M.levels.length > 1) { M.levels.pop(); M.active = -1; renderMenuList(); return true; }
    return false;
  }
  function positionMenu(menu, opts) {
    const W = window.innerWidth, H = window.innerHeight;
    const ml = $('.ml', menu);
    const chrome = ($('.mf', menu) ? 40 : 0) + 12;
    const a = opts.anchor ? opts.anchor.getBoundingClientRect() : null;
    const top0 = opts.palette ? 56 : a ? a.bottom + 6 : 8;
    const room = opts.palette || a ? H - top0 - 12 : H - 16;
    ml.style.maxHeight = `${Math.max(96, Math.min(opts.palette ? 264 : 296, room - chrome))}px`;
    const r = menu.getBoundingClientRect();
    let x, y, ox = '50%', oy = '0';
    if (opts.palette) {
      x = (W - r.width) / 2; y = top0;
    } else if (a) {
      x = opts.alignRight ? a.right - r.width : a.left;
      y = top0;
      ox = opts.alignRight ? '100%' : '0';
    } else {
      x = opts.x; y = opts.y; ox = '0';
      if (x + r.width > W - 8) { x = opts.x - r.width; ox = '100%'; }
      if (y + r.height > H - 8) { y = Math.max(8, opts.y - r.height); oy = '100%'; }
    }
    x = clamp(x, 8, Math.max(8, W - r.width - 8));
    y = clamp(y, 8, Math.max(8, H - r.height - 8));
    menu.style.left = `${Math.round(x)}px`;
    menu.style.top = `${Math.round(y)}px`;
    menu.style.setProperty('--ox', ox);
    menu.style.setProperty('--oy', oy);
  }

  $pop.addEventListener('pointerdown', (e) => {
    if (!e.target.closest('.menu')) { e.preventDefault(); closeMenu(); }
  });
  $pop.addEventListener('pointermove', (e) => {
    const b = e.target.closest('.mi');
    if (b && S.menu) setMenuActive(+b.dataset.i);
  });
  $pop.addEventListener('click', (e) => {
    if (e.target.closest('[data-back]')) { menuBack(); return; }
    const b = e.target.closest('.mi');
    if (b) activateMenu(+b.dataset.i, false);
  });
  $pop.addEventListener('contextmenu', (e) => e.preventDefault());

  function menuKey(e) {
    const M = S.menu;
    const k = e.key;
    const inFilter = e.target && e.target.classList && e.target.classList.contains('mf');
    if (k === 'ArrowDown' || (k === 'Tab' && !e.shiftKey)) { e.preventDefault(); moveMenu(1); return; }
    if (k === 'ArrowUp' || (k === 'Tab' && e.shiftKey)) { e.preventDefault(); moveMenu(-1); return; }
    if (k === 'Enter') { e.preventDefault(); activateMenu(M.active, true); return; }
    if (k === 'ArrowRight') {
      const it = M.visible && M.visible[M.active];
      if (it && it.sub && (!inFilter || !M.q)) { e.preventDefault(); activateMenu(M.active, true); }
      return;
    }
    if (k === 'ArrowLeft' || (k === 'Backspace' && !M.q)) {
      if (M.levels.length > 1) { e.preventDefault(); menuBack(); }
      return;
    }
    if (k === 'Escape') {
      e.preventDefault();
      if (M.q && inFilter) { e.target.value = ''; M.q = ''; renderMenuList(); return; }
      if (!menuBack()) closeMenu();
      return;
    }
    if (k === 'Home' || k === 'End') {
      if (inFilter && M.q) return;
      e.preventDefault();
      M.active = k === 'Home' ? -1 : M.visible.length; moveMenu(k === 'Home' ? 1 : -1);
      return;
    }
    if ((e.ctrlKey || e.metaKey) && k.toLowerCase() === 'k') { e.preventDefault(); closeMenu(); return; }
    if (!inFilter && (k.length === 1 || k === 'Backspace' || k === 'Delete')) e.preventDefault();
  }

  /* ----- Menu contents ----- */

  function actionsFor(ids) {
    const list = ids.map(byId).filter(Boolean);
    const v = S.view || {};
    const A = [];
    if (list.length) {
      const it = list[0];
      const multi = list.length > 1;
      const textual = list.every((i) => i.kind === 'text' || i.kind === 'code' || i.kind === 'link');
      const n = list.length;
      A.push({ label: multi ? `Paste ${n} clips` : 'Paste', icon: I.paste, run: () => paste(ids) });
      A.push({ label: 'Paste as plain text', icon: I.plain, run: () => paste(ids, 'plain') });
      A.push({ label: 'Copy', icon: I.copy, run: () => copy(ids) });
      if (multi) A.push({ label: 'Paste one by one', icon: I.stack, run: () => stack(ids) });
      if (textual) {
        A.push({ sep: true });
        for (const [mode, label, icon] of TRANSFORMS) A.push({ label, icon, run: () => paste(ids, mode) });
      }
      A.push({ sep: true });
      if (!multi && (it.kind === 'text' || it.kind === 'code')) A.push({ label: 'Edit', icon: I.edit, run: () => openEditor(it) });
      const custom = collections().filter((c) => !c.builtin);
      if (custom.length) {
        A.push({
          label: 'Add to collection', subTitle: 'Add to collection', subPrefix: 'Add to', icon: I.folderSm,
          sub: () => custom.map((c) => ({
            label: c.name, swatch: c.color || null, count: c.count,
            checked: list.every((i) => (i.collections || []).includes(c.id)),
            run: () => {
              const inAll = list.every((i) => (i.collections || []).includes(c.id));
              send({ type: inAll ? 'uncollect' : 'collect', ids, collection: c.id });
            },
          })),
        });
      }
      const here = collection(S.q.collection);
      if (here && !here.builtin && list.some((i) => (i.collections || []).includes(here.id))) {
        A.push({ label: `Remove from ${here.name}`, icon: I.remove, run: () => send({ type: 'uncollect', ids, collection: here.id }) });
      }
      const pinned = list.every((i) => i.pinned);
      A.push({ label: pinned ? 'Unpin' : 'Pin', icon: I.star, run: () => togglePin(ids) });
      if (!multi && it.kind === 'image' && it.ocr) A.push({ label: 'Copy text from image', icon: I.ocr, run: () => send({ type: 'ocr', id: it.id }) });
      A.push({ sep: true });
    }
    const t = S.q.filter.type, ap = S.q.filter.app;
    A.push({
      label: 'Filter by type', subTitle: 'Type', icon: TYPE_ICON[t] || I.text,
      sub: () => [{ label: 'All types', icon: I.grid, checked: !t, run: () => setFilter('type', null) },
        ...(v.types || []).map((x) => ({ label: x.label, icon: TYPE_ICON[x.id] || I.text, count: x.count, checked: x.id === t, run: () => setFilter('type', x.id) }))],
    });
    A.push({
      label: 'Filter by app', subTitle: 'App', icon: I.files,
      sub: () => [{ label: 'All apps', icon: I.grid, checked: !ap, run: () => setFilter('app', null) },
        ...(v.apps || []).map((x) => ({ label: x.name, img: x.icon, count: x.count, checked: x.id === ap, run: () => setFilter('app', x.id) }))],
    });
    A.push({ label: S.layout === 'grid' ? 'Show as strip' : 'Show as grid', icon: S.layout === 'grid' ? I.strip : I.grid, run: () => setLayout(S.layout === 'grid' ? 'strip' : 'grid') });
    A.push({ label: 'New collection', icon: I.plus, run: () => startTabEdit(null) });
    A.push({ sep: true });
    A.push(v.paused
      ? { label: 'Resume history', icon: I.play, run: () => send({ type: 'resume' }) }
      : { label: 'Pause history', icon: I.pause, run: () => send({ type: 'pause' }) });
    if (v.undo) A.push({ label: 'Undo', icon: I.back, run: undo });
    if (list.length) A.push({ label: list.length > 1 ? `Delete ${list.length} clips` : 'Delete', icon: I.trash, danger: true, run: () => del(ids) });
    // Drop a trailing or doubled separator.
    return A.filter((x, i, a) => !(x.sep && (i === 0 || a[i - 1].sep || i === a.length - 1)));
  }

  function openActions(at) {
    const ids = selIds();
    openMenu({
      items: () => actionsFor(ids),
      palette: !at, x: at && at.x, y: at && at.y,
      filter: !at, placeholder: 'Search', label: 'Actions',
    });
  }

  function openTypeMenu() {
    const v = S.view || {};
    const types = (v.types || []).slice();
    const t = S.q.filter.type;
    if (t && !types.some((x) => x.id === t)) types.unshift({ id: t, label: t[0].toUpperCase() + t.slice(1), count: 0 });
    openMenu({
      anchor: $typeChip, label: 'Type',
      items: [
        { label: 'All types', icon: I.grid, checked: !t, run: () => setFilter('type', null) },
        { sep: true },
        ...types.map((x) => ({ label: x.label, icon: TYPE_ICON[x.id] || I.text, count: x.count, checked: x.id === t, run: () => setFilter('type', x.id) })),
      ],
    });
  }
  function openAppMenu() {
    const v = S.view || {};
    const apps = (v.apps || []).slice();
    const a = S.q.filter.app;
    openMenu({
      anchor: $appChip, label: 'App', alignRight: true, filter: apps.length > 8, placeholder: 'Search',
      items: [
        { label: 'All apps', icon: I.grid, checked: !a, run: () => setFilter('app', null) },
        { sep: true },
        ...apps.map((x) => ({ label: x.name, img: x.icon, icon: x.icon == null ? I.files : '', count: x.count, checked: x.id === a, run: () => setFilter('app', x.id) })),
      ],
    });
  }

  function openTabMenu(id, x, y) {
    const c = collection(id);
    if (!c || c.builtin) return;
    openMenu({
      x, y, label: c.name,
      items: [
        { label: 'Rename', icon: I.rename, run: () => startTabEdit(id) },
        {
          label: 'Color', icon: I.palette, subTitle: 'Color',
          sub: () => [
            { label: 'None', swatch: null, checked: !c.color, run: () => send({ type: 'collection.color', id, color: null }) },
            ...SWATCHES.map(([name, hex]) => ({ label: name, swatch: hex, checked: (c.color || '').toUpperCase() === hex, run: () => send({ type: 'collection.color', id, color: hex }) })),
          ],
        },
        { sep: true },
        { label: 'Delete', icon: I.trash, danger: true, run: () => { if (S.q.collection === id) setCollection('history'); send({ type: 'collection.delete', id }); } },
      ],
    });
  }

  /* ---------------- Collections: create, rename, reorder ---------------- */

  function startTabEdit(id) {
    closeMenu();
    let tab;
    if (id == null) {
      tab = document.createElement('div');
      tab.className = 'tab editing';
      $tabs.insertBefore(tab, $('.tab.add', $tabs));
    } else {
      tab = $(`.tab[data-id="${CSS.escape(id)}"]`, $tabs);
      if (!tab) return;
      tab.classList.add('editing');
    }
    const c = id != null ? collection(id) : null;
    const dot = c && c.color ? `<span class="dot" style="--c:${esc(c.color)}"></span>` : '';
    tab.innerHTML = `${dot}<input class="tabin" type="text" spellcheck="false" autocomplete="off" maxlength="40" aria-label="${id == null ? 'New collection name' : 'Collection name'}">`;
    const input = $('input', tab);
    input.value = c ? c.name : '';
    const fit = () => { input.style.width = `${Math.max(6, input.value.length + 1)}ch`; };
    fit();
    input.addEventListener('input', fit);
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); endTabEdit(true); }
      else if (e.key === 'Escape') { e.preventDefault(); endTabEdit(false); }
      e.stopPropagation();
    });
    input.addEventListener('blur', () => endTabEdit(true));
    S.tabEdit = { id, input };
    input.focus();
    input.select();
    tab.scrollIntoView({ inline: 'nearest', block: 'nearest' });
  }
  function endTabEdit(commit) {
    const t = S.tabEdit;
    if (!t) return;
    S.tabEdit = null;
    const name = t.input.value.trim();
    if (commit && name) {
      if (t.id == null) send({ type: 'collection.create', name });
      else if (collection(t.id) && collection(t.id).name !== name) {
        send({ type: 'collection.rename', id: t.id, name });
        collection(t.id).name = name; // optimistic until the next state
      }
    }
    renderTabs(true);
    if (S.focus === 'search') $q.focus({ preventScroll: true }); else $cards.focus({ preventScroll: true });
  }

  $tabs.addEventListener('click', (e) => {
    if (S.tabDragJustEnded) return;
    const t = e.target.closest('.tab');
    if (!t || t.classList.contains('editing')) return;
    if (t.dataset.add) { startTabEdit(null); return; }
    setCollection(t.dataset.id);
  });
  $tabs.addEventListener('dblclick', (e) => {
    const t = e.target.closest('.tab[data-id]');
    if (t && !t.dataset.builtin) startTabEdit(t.dataset.id);
  });
  $tabs.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    const t = e.target.closest('.tab[data-id]');
    if (t && !t.dataset.builtin) openTabMenu(t.dataset.id, e.clientX, e.clientY);
  });
  $tabs.addEventListener('wheel', (e) => {
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) { $tabs.scrollLeft += e.deltaY; e.preventDefault(); }
  }, { passive: false });

  // Drag a custom tab to reorder.
  $tabs.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    const t = e.target.closest('.tab[data-id]');
    if (!t || t.dataset.builtin || t.classList.contains('editing')) return;
    S.tabDragStart = { el: t, x: e.clientX, id: t.dataset.id, pid: e.pointerId, moved: false };
  });
  $tabs.addEventListener('pointermove', (e) => {
    const d = S.tabDragStart;
    if (!d || e.pointerId !== d.pid) return;
    if (!d.moved) {
      if (Math.abs(e.clientX - d.x) < 5) return;
      d.moved = true;
      S.tabDrag = d;
      d.el.classList.add('lift');
      $tabs.setPointerCapture(e.pointerId);
    }
    const tabs = [...$tabs.querySelectorAll('.tab[data-id]:not([data-builtin])')];
    const others = tabs.filter((x) => x !== d.el);
    let before = null;
    for (const o of others) {
      const r = o.getBoundingClientRect();
      if (e.clientX < r.left + r.width / 2) { before = o; break; }
    }
    const ref = before || (others.length ? others[others.length - 1].nextSibling : null);
    if (ref !== d.el && d.el.nextSibling !== ref) {
      const rects = new Map(others.map((o) => [o, o.getBoundingClientRect().left]));
      $tabs.insertBefore(d.el, ref || $('.tab.add', $tabs));
      if (!reduced()) {
        for (const [o, l0] of rects) {
          const dx = l0 - o.getBoundingClientRect().left;
          if (dx) o.animate([{ transform: `translateX(${dx}px)` }, { transform: 'none' }], { duration: 180, easing: 'cubic-bezier(.2,.8,.2,1)' });
        }
      }
    }
  });
  const endTabDrag = (e) => {
    const d = S.tabDragStart;
    if (!d || (e && e.pointerId !== d.pid)) return;
    S.tabDragStart = null;
    if (!d.moved) return;
    d.el.classList.remove('lift');
    S.tabDrag = null;
    S.tabDragJustEnded = true;
    setTimeout(() => { S.tabDragJustEnded = false; }, 0);
    const ids = [...$tabs.querySelectorAll('.tab[data-id]:not([data-builtin])')].map((x) => x.dataset.id);
    const before = collections().filter((c) => !c.builtin).map((c) => c.id);
    if (ids.join('\n') !== before.join('\n')) {
      send({ type: 'collection.reorder', ids });
      const map = new Map(collections().map((c) => [c.id, c]));
      S.view.collections = [...collections().filter((c) => c.builtin), ...ids.map((id) => map.get(id))];
    }
    renderTabs(true);
  };
  $tabs.addEventListener('pointerup', endTabDrag);
  $tabs.addEventListener('pointercancel', endTabDrag);

  /* ---------------- Cards: mouse, drag ---------------- */

  $cards.addEventListener('pointerdown', (e) => {
    const c = e.target.closest('.card');
    if (!c || e.button !== 0) return;
    S.drag = { id: +c.dataset.id, el: c, x: e.clientX, y: e.clientY, pid: e.pointerId, live: false };
  });
  $cards.addEventListener('pointermove', (e) => {
    const d = S.drag;
    if (!d || e.pointerId !== d.pid) return;
    if (!d.live) {
      if (Math.hypot(e.clientX - d.x, e.clientY - d.y) < 6) return;
      beginCardDrag(e);
    }
    moveCardDrag(e);
  });
  $cards.addEventListener('pointerup', (e) => {
    const d = S.drag;
    if (!d || e.pointerId !== d.pid) return;
    if (d.live) dropCard(e);
    else S.drag = null;
  });
  $cards.addEventListener('pointercancel', () => endCardDrag(true));
  $cards.addEventListener('lostpointercapture', () => { if (S.drag && S.drag.live) endCardDrag(true); });

  function beginCardDrag(e) {
    const d = S.drag;
    d.live = true;
    d.ids = S.sel.includes(d.id) && S.sel.length > 1 ? selIds() : [d.id];
    const r = d.el.getBoundingClientRect();
    d.ox = e.clientX - r.left; d.oy = e.clientY - r.top;
    const g = d.el.cloneNode(true);
    g.classList.add('ghost');
    g.classList.remove('cur', 'pick');
    g.removeAttribute('id');
    g.setAttribute('aria-hidden', 'true');
    g.style.width = `${r.width}px`;
    g.style.height = `${r.height}px`;
    if (d.ids.length > 1) g.insertAdjacentHTML('beforeend', `<span class="gn">${d.ids.length}</span>`);
    document.body.appendChild(g);
    d.ghost = g;
    d.el.classList.add('dragging');
    try { $cards.setPointerCapture(e.pointerId); } catch (_) { /* ignore */ }
    if (!reduced()) g.animate([{ scale: 1 }, { scale: 0.86 }], { duration: 140, fill: 'forwards', easing: 'cubic-bezier(.2,.8,.2,1)' });
  }
  function moveCardDrag(e) {
    const d = S.drag;
    if (!d || !d.ghost) return;
    d.ghost.style.transform = `translate(${e.clientX - d.ox}px, ${e.clientY - d.oy}px)`;
    const out = e.clientX < 0 || e.clientY < 0 || e.clientX >= window.innerWidth || e.clientY >= window.innerHeight;
    if (out) {
      // Leaving the window: the host takes over with a native drag-out.
      const id = d.id;
      endCardDrag(true);
      send({ type: 'host.drag', id });
      return;
    }
    const hit = document.elementFromPoint(e.clientX, e.clientY);
    const tab = hit && hit.closest('.tab[data-id]');
    const ok = tab && tab.dataset.id !== 'history' ? tab : null;
    if (d.over !== ok) {
      if (d.over) d.over.classList.remove('drop');
      if (ok) ok.classList.add('drop');
      d.over = ok;
    }
  }
  function dropCard() {
    const d = S.drag;
    const tab = d && d.over;
    const ids = d ? d.ids : [];
    endCardDrag(false);
    if (!tab) return;
    const id = tab.dataset.id;
    if (id === 'pinned') {
      for (const x of ids) { const it = byId(x); if (it && !it.pinned) send({ type: 'pin', id: x }); }
    } else {
      send({ type: 'collect', ids, collection: id });
    }
    if (!reduced()) tab.animate([{ transform: 'scale(1.08)' }, { transform: 'none' }], { duration: 220, easing: 'cubic-bezier(.2,.8,.2,1)' });
  }
  function endCardDrag(cancel) {
    const d = S.drag;
    if (!d) return;
    S.drag = null;
    if (d.over) d.over.classList.remove('drop');
    if (d.el) d.el.classList.remove('dragging');
    if (d.ghost) d.ghost.remove();
    if (d.live) { S.cardDragJustEnded = true; setTimeout(() => { S.cardDragJustEnded = false; }, 0); }
    try { $cards.releasePointerCapture(d.pid); } catch (_) { /* ignore */ }
    void cancel;
  }

  $cards.addEventListener('click', (e) => {
    if (S.cardDragJustEnded) return;
    const c = e.target.closest('.card');
    if (!c) return;
    const id = +c.dataset.id;
    if (e.ctrlKey || e.metaKey) setCursor(id, { toggle: true });
    else if (e.shiftKey) {
      const a = indexOf(S.cur), b = indexOf(id);
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        const range = items().slice(lo, hi + 1).map((i) => i.id);
        S.sel = [...new Set([...S.sel, ...range])];
        S.cur = id;
        updateSel(true);
      } else setCursor(id);
    } else setCursor(id);
  });
  $cards.addEventListener('dblclick', (e) => {
    const c = e.target.closest('.card');
    if (!c) return;
    const id = +c.dataset.id;
    paste(S.sel.length > 1 && S.sel.includes(id) ? selIds() : [id]);
  });
  $cards.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    const c = e.target.closest('.card');
    if (!c) return;
    const id = +c.dataset.id;
    if (!S.sel.includes(id)) setCursor(id); else { S.cur = id; updateSel(false); }
    openActions({ x: e.clientX + 2, y: e.clientY + 2 });
  });

  /* ---------------- Top row events ---------------- */

  $q.addEventListener('input', () => {
    S.q.query = $q.value;
    $clear.hidden = !$q.value;
    sendQuery(false);
  });
  $q.addEventListener('focus', () => { S.focus = 'search'; applyFocus(); });
  $cards.addEventListener('focus', () => { S.focus = 'cards'; applyFocus(); });
  $clear.addEventListener('click', () => { clearSearch(); $q.focus(); });
  $typeChip.addEventListener('click', () => (S.menu && S.menu.opts.anchor === $typeChip ? closeMenu() : openTypeMenu()));
  $appChip.addEventListener('click', () => (S.menu && S.menu.opts.anchor === $appChip ? closeMenu() : openAppMenu()));
  $layoutBtn.addEventListener('click', () => setLayout(S.layout === 'grid' ? 'strip' : 'grid'));
  $status.addEventListener('click', (e) => {
    const b = e.target.closest('[data-act]');
    if (!b) return;
    if (b.dataset.act === 'undo') undo();
    if (b.dataset.act === 'resume') send({ type: 'resume' });
  });
  // Buttons never keep focus: the keyboard stays with search or cards.
  for (const b of [$clear, $typeChip, $appChip, $layoutBtn]) b.addEventListener('mousedown', (e) => e.preventDefault());
  $status.addEventListener('mousedown', (e) => e.preventDefault());
  $tabs.addEventListener('mousedown', (e) => { if (!e.target.closest('input')) e.preventDefault(); });
  $cards.addEventListener('mousedown', (e) => { if (e.target.closest('.card')) { e.preventDefault(); } });
  $preview.addEventListener('dblclick', () => openPreview());

  function clearSearch() {
    $q.value = '';
    S.q.query = '';
    $clear.hidden = true;
    sendQuery(true);
  }

  function applyFocus() {
    $cards.classList.toggle('focused', S.focus === 'cards' && document.activeElement === $cards);
  }
  function focusCards() {
    S.focus = 'cards';
    $cards.focus({ preventScroll: true });
    applyFocus();
  }
  function focusSearch(select) {
    S.focus = 'search';
    $q.focus({ preventScroll: true });
    if (select) $q.select();
    applyFocus();
  }

  document.addEventListener('contextmenu', (e) => {
    if (!e.target.closest('input, textarea')) e.preventDefault();
  });

  /* ---------------- Keyboard ---------------- */

  const isPrintable = (e) => e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey;

  window.addEventListener('keyup', (e) => { if (e.key === 'Control') $win.classList.remove('ctrl'); });
  window.addEventListener('blur', () => $win.classList.remove('ctrl'));

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Control') $win.classList.add('ctrl');
    if (S.tabEdit) return; // the name field handles its own keys
    if (S.menu) { menuKey(e); return; }

    const k = e.key;
    const ctrl = e.ctrlKey || e.metaKey;
    const shift = e.shiftKey;
    const lk = k.length === 1 ? k.toLowerCase() : k;

    if (S.overlay === 'edit') {
      if (k === 'Escape') { e.preventDefault(); closeOverlay(); }
      else if (ctrl && (k === 'Enter' || lk === 's')) { e.preventDefault(); saveEditor(); }
      return;
    }

    const inSearch = document.activeElement === $q;
    const qEmpty = !$q.value;
    const cardsKeys = !inSearch || qEmpty; // keys that would edit text belong to search while it has text

    if (S.overlay === 'preview') {
      if (k === ' ' || k === 'Escape') { e.preventDefault(); closeOverlay(); return; }
    }

    // Shortcuts with Ctrl first.
    if (ctrl && !e.altKey) {
      if (/^[1-9]$/.test(k)) {
        e.preventDefault();
        const it = items()[+k - 1];
        if (it) paste([it.id]);
        return;
      }
      switch (lk) {
        case 'Enter': e.preventDefault(); copy(selIds()); return;
        case 'k': e.preventDefault(); openActions(null); return;
        case 'f': e.preventDefault(); closeOverlay(); focusSearch(true); return;
        case 'p': e.preventDefault(); togglePin(selIds()); return;
        case 'z':
          if (inSearch && !qEmpty) return; // text undo in the field
          e.preventDefault(); if (S.view && S.view.undo) undo(); return;
        case 'v':
          if (shift) { e.preventDefault(); stack(selIds()); }
          return;
        case 'Tab': {
          e.preventDefault();
          const cols = collections();
          if (!cols.length) return;
          const i = cols.findIndex((c) => c.id === S.q.collection);
          setCollection(cols[(i + (shift ? -1 : 1) + cols.length) % cols.length].id);
          return;
        }
        case 'a':
          if (!inSearch || qEmpty) { e.preventDefault(); S.sel = items().map((i) => i.id); updateSel(false); }
          return;
        default: break;
      }
    }

    switch (k) {
      case 'Escape':
        e.preventDefault();
        if (S.sel.length > 1) { S.sel = S.cur != null ? [S.cur] : []; updateSel(false); return; }
        if (!qEmpty) { clearSearch(); focusSearch(false); return; }
        send({ type: 'host.close' });
        return;
      case 'Enter':
        e.preventDefault();
        if (e.altKey) return;
        paste(selIds(), shift ? 'plain' : 'normal');
        return;
      case 'Tab':
        e.preventDefault();
        if (inSearch) focusCards(); else focusSearch(false);
        return;
      case 'ArrowLeft': case 'ArrowRight': {
        if (e.altKey) return;
        e.preventDefault();
        move(k === 'ArrowRight' ? 1 : -1, { extend: shift });
        return;
      }
      case 'ArrowUp': case 'ArrowDown': {
        e.preventDefault();
        if (S.layout === 'grid') move((k === 'ArrowDown' ? 1 : -1) * S.cols, { extend: shift });
        return;
      }
      case 'Home': case 'End':
        if (!cardsKeys) return;
        e.preventDefault();
        if (items().length) setCursor(items()[k === 'Home' ? 0 : items().length - 1].id, { extend: shift });
        return;
      case 'PageDown': case 'PageUp':
        e.preventDefault();
        move((k === 'PageDown' ? 1 : -1) * (S.layout === 'grid' ? S.cols * 2 : 4));
        return;
      case ' ':
        if (!cardsKeys) return;
        e.preventDefault();
        if (S.overlay === 'preview') closeOverlay(); else openPreview();
        return;
      case 'Delete':
        if (!cardsKeys) return;
        e.preventDefault();
        del(selIds());
        return;
      case 'Backspace':
        if (!inSearch && qEmpty) { e.preventDefault(); return; }
        if (!inSearch) focusSearch(false);
        return;
      default:
        break;
    }

    // Typing always goes to search (the key lands in the field once it has focus).
    if (isPrintable(e) && !inSearch) { closeOverlay(); focusSearch(false); }
  }, true);

  /* ---------------- Start ---------------- */

  applyLayout();
  renderTop();
  send({ type: 'host.ready' });
  if (dev && dev.ready) dev.ready({ onMessage, S });
})();
