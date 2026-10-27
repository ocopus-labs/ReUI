//! Idle cost, measured live (About): CPU over the last few seconds and private memory of
//! ReUI's own processes. Sampled only while About is showing.

use std::time::Instant;

use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ};

pub const PROCESSES: [&str; 3] = ["reui-hub.exe", "reui-island.exe", "reui-settings.exe"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// Private bytes.
    pub memory: u64,
    /// Share of the whole machine (all cores), 0..=100; `None` until two samples exist.
    pub cpu: Option<f64>,
    pub running: bool,
}

/// One sample: per process name, (CPU time in 100 ns, private bytes, any running).
type Sample = [(u64, u64, bool); 3];

pub struct Stats {
    last: Option<(Instant, Sample)>,
    pub usage: [Usage; 3],
    cores: f64,
}

impl Default for Stats {
    fn default() -> Self {
        Self { last: None, usage: Default::default(), cores: std::thread::available_parallelism().map_or(1.0, |n| n.get() as f64) }
    }
}

impl Stats {
    pub fn sample(&mut self) {
        let now = Instant::now();
        let sample = read();
        for (i, &(cpu_time, memory, running)) in sample.iter().enumerate() {
            let cpu = self.last.as_ref().map(|(at, last)| cpu_percent(cpu_time.saturating_sub(last[i].0), now - *at, self.cores));
            self.usage[i] = Usage { memory, cpu, running };
        }
        self.last = Some((now, sample));
    }

    /// Forget the old sample, so the next one starts a fresh interval.
    pub fn reset(&mut self) {
        self.last = None;
    }

    pub fn total(&self) -> Usage {
        let running: Vec<&Usage> = self.usage.iter().filter(|u| u.running).collect();
        Usage {
            memory: running.iter().map(|u| u.memory).sum(),
            cpu: running.iter().map(|u| u.cpu).sum(),
            running: !running.is_empty(),
        }
    }
}

/// CPU time used over a wall-clock interval, as a share of every core.
pub fn cpu_percent(cpu_100ns: u64, wall: std::time::Duration, cores: f64) -> f64 {
    let wall_100ns = wall.as_nanos() as f64 / 100.0;
    if wall_100ns <= 0.0 {
        return 0.0;
    }
    (cpu_100ns as f64 / wall_100ns / cores * 100.0).clamp(0.0, 100.0)
}

/// "0.0 %", "1.4 %": one decimal, tabular.
pub fn percent(cpu: f64) -> String {
    format!("{cpu:.1} %")
}

fn read() -> Sample {
    let mut sample = Sample::default();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return sample };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
            if let Some(i) = PROCESSES.iter().position(|p| *p == name) {
                if let Some((cpu, memory)) = process(entry.th32ProcessID) {
                    sample[i].0 += cpu;
                    sample[i].1 += memory;
                    sample[i].2 = true;
                }
            }
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        _ = CloseHandle(snapshot);
    }
    sample
}

/// (CPU time in 100 ns units, private bytes)
fn process(pid: u32) -> Option<(u64, u64)> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let times = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let mut counters = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
        let memory = GetProcessMemoryInfo(handle, &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS, counters.cb).is_ok();
        _ = CloseHandle(handle);
        let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
        (times && memory).then(|| (ticks(kernel) + ticks(user), counters.PrivateUsage as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cpu_is_a_share_of_all_cores() {
        // 0.5 s of CPU over 2 s on 4 cores: 6.25 %.
        assert!((cpu_percent(5_000_000, Duration::from_secs(2), 4.0) - 6.25).abs() < 1e-9);
        assert_eq!(cpu_percent(1, Duration::ZERO, 4.0), 0.0);
        assert_eq!(percent(0.04), "0.0 %");
        assert_eq!(percent(1.45), "1.4 %");
    }

    #[test]
    fn this_process_is_measured() {
        let mut stats = Stats::default();
        stats.sample();
        stats.sample();
        // The test binary isn't one of ReUI's; whatever runs, totals add up.
        let total = stats.total();
        assert_eq!(total.memory, stats.usage.iter().filter(|u| u.running).map(|u| u.memory).sum::<u64>());
    }
}
