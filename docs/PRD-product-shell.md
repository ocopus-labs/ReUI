# PRD: Product shell (install, first run, settings, trust)

Status: decisions accepted Oct 28 with D1 changed: **distribution from GitHub releases** (per-user setup exe + zip, built by GitHub Actions; unsigned at first), not the Store · Owner: Rohit · Oct 28, 2026 · Before feature #7

**Persona:** knows what a dynamic island is (iPhone, or Mac notch apps like NotchNook, Alcove, boring.notch) but has never heard of ReUI. They find it, install it, and decide within a minute whether to keep it.

**Inputs:**
- A first-time-user audit of ReUI as it is today.
- A study of how comparable apps onboard and why people keep or uninstall them:
  - Mac: boring.notch (onboarding code and issues), Atoll, NotchNook, Alcove, DynamicLake, MediaMate, Supaste, Warble.
  - Windows: FluentFlyout, ModernFlyouts, DynamicWin and other Windows islands (issues), PowerToys, Raycast, EarTrumpet, TranslucentTB.
  - Microsoft's code-signing, tray and Store-policy docs.
- Sources are in the appendix.

## 1. Problem

ReUI's features work, but nobody could use them:

| Today | What the persona hits |
|---|---|
| A `cargo build` in a dev folder | Nothing to download; an unsigned exe would show "Windows protected your PC / Unknown publisher" |
| No installer, Start entry, auto-start or uninstall | "How do I start it again tomorrow?" |
| First launch shows nothing if nothing is playing | "Did it even start?" (DynamicWin's top issues: "installed but nothing but a tray icon", "doesn't open") |
| Hotkeys (Win+Alt+V, Ctrl+Alt+1–9), hover, scroll, swipe and right-click are documented only in the README | Most features are never found |
| Settings live in five JSON files plus a tray menu of ~35 "X: on / X: off" items that can't show their state | No way to change anything comfortably |
| Coding agents need `reui-bridge install claude` in a terminal | The flagship integration is invisible |
| Clipboard history, network lookups (lyrics, link previews) and the volume-flyout hack are explained only in PRDs | Trust questions are unanswered |

## 2. Goals

1. **Installed and visibly working in 60 seconds:** signed, per-user, no admin, starts with Windows.
2. **Teaches itself:** a short demo on first launch, one welcome window, and one-time tips at the moment each feature matters.
3. **One place for every setting:** a real Settings window, Windows-native in look and structure (General + one page per feature), with JSON only as storage.
4. **Earns trust:** every network call, sensitive feature and system hack is an explicit, honest opt-in, and there's a plain privacy statement.
5. **Never the reason to uninstall:** never covers tabs or fullscreen, shows its idle cost, updates cleanly, and uninstalls cleanly.

## 3. Non-goals (now)

- A paid tier.
- Cloud accounts or sync.
- Themes and skins beyond light/dark/system for the Settings window. The island keeps its own black look.
- Mac or Linux.

## 4. The journey

### First 60 seconds

1. **Get it:** from the Microsoft Store or winget (*D1*). It installs per-user in seconds with no admin, and launches at the end.
2. **The island introduces itself:**
   - It animates in at the top centre and runs a **5-second demo**: a sample "Now Playing" card expands, then collapses.
   - Then a one-time hint inside the pill: "Hover me". This is the only kind of text in the island that teaches, and it shows once.
3. **One welcome window, three steps at most, shown once ever** (boring.notch's bug was a welcome on every launch):
   1. **What ReUI shows:** one card per feature with a toggle.
      - On by default: Now Playing, Devices, Clipboard, Volume pop-up.
      - Off by default: replacing Windows' volume pop-up, lyrics (network), AirPods detail (nearby scan), coding agents.
   2. **Your clipboard hotkey:** shows Win+Alt+V; press keys to change it; conflicts are checked.
   3. **Start with Windows** (on) → *Done* / *Open Settings*.
4. **No permission wall.** Everything else is asked when it becomes relevant.

### First day: teach in context, once each

| Moment | One-time tip or offer |
|---|---|
| First copy | The copy moment, plus "Win+Alt+V opens history" in the expanded toast (first 3 copies only, then never) |
| First volume key | The volume pop-up; if Windows' pop-up also showed: "Replace Windows' volume pop-up?" (*Yes* / *Not now*) |
| A Claude Code / Codex / OpenCode process first seen | "Show Claude Code in the island?" → *Connect* (shows exactly which file changes; reversible) |
| First music with lyrics available | "Show synced lyrics? Song titles are sent to lrclib.net" (*Turn on* / *No thanks*) |
| First hover that opens a card | Nothing; hover teaches itself |

Each offer appears **once** and is remembered. *Not now* means the feature waits in Settings.

### First week

- "What's new" after updates (can be turned off).
- The tray shows a **Pause ReUI for 1 hour** entry.
- Help → replay the tour.

## 5. Surfaces

### 5.1 Packaging, start and stop (*D1*, *D2*)

- **Distribution:**
  - ~~Microsoft Store (MSIX)~~ **GitHub releases** (decided Oct 28) as the main channel: a per-user `ReUI-Setup.exe` and a zip: no SmartScreen, free signing, background updates, clean uninstall.
  - **winget** for enthusiasts.
  - A **signed** installer on GitHub releases.
  - No "trust this certificate" steps, ever (the FluentFlyout/ModernFlyouts sideload pattern scares people).
- **Per-user, no admin.**
- **Start with Windows:** MSIX `windows.startupTask` (a `HKCU\…\Run` entry for the non-Store installer). The toggle shows the **real** state, including "disabled in Task Manager → turn it on there".
- **Single instance:** launching ReUI again (Start menu, desktop) opens Settings.
- **Quit** from the tray, and a hotkey to hide the island for an hour.

### 5.2 Settings window (*D3*)

- **Look:** Windows 11 conventions: Mica backdrop, left navigation, setting cards (icon, title, one-line description, control), a search box, light/dark/system.
- **Structure** (PowerToys' model; each feature page starts with its **Enable** toggle and a one-paragraph description):
  - **General:**
    - start with Windows;
    - island position: monitor (primary / where the mouse is / all);
    - visibility (Always / When active / When needed), hover delay, and "hide in fullscreen" (games, video, presentations);
    - reduced motion;
    - tray icon;
    - pause.
  - **Now Playing:** visualizer (real/simple/off), lyrics (with the network note), swipe to skip, retract after pause, show video sources.
  - **Clipboard:** hotkey (recorder + conflict check), recent-paste keys, retention, max items, ignored apps (picker), secret detection, link previews (network note), pause, clear history.
  - **Volume & brightness:** HUDs on/off, show %, replace Windows' pop-up (honest description, restored on quit), lock keys, battery saver, mic mute, monitors for brightness.
  - **Devices:** connect cards, low battery, charging, drives, phones, AirPods detail (nearby scan note), the device list with battery.
  - **Coding agents:** Claude Code / Codex / OpenCode, each with *Connect* / *Disconnect*, what file changes, status ("Connected · last event 2 min ago"), and a *Send test event* button; token usage on/off.
  - **Privacy & data:**
    - the plain statement: "Everything stays on this PC. ReUI connects to the internet only for: lyrics (lrclib.net) and link previews, both off/opt-in";
    - clipboard data location and size;
    - export / delete all data.
  - **About:** version, updates, "What's new", **idle cost** ("Idle: 14 MB · 0.0 % CPU", measured live), logs, *Report a bug* (prefilled GitHub issue with diagnostics, no personal data), licenses.
- **Storage:** the Settings window writes the same JSON files modules already read (one schema, documented), and modules reload on change (a file watcher or an IPC "settings changed" command).

### 5.3 Tray (short, Windows conventions)

- **Left-click:** Settings.
- **Right-click menu:** Settings · Pause ReUI for 1 hour / Resume · Clipboard history · Devices · Help · Quit.
- **The ~35 toggle items move into Settings.**
- New tray icons land in the overflow, so discovery never relies on the tray.

### 5.4 In-island tips

- One-time, contextual, short, dismissed by use.
- Tracked in `%APPDATA%\ReUI\tips.json`.
- Never permanent filler text (DESIGN's no-filler rule still holds: tips are first-time teaching, not decoration).

### 5.5 Updates, uninstall, help

- **Updates:**
  - Store/winget handle delivery.
  - The app shows "What's new" once after an update (can be turned off).
  - Settings survive updates (schema migrations).
- **Uninstall:**
  - Remove the startup entry.
  - Restore Windows' volume pop-up (crash-safe file already exists).
  - Remove agent hooks (the bridge's uninstall).
  - Offer to delete data (clipboard history, caches).
  - Store policy 10.2.7 requires a clean uninstall.
- **Help:** a short online guide (what each feature does, all shortcuts), replay the tour, report a bug.

### 5.6 Behaviour fixes the research flags as uninstall reasons

| Risk | Requirement | Today |
|---|---|---|
| Covers browser tabs / taskbar | The island takes clicks only on its visible shape after hover intent; no opening on drag-over (boring.notch #1530) | Hover intent exists; drag-over not handled |
| Shows over fullscreen | Hidden in fullscreen games, video and presentations by default (setting) | Hidden for fullscreen windows; verify games/video/presentation modes |
| Breaks after sleep / Explorer restart | Re-place and re-raise after resume, display changes and `TaskbarCreated` | Topmost re-asserted on foreground changes; resume path to verify |
| Idle cost doubts | Measured idle numbers in About | — |
| Multi-monitor | Choose the monitor; correct centring at any DPI | Primary only |

## 6. Edge cases

| ID | Case | Required behaviour |
|---|---|---|
| P-1 | Welcome dismissed with the window's ✕ | Counts as done (defaults kept); never shown again |
| P-2 | User disables the startup task in Task Manager | The toggle shows "Disabled in Task Manager" with a link |
| P-3 | Hotkey taken by another app | Welcome/Settings say so and suggest an alternative |
| P-4 | Settings JSON edited by hand while running | Reloaded; invalid → keep last good, show an error card in Settings |
| P-5 | Two ReUI versions (Store + installer) | The second instance opens the first's Settings and quits |
| P-6 | Uninstall while Windows' pop-up is hidden | Restored before removal |
| P-7 | Agent config file edited by the user later | Connect status re-checks; never overwrites their changes silently |
| P-8 | No internet | Nothing in the shell needs it (updates through the Store) |
| P-9 | Accessibility (screen reader, keyboard-only, high contrast) | Settings fully keyboard-operable and named; reduced motion respected everywhere |

## 7. Milestones

| M | Scope | Exit |
|---|---|---|
| **P0: Package & start** (*built Oct 28: per-user `ReUI-Setup.exe` + zip via GitHub Actions, HKCU Run, Start entry, Apps & features uninstall, single instance → Settings*) | MSIX packaging (Store-ready manifest, startupTask), signed GitHub build, single instance, Start entry → Settings, short tray menu, quit/pause | A friend installs from a link without warnings and it starts at login |
| **P1: Settings window** (*built Oct 28: custom native window in the island's design language, not Fluent; live reload; short tray*) | General + feature pages + Privacy + About (idle cost, logs, report bug); modules reload on change; tray toggles removed | Every JSON setting reachable; nothing requires editing files |
| **P2: First run** | 5-s demo, welcome (3 steps, once), one-time in-island tips, contextual offers (flyout, lyrics, agents) | A first-time tester understands hover, the clipboard hotkey and agents without the README |
| **P3: Agents in the GUI** | Connect/Disconnect per agent, file diff preview, status, test event | Claude Code connected with two clicks |
| **P4: Updates, uninstall, help** | "What's new", clean uninstall (hooks, startup, flyout, data), online guide, replay tour | Uninstall leaves nothing behind |
| **P5: Behaviour fixes** | Drag-over guard, fullscreen variants, resume/Explorer recovery, monitor choice | The section 5.6 table all green |

## 8. Open decisions

| ID | Question | Proposal |
|---|---|---|
| D1 | Main distribution | **Microsoft Store (MSIX)** first (free signing, no SmartScreen, updates), plus winget and a signed GitHub build. Note: Store policy 10.2.8 restricts "undocumented APIs" that change the Windows experience, so the Windows-pop-up replacement stays opt-in with explicit consent and may need to be left out of the Store build |
| D2 | Signing for the non-Store build | Azure Artifact Signing is limited to US/Canada individuals; otherwise an OV code-signing certificate (yearly cost). Until then, Store + winget only |
| D3 | How to build the Settings window | ~~WebView2 + Fluent~~ **Decided Oct 28: a custom native window drawn like the island** (Rohit: "use custom UI for settings and should match with how the bar looks"): fast to build to a native look, opened on demand, closed when done (the earlier rejection was about the *clipboard* leaving the notch, not WebView). Alternatives: a C# WinUI 3 settings app (most native, adds .NET), or hand-drawn Direct2D (slow to build) |
| D4 | Default visibility for new users | **When active** (shown while something is happening, hidden otherwise), with the first-run demo so it never looks broken |
| D5 | Coding agents in the welcome | Not in the welcome; offered when an agent is first seen running |
| D6 | One-time tips inside the island | Yes, at most once each, never permanent |
| D7 | Product name / icon / website | Needed for the Store; decide before P0 ships |

## Appendix: sources

- **Onboarding:**
  - [boring.notch](https://github.com/TheBoredTeam/boring.notch) (onboarding; issues [#285](https://github.com/TheBoredTeam/boring.notch/issues/285), #336, #417, #1260, #1278, #1530)
  - [Atoll](https://github.com/Ebullioscopic/Atoll)
  - [Warble first-run demo](https://github.com/LorcanChinnock/warble/pull/153)
  - [Alcove FAQ](https://tryalcove.com/faqs)
  - [DynamicLake review](https://thesweetbits.com/tools/dynamiclake-review/)
  - [MacStories on NotchNook & MediaMate](https://www.macstories.net/reviews/notchnook-and-mediamate-two-apps-to-add-a-dynamic-island-to-the-mac/)
  - [Supaste](https://www.supaste.com/updates/supaste-v1.4-is-here)
  - [Raycast quickstart](https://manual.raycast.com/quickstart)
  - [PowerToys welcome](https://winaero.com/powertoys-0-33-1-stable-is-out-along-with-a-new-welcome-screen/)
- **Windows islands and flyouts:**
  - [DynamicWin issues](https://github.com/FlorianButz/DynamicWin/issues)
  - [devcode90 #73](https://github.com/devcode90/Dynamic-Island-for-Windows/issues/73)
  - [FluentFlyout](https://github.com/unchihugo/FluentFlyout)
  - [ModernFlyouts](https://github.com/ModernFlyouts-Community/ModernFlyouts)
  - [MakeUseOf on Dynamic Edge](https://www.makeuseof.com/windows-11-dynamic-island-free/)
  - [NotchPixel guide](https://notchpixel.com/guides/dynamic-island-for-windows) (vendor)
- **Microsoft:**
  - [PowerToys General settings](https://learn.microsoft.com/en-us/windows/powertoys/general)
  - [Code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
  - [Notification area guidelines](https://learn.microsoft.com/en-us/windows/win32/uxguide/winenv-notification)
  - [StartupTaskState](https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.startuptaskstate)
  - [Store policies](https://learn.microsoft.com/en-us/windows/apps/publish/store-policies)
