# ReUI guide

ReUI puts a dynamic island at the top of your screen: a small black pill that shows what's happening on your PC and lets you act on it without switching windows. This page covers what each feature does and every shortcut and gesture. Settings (left-click the tray icon, or launch ReUI again from the Start menu) has a page for each feature.

## The island

| Do this | It does |
| --- | --- |
| Rest the pointer on the pill | After a short pause it becomes clickable and peeks. Until then clicks go through to what's underneath (browser tabs stay usable) |
| Click the pill | Opens the card |
| Rest the pointer on the pill while nothing is happening | Shows the date and time |
| Click the pill while nothing is happening | Opens a row of glyphs: Clipboard, Timers, Devices, System stats (and Now Playing while something plays). Click one to go there; Esc or moving away closes the row |
| Move the pointer away | The card folds back |
| Scroll the wheel over the island | Steps through everything that's going on; over a row of cards it scrolls the row |
| Click the small bubble beside the pill | Brings that activity into the pill |
| Right-click the pill or the tray icon | Settings, Pause ReUI for 1 hour, Hide the island (Show the island while hidden), Clipboard history, Devices, System stats, Quit |
| Push the pointer against the top edge | Shows a hidden island (Settings → General → Show the island: When active / When needed) |
| Click a glyph under an open card | Goes to that view: Clipboard, Timers, Devices, System stats, and Now Playing while something plays. The wheel over the glyphs steps through them; Ctrl+Tab / Ctrl+Shift+Tab does too while the card has the keyboard (clipboard history, the timer field). Off in Settings → General → View switcher |
| Click the dot beside the glyphs | A permission request or question arrived while you were in a view: the dot (in its colour) says it's waiting, and a click brings it forward |

The island hides while a game, video or presentation fills the screen (Settings → General → Hide in full screen), and can sit on your main display, the display the pointer is on, or one display you choose (Settings → General → Display). In "When active" and "When needed" it still shows in full whenever the desktop is in front (you clicked the wallpaper or pressed Win+D), and keeps out of the way inside apps (Settings → General → Always show on the desktop). If you hide the taskbar, Settings → General → Clock in the resting island puts the time in the island while nothing else is showing.

**Appearance** (Settings → Appearance, with the island previewed at the top):
- **Style:** *Island* floats below the top edge; *Notch* joins it to the edge like a Mac's notch (flat top, curved ears where it meets the edge), with a second activity inside its right end instead of a bubble.
- **Theme:** *Dark* (black, the default), *Light* (white with a hairline edge), or *Auto*, which follows Windows' app mode (Settings → Personalization → Colors) as soon as you change it.
- **Accent:** ReUI blue or your Windows accent, for whatever has no colour of its own (music, timers and agents keep theirs).
- **Roundness** (square-ish to a capsule), **Gap from the top** (Island only), **Padding** (Compact / Regular / Roomy, the space inside an open card) and **Size** (90–125 %). Inner corners always follow the outer ones.

The island picks up a change within a moment; nothing else restarts.

**Pause ReUI** (tray, or Settings → General) quiets the island and stops recording the clipboard for an hour. Permission requests from coding agents still show.

**Win+Alt+I** hides the island until you press it again (or pick Tray → Show the island). Unlike a pause, everything keeps working: the clipboard is still recorded, timers still run, and permission requests and questions from coding agents still show; a card you open with its shortcut (Win+Alt+V, Win+Ctrl+T, Win+Alt+S) shows too. Change it in Settings → General → Hide the island. (Not Win+Alt+H: Windows uses that for voice typing.)

## Now Playing

