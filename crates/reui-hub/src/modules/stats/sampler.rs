//! The sampler: a thread that exists only while the stats card is open. It reads every
//! second (Task Manager's normal speed), turns two reads into rates and shares, and sends a
//! [`Reading`]. Dropping the [`Sampler`] ends the thread at once (its wait is on the stop
//! channel), so a closed card costs nothing: no thread, no timer, no counters open.
//!
//! Everything here blocks (PDH, WinRT for the battery), so none of it runs on the hub thread.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use super::maths::{self, CpuTimes, Interface};
use super::{os, pdh};
use crate::modules::devices::power;

/// Task Manager's "Normal" update speed.
pub const INTERVAL: Duration = Duration::from_secs(1);
/// The first reading comes this soon, so the card can open with numbers in it.
const FIRST: Duration = Duration::from_millis(250);

/// What to read; the parts the card doesn't show cost nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    pub gpu: bool,
    pub disk: bool,
    pub network: bool,
    pub power: bool,
}

/// One second's worth. `None` is "unknown" (no counter, first sample), never zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reading {
    /// Which run of the sampler this came from, so a reading still queued from a card that
    /// closed is dropped.
    pub run: u32,
    /// Share of all cores busy, 0..=1.
    pub cpu: Option<f32>,
    /// (in use, total) bytes.
    pub memory: Option<(u64, u64)>,
    /// The busiest engine of any GPU, 0..=1.
    pub gpu: Option<f32>,
    /// That GPU's dedicated memory in use, bytes.
    pub gpu_memory: Option<u64>,
    /// Disks' active time, 0..=1.
    pub disk: Option<f32>,
    /// (down, up) bytes per second and the fastest link's bits per second.
    pub network: Option<(f64, f64, u64)>,
    /// The battery's charge rate in mW, negative while it discharges; `None` on a desktop.
    pub power_mw: Option<i32>,
}

pub struct Sampler {
    _stop: mpsc::Sender<()>,
}

impl Sampler {
    /// Starts the thread. `None` if it couldn't be created.
    pub fn start(run: u32, wanted: Wanted, out: UnboundedSender<Reading>) -> Option<Self> {
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::Builder::new().name("stats".into()).spawn(move || sample(run, wanted, &stopped, &out));
        thread.ok().map(|_| Self { _stop: stop })
    }
}

/// The previous read, for the deltas.
struct Last {
    at: Instant,
    cpu: Option<CpuTimes>,
    interfaces: Option<Vec<Interface>>,
}

fn sample(run: u32, wanted: Wanted, stopped: &mpsc::Receiver<()>, out: &UnboundedSender<Reading>) {
    // The battery's report is WinRT.
    let battery = if wanted.power && power::has_battery() {
        _ = unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED) };
        power::aggregate()
    } else {
        None
    };
    let mut query = (wanted.gpu || wanted.disk).then(|| pdh::Query::open(wanted.gpu, wanted.disk)).flatten();
    let read_net = || wanted.network.then(os::interfaces).flatten();
    let mut last = Last { at: Instant::now(), cpu: os::cpu(), interfaces: read_net() };
    let mut wait = FIRST;
    loop {
        match stopped.recv_timeout(wait) {
            Err(RecvTimeoutError::Timeout) => {}
            // The card closed (the sender was dropped).
            _ => return,
        }
        wait = INTERVAL;
        let started = Instant::now();
        let (cpu, interfaces) = (os::cpu(), read_net());
        let secs = started.duration_since(last.at).as_secs_f64();
        let mut reading = Reading { run, memory: os::memory(), power_mw: battery.as_ref().and_then(power::charge_rate_mw), ..Reading::default() };
        reading.cpu = last.cpu.zip(cpu).and_then(|(a, b)| maths::cpu_busy(a, b));
        reading.network = last.interfaces.as_deref().zip(interfaces.as_deref()).map(|(a, b)| maths::network(a, b, secs));
        if let Some(q) = query.as_mut() {
            let values = q.collect();
            if let Some((busy, adapter)) = maths::gpu_busiest(values.engines.iter().map(|(n, v)| (n.as_str(), *v))) {
                reading.gpu = Some(busy);
                reading.gpu_memory = values.dedicated.iter().find(|(n, _)| maths::adapter_of(n) == Some(adapter)).map(|(_, v)| *v as u64);
            }
            reading.disk = values.disk;
        }
        last = Last { at: started, cpu, interfaces };
        if out.send(reading).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What each read costs on this machine: `cargo test -p reui-hub stats_cost -- --ignored
    /// --nocapture`. Wall time per call, and CPU time per call over the whole run (process
    /// times are only precise in total).
    #[test]
    #[ignore]
    fn stats_cost_per_read() {
        fn cpu_time() -> f64 {
            use windows::Win32::Foundation::FILETIME;
            use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
            let (mut a, mut b, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            unsafe { GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u) }.unwrap();
            let t = |t: FILETIME| ((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) as f64 / 1e4;
            t(k) + t(u)
        }
        fn cost(name: &str, n: u32, mut f: impl FnMut()) {
            let (wall, cpu) = (Instant::now(), cpu_time());
            for _ in 0..n {
                f();
            }
            let per = |ms: f64| ms / f64::from(n);
            println!("{name:<28} wall {:>8.3} ms   cpu {:>8.3} ms", per(wall.elapsed().as_secs_f64() * 1e3), per(cpu_time() - cpu));
        }
        cost("GetSystemTimes", 2000, || _ = os::cpu());
        cost("GlobalMemoryStatusEx", 2000, || _ = os::memory());
        cost("GetIfTable2", 200, || _ = os::interfaces());
        if let Some(b) = power::has_battery().then(power::aggregate).flatten() {
            cost("Battery report", 200, || _ = power::charge_rate_mw(&b));
        }
        let mut disk = pdh::Query::open(false, true).expect("disk counters");
        cost("PDH disk", 200, || _ = disk.collect());
        let mut gpu = pdh::Query::open(true, false).expect("GPU counters");
        let mut instances = 0;
        cost("PDH GPU engines + memory", 50, || instances = gpu.collect().engines.len());
        println!("GPU engine instances: {instances}");
        cost("PDH open (GPU + disk)", 5, || _ = pdh::Query::open(true, true));
    }
}
