//! Display brightness for the OSD. The built-in panel is read and set through WMI
//! (`root\wmi`: `WmiMonitorBrightness`, `WmiMonitorBrightnessMethods.WmiSetBrightness`);
//! its changes arrive as a power-setting notification on the OSD's window thread
//! (`osd_os.rs`), not here. External monitors are set over DDC/CI (Dxva2), which has no
//! change events, so they only ever show what ReUI set.
//!
//! Everything blocking runs on one worker thread that owns the COM objects and the physical
//! monitor handles. Writes are throttled per display, trailing edge (the latest value always
//! lands): 50 ms for WMI, 150 ms for DDC/CI. A DDC/CI monitor that fails a write is dropped
//! for the session and reported, so the HUD can take its slider away.

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{BSTR, PCWSTR};
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitor, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, GetMonitorBrightness, GetMonitorCapabilities,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR, QueryDisplayConfig, SetMonitorBrightness, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, MC_CAPS_BRIGHTNESS, PHYSICAL_MONITOR, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE};
use windows::Win32::System::Variant::{VariantClear, VariantToStringAlloc, VariantToUInt32, VARENUM, VARIANT, VT_I4, VT_UI1};
use windows::Win32::System::Wmi::{IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE};

/// The built-in panel's id.
pub const INTERNAL: &str = "internal";
const WMI_INTERVAL: Duration = Duration::from_millis(50);
const DDC_INTERVAL: Duration = Duration::from_millis(150);

/// A display whose brightness ReUI can set.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// [`INTERNAL`], or `ddc-<GDI name>-<n>` for external monitors.
    pub id: String,
    pub name: String,
    /// 0..=1
    pub level: f32,
}

/// What the worker reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    /// Every display that can be set, built-in panel first.
    Displays(Vec<Display>),
    /// A write failed; for DDC/CI monitors this one is gone for the session.
    Failed { id: String },
}

enum Cmd {
    /// Find displays and read their levels (at start, and when displays change).
    Probe,
    Set { id: String, level: f32 },
    Stop,
}

/// Trailing-edge throttle: the latest offered value is written at most once per `interval`.
#[derive(Debug, Clone)]
pub struct Throttle {
    interval: Duration,
    last: Option<Instant>,
    pending: Option<f32>,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self { interval, last: None, pending: None }
    }

    pub fn offer(&mut self, level: f32) {
        self.pending = Some(level);
    }

    /// The value to write now, if any.
    pub fn due(&mut self, now: Instant) -> Option<f32> {
        if self.last.is_some_and(|last| now < last + self.interval) {
            return None;
        }
        let v = self.pending.take()?;
        self.last = Some(now);
        Some(v)
    }

    /// When a pending value becomes due.
    pub fn next(&self) -> Option<Instant> {
        self.pending?;
        Some(self.last.map_or_else(Instant::now, |last| last + self.interval))
    }
}

/// The brightness worker; dropping it stops the thread and releases the monitors.
pub struct Worker {
    cmd: mpsc::Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(reports: UnboundedSender<Report>) -> Option<Self> {
        let (cmd, rx) = mpsc::channel();
        let thread = std::thread::Builder::new().name("reui-brightness".into()).spawn(move || run(rx, reports)).ok()?;
        Some(Self { cmd, thread: Some(thread) })
    }

    pub fn probe(&self) {
        _ = self.cmd.send(Cmd::Probe);
    }

    pub fn set(&self, id: &str, level: f32) {
        _ = self.cmd.send(Cmd::Set { id: id.to_owned(), level: level.clamp(0.0, 1.0) });
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        _ = self.cmd.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            _ = t.join();
        }
    }
}

struct Ddc {
    handle: HANDLE,
    min: u32,
    max: u32,
}

impl Drop for Ddc {
    fn drop(&mut self) {
        _ = unsafe { DestroyPhysicalMonitor(self.handle) };
    }
}

struct State {
    wmi: Option<Wmi>,
    wmi_failed: bool,
    ddc: HashMap<String, Ddc>,
    throttles: HashMap<String, Throttle>,
}

