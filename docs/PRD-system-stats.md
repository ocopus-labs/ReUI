# PRD: System stats (CPU, GPU, memory, disk, network in the island)

Status: M0–M1 built, the v2 card (§5.2, from M3: sparklines and Task Manager) built, M2 and the rest of M3 proposed · Owner: Rohit · "System stats" in [PLAN.md](../PLAN.md)'s catalogue · Oct 30, 2026

**References:**
- **Windows Task Manager** (Performance tab): the numbers people trust and compare against. CPU, memory, disk active time, network throughput and each GPU's busiest engine, refreshed once a second ("Normal" update speed). It is a window you open and close; it costs a few percent of a core while it shows.
- **macOS menu bar monitors:**
  - **iStat Menus** ([Bjango](https://bjango.com/mac/istatmenus/)): a compact figure or sparkline per stat in the menu bar, with a detailed dropdown (per-core, processes, history graphs).
  - **Stats** ([exelban/stats](https://github.com/exelban/stats), open source): the same idea, one module per stat, each with its own update interval; users file issues about its own CPU use when many modules sample at 1 s.
  - Both keep sampling while their menu bar items are visible, which is always. That is the cost we don't want.
- **Apple's Dynamic Island** has no system stats. Notch apps on the Mac (NotchNook, Alcove, boring.notch) don't either; a few third-party widgets add a CPU line. There's no convention to follow, so the card follows ReUI's own: eyebrow → hero → detail, then quieter facts.

**Inputs:**
- v1 used `Row::Progress { fraction, leading, trailing }` (a meter with a caption on each side) and needed no protocol change. The v2 card (§5.2) adds `Row::Gauges`, `Row::Facts` and `Element::Button` (protocol 19).
- `expanded` / `collapsed`: the island already tells a module when its card opens and closes, for any reason.
- Summoned cards (the timers' launcher, Devices' overview): a hotkey or the tray publishes a Passive activity and asks the island to open it.
- `crates/reui-settings/src/stats.rs` already measures ReUI's own CPU and memory (About), sampled only while About shows; the same rule applies here.

## 1. Problem

"Why is my fan loud?" means opening Task Manager, waiting for it, finding the Performance tab, then closing it again. A glance at CPU, GPU, memory and network is exactly what the island is for: one keypress, read it, gone. But stats are polled by nature (no OS event says "CPU changed"), and ReUI's first rule is that idle costs nothing. Every menu bar monitor on the Mac breaks that rule by sampling all the time.

## 2. Goals

1. **One keypress to see how busy the PC is:** CPU and memory as the hero, each with its last minute, then GPU, disk and network as one line of facts.
2. **Numbers that match Task Manager**, so nobody has to wonder which is right.
3. **Zero cost while the card is closed:** no thread, no timer, no counters open, no hub wake-ups. Measured, not assumed.
4. **A small, stated cost while it's open:** 1 Hz sampling on its own thread, never on the hub thread.

## 3. Non-goals

- A process list ("what's using it") and killing processes: Task Manager's job, one click away.
- History graphs beyond the last minute. The v2 card's sparklines hold only what was read since the card opened; nothing is recorded while it's closed, and a new opening starts empty.
- Per-core CPU, temperatures, fan speeds, clock speeds (temperatures need a kernel driver or WMI vendor classes, both slow and unreliable).
- Alerts ("CPU over 90 % for a minute"): that would need sampling while closed.
- Remote machines, logging to a file.

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| S1 | Fan spins up | Win+Alt+S. The card opens with CPU 87 % and memory 67 % ("21.4 of 31.7 GB") as gauges, GPU 12 % under them. Three seconds later CPU's numerals turn orange (still ≥ 90 %), and its line shows the climb. Switching apps or Win+Alt+S again closes it, and sampling stops |
| S2 | Download crawling | Tray → System stats. Network reads "↓ 1.2 MB/s  ↑ 80 KB/s" |
| S3 | Game stutters | Win+Alt+S over a windowed game: GPU 99 % · 7.8 GB. In exclusive fullscreen the island is hidden, so the card doesn't open and nothing is sampled |
| S4 | Laptop on battery | The line under CPU's numerals reads "On battery, 14 W"; plugged in, "Charging, 45 W" |
| S8 | What is it? | The glyph at the card's top right opens Task Manager (processes, per core, longer history) |
| S5 | Left open | The island closes an untouched summoned card after a minute; sampling stops with it |
| S6 | A script wants it | `reui-bridge stats` opens the card (also how it's tested) |
| S7 | Pinned CPU (M2, opt-in) | The compact pill shows a CPU ring and "23%" while no other activity is in front. Costs one `GetSystemTimes` every 2 s only while that pill is on screen (§9) |

## 5. Experience

### 5.1 Opening it

- **Hotkey:** **Win+Alt+S** by default, configurable like the clipboard's and the timers'. Chosen because Windows 11 doesn't use it (Microsoft's list has Win+Alt+B, D, H, K, R and Enter; Game Bar adds G, R, T and PrtScn), PowerToys' defaults don't either, and it sits beside ReUI's Win+Alt+V. Win+Ctrl+S was the old Speech Recognition toggle and Win+Ctrl+P is free but meaningless. If another app has it, Settings says so (like the others) and the tray still works.
- **Tray and pill menu:** "System stats", after Devices.
- **Settings → System stats → Show**, and `reui-bridge stats`.
- Pressing the hotkey again closes it, like the clipboard panel.

### 5.2 The card (Passive, key `system`, summoned; accent `color.system.teal` #40C8E0)

The v2 card (roomy; `stats/view.rs`, drawn by the island's `render/rows/gauges.rs` from `reui_design::layout::gauges`, DESIGN.md "System stats"):

![The System stats card](images/system-stats.png)

- **Header:** the eyebrow "System" alone, and a bare glyph at its end that opens Task Manager (`Element::Button`, action `taskmgr`; the island's click allows the foreground, `ShellExecute` handles Task Manager's elevation). No headline: the gauges are the hero.
- **Hero, `Row::Gauges`:** CPU and Memory side by side. Each is a ring filled to its share (the accent) with a quiet glyph inside (a chip, a memory module, in the secondary label colour), the label above big tabular numerals with a smaller, quieter "%" on their baseline, and one fact under them:

| Gauge | Ring and numerals | Under them |
|---|---|---|
| CPU | busy share | the battery's flow when there is one, "On battery, 14 W" / "Charging, 45 W"; on a desktop nothing |
| Memory | in use / total | "11.2 of 31.7 GB" |

  Under each, across its column, **the last minute** as a sparkline: a 1.5 px accent line, a soft accent area fading down to the zero line, the newest point marked with a dot. One point a second, kept in `history.rs` only while the card is open, newest at the right, so it fills in from the right; it never shows time that wasn't measured. 0–100 % fills its height (an honest scale: an idle CPU's line lies low).
- **`Row::Facts`:** one quiet line for the rest, each a tertiary label over its tabular value, spread from inset to inset; left out when turned off or unknown (never shown as 0). No row at all when none is left.

| Fact | Value |
|---|---|
| GPU | "41%", or "41% · 2.1 GB" with its dedicated memory in use |
| Disk | "3%" (active time) |
| Network | "↓ 1.2 MB/s  ↑ 80 KB/s" |

- **Colour carries meaning only:** the accent tints every ring, line and area; no colour per metric. A number at 90 % or more for three readings running (CPU, memory, GPU, disk) turns `Warning`; one quiet second ends it.
- **Compact** (only seen for a moment while it opens or closes): the ring and "23%". **Minimal:** the ring.
- Updates once a second; the island redraws the open card once per reading. Nothing animates between readings.
- **No filler:** no "Updated 1 s ago", no units legend, no axis labels, no icons per fact.

**Why this shape (v2).** v1 was a CPU ring and "1 %" hero over four grey meters: correct, but the meters were the same weight as each other and as nothing in particular, and the network bar (busier direction over link speed) was always a sliver. The questions people bring to the card are "is it the CPU or memory?" and "is it still happening?". So those two get the hero, side by side, and the last minute answers "still?" without anyone remembering the number from a second ago. A reference (a Mac notch app's stats panel) used a colour per metric, icons per row and nested panels; here one accent does all the drawing, so the only colour that changes is the one that means something (orange when a number stays high). GPU, disk and network are rarely the answer, so they're facts, read when wanted and quiet otherwise. Task Manager is the honest next step for "what is it", so it's one click from the header instead of a process list here.

### 5.3 What it never does

- Sample while the card is closed, or while the island is hidden.
- Stay in the pill after it closes: the activity exists only while it's open.
- Show a number it doesn't have: a VM without GPU counters has no GPU row.

## 6. Platform: the cheapest correct read for each stat

Costs measured on this PC (Windows 11 26200, a 12-thread desktop at 3.5 GHz, no battery, 249 GPU engine instances with a normal desktop open), release build, per call: `cargo test --release -p reui-hub stats_cost -- --ignored --nocapture`.

| Stat | How | Cost per read | Notes |
|---|---|---|---|
| CPU | `GetSystemTimes` twice; busy = 1 − Δidle / (Δkernel + Δuser) (kernel time includes idle) | 0.04 ms wall, ~0.01 ms CPU | **Equals PDH `\Processor Information(_Total)\% Processor Time`** (measured side by side under load: 45.6 / 45.6, 64.3 / 64.3). `% Processor Utility` read 25–30 % higher (58.6, 83.7): it scales by the current frequency over base, so it passes 100 on a boosting CPU. Task Manager used Utility from Windows 8 to Windows 11 23H2, and switched to Processor Time in 24H2 (rolled out to everyone in May 2025), so on the Windows 11 ReUI supports today, `GetSystemTimes` matches Task Manager. On Windows 10, Task Manager still reads Utility: expect ours to be lower under turbo there (§13 V1) |
| Per-core CPU | `NtQuerySystemInformation(SystemProcessorPerformanceInformation)` deltas | ~µs | Non-goal for v1; noted for a sparkline grid later |
| Memory | `GlobalMemoryStatusEx`: in use = `ullTotalPhys − ullAvailPhys` | ~1 µs | Task Manager's "In use" (standby counts as available). Shown in binary GB written "GB", as Task Manager does. `ullTotalPhys` is a little under the installed amount (firmware reservations) |
| GPU utilisation | PDH `\GPU Engine(*)\Utilization Percentage` (wildcard), via `PdhAddEnglishCounterW`. Instances are `pid_<pid>_luid_<hi>_<lo>_phys_<n>_eng_<n>_engtype_<type>`: **sum per engine (luid + phys + eng) over processes, then take the busiest engine of any adapter** | 0.27 ms wall, ~0.3 ms CPU (249 instances) | Task Manager's own rule is "the busiest engine" ([DirectX blog](https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/)); its source is VidSch/VidMm in the kernel, which these counters expose. Two classic bugs: taking the busiest *process* (three apps at 44 + 34 + 13 read 44, Task Manager 91: [NetSpeedTray #338](https://github.com/erez-c137/NetSpeedTray/pull/338)) and *adding engine types* (3D + video decode read over 100: [yasb #1163](https://github.com/amnweb/yasb/pull/1163)). `engtype` is left out of the key: it's sometimes blank for one process on the same engine. .NET's `PerformanceCounter` can't see this set; PDH can ([SystemManager #2995](https://github.com/laurentiu021/SystemManager/pull/2995)). The instance count grows with processes using the GPU, so this is the most expensive read; it's skipped entirely when the GPU row is off |
| GPU memory | PDH `\GPU Adapter Memory(*)\Dedicated Usage`, the adapter of the busiest engine | (in the GPU figure) | Same query, one collection. DXGI `IDXGIAdapter3::QueryVideoMemoryInfo` reports *this process's* budget and usage, not the adapter's total, so it's the wrong API for "how full is the GPU"; `DXGI_ADAPTER_DESC.DedicatedVideoMemory` gives the total if we ever show "of 8 GB" (M3) |
| Disk | PDH `\PhysicalDisk(_Total)\% Idle Time`; active = 100 − idle | 0.03 ms wall | Task Manager's "Active time" is per disk; `_Total` averages them. `% Disk Time` is not used: it's a queue-length-based figure that passes 100. `Disk Bytes/sec` is available on the same object if we want throughput (M3) |
| Network | `GetIfTable2`: `InOctets` / `OutOctets` deltas summed over interfaces that are **up, `HardwareInterface`, not `FilterInterface`**, not loopback or tunnel; bar = busier direction × 8 / the fastest `ReceiveLinkSpeed`/`TransmitLinkSpeed` | 0.63 ms wall, ~0.47 ms CPU | The most expensive of the cheap reads (every row is ~1.3 KB and NDIS is asked for each, filters included). Filter rows repeat their adapter's bytes, and a VPN's virtual adapter carries traffic that also crosses the physical one, so counting either doubles it. If it shows up in a profile: list once a minute, then `GetIfEntry2` only the counted LUIDs |
| Battery power | `Windows.Devices.Power.Battery.AggregateBattery().GetReport().ChargeRateInMilliwatts` (negative while discharging), the Devices module's reader (`devices/power.rs`) | WinRT call, < 1 ms | Only on machines with a battery (`GetSystemPowerStatus`); the sampler thread initialises WinRT (MTA) for it |
| Opening the PDH query | `PdhOpenQueryW` + `PdhAddEnglishCounterW` ×3 + a first `PdhCollectQueryData` (rate counters need two) | 0.3 ms wall | Once per card opening, on the sampler thread |

**Total per sample:** about 1 ms of wall time and well under 1 ms of CPU on the sampler thread, once a second: < 0.1 % of one core while the card is open. **Localized Windows:** every counter is added by its English path (`PdhAddEnglishCounterW`), so German or Japanese counter names don't matter. **No PDH** (the service is broken, counters disabled with `lodctr /d`, a locked-down VM): the GPU and disk rows are left out; CPU, memory and network don't use PDH.

**Sampling plan:**
- **Card closed:** nothing exists. No thread, no PDH query, no timer, no hub wake-up. The hotkey is a `RegisterHotKey` on the shared system-events window, which blocks in `GetMessage`.
- **Card open:** one thread, `stats`, created when the card is asked for. First reading after 250 ms (so the card opens with numbers), then every 1 s (Task Manager's "Normal"). It waits on its stop channel with a timeout, so closing the card ends it at once.
- **Never on the hub thread:** the hub only receives a `Reading` and publishes it.

## 7. When the card counts as open

`card.rs`, pure and tested with a fake clock:

| Event | Then |
|---|---|
| Hotkey / tray / command while closed | Start a sampler run (numbered) |
| Its first reading | Publish, ask the island to open the card; it must say `expanded` within 3 s |
| `expanded` | Open; a 30-minute safety deadline starts |
| Each reading | Publish (readings from an earlier run are dropped) |
| `collapsed` (dismissed, hotkey again, another app in front, another card in front, the island hidden for fullscreen) | Stop the sampler, retract the card |
| No first reading in 2 s | The sampler isn't working: close |
| No `expanded` in 3 s | The island is hidden or ignored the request (exclusive fullscreen): close |
| 30 min open without `collapsed` | The island went away without saying so (crashed): close |
| Settings `reload` | Close; the next open reads the new rows |

The island closes an untouched summoned card after a minute (`motion.delay.panel-idle`), so in practice sampling lasts as long as someone is looking.

## 8. Edge cases

| ID | Case | Behaviour |
|---|---|---|
| E-S1 | Hybrid GPUs (integrated + discrete) | One GPU row: the busiest engine of *any* adapter (Task Manager's Processes column does the same), with that adapter's dedicated memory. A sleeping discrete GPU has no busy engines, so the integrated one wins |
| E-S2 | No GPU counters (VM, Basic Display Adapter, WDDM < 2.0) | No GPU row |
| E-S3 | VPN connected | Its virtual adapter isn't counted; the physical adapter carries the (encrypted) bytes, so totals stay right. A VPN whose adapter claims `HardwareInterface` would be counted twice (§13 V3) |
| E-S4 | Wi-Fi and Ethernet both up | Both counted |
| E-S5 | Link speed unknown (all ones) | Nothing changes: since v2 the card shows rates only, no bar against the link |
| E-S6 | Adapter appears or disappears between samples | It counts from its second sample; a counter that went backwards counts as 0 |
| E-S7 | Localized counter names | English paths via `PdhAddEnglishCounterW` |
| E-S8 | PDH unavailable or the disk counters disabled | Those rows left out |
| E-S9 | Desktop (no battery) | No line under CPU's numerals (memory keeps its line; the gauges stay level) |
| E-S10 | The hotkey is taken | Logged; Settings says so when you record it; the tray works |
| E-S11 | Card asked for while the island is hidden (fullscreen) | Not opened; the sampler stops after 3 s |
| E-S12 | ReUI paused | The card still opens on request (it's summoned, like the clipboard) |
| E-S13 | Island crashes while the card is open | The sampler runs until the 30-minute deadline at worst (< 0.1 % of a core) |
| E-S14 | Sleep while open | The island closes the card on the foreground change; a reading straddling sleep is a long delta, still correct |

## 9. Performance budget

- **Closed:** 0 CPU, 0 wake-ups, 0 threads, no memory beyond the module's state (< 10 KB). Measured: §12.
- **Open:** the sampler's < 1 ms of CPU a second, one hub wake-up and one island redraw a second. Measured: §12.
- **Pinned CPU (M2):** a compact view that keeps updating would need sampling while it shows. Cost: one `GetSystemTimes` (~10 µs CPU) every 2 s on a thread, one hub publish and one compact redraw every 2 s, i.e. a few hundred µs a second all told. It also needs a protocol addition (the island telling the module when its compact presentation is on screen, `shown` / `hidden`, alongside `expanded` / `collapsed`), because today a module only hears about its expanded card. That's why it's opt-in and later.

## 10. Settings (`%APPDATA%\ReUI\stats.json`, page "System stats")

| Setting | Key | Default |
|---|---|---|
| System stats module | `enabled` | on |
| Shortcut | `shortcut` | Win+Alt+S |
| GPU fact | `gpu` | on (off skips the GPU counters, the most expensive read) |
| Disk fact | `disk` | on |
| Network fact | `network` | on |
| Power under CPU | `power` | on |

The page also has **Show** (opens the card) and says, once, that it's read only while the card is open.

## 11. Milestones

| M | Scope | Exit |
|---|---|---|
| **M0: Sampler** ✅ | `os.rs`, `pdh.rs`, `maths.rs` (CPU delta, rates, interface filter, busiest engine, tested), `sampler.rs` thread, per-read cost test | Readings match Task Manager on this PC; cost per read measured |
| **M1: Card** ✅ | `card.rs` open/closed state machine (fake-clock tests), view, module, Win+Alt+S, tray entry, `reui-bridge stats`, `stats.json`, `--no-stats`, Settings page | S1–S6; hub idle unchanged with the card closed; cost open measured |
| **M2: Pinned CPU** | Opt-in compact CPU while nothing else is in front; protocol `shown`/`hidden` for the compact presentation; 2 s sampling only while shown | S7; idle cost unchanged when not pinned; pinned cost measured |
| **v2 card** ✅ | §5.2: CPU and Memory gauges with the last minute (`history.rs`, pure, tested), facts, Task Manager in the header, Warning when a number stays high; protocol 19 (`Row::Gauges`, `Row::Facts`, `Element::Button`); layout in `reui_design::layout::gauges`; Settings' Stats page preview | S1–S8; idle unchanged (no new timers; one redraw per reading) |
| **M3: Detail** | GPU "of 8 GB" (DXGI), disk throughput | Reviewed against DESIGN.md's restraint rule before building |

## 12. Acceptance and measurement

- **Unit tests:** CPU busy maths (including no time and backwards), rates across counter resets, the interface filter (filter rows, VPN, loopback, down, newly arrived), the GPU rule (summed per engine, busiest wins, blank `engtype`, clamped), number formats, the card's open/closed machine (toggle, stale runs, each deadline), the view (unknown facts left out, the last minute, Warning only when sustained), the history (a minute at most, skipped readings), the gauges' layout (columns, level rows, sparkline points from the right, facts inset to inset), settings fallbacks, the Settings page writing the keys.
- **Live (this PC):** a `REUI_CONFIG_DIR` sandbox hub (release build) with only this module on (`--no-agents --no-clipboard --no-media --no-osd --no-devices --no-timers --no-updates --no-onboarding`), the card opened with `reui-bridge stats`. CPU from `QueryProcessCycleTime` (exact; process times only move in 15.6 ms ticks) at the nominal 3.5 GHz, the way About's `stats.rs` reads ReUI's own processes:

| State | Hub | Island |
|---|---|---|
| Baseline: same sandbox with `--no-stats`, 30 s | 0.39 ms (0.001 % of a core) | 10 ms (0.03 %) |
| Card closed, 30 s | 0.55 ms (0.002 % of a core): unchanged, both noise | 7.5 ms (0.03 %) |
| Card open, 25 s | 32 ms: **1.3 ms a second, 0.13 % of one core** (0.011 % of the 12-thread machine) | 45 ms: 1.8 ms a second, 0.18 % of one core |
| Closed again after use, 60 s (process times) | 0 ms, the sampler thread gone | 0 ms |

  Open, the hub's share is the sampler's reads (~0.8 ms, §6) plus publishing; the island's is one redraw of the card a second.
- **Screenshots:** the open card on this PC, updating once a second; Settings → System stats (`reui-settings --shots`).

## 13. To verify

| ID | Question |
|---|---|
| V1 | Windows 10 (out of mainstream support since Oct 2025, ESU after): Task Manager there still shows `% Processor Utility`. Do we read Utility on Windows 10 to match, or keep one definition everywhere? Proposal: one definition (`% Processor Time`, also what Process Explorer and Resource Monitor show) |
| V2 | GPU counter cost on a machine with many GPU processes (browsers with many tabs, 1000+ instances): measure; if > 2 ms, fall back to `D3DKMTQueryStatistics` per node (what System Informer uses), which doesn't scale with processes |
| V3 | VPN clients whose adapter claims `HardwareInterface` (some WireGuard and OpenVPN TAP builds): check one, and filter by `TunnelType` / `IfType` 53 (propVirtual) if needed |
| V4 | Laptop: `ChargeRateInMilliwatts` sign and update rate on a few machines (some report only every 10–60 s); the detail line may need to hold the last value |
| V5 | ARM64 (Snapdragon X): GPU engine counters (Adreno, WDDM 3.x) and the NPU engine (Task Manager shows NPU separately; it should not win "GPU") |

## Appendix: sources

- **CPU:**
  - [Microsoft Learn: CPU usage over 100 % with Turbo Boost (Processor Utility vs Processor Time; Task Manager uses Utility since Windows 8)](https://learn.microsoft.com/en-us/troubleshoot/windows-client/performance/cpu-usage-exceeds-100)
  - [Windows Latest: Windows 11 24H2's Task Manager CPU formula rolls out to everyone (May 2025)](https://www.windowslatest.com/2025/05/26/windows-11-24h2s-task-manager-new-cpu-usage-formula-rolls-out-to-everyone/)
  - [Alois Kraus: Windows Task Manager Shows Misleading Values (Utility ≈ Time × f/f_base; fixed in 24H2)](https://aloiskraus.wordpress.com/2025/02/09/windows-task-manager-shows-misleading-values/)
  - [Aaron Margosis: Task Manager's CPU numbers are all but meaningless](https://aaron-margosis.medium.com/task-managers-cpu-numbers-are-all-but-meaningless-2d165b421e43)
  - [GetSystemTimes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getsystemtimes)
- **Memory:** [GlobalMemoryStatusEx](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-globalmemorystatusex)
- **GPU:**
  - [DirectX blog: GPUs in the Task Manager (busiest engine; VidSch/VidMm; dedicated vs shared)](https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/)
  - [yasb #1163: sum per engine instance, report the busiest](https://github.com/amnweb/yasb/pull/1163)
  - [NetSpeedTray #338: busiest engine with its processes summed, not the busiest process](https://github.com/erez-c137/NetSpeedTray/pull/338)
  - [SystemManager #2995: the counters Task Manager uses, read through PDH](https://github.com/laurentiu021/SystemManager/pull/2995)
  - [IDXGIAdapter3::QueryVideoMemoryInfo](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo)
- **PDH:** [PdhAddEnglishCounterW](https://learn.microsoft.com/en-us/windows/win32/api/pdh/nf-pdh-pdhaddenglishcounterw), [PdhGetFormattedCounterArrayW](https://learn.microsoft.com/en-us/windows/win32/api/pdh/nf-pdh-pdhgetformattedcounterarrayw)
- **Network:** [GetIfTable2](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getiftable2), [MIB_IF_ROW2 (HardwareInterface, FilterInterface)](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/ns-netioapi-mib_if_row2)
- **Battery:** [BatteryReport.ChargeRateInMilliwatts](https://learn.microsoft.com/en-us/uwp/api/windows.devices.power.batteryreport.chargerateinmilliwatts)
- **Shortcuts:** [Keyboard shortcuts in Windows (Microsoft)](https://support.microsoft.com/en-us/accessibility/windows/keyboard-shortcuts-in-windows)
- **Menu bar monitors:** [iStat Menus](https://bjango.com/mac/istatmenus/), [Stats](https://github.com/exelban/stats)
