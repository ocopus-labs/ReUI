# PRD: The ReUI website

Status: in progress (M1 built, local only) · Owner: Rohit · Oct 30, 2026

**Inputs:** the README (every claim the site makes comes from it), `DESIGN.md` and `design/tokens.json` (the island's geometry, colours and springs), `docs/PRD-appearance.md` (Island and Notch styles), the island pictures Settings' shot mode draws (`reui-settings --shots --stages`), and the reference the owner picked: Supaste's landing page (a Mac clipboard app), for its polish rather than its layout.

**Where it lives:** `site/` (Astro, static). `npm run dev` in `site/`, http://localhost:4321. Not deployed; hosting is open (§13).

## 1. Problem

ReUI is about to go public, and the only page about it is the GitHub README: a long list of features with small pictures on a grey backdrop. Nobody understands a dynamic island from a list; they understand it the moment they see one move. A first draft of the site in the README's shape (headline, then a screenshot and a paragraph per feature) read as generic AI output and was thrown away.

## 2. Goals

1. **Visitors use ReUI before they download it.** The page has its own working island, built from ReUI's tokens and springs, and every section makes it do something real. No screenshots of the island anywhere.
2. **Award-grade craft.** Every section is designed on its own, built, reviewed by the owner, then the next. Big type, a real photograph, few words, motion that means something.
3. **Honest.** Every claim is in the README. No testimonials, user counts or ratings until real ones exist. The island on the page behaves as the app does (click-through, peeks, cards), and where it can't (no Win+Alt+V in a browser) the page says what the shortcut does instead of faking it.
4. **One action.** Download for Windows. GitHub is the only other destination.
5. **Fast and calm.** It looks finished before any script runs, costs nothing when nothing moves (no animation loop at rest, like the app), and works on a phone even though the product is for desktops.

## 3. Non-goals

- Docs, a blog or a changelog on the site (they stay on GitHub: `docs/guide.md`, `CHANGELOG.md`).
- Pricing, accounts, newsletters, analytics or cookies.
- A dark mode for the page (the hero photograph and the black island set the look).
- Pictures of other people's apps or brands (logos, real product screenshots). Mocked browser tabs use neutral names and letter tiles.

## 4. Audience

People on Windows who saw the Dynamic Island on an iPhone or a Mac notch app and want it, and developers running Claude Code, Codex or OpenCode who want to answer them without switching windows. Most arrive on a desktop browser from a link (GitHub, Reddit, X, Hacker News); some on a phone, who should still get it and come back on a PC.

## 5. Creative direction

### 5.1 The concept: the page is the screen

The browser window stands in for a Windows screen, and the page's island sits at its top centre exactly where ReUI's sits on a PC: fixed, 11 px from the top, at real size. It is never a picture. Scrolling drives it: sections ask it for their moment, the way modules ask the hub. The visitor learns the product by watching it and touching it, and the copy only names what they just saw.

### 5.2 Rules

1. **Show, then say.** Each section has one live moment on the island and at most one headline, one sentence and one detail line.
2. **One idea per screen.** A section that needs two ideas is two sections.
3. **Real, not mock.** The island uses the app's sizes, colours, corner maths and spring presets. Copy comes from the README. The photo is a photograph-grade image we're licensed to use.
4. **The island is the darkest thing on the page.** Everything else is light and quiet so the black shape reads, as it does on a bright wallpaper.
5. **No AI-slop tells:** no glows, gradient blobs, floating 3D mockups, emoji, "unlock / supercharge / seamless", three equal feature cards as a layout crutch, centred-everything, fake logos bars, invented numbers. (The owner's standing rule for ReUI's visuals applies here.)
6. **Motion explains state.** It answers a scroll or a pointer and then stops. Nothing loops forever except the island's own level bars while music "plays", which the app does too.

### 5.3 Voice

ReUI's own voice: plain, short, concrete, British spelling where the README uses it ("colours"). Headlines are a plain statement plus an accent phrase set in the serif italic ("Your agents ask. *You answer.*"). No exclamation marks, no "click to", no hotkey hints in running text other than the shortcut that *is* the feature (Win+Alt+V, Win+Ctrl+T, Win+Alt+S).

### 5.4 Visual system

| Element | Choice |
| --- | --- |
| Display and text type | Geist (variable, self-hosted via Fontsource), weight 600 for headlines, tracking −0.045 to −0.065 em at display sizes. Headlines say plainly what happens; no "Statement. *Accent.*" slogans |
| Accent type | Instrument Serif italic, only for "Windows" in the hero |
| Mono | Geist Mono for eyebrows, indexes and hosts |
| Island type | Segoe UI Variable, as in the app (falls back to the system UI font off Windows) |
| Page colour | `#efeee9` (warm paper); ink `#0e0e10`, secondary `#57575c`, tertiary `#8d8c88`, hairlines at 12 % ink |
| Accent colour | None on the page. The island carries the colour (each moment's accent from `tokens.json`); ReUI blue only for focus rings |
| Photography | The hero's moss-and-sky image (Super Visuals, CC BY 4.0: credit required, §9). Large calm sky at the top for the island, texture at the bottom |
| Texture | A still film grain over the whole page at 11 % (an SVG noise tile, painted once) |
| Layout | Full-bleed sections; content edges at `clamp(20px, 3vw, 40px)`; the centre column stays clear under the island |
| Surfaces | Sections after the hero are sheets: page colour, 28 px top corners, sliding over the pinned hero |

### 5.5 Motion

| Motion | Spec |
| --- | --- |
| Island size | ReUI's springs (`motion.spring.expand / collapse / peek`), damping ratio and period from the tokens; one time scale (`TIME` in `src/scripts/spring.ts`) tuned by eye against the app |
| Island content | The outgoing layer fades in 120 ms; the incoming one fades in 240 ms after 80 ms and settles from 0.92 scale, as `motion.crossfade` does |
| Second activity | The bubble pops from 0.6 scale (`motion.show.from-scale`) 11 px beside the pill |
| Hero entrance | Photo settles from 1.06 scale over 2.4 s; the thread draws down; the two words rise out of masks, 0.13 s apart; the text row fades in last |
| Thread pulse | A short light runs down the hairline each time the island changes, while the hero is on screen |
| Section reveals | Captions rise 24 px and fade in, 0.6 s, `cubic-bezier(0.16, 1, 0.3, 1)` |
| Reduced motion | Springs use `motion.spring.reduced`, entrances and pulses are off, bars stand still |

## 6. The page's island

`src/components/Island.astro` (markup, styles, behaviour) and `src/scripts/spring.ts`.

### 6.1 Moments

Each moment is a layer over one black shape that springs to the moment's size. Sizes are the app's, measured from its own renders.

| Moment | Kind | Size (px) | Opens |
| --- | --- | --- | --- |
| `idle` | Resting pill | 120 × 36.67 | — |
| `music` | Compact: art circle, level bars; timer bubble beside it | 116 × 36.67 | `now-playing` |
| `copied` | Compact, joined: clipboard glyph + "Copied" | 120 × 36.67 | `clipboard` |
| `volume` | Level: fill to 56 %, value at the end | 200 × 36.67 | — (a level has no card) |
| `timer` | Compact: "3:42" in orange, ring at the end | 120 × 36.67 | `timer-card` |
| `agent` | Compact, joined: Claude's mark + "Allow?" | 120 × 36.67 | `permission` |
| `allowed`, `denied` | Compact, joined: the answer, then the loop goes on | 120 × 36.67 | — |
| `launcher` | Card: the timer field (preview, field, presets), filled by the Timers section | 420 × 200 | — |
| `countdown`, `bell` | Compact: a running timer's time and ring; a bell and a word | 120 × 36.67 | — |
| `alert` | Card: bell, what, when, two answers (a finished timer, a due reminder) | 420 × 156 | — |
| `now-playing` | Card: art tile, title, timeline, transport, volume | 420 × 217 | folds to `music` |
| `clipboard` | Wide card: search, History / Pinned, a row of clips | 560 × 240 | folds to `copied` |
| `timer-card` | Card: ring, 34 pt numerals, pause and close | 420 × 118 | folds to `timer` |
| `permission` | Roomy card: session line, `npm test`, Deny / Always / Allow | 480 × 200 | folds to `agent` |

Also: `earbuds`, `call`, `mic`, `download`, `activity`, `activity-done`, `meeting`, `stats`, `shelf-drop`, `shelf` (compact, section 6; a joined label's width is measured from its content, never under 120). To add: `clock` (the idle clock).

### 6.2 Behaviour

- **The loop.** Until something takes over, the island runs a morning: music 3.4 s, copied 2 s, volume 2 s, timer 2.6 s, agent 2.8 s.
- **Peek and open.** A 250 ms rest on the pill peeks it (6 px larger, `peek` spring). A click opens the moment's own card; a click on a card, or leaving it for 450 ms, folds it back. Volume and idle don't open.
- **Answers.** Deny, Always and Allow on the permission card work: "Allowed" or "Denied", then the loop resumes.
- **Copying.** Copying anything on the page (Ctrl+C) shows "Copied", as ReUI does.
- **Hiding** (`visible(on)`): a hide-until-needed mode; the shape shrinks to `motion.show.from-scale` and fades. **No click-through:** a click on the pill is always the island's in the app, so the site never shows one passing through it (`passThrough` exists but no section uses it).
- **Driving** (`drive(moment, owner)`): a section takes the island and shows its moment; the loop stops until that same section lets go. Sections never release each other's hold.
- **Staging** (`stage(top, scale)`): moves the whole island off the edge and scales it, for the close-up. Content stays live (clickable) at any scale.
- **Events.** Every change dispatches `island:change` (the hero's thread listens); a card's own buttons (`data-action`) dispatch `island:action` for the section driving it.
- **Filling** (`fill(moment, fields)`): a section writes a card's text (`data-field`), a ring's progress (`arc`) and the field's query.
- **Places** (DESIGN.md "Idle", PRD-view-switcher.md): clicking the resting pill opens the row of places (Clipboard, Shelf, Timers, Calendar, Devices, System stats, Now Playing; the app's order and glyphs), a place opens its card, and under any open place card the view switcher (the same glyphs in a capsule, the open one filled) moves between them. A card folds back to wherever it was opened from; a switcher that slides out from under the pointer (a shorter card) doesn't count as leaving.
- **Icons and logos** (`src/scripts/icons.ts`): Fluent UI System Icons (MIT) for the app's Segoe Fluent glyphs; the agents' logos and ReUI's device drawings straight from `design/assets`; Simple Icons (CC0) for third-party brands (Zoom, Discord, Webex, WhatsApp, Telegram, Signal, Spotify, Chrome, Firefox, Brave, Opera; Microsoft's marks aren't in it).
- **Now Playing** is a real track: Blinding Lights, The Weeknd (After Hours, 3:20) in Spotify, with its cover from Apple's iTunes Search API (`src/assets/now-playing.jpg`): the label's copyright, to settle before launch (§13).
- **Review hook.** `#island=<moment>` holds one moment with the loop off, for screenshots and review.

### 6.3 Notch style

ReUI has two styles (`docs/PRD-appearance.md`): **Island** floats 11 px under the edge; **Notch** is flush with the top edge, square top corners joined to the edge by two concave ears, rounded bottom corners, the second activity inside the notch's trailing end instead of a bubble. The page's island gets the same switch (`style('island' | 'notch')`): top 0, top radius 0, ears drawn as two small masked squares at the top corners (radius `notch.ear`), bubble folded inside. The change is animated: the shape rises to the edge as the ears grow in. The Notch section (§7, section 4) owns it; everywhere else the page's island is the Island style. Themes switch the same way (`theme('dark' | 'light')`, colours from `theme.light`); in the light theme the section's page goes dark so the white shape reads. `stage(drop, scale)` measures from wherever the island rests, so a close-up works in either style.

## 7. Sections

The storyboard. Each is built, shown to the owner, adjusted, then the next starts.

| # | Section | Live moment | Copy | Status |
| --- | --- | --- | --- | --- |
| 1 | **Hero** | The loop on the photo's sky (it includes the resting pill, which opens the row of places) | "The Dynamic Island for *Windows*" · one plain sentence · Download for Windows · "Free and open source · Windows 10 and 11" | Built |
| 2 | **Close-up** | Scrolling pins a stage; the island leaves the edge and grows (up to 2×, fitted to the widest card) and plays three cards, one per screen; back to the edge at the end | 01 "What's playing, *one glance away.*" · 02 "Everything you copied. *Win+Alt+V.*" · 03 "Type *10m tea.*" (agents moved to their own section) | Built |
| 2b | **Coding agents** | A light Windows Terminal running Claude Code pins to the top of the screen, the page's island over its title bar. Scrolling plays one session in the app's own wording: permission for `npm test` (Allow), a failing test and a question with two choices (pick, Send), a plan (Approve), the fix and 42 passing, then Claude's mark and a check. A cursor answers unless the visitor does; answering on the island moves the scroll to the next beat | "Your agents ask. *You answer.*" · one line per beat · secrets as dots, never takes the keyboard | Built |
| 3 | **Out of your way** | Magnified: the camera zooms into the top centre of a browser (tab strip at ~2.4×) and the page's island moves down and scales to sit on it. Scrolling plays, one line per beat: hidden, the cursor clicks the middle tab; a decision ("Allow?") comes down; pushing the edge above the pill brings it back; pushing the edge over another tab does nothing, and the tab takes the click. Never claims a click goes through the pill (it doesn't in the app) | "Your tabs *stay yours.*" · "Keep it hidden. *Until something needs you.*" · "Something needs you. *It comes down.*" · "Push against the edge above it. *It's back.*" · "The rest of the edge *stays your tabs'.*" | Built |
| 4 | **Island or notch** | Scrolling turns the page's island into a notch (rises to the edge, ears grow) and back; two small switches let the visitor flip Island / Notch and Dark / Light themselves | "Float it, or *join the edge.*" · Dark, light or Windows' mode; roundness, size and accent are yours | Built |
| 5 | **Timers and reminders** | A real field on the page reads a line as ReUI's timer field does (`src/scripts/timefield.ts`, a subset of the app's grammar, same previews). While it's on screen the island is the field's card (Timer/Reminder preview, the field, 5/10/25 min presets). Enter: a timer counts down in the pill (60× fast, said under the field) and ends with **+1 min** / **Stop**; a reminder shows the card it opens when due, **Snooze** / **Done**. Examples type themselves in | "Call mom *at 6.*" · one field for timers, focus, stopwatch and reminders, previewed before Enter | Built |
| 6 | **The rest of your day** | An editorial list, one line per moment; resting on (or focusing) a line plays it on the island, worded and coloured as the app's modules do: earbuds (battery ring), a Teams call (clock counting up), the mic in use (app + orange mic), a download (green ring filling, size · name), a build (blue ring to 100 %, then Done), the next meeting (red ring, Beta), stats (teal ring, CPU %). The Shelf line holds a file to drag to the top of the window: within `shelf.reach` the island becomes "Drop to hold"; dropped there, the shelf holds it (count) | "And the rest *of your day.*" · one line each, from the README | Built |
| 7 | **Idle costs nothing** | The island goes back to its resting pill; the README's numbers set large in three columns (CPU at rest counts down to 0.00 % once, 2 MB hub, 0.05 % with music) with the measurement's conditions | "Idle costs *nothing.*" | Built |
| 8 | **Private by default** | — | "Your PC. *Your data.*" · no account, no telemetry; the four times ReUI goes online as a settings-like list (what it sends, on/off) | Built |
| 9 | **Download** | The hero's photograph again as a bookend; resting on (or focusing) the button plays a download arriving on the island (ReUI-Setup.exe, green ring) | "Free. *Open source.*" · MIT or Apache 2.0 · requirements · the SmartScreen note | Built |
| 10 | **Questions** | — | "Before you *install.*" · eight answers; the tabs answer is about hiding and the reveal zone, never clicks going through | Built |
| 11 | **Footer** | — | Links, the CC BY 4.0 credit (Super Visuals, linked; the author's name to add), the giant wordmark | Built |

## 8. Content rules

- Facts come from `README.md`; when the README changes, the site follows (and the other way round).
- Shortcuts are the ones in `docs/guide.md`.
- Numbers come from the README's Performance table, with its conditions.
- Anything not shipped is not on the page, except Calendar, marked Beta as in the app.
- Download links go to `github.com/ocopus-labs/ReUI/releases/latest`; they work only once the repository is public.

## 9. Assets and credits

| Asset | Source | Licence / note |
| --- | --- | --- |
| Hero photograph (`site/src/assets/hero.jpg`, 3840 px) | "Super Visuals: 50+ stunning backgrounds v1", Figma Community (`AI_Bg_016`) | CC BY 4.0: credit the author in the footer (name to confirm, §13) |
| Logo and favicons | `design/logo/` | Ours |
| Island moments | Drawn in HTML/CSS from `tokens.json`; transparent renders of the app's stages (`npm run images`) for checking sizes | Ours |
| Fonts | Geist, Geist Mono (OFL), Instrument Serif (OFL), via Fontsource | Self-hosted |
| Other backgrounds | The owner may pick more from the same library (one per later section at most); same credit | CC BY 4.0 |

## 10. Technical

- **Stack:** Astro (static output), scaffolded with `npm create astro`; no UI framework. Sections are `src/sections/*.astro`, each with its own scoped styles and a small script; the page composes them in `src/pages/index.astro`.
- **Smooth scrolling:** Lenis (`lenis`, set up once in `Base.astro`, off with reduced motion). The scenes read the real scroll position, which Lenis moves, and the agents section's own jumps go through `lenis.scrollTo`. Lenis runs a frame loop while the page is open; it is the one exception to the next rule.
- **No animation loop at rest.** The island's springs run on `requestAnimationFrame` only while moving and stop when settled; scroll handlers are rAF-throttled and passive; the grain is a still image.
- **Budgets:** page JS under 30 KB gzipped; hero image under 400 KB at 3840 px (AVIF/WebP from Astro's image service at 1280–3840); LCP under 2 s on a fast 4G profile; no layout shift from fonts (`font-display: swap` with metric-matched fallbacks).
- **Accessibility:** the island is `role="img"` with a label, and its buttons are real buttons; every interactive demo also works by keyboard (the browser tabs, the timer field, the switches); `prefers-reduced-motion` honoured (§5.5); contrast AA for all text, including white text on the photo (a scrim behind it).
- **Mobile:** sections stack; the close-up scales to the screen width; the island stays real size at the top.
- **Review tooling:** `node scripts/shoot.mjs <dir> "path#hash|WxH|waitMs|scrollScreens" …` screenshots the running dev site with the installed Chrome, scrolling in steps so scroll scenes play.
- **SEO and sharing:** title, description, Open Graph image (a still of the hero), `og:url` once hosted.

## 11. Edge cases

- **No JavaScript:** the hero, all copy and the download button render; the island shows its resting pill; scroll sections show their captions in order without pinning.
- **Very wide or short screens:** the close-up's scale is the smallest of 2×, 86 % of the width over the widest card, and 42 % of the height over the tallest.
- **Tab hidden:** the loop stops while the page isn't visible.
- **Two sections overlapping on fast scroll:** each drives the island under its own name; letting go only clears its own hold.

## 12. Milestones

| # | Scope | Done when |
| --- | --- | --- |
| M1 | Hero (photo, "Look up.", thread), the page's island (moments, springs, cards, answers, copy, click-through), close-up, Out of your way | Owner approved: built |
| M2 | Notch style on the page's island; sections 4 (Island or notch) and 5 (Timers) | Each approved by the owner in the browser |
| M3 | Sections 6–11 | Each approved; README and site say the same things |
| M4 | Polish: OG image, the credit, performance budget checked (Lighthouse), keyboard and screen-reader pass, phone pass, no-JS pass | All budgets met; owner sign-off |
| M5 | Hosting and launch with the public repository | Live URL; download links work |

## 13. Open questions

- **The Now Playing cover** is After Hours' artwork (the label's copyright): keep it, swap for a track whose artwork we may use, or our own.
- **The README's click-through claim** ("Clicks pass through … until you pause on the island") doesn't hold in the app: fix the app or the wording.
- **Hosting and domain:** Cloudflare Pages, GitHub Pages once public, or elsewhere; and a domain.
- **The photograph's author** for the CC BY credit.
- **The island's drawing:** HTML/CSS today (its glyphs are simpler than the app's Segoe Fluent ones). Exporting the app's glyphs as SVG would make it exact.