fn run(rx: mpsc::Receiver<Cmd>, reports: UnboundedSender<Report>) {
    _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut state = State { wmi: None, wmi_failed: false, ddc: HashMap::new(), throttles: HashMap::new() };
    loop {
        let next = state.throttles.values().filter_map(Throttle::next).min();
        let cmd = match next {
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(c) => Some(c),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => break,
            },
        };
        match cmd {
            Some(Cmd::Stop) => break,
            Some(Cmd::Probe) => {
                let displays = state.probe();
                _ = reports.send(Report::Displays(displays));
            }
            Some(Cmd::Set { id, level }) => {
                let interval = if id == INTERNAL { WMI_INTERVAL } else { DDC_INTERVAL };
                state.throttles.entry(id).or_insert_with(|| Throttle::new(interval)).offer(level);
            }
            None => {}
        }
        let now = Instant::now();
        let due: Vec<(String, f32)> = state.throttles.iter_mut().filter_map(|(id, t)| Some((id.clone(), t.due(now)?))).collect();
        for (id, level) in due {
            if !state.write(&id, level) {
                if id != INTERNAL {
                    eprintln!("brightness: {id} didn't take a DDC/CI write; leaving it alone this session");
                    state.ddc.remove(&id);
                    state.throttles.remove(&id);
                }
                _ = reports.send(Report::Failed { id });
            }
        }
    }
}

impl State {
    fn wmi(&mut self) -> Option<&Wmi> {
        if self.wmi.is_none() && !self.wmi_failed {
            self.wmi = Wmi::connect();
            self.wmi_failed = self.wmi.is_none();
        }
        self.wmi.as_ref()
    }

    fn probe(&mut self) -> Vec<Display> {
        let names = display_names();
        let mut out = Vec::new();
        if let Some(level) = self.wmi().and_then(Wmi::brightness) {
            let friendly = names.values().find(|n| n.internal).map(|n| n.friendly.as_str());
            out.push(Display { id: INTERNAL.into(), name: super::osd_view::display_label(friendly, true, 1), level: f32::from(level) / 100.0 });
        }
        self.ddc.clear();
        self.throttles.retain(|id, _| id == INTERNAL);
        for (gdi, monitor) in monitors() {
            let name = names.get(&gdi);
            if name.is_some_and(|n| n.internal) {
                continue;
            }
            let short = gdi.trim_start_matches(r"\\.\").to_owned();
            for (i, physical) in physical_monitors(monitor).into_iter().enumerate() {
                let Some((ddc, level)) = probe_ddc(physical) else { continue };
                let id = format!("ddc-{short}-{i}");
                let label = super::osd_view::display_label(name.map(|n| n.friendly.as_str()), false, out.len() + 1);
                out.push(Display { id: id.clone(), name: label, level });
                self.ddc.insert(id, ddc);
            }
        }
        out
    }

    fn write(&mut self, id: &str, level: f32) -> bool {
        if id == INTERNAL {
            return self.wmi().is_some_and(|w| w.set_brightness((level * 100.0).round() as u8));
        }
        let Some(ddc) = self.ddc.get(id) else { return false };
        let value = ddc_value(level, ddc.min, ddc.max);
        unsafe { SetMonitorBrightness(ddc.handle, value) != 0 }
    }
}

/// 0..=1 → the monitor's own range.
pub fn ddc_value(level: f32, min: u32, max: u32) -> u32 {
    let span = max.saturating_sub(min);
    min + (level.clamp(0.0, 1.0) * span as f32).round() as u32
}

/// The monitor's range → 0..=1.
pub fn ddc_level(value: u32, min: u32, max: u32) -> f32 {
    let span = max.saturating_sub(min);
    if span == 0 {
        return 0.0;
    }
    (value.clamp(min, max) - min) as f32 / span as f32
}

/// DDC/CI with brightness: the handle and its current level. Slow (the capabilities string
/// can take a second); failures are quiet.
fn probe_ddc(physical: PHYSICAL_MONITOR) -> Option<(Ddc, f32)> {
    let ddc = Ddc { handle: physical.hPhysicalMonitor, min: 0, max: 0 };
    let (mut caps, mut temps) = (0u32, 0u32);
    let parsed = unsafe { GetMonitorCapabilities(ddc.handle, &mut caps, &mut temps) } != 0;
    if parsed && caps & MC_CAPS_BRIGHTNESS == 0 {
        return None; // dropping `ddc` releases the handle
    }
    // Many monitors send a capabilities string Windows can't parse but answer brightness
    // (VCP 0x10) anyway, so a read that works is enough.
    let (mut min, mut cur, mut max) = (0u32, 0u32, 0u32);
    if unsafe { GetMonitorBrightness(ddc.handle, &mut min, &mut cur, &mut max) } == 0 || max <= min {
        return None;
    }
    let mut ddc = ddc;
    (ddc.min, ddc.max) = (min, max);
    Some((ddc, ddc_level(cur, min, max)))
}

// ---- monitors and their names ------------------------------------------------------------

/// `(GDI device name, handle)` of every monitor.
fn monitors() -> Vec<(String, HMONITOR)> {
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> windows::core::BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<(String, HMONITOR)>) };
        let mut info = MONITORINFOEXW { monitorInfo: MONITORINFO { cbSize: size_of::<MONITORINFOEXW>() as u32, ..Default::default() }, ..Default::default() };
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
            out.push((wide(&info.szDevice), monitor));
        }
        true.into()
    }
    let mut out: Vec<(String, HMONITOR)> = Vec::new();
    _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut out as *mut _ as isize)) };
    out
}

