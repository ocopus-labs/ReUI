# PRD: System OSD (volume, brightness and system states)

Status: decisions D1–D6 accepted Oct 28; M0–M4 built Oct 28 (volume HUD verified live; brightness, flyout hiding and optional states untested on real hardware) · Owner: Rohit · Feature #5 in [PLAN.md](../PLAN.md) · Oct 28, 2026

**References:**
- **macOS notch apps** replacing Apple's HUDs:
  - Alcove (best-reviewed for feel; filters false triggers);
  - boring.notch (smooth stepping, ⌥⇧ fine steps, external displays);
  - MediaMate (shows the output device in the audio HUD);
  - SlimHUD (thin edge bars).
- **Windows:**
  - Windows 11's own flyout: a horizontal pill with no %. Since late 2025 it can be placed **top-centre**, right where the island is.
  - FluentFlyout (volume flyout; its bug #725 shows volume and brightness share one Windows host window);
  - ModernFlyouts (WMI brightness, shell-hook appcommands);
  - HideVolumeOSD and the Windhawk "hide-volume-brightness-osd" mod (hiding the native flyout);
  - Twinkle Tray (DDC/CI brightness for external monitors, brightness keys via Raw Input, ignoring its own echoes).

**Inputs:** a platform and competitor study (sources in the appendix) and a survey of ReUI's code. `audio.rs` already reads and sets system volume, mute and outputs, and watches for changes. Brightness and hiding the native flyout have no code yet.

## 1. Problem

Pressing the volume keys (or the brightness keys on a laptop) shows Windows' own flyout. It's a separate, generic surface, and with the 2025 "top-centre" option it can land right on the island. A dynamic island should own these moments: one place at the top of the screen that shows the change, lets you fine-tune it, and then gets out of the way.

## 2. Goals

1. **Every change is seen.** Volume, mute, output device and brightness changes made by keys, apps or Windows all show in the island within one frame of the event, and nothing fires for changes ReUI made itself.
2. **Adjustable right there.** Scroll or drag on the HUD to set the level, click to mute, switch the output device.
3. **Feels physical.** The level animates (no re-entry per step) and lingers about 1.5 s after the last change. Bursts read as one gesture.
4. **Never in the way.** It doesn't cover what you're doing (music keeps its place), doesn't show in fullscreen games, and costs 0 % CPU between changes.
5. **Optionally replaces Windows' flyout,** safely: off by default, always restored.

## 3. Non-goals (v1)

- Per-app volume mixer (EarTrumpet territory; the music card already has the player's own volume).
- Keyboard backlight (vendor-specific, no Windows API).
- Night light, camera privacy shutter, Do Not Disturb (no public change notifications).
- Polling external monitors for their own button presses (DDC/CI has no events).

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| S1 | Press volume up three times | The island shows the speaker glyph and a level bar that climbs through the three steps, then fades 1.5 s after the last press |
| S2 | Press mute | The glyph snaps to muted, the bar dims; again: back |
| S3 | Music is playing, then volume changes | Music keeps the pill; the volume shows in the bubble as a small level ring, then goes |
| S4 | Laptop brightness keys | Sun glyph + level bar, on the laptop screen's island |
| S5 | Scroll on the HUD | Level changes 2 % per notch (Shift: 1 %); no echo HUD from our own change |
| S6 | Plug in headphones (default device switches) | A brief "Headphones" device moment; no spurious volume HUD from the switch |
| S7 | Caps Lock on | A brief "Caps Lock" pill (optional state, see 5.4) |
| S8 | Playing a fullscreen game, press volume | Nothing from ReUI (Windows' own flyout, if not replaced) |
| S9 | "Replace Windows flyout" on | Only ReUI shows; turning it off, quitting or crashing ReUI restores Windows' flyout |
| S10 | External monitor via ReUI | Drag the brightness slider in the HUD's expanded view; the monitor follows (throttled) |

## 5. Experience

### 5.1 Presentations

- **Compact (the HUD):** the whole pill is the meter (revised Oct 29, 2026, from Rohit's sketch).
  - The pill is the track; an accent fill grows from the left, inset so its round ends stay concentric with the pill's, and animates between values (`Element::Level` as the compact trailing side).
  - The glyph sits in the fill's left end and changes by tier: speaker 0 / 1 / 2 / 3 waves, muted; sun dim / bright.
  - The percentage is always at the right end. Muted dims the fill and quiets the number. The "Show percentage" setting covers the open pop-up only.
  - **Audio device:** the device glyph or short name ("AirPods", "Speakers") replaces the speaker glyph when it isn't the usual speakers.
- **Minimal (bubble, when something else holds the pill):** a small **level ring** (`Element::Ring(level)`) with the glyph inside it.
- **Expanded (hover or click on the HUD):**
  - A slider row (`Row::Slider`, reused from music): glyph button = mute, drag/wheel = level, trailing menu = output devices (volume) or monitors (brightness, when several).
  - For brightness with several displays: one slider per display, labelled by monitor name.

### 5.2 Timing and motion

- **Appear:**
  - As a **Brief** activity, so it takes the pill when the island is idle and the bubble otherwise.
  - It has the volume's own key, so a second change *updates* the same activity instead of replaying the entrance.
  - The level bar eases to the new value in 120 ms.
- **Linger:** 1.5 s after the last change (2.5 s after a drag ends). While the pointer is over it, it stays.
- **Edges:** at 0 % and 100 % a further key press gives a tiny squash on the bar, a stand-in for a haptic.
- **Mute:** the glyph snaps (no fade); the bar drops to a dim track.
- **Reduced motion:** values jump, and the HUD cross-fades instead of morphing.
- **Visibility mode "When needed":** today a Brief never shows in that mode. The OSD gets an exception, since a change the user just made *is* needed (D5).

### 5.3 Where it shows

- **Volume:** the island on the monitor with the foreground window.
- **Brightness:** the island on the monitor whose brightness changed (internal panel, or the monitor ReUI just set).
- **Fullscreen** (games, presentations, D3D exclusive): no HUD; Windows' flyout is restored if it was hidden.

### 5.4 Optional states (setting, all off by default)

- **Caps Lock / Num Lock:** a brief pill. Detected through a low-level keyboard hook already used elsewhere, plus `GetKeyState`.
- **Battery saver on/off:** `GUID_POWER_SAVING_STATUS`.
- **Charging started/stopped:** `GUID_ACDC_POWER_SOURCE` with battery %. This overlaps the planned Devices feature (#6), which will own battery.
- **Microphone mute:** the endpoint-volume callback on the default capture device. The planned Mic/camera feature (#9) will own it.

## 6. Detection and control

| Signal | Detect | Set | Notes |
|---|---|---|---|
| Volume, mute | `IAudioEndpointVolumeCallback` on the default render endpoint: **use** `fMasterVolume`, `bMuted`, `guidEventContext` from the notification | `SetMasterVolumeLevelScalar` / `SetMute` with ReUI's context GUID | Drop notifications carrying our GUID (no echo HUD). Ignore changes within 500 ms of a default-device switch (S6) |
| Default device | `IMMNotificationClient::OnDefaultDeviceChanged` (render, multimedia) | `IPolicyConfig` (exists) | Rebind the volume callback (exists) |
| Internal brightness | `RegisterPowerSettingNotification(GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS)` (what Twinkle Tray uses by default); fallback WMI `WmiMonitorBrightnessEvent` | WMI `WmiMonitorBrightnessMethods.WmiSetBrightness` | Event-driven; ignore events for 400 ms after our own write |
| External brightness | none (DDC/CI has no events) | Dxva2 `SetMonitorBrightness` on a worker, trailing-edge throttle ≥ 150 ms per monitor | Shown only for ReUI-initiated changes; probe `MC_CAPS_BRIGHTNESS`; fail quietly |
| Keys (hint only) | `WH_KEYBOARD_LL` VK_VOLUME_* (exists in the hub's OS layer); brightness keys optional via Raw Input consumer page 0x0C usages 0x6F/0x70 | — | Values come from the change events, never from counting keys |

**Engineering changes:**
- `audio::watch` sends a typed event `Volume { level, muted, ours }` / `Device { id, name }` instead of `()`.
- A shared audio watcher, so media and the OSD don't register twice.
- A new `brightness.rs` (power-setting notification, WMI set, DDC/CI).
- A new `osd.rs` module.

## 7. Replacing Windows' flyout (opt-in)

- **How** (FluentFlyout's method for Windows 11):
  1. Find the `XamlExplorerHostIslandWindow` whose `DesktopWindowXamlSource` child hosts the OSD.
  2. Make it layered and transparent, move it off-screen, and minimise it without activating it.
  3. Cache the handle and re-find it if it dies.
- **Scope:** it's **all or nothing.** The same host shows volume, brightness, media, airplane mode and camera. So it's offered only as a whole ("Replace Windows volume & brightness flyout"), hidden only while ReUI is running and that setting is on.
- **Re-applying:** re-apply on `TaskbarCreated` (Explorer restart) and lazily on each of our shows. No polling timer.
- **Restore** on: setting off, quit, ReUI crash (the hub's island supervisor restores on island exit), and entering fullscreen.
- **Not found** (new Windows build, host not created yet): fall back to showing both. A one-line note goes in the log with the build number.

## 8. Edge cases

| ID | Case | Required behaviour |
|---|---|---|
| O-1 | Held volume key (auto-repeat ~30/s) | One activity, the bar follows; publishes coalesced to ≤ 30 Hz |
| O-2 | Change from an app (game, Spotify's own slider) | Shows like any change (it's the system volume) |
| O-3 | Our own drag/scroll | No echo; the HUD shows our value optimistically |
| O-4 | Device switch causes level jump | Show the device moment only; no volume HUD for 500 ms |
| O-5 | No audio device | No volume HUD; Windows' flyout restored if hidden |
| O-6 | Brightness on desktop PC (no internal panel) | Brightness OSD inactive unless a DDC/CI monitor exists; then only ReUI-initiated |
| O-7 | Auto-brightness / adaptive brightness changes | Ignore changes not preceded by a brightness key or user action within 1 s (setting "show automatic changes", off) |
| O-8 | Charger plugged in changes brightness | Same as O-7 |
| O-9 | DDC/CI write slow or failing | Optimistic UI; on failure, revert the bar and mark the monitor unsupported for this session |
| O-10 | Explorer restarts while flyout hidden | Re-find and re-hide on `TaskbarCreated` |
| O-11 | ReUI crashes with flyout hidden | Supervisor restores it; on next start, always restore before deciding |
| O-12 | Remote Desktop session | Volume of the remote audio endpoint; no brightness |
| O-13 | Music card open, volume key pressed | The card's slider moves; no separate HUD (the card already shows it) |

## 9. Performance budget

- **Idle:** 0 % CPU, no timers, no polling. Callbacks only.
- **During a change:** at most 30 publishes per second, 2 ms of hub work per event, island redraw of the compact surface only.
- **Brightness writes:** ≤ 20 Hz to WMI and ≤ 7 Hz to DDC/CI, on a worker thread.

## 10. Settings (`%APPDATA%\ReUI\osd.json`, tray toggles)

| Setting | Default |
|---|---|
| Volume HUD | on |
| Brightness HUD | on |
| Show percentage | off |
| Replace Windows flyout | **off** (D2) |
| Show automatic brightness changes | off |
| Lock keys (Caps/Num) | off |
| Battery saver | off |
| Scroll step | 2 % |

## 11. Milestones

| M | Scope | Exit |
|---|---|---|
| **M0: Volume HUD** (*built*) | Typed audio events (level, mute, ours), shared watcher, `Element::Level` (protocol), `osd.rs` with the Brief volume activity, glyph tiers, linger, minimal ring, fullscreen skip, "When needed" exception | S1–S3, S8 |
| **M1: Interact** (*built*) | Expanded slider (drag, wheel, mute, output menu), no-echo, device-switch moment, O-1–O-4, O-13 | S5, S6 |
| **M2: Brightness** (*built*) | Internal panel (power-setting event + WMI set), external DDC/CI (set only), per-monitor island, O-6–O-9 | S4, S10 |
| **M3: Replace Windows flyout** (*built*) | Opt-in hide/restore, TaskbarCreated, crash restore, fallbacks | S9; flyout always back after quit or crash |
| **M4: Optional states** (*built*) | Caps/Num Lock, battery saver, charging, mic mute (behind settings) | S7 |
| **M5: Mixer** (*built*) | Each playing app's level under the volume slider (`Row::Mixer`, protocol 22): icon = the app's mute, name over its bar, scrolling past three; `IAudioSessionManager2` + `IAudioSessionNotification` + `IAudioSessionEvents` on the default output, watched only while the card is open with it on, no polling; the header's equalizer button and osd.json `mixer`. Evidence: ModernFlyouts #164 and #103, FluentFlyout #26, PILLAR ([research-features.md](research-features.md) #9) | Levels set from the card show in Windows' mixer and back; an app that pauses keeps its line |

## 12. Acceptance test plan

- **Unit tests:**
  - glyph tiers;
  - coalescing and linger timing;
  - echo filtering by context GUID;
  - the device-switch suppression window;
  - DDC throttle;
  - flyout host matching (on recorded window trees).
- **Script** `tools/osd-check.ps1`: sets the system volume via Core Audio with a *foreign* GUID, screenshots the island, and restores the original volume. It never touches brightness unless asked.
- **Manual matrix:**
  - media keys on two keyboards;
  - laptop brightness keys;
  - an external DDC/CI monitor;
  - plugging and unplugging headphones;
  - a fullscreen game;
  - Explorer restart with the flyout hidden;
  - an RDP session.

## 13. Open decisions

| ID | Question | Proposal |
|---|---|---|
| D1 | Volume HUD while music is playing: bubble (ring) or briefly take the pill? | Bubble ring (music keeps the pill), like the copy bubble; if the music card is open, its slider moves instead |
| D2 | Replace Windows' flyout by default? | No, opt-in (undocumented window hack; all-or-nothing) |
| D3 | Show the percentage? | Off by default; shown while dragging |
| D4 | Scroll on the compact pill to change volume? | No (the wheel already cycles activities and swipe skips tracks); scroll works on the HUD itself and on the expanded slider |
| D5 | Show in "When needed" mode? | Yes: a change you just made is needed |
| D6 | Optional states in v1 (Caps/Num Lock, battery saver, charging, mic mute)? | Lock keys and battery saver in M4; charging and mic mute wait for features #6 and #9 |

## Appendix: sources

- **Microsoft:**
  - [AUDIO_VOLUME_NOTIFICATION_DATA](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/ns-endpointvolume-audio_volume_notification_data)
  - [SetMonitorBrightness](https://learn.microsoft.com/en-us/windows/win32/api/highlevelmonitorconfigurationapi/nf-highlevelmonitorconfigurationapi-setmonitorbrightness)
  - [Power setting GUIDs](https://learn.microsoft.com/en-us/windows/win32/power/power-setting-guids)
  - [RegisterShellHookWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registershellhookwindow)
  - [BrightnessOverride](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.display.brightnessoverride) (not used)
  - [FocusSessionManager](https://learn.microsoft.com/en-us/uwp/api/windows.ui.shell.focussessionmanager)
- **Windows OSD:**
  - [Windows 11 flyout redesign](https://www.xda-developers.com/new-windows-11-preview-finally-redesigns-the-volume-and-brightness-flyouts/)
  - [OSD position setting (2025)](https://www.windowslatest.com/2025/06/20/windows-11-will-let-you-change-on-screen-indicators-position-like-volume-flyout/)
- **Projects:**
  - [FluentFlyout](https://github.com/unchihugo/FluentFlyout) ([#725](https://github.com/unchihugo/FluentFlyout/issues/725))
  - [ModernFlyouts](https://github.com/ModernFlyouts-Community/ModernFlyouts)
  - [hide-volume-brightness-osd](https://github.com/louis141414/hide-volume-brightness-osd)
  - [Twinkle Tray](https://github.com/xanderfrangos/twinkle-tray)
  - [boring.notch releases](https://github.com/TheBoredTeam/boring.notch/releases)
  - [MediaMate](https://wouter01.github.io/MediaMate/)
  - [SlimHUD](https://github.com/AlexPerathoner/SlimHUD)
  - [Seam vs Alcove](https://getseam.app/blog/seam-vs-alcove)
- **Code survey** (Oct 28, 2026): `audio.rs`, `media.rs`, protocol, `order.rs`, `render.rs`, `pill.rs`, settings and tray patterns.
