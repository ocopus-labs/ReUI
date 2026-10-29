# Research: personalised device moments (exact model, product art, per-part batteries, colour)

Status: research, not a PRD · Oct 30, 2026 · Feeds a later revision of [PRD-devices.md](PRD-devices.md)

**Question:** When a Bluetooth or other device connects, how can ReUI show what it really is, like the iPhone's AirPods card or Android's Fast Pair half-sheet? That means the exact model name, an image that looks like the product, the right colour, and per-bud and case batteries. ReUI is a native, unpackaged Win32/WinRT exe shipped from GitHub, and the island is black and quiet (DESIGN.md).

**Labels used below:**
- **[Doc]** documented by the vendor or the spec.
- **[RE]** reverse-engineered by the community; can change without notice.
- **[Obs]** observed on this PC (Windows 11 Pro 26200) while writing this.
- **[Unk]** not established.

---

## TL;DR

1. **Windows has an exact-model signal that's free and offline, and ReUI doesn't read it yet.** It's the Bluetooth **Device ID profile** VID/PID that Windows stores on Classic devnodes. It's exposed as `DEVPKEY_Bluetooth_DeviceVID/PID/VIDSource` and in hardware IDs like `BTHENUM\{0000110b-…}_VID&0001004C_PID&2014` [Doc/Obs].
   - For Apple, `0x004C` plus PID `0x20xx` is the exact model with no BLE scan.
   - It's only as good as the vendor's VID. The "Buds 3r" on this PC reports VID `0x0642`, which is **Bluetrum**, the chip maker, not the brand [Obs].
2. **Windows' own pictures are generic.**
   - `DeviceInformation.GetThumbnailAsync` works from an unpackaged process [Obs]. For Bluetooth audio it returns the Windows 7-era DDORes headset icon (`DDORes.dll,-2033`), and the container's model name is "Bluetooth Peripheral Device" [Obs].
   - Device metadata (WMIS, the source of the old photo-realistic icons) was **retired in May 2025** [Doc].
   - Its replacement, INF `ContainerIcon`, only appears if an OEM ships a driver package (Windows 11 24H2+) [Doc]. Most earbuds use inbox drivers.
3. **Swift Pair carries no image and no model**, only a name and a CoD/Appearance, and only while in pairing mode [Doc].
4. **Apple's proximity-pairing advert gives model, colour, L/R/case battery, charging, lid and in-ear in the clear** [RE]. ReUI already parses most of it. The colour byte and a dozen newer model IDs are missing, and the open-source references are all GPL/AGPL.
5. **Google Fast Pair images can't legally be used.**
   - The model ID is documented, but it's only broadcast in pairing mode. After pairing, the advert carries an account-key filter plus optional L/R/case battery [Doc].
   - The metadata endpoint (`nearbydevices-pa.googleapis.com/v1/device/{id}`) is a **private** Google API that needs a Google-issued key. Using Chrome's or GMS's key would break Google's API terms, and the images carry no licence for third-party display.
