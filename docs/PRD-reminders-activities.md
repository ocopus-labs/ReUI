# PRD: Reminders and live activities from scripts

Status: draft for review · Owner: Rohit · Items 6 and 7 in [research-features.md](research-features.md) §3 · Oct 30, 2026

**Evidence** (details and links in research-features.md):
- **Reminders:** on boring.notch's roadmap, with 11 issue titles (#384, #695, #780); Atoll #78 "quick reminders"; exuupery's users ask for recurring reminders, a ringtone, and editing one. Five competitors ship them (NotchNook, Notchify, NotchPixel, Smart Notch, PILLAR). Nobody lets you type one in a line.
- **Script activities:** boring.notch's "Extension system" roadmap item and #670/#671 ("real live activities"); exuupery's "we can't do without a plugin system"; NotchPixel sells "custom scripts". Builds, backups and long CLI jobs are exactly what a developer's island should show, and `reui-bridge` already reaches the hub.

**Inputs:** the timer field (`timers/parse.rs`, Win+Ctrl+T) and its live preview in the hero; `timer-state.json` and its late-finish rules; deadline sleeping with fake-clock tests; `reui-bridge timer|stats|hide` (module commands over the per-user pipe); the protocol's Ongoing / Brief / Alert priorities, `Element::Ring`, `Row::Progress`, `Clock`.

## 1. Problem

Windows' own reminders live in apps that are behind other windows (To Do, Outlook, Cortana's leftovers), take several clicks to make, and announce themselves with a toast that Do Not Disturb swallows. And a script you started (a build, `winget upgrade`, a backup) has no way to say "42 %" or "done" anywhere you can see without switching to its terminal.

## 2. Goals

1. **A reminder in one line.** In the timer field: `call mom at 6`, `standup 9:30`, `in 2h check build`, `tomorrow 9 dentist`, `every day 21:00 meds`. The hero shows when it will fire before Enter, like a timer's length.
2. **Impossible to miss, easy to put off.** It fires as an Alert card: **Done** or **Snooze** (10 min). It survives sleep and restarts, and a late one says so ("Was due at 18:00").
3. **Any script gets a live activity** with one command line: `reui-bridge activity build --title Build --progress 0.42`. Progress in the pill, the result as a Brief.
4. **Nothing costs anything while idle.** One hub wake-up at the next due reminder, none for activities except a stale check at its deadline.

## 3. Non-goals (v1)

- Lists, notes, due dates without a time, sync with Microsoft To Do or Outlook (network and accounts).
- Waking the PC from sleep for a reminder; a reminder due while asleep shows on wake, late.
- Editing an upcoming reminder in place (M2: the Timers place lists them; Ctrl+Z removes the one just added).
- Images, links or buttons from scripts. Scripts send text and a number; nothing they send is opened or run.

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| R1 | Quick one | Win+Ctrl+T, `call mom at 6`. The hero reads "Reminder · 6:00 PM · Call mom". Enter. At 18:00 the island opens on "Reminder / Call mom / 6:00 PM" with Snooze and Done |
| R2 | Relative | `in 2h check build` → in two hours |
| R3 | Tomorrow | `tomorrow 9 dentist` → 9:00 AM tomorrow (a bare hour on another day reads 7–11 as morning, 12–6 as afternoon) |
| R4 | Every day | `every day 21:00 meds`. Done puts it on tomorrow at 21:00; Snooze brings it back in 10 min and then it carries on daily |
| R5 | Laptop asleep at 18:00 | On wake: "Was due at 6:00 PM", no sound |
| R6 | ReUI restarted | Upcoming reminders come back; one that came due meanwhile shows late |
| R7 | Typo | Ctrl+Z in the timer field removes the reminder just added |
| A1 | A build | `reui-bridge activity build --title Build --progress 0.3` … `--progress 1 --state done`. Pill: a ring and "30%"; open: Build, the detail, a progress bar. Done shows as a Brief and goes |
| A2 | A script dies | No update for 10 minutes: the activity ends by itself (unless `--keep`) |
| A3 | A noisy script | Updates 100 times a second: the island gets at most four a second |

## 5. Experience

### 5.1 The grammar (`timers/remind/grammar.rs`, pure)

Small and explicit. A line is a reminder when it has one of these; otherwise it's a timer as before (`10 tea` is still a 10-minute timer).

| Part | Forms |
|---|---|
| A time | `at 6`, `at 6:30`, `at 18:00`, `6pm`, `6 pm`, `6:30am`, `noon`, `midnight`; `9:30` with words beside it (`standup 9:30`); alone, `5:00` is still a 5-minute timer |
| A day | `today`, `tonight`, `tomorrow`, a weekday (`friday`, `fri`, `on fri`) |
| Relative | `in 2h`, `in 90 min`, `in 1h30`, `in 3 days`; at the start or the end |
| A repeat | `every day` / `daily`, `every weekday` / `weekdays`, `every monday`, `weekly` (on the day of the first one) |
| The text | Whatever's left, first letter capitalised; up to 80 characters |

