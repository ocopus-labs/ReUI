//! Measures Windows' notification listener from an unpackaged exe (no package identity),
//! for docs/PRD-notifications.md. Read-only towards the user's notifications: it never
//! removes or clears them, and prints only counts and timings for them. Full fields are
//! printed only for the test toasts it sends itself, under Windows PowerShell's AUMID with a
//! tag of its own (and only those are removed again, through the toast history by tag).
//!
//! ```text
//! cargo run -p reui-hub --example notifications_probe -- access
//! cargo run -p reui-hub --example notifications_probe -- read [calls]
//! cargo run -p reui-hub --example notifications_probe -- events
//! cargo run -p reui-hub --example notifications_probe -- poll <interval_ms> <seconds> [service_pid]
//! cargo run -p reui-hub --example notifications_probe -- latency <interval_ms> <toasts>
//! cargo run -p reui-hub --example notifications_probe -- scale <toasts>
//! cargo run -p reui-hub --example notifications_probe -- wnf <names.txt>
//! ```

use std::time::{Duration, Instant};

use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{Size, TypedEventHandler};
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, ToastNotification, ToastNotificationManager, UserNotification, UserNotificationChangedEventArgs};
use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

const PS_AUMID: &str = r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";
const TAG_GROUP: &str = "reui-probe";

fn main() -> windows::core::Result<()> {
    unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED)? };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, d: u64| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(d);
    let t = Instant::now();
    let listener = UserNotificationListener::Current()?;
    println!("Current(): {:?}", t.elapsed());
    match args.first().map(String::as_str) {
        Some("access") => access(&listener),
        Some("read") => read(&listener, arg(1, 20) as usize),
        Some("events") => events(&listener),
        Some("poll") => poll(&listener, arg(1, 2000), arg(2, 60), args.get(3).and_then(|a| a.parse().ok())),
        Some("latency") => latency(&listener, arg(1, 2000), arg(2, 5) as usize),
        Some("scale") => scale(&listener, arg(1, 20) as usize),
        Some("wnfwatch") => wnfwatch(arg(1, 60)),
        Some("wnf") => wnf(args.get(1).map(String::as_str).unwrap_or("names.txt")),
        _ => {
            eprintln!("access | read [n] | events | poll <ms> <s> [pid] | latency <ms> <n>");
            Ok(())
        }
    }
}

fn access(listener: &UserNotificationListener) -> windows::core::Result<()> {
    let status = listener.GetAccessStatus()?;
    println!("GetAccessStatus: {}", name(status));
    let t = Instant::now();
    let asked = listener.RequestAccessAsync()?.join()?;
    println!("RequestAccessAsync: {} in {:?}", name(asked), t.elapsed());
    Ok(())
}

fn name(s: UserNotificationListenerAccessStatus) -> &'static str {
    match s {
        UserNotificationListenerAccessStatus::Allowed => "Allowed",
        UserNotificationListenerAccessStatus::Denied => "Denied",
        UserNotificationListenerAccessStatus::Unspecified => "Unspecified",
        _ => "?",
    }
}

fn list(listener: &UserNotificationListener) -> windows::core::Result<Vec<UserNotification>> {
    let v = listener.GetNotificationsAsync(NotificationKinds::Toast)?.join()?;
    let mut out = Vec::with_capacity(v.Size()? as usize);
    for n in &v {
        out.push(n);
    }
    Ok(out)
}

/// Our own test toasts are the ones under PowerShell's AUMID whose first line is ours.
fn is_probe(n: &UserNotification) -> bool {
    let aumid = n.AppInfo().and_then(|a| a.AppUserModelId()).unwrap_or_default();
    aumid == PS_AUMID && texts(n).first().is_some_and(|t| t.starts_with("ReUI probe"))
}

