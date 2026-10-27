# PRD: Devices (Bluetooth, batteries, charging, drives)

Status: draft for review · Owner: Rohit · Feature #6 in [PLAN.md](../PLAN.md) · Oct 28, 2026

**References:**
- **iPhone Dynamic Island:** AirPods connected (device image + battery for buds and case), charging (bolt + %, green or red at ≤ 20 %), low battery at **20 % and 10 %** as a brief non-modal alert.
- **macOS notch apps:** the AirPods "reveal" and the charging animation are the headline features in Alcove, NotchNook and boring.notch. A mouse/keyboard battery list is the differentiator (Atoll, Notchy, Seam).
- **Windows:**
  - Settings and Quick Settings show Bluetooth battery %, but there's **no connect or low-battery moment**, which is why Bluetooth Battery Monitor, BlueGauge (Rust, MIT) and the Store indicators exist.
  - There's no charging animation (only a "charging slowly over USB" toast).
  - Vendor apps (Logitech G Hub, Razer, Galaxy Buds, Sony) use private protocols.

**Inputs:** a platform and competitor study (sources in the appendix) and ReUI's code after the reorganisation. Reusable already:
- `osd/os.rs`: a hidden window with power-setting notifications and Raw Input;
- `audio/events.rs`: the default-device events, so the Devices card can absorb the OSD's device moment;
- the Brief priority and bubble placement;
- `Element::Ring` and `Element::Level`;
- the settings and tray patterns.

## 1. Problem

Windows tells you nothing when your earbuds connect or your mouse is about to die. You find out from Settings, or when it stops working. Plugging in a laptop shows only a tray icon. A USB drive gets a generic notification and a hidden eject. These are exactly the small, glanceable moments the island exists for.

## 2. Goals

1. **Know at a glance.** A device connecting shows what it is and its battery. Earbuds show left, right and case when possible.
2. **Never surprised by a dead battery.** Peripherals and the laptop warn at 20 % and 10 %, once each per discharge.
3. **Charging feels good.** Plugging in shows a bolt and the % (and later the time to full); unplugging is quiet.
4. **Drives are one click from safe.** A drive plugged in shows its name and space, with Eject right there. If ejecting is blocked, ReUI says by what.
5. **Silent otherwise.** No spam at boot, resume, flapping connections, or docks full of devices. 0 % idle CPU and no radio cost (no Bluetooth connections of our own).

## 3. Non-goals (v1)

- Vendor protocols: Logitech HID++, Razer, Galaxy Buds and Sony RFCOMM.
- Pairing new devices, or controlling device features (ANC, EQ).
- Printers, Windows Hello, security keys, displays. Mic/camera in use is feature #9.
- Phones beyond "connected" (no file transfer).

## 4. Scenarios

