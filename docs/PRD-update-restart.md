# PRD: Windows Update restart

Status: draft · Owner: Rohit · From [research-features.md](research-features.md) §5d #3 · Nov 1, 2026

**References:**
- **The ask** (r/SideProject, Oct 31, 2026): "Windows schedules a restart and the warning is easy to miss; open work is lost."
- **What Windows does today:**
  - When updates need a restart, Settings → Windows Update says so, and a toast may appear.
  - Outside **active hours** (8 to 17 on this PC, `UX\Settings`), Windows restarts by itself. It picks the time with its "smart scheduler" when the PC looks unused.
  - Before a forced restart (a deadline, set by an organisation's policy), it shows a full-screen warning about 15 minutes ahead.
  - The toast is easy to dismiss, and Focus / Do Not Disturb holds it back. The restart comes anyway.
- **Measured on this PC** (Nov 1, 2026, ordinary user, no admin):

  | What | Where | Readable | Change notification |
  |---|---|---|---|
  | A restart is needed | key `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired` exists | yes | yes (`RegNotifyChangeKeyValue` on `Auto Update`, subtree) |
  | Same, from the documented API | `Microsoft.Update.SystemInfo` → `RebootRequired` (WUA COM) | yes | no (would need polling) |
  | Active hours | `HKLM\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings`: `ActiveHoursStart` / `ActiveHoursEnd` (hours) | yes | yes |
  | When Windows expects to restart | `HKLM\SOFTWARE\Microsoft\WindowsUpdate\UX\StateVariables`: `SmartSchedulerPredictedStartTimePoint` / `…EndTimePoint` (Unix ms), `…Confidence` (50 here) | yes | yes |
  | The last restart Windows did | same key: `CurrentRebootStartTime`, `CurrentRebootReason` ("Interactive") | yes | yes |
  | The Update Orchestrator's scheduled tasks (`\Microsoft\Windows\UpdateOrchestrator\`) | Task Scheduler | **no** (not visible to a user) | n/a |

  No restart was pending when this was measured, so **which value holds a restart Windows has actually scheduled** (as opposed to the predicted window) is still unknown. M1 finds out (§6).
- **ReUI today:**
  - `modules/privacy/store.rs` already watches a registry subtree from its own thread (`RegNotifyChangeKeyValue`, re-armed before each read). The same pattern does here.
  - Alerts break through Focus because they're ReUI's own, not toasts.
  - `Element::Clock` counts down in the island from a start time, with nothing sent each second.

## 1. Problem

Windows decides to restart for updates. The warning is a toast that's easy to miss or that Focus holds back. The restart then comes anyway, at night with the PC left on, or with a 15-minute warning during work, and unsaved work is lost.

## 2. Goals

1. **Know before it happens.** When updates are waiting for a restart, the island says so once, then says it again, louder, shortly before Windows will restart.
2. **Two answers, no more:** restart now, or choose when (Windows' own "Schedule the restart" page).
3. **Nothing at rest.** No polling: the hub sleeps until Windows Update writes to the registry.
4. **No admin, ever.** ReUI only reads; the answers are ones any user can give.
5. **Never cry wolf.** No card when no restart is needed, or when one only might be needed some day. Times are shown only when Windows has one.

## 3. Non-goals

- Postponing the restart from the card. Pausing updates or moving active hours writes `HKLM` (admin) or goes through the Update Orchestrator's undocumented COM. Windows' own page does it, one click away.
- Installing or downloading updates, update history, driver updates.
- Restarts other programs ask for (an installer's "restart later").

## 4. Experience

### 4.1 Moments

| When | Priority | Pill | Card |
|---|---|---|---|
| Updates are waiting for a restart (seen once per update) | brief, then gone | restart glyph · "Restart" | eyebrow **Windows Update** · hero **Restart needed** · detail the time, if Windows has one ("Tonight around 23:30") · **Schedule** / **Restart now** |
| 60 min before a restart Windows has scheduled | alert | restart glyph · countdown | hero **Restarting in 58 min** (a counting-down clock) · detail "Save your work" · **Schedule** / **Restart now** |
| 10 min before | alert, again | same | same, counting down |
| Restart no longer needed (done, or it was cancelled) | retract | | |

- The **brief** at first sight is the copy toast's kind: it shows a few seconds, then it's available from the tray (*D2*) and the alert does the rest.
- During a full-screen game or presentation, the 60-minute alert waits like other alerts; the 10-minute one comes through (*D3*).
- **No time from Windows** (only "a restart is needed"): the brief alone, with no countdown. Active hours aren't turned into a guess.

### 4.2 Buttons

- **Restart now** does what Start → **Update and restart** does: `InitiateShutdownW` with `SHUTDOWN_RESTART | SHUTDOWN_INSTALL_UPDATES` (documented: "installs pending updates before restarting") and the "planned, OS upgrade" reason, after enabling `SeShutdownPrivilege`, which every interactive user holds. A plain `ExitWindowsEx` reboot is the separate **Restart** entry, which may leave the updates waiting. It asks once: the button turns into **Restart · sure?** for 3 s, so a stray click on a small pill can't close everything (*D1*).
- **Schedule** opens `ms-settings:windowsupdate`, where "Schedule the restart" and "Pause updates" are, and dismisses the card.
- Right-click → the usual quiet options (off, snooze for an hour) as for other moments.

### 4.3 Settings

Settings → General → **Windows Update restart**: on (default) / off. Nothing else.

## 5. Design

- **Module** `modules/update_restart/` (one job per file):
  - `watch.rs`: the OS side on its own thread (`reui-wu-watch`). It waits on `RegNotifyChangeKeyValue` events for `…\Auto Update` and `…\WindowsUpdate\UX` (subtrees), re-armed before each read, plus a stop event. Each wake reads the few values in §References into a `State { needed, scheduled: Option<i64 ms>, predicted: Option<(i64, i64)>, active: (u8, u8) }` and sends it only when it changed.
  - `plan.rs`: pure logic with a fake clock and tests. From a `State` and now, it gives what to show and when to wake next (the brief, the 60- and 10-minute alerts, retract). The module sleeps on that one deadline: one timer, no tick.
  - `view.rs`: the views (meaning only: glyph, text, clock, buttons).
  - `restart.rs`: enables `SeShutdownPrivilege` and calls `InitiateShutdownW` (restart, install updates).
  - `mod.rs`: the loop: watcher states, plan deadlines, actions.
- **Protocol:** nothing new. Text, glyph, a counting-down `Clock`, buttons.
- **Flag:** `--no-update-restart`.
- **Cost:** one thread asleep on two registry events, a few `RegQueryValueEx` calls when Windows Update writes (a few times a day), one timer while a restart is coming.

## 6. Milestones

| | What | Done when |
|---|---|---|
| **M1** | **Spike:** find the scheduled restart's value. `tools/windows-update-state.ps1` saves every value under the Windows Update keys to JSON (read-only, no admin; `-Diff a,b` compares two). Run it once updates wait for a restart, after picking a time in "Schedule the restart", and as the restart nears, and diff each against the baseline (taken Nov 1, 2026 with nothing pending: 567 values). | The values are known and written down here (§7), or the countdown is dropped and only "Restart needed" ships |
| **M2** | Watcher, plan, views, Restart now (with its confirm) and Schedule; the setting and the flag | Tests for `plan.rs`; on a PC with a pending restart, the brief shows once and Restart now restarts and applies the updates; 0 CPU at rest |
| **M3** | The 60- and 10-minute alerts with the countdown | A scheduled restart shows the countdown in the island at T−60 and T−10, through Focus, and the card retracts if the restart is cancelled |

Each milestone is its own PR; `docs/guide.md`, the README feature list, `ARCHITECTURE.md` and `docs/development.md` (the flag) change with M2.

## 7. Open questions and decisions

- **Q1 (M1): where's the scheduled time?** Candidates: a value Windows adds to `UX\StateVariables` or `Orchestrator` once a restart is scheduled, against the smart scheduler's predicted window (which exists even when nothing is pending, so on its own it can't mean "scheduled").
- **Q2, answered:** "Update and restart" is documented: `InitiateShutdownW`'s `SHUTDOWN_INSTALL_UPDATES` flag. Still to see once on a real pending restart (M2).
- **D1: confirm Restart now?** Yes (proposal, in until Rohit says otherwise): a second press within 3 s. Restarting closes every app.
- **D2: after the brief, where does it stay?** The proposal is a line in the tray menu ("Restart for updates…") until the restart, with nothing in the resting pill; the alternative is a lasting minimal glyph beside the pill. A tray line needs something new: modules can't add tray entries today (ReUI's own "Install update" entry is built into the tray). Until D2 is answered, M2 shows the brief once and M3's alerts do the rest.
- **D3: through a full-screen app?** Yes for the 10-minute alert (proposal); it's the last chance to save. The 60-minute one, no.
- **D4: on by default?** Yes (proposal). It's silent unless a restart is coming.