fn texts(n: &UserNotification) -> Vec<String> {
    let binding = n.Notification().and_then(|n| n.Visual()).and_then(|v| v.GetBinding(&KnownNotificationBindings::ToastGeneric()?));
    let Ok(binding) = binding else { return Vec::new() };
    binding.GetTextElements().map(|els| (&els).into_iter().filter_map(|e| e.Text().ok()).map(|h| h.to_string()).collect()).unwrap_or_default()
}

fn read(listener: &UserNotificationListener, calls: usize) -> windows::core::Result<()> {
    send(1)?;
    std::thread::sleep(Duration::from_millis(500));
    let cpu0 = cpu_ms(None);
    let mut times = Vec::new();
    let mut last = Vec::new();
    for _ in 0..calls {
        let t = Instant::now();
        last = list(listener)?;
        times.push(t.elapsed());
    }
    let cpu = cpu_ms(None) - cpu0;
    times.sort();
    println!("GetNotificationsAsync x{calls}: count {}, min {:?}, median {:?}, max {:?}, cpu {:.2} ms/call", last.len(), times[0], times[times.len() / 2], times[times.len() - 1], cpu / calls as f64);

    // Field cost and availability, per notification, without printing the user's data.
    let (mut with_name, mut with_text, mut with_logo) = (0, 0, 0);
    let t = Instant::now();
    for n in &last {
        let info = n.AppInfo().ok();
        with_name += info.as_ref().and_then(|a| a.DisplayInfo().ok()).and_then(|d| d.DisplayName().ok()).is_some_and(|s| !s.is_empty()) as u32;
        with_text += (!texts(n).is_empty()) as u32;
        with_logo += info.as_ref().and_then(|a| a.DisplayInfo().ok()).and_then(|d| d.GetLogo(Size { Width: 64.0, Height: 64.0 }).ok()).is_some() as u32;
    }
    println!("fields for {} toasts in {:?}: display name {with_name}, text {with_text}, logo ref {with_logo}", last.len(), t.elapsed());

    for n in last.iter().filter(|n| is_probe(n)) {
        let info = n.AppInfo()?;
        let display = info.DisplayInfo()?;
        let t = Instant::now();
        let logo = display.GetLogo(Size { Width: 64.0, Height: 64.0 }).and_then(|r| r.OpenReadAsync()?.join()).map(|s| format!("{} bytes {}", s.Size().unwrap_or(0), s.ContentType().unwrap_or_default())).unwrap_or_else(|e| format!("err {:#010x} {}", e.code().0 as u32, e.message()));
        let logo_ms = t.elapsed();
        println!(
            "probe toast: id {} aumid {:?} name {:?} pfn {:?} created {} texts {:?} logo {logo} ({logo_ms:?})",
            n.Id()?,
            info.AppUserModelId()?.to_string(),
            display.DisplayName()?.to_string(),
            info.PackageFamilyName().map(|h| h.to_string()).unwrap_or_else(|e| format!("err {e}")),
            n.CreationTime()?.UniversalTime,
            texts(n),
        );
        let t = Instant::now();
        let again = listener.GetNotification(n.Id()?)?;
        println!("GetNotification(id): {:?} ({})", t.elapsed(), again.Id()?);
    }
    clear_probes();
    Ok(())
}

fn events(listener: &UserNotificationListener) -> windows::core::Result<()> {
    let handler = TypedEventHandler::<UserNotificationListener, UserNotificationChangedEventArgs>::new(|_, args| {
        if let Some(a) = args.as_ref() {
            println!("NotificationChanged: kind {:?} id {:?}", a.ChangeKind().map(|k| k.0), a.UserNotificationId());
        }
        Ok(())
    });
    match listener.NotificationChanged(&handler) {
        Ok(token) => {
            println!("NotificationChanged: subscribed ({token}); sending a test toast");
            send(1)?;
            std::thread::sleep(Duration::from_secs(3));
            _ = listener.RemoveNotificationChanged(token);
        }
        Err(e) => println!("NotificationChanged: {:#010x} {}", e.code().0 as u32, e.message()),
    }
    // ClearNotifications would clear the user's notifications, so it is never called; its
    // failure without identity is the research doc's finding, not re-measured here.
    clear_probes();
    Ok(())
}

