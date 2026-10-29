# PRD: Privacy indicators (microphone, camera, screen, location in use)

Status: M0–M1 built Oct 30, 2026 · Owner: Rohit · Feature #9 in [PLAN.md](../PLAN.md) · Oct 30, 2026

**References:**
- **iPhone:** a green dot when an app uses the camera (with or without the mic) and an orange dot for the microphone alone, top right of the status bar; Control Center then names the app ("Camera in use by FaceTime") and recently used ones. A screen recording turns the status-bar time into a red capsule. The dot is a fact, not a control: no buttons, no prompt ([Apple: the orange and green indicators](https://support.apple.com/en-us/108331)).
- **macOS:** an orange dot by Control Center in the menu bar for the mic, green for the camera, purple while the screen or system audio is recorded; Control Center names the apps ( [Apple: Control Center](https://support.apple.com/guide/mac-help/quickly-change-settings-mchl50f94f8f/mac)).
- **Windows 11:**
  - A mic, camera or location glyph appears in the notification area only while something uses it; hovering names the app ([askvg](https://www.askvg.com/enable-camera-on-off-indicator-notification-in-windows-11/), [WindowsForum](https://windowsforum.com/news/windows-sensor-icons-your-tiny-privacy-alerts-for-microphone-and-location.390894/)). It lives in the tray overflow by default, so most people never see it. Settings › Privacy & security › Microphone / Camera / Location lists "Currently in use" and "Last accessed" per app from the same store this feature reads.
  - Screen capture through Windows.Graphics.Capture draws a border around what's captured ([Microsoft Learn: screen capture](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture)); the Insider build 26340.9482 turns it red ([release notes](https://learn.microsoft.com/en-us/windows-insider/release-notes/experimental/preview-build-26340-9482)). There's no tray indicator for screen capture and none announced.
  - No public API says who is using a device: `AppCapability` checks or requests access for a process, its `AccessChanged` reports permission changes, not use ([Microsoft Learn](https://learn.microsoft.com/en-us/uwp/api/windows.security.authorization.appcapabilityaccess.appcapability)).

**Inputs:** web research (sources inline and in the appendix), a read of this machine's consent store (Windows 11 25H2, 26200), and ReUI's building blocks: the shared Core Audio module (`modules/audio`: the default microphone's mute and its change events), the clipboard's exe icon and app naming, `Element::Dot`, the Brief priority, and the settings-file pattern.

## 1. Problem

On Windows you can't tell, at a glance, that your camera or microphone is on, or which app has it. The tray glyph is hidden in the overflow and says nothing about screen sharing. A call that ended but left the mic open, a browser tab still holding the camera, a recorder you forgot: these are exactly the quiet, always-true facts the island is for.

## 2. Goals

1. **Always know.** While any app uses the microphone, camera or captures the screen, the island shows it, with the app's icon and the iOS colour for what it uses.
2. **Know who.** One look at the open card names every app and what each uses, and since when.
3. **One action that matters.** Mute the microphone from the card. Everything else is Windows' to decide ("Privacy settings").
4. **Never wrong.** No dot for ReUI itself, no dot left behind by an app that crashed, no flash for a screenshot.
5. **Free when idle.** Event-driven: a registry change notification and process waits. 0 % CPU while nothing changes.

## 3. Non-goals (v1)

- Blocking or revoking access (that's Windows' Settings; ReUI links to it).
- A history of past use (Settings › Privacy shows "Last accessed"; the CapabilityAccessManager database has more).
- Desktop Duplication / GDI screen capture (no OS record exists, see §6).
- Per-app mute (the mic button mutes the default microphone, as the Windows key + Alt + K call mute does).
- Calls (who you're talking to, hang up): PLAN.md's "Calls" feature.

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| S1 | A Teams call starts with the camera | The pill shows Teams' icon and a green camera glyph; minimal bubble: a green dot |
| S2 | Only the mic (a voice note, Discord) | Orange mic glyph; open: "Microphone", "Discord", "Since 10:42", mute + Privacy settings |
| S3 | Mute from the island | Mic glyph turns to mic-off; the card says "Muted"; Windows' own mute agrees |
| S4 | OBS records the screen | Purple screen glyph, OBS's icon |
| S5 | Snipping Tool screenshot | Nothing (a capture under 1 s) |
| S6 | A second app starts the camera mid-call | A Brief: that app's icon and the camera glyph, 3 s (in the bubble if the pill is taken) |
| S7 | Chrome crashed while sharing, never wrote its stop | Nothing (no process from before the share is running) |
| S8 | The hub starts mid-call | The indicator shows at once, no Brief |
| S9 | The island's visualizer captures loopback audio | Never counted |
| S10 | Music is playing when the mic goes on | The indicator is the newest Ongoing activity, so it takes the pill; music goes to the bubble; a wheel brings it back |

## 5. Experience

### 5.1 Presentations

- **Indicator (Ongoing, key `in-use`):** published while anything is in use, retracted when nothing is.
  - **Accent:** the most important kind in use: screen `#BF5AF2` (purple) › camera `#30D158` (green) › microphone `#FF9F0A` (orange) › location `#0A84FF` (blue). Apple's system colours, as in iOS (camera green, mic orange) and macOS (screen recording purple).
  - **Compact:** leading the main app's icon (a 24 circle, `Art::Image`; the kind's glyph on a tinted tile until the icon arrives); trailing the kind's glyph in the accent. Muted mic: a quiet mic-off glyph instead.
  - **Minimal:** `Element::Dot` in the accent: exactly the iOS dot.
  - **Expanded** (eyebrow → hero → detail):
    - eyebrow: what the main app uses ("Camera and microphone");
    - hero: the app's name ("Teams");
    - detail: one fact: "Since 10:42" (or "Muted");
    - rows: one quiet line per other app ("Chrome · Microphone"), then the buttons: a mic / mic-off glyph button while a microphone is in use (accent-tinted while muted) and **Privacy settings** (Settings › Privacy & security, on the page for the main kind).
- **Started (Brief, key `started`, 3 s):** another app or kind started while the indicator was up: its icon and the kind's glyph, in that kind's colour. No card; clicking opens the indicator.

### 5.2 Priority rules

- The indicator is **Ongoing but quiet**: it never expands itself, never alerts, never makes a sound. It takes the pill only because it's the newest Ongoing activity (the hub's rule for equal priorities), and any Decision or Alert goes in front of it.
- **Something new is a Brief:** a second app or kind joining (S6). The first use needs none: the indicator appearing says it.
- Fullscreen: hidden with the rest of the island (`hide_in_fullscreen`); a game or a video call in fullscreen knows it's on.

### 5.3 Showing and going

- **Screen settles for 1 s.** A screenshot is a capture that starts and stops within milliseconds (the store on this machine holds Snipping Tool captures 17 ms long); only a capture still running after `SCREEN_SETTLE` is a share or a recording.
- **Camera, microphone and location show at once**, and a use that ends straight away still shows for `MIN_SHOWN` (3 s), so a single grabbed frame is seen.
- **A stopped-and-restarted session** of the same app is the same dot (Teams opens and closes the mic as you join).
- **Off** in Settings for a kind: it never shows; switched back on, what's in use shows at once.

### 5.4 What it never does

- Count ReUI's own programs (S9).
- Keep a dot for an app that isn't running (S7).
- Log app names or paths beyond the debug console.

## 6. Platform details

| Signal | How | Event? | Misses |
|---|---|---|---|
| **Microphone, camera, location: who and since when** | The Capability Access Manager's consent store, `HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{microphone,webcam,location}`: a subkey per packaged app (package family name) and, under `NonPackaged`, one per desktop exe (`C:#Program Files#…#app.exe`, `#` for `\`). Each has `LastUsedTimeStart` / `LastUsedTimeStop` (FILETIME QWORDs). **In use = start set and stop 0 or older than start.** It's what Settings › Privacy shows as "Currently in use", and it covers apps without package identity | **Yes:** `RegNotifyChangeKeyValue` on `ConsentStore`, subtree, `REG_NOTIFY_CHANGE_NAME \| REG_NOTIFY_CHANGE_LAST_SET`, async with an event. One-shot: re-armed before each read. Without `REG_NOTIFY_THREAD_AGNOSTIC` it's tied to the registering thread, so it's registered from the watcher's own long-lived thread ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regnotifychangekeyvalue)) | No process id (an app with several processes is one entry). Undocumented as an API: Microsoft only documents the Settings pages. Desktop apps "may not always appear" ([Microsoft Support](https://support.microsoft.com/en-us/windows/privacy/windows-camera-microphone-and-privacy)): drivers and apps that bypass the frame server or audio engine. A crash can leave the stop unwritten |
| **Screen capture** | The same store's `graphicsCaptureProgrammatic` (Windows.Graphics.Capture, "Screenshots and screen recording" in Settings) and `graphicsCaptureWithoutBorder` ("Screenshot borders"): **both carry `LastUsedTimeStart` / `LastUsedTimeStop` on 26200** (seen here for Discord, Chrome and Snipping Tool). One capture may write both keys; they merge (earliest start) | Yes (same watch) | Desktop Duplication (`IDXGIOutputDuplication`) and GDI `BitBlt` leave no record and no border: older recorders and some remote-desktop tools are invisible. Other processes' `GraphicsCaptureSession`s can't be enumerated |
| **A use's process is still alive** | ToolHelp snapshot, then `QueryFullProcessImageNameW` + `GetProcessTimes` (`PROCESS_QUERY_LIMITED_INFORMATION`). A use counts only while a process of that app (same exe path; for packaged apps, an exe in the package's `WindowsApps\Name_…_PublisherId` folder) is running **and was created before the use started** (2 s slack) | **Yes:** the watcher also waits on a `SYNCHRONIZE` handle to every holding process, so an exit (or crash) wakes it at once | Elevated or protected processes may refuse the query: trusted by file name (a dot that might be stale beats hiding a live camera). Waits are capped at 62 processes (`WaitForMultipleObjects`' 64 minus two) |
| **Microphone mute** | `IAudioEndpointVolume` on the default capture endpoint (`eCapture`, `eConsole`): `GetMute`, `SetMute` tagged with ReUI's event context, and `IAudioEndpointVolumeCallback` for changes (the shared `modules/audio` watcher, `capture: true`) | Yes | Mutes the default microphone only; an app recording another one isn't muted |
| **Microphone, process level (M2)** | Core Audio capture sessions: `IAudioSessionManager2` on each active `eCapture` endpoint, `IAudioSessionNotification::OnSessionCreated`, `IAudioSessionEvents::OnStateChanged` (active / inactive), `IAudioSessionControl2::GetProcessId`. Process-exact, and knows which microphone. New-session notifications arrive only after a first `GetCount`, on an MTA thread ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessionmanager2-registersessionnotification)) | Yes | Multi-process sessions give `AUDCLNT_S_NO_SINGLE_PROCESS`; helper processes (Teams, Zoom) need mapping to their app; exclusive-mode streams unconfirmed. **Loopback capture opens the render endpoint** ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)), so the island's visualizer never appears here |
| Windows' own tray indicator | Driven by the Capability Access Manager service (camsvc), i.e. the same records ([winsides](https://winsides.com/capability-access-manager-service-on-windows-11/)) | — | Not readable by other apps |
| Usage history | `C:\ProgramData\Microsoft\Windows\CapabilityAccessManager\CapabilityAccessManager.db` (SQLite, WAL): `NonPackagedUsageHistory` etc. ([app-access-history](https://github.com/KotlaAvinash/app-access-history)) | No (written after the fact) | Not needed for "now"; noted for a history view |

**Engineering** (`crates/reui-hub/src/modules/privacy/`):
- `consent.rs` (pure): kinds and their capability keys, entries, "in use", ReUI's own exes, FILETIME → Unix; tests on a recorded snapshot.
- `alive.rs` (pure): a use and the processes that may hold it (path, package folder, creation time); tests.
- `store.rs`: the `reui-privacy` thread: registry walk, the change notification, process list and waits.
- `tracker.rs` (pure, clock passed in): screen settle, minimum shown, started-while-up Briefs; tests.
- `view.rs` (pure): the indicator and the Brief; kind labels; packaged app names; tests.
- `settings.rs`: `privacy.json`.
- `mod.rs`: the module loop; icons through the clipboard's `exe_icon` (`SHGetFileInfoW` → WIC PNG), names through its `app_name`; "Since" through the Timers' `time_of_day` (the user's own clock format).

**Cost:** each wake reads five capability keys (89 records here, well under a millisecond) and, only while something is in use, the process list. Idle: one thread blocked in `WaitForMultipleObjects`.

## 7. Protocol

No change. Everything is existing elements: `Art::Image` (icon), `Element::Glyph` with `Tone::Accent`, `Element::Dot`, `Row::Note`, `Row::Buttons` (glyph and text), the activity accent, Ongoing and Brief.

## 8. Edge cases

| ID | Case | Required behaviour |
|---|---|---|
| P-1 | ReUI's visualizer captures loopback audio | Never shown: any exe in the hub's own folder or named `reui-*.exe` is dropped before anything else (loopback is also on the render endpoint, so M2's sessions won't see it) |
| P-2 | Teams / Zoom / Chrome with many processes | The store has one entry per exe path, so one app is one line; liveness accepts any of its processes from before the use |
| P-3 | Several versions of one app (Discord's `app-1.0.9239` folders) | Different paths: only the one in use has a live entry |
| P-4 | Elevated app | Its use is recorded in the user's hive (to verify, §11); its process may refuse queries: trusted by file name, no exit wait (noticed on the next registry change) |
| P-5 | Session that never wrote its stop (crash, kill) | Dropped when no process from before its start runs; seen here: a 4-day-old Chrome screen capture with stop 0, correctly not shown |
| P-6 | Process exits without writing its stop | The wait on its handle wakes the watcher; the dot goes at once |
| P-7 | Screenshot, tab-picker preview | Under 1 s: not shown (§5.3) |
| P-8 | One capture writes both screen keys | One use, earliest start |
| P-9 | A kind's key doesn't exist yet (location never used) | Skipped; the subtree watch on `ConsentStore` sees it appear |
| P-10 | Windows Hello's IR camera | Not expected in the store (system component); to verify |
| P-11 | Packaged app hosted by another process (a PWA, a WebView) | Matched by its package folder; a browser-hosted app shows as the browser |
| P-12 | Mic muted elsewhere (Win+Alt+K, a headset button) | The mute event updates the card |
| P-13 | No default microphone | No mute button state (the button still shows while an app uses some mic; pressing it fails quietly) |
| P-14 | Hub restarts | The first snapshot shows what's in use at once, no Brief |

## 9. Performance budget

- **Idle:** 0 % CPU; one parked thread; no polling anywhere.
- **On a change:** < 1 ms registry read; a process snapshot (and a query per candidate) only while something is in use.
- **Memory:** icons (32 px PNGs) per app shown, kept for the session.

## 10. Settings (`%APPDATA%\ReUI\privacy.json`, page "Mic & camera")

| Key | Default | |
|---|---|---|
| `enabled` | on | Read when the hub starts (`--no-privacy` overrides) |
| `microphone` | on | |
| `camera` | on | |
| `screen` | on | Screen sharing and recording |
| `location` | **off** | Windows' widgets and Weather hold location for long stretches (D2) |

## 11. Milestones

| M | Scope | Exit |
|---|---|---|
| **M0: Store watcher** ✅ | `consent.rs`, `alive.rs`, `store.rs`: the registry walk, change notification, liveness and process waits; tests on recorded snapshots | The real store reads as "nothing in use" here, the stale Chrome capture dropped (done) |
| **M1: Indicator** ✅ | Module, views, tracker (settle, linger, Briefs), mic mute, Privacy settings, `privacy.json`, `--no-privacy`, Settings page | S1–S9 by unit tests; the sandboxed hub on screen |
| M2: Core Audio sessions | Capture sessions per process as a second microphone signal: names the exact process (Teams' helper), shows which microphone, catches desktop apps missing from the store | S2 with an app the store misses |
| M3: HKLM and history | Read `HKLM\…\ConsentStore` if elevated apps land there (§12, V2); recently used apps in the card | |

## 12. To verify

| ID | Question | How |
|---|---|---|
| V1 | Does `ms-settings:privacy-graphicscaptureprogrammatic` open "Screenshots and screen recording"? | Press Privacy settings during a screen share |
| V2 | Where does an elevated app's use land (HKCU or HKLM)? This machine's HKLM store holds only `svchost.exe` | Run a recorder elevated |
| V3 | Does every Windows.Graphics.Capture session write `graphicsCaptureProgrammatic`, or only apps that went through the consent prompt? | OBS, Teams, Chrome's tab share |
| V4 | Do packaged apps' processes always live in their package folder (the Camera app, Teams 2)? | Camera app |
| V5 | Windows Hello, IR cameras, virtual cameras (OBS Virtual Camera, NVIDIA Broadcast) | |

## 13. Open decisions

| ID | Question | Proposal |
|---|---|---|
| D1 | Which kind leads when several are in use? | Screen › camera › microphone › location: the most consequential to have forgotten |
| D2 | Location by default? | Off: rarely surprising on a desktop, and often held for long stretches by system widgets |
| D3 | Ongoing or Passive? | Ongoing (it's true right now, like music); Passive would hide it behind every other activity |
| D4 | A Brief for the very first use too? | No: the indicator appearing in the pill is the moment |
| D5 | Mute: the default microphone, or every microphone in use? | The default (what Windows' call mute and the OSD's mic moment use); M2 can mute the session's own endpoint |

## Appendix: sources

- **Microsoft:**
  - [RegNotifyChangeKeyValue](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regnotifychangekeyvalue)
  - [AppCapability](https://learn.microsoft.com/en-us/uwp/api/windows.security.authorization.appcapabilityaccess.appcapability)
  - [Screen capture (Windows.Graphics.Capture)](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture), [GraphicsCaptureSession.IsBorderRequired](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired)
  - [Audio sessions](https://learn.microsoft.com/en-us/windows/win32/coreaudio/audio-sessions), [RegisterSessionNotification](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessionmanager2-registersessionnotification), [IAudioSessionControl2::GetProcessId](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-getprocessid), [Loopback recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)
  - [Windows camera, microphone, and privacy](https://support.microsoft.com/en-us/windows/privacy/windows-camera-microphone-and-privacy)
  - Insider builds [26340.9482](https://learn.microsoft.com/en-us/windows-insider/release-notes/experimental/preview-build-26340-9482) (red capture border) and [26340.9233](https://learn.microsoft.com/en-us/windows-insider/release-notes/experimental/preview-build-26340-9233) (per-app camera, mic and location permissions for desktop apps)
- **The consent store in practice:**
  - [Neowin: what's using your camera, mic and location](https://www.neowin.net/guides/how-to-see-exactly-whats-using-your-camera-mic-and-location-in-windows-and-for-how-long/)
  - [Atomic Red Team T1125](https://github.com/redcanaryco/atomic-red-team/blob/master/atomics/T1125/T1125.yaml) (writes fake webcam usage under HKLM)
  - [trigo-whispr #9](https://github.com/llexa97/trigo-whispr/issues/9) ("in use" from the store's times)
  - [app-access-history](https://github.com/KotlaAvinash/app-access-history), [CAMtool](https://github.com/whitedevil1026/CAMtool) (the CapabilityAccessManager database)
  - [Cap #1588](https://github.com/CapSoftware/Cap/issues/1588) (an indicator stuck on until the app quits)
  - [ElevenForum: screen capture permissions](https://www.elevenforum.com/t/enable-or-disable-let-apps-take-screenshots-and-record-screen-in-windows-11.46527/)
- **Apple:** [About the recording indicators](https://support.apple.com/en-us/108331), [Control Center on Mac](https://support.apple.com/guide/mac-help/quickly-change-settings-mchl50f94f8f/mac)
