# What to build next: competitor features and user demand

Research done Oct 30, 2026. The question was which features people actually need from a dynamic island, with evidence. This document has the competitors and what they ship, what users ask for, a ranked list of candidates, and a recommendation. The file shelf and calendar are already being built, so they are not treated as gaps.

**How it was done, and its limits**
- **GitHub:** issues were sorted by reactions and comments through the GitHub search API on Oct 30, 2026. Counts are as of that day. "12r 15c" means 12 reactions and 15 comments.
- **Reddit:** reddit.com blocks our crawler and its API, so no Reddit thread was read directly. Reddit demand appears here only where a press article or the external report (§6) cites it, and it is marked as such.
- **Store reviews:** Microsoft Store and Gumroad review text can't be read by machine. Ratings come from the vendors' own pages and the press. NotchNook, Alcove and DynamicLake are not on the Mac App Store.
- **Vendor bias:** many comparison pages are written by a competitor (Notchify, NotchPixel, Seam, Korinotch, Notchy). They are used for what an app ships, never as evidence of demand.
- **Hacker News:** HN Algolia finds only link posts about NotchNook and boring.notch, with 0–1 comments each. HN has no demand signal.

---

## 1. Competitors and what they ship

### Mac notch apps

| App | Price / licence | Flagship | Source |
|---|---|---|---|
| **boring.notch** | Free, GPL, 11.2k stars | Music with visualizer, calendar, shelf with AirDrop, HUD replacement, camera mirror. Its roadmap has reminders, weather, Bluetooth, lock-screen widgets, extensions, and "Notifications (under consideration)" | [GitHub](https://github.com/TheBoredTeam/boring.notch) |
| **NotchNook** | $15 lifetime or $2/month since its Oct 2026 relaunch (was $25 / $3 a month) | Music, calendar, notes, to-dos, timers, mirror, file tray with AirDrop, Shortcuts, weather, HUDs. No clipboard | [Seam roundup](https://getseam.app/blog/best-mac-notch-apps-2026), [MacStories](https://www.macstories.net/reviews/notchnook-and-mediamate-two-apps-to-add-a-dynamic-island-to-the-mac/), [FavTray](https://favtray.com/blog/notchnook-vs-boring-notch) |
| **Alcove** | $14.99 one-time | Polish: smooth motion, HUDs, live activities, swipe gestures, lock-screen widgets. Sources disagree on whether it mirrors notifications and has a file tray | [Theodore HQ](https://www.theodorehq.com/macdept/app/alcove), [Korinotch](https://korinotch.com/blog/alcove-mac-review-alternatives.html) |
| **Atoll** | Free, GPL, about 5.1k stars | The widest free set: media, privacy indicators, screen-recording and downloads (beta) live activities, battery, stats with temperatures, timers, clipboard, colour picker, calendar, shelf with AirDrop, a terminal tab, LLM usage tracking | [GitHub](https://github.com/Ebullioscopic/Atoll) |
| **DynamicLake Pro** | About $14–17 one-time | **Notifications** (Messages, WhatsApp, Telegram, Slack, Mail), incoming calls and meetings (FaceTime, Zoom, Meet, Teams), Bluetooth, HUDs, calendar with weather, drop with AirDrop, app switcher, timer | [iDownloadBlog](https://www.idownloadblog.com/2024/12/06/dynamiclake-pro/), [SweetBits](https://thesweetbits.com/tools/dynamiclake-review/), [Gumroad](https://avirok1.gumroad.com/l/nyyfw) |
| **MediaMate** | €6.99 (a limited free version exists) | HUD replacement (volume, brightness, keyboard backlight) and a Now Playing HUD, nothing else | [Gumroad](https://wouter01.gumroad.com/l/mediamate) |
| **NotchDrop** | Free, open source | A file shelf with AirDrop, nothing else | [GitHub](https://github.com/Lakr233/NotchDrop) |
| **Notchmeister / TopNotch** | Free | Cosmetic only: Notchmeister decorates the notch, TopNotch hides it. No tools | [MacRumors](https://www.macrumors.com/2021/12/17/notchmeister-decorate-macbook-pro-notch/), [OSXDaily](https://osxdaily.com/2022/07/17/hide-display-notch-macbook-pro-topnotch/) |

### Windows islands and flyouts

| App | Price / licence | Flagship | Source |
|---|---|---|---|
| **devcode90 "Dynamic Island for Windows"** | Free, MIT, 48 stars; a Windhawk mod | The longest free Windows list: media with lyrics, calendar, weather (wttr.in), game overlay (FPS, CPU, GPU), stats, focus timer, Quick Lookup (Ctrl+Alt+Space), file tray, mic and camera dots, Bluetooth battery, notifications, clipboard, DND awareness, lock keys, battery, USB, volume and brightness | [GitHub](https://github.com/devcode90/Dynamic-Island-for-Windows) |
| **Notchify** | Paid, about $2–2.50, Store trial; claims a 4.0 rating and 4,000+ downloads | Notifications grouped by chat, media with synced lyrics, clipboard with drag-out, file tray, screen recording, Bluetooth battery, 20+ widgets (weather, calendar, to-do, notes, Pomodoro, world clock, graphs), quick actions (lock, sleep, mute, theme, snip), app shortcuts, privacy dots. 5.8 added Discord controls, phone transfer and downloads | [notchify.app](https://notchify.app/en/), [Windows Central](https://www.windowscentral.com/software-apps/notchify-brings-macos-style-dynamic-island-flair-to-windows-11) |
| **Dynamic Edge** | Free, Plus about $4.99; WinUI 3; source not published ([#4](https://github.com/Eul45/Dynamic-Edge/issues/4)) | Island or side "edge dock", media with lyrics, categorised clipboard (offline), timers and stopwatch, three launcher slots, file tray (Oct 2026), Bluetooth battery, OCR, song recognition, a virtual pet. 79 upvotes on Product Hunt | [Store](https://apps.microsoft.com/detail/9pf1587ksn2k), [site](https://www.eyuel.com.et/dynamic-edge), [Product Hunt](https://www.producthunt.com/products/dynamic-edge) |
| **NotchPixel** | $9.99, 7-day trial | Gamers and streamers: call timer with a global mic-mute key, notifications, pop-ups, game stats, send files to your phone, clipboard, downloads, reminders, weather, clocks, to-do, focus, coding-agent status, screen recording, stream chat, custom scripts | [notchpixel.com](https://notchpixel.com/guides/dynamic-island-for-windows) (written by its own vendor) |
| **Smart Notch** | Free, with a Pro tier | Media with lyrics, HUDs, Bluetooth battery, focus timer and task list, privacy dots, night light, DND, memory optimizer | [Notchify's comparison](https://notchify.app/compare/) |
| **Halo DynamicWin** | Free, MIT | Media, notifications, file tray, download progress, Claude Code and Codex panels | same |
| **Lumen** (Risuleia) | Free, MIT, 35 stars; Rust + Slint | Minimal: media with visualizer, notifications, mic and camera, click-through when idle, hidden in fullscreen, auto-update | [GitHub](https://github.com/Risuleia/Lumen) |
| **PILLAR** (warpirate) | Free, MIT, 16 stars; Tauri + React | Media, focus timers, notifications with history, per-app mixer, output switcher, stats, tasks, notes, agenda, and "Prism", an AI assistant that runs on Groq's **cloud** API (app name and window title are sent) | [GitHub](https://github.com/warpirate/pillar-dynamic-island-for-windows) |
| **Island** (exuupery) | Free, Pro $4.99; 80 stars, mostly Russian users | Live events, clipboard, screenshots, command bar (Alt+Space), weather, AI, reminders, notes, downloads, Claude Code, VPN toggles | [GitHub](https://github.com/exuupery/island), [roadmap #10](https://github.com/exuupery/island/issues/10) |
| **Isle, Dlives, Win Dynamic Island, Up A Notch, BuildNotch, hartono0297, HeyyCzer** | Mostly free | Subsets of the above. BuildNotch and HeyyCzer focus on developers and AI agents; Up A Notch is only a shape | [Isle](https://isle.nxsapps.com/), [Dlives](https://dlives.vercel.app/), [GitHub topic](https://github.com/topics/dynamic-island-windows) |
| **ModernFlyouts** | Free, open source | Replaces the volume, brightness and media flyouts; airplane mode and lock keys | [GitHub](https://github.com/ModernFlyouts-Community/ModernFlyouts) |
| **FluentFlyout** | Free, with a premium tier | Media flyout, volume slider, a taskbar widget with visualizer, lock keys. Lyrics are in progress ([discussion #1003](https://github.com/unchihugo/FluentFlyout/discussions/1003)) | [GitHub](https://github.com/unchihugo/FluentFlyout) |
| **Droptop Four** | Free (Rainmeter), paid extras | A Mac-style menu bar and launcher; weather, Gmail, media, Wi-Fi. Reviews call it heavy (a 95 MB skin) | [droptopfour.com](https://www.droptopfour.com/), [Beebom](https://beebom.com/how-to-get-macos-menu-bar-windows-11/) |

### Feature matrix

✓ = ships. **F** = a flagship feature. **$** = paid app or paid tier only. β = beta. ◐ = partial. R = on its roadmap. B = ReUI is building it now. – = not found (only means we found no evidence it ships).

Mac columns: bn = boring.notch, NN = NotchNook, Alc = Alcove, Atl = Atoll, DL = DynamicLake, MM = MediaMate.
Windows columns: dc90 = devcode90 mod, Nfy = Notchify, DE = Dynamic Edge, NP = NotchPixel, SN = Smart Notch, Halo = Halo DynamicWin, Lum = Lumen, Pil = PILLAR, Isl = exuupery Island, MF/FF = ModernFlyouts and FluentFlyout.

| Feature | ReUI | bn | NN $ | Alc $ | Atl | DL $ | MM $ | dc90 | Nfy $ | DE | NP $ | SN | Halo | Lum | Pil | Isl | MF/FF |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Now Playing + controls | ✓ | **F** | ✓ | ✓ | ✓ | ✓ | **F** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Synced lyrics | ✓ | ◐ | – | – | – | – | – | ✓ | ✓ | ✓ | ✓ | ✓ | – | – | – | – | R |
| Player switcher / per-app volume | ✓ | ◐ | – | – | – | ✓ | – | ✓ | ✓ | – | – | – | – | – | ✓ | – | ◐ |
| Volume / brightness HUD | ✓ | ✓ | ✓ | **F** | ✓ | ✓ | **F** | ✓ | ✓ | ✓ | ✓ | ✓ | – | – | ✓ | – | **F** |
| Lock keys / mic-mute indicator | ✓ | – | – | – | – | ✓ | – | ✓ | – | – | ✓ | – | – | – | – | – | ✓ |
| Clipboard history | ✓ | – | – | – | ✓ | ◐ | – | ✓ | ✓ | **F** | ✓ | ✓ | – | – | – | ✓ | – |
| File shelf | B | **F** | **F** | ? | ✓ | ✓ | – | ✓ | ✓ | ✓ | ✓ | $ | ✓ | – | – | – | – |
| AirDrop / phone transfer | – | ✓ | ✓ | – | ✓ | ✓ | – | – | ✓ | – | ✓ | – | – | – | – | – | – |
| Calendar / next meeting | B | ✓ | ✓ | ✓ | ✓ | ✓ | – | ✓ | ✓ | – | – | – | – | – | ✓ | – | – |
| **Notifications mirroring** | – | R | ? | ? | – | **F** | – | ✓ | **F** | – | ✓ | – | ✓ | ✓ | ✓ | R | – |
| Calls / meetings (timer, mute) | – | – | – | – | – | **F** | – | – | ✓ | – | **F** | – | – | – | – | R | – |
| Bluetooth connect + battery | ✓ (with drawings) | R | – | – | ✓ | ✓ | – | ✓ | ✓ | ✓ | ✓ | ✓ | – | – | – | – | R |
| Battery / charging | ✓ | ✓ | – | ✓ | ✓ | – | – | ✓ | ✓ | – | ✓ | – | – | – | ✓ | – | – |
| Mic / camera / screen privacy | ✓ | – | – | – | ✓ | – | – | ✓ | ✓ | – | – | ✓ | – | ✓ | – | – | – |
| Timers / Pomodoro / stopwatch | ✓ | – | ✓ | – | ✓ | ✓ | – | ✓ | ✓ | ✓ | ✓ | ✓ | – | – | ✓ | ✓ | – |
| Reminders / to-do | – | R | ✓ | – | – | – | – | – | ✓ | – | ✓ | ✓ | – | – | ✓ | ✓ | – |
| Notes | – | – | ✓ | – | ✓ | – | – | – | ✓ | – | – | – | – | – | ✓ | ✓ | – |
| Weather | – | R | ✓ | – | ✓ | ✓ | – | ✓ | ✓ | – | ✓ | – | – | – | – | ✓ | – |
| System stats | ✓ | – | – | – | ✓ | – | – | ✓ | ✓ | – | ✓ | – | – | – | ✓ | – | – |
| Downloads progress | – | – | – | – | β | – | – | – | ✓ | – | ✓ | – | ✓ | – | – | ✓ | – |
| Screenshots / screen recording | ◐ (screen-share dot) | – | – | – | ✓ | – | – | ◐ | ✓ | – | ✓ | – | – | – | – | ✓ | – |
| Quick toggles / actions | – | – | ✓ (Shortcuts) | – | – | – | – | – | ✓ | ◐ (launcher) | ✓ | ✓ | – | – | – | ✓ | ◐ |
| Coding agents | **F** (answers permissions and questions) | R | – | – | ◐ (LLM cost) | – | – | – | – | – | ◐ (status) | – | ✓ (panels) | – | – | ✓ | – |
| Built-in AI chat | – (by design) | R | – | – | – | – | – | ◐ (lookup) | – | – | – | – | – | – | ✓ (cloud) | ✓ | – |
| Camera mirror | – | ✓ | **F** | – | – | – | – | – | – | – | – | – | – | – | – | – | – |
| Game overlay (FPS) | – | – | – | – | – | – | – | ✓ | – | – | ✓ | – | – | – | – | R | – |
| Auto-update | ✓ | – | ✓ | ✓ | – | – | – | (Windhawk) | (Store) | (Store) | (Store) | (Store) | ✓ | ✓ | – | ✓ | ✓ |

What the matrix shows:
- **Table stakes are covered.** Media, HUDs, clipboard, Bluetooth, timers, privacy dots and stats: ReUI ships them all, and in depth (player switcher, per-app volume, device drawings, screen and location indicators).
- **The common gaps are** notifications, weather, reminders/to-do, downloads, screenshots, phone transfer, quick toggles and calls. Shelf and calendar are being built.
- **ReUI's unique ground** is coding agents answered from the island. Others only show status (NotchPixel, Halo, HeyyCzer, BuildNotch) or track cost (Atoll).

---

## 2. What users ask for

### 2.1 GitHub issues, sorted by reactions

**boring.notch** (the largest community; [issues](https://github.com/TheBoredTeam/boring.notch/issues?q=sort%3Areactions-%2B1-desc)):

| Ask | Evidence |
|---|---|
| Let me turn off animations that get in the way | [#364](https://github.com/TheBoredTeam/boring.notch/issues/364) 26r, [#350](https://github.com/TheBoredTeam/boring.notch/issues/350) 21r, [#335](https://github.com/TheBoredTeam/boring.notch/issues/335) 21r, [#303](https://github.com/TheBoredTeam/boring.notch/issues/303) 16r 14c, [#325](https://github.com/TheBoredTeam/boring.notch/issues/325) 12r. Together the most-reacted topic in the repo. **Behaviour, not a feature** |
| Media keeps working after OS updates | [#417](https://github.com/TheBoredTeam/boring.notch/issues/417) 54r 58c, [#779](https://github.com/TheBoredTeam/boring.notch/issues/779) 8r 13c |
| Hide in fullscreen and on external displays | [#119](https://github.com/TheBoredTeam/boring.notch/issues/119) 14r, [#247](https://github.com/TheBoredTeam/boring.notch/issues/247) 8r, [#239](https://github.com/TheBoredTeam/boring.notch/issues/239) 5r, [#110](https://github.com/TheBoredTeam/boring.notch/issues/110) "Exclude Apps" 5r, [#879](https://github.com/TheBoredTeam/boring.notch/issues/879) |
| **Notifications** | [#592](https://github.com/TheBoredTeam/boring.notch/issues/592) 13r (the most-reacted open feature: "I feel like it is a must have… if implemented like it is in DynamicLake, it would be amazing"), [#1489](https://github.com/TheBoredTeam/boring.notch/issues/1489). 15 issue titles mention notifications |
| Pomodoro | [#950](https://github.com/TheBoredTeam/boring.notch/issues/950) 10r (ReUI has it) |
| Liquid Glass look | [#922](https://github.com/TheBoredTeam/boring.notch/issues/922) 10r (visual; Mac-specific) |
| AI assistant | [#1104](https://github.com/TheBoredTeam/boring.notch/issues/1104) 9r ("other dynamic island apps… have an AI assistant") |
| Teleprompter | [#351](https://github.com/TheBoredTeam/boring.notch/issues/351) 8r |
| Phone transfer for Android | NearDrop [#768](https://github.com/TheBoredTeam/boring.notch/issues/768) 7r, LocalSend [#630](https://github.com/TheBoredTeam/boring.notch/issues/630) 3r |
| A device connected (AirPods) and its battery | [#206](https://github.com/TheBoredTeam/boring.notch/issues/206) 5r 10c, [#694](https://github.com/TheBoredTeam/boring.notch/issues/694) 5r, [#1387](https://github.com/TheBoredTeam/boring.notch/issues/1387) 10c, [#418](https://github.com/TheBoredTeam/boring.notch/issues/418) (ReUI has it) |
| Mirror | [#436](https://github.com/TheBoredTeam/boring.notch/issues/436) 5r, [#521](https://github.com/TheBoredTeam/boring.notch/issues/521) 3r; 20 titles mention a mirror (many are bugs in the existing one) |
| Calendar | 40 issue titles (mostly bugs and asks on the existing calendar): [#216](https://github.com/TheBoredTeam/boring.notch/issues/216), [#151](https://github.com/TheBoredTeam/boring.notch/issues/151), [#1413](https://github.com/TheBoredTeam/boring.notch/issues/1413) (open in Google Calendar on the web) |
| Reminders / tasks / notes | 11 titles mention reminders: [#384](https://github.com/TheBoredTeam/boring.notch/issues/384), [#695](https://github.com/TheBoredTeam/boring.notch/issues/695), [#780](https://github.com/TheBoredTeam/boring.notch/issues/780) (notes and clipboard) |
| More music sources | Tidal [#717](https://github.com/TheBoredTeam/boring.notch/issues/717), Plexamp [#730](https://github.com/TheBoredTeam/boring.notch/issues/730), Qobuz [#1345](https://github.com/TheBoredTeam/boring.notch/issues/1345); 26 titles mention Spotify. On Windows, GSMTC covers these for free |
| Extensions / real live activities | [#670](https://github.com/TheBoredTeam/boring.notch/issues/670), [#671](https://github.com/TheBoredTeam/boring.notch/issues/671); "Extension system" is on its roadmap |
| Coding agents | [#951](https://github.com/TheBoredTeam/boring.notch/issues/951) "Claude Code Session Monitoring" 3r 8c |
| Smaller asks | Answer calls [#414](https://github.com/TheBoredTeam/boring.notch/issues/414), battery-saver toggle [#796](https://github.com/TheBoredTeam/boring.notch/issues/796), workspace indicator [#443](https://github.com/TheBoredTeam/boring.notch/issues/443), Wi-Fi [#1274](https://github.com/TheBoredTeam/boring.notch/issues/1274), image conversion on the shelf [#1291](https://github.com/TheBoredTeam/boring.notch/issues/1291) |

Weather, translation, stocks, calculators and screen recording each have **zero** boring.notch issue titles.

**Atoll** ([issues](https://github.com/Ebullioscopic/Atoll/issues)): the reaction counts are small. The top ask is "Dynamic Island integration for terminal command permissions" ([#390](https://github.com/Ebullioscopic/Atoll/issues/390), 5r), which is exactly what ReUI does. Then an "AI Agent Hub for local coding agents" ([#539](https://github.com/Ebullioscopic/Atoll/issues/539)), duplicate HUDs ([#419](https://github.com/Ebullioscopic/Atoll/issues/419)), Notes sync ([#450](https://github.com/Ebullioscopic/Atoll/issues/450)), "favourite this song" ([#599](https://github.com/Ebullioscopic/Atoll/issues/599)), quick reminders ([#78](https://github.com/Ebullioscopic/Atoll/issues/78)), app launching ([#643](https://github.com/Ebullioscopic/Atoll/issues/643)), very high CPU ([#641](https://github.com/Ebullioscopic/Atoll/issues/641), 12c), and "show weather when there are no calendar events" ([discussion #821](https://github.com/Ebullioscopic/Atoll/discussions/821)).

**devcode90 mod** ([repo](https://github.com/devcode90/Dynamic-Island-for-Windows/issues), plus [windhawk-mods](https://github.com/ramensoftware/windhawk-mods/issues?q=%22Dynamic+Island%22)):
- **Hiding** is the top theme: [#60](https://github.com/devcode90/Dynamic-Island-for-Windows/issues/60), [#43](https://github.com/devcode90/Dynamic-Island-for-Windows/issues/43), [#32](https://github.com/devcode90/Dynamic-Island-for-Windows/issues/32), [#57](https://github.com/devcode90/Dynamic-Island-for-Windows/issues/57) (all "doesn't hide / auto-hide"), and [windhawk-mods#4738](https://github.com/ramensoftware/windhawk-mods/issues/4738): media "forces Island to stay open continuously instead of auto-hiding".
- **Mic mute shown, Windows' flyouts hidden:** [#147](https://github.com/devcode90/Dynamic-Island-for-Windows/issues/147).
- **Calendar:** [#92](https://github.com/devcode90/Dynamic-Island-for-Windows/issues/92).
- **Weather:** Fahrenheit [#13](https://github.com/devcode90/Dynamic-Island-for-Windows/issues/13), OpenWeatherMap [windhawk-mods#6093](https://github.com/ramensoftware/windhawk-mods/issues/6093), blank weather [#5997](https://github.com/ramensoftware/windhawk-mods/issues/5997). But also [windhawk-mods#4424](https://github.com/ramensoftware/windhawk-mods/issues/4424): "seeing the weather all the time makes the top bar look a bit cluttered… show only the clock".
- [Discussion #4194](https://github.com/ramensoftware/windhawk-mods/discussions/4194) (3 upvotes): "show only selected info when no media is playing", weather, stocks, "Windows Action Center notifications".
- **Screenshot toasts arrive late:** [#145](https://github.com/devcode90/Dynamic-Island-for-Windows/issues/145).

**ModernFlyouts** ([issues](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues)): it doesn't replace the Windows 11 flyouts ([#920](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/920), 25r 27c). **Per-app volume** ([#164](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/164), 20r 17c). Replace the tray volume flyout ([#103](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/103), 16r). Battery flyout ([#352](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/352), 8r 16c). Brightness on desktops ([#102](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/102), 6r). An AutoPlay flyout ([#240](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/240)). "Bluetooth is connected" ([#456](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/456)). **Mic mute and camera notifications** ([#225](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/225), 4r 12c). Multi-monitor ([#736](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/736)).

**FluentFlyout** ([issues](https://github.com/unchihugo/FluentFlyout/issues)): volume slider ([#26](https://github.com/unchihugo/FluentFlyout/issues/26), 25r). **App allow/block list** ([#81](https://github.com/unchihugo/FluentFlyout/issues/81), 17r). Switching between media apps ([#206](https://github.com/unchihugo/FluentFlyout/issues/206), 10r). Choose the screen ([#30](https://github.com/unchihugo/FluentFlyout/issues/30), 8r). Several flyouts for several apps ([#66](https://github.com/unchihugo/FluentFlyout/issues/66)). Don't pop up on every key press ([discussion #1123](https://github.com/unchihugo/FluentFlyout/discussions/1123)).

**exuupery Island** (Windows; [roadmap #10](https://github.com/exuupery/island/issues/10), 8r 35c; [#9](https://github.com/exuupery/island/issues/9), 7r):
- Wi-Fi network list ([#11](https://github.com/exuupery/island/issues/11)).
- Several monitors.
- "Windows should stop at the island, not the screen edge" ([#6](https://github.com/exuupery/island/issues/6)). One commenter: it "covers browser tabs and gets in the way even with auto-hide on".
- Call status in Zoom, Discord and Telegram, with buttons.
- NVIDIA recording status, and a game overlay with FPS.
- Mic mute from the island ("doesn't work").
- **Recurring reminders**, a ringtone for reminders, and editing a reminder.
- Hide in chosen apps, and the inverse: show only in chosen apps.
- Pick which media source shows.
- Screen recording with sound, and markup on screenshots.
- A plugin system ("we can't do without one").
- Integrations with more agents ("The Claude Code integration is really cool! Will there be others?").

**Dynamic Edge** ([issues](https://github.com/Eul45/Dynamic-Edge/issues)): "lower hot edge sensitivity" ([#1](https://github.com/Eul45/Dynamic-Edge/issues/1)) and tab order ([#2](https://github.com/Eul45/Dynamic-Edge/issues/2)).

**Lumen** ([issues](https://github.com/Risuleia/Lumen/issues)): jitter when the pointer is at the very top ([#11](https://github.com/Risuleia/Lumen/issues/11)), hide it completely when collapsed ([#6](https://github.com/Risuleia/Lumen/issues/6)), vertical offset ([#2](https://github.com/Risuleia/Lumen/issues/2)), and conflicts with the YASB bar ([#3](https://github.com/Risuleia/Lumen/issues/3), 14c).

**PILLAR** has no issues yet.

### 2.2 Reviews and press: praise, gaps, uninstall reasons

- **Battery and CPU are the top reason people uninstall.**
  - NotchNook: a MacRumors user blamed it for lost battery life, and NotchNook's Discord told users to roll back a version ([thread](https://forums.macrumors.com/threads/notchnook.2463695/)).
  - Notchy's uninstall guide names battery and CPU as the most common reasons ([notchy.dev](https://notchy.dev/uninstall-notchnook/); a competitor).
  - Notchify: a Reddit user's CPU breakdown made its developer cut idle CPU, by the developer's own claim, "from 55% to 5%" ([WindowsForum](https://windowsforum.com/windows-news.4/notchify-claims-idle-cpu-cut-from-55-to-5-after-fix.442253/), [Windows Central](https://www.windowscentral.com/software-apps/notchify-brings-macos-style-dynamic-island-flair-to-windows-11)).
  - Atoll [#641](https://github.com/Ebullioscopic/Atoll/issues/641) "Very High CPU Usage".
- **It covers what's under it.** "Windows does not reserve the space… must share it with tabs and title bars" (Notchify's developer, via WindowsForum). Windows Central: "Apps do not wrap around Notchify, which can result in content being covered" and "the animations are a bit janky".
- **Price and trial.** NotchNook's Reddit launch feedback: the price was too high and a one-hour trial too short ([Digital Trends via Yahoo](https://tech.yahoo.com/computing/articles/macbook-notch-redeemed-130056005.html)). Windows apps cost $0–10, and the free ones are good.
- **Praise goes to finish, not breadth.** Alcove is "the one that feels finished" ([Theodore HQ](https://www.theodorehq.com/macdept/app/alcove)). Notchify works because it "gathers information in one spot" on a big monitor.
- **Why notifications in the island at all:** Windows 11 doesn't let you choose where notifications appear (Windows Central on Notchify).

---

## 3. The external report (Oct 30, 2026), checked

Its thesis: users aren't asking for more widgets but for an island that doesn't get in the way. "The most considerate Dynamic Island for Windows, not the longest feature list." **The evidence above supports it.** The most-reacted items on every tracker are behaviour items: animations, hiding, fullscreen, app exclusions, CPU. No widget comes close.

| Claim | Check | Verdict |
|---|---|---|
| Hovering the top edge conflicts with browser tabs; users want an adjustable trigger zone and click-to-expand (r/Windows11 `1vug969`) | Reddit unreadable. In substance: Notchify's developer quoted on WindowsForum, exuupery [#10](https://github.com/exuupery/island/issues/10) ("covers browser tabs… even with auto-hide"), Dynamic Edge [#1](https://github.com/Eul45/Dynamic-Edge/issues/1), Lumen [#11](https://github.com/Risuleia/Lumen/issues/11) | **Confirmed (thread itself not read)** |
| Notchify idle CPU cut from about 56 % to 6 % of a core; input lag fixed (r/windowsapps `1vgdb1y`) | WindowsForum says "55% to 5%", as **the developer's claim**; Windows Central confirms the Reddit CPU breakdown. No source found for the input-lag fix or for "high CPU while hidden" | **Mostly confirmed; numbers are self-reported** |
| Media: seeking, synced lyrics, switching players without jumping between sessions | FluentFlyout [#206](https://github.com/unchihugo/FluentFlyout/issues/206) 10r and [#66](https://github.com/unchihugo/FluentFlyout/issues/66), boring.notch [#391](https://github.com/TheBoredTeam/boring.notch/issues/391), exuupery [#8](https://github.com/exuupery/island/issues/8) | **Confirmed.** ReUI already does all three (PRD-now-playing M1) |
| Visibility: media keeps the island visible ([windhawk-mods#4738](https://github.com/ramensoftware/windhawk-mods/issues/4738)) | Issue read: closed, 1r 5c | **Confirmed.** ReUI's "When needed" mode is the answer; "When active" (the new-user default) still shows the island while music plays |
| Customisation without clutter ([#4424](https://github.com/ramensoftware/windhawk-mods/issues/4424), [discussion #4194](https://github.com/ramensoftware/windhawk-mods/discussions/4194)) | Both read | **Confirmed** |
| Dynamic Edge users want larger text, panel management, clipboard exclusions (r/Windows11 `1w7wld1`) | Reddit unreadable; its GitHub shows edge sensitivity and tab order | **Partly confirmed** |
| Competitors named (Notchify, Dynamic Edge, devcode90, Lumen, PILLAR, NotchPixel's comparison) | All verified (§1). **Correction:** PILLAR's "private AI assistant" sends the app name and window title to Groq's cloud API | **Confirmed, with the correction** |
| Trust: signed installer, privacy disclosure, persistent settings, published performance numbers, known limitations | Unsigned installers are common (PILLAR, BuildNotch per its FAQ); boring.notch [#905](https://github.com/TheBoredTeam/boring.notch/issues/905) asks for install guidance. ReUI's PRD-product-shell §5.6 already lists "Idle cost doubts → measured idle numbers in About" as not done | **Agreed.** ReUI's installer is not signed today (`scripts/package.ps1` has no signing step) |

**Where ReUI stands on the report's P0 list:**
- **Already done (PRD-product-shell §5.6):**
  - click-through until hover intent, with a hover delay;
  - no opening while something is dragged;
  - Win+Alt+I to hide, and "When active" / "When needed" modes;
  - hidden in fullscreen, including borderless games and browser video;
  - choice of monitor, DPI handling, and re-placing after sleep and Explorer restarts.
- **Not done:** per-app rules, reserving the screen strip, published idle numbers, and a signed installer. These are the "behaviour and trust" items in §5.

---

## 4. Candidate features, ranked

**Demand** counts independent sources: separate issues from different trackers, plus how many competitors ship the feature as a strong one. **Fit** is with DESIGN.md (black and quiet, one accent, no filler, never steal focus) and PLAN.md (event-driven, zero idle cost, nothing leaves the PC unless needed, no package identity). Effort: S = days, M = 1–2 weeks, L = more.

| # | Candidate | Demand | Fit | Windows approach and blockers | Effort |
|---|---|---|---|---|---|
| 1 | **Notifications mirroring** | **High.** Top boring.notch open ask (#592 13r, #1489); windhawk discussion #4194; exuupery; DynamicLake's flagship; 8 Windows competitors ship it | Good, if opt-in per app and quiet. It is a moment that comes to you | `UserNotificationListener`. **Without package identity:** `RequestAccessAsync`, `GetNotificationsAsync` and `RemoveNotification` work; the `NotificationChanged` event and `ClearNotifications` fail with 0x80070490 ([stm32-desk-dashboard #33](https://github.com/egungor/stm32-desk-dashboard/issues/33), [PR #78](https://github.com/egungor/stm32-desk-dashboard/pull/78)). So it is polling (they used 500 ms) or a **sparse signed package** for identity ([explorer-lite PR #14](https://github.com/justdiegocom/explorer-lite/pull/14)), which needs signing (T2). A spike: wake on changes to the notification database (`%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db*`, via `ReadDirectoryChangesW`) and only then call `GetNotificationsAsync`. That would keep idle at zero; it is unverified | L |
| 2 | **Calls: timer, mute, camera** | **Medium-high.** DynamicLake and NotchPixel flagships; Notchify Discord controls; exuupery roadmap; boring.notch #414; devcode90 #147; ModernFlyouts #225 (12c) | Very good. It builds on the privacy indicators that exist | Mic in use by a known call app (Teams, Zoom, Discord, Meet in a browser, Slack) is already detected through the consent store. Mute = the default capture endpoint's `IAudioEndpointVolume::SetMute`, already used for the mic-mute OSD. **Teams' local third-party API was retired in summer 2026** ([MC1266901](https://learn.microsoft.com/en-us/answers/questions/5866166/retirement-of-legacy-third-party-meeting-and-call), [MuteMe](https://muteme.com/blogs/news/dont-panic-a-solution-is-coming-for-our-teams-users)), so muting at the system level is the only approach that works in every app. Show that ReUI's mute is not the app's own mute | S–M |
| 3 | **Downloads in progress** | **Medium.** Shipped by Atoll (β), Notchify 5.8, NotchPixel, Halo, exuupery (whose #26 "doesn't detect Firefox" shows it's hard); in the report's P2 | Good: an ongoing activity that ends in a moment with Open / Show in folder / drag out to the shelf | No system API. Watch the Downloads known folder (`FOLDERID_Downloads`, plus the browser's folder) with `ReadDirectoryChangesW` for `.crdownload` (Chromium, Edge), `.part` (Firefox) and `.partial`; bytes and speed from size growth; done on rename. The total size is unknown, so progress is indeterminate. Event-driven, local | M |
| 4 | **Screenshot moment** | **Medium.** Notchify (snip, recording), exuupery (screenshots, markup asks), devcode90 #145, Atoll recording | Good: it extends clipboard and shelf | A new file in the Screenshots known folder (`FOLDERID_Screenshots`, written by Win+PrtScn and Snipping Tool) or an image from Snipping Tool on the clipboard (already captured). Show the thumbnail with Copy, Open, drag out, Delete, like macOS's floating thumbnail. Recording state already shows as the purple screen dot | S–M |
| 5 | **Phone transfer (LocalSend) into the shelf** | **Medium.** boring.notch #768 7r and #630; Atoll AirDrop plus [discussion #719](https://github.com/Ebullioscopic/Atoll/discussions/719) (LocalSend); NotchDrop's whole product; Notchify 5.8; NotchPixel | Good: LAN only, no cloud, open protocol | Speak the open [LocalSend protocol v2](https://github.com/localsend/protocol): UDP multicast on 224.0.0.167:53317, REST on TCP 53317, HTTPS with a fingerprint. Phones only need the free LocalSend app. Phone Link and Quick Share have no public API. Needs a firewall rule from the installer and opt-in. The listener stays idle (a socket wait) | M–L |
| 6 | **Live activities from scripts** (`reui-bridge` progress/status) | **Medium.** boring.notch "Extension system" roadmap plus #670 and #671; exuupery #8 "plugin system"; NotchPixel custom scripts; Atoll "shelf from Terminal" | Excellent: semantic protocol, modules don't draw, agent-friendly | Extend `reui-bridge` (it already does `timer`): `reui-bridge activity start|update|end` with a title, progress, accent and an "open" target. Builds, renders, backups and long CLI jobs get a pill. No new OS API | M |
| 7 | **Reminders: quick capture and repeat** | **Medium.** boring.notch roadmap, #384 and 11 reminder titles; Atoll #78; exuupery (repeat, ringtone, edit); shipped by NotchNook, Notchify, NotchPixel, Smart Notch, PILLAR | Good: the timer parser already exists ("10m tea") | Extend the timer field to `5pm call mom` and `every day 23:00 pills`. Stored locally; the hub's existing timer wheel wakes it, no polling. Microsoft To Do sync would need the network and is out of scope | S–M |
| 8 | **Weather, opt-in** | **Medium in what ships** (NotchNook, Atoll, DynamicLake, devcode90, Notchify, NotchPixel, exuupery, Droptop), **low in asks** (0 boring.notch titles; windhawk #6093, #5997, #13). Counter-signal: #4424 "cluttered" | OK only as quiet detail: the date peek or the calendar card, never the resting pill | [Open-Meteo](https://open-meteo.com/), no key. Location: a city typed by the user, or `Windows.Devices.Geolocation` (works unpackaged when "Let desktop apps access your location" is on). Fetched hourly only while the PC is in use; a network call, so opt-in like lyrics | S–M |
| 9 | **Full volume mixer** (every app, not just the player) | **Medium-high on Windows.** ModernFlyouts #164 20r 17c and #103 16r; FluentFlyout #26 25r; PILLAR | Very good: it extends the existing volume card | `IAudioSessionManager2` session enumeration plus `IAudioSessionNotification` (event-driven), the same code as Now Playing's per-app slider | S |
| 10 | **Per-app rules** (hide over these apps / show only in these; skip notifications and media from these) | **High.** FluentFlyout #81 17r; boring.notch #110 and #879; exuupery (both directions); devcode90 ships media exclusions | Excellent: "considerate" | The foreground-window hook already exists for fullscreen detection (`EVENT_SYSTEM_FOREGROUND`); match the exe or AUMID. Listed as T3 below | S |
| 11 | Quick toggles (mic mute, DND, dark mode, Wi-Fi/Bluetooth radio, battery saver) | Medium-low: Notchify, Smart Notch, exuupery #11 (2r); boring.notch #796 and #1274 | Mixed: Windows' Quick Settings (Win+A) already does this | Radios: `Windows.Devices.Radios`. Dark mode: the `AppsUseLightTheme` registry value plus `WM_SETTINGCHANGE`. **DND has no public API** (PRD-timers-focus §6) | M |
| 12 | Notes / scratchpad | Medium-low: NotchNook, Atoll, Notchify, PILLAR, exuupery; boring.notch #695 and #780 | Weak: an editor in the island means text input and focus | Local file | M |
| 13 | Camera mirror | Mac-only demand (boring.notch #436, NotchNook's flagship); no Windows competitor ships it | Poor: live video in the island; Windows has the Camera app | `MediaCapture` | M |
| 14 | Game overlay / FPS | Low-medium: devcode90, NotchPixel, exuupery roadmap | Poor: ReUI hides in fullscreen by design; FPS needs ETW/PresentMon tracing | ETW | L |
| 15 | Built-in AI chat | boring.notch #1104 9r; PILLAR and exuupery ship it | **Against direction** (AI = external agents) and privacy (PILLAR's goes to Groq) | — | — |
| 16 | Stocks, crypto, sports scores | One ask (windhawk #4194, stocks) | Poor: network, filler | — | — |
| 17 | Keyboard-layout HUD | No asks found. ModernFlyouts #584 is the opposite, a bug where the volume flyout popped on a layout switch | — | — | S |
| 18 | App switcher, launcher, emoji / colour picker, calculator, converter, translator, dictionary | Few asks (Atoll #643; devcode90's Quick Lookup; Atoll's colour picker) | Windows or PowerToys cover them (Alt+Tab, Win+., Run, Color Picker) | — | — |
| — | Windows Update status, printer progress, VPN status, battery health, world clocks, network speed | **No user asks found.** VPN shows up only as exuupery commenters wanting more clients toggled; network speed only as an Atoll stats tile | — | — | — |

---

## 5. Recommendation

### 5a. Behaviour and trust fixes first (cheap; they decide whether people keep it)

These come first because the most-reacted items on every tracker are behaviour and trust, not widgets.

| # | Fix | User problem | Evidence | Approach | Effort |
|---|---|---|---|---|---|
| T1 | **Publish idle cost**: measured CPU, GPU/DWM and memory per module in Settings → About, plus a "known limitations" page | "Will this drain my battery or slow my PC?" | Top uninstall reason (NotchNook on MacRumors, Notchify's 55 %→5 %, Atoll #641); PRD-product-shell §5.6 marks it not done | The soak harness already measures this; show the last release's numbers | S |
| T2 | **Signed installer and updates** | SmartScreen's "unknown publisher" warning on first run and every update | Unsigned peers (PILLAR, BuildNotch); boring.notch #905 and #37 | [SignPath Foundation](https://signpath.org/) (free for OSS, but it appears as the publisher) or [Azure Artifact Signing](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options) ($9.99/month; individuals in the US and Canada only). Signing also unlocks a sparse package for notifications (#1) | S–M |
| T3 | **Per-app rules** (candidate #10) | "It covers my editor/game/video app; don't show there" | FluentFlyout #81 17r; boring.notch #110 and #879; exuupery | Foreground hook plus an exe list in Settings: *Hide over*, *Show only over*, *Ignore media/notifications from* | S |
| T4 | **Optional reserved strip** (Notch style) | Maximised windows put tabs and title buttons under the island | WindowsForum (Notchify's developer), Windows Central, exuupery #6 | Register an AppBar the island's height (`SHAppBarMessage(ABM_NEW / ABM_SETPOS)`, ABE_TOP) so maximised windows start below it. Off by default (it costs about 32 px); per monitor | M |
| T5 | **Make "When needed" easy to find** | Music keeps the island showing ([windhawk-mods#4738](https://github.com/ramensoftware/windhawk-mods/issues/4738)) | Same as T3 | One contextual offer after the first hour of continuous media in "When active": switch to "When needed" (ReUI's existing offer mechanism; no new text in the pill) | S |

### 5b. Top 10 new features, in order

1. **Notifications, opt-in per app**
   - **Problem:** Windows toasts land in a corner you don't watch, and Windows won't let you move them.
   - **Evidence:** boring.notch's most-reacted open feature (#592), windhawk #4194, exuupery; DynamicLake's flagship; 8 Windows competitors ship it.
   - **Windows:** `UserNotificationListener`. Read and remove work unpackaged; live events need identity (sparse package after T2). Until then, the spike found a free trigger: the shell's notification-count WNF state moves with every toast, so ReUI lists only then. The database file is written too late to use. Measurements and plan: [PRD-notifications.md](PRD-notifications.md).
   - **Effort:** L.
   - **Better than competitors:**
     - Off until you pick apps, and nothing leaves the PC.
     - A message from a chat you're in is grouped by conversation (Notchify does this).
     - A toast never takes the keyboard.
     - DND and focus sessions are respected.
     - Coding-agent and call moments keep priority over chat.
   - Codes and secrets in a toast get the clipboard's secret filter.
2. **Calls**
   - **Problem:** "Am I muted? How long has this call run?", without finding the meeting window.
   - **Evidence:** DynamicLake and NotchPixel flagships, Notchify Discord controls, exuupery roadmap, ModernFlyouts #225, devcode90 #147, boring.notch #414.
   - **Windows:** call-app detection from the existing mic/camera consent-store module; system capture mute with `IAudioEndpointVolume`; the timer from the mic-in-use start. (Teams' local API was retired in 2026, so system mute is the only approach that works across apps.)
   - **Effort:** S–M.
   - **Better:** works for every app with no plugins or accounts, and the mute state is true at the device level.
3. **Downloads**
   - **Problem:** "Is it done yet, and where did it go?"
   - **Evidence:** Atoll, Notchify, NotchPixel, Halo, exuupery; the report's P2.
   - **Windows:** `ReadDirectoryChangesW` on the Downloads folders, with browser temp-file patterns.
   - **Effort:** M.
   - **Better:** finishes as a card you can drag straight onto the shelf or into an app. Event-driven, so it costs nothing between downloads.
4. **Screenshot moment**
   - **Problem:** after Win+Shift+S the shot lives only on the clipboard or in a folder.
   - **Evidence:** Notchify, exuupery, devcode90 #145, Atoll recording.
   - **Windows:** a Screenshots known-folder watch, plus the existing clipboard capture.
   - **Effort:** S–M.
   - **Better:** one card that ties clipboard, shelf and drag-out together. No second capture tool.
5. **Per-app volume mixer**
   - **Problem:** turning one app down means opening Windows' mixer.
   - **Evidence:** ModernFlyouts #164 (20r 17c) and #103 (16r), FluentFlyout #26 (25r), PILLAR.
   - **Windows:** `IAudioSessionManager2` plus session notifications.
   - **Effort:** S.
   - **Better:** inside the volume card you already get from the keys, with the output picker beside it.
6. **Phone transfer (LocalSend) in the shelf**
   - **Problem:** getting a file from the phone to the PC, or back, without cables or cloud.
   - **Evidence:** boring.notch #768 (7r) and #630; Atoll's AirDrop and LocalSend discussion; NotchDrop; Notchify 5.8; NotchPixel.
   - **Windows:** a native LocalSend v2 implementation, LAN only.
   - **Effort:** M–L.
   - **Better:** works with the free LocalSend apps on Android and iOS, with no account or server. Received files land on the shelf.
7. **Live activities from scripts**
   - **Problem:** long jobs (builds, renders, backups) give no signal when they finish.
   - **Evidence:** boring.notch extension roadmap plus #670 and #671; exuupery "plugin system"; NotchPixel scripts.
   - **Windows:** `reui-bridge activity …` over the existing pipe.
   - **Effort:** M.
   - **Better:** the island stays the only thing that draws, so third-party activities still look like ReUI. This also fits the coding-agent audience.
8. **Reminders**
   - **Problem:** "remind me at 5" with nowhere quick to say it.
   - **Evidence:** boring.notch roadmap plus 11 titles, Atoll #78, exuupery (repeat, sound, edit); shipped by 5 competitors.
   - **Windows:** the timer parser and scheduler, local storage.
   - **Effort:** S–M.
   - **Better:** one field for timers, focus and reminders ("5pm call mom", "every day 23:00 pills"), with no to-do app attached.
9. **Weather, opt-in, in the date peek and the calendar card**
   - **Problem:** "do I need a jacket before the 3 pm meeting?"
   - **Evidence:** shipped by 8 competitors but asked for rarely, and #4424 calls it clutter.
   - **Windows:** Open-Meteo, a typed city or Geolocation.
   - **Effort:** S–M.
   - **Better:** never in the resting pill; it shows only where you already look (the peek, the next meeting), honouring "no filler".
10. **Quick controls: mic mute, dark mode, Wi-Fi and Bluetooth radios** (as a place)
    - **Problem:** the two or three toggles people flip all day.
    - **Evidence:** Notchify, Smart Notch, exuupery #11, boring.notch #796 and #1274.
    - **Windows:** `Windows.Devices.Radios`, the theme registry value with `WM_SETTINGCHANGE`, endpoint mute. **No DND** (no public API).
    - **Effort:** M.
    - **Better:** only toggles Windows makes slow to reach, nothing that duplicates Win+A. This is last because the evidence is thin.

### 5c. Don't build (for now)

- **Built-in AI chat.** It has asks (boring.notch #1104) and PILLAR and exuupery ship it. But it goes against the agreed direction (AI = external agents answered from the island), and in practice it sends context to a cloud API. ReUI's agent moments are the better answer, and Atoll's top request (#390) is exactly them.
- **Camera mirror.** The demand is Mac users replacing Hand Mirror, and no Windows island ships one. Live video costs CPU, and Windows has the Camera app.
- **Game overlay / FPS.** ReUI hides in fullscreen on purpose. FPS needs ETW/PresentMon. Xbox Game Bar and RTSS already do it.
- **Stocks, crypto, sports scores.** One ask in total. Network plus constant numbers is filler.
- **Virtual pets, teleprompter, Liquid Glass.** These are novelty or visual. The pet is in Dynamic Edge and windhawk #6664; the teleprompter is boring.notch #351. The UI is a placeholder until the design pass.
- **App switcher, launcher, emoji or colour picker, calculator, converter, translator.** Windows and PowerToys already do these well (Alt+Tab, Win+., Run, Color Picker), and asks are rare.
- **Teams/Zoom-specific call APIs.** Teams' local API was retired in 2026; build system-level mute (#2) instead.
- **Windows Update, printer, VPN, battery-health, network-speed widgets.** No demand found.

---

## Sources

The main sources are linked inline. Further sources:
- Roundups (all written by competitors): [Notchy](https://notchy.dev/best-mac-notch-apps/), [brow-app](https://brow-app.com/blog/best-macbook-notch-apps-2026), [apps.deals](https://blog.apps.deals/best-notch-apps-mac).
- Notification listener: [Microsoft docs](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/notification-listener), [WinToastRelay](https://github.com/RavelloH/WinToastRelay) (ships as MSIX for identity).
- Teams local API: [teams-local-mute](https://github.com/Russell-KV4S/teams-local-mute), [TeamSpot](https://github.com/benbrandt22/TeamSpot).
- More Windows islands: [Win11-Dynamic-Island (Windhawk)](https://github.com/Tederby/Win11-Dynamic-Island).
