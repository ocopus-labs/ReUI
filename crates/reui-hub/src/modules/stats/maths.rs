//! The arithmetic between two samples, kept apart from the OS reads so it can be tested:
//! CPU busy share from `GetSystemTimes`, byte rates from interface counters, which network
//! interfaces count, and the GPU's busiest engine from PDH's per-process instances.

use std::collections::HashMap;

/// `GetSystemTimes`, in 100 ns ticks. Kernel time includes idle time (the docs say so), so
/// busy is `kernel + user - idle`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

/// The share of all cores that was busy between two samples, 0..=1; `None` when no time
/// passed (or the counters went backwards, which they don't, but a bad read could).
///
/// This is `% Processor Time`, the figure Windows 11 24H2's Task Manager shows (it moved off
/// `% Processor Utility`, which scales by frequency and read high on boosting CPUs).
pub fn cpu_busy(before: CpuTimes, after: CpuTimes) -> Option<f32> {
    let idle = after.idle.checked_sub(before.idle)?;
    let total = after.kernel.checked_sub(before.kernel)? + after.user.checked_sub(before.user)?;
    if total == 0 {
        return None;
    }
    Some((1.0 - idle as f64 / total as f64).clamp(0.0, 1.0) as f32)
}

/// Bytes per second from a counter read twice, `secs` apart. A counter that went backwards
/// (an adapter reset, a driver reload) reads as no traffic rather than a huge number.
pub fn rate(before: u64, after: u64, secs: f64) -> f64 {
    if secs <= 0.0 {
        return 0.0;
    }
    after.checked_sub(before).map_or(0.0, |d| d as f64 / secs)
}

/// What decides whether an interface's bytes count toward "the network".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interface {
    /// `InterfaceLuid`, stable while the adapter exists.
    pub luid: u64,
    /// `IFTYPE` (24 loopback, 131 tunnel).
    pub kind: u32,
    /// `OperStatus` is up.
    pub up: bool,
    /// `HardwareInterface`: a real NIC (Ethernet, Wi-Fi, a phone's USB tether).
    pub hardware: bool,
    /// `FilterInterface`: an NDIS filter's view of another interface; the same bytes again.
    pub filter: bool,
    pub rx: u64,
    pub tx: u64,
    /// Bits per second, the larger of the two directions.
    pub link_bps: u64,
}

const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const IF_TYPE_TUNNEL: u32 = 131;

/// Only real adapters that are up. Filter rows repeat their adapter's counters, and a VPN's
/// virtual adapter carries traffic that also crosses the physical one (encrypted), so
/// counting either would double it. Loopback never leaves the machine.
pub fn counts(i: &Interface) -> bool {
    i.up && i.hardware && !i.filter && i.kind != IF_TYPE_SOFTWARE_LOOPBACK && i.kind != IF_TYPE_TUNNEL
}

/// Download and upload in bytes per second across the counted interfaces present in both
/// samples (one that came or went in between has no rate yet), and the fastest link among
/// them in bits per second.
pub fn network(before: &[Interface], after: &[Interface], secs: f64) -> (f64, f64, u64) {
    let earlier: HashMap<u64, &Interface> = before.iter().filter(|i| counts(i)).map(|i| (i.luid, i)).collect();
    let (mut down, mut up, mut link) = (0.0, 0.0, 0);
    for now in after.iter().filter(|i| counts(i)) {
        let Some(then) = earlier.get(&now.luid) else { continue };
        down += rate(then.rx, now.rx, secs);
        up += rate(then.tx, now.tx, secs);
        link = link.max(now.link_bps);
    }
    (down, up, link)
}

/// One `\GPU Engine(*)\Utilization Percentage` instance's engine, without its process:
/// `pid_1234_luid_0x0_0xD6E1_phys_0_eng_3_engtype_VideoDecode` → `luid_0x0_0xD6E1_phys_0_eng_3`.
/// `engtype` is left out on purpose: it's sometimes blank, and one engine can carry
/// different labels across processes.
pub fn engine_of(instance: &str) -> Option<&str> {
    let at = instance.find("luid_")?;
    let rest = &instance[at..];
    let end = rest.find("_engtype").unwrap_or(rest.len());
    let engine = &rest[..end];
    engine.contains("_eng_").then_some(engine)
}

