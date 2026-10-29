# PRD: Calls (an ongoing call: how long, and am I muted?)

Status: M0–M1 built Oct 30, 2026 · Owner: Rohit · "Calls" in [PLAN.md](../PLAN.md), #2 in [research-features.md](research-features.md) · Oct 30, 2026

**References:**
- **iPhone:** a call you leave runs on as a green capsule in the Dynamic Island: the call's glyph and a running time; opened, the caller, the time, Mute, Speaker and End ([Apple: Dynamic Island](https://support.apple.com/guide/iphone/use-the-dynamic-island-iph28f50d10d/ios)).
- **Windows island apps:** DynamicLake Pro and NotchPixel lead with calls (a call timer, a global mic mute); Notchify has Discord controls (research-features.md, competitor table).
- **Asks:** ModernFlyouts #225, devcode90 #147, boring.notch #414, exuupery's roadmap.
- **What can't be done:** Teams' local third-party API was retired in summer 2026 ([MC1266901](https://learn.microsoft.com/en-us/answers/questions/5866166/retirement-of-legacy-third-party-meeting-and-call), [MuteMe](https://muteme.com/blogs/news/dont-panic-a-solution-is-coming-for-our-teams-users)); Zoom, Discord and Slack have no local API a desktop app can join without an account and an OAuth app. So nothing here talks to a call app: a call is the system's view of it (who holds the microphone) and the mute is the system's (the default microphone).

**Inputs:** the privacy indicator ([PRD-privacy-indicators.md](PRD-privacy-indicators.md): the consent store says which app holds the microphone, camera and screen, since when, and keeps a dot only while that app's process lives), the shared Core Audio module (`set_capture_muted`, the capture mute event), the player activation code (`media/player.rs`: an app's window brought forward), the Clock element.

## 1. Problem

"Am I muted? How long has this call run?" Today the answer is in the meeting window, which is behind the document you're presenting or minimised. The privacy indicator already says "Teams is using the microphone", but not that it's a call, not for how long, and its mute is one button among others.

## 2. Goals

1. **A call is a call.** When a known call app holds the microphone for a few seconds, the island shows an Ongoing call: the app's icon and a running time.
2. **Mute in one click, true at the device.** The open card's mute is the system microphone's: whatever the app shows, nothing reaches it.
3. **Back to the call.** "Open Teams" brings its window forward.
4. **One activity per app.** The call card replaces the privacy indicator for that app; another app using the microphone still has its dot.
5. **Free when idle.** No new watcher: the same registry notification and process waits as the indicator; one timer for the settle and the end.

## 3. Non-goals (v1)

- Who you're talking to, hang up, raise hand, leave: only the app knows, and there's no local API (above).
- **Camera off.** There's no cross-app way to stop another app's camera stream. Windows can disable a camera only system-wide from Settings › Bluetooth & devices › Cameras, which needs administrator rights and may need a reboot ([Microsoft Support](https://support.microsoft.com/en-us/windows/hardware/camera/manage-cameras-with-camera-settings-in-windows-11)); the privacy switch (Settings › Privacy & security › Camera) only refuses *new* access and changes the user's settings for good; `AppCapability` checks access, it doesn't revoke it ([Microsoft Learn](https://learn.microsoft.com/en-us/uwp/api/windows.security.authorization.appcapabilityaccess.appcapability)). So the card says the camera is on and offers nothing that would pretend to turn it off.
- Calls in a browser tab by name (Meet, Teams on the web): see D2.
- Incoming-call rings (no event to read).

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| C1 | Join a Teams meeting | The mic dot for the first seconds, then the call: Teams' icon and 0:05, counting from when the mic went on |
| C2 | Mute from the island | The card's mic button: "Teams · Microphone muted"; the pill's side turns to a red mic-off; Win+Alt+K agrees |
| C3 | Teams' own mute | Nothing changes: the app keeps the device open (that's why the card says "Microphone muted", the device's state, not "Muted") |
| C4 | Camera on in the call | Eyebrow "Video call" |
| C5 | Share the screen from Teams | Detail "Teams · Sharing screen"; no second (purple) activity for Teams |
| C6 | Discord voice for two hours | 2:00:00, the island advancing it; no publishes in between |
| C7 | Leave the meeting | The mic is released; 3 s later the call is gone |
| C8 | Teams closes and reopens the mic while joining, or switches microphones | Same call, same start: a release shorter than 3 s isn't an end |
| C9 | A voice note in WhatsApp (4 s) | The privacy dot; no call (it didn't settle) |
| C10 | Chrome holds the mic (Meet) | The privacy indicator ("Chrome · Microphone"), unless *Browsers* is on (D2) |
| C11 | Hub starts mid-call | The call at once, its time from the mic's real start |
| C12 | Fullscreen game, Discord call | Hidden with the island (`hide_in_fullscreen`); nothing about a call is a decision |

## 5. Experience

### 5.1 Presentations

**Call (Ongoing, key `call:<app>`):** published once a call settles, retracted when it ends.
- **Accent:** iOS's call green `#30D158`.
- **Compact:** leading the app's icon (24 circle; a phone glyph tile until it arrives); trailing the call's time (`Element::Clock`, counting up from the mic's start, advanced by the island). Muted: the leading side is a bare mic-off glyph in `Negative`, so "am I muted?" is the first thing the pill says.
- **Minimal:** the phone glyph in the accent (mic-off in `Negative` while muted).
- **Expanded** (eyebrow → hero → detail):
  - eyebrow: "Call", or "Video call" while the app also holds the camera;
  - hero: the time (`Headline::Clock`, `value` numerals);
  - detail: the app's name, and one fact when there is one: "Teams · Microphone muted" › "Teams · Sharing screen";
  - rows: buttons: the mic glyph (mic-off and `Accent` while muted) and **Open Teams**.

### 5.2 Priority and quiet

- Ongoing, never expands itself, never makes a sound, no Brief when it starts (the call appearing is the news). So Do Not Disturb has nothing to hold back, and a decision or alert still goes in front of it.
- Fullscreen: hidden with the rest of the island; a call never brings it back.

### 5.3 Showing and going (`privacy/calls.rs`, pure, clock passed in)

- **Settle:** a known call app's microphone use counts as a call once it has lasted `CALL_SETTLE` (5 s) by the store's own start time; a hub started mid-call shows it at once.
- **End:** the microphone gone for `CALL_END` (3 s). Back within that, it's the same call with the same start (C8).
- **Time:** from the first microphone start of the call, not from when it settled.
- **Camera, screen:** facts about a call, never a call by themselves (a camera preview isn't a call).

### 5.4 One activity, not two

While an app has a call, the privacy indicator leaves out everything that app uses (microphone, camera, screen) and doesn't announce it starting the camera mid-call. The indicator stays for other apps. During the 5 s settle the mic dot is what shows: something *is* using the microphone, and a call isn't certain yet.

## 6. Platform details

| Signal | How | Event? | Misses |
|---|---|---|---|
| A call app holds the mic, since when | The privacy module's consent store (`store.rs`) and liveness (`alive.rs`), unfiltered by its kind toggles | Yes (registry change notification, process waits) | Same as the indicator's: one entry per exe, an app missing from the store has no call |
| Which apps are call apps | Exe file names and package families (`calls.rs`): Teams (`ms-teams.exe`, `Teams.exe`, `MSTeams_*`), Zoom (`Zoom.exe`), Discord (`Discord*.exe`), Slack (`slack.exe`), Webex (`CiscoCollabHost.exe`, `webexmta.exe`, `atmgr.exe`, `CiscoWebexStart.exe`), messengers (WhatsApp `5319275A.WhatsAppDesktop_*`/`WhatsApp.exe`, `Telegram.exe`, `Signal.exe`), browsers (opt-in) | — | A renamed or new call app |
| Mute | `IAudioEndpointVolume::SetMute` on the default capture endpoint (`audio::set_capture_muted`), its change event | Yes | An app recording a microphone that isn't the default isn't muted (as the indicator, D5) |
| Open the app | The app's top-level window by its exe (`media::player::app_windows`, `focus`); packaged apps by their exe in the package folder. The island allowed the foreground with the click (`AllowSetForegroundWindow`) | — | A call app with only a tray icon has no window to bring |

## 7. Protocol

No change: `Element::Clock`, `Headline::Clock`, `Art::Image`, glyphs and tones, `Row::Buttons`.

## 8. Settings (`privacy.json`, page "Mic & camera", group "Calls")

| Key | Default | |
|---|---|---|
| `calls` | on | Calls at all (needs the privacy module on) |
| `call_teams`, `call_zoom`, `call_discord`, `call_slack`, `call_webex` | on | Each app |
| `call_messengers` | on | WhatsApp, Telegram, Signal |
| `call_browsers` | **off** | A browser holding the mic counts as a call, named as the browser (D2) |

## 9. Milestones

| M | Scope | Exit |
|---|---|---|
| **M0: Detection** ✅ | `calls.rs`: known apps, settle, end grace, time; tests on a fake clock | C1, C7–C9, C11 by unit tests |
| **M1: Card** ✅ | `call_view.rs`, the module's wiring (supersede the indicator, mute, Open), settings and the Settings rows | C2–C5 by unit tests; the card on screen in a sandboxed hub |
| M2: Which microphone | Core Audio capture sessions (privacy M2): mute the session's own endpoint when it isn't the default | |

## 10. Open decisions

| ID | Question | Proposal |
|---|---|---|
| D1 | Settle time | 5 s: longer than a voice note's start, short enough that joining feels instant |
| D2 | Browsers | Off by default, and no tab titles. A window title shows only the front tab of each window (a Meet call in a background tab reads as something else), and reading every browser window's title to find a call is more than this needs to know. When on, any browser holding the mic for 5 s is "Call · Chrome" |
| D3 | Accent while muted | Stays green; the mute is said by the red mic-off and the detail, as iOS keeps a call green while muted |
| D4 | Two call apps at once | Two activities (`call:<app>`); the newest in the pill, the other in the bubble |