fn physical_monitors(monitor: HMONITOR) -> Vec<PHYSICAL_MONITOR> {
    let mut n = 0u32;
    if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut n) }.is_err() || n == 0 {
        return Vec::new();
    }
    let mut list = vec![PHYSICAL_MONITOR::default(); n as usize];
    if unsafe { GetPhysicalMonitorsFromHMONITOR(monitor, &mut list) }.is_err() {
        return Vec::new();
    }
    list
}

fn wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

struct DisplayName {
    friendly: String,
    internal: bool,
}

/// Built-in panels: the internal connector, or embedded DisplayPort / UDI.
pub fn is_internal_technology(technology: i32) -> bool {
    technology == i32::MIN /* DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL, 0x80000000 */ || technology == 11 || technology == 13
}

/// Monitor names by GDI device name (`\\.\DISPLAY1`), from the display configuration.
fn display_names() -> HashMap<String, DisplayName> {
    let mut out = HashMap::new();
    let (mut np, mut nm) = (0u32, 0u32);
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) } != ERROR_SUCCESS {
        return out;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
    if unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None) } != ERROR_SUCCESS {
        return out;
    }
    for path in &paths[..np as usize] {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0 {
            continue;
        }
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            },
            ..Default::default()
        };
        let friendly = if unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } == 0 { wide(&target.monitorFriendlyDeviceName) } else { String::new() };
        out.insert(wide(&source.viewGdiDeviceName), DisplayName { friendly, internal: is_internal_technology(path.targetInfo.outputTechnology.0) });
    }
    out
}

// ---- WMI ---------------------------------------------------------------------------------

struct Wmi(IWbemServices);

fn variant(vt: VARENUM, set: impl FnOnce(&mut VARIANT)) -> VARIANT {
    let mut v = VARIANT::default();
    // SAFETY: plain-data members of a zeroed VARIANT.
    unsafe {
        (*v.Anonymous.Anonymous).vt = vt;
    }
    set(&mut v);
    v
}