/// A poll loop as the module would run it: list, diff ids, sleep. CPU of this process and
/// (given its pid) of the notification service it calls into.
fn poll(listener: &UserNotificationListener, ms: u64, seconds: u64, service: Option<u32>) -> windows::core::Result<()> {
    let (cpu0, svc0) = (cpu_ms(None), service.map(|p| cpu_ms(Some(p))));
    let (cyc0, svcyc0) = (cycles(None), service.map(|p| cycles(Some(p))));
    let (mut first, mut max) = (Duration::ZERO, Duration::ZERO);
    let start = Instant::now();
    let mut seen: Vec<u32> = Vec::new();
    let mut polls = 0u64;
    let mut call = Duration::ZERO;
    while start.elapsed() < Duration::from_secs(seconds) {
        let t = Instant::now();
        let ids: Vec<u32> = list(listener)?.iter().filter_map(|n| n.Id().ok()).collect();
        let took = t.elapsed();
        call += took;
        if polls == 0 {
            first = took;
        } else {
            max = max.max(took);
        }
        let new = ids.iter().filter(|i| !seen.contains(i)).count();
        if polls > 0 && new > 0 {
            println!("{:>6.1}s: {new} new", start.elapsed().as_secs_f64());
        }
        seen = ids;
        polls += 1;
        std::thread::sleep(Duration::from_millis(ms));
    }
    let wall = start.elapsed().as_secs_f64();
    let cpu = cpu_ms(None) - cpu0;
    let cyc = cycles(None) - cyc0;
    println!(
        "poll {ms} ms for {wall:.0}s: {polls} polls, first call {first:?}, then mean {:?} max {max:?}; probe cpu {cpu:.1} ms ({:.4} % of one core), {:.2} Mcycles/poll",
        (call - first) / (polls.max(2) - 1) as u32,
        cpu / (wall * 10.0),
        cyc as f64 / polls as f64 / 1e6
    );
    if let (Some(pid), Some(s0), Some(c0)) = (service, svc0, svcyc0) {
        let svc = cpu_ms(Some(pid)) - s0;
        let c = cycles(Some(pid)) - c0;
        println!("  service pid {pid}: cpu {svc:.1} ms, {:.2} Mcycles/poll (includes everything else it did)", c as f64 / polls as f64 / 1e6);
    }
    Ok(())
}

/// Sends test toasts and measures how long a poller at `ms` takes to see each.
fn latency(listener: &UserNotificationListener, ms: u64, toasts: usize) -> windows::core::Result<()> {
    let mut seen: Vec<u32> = list(listener)?.iter().filter_map(|n| n.Id().ok()).collect();
    for i in 0..toasts {
        // Off the poll grid, so the latency spread is real.
        std::thread::sleep(Duration::from_millis(ms + 137 * i as u64));
        let sent = Instant::now();
        send(i + 1)?;
        loop {
            let notes = list(listener)?;
            let fresh = notes.iter().filter(|n| n.Id().is_ok_and(|id| !seen.contains(&id))).any(is_probe);
            seen = notes.iter().filter_map(|n| n.Id().ok()).collect();
            if fresh {
                println!("toast {}: seen after {:?}", i + 1, sent.elapsed());
                break;
            }
            if sent.elapsed() > Duration::from_secs(30) {
                println!("toast {}: not seen in 30 s", i + 1);
                break;
            }
            // The poller's next tick, measured from the send so the result is send-to-seen.
            std::thread::sleep(Duration::from_millis(ms));
        }
    }
    clear_probes();
    Ok(())
}

