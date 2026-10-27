# PRD: Now Playing (music and media)

Status: decisions D1–D8 accepted Oct 28 (D1 revised: copy shows in the bubble); M0–M5 built Oct 28. Hands-on checks pending (section 10) · Owner: Rohit · Feature #2 in [PLAN.md](../PLAN.md) · Oct 28, 2026

**References**
- `design/reference/inspiration/colored-activities.jpg`:
  - The "STAY" card: the whole card is tinted from the artwork, with title + artist and a ♥ at the right, a scrubber with elapsed and total times, and a controls row (shuffle · prev · big play · next · repeat).
  - The Spotify compact: the app's logo leading, live bars in the app's colour trailing.
- `live-activities-grid.jpg`, the book card: artwork left, title and author, a badge top right, a progress bar with what's left ("−281p"), page dots for more activities, and a soft glow of the artwork's colour.
- `figma-kit-presentations.png`: compact, minimal and expanded are one activity seen at three sizes.

**Inputs:** a code audit of `media.rs`, `artwork.rs` and the island's media drawing, and a study of competitors and the platform. Competitors: iPhone Now Playing, boring.notch, NotchNook, Alcove, DynamicLake, Atoll, FluentFlyout, ModernFlyouts, Windows 11's media flyout, and Windows island projects on GitHub. Platform sources: Microsoft docs, LRCLIB and Spotify's API terms. Both are summarised in the appendix.

---

## 1. Problem

Music is the activity people see most, so it is where an island earns trust. Today ReUI shows the playing track with artwork, a live timeline, seek, and prev/play/next for whatever Windows considers the current media session. It is good on the happy path and fragile around it:
- **Fragile:** one failed WinRT call ends the module for good; a hung artwork read freezes the whole hub (clipboard and agent permissions included); the timeline jumps after a pause.
- **Limited:** the bars are fake, there's no way to pick which player is shown, a song paused yesterday keeps the island awake, and there are no lyrics, volume or output controls.

## 2. Goals

1. **Always right.** The island shows the player you're actually listening to, its true position, and only the controls that work. It never sticks on a stale or closed source.
2. **Feels alive, costs nothing.** The visual reacts to the actual audio while playing, and costs zero when paused or hidden (no timers, no capture).
3. **Everything at the top of the screen.** The things you'd otherwise switch apps for: play/pause/skip, scrub, shuffle/repeat, volume, output device, lyrics, and switching player.
4. **Never blocks the rest of ReUI.** Media work never stalls the hub thread.

## 3. Non-goals (v1)

- Like/♥, queue or "up next", playlists. GSMTC has none of these; Spotify's Web API now needs Premium, OAuth and an allowlist of at most 5 users.
- Per-tab browser control. Windows exposes one session per browser.
- Casting (Chromecast/AirPlay) and Spotify Connect device lists.
- Video playback UI beyond what music gets (no picture-in-picture).

## 4. Scenarios

