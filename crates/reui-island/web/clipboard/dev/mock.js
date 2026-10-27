/* Dev harness for the clipboard window. Not embedded in the exe.
   Simulates window.chrome.webview (both directions) and a small hub.
   Query switches:
     ?layout=grid  ?state=empty  ?noresults=1  ?preview=1  ?palette=1  ?menu=type|app|tab
     ?paused=1  ?undo=1  ?stack=1  ?multi=1  ?sel=N  ?edit=1  ?newclip=1  ?newtab=1  ?log=1 */
(() => {
  'use strict';
  const params = new URLSearchParams(location.search);
  const flag = (k) => params.has(k) && params.get(k) !== '0';

  /* ---------------- Fake images (SVG data URIs) ---------------- */

  const svg = (w, h, body) => 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${body}</svg>`);
  const sq = (fill, inner) => svg(32, 32, `<rect width="32" height="32" rx="7.5" fill="${fill}"/>${inner}`);

  const IMAGES = {
    // App icons
    1: svg(32, 32, `<circle cx="16" cy="16" r="15" fill="#DB4437"/><path d="M16 16 L29 23.5 A15 15 0 0 1 3 23.5Z" fill="#0F9D58"/><path d="M16 16 L3 23.5 A15 15 0 0 1 16 1 Z" fill="#F4B400" opacity="0"/><path d="M16 16 L29 23.5 A15 15 0 0 0 16 1 Z" fill="#F4B400"/><circle cx="16" cy="16" r="6.6" fill="#fff"/><circle cx="16" cy="16" r="5" fill="#4285F4"/>`),
    2: sq('#1F8AD2', `<path d="M22.5 6.5 L12 16 L22.5 25.5 L25.5 24 V8Z" fill="#fff" opacity=".95"/><path d="M6.5 12 L9 10.5 L22.5 22 V25.5Z M6.5 20 L9 21.5 L22.5 10 V6.5Z" fill="#fff" opacity=".75"/>`),
    3: sq('#1E1E1E', `<circle cx="13" cy="9" r="3.4" fill="#F24E1E"/><circle cx="19.5" cy="9" r="3.4" fill="#FF7262"/><circle cx="13" cy="16" r="3.4" fill="#A259FF"/><circle cx="19.5" cy="16" r="3.4" fill="#1ABCFE"/><circle cx="13" cy="23" r="3.4" fill="#0ACF83"/>`),
    4: sq('#fff', `<rect x="8" y="13.5" width="10" height="3.6" rx="1.8" fill="#36C5F0"/><rect x="14.4" y="8" width="3.6" height="10" rx="1.8" fill="#2EB67D"/><rect x="14" y="15" width="10" height="3.6" rx="1.8" fill="#ECB22E"/><rect x="14" y="14.4" width="3.6" height="10" rx="1.8" fill="#E01E5A"/>`),
    5: sq('#2B2B2F', `<path d="M8.5 11l5 5-5 5" stroke="#8FE388" stroke-width="2.4" fill="none" stroke-linecap="round" stroke-linejoin="round"/><path d="M16 22h8" stroke="#ddd" stroke-width="2.4" stroke-linecap="round"/>`),
    6: svg(32, 32, `<path d="M3 9a3 3 0 0 1 3-3h7l3 3h10a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z" fill="#F8C24A"/><path d="M3 13h26v11a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z" fill="#FFD866"/>`),
    7: sq('#fff', `<path d="M11 9v14M11 9l10 14M21 9v14" stroke="#111" stroke-width="2.4" fill="none" stroke-linejoin="round"/>`),
    8: svg(32, 32, `<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#7B61FF"/><stop offset="1" stop-color="#1E90FF"/></linearGradient></defs><rect width="32" height="32" rx="7.5" fill="url(#g)"/><circle cx="11" cy="21" r="3.2" stroke="#fff" stroke-width="2" fill="none"/><circle cx="21" cy="21" r="3.2" stroke="#fff" stroke-width="2" fill="none"/><path d="M13 18.5 L21 8M19 18.5 L11 8" stroke="#fff" stroke-width="2" stroke-linecap="round"/>`),
    9: sq('#001E36', `<text x="16" y="21.5" text-anchor="middle" font-family="Segoe UI" font-weight="700" font-size="13" fill="#31A8FF">Ps</text>`),

    // Content
    901: svg(1600, 900, `<defs><linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2E5BD8"/><stop offset=".55" stop-color="#8EC5FF"/><stop offset="1" stop-color="#E9F3FF"/></linearGradient><linearGradient id="h1" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#9BC35A"/><stop offset="1" stop-color="#3E6B1F"/></linearGradient><linearGradient id="h2" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#7FAE45"/><stop offset="1" stop-color="#2C4F14"/></linearGradient></defs><rect width="1600" height="900" fill="url(#s)"/><circle cx="1180" cy="250" r="90" fill="#FFF6D8" opacity=".9"/><path d="M0 560 C300 470 520 520 800 560 S1300 470 1600 520 V900 H0Z" fill="#6E9AAE" opacity=".55"/><path d="M0 650 C320 520 640 560 900 640 S1400 600 1600 560 V900 H0Z" fill="url(#h1)"/><path d="M0 760 C380 650 760 700 1100 770 S1500 760 1600 730 V900 H0Z" fill="url(#h2)"/>`),
    902: svg(1200, 800, `<defs><radialGradient id="a" cx=".2" cy=".25" r=".7"><stop offset="0" stop-color="#FF5FA2"/><stop offset="1" stop-color="#FF5FA2" stop-opacity="0"/></radialGradient><radialGradient id="b" cx=".85" cy=".3" r=".6"><stop offset="0" stop-color="#FFB547"/><stop offset="1" stop-color="#FFB547" stop-opacity="0"/></radialGradient><radialGradient id="c" cx=".55" cy=".95" r=".7"><stop offset="0" stop-color="#6A5BFF"/><stop offset="1" stop-color="#6A5BFF" stop-opacity="0"/></radialGradient></defs><rect width="1200" height="800" fill="#1B0F3A"/><rect width="1200" height="800" fill="url(#a)"/><rect width="1200" height="800" fill="url(#b)"/><rect width="1200" height="800" fill="url(#c)"/>`),
    903: svg(1920, 1080, `<rect width="1920" height="1080" fill="#202124"/><rect x="0" y="0" width="1920" height="64" fill="#2B2C30"/><circle cx="40" cy="32" r="9" fill="#FF5F57"/><circle cx="70" cy="32" r="9" fill="#FEBC2E"/><circle cx="100" cy="32" r="9" fill="#28C840"/><rect x="0" y="64" width="360" height="1016" fill="#26272B"/>${[0,1,2,3,4,5,6].map((i) => `<rect x="40" y="${120 + i * 64}" width="${220 - i * 14}" height="18" rx="9" fill="#3A3B40"/>`).join('')}<rect x="440" y="120" width="900" height="56" rx="12" fill="#E8EAED"/><rect x="440" y="230" width="1380" height="380" rx="24" fill="#0A84FF"/><rect x="440" y="660" width="660" height="340" rx="24" fill="#34363B"/><rect x="1160" y="660" width="660" height="340" rx="24" fill="#34363B"/><rect x="500" y="300" width="560" height="54" rx="10" fill="#fff" opacity=".9"/><rect x="500" y="380" width="380" height="30" rx="8" fill="#fff" opacity=".55"/>`),
    904: svg(1200, 630, `<rect width="1200" height="630" fill="#0D1117"/><text x="80" y="200" font-family="Segoe UI" font-size="38" fill="#8B949E">rust-lang/</text><text x="80" y="290" font-family="Segoe UI" font-weight="700" font-size="88" fill="#F0F6FC">rust</text><text x="80" y="370" font-family="Segoe UI" font-size="30" fill="#8B949E">Empowering everyone to build reliable and</text><text x="80" y="412" font-family="Segoe UI" font-size="30" fill="#8B949E">efficient software.</text><circle cx="1040" cy="210" r="90" fill="#F74C00"/><text x="1040" y="236" text-anchor="middle" font-family="Segoe UI" font-weight="700" font-size="72" fill="#0D1117">R</text><rect x="0" y="606" width="760" height="24" fill="#DEA584"/><rect x="760" y="606" width="260" height="24" fill="#3572A5"/><rect x="1020" y="606" width="180" height="24" fill="#89E051"/>`),
    905: svg(1200, 630, `<defs><linearGradient id="w" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#F6E7D3"/><stop offset="1" stop-color="#E8C9A6"/></linearGradient></defs><rect width="1200" height="630" fill="url(#w)"/><text x="80" y="250" font-family="Georgia" font-size="76" fill="#2A1B0E">The design of</text><text x="80" y="345" font-family="Georgia" font-style="italic" font-size="76" fill="#2A1B0E">everyday clipboards</text><rect x="80" y="420" width="120" height="6" fill="#C0562A"/>`),
    906: svg(1200, 1200, `<defs><radialGradient id="o" cx=".4" cy=".35" r=".65"><stop offset="0" stop-color="#C4F1FF"/><stop offset=".45" stop-color="#3FA9F5"/><stop offset="1" stop-color="#08214A"/></radialGradient></defs><rect width="1200" height="1200" fill="#05060A"/><circle cx="600" cy="620" r="380" fill="url(#o)"/><ellipse cx="600" cy="1060" rx="420" ry="40" fill="#3FA9F5" opacity=".18"/>`),
    907: svg(1280, 800, `<rect width="1280" height="800" fill="#F2F2F4"/><rect x="140" y="160" width="1000" height="480" rx="40" fill="#fff" stroke="#E2E2E6" stroke-width="4"/><circle cx="330" cy="400" r="110" fill="#FF9F0A"/><rect x="520" y="300" width="500" height="44" rx="12" fill="#1C1C1E"/><rect x="520" y="370" width="380" height="28" rx="10" fill="#8E8E93"/><rect x="520" y="430" width="220" height="64" rx="32" fill="#0A84FF"/>`),
  };

  // A real-size 64-bit id (beyond Number.MAX_SAFE_INTEGER) to prove ids stay exact strings.
  IMAGES['13021587761245113856'] = IMAGES[901];

  /* ---------------- Fake hub data ---------------- */

  const APPS = {
    'chrome.exe': { name: 'Chrome', icon: 1 },
    'Code.exe': { name: 'Visual Studio Code', icon: 2 },
    'Figma.exe': { name: 'Figma', icon: 3 },
    'slack.exe': { name: 'Slack', icon: 4 },
    'WindowsTerminal.exe': { name: 'Terminal', icon: 5 },
    'explorer.exe': { name: 'File Explorer', icon: 6 },
    'Notion.exe': { name: 'Notion', icon: 7 },
    'SnippingTool.exe': { name: 'Snipping Tool', icon: 8 },
    'Photoshop.exe': { name: 'Photoshop', icon: 9 },
  };

  let nextId = 100;
  // Image ids are opaque decimal strings in the contract (64-bit values don't fit a JS number).
  const imgId = (n) => (n == null ? null : String(n));
  const mk = (o) => {
    const app = APPS[o.exe] || null;
    if ('image' in o) o.image = imgId(o.image);
    return Object.assign({
      id: nextId++, kind: 'text', text: null, rich: false, title: null, site: null, url: null, image: null,
      icon: app ? imgId(app.icon) : null, app: app ? app.name : null, age: '1h', meta: null, color: null, files: null,
      pinned: false, collections: [], cut: false, missing: false, lines: 1, ocr: false,
    }, o);
  };

  let ITEMS = [
    mk({ exe: 'slack.exe', age: 'now', text: 'Can we move the design review to Thursday?\nThe new clipboard window needs one more pass on the cards, and I want Rohit to see the grid layout before we lock it.', lines: 2, meta: '2 lines' }),
    mk({ exe: 'SnippingTool.exe', kind: 'image', image: 903, age: '2m', meta: '1920 × 1080', ocr: true, text: 'Dashboard Overview Revenue Q3 Reports' }),
    mk({ exe: 'chrome.exe', kind: 'link', url: 'https://github.com/rust-lang/rust', title: 'rust-lang/rust: Empowering everyone to build reliable and efficient software.', site: 'github.com', image: 904, age: '5m', meta: 'github.com', collections: ['c9'] }),
    mk({ exe: 'Figma.exe', kind: 'color', color: '#0A84FF', age: '12m', collections: ['c8'] }),
    mk({ exe: 'Code.exe', kind: 'code', age: '18m', pinned: true, lines: 6, meta: '6 lines', collections: ['c10'],
      text: 'fn main() {\n    // Greet whoever opened the window\n    let name = "ReUI";\n    let count = 42;\n    println!("Hello, {}! {}", name, count);\n}' }),
    mk({ exe: 'explorer.exe', kind: 'files', files: ['Quarterly report.pdf', 'Budget 2026.xlsx', 'Notes.txt'], age: '25m', meta: '3 files' }),
    mk({ exe: 'chrome.exe', kind: 'image', image: '13021587761245113856', age: '40m', meta: '1600 × 900', collections: ['c9'] }),
    mk({ exe: 'Notion.exe', age: '1h', text: 'Minneapolis 55410\n2941 Rocket Drive\nUnited States', lines: 3, meta: '3 lines' }),
    mk({ exe: 'chrome.exe', kind: 'link', url: 'https://docs.rs/windows/latest/windows/Win32/UI/WindowsAndMessaging/', title: 'windows::Win32::UI::WindowsAndMessaging - Rust', site: 'docs.rs', age: '1h', meta: 'docs.rs' }),
    mk({ exe: 'Figma.exe', kind: 'color', color: '#FF9F0A', age: '2h', collections: ['c8'] }),
    mk({ exe: 'WindowsTerminal.exe', kind: 'code', age: '2h', meta: '1 line', text: 'cargo build --release -p reui-island && ./target/release/reui-island.exe' }),
    mk({ exe: 'chrome.exe', age: '3h', rich: true, lines: 1, text: 'Supaste saves your clipboard and screenshots in a beautiful visual history, automatically grouped by type, app and custom categories, so you can find anything in seconds.' }),
    mk({ exe: 'Figma.exe', kind: 'image', image: 902, age: '3h', meta: '1200 × 800', pinned: true, collections: ['c9'] }),
    mk({ exe: 'Notion.exe', age: '4h', lines: 5, meta: '5 lines', collections: ['c7'], pinned: true,
      text: 'You are a senior Rust reviewer.\nPoint out unsafe blocks, needless clones and blocking calls on the UI thread.\nSuggest the smallest fix for each.\nKeep the tone direct.\nEnd with a one-line verdict.' }),
    mk({ exe: 'Figma.exe', kind: 'color', color: '#30D158', age: '5h', collections: ['c8'] }),
    mk({ exe: 'chrome.exe', kind: 'code', age: '5h', lines: 5, meta: '5 lines', text: '{\n  "name": "reui",\n  "version": "0.4.0",\n  "private": true\n}' }),
    mk({ exe: 'chrome.exe', kind: 'link', url: 'https://medium.com/@ux/the-design-of-everyday-clipboards', title: 'The design of everyday clipboards', site: 'medium.com', image: 905, age: '6h', meta: 'medium.com' }),
    mk({ exe: 'slack.exe', age: '7h', text: 'rohit@reui.dev', meta: null }),
    mk({ exe: 'explorer.exe', kind: 'files', files: ['old-logo.svg'], age: '1d', meta: 'Missing', missing: true }),
    mk({ exe: 'Code.exe', age: '1d', cut: true, meta: 'Cut', text: 'TODO: wire host.resize to the WebView bounds' }),
    mk({ exe: 'Photoshop.exe', kind: 'image', image: 906, age: '1d', meta: '1200 × 1200', collections: ['c9'] }),
    mk({ exe: 'Notion.exe', age: '2d', lines: 3, meta: '3 lines', collections: ['c7'],
      text: 'Summarise this thread in three bullets.\nLead with the decision, then owners and dates.\nNo filler.' }),
    mk({ exe: 'Figma.exe', kind: 'color', color: '#BF5AF2', age: '2d', collections: ['c8'] }),
    mk({ exe: 'chrome.exe', kind: 'image', image: 907, age: '3d', meta: '1280 × 800' }),
    mk({ exe: 'Code.exe', kind: 'code', age: '3d', lines: 4, meta: '4 lines', collections: ['c10'],
      text: 'SELECT id, kind, created_at\nFROM clips\nWHERE pinned = 1\nORDER BY created_at DESC;' }),
    mk({ exe: 'explorer.exe', kind: 'files', files: ['Assets'], age: '4d', meta: 'Folder' }),
    mk({ exe: 'slack.exe', age: '5d', lines: 1, text: 'Ship it once the focus restore works in Word and VS Code.' }),
    mk({ exe: 'Figma.exe', kind: 'color', color: '#F2F2F7', age: '6d', collections: ['c8'] }),
  ];
  const FULL = new Map(); // full preview text by id
  FULL.set(ITEMS[11].id, ITEMS[11].text + '\n\nOne-time purchase. Fully offline and private. Works with every app you already use, and remembers what you copied last week as well as what you copied a second ago.');

  let COLLECTIONS = [
    { id: 'history', name: 'History', color: null, builtin: true },
    { id: 'pinned', name: 'Pinned', color: null, builtin: true },
    { id: 'c7', name: 'Prompts', color: '#0A84FF', builtin: false },
    { id: 'c8', name: 'Colors', color: '#FF9F0A', builtin: false },
    { id: 'c9', name: 'Assets', color: '#BF5AF2', builtin: false },
    { id: 'c10', name: 'Snippets', color: null, builtin: false },
  ];

  if (params.get('state') === 'empty') ITEMS = [];
  if (flag('still')) {
    const st = document.createElement('style');
    st.textContent = '*,*::before,*::after{transition:none!important}';
    document.head.appendChild(st);
  }

  const Q = { query: '', collection: 'history', filter: { type: null, app: null } };
  let layout = params.get('layout') === 'grid' ? 'grid' : 'strip';
  let paused = flag('paused');
  let undoStack = [];
  let undoFlag = false;
  let stackState = { active: false, remaining: 0 };
  let cid = 20;

  const TYPE_OF = { text: 'text', code: 'code', link: 'links', image: 'images', color: 'colors', files: 'files' };
  const TYPE_LABEL = { text: 'Text', code: 'Code', links: 'Links', images: 'Images', colors: 'Colors', files: 'Files' };
  const exeOf = (it) => Object.keys(APPS).find((k) => APPS[k].name === it.app) || null;

  function inCollection(it, c) {
    if (c === 'history') return true;
    if (c === 'pinned') return it.pinned;
    return it.collections.includes(c);
  }
  function matches(it, q) {
    if (!q) return true;
    const hay = [it.text, it.title, it.url, it.site, it.app, ...(it.files || []), it.color].join(' ').toLowerCase();
    return hay.includes(q.toLowerCase());
  }

  function view() {
    const base = ITEMS.filter((it) => inCollection(it, Q.collection) && matches(it, Q.query));
    const byApp = base.filter((it) => !Q.filter.app || exeOf(it) === Q.filter.app);
    const byType = base.filter((it) => !Q.filter.type || TYPE_OF[it.kind] === Q.filter.type);
    const all = byApp.filter((it) => !Q.filter.type || TYPE_OF[it.kind] === Q.filter.type);
    let list = all.slice();
    if (Q.collection === 'pinned') list.sort((a, b) => b.pinned - a.pinned);
    const typeCounts = {};
    for (const it of byApp) typeCounts[TYPE_OF[it.kind]] = (typeCounts[TYPE_OF[it.kind]] || 0) + 1;
    const appCounts = {};
    for (const it of byType) { const e = exeOf(it); if (e) appCounts[e] = (appCounts[e] || 0) + 1; }
    const inColl = ITEMS.filter((it) => inCollection(it, Q.collection));
    return {
      query: Q.query, collection: Q.collection, filter: { ...Q.filter }, layout,
      collections: COLLECTIONS.map((c) => ({ ...c, count: ITEMS.filter((it) => inCollection(it, c.id)).length })),
      types: Object.keys(TYPE_LABEL).filter((t) => typeCounts[t]).map((t) => ({ id: t, label: TYPE_LABEL[t], count: typeCounts[t] })),
      apps: Object.keys(appCounts).map((e) => ({ id: e, name: APPS[e].name, icon: imgId(APPS[e].icon), count: appCounts[e] })),
      items: list.slice(0, 60).map((it) => ({ ...it, text: it.text && it.text.slice(0, 2000) })),
      total: list.length,
      paused, undo: undoFlag, stack: { ...stackState },
      empty: list.length ? null : inColl.length || ITEMS.length ? 'No matches' : 'Nothing copied yet',
    };
  }

  /* ---------------- Fake webview bridge ---------------- */

  const listeners = [];
  const SENT = [];
  const logEl = flag('log') ? document.createElement('pre') : null;
  if (logEl) { logEl.id = 'mocklog'; addEventListener('DOMContentLoaded', () => document.body.appendChild(logEl)); }
  function log(dir, msg) {
    const line = `${dir} ${JSON.stringify(msg)}`;
    console.log('%c' + line, `color:${dir === '→' ? '#7cc4ff' : '#9fe3a3'}`);
    if (logEl) { logEl.textContent = (line + '\n' + logEl.textContent).slice(0, 4000); }
  }
  function toPage(msg) {
    log('←', msg);
    // The real host posts JSON; deliver it as an object like PostWebMessageAsJson does.
    setTimeout(() => listeners.forEach((fn) => fn({ data: JSON.parse(JSON.stringify(msg)) })), 0);
  }
  const state = () => toPage({ type: 'state', view: view() });

  function hub(msg) {
    const byIds = (ids) => ITEMS.filter((it) => ids.includes(it.id));
    switch (msg.type) {
      case 'host.ready':
        toPage({ type: 'open', target: { hwnd: 1, focus: 2, caret: null }, view: view() });
        setTimeout(() => { toPage({ type: 'host.shown' }); scenario(); }, 30);
        break;
      case 'host.close': toPage({ type: 'host.hidden' }); setTimeout(() => toPage({ type: 'host.shown' }), 600); break;
      case 'host.resize': document.documentElement.dataset.size = `${msg.w}x${msg.h}`; break;
      case 'host.drag': break;
      case 'query': Object.assign(Q, { query: msg.query, collection: msg.collection, filter: { ...msg.filter } }); state(); break;
      case 'layout': layout = msg.layout; state(); break;
      case 'paste': case 'copy': break;
      case 'stack': stackState = { active: true, remaining: msg.ids.length }; state(); break;
      case 'pin': case 'unpin': byIds([msg.id]).forEach((it) => { it.pinned = msg.type === 'pin'; }); state(); break;
      case 'delete':
        undoStack.push(ITEMS.slice());
        ITEMS = ITEMS.filter((it) => !msg.ids.includes(it.id));
        undoFlag = true; state();
        setTimeout(() => { undoFlag = false; state(); }, 5000);
        break;
      case 'undo': if (undoStack.length) ITEMS = undoStack.pop(); undoFlag = false; state(); break;
      case 'collect': byIds(msg.ids).forEach((it) => { if (!it.collections.includes(msg.collection)) it.collections.push(msg.collection); }); state(); break;
      case 'uncollect': byIds(msg.ids).forEach((it) => { it.collections = it.collections.filter((c) => c !== msg.collection); }); state(); break;
      case 'collection.create': COLLECTIONS.push({ id: 'c' + cid++, name: msg.name, color: msg.color || null, builtin: false }); state(); break;
      case 'collection.rename': COLLECTIONS.forEach((c) => { if (c.id === msg.id) c.name = msg.name; }); state(); break;
      case 'collection.color': COLLECTIONS.forEach((c) => { if (c.id === msg.id) c.color = msg.color; }); state(); break;
      case 'collection.delete':
        COLLECTIONS = COLLECTIONS.filter((c) => c.id !== msg.id);
        ITEMS.forEach((it) => { it.collections = it.collections.filter((c) => c !== msg.id); });
        if (Q.collection === msg.id) Q.collection = 'history';
        state(); break;
      case 'collection.reorder': {
        const map = new Map(COLLECTIONS.map((c) => [c.id, c]));
        COLLECTIONS = [...COLLECTIONS.filter((c) => c.builtin), ...msg.ids.map((id) => map.get(id))];
        state(); break;
      }
      case 'edit': byIds([msg.id]).forEach((it) => { it.text = msg.text; FULL.delete(it.id); }); state(); break;
      case 'preview': {
        const it = ITEMS.find((x) => x.id === msg.id);
        if (!it) break;
        const text = it.kind === 'image' ? (it.ocr ? it.text : null) : FULL.get(it.id) || it.text;
        setTimeout(() => toPage({ type: 'preview', id: it.id, text: text || null, image: it.kind === 'image' ? imgId(it.image) : null }), 15);
        break;
      }
      case 'ocr': case 'export': break;
      case 'pause': paused = true; state(); break;
      case 'resume': paused = false; state(); break;
      default: break;
    }
  }

  window.chrome = window.chrome || {};
  window.chrome.webview = {
    postMessage(msg) { log('→', msg); SENT.push(msg); setTimeout(() => hub(msg), 5); },
    addEventListener(type, fn) { if (type === 'message') listeners.push(fn); },
    removeEventListener(type, fn) { const i = listeners.indexOf(fn); if (i >= 0) listeners.splice(i, 1); },
  };

  window.ReuiDev = {
    still: flag('still'), // screenshots: no motion
    image: (id) => IMAGES[id] || svg(64, 64, '<rect width="64" height="64" fill="#333"/>'),
    ready(api) { window.ReuiDev.api = api; },
    // Simulate a copy arriving while the window is open (W-10).
    copy(text) { ITEMS.unshift(mk({ exe: 'chrome.exe', age: 'now', text: text || 'Fresh copy ' + new Date().toLocaleTimeString() })); state(); },
  };

  /* ---------------- Self-test (?test=1): drives keys, checks outgoing messages ---------------- */

  async function runTests() {
    const wait = (ms) => new Promise((r) => setTimeout(r, ms));
    const results = [];
    const since = () => { const n = SENT.length; return () => SENT.slice(n).filter((m) => m.type !== 'preview' && m.type !== 'host.resize'); };
    async function t(name, act, check, ms = 120) {
      const got = since();
      await act();
      await wait(ms);
      const msgs = got();
      let ok = false;
      try { ok = !!check(msgs); } catch (_) { ok = false; }
      results.push(`${ok ? 'PASS' : 'FAIL'} ${name} ${ok ? '' : JSON.stringify(msgs)}`);
    }
    const first = () => view().items[0].id;
    const q = document.getElementById('q');
    const type = (v) => { q.value = v; q.dispatchEvent(new Event('input')); };
    await t('Enter pastes the selection', () => key('Enter'), (m) => m[0].type === 'paste' && m[0].ids[0] === first() && m[0].mode === 'normal');
    await t('Shift+Enter pastes plain', () => key('Enter', { shiftKey: true }), (m) => m[0].mode === 'plain');
    await t('Ctrl+Enter copies', () => key('Enter', { ctrlKey: true }), (m) => m[0].type === 'copy');
    await t('Ctrl+3 pastes the third', () => key('3', { ctrlKey: true }), (m) => m[0].ids[0] === view().items[2].id);
    await t('Right then Enter pastes the second', () => { key('ArrowRight'); key('Enter'); }, (m) => m[0].ids[0] === view().items[1].id);
    await t('Shift+Right x2, Enter pastes three', () => { key('Home'); key('ArrowRight', { shiftKey: true }); key('ArrowRight', { shiftKey: true }); key('Enter'); }, (m) => m[0].ids.length === 3);
    await t('Ctrl+Shift+V starts a stack', () => key('V', { ctrlKey: true, shiftKey: true }), (m) => m[0].type === 'stack' && m[0].ids.length === 3);
    await t('Esc collapses multi-select', () => { key('Escape'); key('Enter'); }, (m) => m[0].type === 'paste' && m[0].ids.length === 1);
    await t('Typing queries (debounced once)', () => { q.focus(); type('r'); type('ru'); type('rust'); }, (m) => m.length === 1 && m[0].type === 'query' && m[0].query === 'rust' && m[0].collection === 'history', 200);
    await t('Space types into a non-empty search', () => key(' '), (m) => m.length === 0 && !document.querySelector('.win.ov'));
    await t('Delete with text in search does not delete', () => key('Delete'), (m) => m.length === 0);
    await t('Esc clears the search', () => key('Escape'), (m) => m[0].type === 'query' && m[0].query === '');
    await t('Space opens the preview', () => key(' '), () => document.querySelector('.win.ov'));
    await t('Space closes the preview', () => key(' '), () => !document.querySelector('.win.ov'));
    await t('Ctrl+P pins', () => key('p', { ctrlKey: true }), (m) => m[0].type === 'pin' || m[0].type === 'unpin');
    await t('Ctrl+Tab next collection', () => key('Tab', { ctrlKey: true }), (m) => m[0].type === 'query' && m[0].collection === 'pinned');
    await t('Ctrl+Shift+Tab back', () => key('Tab', { ctrlKey: true, shiftKey: true }), (m) => m[0].collection === 'history');
    await t('Ctrl+K opens actions', () => key('k', { ctrlKey: true }), () => document.querySelector('.menu.palette'));
    await t('Palette filter + Enter runs', async () => { const f = document.querySelector('.mf'); f.value = 'upper'; f.dispatchEvent(new Event('input')); await wait(10); key('Enter'); }, (m) => m[0].type === 'paste' && m[0].mode === 'upper' && !document.querySelector('.menu'));
    await t('Palette submenu: add to collection', async () => { key('k', { ctrlKey: true }); const f = document.querySelector('.mf'); f.value = 'prompts'; f.dispatchEvent(new Event('input')); await wait(10); key('Enter'); }, (m) => m[0].type === 'collect' && m[0].collection === 'c7');
    await t('Delete deletes, Ctrl+Z undoes', async () => { key('Delete'); await wait(60); key('z', { ctrlKey: true }); }, (m) => m[0].type === 'delete' && m.some((x) => x.type === 'undo'));
    await t('Tab moves focus to cards', () => key('Tab'), () => document.activeElement.id === 'cards');
    await t('Typing returns to search', () => key('a'), () => document.activeElement.id === 'q');
    await t('Layout toggle', () => document.getElementById('layoutBtn').click(), (m) => m[0].type === 'layout' && m[0].layout === 'grid' && SENT.some((x) => x.type === 'host.resize' && x.h > 430));
    await t('Esc on empty search closes', () => { document.getElementById('layoutBtn').click(); key('Escape'); }, (m) => m.some((x) => x.type === 'host.close'));
    results.push('resizes: ' + JSON.stringify(SENT.filter((x) => x.type === 'host.resize').map((x) => x.h)));
    const pre = document.createElement('pre');
    pre.id = 'testout';
    pre.textContent = results.join(String.fromCharCode(10));
    document.body.appendChild(pre);
  }

  /* ---------------- Scenario switches (for screenshots) ---------------- */

  const key = (k, o = {}) => document.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...o }));
  function scenario() {
    const after = (ms, fn) => setTimeout(fn, ms);
    if (flag('noresults')) after(40, () => { const q = document.getElementById('q'); q.value = 'zebra'; q.dispatchEvent(new Event('input')); });
    if (params.get('sel')) after(60, () => { for (let i = 1; i < +params.get('sel'); i++) key('ArrowRight'); });
    if (flag('multi')) after(80, () => { key('ArrowRight', { shiftKey: true }); key('ArrowRight', { shiftKey: true }); });
    if (flag('stack')) after(80, () => { key('ArrowRight', { shiftKey: true }); key('ArrowRight', { shiftKey: true }); key('ArrowRight', { shiftKey: true }); key('V', { ctrlKey: true, shiftKey: true }); });
    if (flag('undo')) after(80, () => key('Delete'));
    if (flag('preview')) after(150, () => key(' '));
    if (flag('palette')) after(150, () => key('k', { ctrlKey: true }));
    if (flag('edit')) after(150, () => { key('k', { ctrlKey: true }); setTimeout(() => { const f = document.querySelector('.mf'); f.value = 'edit'; f.dispatchEvent(new Event('input')); key('Enter'); }, 30); });
    if (params.get('menu') === 'type') after(150, () => document.getElementById('typeChip').click());
    if (params.get('menu') === 'app') after(150, () => document.getElementById('appChip').click());
    if (params.get('menu') === 'tab') after(150, () => {
      const t = document.querySelector('.tab[data-id="c8"]'); const r = t.getBoundingClientRect();
      t.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: r.left + 20, clientY: r.bottom + 4 }));
    });
    if (flag('newtab')) after(150, () => document.querySelector('.tab.add').click());
    if (flag('test')) after(100, runTests);
    if (flag('newclip')) after(400, () => window.ReuiDev.copy('Copied while the window was open'));
  }
})();