/// What a list costs as the notification centre fills: `toasts` probe toasts at once.
fn scale(listener: &UserNotificationListener, toasts: usize) -> windows::core::Result<()> {
    for i in 0..toasts {
        send(i + 1)?;
    }
    std::thread::sleep(Duration::from_secs(2));
    let (cpu0, cyc0) = (cpu_ms(None), cycles(None));
    let t = Instant::now();
    let mut count = 0;
    for _ in 0..20 {
        count = list(listener)?.len();
    }
    let per = t.elapsed() / 20;
    println!("{count} toasts listed: {per:?}/call, cpu {:.2} ms/call, {:.2} Mcycles/call", (cpu_ms(None) - cpu0) / 20.0, (cycles(None) - cyc0) as f64 / 20.0 / 1e6);
    let notes = list(listener)?;
    let t = Instant::now();
    let ids: Vec<u32> = notes.iter().filter_map(|n| n.Id().ok()).collect();
    println!("ids of {}: {:?}", ids.len(), t.elapsed());
    let t = Instant::now();
    let n = notes.iter().filter(|n| is_probe(n)).count();
    println!("app + text of {n}: {:?}", t.elapsed());
    clear_probes();
    Ok(())
}

type QueryWnf = unsafe extern "system" fn(state: *const u64, type_id: *const std::ffi::c_void, scope: *const std::ffi::c_void, stamp: *mut u32, buffer: *mut std::ffi::c_void, size: *mut u32) -> i32;

/// Which well-known WNF states change when a toast is published: each state's change stamp
/// (never its data) before and after a quiet wait, then before and after a test toast.
fn wnf(names: &str) -> windows::core::Result<()> {
    use windows::core::{s, w};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    let query: QueryWnf = unsafe { std::mem::transmute(GetProcAddress(GetModuleHandleW(w!("ntdll.dll"))?, s!("NtQueryWnfStateData")).expect("NtQueryWnfStateData")) };
    let ids: Vec<u64> = std::fs::read_to_string(names).unwrap_or_default().lines().filter_map(|l| u64::from_str_radix(l.trim(), 16).ok()).collect();
    let mut buffer = vec![0u8; 8192];
    let mut stamps = || -> Vec<Option<u32>> {
        ids.iter()
            .map(|id| {
                let (mut stamp, mut size) = (0u32, buffer.len() as u32);
                let status = unsafe { query(id, std::ptr::null(), std::ptr::null(), &mut stamp, buffer.as_mut_ptr().cast(), &mut size) };
                (status >= 0).then_some(stamp)
            })
            .collect()
    };
    let changed = |a: &[Option<u32>], b: &[Option<u32>]| -> Vec<u64> { ids.iter().zip(a.iter().zip(b)).filter(|(_, (x, y))| x.is_some() && x != y).map(|(id, _)| *id).collect() };
    let s0 = stamps();
    println!("{} names, {} readable", ids.len(), s0.iter().flatten().count());
    for round in 1..=3 {
        let a = stamps();
        std::thread::sleep(Duration::from_secs(3));
        let b = stamps();
        send(round)?;
        std::thread::sleep(Duration::from_secs(3));
        let c = stamps();
        let quiet = changed(&a, &b);
        let toast: Vec<u64> = changed(&b, &c).into_iter().filter(|id| !quiet.contains(id)).collect();
        println!("round {round}: quiet changes {quiet:x?}; toast-only changes {toast:x?}");
        clear_probes();
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

type WnfCallback = unsafe extern "system" fn(state: u64, stamp: u32, type_id: *const std::ffi::c_void, context: *const std::ffi::c_void, buffer: *const u8, length: u32) -> i32;
type SubscribeWnf = unsafe extern "system" fn(subscription: *mut *mut std::ffi::c_void, state: u64, stamp: u32, callback: WnfCallback, context: *const std::ffi::c_void, type_id: *const std::ffi::c_void, group: u32, unknown: u32) -> i32;

static SENT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

unsafe extern "system" fn on_wnf(state: u64, stamp: u32, _: *const std::ffi::c_void, _: *const std::ffi::c_void, buffer: *const u8, length: u32) -> i32 {
    let sent = *SENT.lock().unwrap();
    let since = sent.map(|t| format!("{:?} after the send", t.elapsed())).unwrap_or_default();
    // The data only once the probe has sent its own toast; in the quiet minute the length alone.
    let data: &[u8] = if buffer.is_null() || sent.is_none() { &[] } else { std::slice::from_raw_parts(buffer, length.min(64) as usize) };
    println!("  wnf {state:x} stamp {stamp} len {length} {since} {data:02x?}");
    0
}

/// Subscribes to the states `wnf` found and reports each change: during a quiet minute,
/// then for test toasts and their removal.
fn wnfwatch(quiet: u64) -> windows::core::Result<()> {
    use windows::core::{s, w};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    let ntdll = unsafe { GetModuleHandleW(w!("ntdll.dll"))? };
    let subscribe: SubscribeWnf = unsafe { std::mem::transmute(GetProcAddress(ntdll, s!("RtlSubscribeWnfStateChangeNotification")).expect("subscribe")) };
    let query: QueryWnf = unsafe { std::mem::transmute(GetProcAddress(ntdll, s!("NtQueryWnfStateData")).expect("query")) };
    for state in [0x0D83_063E_A3B8_D035u64, 0x0D83_063E_A3BC_1035, 0x0D83_063E_A3BD_0035, 0x0D83_063E_A3BF_5875] {
        let (mut stamp, mut size, mut buf) = (0u32, 64u32, [0u8; 64]);
        unsafe { query(&state, std::ptr::null(), std::ptr::null(), &mut stamp, buf.as_mut_ptr().cast(), &mut size) };
        let mut sub = std::ptr::null_mut();
        let status = unsafe { subscribe(&mut sub, state, stamp, on_wnf, std::ptr::null(), std::ptr::null(), 0, 0) };
        println!("subscribed {state:x} at stamp {stamp} (size {size}): {status:#x}");
    }
    println!("quiet for {quiet} s");
    std::thread::sleep(Duration::from_secs(quiet));
    for n in 1..=3 {
        println!("sending toast {n}");
        *SENT.lock().unwrap() = Some(Instant::now());
        send(n)?;
        std::thread::sleep(Duration::from_secs(3));
    }
    println!("removing the probe's toasts");
    *SENT.lock().unwrap() = Some(Instant::now());
    clear_probes();
    std::thread::sleep(Duration::from_secs(3));
    Ok(())
}

fn send(n: usize) -> windows::core::Result<()> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>ReUI probe {n}</text><text>A test toast from ReUI's notification spike</text></binding></visual></toast>"
    )))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    toast.SetTag(&HSTRING::from(format!("{TAG_GROUP}-{n}")))?;
    toast.SetGroup(&HSTRING::from(TAG_GROUP))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(PS_AUMID))?.Show(&toast)
}

