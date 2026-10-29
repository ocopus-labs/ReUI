# PRD: Calendar (the next meeting, a countdown, Join)

Status: M0–M1 built · Owner: Rohit · Feature #10 in [PLAN.md](../PLAN.md) ("Calendar and tasks") · Oct 30, 2026

**References:**
- **Mac notch apps.** NotchNook, Alcove and boring.notch show the next event in the open notch (title, time, a coloured calendar bar), and NotchNook and Alcove add a "Join" button for Zoom/Meet/Teams links. They read macOS's EventKit, which the OS fills from every account the user has added, so they never ask for a password. Alcove counts down the last minutes before a meeting in the closed notch.
- **Windows.** The Windhawk "Dynamic Island for Windows" mod shows the next Outlook appointment. Windows has no EventKit: since Mail & Calendar was retired (Dec 31, 2024), nothing on a Windows 11 PC keeps a local copy of the user's calendar that other apps can read (§6).
- **iOS.** A meeting is not a Live Activity: Calendar's alert is a notification at the reminder time, with no countdown. The island's Live Activity grammar (a ring that runs down, a title, one main button) is ours to apply.

**Inputs (what ReUI already has):**
- the island runs clocks it's sent (`Clock`, `ClockRing`), so a countdown needs no hub ticks (timers, PRD-timers-focus §7);
- the timers module's pattern: sleep until the next thing is due, wake early on resume and clock changes (`system_events`);
- a place in the view switcher (`modules/destinations.rs`), the tray, a Settings page with a text field (the clipboard's "Add an app");
- `reqwest` (native TLS) on the hub's runtime, as lyrics use it.

## 1. Problem

A meeting starts and you notice five minutes late, because the reminder toast was swallowed by Do Not Disturb or covered by a full-screen window. Then you hunt for the link in a long invitation. The island is where the time already is: it can count the last minutes down and put the link one click away.

## 2. Goals

1. **Never late without knowing.** In the last minutes before a meeting the pill counts down; at the start it comes forward.
2. **One click to join.** The card's main action opens the Teams, Zoom, Meet or Webex link found in the event, whichever field it's in.
3. **Private by default.** Nothing leaves the PC except the requests to the user's own calendar provider. Secret links are encrypted on disk and never logged.
4. **Out of the way.** Nothing in the pill until the last minutes; all-day events, declined and cancelled ones never take it; no nagging.
5. **Costs nothing.** No timers between events except the feed refresh; no hub ticks during a countdown.

## 3. Non-goals (v1)

- Creating, editing or answering events (accept/decline); reminders at the event's own reminder time.
- Tasks and quick capture (PLAN's "Calendar and tasks" second half: its own PRD).
- A month or week view: the card is today, not a calendar app.
- Sign-in to Microsoft or Google (M2, M3: §6).

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| S1 | A meeting in 10 minutes | At 9:50 the pill shows a ring running down and "Standup". Clicking it opens the card: eyebrow "Calendar", hero "Standup", detail "10:00 – 10:15 · Teams", and a **Join** capsule beside the hero |
| S2 | It starts | At 10:00 it comes forward as an Alert (the island doesn't expand by itself); the card's Join opens the Teams link. Joining, or opening and closing the card, dismisses it; otherwise it goes 5 minutes in |
| S3 | Back-to-back | 10:00 – 11:00 and 11:00 – 11:30. At 10:50 the second counts down while the first is still on; the card lists both |
| S4 | All-day event | Never in the pill; listed last in the card ("All day") when there's room |
| S5 | Declined or cancelled | Not shown at all |
| S6 | Not a video meeting | "Lunch, Room 4": the countdown and the card as S1, the detail says "Room 4", no Join |
| S7 | Going there on purpose | The Calendar tab in the view switcher (or the tray's Calendar) opens the card: the next event today and the two after it; tomorrow's first when today is done |
| S8 | Laptop asleep across 10:00 | On wake at 10:03 the start alert shows; at 10:20 nothing does (the meeting has been on for a while; the card still lists it) |
| S9 | Offline | The last copy of each feed (encrypted on disk) keeps the schedule right; a refresh is tried when the network comes back |
| S10 | Time zone change, DST | A flight lands in another zone: times are re-read in the new zone at once (`WM_TIMECHANGE`); a meeting set in New York shows at its local time here |
| S11 | No calendar yet | The Calendar card says "No calendars" with **Add a calendar**, which opens Settings → Calendar |

## 5. Experience

### 5.1 Priorities

| When | Activity (module `calendar`) | Priority |
|---|---|---|
| `lead` minutes (default 10) before a timed event | `next`: the countdown | Ongoing |
| From its start, until joined, seen, or 5 min in | `now`: the start | Alert |
| Opened on purpose (switcher, tray) | `today`: the day's card (or `next`/`now` if one is out) | Passive, summoned |

- **Never nags:** no sound, no expansion by itself, no repeat. The countdown is Ongoing, so in "Only when it needs me" it doesn't show; the start does.
- **The earliest wins** when two meetings overlap; the other is in the card's list.

### 5.2 Compact and minimal

- **Countdown:** leading a `ClockRing` (time left of `lead`, running by itself), trailing the title (the island caps it at its text width). Minimal: the ring.
- **Start:** leading the video glyph (`E714`) in the accent, trailing the title. Minimal: the glyph.
- Accent: Apple Calendar red `#FF453A`.

### 5.3 The card

```
╭──────────────────────────────────────────────────╮
│ (ring)  CALENDAR                       ( Join )  │  eyebrow → hero → detail; Join beside the hero
│         Standup                                  │
│         10:00 – 10:15 · Teams                    │
│                                                  │
│ 11:00   Design review                Room 4      │  Agenda: the next two today
│ 14:30   1:1 with Sam                 Zoom        │
╰──────────────────────────────────────────────────╯
```

- **Header:** leading the countdown ring while one runs, else the calendar glyph tile; eyebrow "Calendar" (or "Tomorrow" when today is done); hero the title; detail the time span and one place: the room if the location is a room, else the meeting service's name ("Teams").
- **Join** (Primary text button) as the header's trailing element: the one main action, beside the hero, never a row of its own. Shown from the countdown through the end of the event, only when a meeting link was found.
- **Agenda row** (new, `Row::Agenda`): the rest of today, at most two lines, each a time, a title and a quieter place. All-day events after the timed ones, "All day". No row when there's nothing else.
- **No filler.** No "No more events today", no counts: a day with nothing after the hero shows only the header. A day with nothing at all shows tomorrow's first event; with nothing for two days, the hero is "Nothing today" and nothing else.

### 5.4 Meeting links

Looked for in this order, first hit wins: `X-MICROSOFT-SKYPETEAMSMEETINGURL`, `X-GOOGLE-CONFERENCE`, `CONFERENCE` (RFC 7986), `URL`, `LOCATION`, `DESCRIPTION`, `X-ALT-DESC` (HTML). Before matching, Outlook Safe Links (`*.safelinks.protection.outlook.com/?url=…`) and Google redirects (`google.com/url?q=…`) are unwrapped.

| Service | Recognised |
|---|---|
| Teams | `teams.microsoft.com/l/meetup-join/…`, `teams.microsoft.com/meet/…`, `teams.live.com/meet/…` |
| Zoom | `*.zoom.us/j/…`, `/my/…`, `/w/…`, `/s/…`; `zoomgov.com` |
| Google Meet | `meet.google.com/abc-defg-hij` |
| Webex | `*.webex.com/meet/…`, `/join/…`, `/j.php…`, `/wbxmjs/…` |
| GoTo | `meet.goto.com/…`, `*.gotomeeting.com/join/…` |
| Others | `whereby.com/…`, `meet.jit.si/…`, `chime.aws/…` |

Only `https` links on these hosts are offered, so an invitation can't make Join run a file or another app's protocol. Join hands the link to Windows (`ShellExecuteW "open"`): the browser, which hands it to the Teams or Zoom app if installed. (Opening `msteams:`/`zoommtg:` directly when their handler is registered is M2.)

### 5.5 The view switcher, tray, Settings

- **A place:** Calendar (`E787`) after Timers in `destinations.rs`, always available while the module runs; `open` brings the countdown or start card forward if one is out, else the day's card.
- **Tray:** "Calendar" beside Devices and System stats.
- **Settings → Calendar** (`calendar.json`): the on/off switch; **Your calendars** (each source by its host, "calendar.google.com", with a toggle, the state of its last refresh, and Remove); **Add an ICS link** (a field and Add); **Windows calendar** (the Windows store toggle, off by default); **Count down from** (5 / 10 / 15 min).

| Key | Default | |
|---|---|---|
| `enabled` | true | |
| `lead_min` | 10 | 1–60 |
| `windows` | false | read Windows' own calendar store (§6.1) |
| `sources` | `[]` | `{ id, host, url, on }`; `url` is `dpapi:` + hex of a DPAPI blob |

A link typed into the file by hand (no `dpapi:`) is encrypted by the hub the first time it reads it and the file rewritten.

## 6. Where the calendar comes from (research)

What a non-packaged Win32 app (ReUI: no package identity, a per-user exe) can read, most private first.

### 6.1 Windows' calendar store (`AppointmentManager`)

- `AppointmentManager.RequestStoreAsync(AllCalendarsReadOnly)` is documented as needing the `appointments` capability in a package manifest ([docs](https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.appointments.appointmentmanager.requeststoreasync), [capabilities](https://learn.microsoft.com/en-us/windows/uwp/packaging/app-capability-declarations)). **Tried on this PC (Windows 11 26200) from an unpackaged process (Windows PowerShell 5.1, read-only):** it works. The store opened, `FindAppointmentCalendarsAsync` returned one calendar (source "Microsoft account"), `FindAppointmentsAsync` over ±30 days returned **0 appointments**. Desktop apps are covered by Settings → Privacy → Calendar → "Let desktop apps access your calendar" (on here).
- **What's in it on Windows 11 now:** whatever Mail & Calendar synced. That app stopped syncing on Dec 31, 2024 and isn't installed on 24H2 and later ([Microsoft](https://support.microsoft.com/en-us/office/outlook-for-windows-the-future-of-mail-calendar-and-people-on-windows-11-715fc27c-e0f4-4652-9174-47faa751b199), [Windows Central](https://www.windowscentral.com/software-apps/microsoft-renders-mail-and-calendar-apps-inoperable-as-it-forces-users-over-to-the-new-outlook-on-windows)). New Outlook is a web app with its own cache and doesn't write to the store. So on a current PC it's usually empty; on older ones it may still hold a stale copy.
- **Verdict:** cheap, local and private, but rarely has data. Built as an opt-in source (`windows`, off by default), read on its own thread at start, on a refresh, and when the store says it changed.

### 6.2 Outlook

- **Classic Outlook (desktop) via COM:** `Namespace.GetDefaultFolder(olFolderCalendar).Items` with `IncludeRecurrences` + `Restrict` on a date range ([docs](https://learn.microsoft.com/en-us/office/vba/api/outlook.items.includerecurrences)) reads the local OST/PST, offline, every account Outlook has. But reading items from another process can raise the Object Model Guard prompt when antivirus status isn't reported healthy ([docs](https://learn.microsoft.com/en-us/office/vba/outlook/how-to/security/security-behavior-of-the-outlook-object-model)), COM calls start Outlook if it isn't running, and classic Outlook is being phased out for new Outlook. **M4**, only if users of classic Outlook ask; it must never start Outlook.
- **New Outlook:** no COM, no MAPI, no local API ([Microsoft](https://support.microsoft.com/en-us/office/getting-started-with-the-new-outlook-for-windows-656bb8d9-5a60-49b2-a98b-ba7822bc7627)). Its data is reachable only through Microsoft Graph (§6.3) or a published ICS link (§6.5).

### 6.3 Microsoft Graph (work/school and personal Microsoft accounts)

- `GET /me/calendarView?startDateTime=…&endDateTime=…` expands recurrences server-side and carries `onlineMeeting.joinUrl`, `isCancelled`, `responseStatus` ([calendarView](https://learn.microsoft.com/en-us/graph/api/user-list-calendarview), [onlineMeetingInfo](https://learn.microsoft.com/en-us/graph/api/resources/onlinemeetinginfo)); delegated `Calendars.Read` + `offline_access`.
- **Registration:** ReUI needs one Entra app registration of its own (free): "Accounts in any organizational directory and personal Microsoft accounts", a public client (no secret: a desktop app can't keep one, [docs](https://learn.microsoft.com/en-us/entra/identity-platform/msal-client-applications)), redirect `http://localhost` for the browser flow with PKCE ([auth code](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow)) or "Allow public client flows" for the device code flow ([device code](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code)). Browser + loopback is the friendlier one (no code to type).
- **Friction:** work/school tenants may require admin consent for third-party apps, so some users can't sign in at all; publisher verification is needed to avoid an "unverified" consent screen.
- **Tokens:** the refresh token encrypted with DPAPI (`CryptProtectData`, [docs](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata)) in `%LOCALAPPDATA%\ReUI\calendar\`, never logged.
- **Privacy:** requests go only to `login.microsoftonline.com` and `graph.microsoft.com`. **M2.**

### 6.4 Google Calendar API

- OAuth for installed apps with a loopback redirect and PKCE ([docs](https://developers.google.com/identity/protocols/oauth2/native-app)); `events.list` with `singleEvents=true` expands recurrences; `conferenceData.entryPoints` carries the Meet link ([events](https://developers.google.com/calendar/api/v3/reference/events)).
- **Friction:** `calendar.readonly` is a sensitive scope. Until Google verifies ReUI's app (brand + sensitive-scope review, about two weeks), users see "Google hasn't verified this app" and the project is capped at 100 users in its lifetime ([unverified apps](https://support.google.com/cloud/answer/7454865), [FAQ](https://support.google.com/cloud/answer/13463817)). A "client secret" for a desktop client isn't secret, and Google accepts that for installed apps.
- **M3**, after verification is applied for.

### 6.5 A secret ICS link (chosen for M1)

- Google: Settings → the calendar → "Secret address in iCal format" ([help](https://support.google.com/calendar/answer/37648)). Outlook.com / Microsoft 365: Settings → Calendar → Shared calendars → Publish a calendar → "Can view all details" → ICS ([help](https://support.microsoft.com/en-us/office/introduction-to-publishing-internet-calendars-a25e68d6-695a-41c6-a701-103d44ba151d)). iCloud, Fastmail, Proton and most others have the same.
- **No registration, no sign-in, works for every provider.** The link is a password (anyone who has it can read the calendar), so it's stored with DPAPI and shown only by its host.
- **Caveats:** a published Microsoft 365 feed may leave out `X-MICROSOFT-SKYPETEAMSMEETINGURL` and wraps every link in Safe Links ([omarchy-calendar #51](https://github.com/tmn73/omarchy-calendar/pull/51), [DavMail #536](https://github.com/mguessan/davmail/issues/536)), hence §5.4's unwrapping; providers may refresh a published feed only every few minutes to hours (*to verify* per provider); some tenants disable publishing.
- **Polling:** every 15 min with `If-None-Match` / `If-Modified-Since` (a 304 costs a few hundred bytes), immediately on resume, on unlock after more than 15 min, and when the network comes back (`NotifyNetworkConnectivityHintChange`). Failures back off (1, 5, 15 min). `webcal://` is read as `https://`; plain `http` only to loopback (tests).

### 6.6 The plan

| M | Sources | Setup | What leaves the PC |
|---|---|---|---|
| M1 | ICS links; Windows' store (opt-in) | Paste a link | GETs to the user's calendar host |
| M2 | Microsoft account (Graph) | Sign in in the browser | Requests to Microsoft's login and Graph |
| M3 | Google (Calendar API) | Sign in; needs Google's verification first | Requests to Google's OAuth and Calendar API |
| M4 | Classic Outlook (COM), CalDAV | — | Nothing (COM); the CalDAV server |

All sources sit behind one trait (`source.rs`: a name, a fetch that returns events in a window), so a new one is a file.

## 7. Engineering

`crates/reui-hub/src/modules/calendar/`:

| File | Job |
|---|---|
| `mod.rs` | The module loop: sources, the next deadline, inputs, publishing |
| `event.rs` | The event model (an occurrence: id, title, start/end in Unix ms, all-day, place, link, calendar) |
| `ics/lines.rs` | RFC 5545 content lines: unfolding, properties, parameters, text unescaping |
| `ics/time.rs` | DATE / DATE-TIME (UTC, floating, `TZID`) and `VTIMEZONE` (standard/daylight onsets with yearly rules) |
| `ics/zones.rs` | `TZID`s without a `VTIMEZONE`: Windows' own ICU (`icu.dll`, IANA and Windows names), else local time |
| `ics/rrule.rs` | RRULE expansion in the event's wall time: DAILY/WEEKLY/MONTHLY/YEARLY, INTERVAL, COUNT, UNTIL, BYDAY (with ordinals), BYMONTHDAY, BYMONTH, BYSETPOS, WKST; skips ahead to the window |
| `ics/mod.rs` | A feed into occurrences in a window: EXDATE, RDATE, RECURRENCE-ID overrides, STATUS:CANCELLED, the user's own PARTSTAT=DECLINED |
| `links.rs` | Meeting-link detection (pure, tested against real formats) |
| `schedule.rs` | What's due when: the countdown, the start alert, the day's list, the next wake-up (pure, fake clock) |
| `fetch.rs` | ICS over HTTPS with ETag / Last-Modified, backoff; the encrypted cache |
| `windows_store.rs` | `AppointmentStore` on its own thread |
| `network.rs` | Connectivity hints → "try again now" |
| `dpapi.rs` | `CryptProtectData` / `CryptUnprotectData` |
| `settings.rs`, `status.rs`, `view.rs` | `calendar.json`; per-source refresh state for Settings; the views |

**Why our own ICS parser:** the maintained crates either don't parse (`icalendar` mostly writes), or expand recurrences only with `chrono-tz`'s IANA names (`rrule`), while Outlook feeds name zones in Windows terms with a `VTIMEZONE` block ("(UTC+01:00) Amsterdam, Berlin…"), which is the feed's own authoritative definition (RFC 5545 §3.6.5). What the island needs (a few days of events) is a small, testable subset; Windows' ICU covers zone data without bundling a tz database.

**Time:** occurrences are expanded in the event's wall time, then turned into UTC through its zone, so DST lands where the organiser meant it. All-day and floating events are in the PC's zone and are re-read on `TimeChanged`.

**Sleep and clocks:** as timers: the module sleeps until the soonest of (a countdown starting, a start, an alert ending, an event ending, midnight, a refresh) and re-evaluates on `Resumed` and `TimeChanged`. Calendar times are wall-clock by nature, so the deadline is computed from `unix_ms()` each time it sleeps.

## 8. Protocol (v22)

- `Row::Agenda(Vec<AgendaEntry>)`, `AgendaEntry { time, title, detail: Option<String> }`: things coming up, one per line: when, what, and a quieter where. Semantic only; the island lays it out (`layout::rows::agenda_*`, tokens `component.agenda`).
- Everything else reuses `ClockRing`, `Element::Button` (the header's Join) and the summoned card.

## 9. Edge cases

| ID | Case | Behaviour |
|---|---|---|
| E-C1 | Event with no end | DTEND missing: DURATION, else 0 (timed) or one day (all-day) |
| E-C2 | Event in a zone the PC doesn't know and the feed doesn't define | Read as local time |
| E-C3 | DST gap (02:30 on the spring-forward day) | Read with the offset before the change (an hour later in wall time) |
| E-C4 | 10 000-event feed, 10 MB | Parsed off the hub thread; feeds over 20 MB refused; only ±2 days expanded |
| E-C5 | RRULE parts we don't expand (BYHOUR, BYWEEKNO, sub-daily) | The first occurrence only, logged once per feed |
| E-C6 | A link that's 404 or HTML | The source's state says so in Settings; its cached copy keeps working |
| E-C7 | The same event in two feeds | Both kept (they may be different calendars); the hero is one of them |
| E-C8 | A meeting longer than its lead (an all-afternoon workshop) | Countdown 10 min, start alert, then out of the pill |
| E-C9 | Starts while the island is hidden (fullscreen) | The alert waits; it lapses 5 min in like any start |
| E-C10 | The user's own declined event | Hidden when the feed says the calendar's owner (Google's `X-WR-CALNAME` address) declined |

## 10. Performance budget

- Between events: no wake-ups except one refresh per ICS source every 15 min (a 304 most of the time).
- A countdown: one hub wake-up at its start; the island redraws the ring only while it's on screen (timers' rules).
- Parsing: off the hub thread, only when a feed changed.
- Memory: the parsed feed per source (events within a year back and forward), < 2 MB for a busy calendar.

## 11. Privacy

- Nothing is sent anywhere except GETs to the hosts of the links the user added (and, M2/M3, their sign-in provider).
- ICS links, cached feeds and tokens are DPAPI-encrypted (current user) at rest; logs say "a feed" and counts, never a URL, a host's path or a title.
- Settings shows a source by its host only.

## 12. Milestones

| M | Scope | Exit | Status |
|---|---|---|---|
| **M0** | Research (§6); the ICS parser, RRULE, zones; links; schedule (all pure, tested); protocol v22 `Row::Agenda` and its island drawing | Tests; a local feed parsed | Done |
| **M1** | Module: ICS sources (fetch, ETag, cache, backoff, resume/network), Windows store (opt-in), countdown, start alert, Join, the day's card; view-switcher place; tray; `calendar.json` with DPAPI; Settings page | S1–S11 against a local test feed in a sandbox | Done (see §14) |
| M2 | Microsoft account (Graph, browser sign-in); `msteams:`/`zoommtg:` deep links | A personal and a work account | Planned |
| M3 | Google account (after verification) | | Planned |
| M4 | Classic Outlook (COM, never starting it), CalDAV; a Settings preview | | Planned |

## 13. Open decisions

| ID | Question | Proposal |
|---|---|---|
| C1 | "Free until 15:00" on the idle hover? | Not in v1. The idle face is the date and time; a "free until" line is useful only within an hour or two of the next event, and anywhere else it's filler. Revisit with M2 as "Standup in 25 min" on the idle peek only when the next event is within the hour |
| C2 | Lead time default | 10 min (Settings: 5 / 10 / 15) |
| C3 | Should the start alert expand itself? | No: Alert, as every "it happened" in ReUI. One click opens the card with Join in it |
| C4 | Make agenda lines clickable (join that one)? | Not in v1: a mis-click would join the wrong meeting; the hero is always the next one |

## 14. To verify

- A real Google secret ICS link and a published Microsoft 365 / Outlook.com link (Teams links, Safe Links, how often each refreshes). Only a local test feed has been used so far.
- The Windows store toggle on a PC where Mail & Calendar still has data (Windows 10), and whether "Let desktop apps access your calendar" off makes `RequestStoreAsync` fail (expected: access denied, shown in Settings).
- `NotifyNetworkConnectivityHintChange` firing on Wi-Fi reconnect after sleep.