| # | Scenario | v1 |
|---|---|---|
| S1 | Earbuds connect | Brief card: device glyph, name, battery ring(s) (L, R, case for AirPods; one ring otherwise); 3 s, held while hovered |
| S2 | Mouse at 20 %, later 10 % | Each once: glyph + "Mouse 20 %" in red-tinted ring; never while it charges |
| S3 | Laptop plugged in | Bolt + %; green, or red at ≤ 20 %; expanded shows time to full when stable |
| S4 | Laptop at 20 % on battery | Same low-battery moment as peripherals |
| S5 | USB drive plugged in | Drive glyph, label, free/total; Eject in the expanded card (and on click) |
| S6 | Eject blocked | "In use by Explorer" (the blocking app's name), the drive stays |
| S7 | Resume from sleep, three devices reconnect | Nothing (they were already connected) |
| S8 | Headphones connect and become the default output | One card (Devices), not also the OSD's "Headphones" moment |
| S9 | Want an overview | Expanded card: connected battery devices sorted by lowest battery, laptop battery, drives with Eject |
| S10 | Desktop PC (no battery) | No charging feature; everything else works |

## 5. Experience

### 5.1 Presentations

- **Connect card (Brief, key `device:<id>`):**
  - **Compact:** leading device glyph (headphones, earbuds, headset, speaker, mouse, keyboard, gamepad, phone, watch, generic Bluetooth); trailing `Element::Ring(battery)` when known, nothing when unknown.
  - **Minimal:** the ring (or the glyph).
  - **Expanded:** headline = the device name; rows = battery levels (`Row::Progress` per part for earbuds: L / R / Case, as quiet captions), a "Disconnect" button? (*D5*).
  - The battery often arrives 1–5 s after the connect. The card is **upgraded in place** when it does; there's never a placeholder %.
- **Low battery (Brief, key `low:<id>`):** glyph + ring tinted red at ≤ 20 %. Shown once per threshold per discharge, and re-armed after recovering 5 % above it or after charging.
- **Charging (Brief, key `power`):** compact bolt + `Level` (green, or red ≤ 20 %) + "%" text; minimal ring. Expanded: % and, once stable, "Full in 1 h 10 min" (from `AggregateBattery`: (full − remaining) / rate, smoothed, hidden until stable and near 100 %). Unplugged: nothing (*D3*).
- **Drive (Brief, key `drive:<letter>`):** compact drive glyph + label. Expanded: "12.4 GB free of 64 GB", an **Eject** button (Accent), and, if blocked, a quiet "In use by …" line.
- **Devices overview (Passive, key `devices`, *D2*):** the full list (S9), opened from the tray ("Devices") or by clicking any device moment.

### 5.2 Anti-spam rules

- **Startup and resume grace:** no connect cards for 15 s after the hub starts, resume (`PBT_APMRESUMEAUTOMATIC`) or unlock. Devices already connected then are the baseline.
- **Flapping:** a disconnect followed by a reconnect within 10 s is nothing. There's a 60 s cooldown per device between connect cards.
- **Several at once:** connects within 2 s merge into one card ("3 devices connected", glyphs side by side in expanded).
- **Docks:** drive arrivals within 2 s merge. Non-removable volumes, VHD/ISO mounts (*D6*) and system volumes are ignored.
- **Disconnects:** silent (iOS does the same). The device just leaves the overview.
- **OSD hand-off:** when a Bluetooth audio device connects *and* becomes the default output, the OSD's device-switch moment is skipped (S8).

### 5.3 Privacy

- No device names, MAC addresses or instance IDs in logs (kind + a per-session hash only).
- Advertisements are never persisted.
- The AirPods case/L/R scan is **time-boxed** (≤ 20 s after an Apple audio device connects, or while the overview is open) and can be turned off (*D4*). It observes nearby devices' broadcasts and reads only the matching model's.

## 6. Detection

| Signal | How | Cost |
|---|---|---|
| Bluetooth connect/disconnect | `DeviceWatcher` on paired devices (Classic + LE selectors, `AssociationEndpoint`, property `System.Devices.Aep.IsConnected`); connects arrive as `Updated`. Dedupe dual-mode devices by `ContainerId` | Near-free (cache-backed, no scanning) |
| Bluetooth battery | The PnP property Windows shows in Settings, `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2`, read via `CM_Get_DevNode_PropertyW` on the device's service nodes (HFP for Classic, BTHLE for LE) by `ContainerId`; minimum if several. Re-read 3 s after connect, then every 60 s **only while a battery device is connected**, and when the overview opens | One cfgmgr call per minute while connected |
| AirPods L/R/case | `BluetoothLEAdvertisementWatcher` filtered on Apple manufacturer data (0x004C, type 0x07); nibbles for pods and case, primary-pod flip, charging flags; match by model + RSSI | Short bursts only |
| Controllers | `Windows.Gaming.Input` `GamepadAdded`/`Removed` + `TryGetBatteryReport` (coarse 10/40/70/100), polled 60 s while connected | Same as above |
| Device kind | Class of Device (Classic) / Appearance (LE) → glyph | — |
| Laptop power | `GUID_ACDC_POWER_SOURCE`, `GUID_BATTERY_PERCENTAGE_REMAINING` (the OSD window already receives power settings) + `AggregateBattery.ReportUpdated` for rate and capacity; `GetSystemPowerStatus` `BatteryFlag 128` = no battery | Free |
| Slow charger | `PowerManager.PowerSupplyStatusChanged` → `Inadequate` (respect Windows' own "charging slowly" toggle, *D7*) | Free |
| Drives | `RegisterDeviceNotification` `GUID_DEVINTERFACE_VOLUME` (not broadcasts: our windows are tool/message-only) → `GetVolumePathNamesForVolumeName`, `GetDriveTypeW == DRIVE_REMOVABLE`, label and space via `GetVolumeInformationW`/`GetDiskFreeSpaceExW` | Free |
| Eject | volume → disk → devinst, walk `CM_Get_Parent` to the first `DN_REMOVABLE` ancestor, `CM_Request_Device_EjectW` **with a veto buffer** (no system popup; tells us who blocks) | On demand |
| Phones | `GUID_DEVINTERFACE_WPD` arrival: "iPhone connected" / "Phone connected", no actions | Free |

**Engineering:**
- A new `modules/devices/`:
  - `mod.rs` (module loop, anti-spam state);
  - `bluetooth.rs` (watcher, battery reads, kinds);
  - `airpods.rs` (advertisement parsing; pure and unit-tested);
  - `power.rs`;
  - `drives.rs` (arrival, info, eject);
  - `view.rs`;
  - `settings.rs`.
- **Shared with the OSD:** the hidden power/Raw Input window (`osd/os.rs`) moves to a small shared `modules/system_events.rs` that both subscribe to, rather than a second window.

## 7. Edge cases

| ID | Case | Required behaviour |
|---|---|---|
| D-1 | Battery unknown at connect | Card without a ring, upgraded if the value arrives within 5 s |
| D-2 | Battery value stale after disconnect | Ignore values from disconnected devices |
| D-3 | Device reports battery only while connected for a while (HFP) | Re-read at 3 s, 10 s, then every minute |
| D-4 | Dual-mode device (AirPods, many headsets) | One device by ContainerId |
| D-5 | LE device without a battery service removed (BlueGauge 0.3.1 crash) | Handle missing nodes gracefully |
| D-6 | AirPods 4 (no 0x07 ads) / spoofed ads | Fall back to the PnP battery; accept ads only from the connected model, RSSI-near |
| D-7 | Controller coarse levels | Show the step (10/40/70/100), low alert at the 10 step |
| D-8 | Charging rate fluctuates / tapers near 100 % | Smooth over 2 min; hide the time-to-full when unstable or above 95 % |
| D-9 | UPS (`PoHot`) | "On UPS power" moment, quiet |
| D-10 | Card reader with empty slots | Only volumes with media; eject the reader's removable ancestor |
| D-11 | "Removable" SSD that vetoes | Show the veto reason; offer nothing destructive |
| D-12 | Drive pulled without eject | Silent removal from the overview |
| D-13 | Many devices at boot | Grace period; nothing shown |
| D-14 | Fullscreen game | Nothing shown except low battery ≤ 10 % (*D8*) |

## 8. Performance budget

- **Idle:** 0 % CPU; no Bluetooth connections opened by ReUI.
- **While a battery device is connected:** one property read per minute.
- **AirPods scan:** ≤ 20 s bursts.
- **Memory:** < 2 MB for the module.

## 9. Settings (`%APPDATA%\ReUI\devices.json`, tray)

| Setting | Default |
|---|---|
| Connect cards | on |
| Low battery alerts (20 %, 10 %) | on |
| Charging moment | on |
| Drives | on |
| AirPods detailed batteries (BLE scan) | on, time-boxed (*D4*) |
| Phones | on |
| Slow-charger moment | off (Windows has its own) |

## 10. Milestones

| M | Scope | Exit |
|---|---|---|
| **M0: Bluetooth connect + battery** | Watcher, kinds/glyphs, PnP battery, connect card upgraded in place, anti-spam (grace, flap, merge), OSD hand-off | S1, S7, S8 with real earbuds and a mouse |
| **M1: Low battery** | Thresholds, re-arm, charging exclusion, controllers | S2 |
| **M2: Laptop power** | Plug-in moment, time to full, low laptop battery, UPS, desktop detection, shared system-events window | S3, S4, S10 |
| **M3: Drives & phones** | Arrival, info, eject with veto reason, merge, phones | S5, S6 |
| **M4: AirPods detail + overview** | Advertisement parsing (L/R/case), the overview card, tray entry | S9; AirPods show three batteries |

## 11. Acceptance test plan

- **Unit tests:**
  - AirPods payload parsing (recorded payloads, primary flip, N/A nibbles);
  - glyph mapping;
  - anti-spam state machine (grace, flap, cooldown, merge);
  - low-battery thresholds and re-arm;
  - time-to-full smoothing;
  - eject ancestor walk (on a fake tree).
- **Manual matrix:**
  - Bluetooth earbuds (AirPods if available), a BT mouse and keyboard, an Xbox controller, a USB drive, a card reader;
  - an iPhone and an Android phone over USB;
  - sleep/resume with devices connected;
  - a laptop (charging, low battery) — this PC is a desktop, so it needs a laptop.

## 12. Open decisions

| ID | Question | Proposal |
|---|---|---|
| D1 | Connect cards for every Bluetooth device, or only audio + input devices? | Audio, input (mouse/keyboard/controller), phones and watches; others only in the overview |
| D2 | A Devices overview card in v1? | Yes (M4), opened from the tray or by clicking a device moment |
| D3 | Unplug moment? | No (quiet, like iOS) |
| D4 | AirPods detail via a BLE scan of nearby broadcasts | On, but only in ≤ 20 s bursts after an Apple device connects or while the overview is open; setting to turn off |
| D5 | Disconnect button for Bluetooth devices | Not in v1 (needs the Bluetooth stack's disconnect, flaky); Settings link instead |
| D6 | Count mounted ISO/VHD as drives? | No |
| D7 | Slow-charger moment | Off by default (Windows has its own toast) |
| D8 | In fullscreen | Only critical (≤ 10 %) low-battery alerts show |

## Appendix: sources

- **Apple:** [MacRumors: low battery in the Dynamic Island](https://www.macrumors.com/2022/09/14/iphone-14-pro-low-battery-alerts-dynamic-island/), [Apple: MagSafe charging](https://support.apple.com/guide/iphone/magsafe-chargers-and-battery-packs-iphac0f12e49/ios)
- **Microsoft:**
  - [GetDeviceSelectorFromPairingState](https://learn.microsoft.com/en-us/uwp/api/windows.devices.bluetooth.bluetoothdevice.getdeviceselectorfrompairingstate)
  - [GATT client](https://learn.microsoft.com/en-us/windows/apps/develop/devices-sensors/gatt-client)
  - [BluetoothClassOfDevice](https://learn.microsoft.com/en-us/uwp/api/windows.devices.bluetooth.bluetoothclassofdevice)
  - [Power setting GUIDs](https://learn.microsoft.com/en-us/windows/win32/power/power-setting-guids)
  - [Battery](https://learn.microsoft.com/en-us/uwp/api/windows.devices.power.battery)
  - [PowerManager](https://learn.microsoft.com/en-us/uwp/api/windows.system.power.powermanager)
  - [RegisterDeviceNotification](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerdevicenotificationa)
  - [CM_Request_Device_EjectW](https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_request_device_ejectw)
  - [Gamepad.TryGetBatteryReport](https://learn.microsoft.com/en-us/uwp/api/windows.gaming.input.gamepad.trygetbatteryreport)
- **Projects:**
  - [BlueGauge](https://github.com/iKineticate/BlueGauge)
  - [Bluetooth-Battery-Tray](https://github.com/MaksimHO/Bluetooth-Battery-Tray)
  - [Bluetooth Battery Monitor changelog](https://www.bluetoothgoodies.com/battery-monitor/version-history/)
  - AirPods advertisements: [proximity protocol notes](https://github.com/fischejo/airpods-notify/blob/master/doc/proximity_protocol.md), [LibrePods](https://deepwiki.com/kavishdevar/librepods/2.3-ble-advertisements), [PETS 2020 privacy study](https://petsymposium.org/popets/2020/popets-2020-0003.pdf)
  - [notch apps comparison](https://notchy.dev/best-mac-notch-apps/)