Which hour a bare number means (no am/pm):
- **24-hour locale** (the user's time format has `H`): as written, `9:30` is 09:30. A bare hour (`at 6`) is the next 6 o'clock to come round.
- **12-hour locale:** the next one to come round (`at 6` at 14:00 is 18:00; at 19:00 it's 6:00 tomorrow).
- **On another day or a repeat** (`tomorrow 9`, `every day 8`): 7–11 morning, 12 noon, 1–6 afternoon.
- `0`–`23` with a leading zero or above 12, and am/pm, are exact. `12am` is midnight.

Refusals say why in place of the detail line: "Add a time, like tomorrow 9", "That time has passed", "Up to a year ahead", "Repeats need a time of day", "50 reminders is the limit".

### 5.2 Preview and the Timers place

- While typing, the hero is the time it will fire ("6:00 PM"), the eyebrow "Reminder", the detail the text and the day ("Call mom · Tomorrow", "Meds · Every day").
- With the field empty, the timer field lists the next three reminders as facts (when over what).
- Ctrl+Z in the field removes the last reminder added.

### 5.3 Firing (Alert, key `reminder`)

- Eyebrow "Reminder", hero the text, detail the time ("6:00 PM", or "Was due at 6:00 PM" when more than a minute late, with the weekday if it was another day; "· Every day" for repeats).
- **Done** (primary) and **Snooze** (10 min). Several due together page ("1 of 3"), soonest first.
- Compact: a bell and the text. No sound unless "Sound for reminders" is on (the timer sound, once).
- An Alert, never a Decision: it waits for you, and never opens itself through a paused ReUI.

### 5.4 Script activities (module `activities`)

```
reui-bridge activity <id> --title "Build" [--detail "…"] [--progress 0.42]
                         [--state running|done|failed] [--glyph build] [--done-after 5s]
                         [--keep] [--source name]
reui-bridge activity <id> --end
reui-bridge activity --json          # one JSON object per line on stdin
```

- **Running (Ongoing, key `<id>`):** compact: a ring for progress (else the glyph) and the percentage (else the title). Open: eyebrow the title, hero the detail (else the time since it started, counting up), a progress row when there's progress.
- **Done / Failed (Brief):** a check or a cross, "Done" or "Failed", the detail. Gone after `--done-after` (default 6 s, failed 20 s, at most 10 min).
- **Ends by itself** when nothing has been heard for 10 minutes (`stale_min`), unless `--keep`. `--end` takes it away now.
- **Limits:** at most 3 at once per source, 8 in all (the oldest running one makes way); id up to 40 of `a-z 0-9 - _ .`; title 40 and detail 80 characters, control characters dropped; updates coalesced to four a second.
- **Priority** is never above Alert: a script can't ask a question, take the keyboard or open the card by itself.
- **Glyphs** by name: `build`, `test`, `deploy`, `download`, `upload`, `sync`, `backup`, `package`, `database`, `script` (default). Unknown names are the default.
- **Source:** `--source`, else the name of the program that ran the bridge (`pwsh`, `python`, `bash`). Settings can block sources (`blocked` in `activities.json`).

**Security:** the pipe is per-user (its ACL is the current user only), so only the user's own programs can reach it. Everything is text, length-capped, validated in the hub (not only in the bridge). No URLs are opened, nothing is executed, no images or paths are read in v1.

**JSON form:** `{"id":"build","title":"Build","detail":"…","progress":0.42,"state":"running","glyph":"build","done_after_s":5,"keep":false,"end":false}`; `id` is required. Each line is one update, so `my-job | reui-bridge activity --json` streams.

### 5.5 Examples (docs/development.md "Scripts")

- A build script: `cargo build` with `--title Build`, then `--state done` or `--state failed --detail "exit 101"`.
- `winget upgrade --all` wrapped: a running activity with the package being upgraded as the detail.
- A nightly backup: `--glyph backup --progress` per folder, `--keep` because a folder can take an hour.

## 6. Platform details

- **When:** reminders are wall-clock events, so their truth is a Unix time (and, for repeats, the local time of day and the rule). The module sleeps until the next one, recomputing the wait from the wall clock on every wake, a resume and a clock or time-zone change.
- **Local time:** chrono's `Local`; a time inside a DST gap moves to the next valid minute, an ambiguous one takes the earlier.
- **12/24 h:** `GetLocaleInfoEx(LOCALE_STIMEFORMAT)`; times shown in the user's format (`GetTimeFormatEx`, as timers already do).
- **Persistence:** reminders join `timer-state.json` (`reminders`: text, due time, repeat, ringing). Written on every change, never on a tick.
- **Activities are not persisted:** a hub restart ends them; the next update from the script brings them back.

## 7. Protocol

No change. Activities and reminders use existing elements (`Ring`, `Glyph`, `Clock`, `Row::Progress`, `Row::Facts`, `Row::Buttons`, `Pager`). The bridge sends `ClientMsg::Command { module: "activities", action: "update:<json>" }`, the same path as `reui-bridge timer`.

## 8. Edge cases

- **E-R1** Clock changed by hand or time zone moved: the next wait is recomputed; a reminder that's now in the past fires (late if more than a minute).
- **E-R2** Several due at once (after sleep): one card, paged, one sound at most.
- **E-R3** A repeating reminder left unanswered past its next time: still one card; Done moves it to the next time after now (no pile-up).
- **E-R4** A file from a newer version or broken JSON: the timers' rules (unreadable means none, never overwritten mid-read).
- **E-A1** An activity id reused after it ended: a new activity.
- **E-A2** Progress outside 0..1, NaN, or a title of only control characters: clamped, dropped, refused.
- **E-A3** Activities switched off: commands are ignored and any shown end.

## 9. Settings

- `timers.json`: `reminder_sound` (default off). Timers page: "Sound for reminders".
- `activities.json`: `enabled` (default on), `stale_min` (10), `per_source` (3), `blocked` (sources, `[]`). Coding agents page, group "Scripts": "Activities from scripts".

## 10. Milestones

- **M0:** this PRD; the grammar and its tests; the activity request parser and board with fake-clock tests.
- **M1:** reminders in the timer field (preview, firing card, snooze, repeat, persistence, the list in the field, Ctrl+Z); the `activities` module and `reui-bridge activity`; settings; docs.
- **M2:** an upcoming reminder's own card (edit, remove) from the Timers place; a "Later" menu (10 min, 1 hour, tomorrow) once the protocol has a menu button; a script's activity opening its terminal (the bridge's parent chain, as agents do).
