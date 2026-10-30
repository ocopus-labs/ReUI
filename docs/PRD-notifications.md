# PRD: Notifications, opt-in per app

Status: spike done Oct 30, 2026 (measurements, a prototype module off by default); the phase A/B decision is open (§12) · Owner: Rohit · Item 1 of [research-features.md](research-features.md)

**In one paragraph:** ReUI can show other apps' notifications without package identity, cheaply. `UserNotificationListener` reads the notification centre from a plain per-user exe. Its change event fails without identity, but the shell keeps the notification count in a WNF state. That state changes within ~35–220 ms of every toast and every removal, and never otherwise, so ReUI lists only when it moves. Measured on the prototype: **0.0 ms CPU in 120 s idle** (one thread, +1.1 MB) and ~8 ms per toast. Plain polling, the other option without identity, costs more as the notification centre fills (0.5 % of a core at 5 s with twenty toasts waiting). A signed sparse package (phase B) would replace the undocumented WNF trigger with the documented event, once ReUI has code signing.

## 1. Problem

Windows toasts land in the bottom-right corner, which you aren't watching, and you can't move them. People ask island apps for this more than for anything else. It's boring.notch's most-reacted open request (#592), and eight Windows islands ship it ([research-features.md](research-features.md)).

## 2. Goals

1. A toast from an app **you picked** shows on the island as a Brief: the app's icon, the title, and one line of text.
2. **Off until you turn it on.** Nothing is read until then, and after that only the apps you chose are read past their name.
3. **Nothing leaves the PC, and nothing about a toast is stored or logged.**
4. **Idle costs nothing:** no work between toasts, and a small fixed cost per toast.
5. It works with today's distribution: an unsigned per-user setup exe from GitHub, no admin.

## 3. Non-goals

- Replacing Windows' toasts, or hiding them. Windows still shows its banner; the island is a second place to see the toast.
- Replying, or pressing a toast's own buttons. Windows doesn't expose a toast's actions to a listener, with or without identity (§9).
- Notification history. The notification centre already keeps it.
- Badges and tiles.

## 4. What Windows allows: measured on this PC

Windows 11 Pro 26200, release build, `crates/reui-hub/examples/notifications_probe.rs` (rerun any line with `cargo run -p reui-hub --example notifications_probe -- <mode>`). Every measurement used test toasts the probe sent itself, under Windows PowerShell's AUMID and the probe's own tag. The user's own notifications were only counted and timed, never printed, removed or cleared.

| Question | Result |
|---|---|
| `UserNotificationListener::Current()` | Works unpackaged, 4–6 ms |
| Access | `GetAccessStatus` = Allowed. This follows Settings › Privacy & security › Notifications: "Notification access" and "Let desktop apps access your notifications" (`HKCU\…\ConsentStore\userNotificationListener` `Value` and `NonPackaged\Value` = Allow). `RequestAccessAsync` answered Allowed in 4.4 ms and showed **no prompt**: an unpackaged app can't ask, it can only find out. When access is off, the docs say the calls fail silently (an empty list) |
| `GetNotificationsAsync(Toast)` | 1 toast waiting: median 8–9 ms wall, 1.6–3.1 ms CPU in the caller, ~1.9 ms in the notification service (`svchost` with `WpnUserService`). **Grows with the notification centre:** 20 toasts took 63 ms wall and **28 ms CPU** (100 Mcycles) per call. Windows keeps at most 20 per app |
| Per-toast fields | Id: µs (already local once listed). AppInfo plus text: ~10 ms per toast (194 ms for 20), so only new ids are read. AUMID and display name ("Windows PowerShell"): yes. `PackageFamilyName`: empty for a desktop app. Title and body lines from the ToastGeneric binding: yes. Creation time: yes. **`GetLogo` is null for a desktop app.** Packaged apps' logos are still to verify (§11) |
| `NotificationChanged` | **Fails: 0x80070490 "Element not found"**, as the research said |
| `ClearNotifications` | Not called: it would clear the user's notifications. Reported to fail the same way (research, [egungor #78](https://github.com/egungor/stm32-desk-dashboard/pull/78)). ReUI never wants it anyway |
| `RemoveNotification(id)` | **Works unpackaged** (18.6 ms), tested on the probe's own toast |
| Send → listable | 30–100 ms after `Show` |
| Polling at 1 / 2 / 5 s, 1 toast waiting (60 s each) | 0.13 % / 0.078 % / 0.026 % of one core in the caller (1.3–1.6 ms per poll), plus the service's share. 3,600 / 1,800 / 720 wakeups and RPCs an hour. Delay to see a toast: send→listable plus a uniform 0..T (mean ≈ T/2) |
| Same, 20 toasts waiting (from the per-call cost) | ≈ 2.8 % / 1.4 % / 0.56 % of one core |
| WNF states that move with a toast | Compared the change stamps of all 1,605 well-known states (`HKLM\SYSTEM\CurrentControlSet\Control\Notifications`) across a quiet 3 s and a test toast, three rounds. **`0x0D83063EA3BC1035` and `0x0D83063EA3B8D035`** changed every time; their u32 is the toast count (1, 2, 3, then 0 on removal). Subscribed: the callback came **33–216 ms after a send, 2.8 ms after a removal, and never in a quiet minute**. `…A3BD0035` (no data) fired for some toasts only; `…A3BF5875` fired once |
| `wpndatabase.db` | **Not written for 110 s after a toast**: the WAL's size and modified time didn't change, so writes are batched. The service holds the file open, and the schema is undocumented |

## 5. Options

| Option | Verdict |
|---|---|
| **A1. Poll `GetNotificationsAsync`** | Documented and works. But its cost grows with every toast left in the notification centre, and it wakes the hub and the service at every interval. Kept **only as a fallback** (§6.3) |
| **A2. WNF count as a trigger, then list (prototyped)** | Event-driven: zero cost between toasts. Undocumented, the same kind of dependency as the Do Not Disturb read in `timers/dnd.rs`. It's only a trigger: what shows always comes from the documented listener. A slow safety list catches a build where the count stops moving and drops to A1. **Recommended for phase A** |
| **B. Signed sparse package (identity)** | `NotificationChanged` and `UserNotificationChangedTrigger` become the documented trigger, and ReUI gets its own row in the notification-access settings. Needs a certificate every PC already trusts (§8). **Phase B, once code signing exists** |
| UI Automation on toast banners / the notification centre | Rejected. It sees a banner only while it's on screen (none in DND, fullscreen or Focus), needs a system-wide UIA event hook (costly, can stall on hung UI), breaks when the shell's XAML changes, and loses toasts that skip the banner |
| Read `wpndatabase.db` | Rejected. It's undocumented and held by the service, and writes are batched (measured: nothing within 110 s), so it's neither fresh nor a trigger. It's also the whole history of every app's notifications, more than ReUI should ever open |
| ETW (`Microsoft-Windows-PushNotifications-Platform`) | Rejected. A real-time session needs admin or Performance Log Users, and it's undocumented as an interface |

## 6. Experience

### 6.1 Turning it on

- Settings → **Notifications** (a later milestone; today `notifications.json` or `--notifications`). The page opens with its Enable switch **off**, and a one-paragraph description of what is read and that nothing leaves the PC.
- Turning it on checks access. If Windows' switch is off, one line says so and links to `ms-settings:privacy-notifications`. ReUI can't prompt (measured).
- **Pick apps:** the picker lists the apps that notified lately (`seen`: AUMID and display name, newest first, never a toast's text). The module records them from the moment it's on, even before any app is picked. Nothing is picked by default.
- **Hide content:** the island shows the app alone ("Teams"). The title and text are then **never read**, not just hidden.

### 6.2 What the island shows

| Where | What |
|---|---|
| Compact | Leading: the app's icon (a bell glyph until there is one). Trailing: the title, or the app's name when content is hidden |
| Bubble (something else in the pill) | The app's icon |
| Open (click) | Eyebrow: the app. Hero: the title. Detail: the first line of the text. One row: **Open** |

- **Always a Brief, never a Decision or Alert.** It never expands by itself, and it never takes the pill from music, a timer, a call or an agent: it shows in the bubble beside them, then goes. Agent and call moments always come first.
- It stays for 6 s. A burst shows the newest toast. A toast removed from the notification centre (dismissed, or opened there) goes from the island too.
- **Do Not Disturb** (Windows', or a ReUI focus session): nothing shows, as Windows holds back its banners. **Fullscreen:** the island is already hidden (`hide_in_fullscreen`).
- No text that teaches, and no counts.

### 6.3 Actions

- **Open** starts the app or brings it forward (`shell:AppsFolder\<AUMID>`). What the toast itself opens (a chat, a mail) is the app's own activation, which only Windows' toast UI can invoke (§9).
- **Also remove from the notification centre** (off by default; an M3 setting): after Open, `RemoveNotification(id)`, which works without identity. Never `ClearNotifications`.

### 6.4 Privacy

- Off by default, allow-list only. A toast from an app that isn't allowed is read for its AUMID and name (for the picker) and nothing else.
- The title, text and logo live only in memory, for the Brief's 6 s. They are never written to disk or logged; the module's log lines never include a toast's app or text.
- Codes in a toast (one-time passwords) go through the clipboard's secret filter before showing (M2).
- Nothing goes online.

## 7. Cost budget

| | Budget | Prototype (measured, release, other modules off) |
|---|---|---|
| Idle, feature off | 0 (module not constructed) | 0 |
| Idle, feature on | No wakeups between toasts | **0.0 ms CPU in 120 s** (under 15.6 ms of timer resolution, i.e. < 0.013 % of a core); +1 thread, +1.1 MB private |
| Per toast | < 20 ms CPU | ~8 ms (10 toasts: 78 ms, the list growing from 1 to 10); 10 removed at once: 15.6 ms |
| Safety list (WNF mode) | ≤ 1 list / 5 min | 1 list / 5 min (≈ 3–28 ms each) |
| Fallback polling | ≤ 0.1 % of a core with 20 toasts waiting | 5 s, 2 s for a minute after a toast, 15 s after 10 quiet minutes; none while locked. ≈ 0.56 % at 5 s with 20 waiting, so the fallback is a safety net, not the plan |

Why not poll at 1–2 s: a list serialises every toast waiting, so the cost of polling follows the notification centre. 2 s with twenty toasts is ≈ 1.4 % of a core, all day, for a feature that should cost nothing.

## 8. Phase B: what package identity needs and unlocks

From Microsoft Learn ([grant identity to unpackaged apps](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/grant-identity-to-nonpackaged-apps)):

- **Needs:**
  - **An identity package:** a sparse MSIX with an `Identity` (Publisher = the certificate's subject), `uap10:AllowExternalContent`, `runFullTrust` and `unvirtualizedResources`, an `Application` with `TrustLevel="mediumIL"` and `RuntimeBehavior="win32App"`, and `AppListEntry="none"`. It's built with `MakeAppx pack /nv`. Listening also needs the `userNotificationListener` capability ([explorer-lite #14](https://github.com/justdiegocom/explorer-lite/pull/14) did this).
  - **A signature** with a certificate the PC already trusts. A self-signed certificate must be imported into Trusted People first, which is acceptable for development only. The product-shell PRD rules out "trust this certificate" steps. Learn names Azure Trusted Signing (now Azure Artifact Signing) for production. SignPath's free OSS programme would work too.
  - **An `<msix publisher packageName applicationId>` element in `reui-hub.exe`'s application manifest.** It's ignored while the package isn't registered, so it's harmless in unsigned builds.
- **No admin:** a per-user registration (`Add-AppxPackage -Path … -ExternalLocation <install dir>`, or `PackageManager.AddPackageByUriAsync` with `ExternalLocationUri`) is the documented per-user path. Only machine-wide provisioning needs elevation. `reui-setup` would register it after copying the files and remove it on uninstall. A re-register needs a new package version, and the external location must be the real install folder. Requires Windows 10 2004 (19041), which is already ReUI's minimum.
- **Unlocks:**
  - `NotificationChanged`: the documented, push trigger, which replaces the WNF state and the safety list.
  - `UserNotificationChangedTrigger` background tasks (not needed: the hub is running).
  - A named "ReUI" row in the notification-access settings, and the documented consent flow.
  - ReUI's own AUMID for toasts (not needed: ReUI speaks through the island). Unpackaged apps can already send toasts through the Windows App SDK.
- **Side effects to check:**
  - With identity, Windows' privacy settings attribute ReUI's microphone use (the visualizer's loopback) to the package.
  - Packaged-identity rules apply to the hub's file-system and registry writes. `unvirtualizedResources` keeps them where they are today.
  - An enterprise sideloading policy can block registration. The feature then stays in phase A.
- **Signing pays twice:** the same certificate also signs the setup exe, which ends SmartScreen's "Windows protected your PC" (README's install note).

## 9. Impossible without identity (and what stays impossible with it)

| | Without identity | With identity |
|---|---|---|
| Read toasts, ids, app name, text, time | ✓ | ✓ |
| Remove one toast | ✓ | ✓ |
| Change event / background trigger | ✗ (0x80070490). WNF count used instead | ✓ |
| Clear all | ✗ | ✓ (never used) |
| A desktop app's logo through the API | ✗ (null). The shell icon by AUMID works without identity (M2) | Same API, same null for desktop apps |
| ReUI's own row in notification-access settings, a consent prompt | ✗. Governed by "Let desktop apps access your notifications" | ✓ |
| Press a toast's buttons, reply inline, run its launch arguments | ✗ | ✗: the listener exposes no activation |

## 10. Milestones

- **M0 (done, this spike):**
  - The probe: `examples/notifications_probe.rs`.
  - `modules/notifications/`: `wnf` trigger, `listener` thread, `schedule` (tested), `diff` (tested), `view` (tested), `settings` (`notifications.json`, tested).
  - Off by default (`--notifications`, `"enabled": true`).
  - The allow-list and `seen` live in the file.
- **M1:**
  - A Settings page: Enable, access status with a link to Windows' page, an app picker from `seen`, and Hide content.
  - `reload` already works.
  - Guide and README entries.
- **M2:**
  - Icons for desktop apps (`SHCreateItemFromParsingName("shell:AppsFolder\<AUMID>")` → `IShellItemImageFactory`).
  - The secret filter on the text.
  - Grouping a burst from one conversation (Notchify).
  - Keeping the Brief while it's open (it needs a "the card is open" signal from the island).
- **M3:** Remove from the notification centre on Open (off by default).
- **Phase B:**
  - The identity package in `package.ps1`, signed in CI.
  - `reui-setup` registers and unregisters it.
  - The listener prefers `NotificationChanged` when `Package::Current` exists, and the WNF trigger becomes the fallback.

## 11. To verify

- The WNF count on Windows 10 2004–22H2 and on Windows 11 23H2/24H2. If it's missing (the subscription fails), the schedule polls. If it misses a toast (the safety list), it drops to polling and says so in the log.
- Whether the count still moves during Do Not Disturb, and for apps whose banners are off. Either way the island shows nothing in DND, and the safety list catches the rest.
- A packaged app's `GetLogo` (Teams, WhatsApp). This PC had none waiting.
- A toast updated in place (same tag) keeps its id and the count, so it isn't announced again. That's probably right for progress toasts, and possibly wrong for a chat app that replaces its toast.
- The Denied path, by turning Windows' switch off (not done here; it's the user's setting).

## 12. Open decisions

1. **Phase A on the WNF trigger** (recommended: event-driven, measured free at idle, undocumented but checked and with a fallback) **or polling only** (documented; 0.03–0.6 % of a core depending on what's waiting) **or nothing until phase B**.
2. **Code signing** (Azure Artifact Signing, or SignPath for OSS): it unlocks phase B, and the end of the SmartScreen warning.
3. Whether **Remove from the notification centre** belongs in the product at all (M3).

## Appendix: sources

- Microsoft Learn: [Notification listener](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/notification-listener) (capability, access, `GetNotificationsAsync`, `RemoveNotification`, `ClearNotifications`, `UserNotificationChangedTrigger`, `NotificationChanged`); [Grant package identity by packaging with external location](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/grant-identity-to-nonpackaged-apps) (manifest, signing, per-user registration without admin, troubleshooting); [PackageManager.AddPackageByUriAsync](https://learn.microsoft.com/en-us/uwp/api/windows.management.deployment.packagemanager.addpackagebyuriasync).
- Others who hit the same walls (Oct 2026): [foxglove #564](https://github.com/BanditF/foxglove/pull/564), [stm32-desk-dashboard #33](https://github.com/egungor/stm32-desk-dashboard/issues/33) / [#78](https://github.com/egungor/stm32-desk-dashboard/pull/78) (`NotificationChanged` and `ClearNotifications` 0x80070490, `RemoveNotification` works), [explorer-lite #14](https://github.com/justdiegocom/explorer-lite/pull/14) (sparse package with `userNotificationListener`; live events once registered), [versailles #53](https://github.com/crasyK/versailles/issues/53) (dropped for want of a certificate; warns against self-signed installs).
- Measurements: `crates/reui-hub/examples/notifications_probe.rs` (modes `access`, `read`, `events`, `poll`, `latency`, `scale`, `wnf`, `wnfwatch`, `remove`).
