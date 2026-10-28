# PRD: Timers and focus (timers, stopwatch, focus sessions)

Status: draft for review · Owner: Rohit · Feature #7 in [PLAN.md](../PLAN.md) · Oct 28, 2026

**References:**
- **iPhone Dynamic Island:**
  - A running timer is a Live Activity. Compact shows a ring on the left and the time left on the right. Minimal shows the ring. Expanded shows the label and the large numerals, with circular Pause and Cancel buttons.
  - When the timer finishes, an alert offers **Stop** and Repeat. In iOS 26 the large button became Repeat, and users now hit it by mistake ([Tom's Guide](https://www.tomsguide.com/phones/iphones/i-use-timers-on-my-iphone-all-the-time-but-ios-26-has-made-things-worse)). Lesson: Stop stays the dominant action.
  - Multiple timers (iOS 17+) stack with the shortest first.
  - The stopwatch Live Activity (iOS 18) has Pause and cancel, plus Lap.
  - Dismissing the activity never cancels the timer.
- **Mac notch apps:**
  - NotchNook and Alcove have no timer, and Atoll has basic timers only.
  - boring.notch has unmerged timer and Pomodoro PRs (#1223, #1667, #1681). They use presets, a ring on the left of the closed notch with the time left on the right, phase plus round dots, and auto-start and sound settings.
  - **None of them let you type "25m".** That's our opening.
- **Windows:**
  - Clock's *Focus sessions* default to 30 min, with 5 min breaks in sessions longer than 30 min. They have separate end-of-session and end-of-break sounds and turn on Do Not Disturb, and they need the Clock app open.
  - Windows has no timer that lives on screen.
  - No public API lets a desktop app turn Do Not Disturb (DND) on or off (see §6).
- **Pomodoro conventions:**
  - 25 / 5 / 15, with a long break after 4 rounds.
  - Users' top complaints:
    - a finish that's easy to miss (people work through breaks);
    - an alarm that sometimes doesn't fire;
    - harsh sounds that can't be changed;
    - a permanent notification while a timer runs;
    - the app forcing the next phase ("a reminder, not the decision maker", Toggl);
    - accidental pause or repeat.

**Inputs:** a platform and competitor study (sources in the appendix), and ReUI's existing building blocks:
- the island already advances a `Row::Timeline` by itself from a timestamp, so the hub doesn't push every second;
- summoned panels take typed input without stealing focus (the keyboard hook in `clipboard_keys.rs`);
- the `Decision` priority expands the island by itself;
- inline glyph controls beside the hero are already the documented "Apple timer" template in DESIGN.md;
- `system_events` provides resume and time-change events;
- the tray menu and Settings follow established patterns.

## 1. Problem

On Windows, a timer lives in the Clock app, behind other windows. You start it there and then forget it's running, or you miss it going off because the app is buried and the toast was swallowed by Do Not Disturb. A countdown is exactly the kind of small, always-relevant thing the island exists for: visible at a glance and impossible to miss when it ends.

## 2. Goals

1. **Start in a second.** A hotkey, then typing `10`, `25m`, `1h30`, `90s` or `10m tea` and Enter, starts a timer. Presets are one click from the tray.
2. **Always know how long is left.** The running timer sits in the pill and counts down by itself.
3. **Impossible to miss, easy to stop.** The finish expands the island, plays a sound until you stop it (with a cap), and puts Stop as the main button.
4. **Focus without nagging.** A focus session alternates work and breaks. It asks before starting the next phase (unless you opt in to auto-start), and can turn on Do Not Disturb while you work.
5. **Correct across sleep, restarts and clock changes.** A timer never fires late without saying so, never runs negative and never vanishes.
6. **Costs nothing.** 0 % CPU and no wake-ups while idle. A running timer costs one hub wake-up at its deadline, plus one island redraw a second only while its numerals are on screen.

## 3. Non-goals (v1)

- **Alarms at a wall-clock time** ("wake me at 7"), waking the PC from sleep, and recurring alarms (*T6*).
- Task lists, statistics or streaks, Spotify or To Do integration, and app or website blocking.
- Sync with the Windows Clock app or the phone.
- Toast notifications. The island and a sound are the reliable path, and toasts need an AUMID plus Start-menu registration (*T7*).

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| S1 | Tea | Win+Ctrl+T, type `4m tea`, press Enter. The pill shows a ring and `3:59` and counts down. At zero the island expands to "Tea" / "Done" with a sound, and Stop ends it |
| S2 | Quick preset | Tray → Timer → 5 min. Same as S1 |
| S3 | Two timers | Pasta (9 min) and bread (40 min). The pill shows the one ending first, and the card pages between them ("1 of 2"). Each has its own Pause and Cancel |
| S4 | Pause | Pause in the card. The numerals stop and dim, and the ring stops. Resume continues from there |
| S5 | +1 minute | In the finish card, "+1 min" restarts the same timer for a minute. During a run, `+1:00` adds a minute |
| S6 | Focus session | Win+Ctrl+T, `focus` (or the tray). 25 min of work with DND on if enabled. At the end, the island expands: "Break time" / Start break / Skip. After 4 rounds the break is 15 min |
| S7 | Stopwatch | Win+Ctrl+T, `sw` (or the tray). The pill counts up. The card has Pause, Lap and Reset, and shows the last lap |
| S8 | Laptop sleeps mid-timer | On wake, if the timer ended while asleep: the island shows "Finished at 3:14 PM", **quiet** if more than a minute late, and rings if within a minute |
| S9 | ReUI restarts or the PC reboots | Running timers come back with the right time left. Any that ended in between show "Finished at …" quietly |
| S10 | Game in fullscreen | The island is hidden as usual, and the sound still plays. The finish card is waiting when the game leaves fullscreen |
| S11 | Music is playing | The timer takes the pill (Ongoing, and the newest wins) and music moves to the bubble. Swap them with a click on the bubble or the wheel |

## 5. Experience

### 5.1 Starting

- **The launcher (Passive, key `launcher`, summoned):**
  - **Opening it:** Win+Ctrl+T (configurable, like the clipboard hotkey), or Tray → Timer → "New timer…". It opens expanded with keyboard capture, like the clipboard panel.
  - **Search field:** the module parses what you type live. The hero previews the result ("Timer · 10:00 · Tea"); Enter starts it and Esc closes. The parser:
    - `10` means minutes;
    - `10m`, `90s`, `1h30`, `1h 30m`, `1:30:00` and `5:00` work;
    - `10 min`, `2 hours` and `½h` are accepted, and anything left over becomes the label;
    - `focus` and `pomodoro` start a focus session, `sw` and `stopwatch` start the stopwatch;
    - the maximum is 24 h;
    - anything that can't be parsed shows a quiet Note ("Type a length, like 10m") with no error colour.
  - **Preset capsules:** a row below the field, from Settings (default 1, 5, 10, 25 min), then Focus and Stopwatch. Clicking one starts it. The last 3 custom lengths show as the field's placeholder hint (*T4*).
- **Tray and pill menu:** a "Timer" submenu with the presets, Focus session, Stopwatch and New timer….
- **Command line:** `reui-bridge timer 10m tea` takes the same text, for scripts and coding agents.

### 5.2 Running timer (Ongoing, key `timer:<n>`, accent orange `#FF9F0A`)

- **Compact:**
  - **Leading:** a ring with the fraction left.
  - **Trailing:** the time left as tabular numerals: `m:ss` below an hour, `h:mm:ss` above.
  - **Paused:** the numerals and ring dim.
  - The island advances both by itself (see §7).
- **Minimal:** the ring.
- **Expanded:**
  - eyebrow: the label, or "Timer";
  - hero: the numerals (`Headline::Clock`);
  - detail: "Ends 3:42 PM", or "Paused" when paused;
  - inline glyph controls: Pause/Resume (Accent) and Cancel (Secondary), the Apple template;
  - with more than one timer, a Pager.
- **More than one timer:** one activity holds all countdown timers. The one ending soonest is in front, and the pager walks the rest in end order. The bubble rule (a different kind) keeps music or an agent visible.

### 5.3 Finish (Decision, key `done:<n>`)

- **Expansion:** the island expands by itself even if a card is open. The decision hold applies, so it folds to compact after the hold with the accent "Done" still showing until it's stopped.
- **Card:**
  - eyebrow: the label;
  - hero: "Done";
  - detail: "4 min timer";
  - buttons: **Stop** (Primary) and "+1 min" (Secondary). There's no Repeat; "+1 min" is the only second action (*T3*).
- **Compact:** a bell glyph and "Done" in the accent.
- **Sound:**
  - the chosen sound loops until Stop, for at most 60 s (`ring_for`), then stops by itself;
  - the card stays as an Alert ("Done · 3:42 PM") until dismissed with a click or Stop;
  - a second timer finishing while one is ringing joins the same card ("2 timers done") and the ring isn't doubled.
- **Late finish** (sleep or restart, S8 and S9): the hero reads "Finished at 3:14 PM". The sound plays only if it's less than 60 s late, and the card is Alert, not Decision.

### 5.4 Focus session (Ongoing, key `focus`, accent `#5E5CE6` indigo)

- **Phases:** Work 25, Short break 5, Long break 15, with a long break after every 4 work phases. Lengths come from Settings.
- **Compact:** the ring plus `24:59`, as with a timer. During breaks the ring uses the Positive tone (green).
- **Expanded:**
  - eyebrow: "Focus", or "Break" during breaks;
  - hero: the numerals;
  - detail: "Round 2 of 4 · Do Not Disturb on";
  - inline controls: Pause and End.
- **Phase end (Decision):**
  - after work: hero "Break time", Start break (Primary) and Skip break (Secondary);
  - after a break: hero "Back to it", Start focus (Primary) and End (Secondary);
  - the same ring and cap rules as §5.3 apply;
  - with **auto-start** on (off by default), the next phase starts at once and only a short chime plays;
  - unanswered for 10 min: the session ends quietly with "Focus ended after 3 rounds".
- **Do Not Disturb (opt-in, *T1*):**
  - turned on for work phases and back to what it was for breaks and the end;
  - if DND was already on when the session started, ReUI leaves it on (like Clock);
  - see §6 for how, and for the fallback when it can't.

### 5.5 Stopwatch (Ongoing, key `stopwatch`, accent `#30D158` green)

- **Compact:** a stopwatch glyph on the left and the elapsed time on the right (`m:ss`, then `h:mm:ss`; no hundredths in compact).
- **Expanded:**
  - eyebrow: "Stopwatch";
  - hero: the elapsed time (tenths shown only while the card is open, *T5*);
  - detail: the last lap ("Lap 3 · 1:12.4");
  - inline controls: Pause/Resume, Lap and Reset.
- **Limits:** one stopwatch, and laps capped at 99.

### 5.6 What it never does

- No notification while a timer runs. The pill is the indicator, and in "Only when it needs me" visibility mode a running timer doesn't show at all, only its finish does.
- No sound other than the finish.
- No forced phase change.
- No confirmation dialogs. Cancel is one click and is undone by Ctrl+Z while the card is open (like the clipboard's undo).

## 6. Platform details

| Need | How | Notes |
|---|---|---|
| Deadlines | `std::time::Instant` (QPC on Windows: **counts sleep**) for the truth; `tokio::time::sleep_until` the soonest deadline | Waits don't count sleep time, so after resume (`system_events` `Resumed`) the module recomputes everything at once |
| Surviving restarts | `%APPDATA%\ReUI\timer-state.json`: per timer `{ label, length_ms, ends_at_unix_ms | left_ms when paused }`, written on every change (not every tick) | Wall clock is used only across restarts; a clock change while running doesn't move a countdown (*E-T6*) |
| Surface ticking | `Clock { value_ms, at_unix_ms, running, counts_down }` in the protocol; the island computes `value ± (now − at)` and redraws once a second, just after each whole second, only while a running clock is on screen | Hub re-stamps on resume and on `WM_TIMECHANGE` so the surface's wall-clock math stays right |
| Sound | `PlaySoundW` of a file (`SND_FILENAME | SND_ASYNC | SND_LOOP`), `PlaySoundW(None)` to stop; choices: Windows `Alarm01–10.wav` and `Ring01–10.wav` from `C:\Windows\Media`, or none | Files, not `SND_ALIAS`: scheme aliases may be silent. Missing file: fall back to `Alarm01.wav`, then to `MessageBeep` |
| DND state (read) | WNF `WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED` (`0x0D83063EA3BF1C75`) via `NtQueryWnfStateData`: 0 off, 1 priority only, 2 alarms only | Undocumented but stable since 1803 |
| DND toggle | Undocumented `IQuietHoursSettings` COM (CLSID `F53321FA-34F8-4B7F-B9A3-361877CB94CF`, IID `6BFF4732-81EC-4FFB-AE67-B6C1BC29631F`), `put_UserSelectedProfile("Microsoft.QuietHoursProfile.PriorityOnly" / ".Unrestricted")`, called as a raw vtable on a dedicated STA thread | **Verify** by reading WNF ~500 ms later; on failure turn the feature off for the session and say so once in the focus card ("Couldn't turn on Do Not Disturb"). `FocusSessionManager.TryStartFocusSession` is a Limited Access Feature (E_ACCESSDENIED), so it isn't used |
| Hotkey | `RegisterHotKey` on the shared system-events window, Win+Ctrl+T by default (Win+Alt+T is the Xbox Game Bar's recording timer, so it's taken wherever Game Bar runs); taken → said once in Settings like the clipboard (E-C13) | |
| Keyboard in the launcher | The island's existing summoned-panel keyboard hook (no focus steal) | Same as the clipboard panel |

**Engineering:**
- a new `modules/timers/`:
  - `mod.rs`: the module loop and the soonest-deadline sleep;
  - `parse.rs`: the duration parser, pure and unit-tested;
  - `clock.rs`: timers, stopwatch and focus state machines, pure and tested against a fake clock;
  - `store.rs`: `timer-state.json`;
  - `sound.rs`;
  - `dnd.rs`: WNF read, COM toggle and verification;
  - `hotkey.rs`, or the shared hotkey code moved out of clipboard;
  - `view.rs`;
  - `settings.rs`;
- island changes: the clock element and headline, the once-a-second tick for visible running clocks, and dimmed numerals when paused;
- a Settings page, "Timers" (orange tile).

## 7. Protocol (v17)

- `pub struct Clock { value_ms: u64, at_unix_ms: u64, running: bool, down: bool }` with `Clock::value_at(now)`. Counting down clamps at 0, so the surface **never shows negative time**. The hub's finish card replaces it.
- `Element::Clock(Clock)`: tabular numerals, `m:ss` or `h:mm:ss`.
- `Element::ClockRing { clock: Clock, total_ms: u64 }`: the ring, which advances by itself.
- `Headline::Clock { clock: Clock, tenths: bool }`.
- Dimming while paused (`running: false`) is the surface's job.
- The island's tick runs only while a running `Clock` is visible: in compact, in the minimal bubble, or in an open card. It stops when the island is hidden, idle, paused or in fullscreen.

## 8. Edge cases

| ID | Case | Required behaviour |
|---|---|---|
| E-T1 | Timer ends while the PC sleeps | §5.3 late finish; ring only if < 60 s late |
| E-T2 | Timer ends while ReUI is closed | Restored at start as a late finish (quiet) |
| E-T3 | Timer ends while ReUI is paused | The finish still shows: paused ReUI lets permission requests through, and timers count as the same kind of "needs me" (decisions) |
| E-T4 | Timer ends during fullscreen | Sound plays; card waits (S10) |
| E-T5 | Several timers end at once | One finish card ("2 timers done", labels in the detail); one sound |
| E-T6 | System clock or time zone changes | Countdowns unaffected (monotonic); "Ends 3:42 PM" text recomputed; hub re-stamps clocks |
| E-T7 | `0`, `0s`, negative, > 24 h, gibberish | Not startable; the Note says what works. > 24 h clamps with a note |
| E-T8 | 20 timers | Cap at 10 running; the 11th shows "10 timers is the limit" |
| E-T9 | Hotkey taken by another app | Said once in Settings; tray still works |
| E-T10 | Sound file missing, or no audio device | Fallback chain (§6); the visual alert alone still works |
| E-T11 | DND toggle fails or Windows build changes | Verified read-back; disables itself for the session; one quiet line |
| E-T12 | User turns DND off by hand during focus | ReUI doesn't turn it back on until the next work phase |
| E-T13 | Session ends (End, or unanswered) with DND on by ReUI | Restored to what it was before the session |
| E-T14 | ReUI crashes during focus with DND on | `timer-state.json` records "DND set by us"; next start restores it if no session is resumed |
| E-T15 | Timer's card open at the moment it ends | The card turns into the finish card in place |
| E-T16 | Pause near 0 (0:00.4 left) | Paused shows `0:01` (ceil); never `0:00` while not done |
| E-T17 | Stopwatch runs for days | Fine up to `999:59:59`; then shown as `999:59:59+` |
| E-T18 | Visibility "Only when it needs me" | Running timers hidden; finishes and phase ends show |
| E-T19 | Displayed seconds | Countdown rounds **up** (shows 0:01 for the last second, done at 0); stopwatch rounds down |

## 9. Performance budget

- **Idle (no timers):** 0 % CPU and no hub or island wake-ups. The hotkey thread blocks in `GetMessage`.
- **Running timer:** one hub wake-up at the deadline, and an island redraw of one small surface once a second only while its numerals are visible (< 0.1 % of a core). Measure it with the island's `--perf` style counters.
- **Ringing:** `PlaySound` loop only.
- **Memory:** < 1 MB for the module.

## 10. Settings (`%APPDATA%\ReUI\timers.json`, page "Timers"; running timers live in `timer-state.json`)

| Setting | Default |
|---|---|
| Timers module | on |
| Shortcut | Win+Ctrl+T |
| Presets | 1, 5, 10, 25 min |
| Sound | Alarm01, with a preview button, or None |
| Ring for | 1 min (30 s / 1 min / 2 min / until stopped) |
| Focus: work / short break / long break / rounds | 25 / 5 / 15 / 4 |
| Focus: start the next phase automatically | off |
| Focus: turn on Do Not Disturb while working | off (*T1*) |

## 11. Milestones

| M | Scope | Exit |
|---|---|---|
| **M0: Protocol + island clocks** | `Clock`, `Element::Clock`, `ClockRing`, `Headline::Clock`, protocol v17; island tick only while visible; paused dimming; demo timer moved onto it | Demo timer counts down without hub ticks; idle measured at 0 |
| **M1: Timers core** | Parser, timer state machine (fake-clock tests), multiple timers + pager, finish (Decision → Alert), sound loop with cap, +1 min, cancel + undo, resume recompute, `timer-state.json` restore, late finish | S1, S3, S4, S5, S8, S9 |
| **M2: Starting** | Hotkey + launcher (field, live preview, preset capsules), tray / pill menu "Timer" submenu | S1 by keyboard only; S2 |
| **M3: Focus + stopwatch** | Focus phases, ask vs auto-start, round counter, unanswered end; stopwatch with laps | S6 (without DND), S7 |
| **M4: DND + Settings** | WNF read, COM toggle with verification, restore and crash recovery; Settings "Timers" page with sound preview | S6 with DND; E-T11–14 |

## 12. Acceptance test plan

- **Unit tests:**
  - the parser (all the forms in §5.1, labels, limits);
  - countdown and stopwatch display rounding (E-T16, E-T19);
  - the timer and focus state machines on a fake clock (pause, resume, +1 min, phase order, long break every 4, unanswered end);
  - restore from `timer-state.json` (running, paused, ended while off);
  - `Clock::value_at` clamping.
- **Live checks:**
  - S1 to S11 on this PC;
  - sleep with `rundll32 powrprof.dll,SetSuspendState` or the Start menu, with a 1 min timer;
  - a hub kill and restart mid-timer;
  - a time-zone change mid-timer;
  - DND on and off verified in Quick Settings on this build (Windows 11 26200);
  - island CPU while a timer runs, compact versus hidden.

## 13. Open decisions

| ID | Question | Proposal |
|---|---|---|
| T1 | Use the undocumented DND toggle at all? | Yes, **opt-in** (off by default), verified on every use, disables itself on failure. Reading the state is always on (so the detail line is truthful) |
| T2 | Pause ReUI vs a timer finishing | Finishes come through like permission requests (E-T3) |
| T3 | Repeat on the finish card? | No; Stop + "+1 min" only (the iOS 26 lesson). `+1:00` while running is v2 (inline controls are glyph-only) |
| T4 | Remember recent custom lengths? | Yes, the last 3, shown as the field's placeholder hint cycle; no extra UI |
| T5 | Stopwatch tenths | Only in the open card (the tick goes to 10 Hz only then); compact stays at 1 Hz |
| T6 | Alarms at a time of day | v2, with the wall-clock and wake questions answered then |
| T7 | Toasts as a backup for the finish | v2; needs AUMID registration from the installer |

## Appendix: sources

- **Apple:**
  - [WWDC23: Live Activities](https://developer.apple.com/videos/play/wwdc2023/10194/)
  - [WWDC25: AlarmKit](https://developer.apple.com/videos/play/wwdc2025/230/)
  - [iOS 17 multiple timers](https://www.idropnews.com/how-to/how-to-set-multiple-timers-in-ios-17/205615/)
  - [iOS 18 stopwatch](https://www.idownloadblog.com/2024/08/16/ios-18-stopwatch-changes/)
  - [iOS 26 Repeat complaint](https://www.tomsguide.com/phones/iphones/i-use-timers-on-my-iphone-all-the-time-but-ios-26-has-made-things-worse)
- **Microsoft:**
  - [Focus on Windows](https://support.microsoft.com/en-us/windows/make-it-easier-to-focus-on-tasks-0d259fd9-e9d0-702c-c027-007f0e78eaf2)
  - [FocusSessionManager](https://learn.microsoft.com/en-us/uwp/api/windows.ui.shell.focussessionmanager)
  - [Interrupt time](https://learn.microsoft.com/en-us/windows/win32/sysinfo/interrupt-time)
  - [Toasts from desktop apps](https://learn.microsoft.com/en-us/windows/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl)
- **DND internals:**
  - [WNF quiet-hours gist](https://gist.github.com/riverar/980120d7e3a13ed8b1d665cf974c8e31)
  - [WinDndToggle (IQuietHoursSettings)](https://github.com/HalZheng/WinDndToggle)
  - [SHQueryUserNotificationState doesn't see DND](https://github.com/fhaze/kitsu/issues/73)
- **Time and sleep:** [rust#79462](https://github.com/rust-lang/rust/issues/79462)
- **Notch apps:** [boring.notch #1223](https://github.com/TheBoredTeam/boring.notch/pull/1223), [#1667](https://github.com/TheBoredTeam/boring.notch/pull/1667), [comparison](https://notchy.dev/blog/notchnook-vs-boring-notch-vs-alcove/)
- **Pomodoro complaints:**
  - [Zettlr #1174](https://github.com/Zettlr/Zettlr/issues/1174)
  - [Super Productivity #5836](https://github.com/super-productivity/super-productivity/issues/5836)
  - [Toggl community](https://community.toggl.com/t/dont-stop-timer-when-pomodoro-focus-session-ends/3057)