/// The adapter part of an engine or adapter-memory instance: `luid_0x0_0xD6E1_phys_0`.
pub fn adapter_of(instance: &str) -> Option<&str> {
    let at = instance.find("luid_")?;
    let rest = &instance[at..];
    let end = rest.find("_eng_").unwrap_or(rest.len());
    Some(&rest[..end])
}

/// The GPU figure Task Manager shows: each engine's load summed over the processes using
/// it, then the busiest engine (of any adapter), 0..=1. Engine types aren't added together:
/// 3D and video decode run side by side, and adding them reads over 100 % on a GPU that
/// isn't full. Returns the busiest engine's adapter too, for its memory.
pub fn gpu_busiest<'a>(instances: impl IntoIterator<Item = (&'a str, f64)>) -> Option<(f32, &'a str)> {
    let mut engines: HashMap<&str, f64> = HashMap::new();
    for (name, value) in instances {
        if let Some(engine) = engine_of(name) {
            *engines.entry(engine).or_default() += value.max(0.0);
        }
    }
    let (engine, busiest) = engines.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(a.0)))?;
    Some(((busiest / 100.0).clamp(0.0, 1.0) as f32, adapter_of(engine)?))
}

/// "980 KB/s", "1.2 MB/s", "12 MB/s", "0 KB/s": decimal units like Task Manager, at most
/// three significant digits.
pub fn bytes_per_second(rate: f64) -> String {
    let (value, unit) = scaled(rate.max(0.0), &["KB/s", "MB/s", "GB/s"]);
    format!("{} {unit}", short(value))
}

/// "11.2 GB", "980 MB".
pub fn bytes(n: u64) -> String {
    let (value, unit) = scaled(n as f64, &["KB", "MB", "GB", "TB"]);
    format!("{} {unit}", short(value))
}

/// Bytes into the first unit where the value is under 1000 (starting at kilo).
fn scaled<'u>(n: f64, units: &[&'u str]) -> (f64, &'u str) {
    let mut value = n / 1000.0;
    for (i, unit) in units.iter().enumerate() {
        if value < 999.5 || i == units.len() - 1 {
            return (value, unit);
        }
        value /= 1000.0;
    }
    unreachable!()
}

/// One decimal below 10, none from there ("1.2", "12", "980").
fn short(value: f64) -> String {
    if value < 9.95 {
        let text = format!("{value:.1}");
        text.strip_suffix(".0").map_or(text.clone(), str::to_string)
    } else {
        format!("{value:.0}")
    }
}

/// Memory in use of the total, binary gigabytes as Task Manager counts them but writes "GB":
/// "11.2 of 31.7 GB".
pub fn memory(used: u64, total: u64) -> String {
    let gb = |n: u64| n as f64 / (1u64 << 30) as f64;
    format!("{:.1} of {:.1} GB", gb(used), gb(total))
}