| # | Scenario | Today | v1 |
|---|---|---|---|
| S1 | Play a song in Spotify | Compact: art + fake bars; card with timeline and controls | Same, with bars driven by the real audio and a card tinted from the art |
| S2 | Spotify paused, YouTube starts in Chrome | Usually switches (Windows' choice) | Always switches to what's playing; ‹ › to go back to Spotify |
| S3 | Pause music and walk away | Island shows it forever | Rests; after 10 min paused it retracts; comes back when playback resumes |
| S4 | Resume after a 5-min pause | Timeline may jump 5 min ahead | Correct position |
| S5 | Podcast at 1.5× | Timeline drifts until the next event | Advances at 1.5× |
| S6 | Scrub to the chorus | Click to seek only | Drag the bar with a time preview, and seek on release |
| S7 | Turn it down without the mixer | Not possible | Volume slider for that app in the card |
| S8 | Switch to headphones | Not possible | Output device picker in the card |
| S9 | Sing along | Not possible | Opt-in synced lyrics: the current line in the card |
| S10 | Player hangs while giving artwork | **Whole hub freezes** | Artwork times out; the rest of ReUI is unaffected |

## 5. Experience

### 5.1 Presentations

- **Compact** (Apple's two-part rule: leading and trailing read as one):
  - Leading: the artwork (concentric radius).
  - Trailing: the visualizer in the artwork's accent, live while playing and resting when paused.
  - No text, matching today and Apple.
  - With lyrics on, the current line can replace the visualizer (*D3*).
- **Minimal (bubble):** live bars, like the pill (Rohit asked for motion in the bubble; the art-in-bubble idea was dropped). The audit found it shows bars only today; Apple shows the art there. Bars sit over the art's bottom edge while playing.
- **Expanded card:**

```
╭────────────────────────────────────────────────────────╮
│ ╭──────╮  Spotify · Album                     ‹ 1/2 ›  │  eyebrow: app · album; source switcher when >1
│ │ art  │  Title of the song                    ▂▅▇▃    │  hero: title; trailing: visualizer
│ ╰──────╯  Artist                                       │  detail: artist
│  1:23 ━━━━━━━━━━━━●──────────────────────────── −2:01  │  scrubber, elapsed / remaining
│   ⇄      ⏮      ⏯      ⏭      ↻        🔊━━━━━   🎧   │  transport row + volume + output
│  "and I'm so sorry, I'm so sorry…"                     │  lyrics line (when on)
╰────────────────────────────────────────────────────────╯
```

  - **Card colour (*D7*):** a subtle wash of the artwork's accent from the top-left (like the reference), with no blur. Text stays white or secondary, and the contrast of the accent against black is checked (≥ 3:1 for the bar, ≥ 4.5:1 for any accent text).
  - **Times:** elapsed on the left, **remaining** (−) on the right, both tabular. The audit found times skip (0:09 → 0:11) and don't add up; the tick must be aligned to whole seconds of the position.
  - **Controls:** only those the session advertises (`Is*Enabled`): shuffle, prev, play/pause, next, repeat (off / all / one). Volume and output need audio access (5.4).
  - **No timeline** (browser pages that don't call `setPositionState`, live radio): no scrubber row. Live streams show a quiet "LIVE" fact instead of times.
  - **Source switcher:** "‹ 1/2 ›" when more than one session exists; click or wheel cycles players.
  - **Video sources** (YouTube, Netflix in a browser): art cropped to its content (letterbox bars trimmed before the accent is extracted), and the same controls.

### 5.2 Which player is shown (session policy)

ReUI keeps its own ranking instead of trusting `GetCurrentSession` alone:
1. A source the user picked with the switcher, for as long as it exists and has played in the last 10 min.
2. Otherwise the session that is **Playing**. If several are playing: the one that started most recently; ties go to Windows' current session.
3. Otherwise the most recently playing session, **paused**, until it has been paused 10 min (*D6*).
4. Otherwise nothing.

All sessions are subscribed (`PlaybackInfoChanged` on each), so "B started playing" is seen even if Windows doesn't change its current session. Player names are readable: AUMIDs map to friendly names ("Firefox", "Movies & TV", "YouTube Music" for Chrome PWAs where the window title says so).

### 5.3 Timeline

- Position = `Position + (now − LastUpdatedTime) × PlaybackRate` while playing, computed by the island. The protocol gains `rate`.
- **Resume:** when status turns Playing without a fresh timeline stamp, the hub re-stamps `at = now` with the last known position (fixes the jump in S4).
- **Stale stamps:** a stamp older than 2 s while playing, with no event since, is re-stamped too.
- **Missing timeline:** if `LastUpdatedTime` is the epoch (1601-01-01) or length is 0, hide the scrubber.
- **Seek:**
  - Only when `IsPlaybackPositionEnabled`, clamped to `MinSeekTime..MaxSeekTime`.
  - Drag shows a preview time and seeks on release.
  - Optimistic: the bar holds the new position until the player confirms or 2 s pass.
- **Event storms:** timeline events that land within 500 ms of the island's own prediction aren't re-published.

### 5.4 Audio: visualizer, volume, output

- **Visualizer (*D2*):**
  - **Capture:** per-process WASAPI loopback (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`, Windows 10 2004+) for the shown session's process tree. The session's AUMID or exe is matched to the root process; for browsers, the browser's root process so the audio child is included.
  - **Processing:** a real FFT (`realfft`) into 4–5 bands, at the existing bars timer rate (30 fps).
  - **When it runs:** capture is open **only while playing and visible**, and closes within 1 s of pause, hide or switch.
  - **Fallback:** if matching or activation fails, today's synthetic bars.
  - **Privacy:** everything is in memory, only band levels are kept, nothing is recorded or sent, and calls in other apps are never captured.
  - **Setting:** Visualizer = Real (default) / Simple (synthetic) / Off.
- **Volume:** the shown app's own volume through `IAudioSessionManager2`, matching processes the same way, as a slider in the card. Wheel over the slider adjusts it; mute on click of the speaker glyph. If no audio session matches, show system volume (`IAudioEndpointVolume`) and say so with the glyph.
- **Output device (*D4*):**
  - A picker listing active render endpoints with their names; choosing one sets the **system default** for all roles via the undocumented `IPolicyConfig` (what SoundSwitch and EarTrumpet use).
  - Guarded by a capability check at start. If it fails, the button opens `ms-settings:sound` instead.
  - No per-app routing (undocumented and broken across Windows 11 builds).

### 5.5 Lyrics (*D3*)

- **Source:** LRCLIB only (free, no key). No Musixmatch, NetEase, QQ or Spotify internal lyrics.
- **Matching:**
  - Exact `GET /api/get` with track, artist, album and duration (±2 s).
  - Otherwise `/api/search`, filtered by the closest duration (reject anything over ±5 s).
  - Titles are cleaned first: strip "(Remastered)", "- Topic", "feat.", and split YouTube's "Artist - Title".
- **Requests:**
  - Prefetched on track change.
  - Cached on disk, including "no lyrics" results (7 days).
  - A descriptive User-Agent, and backoff on 429/503.
- **Display:**
  - Synced lyrics show the current line in the card, cross-fading at line changes. Plain lyrics show nothing; there's nothing to sync.
  - The compact pill can show the line instead of bars (setting).
  - Hidden when there's no timeline, and for ads. *Video sources get lyrics too (Oct 28): music is often played as YouTube videos, and LRCLIB only matches real songs.*
- **Privacy:** every track title goes to lrclib.net, so this is **off by default**, with one toggle that says exactly that.

### 5.6 Input

| Input | Action |
|---|---|
| Click compact / bubble | Open card (as today; bubble swaps first) |
| Wheel over the card | Cycles activities (as today); over the volume slider it changes volume; over "‹ 1/2 ›" it switches source |
| Horizontal swipe on the compact pill (trackpad) | Next / previous track (*D8*) |
| Click artwork in the card | Bring the player's window to the front (packaged apps: `IApplicationActivationManager`; Win32: its main window) |
| Drag the scrubber | Scrub with preview, seek on release |
| Keyboard, while the card is summoned | Space play/pause, ←/→ ±10 s (when seekable), ↑/↓ volume. Media keys keep working through Windows |

Summoning the card by keyboard (a media hotkey) is **not** in v1; the card opens by hover/click as today.

### 5.7 Lifecycle and priority

- **Appears:** on Playing (or a new session that has metadata).
- **Rests:** on Paused or Stopped, with resting bars, no timers and no capture.
- **Retracts:**
  - after 10 min paused (*D6*);
  - immediately on Closed;
  - after the 2 s grace when the session disappears. During the grace the view is marked not playing, so the bars and timeline stop (the audit found they keep moving).
- **Priority:** Ongoing.
  - A copy never covers it: the copy icon shows in the bubble for 2.5 s (*D1*).
  - Agent decisions always win.
  - Ties with agent sessions: the most recently started is in front, the other in the bubble (as today).

## 6. Edge cases and required behaviour

**Robustness**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-R1 | Subscribing to a session fails (app closed mid-switch) | Log, skip that session, keep running | `?` ends the module for good (media.rs:119-121) |
| E-R2 | GSMTC manager unavailable at login | Retry with backoff (1 s → 30 s) | Module ends (media.rs:89) |
| E-R3 | Artwork read hangs | `spawn_blocking` + 2 s timeout; the hub thread is never blocked | `.join()` on the only hub thread (media.rs:297, 300) |
| E-R4 | Module panics | The hub restarts the module after 1 s and logs to `hub.log` (panic hook added Oct 28) | Disappears silently |
| E-R5 | Hostile `seek:` value | Clamp; no overflow | `ms * 10_000` can overflow (media.rs:156) |

**Sessions**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-S1 | Several players | Policy in 5.2; switcher | Windows' current or first playing (media.rs:187-195) |
| E-S2 | B starts playing, Windows keeps A current | Switch to B | Stays on paused A (guess) |
| E-S3 | No current session, paused ones exist | Show the most recent paused one | Nothing |
| E-S4 | Session identity | Compare by `SourceAppUserModelId` + per-session object | COM pointer (media.rs:111) |
| E-S5 | Unreadable player names (Firefox hash AUMID, ZuneVideo) | Friendly names table + exe fallback | Raw |
| E-S6 | Spotify ads | Recognised (title "Advertisement" / no artist + Spotify); quiet "Ad" fact, no lyrics lookup | Shown as a track |
| E-S7 | Video in a browser | Letterbox trimmed; same card | Centre-cropped 16:9 frame |
| E-S8 | Metadata arrives in bursts (blank title, then title, then art) | Debounce 150 ms; don't show a blank title; swap the art when it arrives | Shows each step |

**Metadata and artwork**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-M1 | Thumbnail > 4 MB | Downscale while reading (WIC) or reject; never send truncated bytes | Truncated bytes sent, grey tile forever |
| E-M2 | Thumbnail stream size 0/unknown | Read to EOF with a cap | Empty image, grey tile |
| E-M3 | No artwork | Music glyph tile in the accent of the app | Grey tile |
| E-M4 | Greyscale artwork | Neutral (white/grey) accent | Pink default; a 1 % vivid speck can win |
| E-M5 | Long or RTL titles | One line, ellipsis; RTL direction from content | Ellipsis; RTL unverified |
| E-M6 | Artwork re-read on every session event | Only on metadata change for the shown session | Any session event triggers a blocking re-read |

**Timeline**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-T1 | Resume without a new stamp | Re-stamp at resume | Jumps ahead by the pause length |
| E-T2 | Stale stamp while playing | Re-stamp if > 2 s old | Runs to the end |
| E-T3 | Playback rate ≠ 1 | Protocol `rate`; island advances at it | Ignored |
| E-T4 | No timeline / live | No scrubber; "LIVE" for live streams | No scrubber |
| E-T5 | Seek limits | Min/MaxSeekTime | Start/End |
| E-T6 | Seek reported `true` but ignored | Optimistic hold 2 s, then snap back to the reported position | n/a |
| E-T7 | Tick alignment | Ticks on whole seconds of the position; remaining = length − elapsed rounded consistently | Skips seconds |

**Controls and audio**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-A1 | Rapid clicks | Optimistic play/pause glyph; one request in flight per control | Waits for the player's event |
| E-A2 | `Try*` returns false | Snap the glyph back; no error toast | Ignored |
| E-A3 | Process can't be matched (sandboxed or UWP host) | Synthetic bars; system-volume slider | n/a |
| E-A4 | Default device changes during capture | Reopen capture on the new endpoint | n/a |
| E-A5 | Exclusive-mode app or DRM path | Capture yields silence: fall back to synthetic bars after 2 s of silence while "playing" | n/a |
| E-A6 | `IPolicyConfig` unavailable | The output button opens Sound settings | n/a |

**Visibility**

| ID | Case | Required behaviour | Today |
|---|---|---|---|
| E-V1 | Fullscreen game or video | Island hidden (as today); capture stopped | Hidden; bars timer stops |
| E-V2 | Paused for hours, "When active" mode | Retracted after 10 min, so the island can hide | Stays forever |
| E-V3 | Reduced motion | Visualizer frozen at its level; cross-fades instead of motion | Bars stop |

## 7. Performance budget

- **Idle or paused:** 0 % CPU, no timers, no capture stream.
- **Playing, island visible:**
  - Hub + island under 1 % of a core on the user's machine, including capture + FFT at 30 fps.
  - DWM no worse than today's bars (they're island-drawn at 30 fps; see DESIGN "no compositor loops").
- **Track change to new art on screen:** under 300 ms. Artwork decode stays off the hub thread.
- **Memory:** at most 2 artworks per session in the hub (current + incoming), each downscaled to 512 px before sending.

## 8. Settings (v1)

All in `%APPDATA%\ReUI\media.json`, with the important ones in the tray:
- Visualizer: Real / Simple / Off.
- Lyrics: off by default.
- Lyrics in compact: off.
- Retract after pause: 10 min.
- Swipe to skip: on.
- Show video sources: on.

## 9. Milestones

| M | Scope | Exit criteria |
|---|---|---|
| **M0: Hardening** (*built Oct 28, 2026; resume-after-pause and the 10-min retract still need a hands-on check*) | E-R1–R5, E-M1–M2, E-M6, E-T1–T2, E-V2 retract after pause, grace stops motion, E-T7 tick alignment | No hub stall with a hung player; module survives any session error; timeline correct after pause/resume in Spotify, Chrome and the Media Player app |
| **M1: Sessions** (*built Oct 28, 2026*) | 5.2 policy, all-session subscription, switcher, friendly names, ads, video letterbox, burst debounce | S2 works across Spotify ⇄ Chrome ⇄ Media Player; no blank-title flicker |
| **M2: Card** (*built Oct 28, 2026*) | 5.1 layout (wash, remaining time, shuffle/repeat, LIVE), drag-scrub, protocol `rate`, optimistic controls, art click opens the app, minimal shows art | Matches the reference layout; S5 and S6 |
| **M3: Real visualizer** (*built Oct 28, 2026*) | 5.4 capture + FFT + fallbacks, setting | Bars follow the music; capture closes on pause; within the CPU budget |
| **M4: Volume and output** (*built Oct 28, 2026*) | Per-app volume slider, system fallback, output picker with Settings fallback | S7 and S8 |
| **M5: Lyrics** (*built Oct 28, 2026*) | 5.5, opt-in, cache | S9 with Spotify and YouTube in Chrome |
| v2 | ♥ via a user-supplied Spotify app; queue/up next; media hotkey to summon the card | — |

## 10. Acceptance test plan

- **Script:** `tools/media-states.ps1` drives the Windows Media Player app (a local test file) through play, pause, seek, rate, next, close, with screenshots at each step. Like `claude-states.ps1`, it cleans up after itself.
- **Unit tests:**
  - session ranking;
  - timeline maths (rate, re-stamp, stale stamps, tick alignment);
  - lyric line lookup and title cleaning;
  - letterbox trimming;
  - AUMID → friendly name.
- **Manual matrix:**
  - Players: Spotify, Chrome (YouTube, YouTube Music PWA), Edge, Firefox ≥ 125, Media Player, VLC (Store), Apple Music for Windows.
  - Each with two players at once, a long pause, a fullscreen video, a Bluetooth headset switch, and 150 % / 200 % scaling.

## 11. Open decisions

| ID | Question | Proposal |
|---|---|---|
| D1 | Copy toast above playing music? | **Decided Oct 28:** no. A copy shows as a copy icon in the bubble beside the music (priority `Brief`); music keeps the pill |
| D2 | Visualizer default | Real (per-process loopback) on by default; Simple and Off in settings |
| D3 | Lyrics | Off by default (titles go to lrclib.net); when on, the line in the card; compact optional |
| D4 | Output picker via undocumented `IPolicyConfig` | Yes, with a capability check and a Sound settings fallback |
| D5 | ♥ / like | Not in v1 (no Windows API; Spotify's terms make it a power-user add-on at best) |
| D6 | When does a paused player leave? | After 10 min paused (setting); back on resume |
| D7 | Card tinted from the artwork | **Decided Oct 28: no.** The wash looked muddy; the card stays black and colour comes from the art, bars, progress and eyebrow. (The protocol's `tint` remains, unused.) |
| D8 | Swipe on the compact pill to skip | Yes (trackpads); a mouse wheel keeps cycling activities |

## Appendix: sources

- **Code audit** of `media.rs`, `artwork.rs`, `pill.rs`, `render.rs`, `order.rs`, `hub.rs` (Oct 28, 2026); its findings are in the "Today" column.
- **Microsoft:**
  - [GSMTC GetCurrentSession](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionmanager.getcurrentsession)
  - [LastUpdatedTime](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessiontimelineproperties.lastupdatedtime)
  - [Process loopback params](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params)
  - [The Old New Thing on the "current" session](https://devblogs.microsoft.com/oldnewthing/20231108-00/?p=108980)
- **GSMTC behaviour:**
  - [pipe01: GSMTC notes](https://blog.pipe01.net/2021/01/gsmtc.html)
  - [ModernFlyouts app support table](https://raw.githubusercontent.com/ModernFlyouts-Community/ModernFlyouts/main/docs/GSMTC-Support-And-Popular-Apps.md)
  - [Firefox 125 timeline](https://bugzilla.mozilla.org/show_bug.cgi?id=1689538)
  - [Chrome `setPositionState`](https://caniuse.com/mdn-api_mediasession_setpositionstate)
- **Competitors:**
  - [boring.notch](https://github.com/TheBoredTeam/boring.notch/releases) ([CPU issue #1260](https://github.com/TheBoredTeam/boring.notch/issues/1260))
  - [Atoll](https://github.com/Ebullioscopic/Atoll)
  - [DynamicLake](https://www.dynamiclake.com/changelogoldversions)
  - [FluentFlyout](https://github.com/unchihugo/FluentFlyout/blob/master/README.md)
  - Windows island projects: [lepi-da-vinci/dynamic-island](https://github.com/lepi-da-vinci/dynamic-island) (WASAPI EQ + LRCLIB), [mediaisland](https://github.com/HalilErenPab/mediaisland) (letterbox trim)
  - [Apple HIG Live Activities](https://developer.apple.com/design/human-interface-guidelines/live-activities)
- **Audio:**
  - [ProcessLoopbackCapture](https://github.com/Naseband/ProcessLoopbackCapture)
  - [SoundSwitch's IPolicyConfig use](https://github.com/Belphemur/SoundSwitch/commit/40f5ba119a41d04b8057e75146345c84dfe238c3)
- **Lyrics:**
  - [LRCLIB](https://github.com/tranxuanthang/lrclib) ([UA/rate limits #130](https://github.com/tranxuanthang/lrclib/issues/130))
  - [Musixmatch free tier limits](https://freeapihub.com/apis/musixmatch)
- **Spotify:**
  - [Developer-mode changes, Feb 2026](https://techcrunch.com/2026/02/06/spotify-changes-developer-mode-api-to-require-premium-accounts-limits-test-users/)
  - [Quota modes](https://developer.spotify.com/documentation/web-api/concepts/quota-modes)
