# Changelog

## 0.1.0 (30 October 2026)

First public build.

- The island: Now Playing, clipboard history (Win+Alt+V), volume and brightness pop-ups, devices (Bluetooth, batteries, charging, drives), and coding agents (Claude Code, Codex, OpenCode).
- Per-user installer (`ReUI-Setup-0.1.0.exe`, no admin): Start menu entry, starts with Windows, uninstall from Apps & features.
- Launching ReUI while it runs opens Settings.
- A welcome in three steps on first run: what ReUI shows, your clipboard shortcut, start with Windows. Replay it from Settings → About.
- Settings → Coding agents: connect Claude Code, Codex or OpenCode in two clicks, see when ReUI last heard from each, and send a test event.
- Updates from GitHub releases: a daily check (can be turned off), Install from Settings → About (verified against the release's checksums), and What's new after an update.
- Timers: type how long (`10m tea`, `1h30`) after Win+Ctrl+T, several at once, a finish you can't miss, focus sessions with optional Do Not Disturb, and a stopwatch. They survive sleep and restarts.
- Calendar: your next meeting counted down in the island, Join for Teams, Zoom, Meet and Webex links, and the day's card in the row of places. Add Google or Outlook calendars by their secret iCal address (kept encrypted).
- Turn Now Playing, Clipboard history or Devices off entirely (takes effect when ReUI restarts).
- The shelf: drag files, images, links or text onto the island to hold them, then drag them out into any app (files are referred to, never copied or moved).
- Reminders in the same field: `call mom at 6`, `in 2h check build`, `every day 21:00 meds`, with Snooze and Done; they survive sleep and restarts.
- System stats (Win+Alt+S): CPU and memory with their last minute, then GPU, disk and network, read only while the card is open.
- Downloads and screenshots: a download's size and speed while it arrives, then Open and Show in folder; a screenshot's thumbnail the moment you take it.
- Calls in Teams, Zoom, Discord, Slack, Webex, WhatsApp, Telegram or Signal, with their running time and a mute that works in every app.
- Mic, camera and screen in use: the app and a coloured dot, and muting the mic in one click.
- The volume mixer: every playing app with its own level, under the volume pop-up.
- Scripts can show their progress in the island: `reui-bridge activity build --progress 0.42`, then `--state done`.
- Appearance: dark, light or following Windows; Island or Notch style; roundness, padding, size and accent.
- Out of your way: open on a click or a rest, hide until something needs you, reveal from the top edge only above the pill, rules per app, and right-click a moment to turn its kind off or snooze it.
- A guide to every feature and shortcut (`docs/guide.md`).