/// Removes only this probe's toasts (its group under PowerShell's AUMID).
fn clear_probes() {
    if let Ok(history) = ToastNotificationManager::History() {
        if let Err(e) = history.RemoveGroupWithId(&HSTRING::from(TAG_GROUP), &HSTRING::from(PS_AUMID)) {
            println!("removing the probe's toasts: {e}");
        }
    }
}

fn cycles(pid: Option<u32>) -> u64 {
    use windows::Win32::System::WindowsProgramming::QueryProcessCycleTime;
    unsafe {
        let (process, close) = match pid {
            Some(pid) => match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(h) => (h, true),
                Err(_) => return 0,
            },
            None => (GetCurrentProcess(), false),
        };
        let mut c = 0;
        _ = QueryProcessCycleTime(process, &mut c);
        if close {
            _ = CloseHandle(process);
        }
        c
    }
}

fn cpu_ms(pid: Option<u32>) -> f64 {
    unsafe {
        let (process, close) = match pid {
            Some(pid) => match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(h) => (h, true),
                Err(_) => return f64::NAN,
            },
            None => (GetCurrentProcess(), false),
        };
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ok = GetProcessTimes(process, &mut c, &mut e, &mut k, &mut u).is_ok();
        if close {
            _ = CloseHandle(process);
        }
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0;
        if ok {
            t(k) + t(u)
        } else {
            f64::NAN
        }
    }
}