6. **Vendor press images are off-limits for bundling.**
   - Apple requires written permission and an actual photograph, not a rendering.
   - Samsung's newsroom is personal and non-commercial use only.
   - Images inside GPL projects (e.g. AirPodsDesktop's PNGs) don't come with clear rights.
7. **Recommendation:** resolve identity in layers, offline:
   - order: Device ID VID/PID → vendor advert (Apple, Samsung, Fast Pair) → non-generic Windows container icon → name heuristic → CoD/Appearance glyph;
   - draw **ReUI's own single-colour line-art silhouettes per model family** (crisp at 24 and 56 DIP, theme-proof, through the existing `Art::Logo` path);
   - let the user drop in their own photo;
   - no device data leaves the PC.

---

## 1. What Windows itself knows

### 1.1 Container and devnode properties

ReUI already groups endpoints by `ContainerId`. Here is what the container and its devnodes hold for real devices on this PC.

| Source | Property | "Buds 3r" (BT earbuds) | "100 wireless" (BT headphones) | Notes |
|---|---|---|---|---|
| Container | `System.Devices.ModelName` | `Bluetooth Peripheral Device` | `Bluetooth Peripheral Device` | Generic for inbox-driver Bluetooth [Obs] |
| Container | `System.Devices.Icon` | `DDORes.dll,-2033` | `DDORes.dll,-2033` | The generic headset bitmap [Obs] |
| Container | `System.Devices.GlyphIcon` | `DDORes.dll,-3021` | `DDORes.dll,-3021` | Monochrome glyph [Obs] |
| Container | Category | `Communication.Headset.Bluetooth`, Audio, Input | — | From `pnputil /enum-containers /properties` [Obs] |
| Devnode | `DEVPKEY_Device_Manufacturer` | `Microsoft` | `Microsoft` | The **driver's** maker, useless [Obs] |
| Devnode | `DEVPKEY_Bluetooth_DeviceVIDSource` / `VID` / `PID` | 1 / `0x0642` / `0x0220` | absent | Device ID profile; not every device has it [Obs] |
| Devnode | `DEVPKEY_Bluetooth_ClassOfDevice` | `0x240404` | `0x260404` | Already used through the AEP properties |

**The Device ID keys:**
- The keys are `DEVPKEY_Bluetooth_DeviceManufacturer` (4), `ModelNumber` (5), `VIDSource` (6, BYTE), `VID` (7, UINT16), `PID` (8, UINT16) and `ProductVersion` (9) in `bthguid.h`, all on fmtid `{2BD67D8B-8BEB-48D5-87E0-6CDA3428040A}` ([SDK header copy](https://github.com/tpn/winsdk-10/blob/master/Include/10.0.16299.0/km/bthguid.h)).
- The shell-property doc for `VendorIdSource` says 1 = Bluetooth SIG company ID and 2 = USB-IF VID ([Doc](https://learn.microsoft.com/en-us/windows/win32/properties/props-system-deviceinterface-bluetooth-vendoridsource)).
- ReUI's `pnp.rs` already reads devnode properties with `CM_Get_DevNode_PropertyW`, so this is one more key on the same walk.

**Which devices have it:**
- The same VID/PID appears in Classic hardware IDs, e.g. `BTHENUM\{0000110b-0000-1000-8000-00805f9b34fb}_VID&00010642_PID&0220` [Obs].
- Real AirPods show `_VID&0001004C_PID&200E` (Pro 1) and `_PID&2014` (Pro 2) in public driver listings ([driveridentifier](https://www.driveridentifier.com/scan/airpods-pro-stereo-driver/download/574487549/82F37B7FC63740BA8A53C71561F63F5E/BTHENUM/%7B0000110b-0000-1000-8000-00805f9b34fb%7D_LOCALMFG&005d), [NTPods issue](https://github.com/arctumn/ntpods/issues/11)).
- An October 2026 Windows tool maps 31 AirPods/Beats PIDs this way ([BatteryInfo PR #36](https://github.com/whisperers26/BatteryInfo/pull/36)).
- LE HID devices (mice, keyboards) usually carry a USB-IF VID/PID from the GATT PnP ID characteristic in their `BTHLEDevice` hardware IDs [Unk on this PC; verify with a Logitech/Microsoft mouse].

**Caveat [Obs]:** VID `0x0642` resolves to *Bluetrum Technology* in the SIG company list ([Nordic DB](https://github.com/NordicSemiconductor/bluetooth-numbers-database)). White-label buds report their **chip vendor** (Bluetrum, Airoha, Realtek…), so VID/PID is an exact-model key only for brands with their own SIG ID:
- Apple `0x004C`;
- Microsoft `0x0006`;
- Samsung `0x0075`;
- Sony `0x012D`;
- Bose `0x009E`;
- and others.

**Reading container properties unpackaged:**
- Container objects can't be reached through SetupDi; use `Windows.Devices.Enumeration` with `DeviceInformationKind.DeviceContainer` or DevQuery ([Microsoft Q&A](https://learn.microsoft.com/en-us/answers/questions/197894/how-to-get-friendlyname-of-a-device-containerid-(s)).
- The probe for this document did exactly that from Windows PowerShell 5.1, unpackaged, with no capabilities [Obs].
- AEP containers also expose `System.Devices.AepContainer.ModelName` ([Doc](https://learn.microsoft.com/en-us/windows/win32/properties/props-system-devices-aepcontainer-modelname)).

### 1.2 `GetThumbnailAsync` / `GetGlyphThumbnailAsync`

- **The API [Doc]:** both return a `DeviceThumbnail` stream ([GetThumbnailAsync](https://learn.microsoft.com/en-us/uwp/api/windows.devices.enumeration.deviceinformation.getthumbnailasync), [DeviceThumbnail](https://learn.microsoft.com/en-us/uwp/api/windows.devices.enumeration.devicethumbnail)). Microsoft's docs team describes them as "the built-in bitmap image" and "the built-in vector image" for the device ([winrt-api #1660](https://github.com/MicrosoftDocs/winrt-api/issues/1660)).
- **On this PC [Obs]:**
  - `GetThumbnailAsync` returned a 256×256 PNG for every container: the glossy Windows 7 Bluetooth headset for both earbuds and headphones, a generic phone, a generic monitor.
  - `GetGlyphThumbnailAsync` returned a small monochrome PNG (≈ 0.3–0.8 KB).
  - Both work unpackaged.
- **Conclusion:** the thumbnail is only as good as the container icon. It's worth calling, because a device with an OEM `ContainerIcon` or a legacy metadata package gets its real photo for free. Treat any icon whose `System.Devices.Icon` points into `DDORes.dll` as generic and ignore it, since ReUI's own glyphs and silhouettes look better on black.

### 1.3 Device metadata (Device Stage, WMIS) and its replacement

- **Retired [Doc]:** "Device metadata is deprecated and will be removed in a future release of Windows" ([WMIS page](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/windows-metadata-and-internet-services)).
  - The Hardware Dev Center post of May 5, 2025 ("End of Servicing and Retirement Plan for Device Metadata on Windows") says WMIS is retired and "no new packages will be delivered to Windows clients via WMIS". Packages already on a PC stay ([ElevenForum repost](https://www.elevenforum.com/t/end-of-servicing-and-retirement-plan-for-device-metadata-on-windows.35831/), [Neowin](https://www.neowin.net/news/microsoft-bringing-two-big-changes-to-hardware-drivers-for-next-gen-windows-11/)).
  - So **no**, product images are no longer served.
- **On this PC [Obs]:**
  - `%ProgramData%\Microsoft\Windows\DeviceMetadataStore\en-US` holds two old packages: Nokia USB phones and Brother printers. Each is a CAB with `PackageInfo.xml`, `DeviceInfo.xml` (`ModelName`, `Manufacturer`, `DeviceIconFile`) and a `device.ico`.
  - `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Device Metadata\PreventDeviceMetadataFromNetwork = 1`.
- **Replacement [Doc]:** [Driver Package Container Metadata](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/driver-package-container-metadata).
  - An OEM INF's `AddProperty` sets `ContainerModelName`, `ContainerManufacturer`, `ContainerCategories` and `ContainerIcon` ("a photo-realistic icon").
  - Requires Windows 11 24H2 KB5052093 (26100.3323) or later.
  - It lands on the container, so ReUI reads it through 1.1/1.2 with no extra work. Few Bluetooth audio vendors ship INFs [Unk how many do].

### 1.4 The images in Settings › Bluetooth & devices

- **[Unk]** No Microsoft documentation describes a separate product-image source for the Settings device list. Searches found nothing that ties Surface, Xbox or Swift Pair device photos to a cloud image service.
- **[Obs]** Every Bluetooth device on this PC shows the same container icon that `GetThumbnailAsync` returns.
- The working assumption is that Settings shows the container icon, so whatever Settings can show, ReUI can show through `GetThumbnailAsync`. That icon comes from DDORes, a legacy metadata package or an OEM `ContainerIcon`. First-party devices (Surface, Xbox) may get photos from Microsoft's own driver or metadata packages [Unk; verify with a Surface or Xbox accessory].
- The new Insider "Battery Status" widget (Oct 2026) shows **type icons**, not product images ([report](https://windowsforum.com/news/windows-11-battery-status-widget-insider-access-setup-and-peripheral-compatibility.447565/)).

### 1.5 Swift Pair

**Payload [Doc]** ([Swift Pair](https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/bluetooth-swift-pair)):
- Microsoft vendor section `0x0006`, Beacon ID, Sub-scenario (0x00 LE, 0x01 BR/EDR, 0x02 both), reserved RSSI `0x80`;
- for dual mode, a 3-byte CoD and a little-endian BR/EDR address;
- an optional, unlocalised **Display Name**.

**What Windows shows:** the icon is derived from the CoD or LE Appearance ("the same as the icon shown in Settings"), and the name comes from the friendly-name section.

**No image, model ID or colour.** The advert is also only present in pairing mode, so it says nothing at connect time.

### 1.6 What an unpackaged Win32 app can read

| Signal | API | Unpackaged? |
|---|---|---|
| Container props, AEP props, thumbnails | `Windows.Devices.Enumeration` | Yes [Obs] |
| Devnode props (VID/PID, battery) | `CM_Get_DevNode_PropertyW` | Yes (ReUI does it today) |
| BLE adverts (Apple, Fast Pair, Samsung) | `BluetoothLEAdvertisementWatcher` | Yes (ReUI's `airpods.rs` does it today) |
| GATT reads (Fast Pair Model ID, DIS) | `BluetoothLEDevice` / GATT | Yes, but it opens a connection (against the PRD's "no connections of our own") |
| RFCOMM vendor protocols (Galaxy Buds, Sony) | Winsock `AF_BTH` / `RfcommDeviceService` | Yes, but it's a connection and can fight the vendor app for the channel |
| Controllers | `Windows.Gaming.Input.RawGameController` (`HardwareVendorId/ProductId`, `DisplayName`) | Yes |

---

## 2. Vendor protocols that identify the exact model and colour

### 2.1 Apple: Continuity "Proximity Pairing" (type 0x07) [RE]

ReUI's `airpods.rs` indexes the manufacturer data after the company ID: `[0]=0x07 [1]=len [2]=0x01`. The fields are:

| Byte | Field | ReUI today |
|---|---|---|
| 3–4 | Model, stored little-endian; Apple's PID is `0x20xx` (bytes `0E 20` = `0x200E`) | Read big-endian as `0x0E20`; same thing |
| 5 | Status: 0x20 = left bud primary (else the bud nibbles and charge flags swap); other bits = in-ear and in-case | Primary flip only |
| 6 | Buds: one nibble per bud, 0–10 ×10 %, 15 = n/a | Yes |
| 7 | High nibble charging flags (L/R/case), low nibble case level | Yes |
| 8 | Lid state; bit 3 open/closed, reliable only from a pod inside the case | No |
| 9 | **Colour** | No |
| 10 | Suffix | — |
| 11+ | Encrypted tail (exact 1 % levels); key only obtainable over AAP | Not read (correct) |

**Sources:** CAPod's `ApplePods.kt` (public payload: `data[1..2]` model, `[3]` status, `[4]` pods, `[5]` flags/case, `[6]` lid, `[7]` colour, offset by the prefix byte) ([CAPod](https://github.com/d4rken-org/capod)), [LibrePods notes](https://deepwiki.com/kavishdevar/librepods/2.3-ble-advertisements), [furiousMAC/continuity](https://github.com/furiousMAC/continuity) (GPL-2.0 Wireshark dissector), [airpods-notify protocol notes](https://github.com/fischejo/airpods-notify/blob/master/doc/proximity_protocol.md).

**Colour codes** (CAPod `HasAppleColor.DeviceColor`):

| Code | Colour | Code | Colour |
|---|---|---|---|
| 00 | white | 07 | gold |
| 01 | black | 08 | rose gold |
| 02 | red | 09 | space grey |
| 03 | blue | 0A | dark blue |
| 04 | pink | 0B | light blue |
| 05 | grey | 0C | yellow |
| 06 | silver | | |

How these map to marketing names (e.g. AirPods Max "Midnight", Beats colourways) is [Unk] per model.

**Model IDs** (CAPod factories, ReUI's byte order):

| Family | IDs |
|---|---|
| AirPods | Gen 1 `0x0220`, Gen 2 `0x0F20`, Gen 3 `0x1320`, Gen 4 `0x1920`, Gen 4 ANC `0x1B20`, "Gen 5" `0x3620`, "Gen 5 Wireless" `0x3020` |
| AirPods Pro | `0x0E20`, Pro 2 `0x1420`, Pro 2 USB-C `0x2420`, Pro 3 `0x2720` |
| AirPods Max | `0x0A20`, USB-C `0x1F20`, Max 2 `0x2D20` |
| Beats | Solo 3 `0x0620`, Solo 4 `0x2520`, Solo Buds `0x2620`, Solo Pro `0x0C20`, Studio Buds `0x1120`, Studio Buds+ `0x1620`, Studio Pro `0x1720`, Fit Pro `0x1220`, Flex `0x1020`, X `0x0520`, Powerbeats 3 `0x0320`, Powerbeats 4 `0x0D20`, Powerbeats Pro `0x0B20`, Powerbeats Pro 2 `0x1D20`, Beats "360" `0x3820` |

Community tables disagree:
- an AirPodsDesktop issue says AirPods 5 report PID `0x2030` ([#240](https://github.com/SpriteOvO/AirPodsDesktop/issues/240));
- another project labels `0x2030` "Pro 2 USB-C" and `0x2024` "AirPods 3 MagSafe";
- BatteryInfo's owner reports AirPods 4 ANC as `0x201B`, which matches CAPod's `0x1B20`;
- [The Apple Wiki "Bluetooth PIDs"](https://theapplewiki.com/wiki/Bluetooth_PIDs) is the best single reference.

So the catalog must be data, not code, and must be checked against real hardware.

**Gaps in ReUI today:**
- `family_of_model` misses `0x2D20`, `0x3620`, `0x3020`, `0x2520`, `0x2620`, `0x3820` and `0x0D20`.
- PRD D-6 says AirPods 4 send no 0x07 ads, but CAPod and BatteryInfo both decode AirPods 4 from them. Re-test.
- Since the connected device's PID is known from §1.1, the advert matcher can require the **exact** model, not just the family, which hardens it against spoofed adverts (the `spam.rs` concern).

**Licences:**
- CAPod GPL-3.0, LibrePods GPL-3.0, AirPodsDesktop GPL-3.0, OpenPods GPL-3.0, AirBattery AGPL-3.0, kAirPods GPL-3.0, furiousMAC GPL-2.0.
- ReUI has **no LICENSE file** yet.
- Byte offsets and ID tables are facts, so reimplement from the documented layout (as `airpods.rs` already does). Don't copy source files or assets.

### 2.2 Google Fast Pair [Doc + private API]

**Discoverable (pairing mode only)** [Doc] ([Provider spec](https://developers.google.com/nearby/fast-pair/specifications/service/provider)):
- Service Data UUID `0xFE2C` = 24-bit **Model ID**, advertised at 100 ms or faster with a stable address.
- It's also readable over GATT from characteristic `FE2C1233-8366-4814-8EB0-01DE32100BEA`, which needs an LE connection.

**Not discoverable (after pairing):** flags, the account-key Bloom filter and a salt, with **no model ID**. Optionally a battery block follows ([Battery notification](https://developers.google.com/nearby/fast-pair/specifications/extensions/batterynotification)):
- length/type `0bLLLLTTTT`, with T = 0011 "show UI" (case opened) or 0100 "hide UI";
- then left, right and case bytes `0bSVVVVVVV` (S = charging, V = 0–100, `0x7F` = unknown); example `0x33 0xD0 0x4B 0x7F`.

The battery bytes are in the clear, so a passive scan can read them. Two limits:
- the advert is only present when the buds have account keys (paired to an Android with a Google account);
- the spec says providers "should not include raw battery data in the advertisement all the time".

Matching an advert to *our* connected device without the account key is the hard part: use RSSI, timing and the connect event [Unk reliability].

**Metadata (name, images, type)** [RE]:
- Chromium OS fetches `https://nearbydevices-pa.googleapis.com/v1/device/<model id>?key=<Chrome's API key>&mode=MODE_RELEASE&alt=proto`. Its traffic annotation reads "Details about a bluetooth peripheral, including name and picture" ([Chromium source mirror](https://github.com/nwjs/chromium.src/blob/HEAD/ash/quick_pair/repository/fast_pair/device_metadata_fetcher.cc)).
- AOSP's hidden `FastPairDeviceMetadata` has `imageUrl` (512×512 PNG), `trueWirelessImageUrlLeftBud`, `…RightBud` and `…Case`, plus a device type ([AOSP](https://android.googlesource.com/platform/prebuilts/fullsdk/sources/+/refs/heads/androidx-appcompat-release/android-34/android/nearby/FastPairDeviceMetadata.java)).
- A KU Leuven research toolkit calls the gRPC `GetObservedDevice` by **impersonating Google Play services** (GMS package name, certificate and API key) ([WhisperPair](https://github.com/KULeuven-COSIC/WhisperPair/blob/main/toolkit-server/src/model-id-resolver/index.ts)).

**Verdict:** the endpoint is private (`-pa`), with no public key or documentation. Google's [API terms](https://developers.google.com/terms) rule it out for ReUI:
- §4(b)(1): credentials identify *your* client and "may not be embedded in open source projects";
- §2(c): "You will not misrepresent or mask either your identity or your API Client's identity";
- §5(a)/(e): content belongs to its owners; no building databases or permanent copies.

The OEM images carry no licence for third-party display. **Not usable.** Model-ID name lists exist ([DiamondRoPlayz/FastPair-Models](https://github.com/DiamondRoPlayz/FastPair-Models), no licence; its author stopped publishing "due to legal reasons").

### 2.3 Samsung Galaxy Buds

- **Adverts [RE]:**
  - Samsung "EasySetup" manufacturer data (company `0x0075`) carries a 3-byte ID that encodes **model and colour**, e.g. `0x39EA48` "Light Purple Buds2", `0x850116` "Black Buds Live".
  - Watches use a 1-byte ID.
  - The tables come from BLE-spam research ([Momentum `easysetup.c`](https://github.com/Next-Flip/Momentum-Apps/blob/dev/ble_spam/protocols/easysetup.c), [Bluetooth-LE-Spam](https://github.com/simondankelmann/Bluetooth-LE-Spam)).
  - When paired buds broadcast it (case open?) is [Unk].
- **RFCOMM [RE]:** Galaxy Buds Client (GPL-3.0, runs on Windows) gets L/R/case batteries and the **device colour** from the buds' extended-status message ([GalaxyBudsClient](https://github.com/ThePBone/GalaxyBudsClient)). That needs our own SPP connection.
- **Official:** Samsung's "Galaxy Buds" Windows app (Store 9NHTLWTKFZNB) shows L/R/case ([Store](https://apps.microsoft.com/detail/9nhtlwtkfznb)). No public API.
- **VID/PID:** SIG `0x0075` if the buds publish Device ID [Unk; verify].

### 2.4 Sony, Bose, Jabra, Nothing, Microsoft Surface

| Brand | Model | Batteries | How | Status |
|---|---|---|---|---|
| Sony | Device ID VID `0x012D` (if present); Fast Pair on recent models | L/R/case over the proprietary RFCOMM "MDR" protocol | [SonyHeadphonesClient](https://github.com/mos9527/SonyHeadphonesClient) (MIT), [Gadgetbridge](https://codeberg.org/Freeyourgadget/Gadgetbridge) (AGPL-3.0) | [RE] |
| Bose | Fast Pair on newer models; Device ID | RFCOMM (older QC35-era) | [based-connect](https://github.com/Denton-L/based-connect) (GPL-3.0, stale) | [RE] |
| Jabra | Name, serial, PID via SDK | Telemetry and properties via SDK | [Jabra .NET SDK](https://developer.jabra.com/sdks-and-tools/dotnet/): proprietary licence, .NET only; the C/C++ SDK is retired ([legacy](https://developer.jabra.com/sdks-and-tools/legacy)) | [Doc], unsuitable for a Rust exe |
| Nothing | Fast Pair; name | L/R/case over RFCOMM | Gadgetbridge ([Nothing](https://gadgetbridge.org/gadgets/headphones/nothing/)) | [RE] |
| Microsoft Surface Headphones/Earbuds | Device ID VID `0x0006`/`0x045E` [Unk] | Windows' own HFP/GATT battery (already read by ReUI) | Surface Audio app; no public API | [Unk] |

The common thread is that **exact model is often offline-recoverable** (Device ID, Apple/Samsung adverts). **Per-bud batteries** beyond Apple and Fast Pair need a vendor RFCOMM session, which conflicts with PRD §2 goal 5 and with the vendor's own app.

### 2.5 Game controllers

- **Model [Doc]:**
  - `RawGameController.HardwareVendorId` / `HardwareProductId` / `DisplayName` (ReUI's `controllers.rs` already holds the `RawGameController`).
  - Map VID/PID with SDL's [`controller_list.h`](https://github.com/libsdl-org/SDL/blob/main/src/joystick/controller_list.h) (zlib licence, safe to derive a table from). Examples: `045E:0B13` Xbox Series (BLE), `045E:02FD` Xbox One S (BT), `054C:0CE6` DualSense, `054C:0DF2` DualSense Edge, `054C:09CC` DS4 v2, `057E:2009` Switch Pro.
- **Colour:**
  - **Switch Pro and Joy-Con [RE]:** body, button and grip RGB live in SPI flash at `0x6050–0x605B` and are read by an HID subcommand ([dekuNukem notes](https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering/blob/master/spi_flash_notes.md)). That's an actual device read, but cheap.
  - **DualSense [RE]:** colour is a 2-character code in the factory serial, e.g. `00` White, `01` Midnight Black, `02` Cosmic Red, `Z1` God of War ([dualsense-ts `factory_info.ts`](https://github.com/nsfm/dualsense-ts/blob/main/src/hid/factory_info.ts), LGPL-3.0). The feature report is undocumented by Sony.
  - **Xbox and DS4 [Unk]:** no known colour source.

---

## 3. Image sources and licensing

| Option | Look | Legal for an open-source app to ship | Verdict |
|---|---|---|---|
| Windows container thumbnail at runtime | Usually the generic DDORes icon; a real photo only with OEM metadata or INF | Showing what the OS gives at runtime, nothing redistributed | **Use when non-generic** |
| Fast Pair metadata images | Real OEM photos, L/R/case | Private API, Google terms, no image licence (§2.2) | **No** |
| Apple product photos (newsroom, Design Resources) | Real | Apple: images of Apple products only with "express written permission"; must be "an actual photograph… not an artist's rendering" ([guidelines](https://www.apple.com/legal/intellectual-property/guidelinesfor3rdparties.html)). Design Resources are licensed for UI mock-ups ([licence](https://developer.apple.com/support/downloads/terms/apple-design-resources/Apple-Design-Resources-License-20230621-English.pdf)); marketing artwork only for App Store apps ([guidelines](https://developer.apple.com/app-store/marketing/guidelines/)) | **No** |
| Samsung newsroom images | Real | "solely for your personal, informational and non-commercial use" ([terms](https://news.samsung.com/global/terms)); staff: newsroom images "can only be used for press publications" ([forum](https://forum.developer.samsung.com/t/using-an-official-samsung-image-as-a-store-asset/22012)) | **No** |
| Other press kits (Sony, Bose, Microsoft) | Real | Typically editorial-only; none found granting redistribution in software [Unk per brand] | **No, unless written permission** |
| PNGs inside GPL projects (e.g. AirPodsDesktop `Resource/Image/Animation/*.png`) | Real-looking | The GPL covers the authors' own work; the images' provenance isn't stated, so it may be Apple's copyright | **No** |
| User-supplied image (drag a photo onto the device in Settings) | Whatever the user picks | The user's own file, stored locally | **Yes** |
| ReUI's own line-art silhouettes per model family | Recognisable, quiet, crisp | Our own work. Naming the product ("AirPods Pro") is a factual reference to the connected device. Avoid logos | **Yes, the core** |
| Commissioned photoreal renders | Real-looking | Our copyright, but Apple's guideline explicitly wants photographs, not renderings, and trade-dress risk grows with realism | Not worth it; maybe later with legal advice |

**Safe to ship:**
- ReUI-drawn single-colour silhouettes keyed by model family;
- Windows' own non-generic container icon at runtime;
- user images.

Everything photographic and vendor-owned stays out of the repo and the installer.

---

## 4. How others do it on Windows

| Product | What it shows | How |
|---|---|---|
| Windows Settings / Quick Settings | Name, a type icon, one battery %; per-bud only if the driver exposes several | Container icon (§1.2), HFP/GATT battery |
| Swift Pair toast | "New *name* found" + CoD icon | §1.5 |
| Battery Status widget (Insider, Oct 2026) | Type icons + % | [report](https://windowsforum.com/news/windows-11-battery-status-widget-insider-access-setup-and-peripheral-compatibility.447565/) |
| [MagicPods](https://magicpods.app/) (Store) | iPhone-style case-open animation, batteries; AirPods, Beats, "some Sony", Airoha fakes; an optional "MagicAAP" driver for exact % and ANC | Proximity adverts + AAP; closed source; its artwork source is undisclosed |
| [AirPodsDesktop](https://github.com/SpriteOvO/AirPodsDesktop) (GPL-3.0) | Pop-up with a product PNG per model, L/R/case | Proximity adverts; bundled PNGs (provenance unstated) |
| [Bluetooth Battery Monitor](https://www.bluetoothgoodies.com/battery-monitor/) (paid) | One list dialog, tray colour by lowest battery, per-pod for TWS that report it | Windows battery + AirPods adverts; no product images |
| [BlueGauge](https://github.com/iKineticate/BlueGauge) (Rust, MIT) | Tray battery icons | PnP battery |
| Galaxy Buds app (Samsung, Store) | Samsung's own product art, L/R/case | Samsung RFCOMM; Samsung owns the art |
| [Galaxy Buds Client](https://github.com/ThePBone/GalaxyBudsClient) (GPL-3.0) | Model, colour, L/R/case, and more | RFCOMM |
| Phone Link | Phone name and status | [Unk] whether it shows model images |

Only the vendor's own apps show real photos legitimately. Third-party tools either bundle images of unclear provenance or show type icons. A crisp, model-specific silhouette with exact name and per-part batteries would already beat everything third-party on Windows, without legal risk.

---

## 5. Recommendation for ReUI

### 5.1 Identity resolution, in layers (all offline)

```
Exact model   1. Device ID VID/PID from the devnode   (Apple 004C:2014 → "AirPods Pro 2")
              2. Vendor advert, matched to the connected device:
                 Apple 0x07 (model + colour), Samsung EasySetup (model + colour),
                 Fast Pair discoverable model ID (only seen during pairing; remember it per container)
              3. Controllers: RawGameController VID/PID (+ Switch colour, opt-in)
Model family  4. Name heuristics (today's family_of_name) when the VID is a chip vendor
              5. Non-generic Windows container icon (GetThumbnailAsync, Icon ∉ DDORes.dll)
Class         6. CoD / LE Appearance → Kind glyph (today's kinds.rs)
User override 0. A user-chosen image or model for this container always wins
```

**Notes:**
- A model learned once is remembered per `ContainerId` (hashed) in `%LOCALAPPDATA%\ReUI`. Examples: a Fast Pair ID seen during pairing, an Apple PID.
- Later connects then resolve instantly without any scan.
- Only the model key and colour are stored, never addresses or adverts.

### 5.2 Data format: a bundled catalog

The catalog would live at `design/devices/catalog.json` and be compiled into `reui-design` at build time, like the tokens. Example:

```json
{
  "version": 1,
  "families": {
    "airpods-pro": { "kind": "earbuds", "parts": ["left", "right", "case"], "art": "airpods-pro" },
    "airpods-max": { "kind": "headphones", "parts": ["single"], "art": "over-ear" },
    "dualsense":   { "kind": "gamepad", "parts": ["single"], "art": "dualsense" }
  },
  "models": [
    { "keys": ["bt:004C:2014", "apple:1420"], "name": "AirPods Pro 2", "family": "airpods-pro" },
    { "keys": ["bt:004C:2024", "apple:2420"], "name": "AirPods Pro 2", "family": "airpods-pro" },
    { "keys": ["usb:054C:0CE6"], "name": "DualSense", "family": "dualsense" },
    { "keys": ["samsung:39EA48"], "name": "Galaxy Buds2", "family": "galaxy-buds2", "colour": "light-purple" },
    { "keys": ["fastpair:01EEB4"], "name": "WH-1000XM4", "family": "over-ear" }
  ],
  "colours": {
    "apple": { "00": "white", "01": "black", "09": "space-grey" }
  }
}
```

**Key shapes:**
- `bt:<SIG vid>:<pid>` (VIDSource 1) and `usb:<vid>:<pid>` (VIDSource 2, LE HID, controllers);
- `apple:<advert model>`, `samsung:<easysetup id>`, `fastpair:<model id>`.

**Rules:**
- **Chip-vendor VIDs** (Bluetrum, Airoha, Realtek, Actions, Qualcomm, …) are listed as `"generic": true` so layer 1 falls through to the name and Kind.
- **Names are facts** (the marketed product name), not trademarks used as ReUI's own. The UI shows the user's device name as hero and the model only where it adds information, following the no-filler-text rule.
- **Art** is ReUI-drawn single-colour SVG per family (`design/devices/art/<family>.svg`, plus `-compact.svg`), not per model. Thirty-odd AirPods/Beats IDs collapse into about eight shapes: AirPods stem, AirPods Pro, AirPods 4, AirPods Max, Beats in-ear, Beats wingtip, on/over-ear, plus a generic earbuds/headphones/headset/speaker/mouse/keyboard/gamepad set.
- **Updates:** the catalog ships in the release; adding devices is a PR, with no network lookup.

### 5.3 Crisp at island sizes, on dark and light

**Silhouette rendering, phase 1 with no protocol change:**
- Send each SVG once as `HubMsg::Image` and show it as `Art::Logo { image, animated: false }`. That path already means "a single-colour SVG on an accent-tinted tile, drawn in the accent colour", so the island decides the colour and therefore the theme.
- If product art should instead sit untinted (label colour) on the black tile, add **one semantic art kind** rather than reusing `Art::Image`. Today's `Art::Image` is cover-fitted for album art and would crop a cut-out. Something like `Art::Object(ImageId)` would mean "a cut-out object; fit it inside, don't crop". The module still says *what*; the island picks fit, colour and size from tokens.

**Two levels of detail per family** (vector strokes don't scale down gracefully):
- **Compact, 24 DIP circle:** a simplified mark drawn on a 24-unit grid with ≥ 1.5 unit strokes, the same weight as Segoe Fluent glyphs, inset concentric with the pill end (DESIGN.md principle 3).
- **Tile, 56 DIP:** the detailed outline, with L/R/case drawn as separable groups (`id="left"`, `"right"`, `"case"`) so a future expanded view can dim a missing bud or pulse a charging one.

**Snapping and theme:**
- Author on whole-unit grids and render through Direct2D's SVG renderer at the device scale, so lines land on pixels (principle 8).
- No glow and no translucent strokes.
- Single-colour art takes `label.primary` or the accent, so the light and Notch looks work with zero extra assets.

**Device colour:**
- Keep it quiet: never colour the battery ring with it (red and green mean battery state).
- Use the colour to choose a fill for the silhouette's body, for example a dark-body variant drawn as outline-only, so "black AirPods Max" doesn't vanish on a black island.
- Possibly name it in Settings' device page. The island doesn't need the word "Midnight".

**Photos (non-generic container icons, user images):**
- Decode with WIC and downscale once, with high-quality cubic filtering, to the exact pixel size (56 × scale, 24 × scale). Use premultiplied alpha and contain-fit on the tile.
- Cache per size.
- A photo without transparency gets the tile's `radius.concentric` clip like album art.

**Per-part batteries:** these stay as today's rows (L / R / Case), with the art showing only which parts exist.

### 5.4 Privacy

**Default design, nothing leaves the PC:**
- VID/PID, adverts, container icons and the catalog are all local.
- No per-device network lookups, ever.

**If the catalog is ever updated between releases:**
- Fetch the **whole** catalog file from the GitHub release, like the update check.
- The request carries nothing device-specific: no model IDs, no addresses, no list of what's paired.
- It's opt-out in Settings, and the bundled copy always works.

**Also:**
- Never call `nearbydevices-pa` (§2.2).
- Keep PRD §5.3:
  - no names, addresses or instance IDs in logs (log `kind` + the catalog key class, e.g. "bt-did", not the value);
  - adverts not persisted;
  - scans time-boxed.
- The remembered model per container is a local hash and model key and is deleted with the device (watcher `Removed`).
- Opt-in connections (GATT Model ID read, Galaxy Buds/Sony RFCOMM, Switch SPI read) are per-feature toggles, off by default, labelled as "connects to the device".

### 5.5 Phased plan

| Phase | Scope | Exit |
|---|---|---|
| **P0: exact names, offline** ✅ built (branch `devices-catalog`; PRD-devices §6.1) | **As built:** the catalog is Rust tables in `modules/devices/catalog/` (not `catalog.json` in `reui-design`: the hub is its only reader), 35 Apple/Beats and 35 controller models, 20 SIG + 5 USB chip vendors, every row sourced (AppleDB MIT + The Apple Wiki, Apple Support names, SDL zlib; no GPL tables). The Apple colour byte and lid bit are decoded already (from P2). No Microsoft accessory ids yet (no permissive source found). Device IDs are read for every paired container, once. Settings' device list doesn't show the model yet; the island's connect card does. **Planned:** Read `DEVPKEY_Bluetooth_DeviceVIDSource/VID/PID` in `pnp.rs`'s container walk. Add `catalog.json` + a build-time table in `reui-design` (names, family, kind, generic-VID list). Seed it with Apple/Beats (PIDs ↔ advert IDs), Microsoft, controllers (from SDL's list) and the chip-vendor list. Advert matching uses the exact model when the PID is known. Add the missing Apple IDs. Re-test PRD D-6 (AirPods 4). | An AirPods/Beats pair shows the exact model name in Settings' device list (and the island where it adds information); unknown buds fall through unchanged |
| **P1: art** | ReUI-drawn SVG silhouettes for ~8 Apple/Beats families + generic kinds (compact + tile LOD), sent via `Art::Logo`. Decide whether `Art::Object` is needed after seeing it on the island | Connect card shows the right silhouette at 24 and 56 DIP, crisp at 100–200 % scaling, in Dark, Light and Notch looks |
| **P2: colour + Windows art + user image** | Parse Apple colour (byte 9) into the dark/light silhouette variant. `GetThumbnailAsync` on the container, used only when the icon isn't DDORes. Settings › Devices: "Choose picture…" per device (stored locally, keyed by container hash) | Black/white AirPods variants render correctly; a device with OEM `ContainerIcon` shows its photo; user picture overrides |
| **P3: more vendors, passive** | Fast Pair: remember the model ID seen while the device is in pairing mode; parse the battery block from non-discoverable adverts (time-boxed scan, matched by connect timing + RSSI). Samsung EasySetup model and colour. Catalog entries for top Sony/Bose/Jabra/Nothing/Samsung/Pixel models by Device ID and Fast Pair ID | Galaxy Buds and a Fast Pair headset get the exact name; L/R/case when the advert carries it |
| **P4: opt-in connections** | Per-feature toggles: GATT Fast Pair Model ID read on first connect; Switch controller colour (SPI); possibly Galaxy Buds status over RFCOMM. Each needs a PRD amendment (it breaks "no connections of our own") | Off by default; measured radio and CPU cost |

### 5.6 To verify on hardware before building

1. That AirPods, Beats and Galaxy Buds actually populate `DEVPKEY_Bluetooth_DeviceVID/PID` on Windows 11 (public hardware IDs say yes for AirPods). Also whether LE-only mice carry `usb:` VID/PID in `BTHLEDevice` IDs.
2. The Apple PID ↔ advert-ID pairs for AirPods 4 / 4 ANC / 5 / Pro 3 / Max 2 (community tables conflict).
3. The Apple colour byte's real values per model (white AirPods should read `00`).
4. Whether Settings shows any non-DDORes image for a Surface or Xbox accessory, and whether `GetThumbnailAsync` returns the same.
5. How often paired Fast Pair buds include the battery block when used with a PC only (no Android account keys means no non-discoverable advert at all).
6. ReUI's own licence. It has none today; choose one before borrowing any table from GPL sources, even as facts.

---

## Sources

**Microsoft**
- [Swift Pair](https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/bluetooth-swift-pair)
- [Windows Metadata and Internet Services (deprecated)](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/windows-metadata-and-internet-services)
- [Driver Package Container Metadata](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/driver-package-container-metadata)
- [Retirement post repost (May 5, 2025)](https://www.elevenforum.com/t/end-of-servicing-and-retirement-plan-for-device-metadata-on-windows.35831/), [Neowin](https://www.neowin.net/news/microsoft-bringing-two-big-changes-to-hardware-drivers-for-next-gen-windows-11/)
- [DeviceInformation.GetThumbnailAsync](https://learn.microsoft.com/en-us/uwp/api/windows.devices.enumeration.deviceinformation.getthumbnailasync), [DeviceThumbnail](https://learn.microsoft.com/en-us/uwp/api/windows.devices.enumeration.devicethumbnail), [winrt-api #1660](https://github.com/MicrosoftDocs/winrt-api/issues/1660)
- [Device information properties](https://learn.microsoft.com/en-us/windows/apps/develop/devices-sensors/device-information-properties), [System.Devices.AepContainer.ModelName](https://learn.microsoft.com/en-us/windows/win32/properties/props-system-devices-aepcontainer-modelname), [DEVPKEY_Device_ContainerId](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/devpkey-device-containerid), [container properties Q&A](https://learn.microsoft.com/en-us/answers/questions/197894/how-to-get-friendlyname-of-a-device-containerid-(s)
- [VendorIdSource](https://learn.microsoft.com/en-us/windows/win32/properties/props-system-deviceinterface-bluetooth-vendoridsource), [`bthguid.h` (SDK copy)](https://github.com/tpn/winsdk-10/blob/master/Include/10.0.16299.0/km/bthguid.h)

**Google**
- [Fast Pair Provider spec](https://developers.google.com/nearby/fast-pair/specifications/service/provider), [Battery notification](https://developers.google.com/nearby/fast-pair/specifications/extensions/batterynotification), [Device information](https://developers.google.com/nearby/fast-pair/specifications/extensions/deviceinformation)
- [AOSP FastPairDeviceMetadata](https://android.googlesource.com/platform/prebuilts/fullsdk/sources/+/refs/heads/androidx-appcompat-release/android-34/android/nearby/FastPairDeviceMetadata.java)
- [Chromium device_metadata_fetcher.cc (mirror)](https://github.com/nwjs/chromium.src/blob/HEAD/ash/quick_pair/repository/fast_pair/device_metadata_fetcher.cc), [WhisperPair resolver](https://github.com/KULeuven-COSIC/WhisperPair/blob/main/toolkit-server/src/model-id-resolver/index.ts)
- [Google APIs Terms of Service](https://developers.google.com/terms), [FastPair-Models list](https://github.com/DiamondRoPlayz/FastPair-Models)

**Apple (community)**
- [CAPod](https://github.com/d4rken-org/capod), [LibrePods](https://github.com/kavishdevar/librepods) ([BLE notes](https://deepwiki.com/kavishdevar/librepods/2.3-ble-advertisements)), [AirPodsDesktop](https://github.com/SpriteOvO/AirPodsDesktop) ([#240](https://github.com/SpriteOvO/AirPodsDesktop/issues/240)), [OpenPods](https://github.com/adolfintel/OpenPods), [AirBattery](https://github.com/lihaoyun6/AirBattery), [kAirPods](https://github.com/can1357/kAirPods)
- [furiousMAC/continuity](https://github.com/furiousMAC/continuity), [airpods-notify notes](https://github.com/fischejo/airpods-notify/blob/master/doc/proximity_protocol.md), [Raycast Toothpick table](https://github.com/raycast/extensions/blob/main/extensions/toothpick/src/core/devices/constants/specifications/apple/airpods.ts)
- [BatteryInfo PR #36](https://github.com/whisperers26/BatteryInfo/pull/36), [The Apple Wiki: Bluetooth PIDs](https://theapplewiki.com/wiki/Bluetooth_PIDs)

**Apple (legal)**
- [Guidelines for using Apple trademarks and copyrights](https://www.apple.com/legal/intellectual-property/guidelinesfor3rdparties.html), [Apple Design Resources licence](https://developer.apple.com/support/downloads/terms/apple-design-resources/Apple-Design-Resources-License-20230621-English.pdf), [App Store marketing guidelines](https://developer.apple.com/app-store/marketing/guidelines/)

**Samsung**
- [Newsroom terms](https://news.samsung.com/global/terms), [developer forum on newsroom images](https://forum.developer.samsung.com/t/using-an-official-samsung-image-as-a-store-asset/22012), [Galaxy Buds (Store)](https://apps.microsoft.com/detail/9nhtlwtkfznb), [GalaxyBudsClient](https://github.com/ThePBone/GalaxyBudsClient), [EasySetup IDs (Momentum)](https://github.com/Next-Flip/Momentum-Apps/blob/dev/ble_spam/protocols/easysetup.c)

**Other vendors**
- [SonyHeadphonesClient](https://github.com/mos9527/SonyHeadphonesClient), [Gadgetbridge](https://codeberg.org/Freeyourgadget/Gadgetbridge) ([Nothing](https://gadgetbridge.org/gadgets/headphones/nothing/), [RE wiki](https://codeberg.org/Freeyourgadget/Gadgetbridge/wiki/BT-Protocol-Reverse-Engineering)), [based-connect](https://github.com/Denton-L/based-connect), [Jabra .NET SDK](https://developer.jabra.com/sdks-and-tools/dotnet/), [Jabra legacy SDKs](https://developer.jabra.com/sdks-and-tools/legacy)

**Controllers**
- [SDL controller_list.h](https://github.com/libsdl-org/SDL/blob/main/src/joystick/controller_list.h), [Switch SPI flash notes](https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering/blob/master/spi_flash_notes.md), [dualsense-ts factory_info](https://github.com/nsfm/dualsense-ts/blob/main/src/hid/factory_info.ts), [virtualgamepad #161](https://github.com/anderstvoss/virtualgamepad/issues/161)

**Windows apps**
- [MagicPods](https://magicpods.app/), [Bluetooth Battery Monitor](https://www.bluetoothgoodies.com/battery-monitor/), [BlueGauge](https://github.com/iKineticate/BlueGauge), [Battery Status widget report](https://windowsforum.com/news/windows-11-battery-status-widget-insider-access-setup-and-peripheral-compatibility.447565/)

**Bluetooth SIG**
- [Company identifiers (Nordic database)](https://github.com/NordicSemiconductor/bluetooth-numbers-database)