Whatever plays on this PC (Spotify, browsers, VLC, Media Player: anything in Windows' media controls) shows its artwork, title and artist, with a timeline and the controls the player offers.

| Do this | It does |
| --- | --- |
| Swipe sideways on the pill (trackpad, or a sideways wheel) | Next or previous track |
| Drag the timeline | Scrub |
| Click the artwork | Opens the player |
| Scroll over the volume row | Changes the volume; click its icon to mute |

Synced lyrics are off until you turn them on (Settings → Now Playing → Synced lyrics); song titles are sent to lrclib.net to find them.

## Clipboard history

Everything you copy (text with its formatting, images, files) is kept on this PC and comes back as rich cards. Copies from password managers and anything that looks like a secret are never saved.

| Shortcut | It does |
| --- | --- |
| **Win+Alt+V** | Opens or closes clipboard history (change it in Settings → Clipboard) |
| Type | Searches |
| ← / → | Selects a card |
| Enter, or click | Pastes into the app you were in |
| Shift+Enter, or Shift+click | Pastes as plain text |
| Ctrl+Enter | Copies only |
| Alt+1 … Alt+9 | Pastes the 1st … 9th card |
| Ctrl+P | Pins or unpins |
| Delete | Deletes (Ctrl+Z brings it back) |
| Tab | Next tab (History, Pinned, then your collections) |
| Esc | Clears the search, then closes |
| Point at a card | Its star (pin or unpin) and bin (delete; Ctrl+Z brings it back) appear in its corner |
| Right-click a card | Its menu |
| **Ctrl+Alt+1 … Ctrl+Alt+9** | Pastes a recent item without opening anything |

If the app you paste into runs as administrator, the item is put on the clipboard and the island says "Press Ctrl+V".

## Volume and brightness

The volume and brightness keys show the level in the island. Hover or click it to drag the level, scroll to change it, click the icon to mute, or switch the output device from its menu. Caps Lock, Num Lock, battery saver and microphone mute can show too (Settings → Volume & brightness).

**Replace Windows' pop-up** hides Windows' own volume and brightness pop-up while ReUI runs. It comes back when ReUI quits, and when you uninstall.

## Devices

Bluetooth devices show a card with their battery when they connect; this PC and its devices warn at 20 % and 10 %; plugging in shows the time to full; USB drives and memory cards show with **Eject**. **Devices** in the tray menu shows every connected device's battery. AirPods case and bud levels come from a short Bluetooth scan nearby, so that's off until you turn it on.

## Timers

Press **Win+Ctrl+T** (or Tray → Timer → New timer…) and type how long: `10` (minutes), `10m`, `90s`, `1h30`, `5:00`, with a label before or after (`10m tea`). Enter starts it; the island shows it counting down. Several can run at once; the one ending first is in the pill and the open card pages through the rest. When one ends the island opens with a sound: **Stop**, or **+1 min**. Timers keep running through sleep and restarts; one that ended meanwhile is shown quietly as "Finished at …".

Type `focus` for a focus session (25 minutes of work, 5-minute breaks, a 15-minute break after every fourth); the island asks before each break and before going back to work. Settings → Timers can make it go on by itself and turn on Do Not Disturb while you work. Type `sw` for a stopwatch, with laps.

`reui-bridge timer 10m tea` starts one from a script or a coding agent.

## Mic & camera

While an app uses your microphone or camera, or shares or records your screen, the island shows its icon and what it uses: orange for the microphone, green for the camera, purple for the screen (and blue for location, if you turn it on in Settings → Mic & camera). Click it to see every app using something and since when, mute the microphone, or open Windows' privacy settings for it. A screenshot doesn't count; a camera that turns on for a moment still shows for a few seconds.

## System stats

Press **Win+Alt+S** (or Tray → System stats) to see how busy the PC is: CPU and memory side by side, each with a ring and a line of the last minute (since the card opened; it fills in from the right), then GPU, disk and network on one line. On a laptop the line under CPU says what the battery gives or takes. A number that stays at 90 % or more for three seconds turns orange. The button at the top right opens Task Manager. Press the shortcut again, or click elsewhere, to close it. The numbers update once a second while the card is open and aren't read at all otherwise. Settings → System stats changes the shortcut and which facts show. `reui-bridge stats` opens it from a script.

## Coding agents

Claude Code, Codex and OpenCode show in the island only when they need you: a permission request you can answer with **Allow** / **Deny** (Claude Code and OpenCode also offer **Always**), a question from Claude Code or OpenCode, a plan or an MCP server's form from Claude Code, a question from Codex to answer in its terminal, or a turn that failed. While they think and run tools, the island stays with your music and timers. Each card starts with a quiet line saying which session it is (the project, what you asked, how long it has been working); a long command or an edit's changed lines show underneath, and the mouse wheel scrolls them. Anything that looks like a password or token shows as dots.

- **Deny and say why:** Alt+click **Deny** (or Shift+Enter once the card has the keyboard) opens a field, "Tell Claude what to do instead"; Enter sends it with the deny.
- **Questions:** click an option to answer. With several to pick, tick them and press **Next** (the last step says **Send**); ‹ on the top line goes back with your answers kept. Type in "Type an answer" for something else (OpenCode only offers it when the question allows it). Nothing reaches the agent until the last step. A long list scrolls with the wheel.
- **Plans:** **Approve**, or **Keep planning** to say what should change (Enter sends it to Claude as feedback).
- **MCP forms** ("Jira (MCP)"): one field at a time: pick from a list, Yes / No, or type into the field and press Enter. **Next** shows once a required field is filled; the last step says **Submit**. A number that doesn't read as one says so under the message. **Decline** says no to the server. A form too complex for the card (lists of things, more than six fields) only offers **Answer in terminal**.
- **MCP links:** **Open link** opens it in your browser and tells Claude it's done; **Decline** says no.
- **Codex questions** show with their options, but only Codex's terminal can answer them: **Answer in terminal** brings that window forward. The card goes once you've answered there. A question asking for a secret shows only its short title.
- **Answer in terminal** hands it back to the agent's own prompt at once, and brings the window it runs in forward.
- **A failed turn** says why, with **Open terminal** beside it; it goes by itself after a while.
- **Several at once:** one card, oldest first; the pill shows how many, and ‹ › on the top line moves between them.
- **Answered in OpenCode itself:** the card goes; whichever answer comes first counts.

A card that arrives by itself never takes your keyboard, so a stray Enter in your editor can't allow anything. Click the card to give it the keyboard: ↑ / ↓ move through choices, 1–9 pick one, Space ticks one, Enter answers (on a permission: the tinted button, Allow), Shift+Enter denies with a message, Esc gives the keyboard back. While the card has the keyboard, what you type goes to it, not to your editor.

If Settings → Coding agents says Claude Code's questions need newer hooks, press **Update hooks**; until then they show with only **Answer in terminal**.

Settings → Coding agents → **Show sessions** brings back what they're working on ("While working"), and **Finished turns** adds a short moment when a turn ends.

Connect an agent in Settings → Coding agents (it shows the file that changes; Disconnect removes exactly what was added). Each agent's row says whether it's connected and when ReUI last heard from it; **Send test** makes the island confirm the connection. Codex asks you to trust new hooks once: run `/hooks` in Codex. OpenCode picks the plugin up on its next start; when ReUI updates, it updates the plugin too (restart OpenCode to load it).

## Updates

ReUI checks GitHub for a new version once a day (Settings → About → Check automatically; nothing personal is sent). When one is out, the island shows it with the first line of what's new: **Install** downloads it (you see how far it is), checks it, installs it and restarts ReUI; **Later** hides it for a day. The same Install is in the tray menu (Install update) and in Settings → About. If something goes wrong, the island says why and offers Try again. After an update, About shows what changed (Settings → About → What's new after updates).

## Help

- **Replay the tour:** Settings → About → Replay the tour.
- **Report a bug:** Settings → About → Report a bug opens a GitHub issue with only ReUI's version and the Windows build filled in.
- **Logs:** `%LOCALAPPDATA%\ReUI` (Settings → About → Logs).
- **Your data:** settings and clipboard history in `%APPDATA%\ReUI`, caches and logs in `%LOCALAPPDATA%\ReUI` (Settings → Privacy & data).