impl Wmi {
    fn connect() -> Option<Self> {
        let run = || -> windows::core::Result<Self> {
            let locator: IWbemLocator = unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }?;
            let empty = BSTR::new();
            let services = unsafe { locator.ConnectServer(&BSTR::from(r"ROOT\WMI"), &empty, &empty, &empty, 0, &empty, None) }?;
            // RPC_C_AUTHN_WINNT (10), RPC_C_AUTHZ_NONE (0): the usual blanket for local WMI.
            unsafe { CoSetProxyBlanket(&services, 10, 0, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE) }?;
            Ok(Self(services))
        };
        run().inspect_err(|e| eprintln!("brightness: WMI unavailable ({e})")).ok()
    }

    fn first(&self, query: &str) -> Option<IWbemClassObject> {
        let flags = WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
        let list = unsafe { self.0.ExecQuery(&BSTR::from("WQL"), &BSTR::from(query), flags, None) }.ok()?;
        let mut objects = [None];
        let mut returned = 0u32;
        _ = unsafe { list.Next(WBEM_INFINITE, &mut objects, &mut returned) };
        if returned == 0 {
            return None;
        }
        objects[0].take()
    }

    fn get(object: &IWbemClassObject, name: &str) -> Option<VARIANT> {
        let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let mut v = VARIANT::default();
        unsafe { object.Get(PCWSTR(name.as_ptr()), 0, &mut v, None, None) }.ok()?;
        Some(v)
    }

    /// The built-in panel's brightness in percent; `None` without one (desktops).
    fn brightness(&self) -> Option<u8> {
        let object = self.first("SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE")?;
        let mut v = Self::get(&object, "CurrentBrightness")?;
        let level = unsafe { VariantToUInt32(&v) }.ok();
        _ = unsafe { VariantClear(&mut v) };
        level.map(|l| l.min(100) as u8)
    }

    fn set_brightness(&self, percent: u8) -> bool {
        let run = || -> Option<()> {
            let instance = self.first("SELECT * FROM WmiMonitorBrightnessMethods WHERE Active=TRUE")?;
            let mut path_v = Self::get(&instance, "__PATH")?;
            let path = unsafe { VariantToStringAlloc(&path_v) }.ok().and_then(|p| {
                let s = unsafe { p.to_string() }.ok();
                unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _)) };
                s
            });
            _ = unsafe { VariantClear(&mut path_v) };
            let path = path?;
            let mut class = None;
            unsafe { self.0.GetObject(&BSTR::from("WmiMonitorBrightnessMethods"), WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut class), None) }.ok()?;
            let class = class?;
            let method: Vec<u16> = "WmiSetBrightness".encode_utf16().chain([0]).collect();
            let mut signature = None;
            unsafe { class.GetMethod(PCWSTR(method.as_ptr()), 0, &mut signature, std::ptr::null_mut()) }.ok()?;
            let params = unsafe { signature?.SpawnInstance(0) }.ok()?;
            let timeout = variant(VT_I4, |v| unsafe { (*v.Anonymous.Anonymous).Anonymous.lVal = 0 });
            let brightness = variant(VT_UI1, |v| unsafe { (*v.Anonymous.Anonymous).Anonymous.bVal = percent.min(100) });
            let put = |name: &str, v: &VARIANT| {
                let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
                unsafe { params.Put(PCWSTR(name.as_ptr()), 0, v, 0) }.ok()
            };
            put("Timeout", &timeout)?;
            put("Brightness", &brightness)?;
            unsafe { self.0.ExecMethod(&BSTR::from(path), &BSTR::from("WmiSetBrightness"), WBEM_GENERIC_FLAG_TYPE(0), None, &params, None, None) }.ok()?;
            Some(())
        };
        run().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_throttle_is_trailing_edge() {
        let t0 = Instant::now();
        let mut t = Throttle::new(Duration::from_millis(150));
        assert_eq!(t.due(t0), None);
        t.offer(0.1);
        assert_eq!(t.due(t0), Some(0.1));
        // Inside the interval values wait; the latest wins.
        t.offer(0.2);
        t.offer(0.3);
        assert_eq!(t.due(t0 + Duration::from_millis(100)), None);
        assert_eq!(t.next(), Some(t0 + Duration::from_millis(150)));
        assert_eq!(t.due(t0 + Duration::from_millis(150)), Some(0.3));
        assert_eq!(t.due(t0 + Duration::from_millis(400)), None);
        assert_eq!(t.next(), None);
    }

    #[test]
    fn brightness_ddc_ranges() {
        assert_eq!(ddc_value(0.5, 0, 100), 50);
        assert_eq!(ddc_value(1.0, 10, 80), 80);
        assert_eq!(ddc_value(-1.0, 10, 80), 10);
        assert_eq!(ddc_level(50, 0, 100), 0.5);
        assert_eq!(ddc_level(5, 10, 80), 0.0);
        assert_eq!(ddc_level(5, 5, 5), 0.0);
    }

    #[test]
    fn brightness_internal_technologies() {
        assert!(is_internal_technology(i32::MIN));
        assert!(is_internal_technology(11) && is_internal_technology(13));
        assert!(!is_internal_technology(5) && !is_internal_technology(10));
    }

    /// Reads (never sets) the displays' brightness. Run with `--ignored`.
    #[test]
    #[ignore = "reads the machine's displays over WMI and DDC/CI"]
    fn brightness_probe_reads() {
        _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let mut state = State { wmi: None, wmi_failed: false, ddc: HashMap::new(), throttles: HashMap::new() };
        println!("{:#?}", state.probe());
        let names: Vec<_> = display_names().into_iter().map(|(gdi, n)| (gdi, n.friendly, n.internal)).collect();
        println!("monitors {:?} names {names:?}", monitors().iter().map(|m| m.0.clone()).collect::<Vec<_>>());
    }
}