/// "23%", rounded, as the Devices cards write it.
pub fn percent(fraction: f32) -> String {
    format!("{}%", (fraction.clamp(0.0, 1.0) * 100.0).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_cpu_is_busy_time_over_all_time() {
        let a = CpuTimes { idle: 1_000, kernel: 2_000, user: 500 };
        // 1000 ticks passed (kernel 600 + user 400), 750 of them idle.
        let b = CpuTimes { idle: 1_750, kernel: 2_600, user: 900 };
        assert_eq!(cpu_busy(a, b), Some(0.25));
        assert_eq!(cpu_busy(a, a), None, "no time passed");
        assert_eq!(cpu_busy(b, a), None, "backwards");
        let all_idle = CpuTimes { idle: 2_000, kernel: 3_000, user: 500 };
        assert_eq!(cpu_busy(a, all_idle), Some(0.0));
    }

    #[test]
    fn stats_rates_survive_resets() {
        assert_eq!(rate(1_000, 3_000, 2.0), 1_000.0);
        assert_eq!(rate(3_000, 1_000, 1.0), 0.0);
        assert_eq!(rate(0, 10, 0.0), 0.0);
    }

    fn nic(luid: u64, rx: u64, tx: u64) -> Interface {
        Interface { luid, kind: 6, up: true, hardware: true, filter: false, rx, tx, link_bps: 1_000_000_000 }
    }

    #[test]
    fn stats_network_counts_real_adapters_once() {
        let wifi = Interface { kind: 71, link_bps: 866_000_000, ..nic(2, 0, 0) };
        let before = [nic(1, 1_000, 100), Interface { luid: 2, ..wifi }];
        let filter = Interface { filter: true, ..nic(3, 0, 0) };
        let vpn = Interface { hardware: false, kind: 53, ..nic(4, 0, 0) };
        let lo = Interface { kind: 24, hardware: false, ..nic(5, 0, 0) };
        let after = [
            nic(1, 3_000, 300),
            Interface { rx: 500, tx: 50, ..wifi },
            Interface { rx: 9_999, ..filter },
            Interface { rx: 9_999, ..vpn },
            Interface { rx: 9_999, ..lo },
            // Arrived between samples: no rate yet.
            nic(6, 9_999, 9_999),
        ];
        assert!(!counts(&filter) && !counts(&vpn) && !counts(&lo));
        assert!(!counts(&Interface { up: false, ..nic(7, 0, 0) }));
        let (down, up, link) = network(&before, &after, 2.0);
        assert_eq!((down, up, link), (1_250.0, 125.0, 1_000_000_000));
    }

    #[test]
    fn stats_gpu_engines_are_summed_over_processes_then_the_busiest_wins() {
        let a = [
            ("pid_100_luid_0x00000000_0x0000D6E1_phys_0_eng_0_engtype_3D", 44.0),
            ("pid_200_luid_0x00000000_0x0000D6E1_phys_0_eng_0_engtype_3D", 34.0),
            ("pid_300_luid_0x00000000_0x0000D6E1_phys_0_eng_0_engtype_", 13.0),
            ("pid_100_luid_0x00000000_0x0000D6E1_phys_0_eng_3_engtype_VideoDecode", 60.0),
            ("pid_400_luid_0x00000000_0x0000AAAA_phys_0_eng_0_engtype_3D", 5.0),
            ("not an engine", 99.0),
        ];
        let (busy, adapter) = gpu_busiest(a).unwrap();
        assert!((busy - 0.91).abs() < 1e-6, "{busy}");
        assert_eq!(adapter, "luid_0x00000000_0x0000D6E1_phys_0");
        // Never over 100 %.
        assert_eq!(gpu_busiest([("pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 80.0), ("pid_2_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 70.0)]).unwrap().0, 1.0);
        assert_eq!(gpu_busiest([]), None);
        assert_eq!(adapter_of("luid_0x00000000_0x0000D6E1_phys_0"), Some("luid_0x00000000_0x0000D6E1_phys_0"));
    }

    #[test]
    fn stats_numbers_read_like_task_manager() {
        assert_eq!(bytes_per_second(0.0), "0 KB/s");
        assert_eq!(bytes_per_second(980_000.0), "980 KB/s");
        assert_eq!(bytes_per_second(1_234_000.0), "1.2 MB/s");
        assert_eq!(bytes_per_second(12_400_000.0), "12 MB/s");
        assert_eq!(bytes_per_second(999_700.0), "1 MB/s");
        assert_eq!(bytes(11_200_000_000), "11 GB");
        assert_eq!(bytes(3_400_000_000), "3.4 GB");
        assert_eq!(bytes(2_000_000_000), "2 GB");
        assert_eq!(memory(12_025_908_428, 34_037_615_820), "11.2 of 31.7 GB");
        assert_eq!(percent(0.234), "23%");
        assert_eq!(percent(1.7), "100%");
    }
}
