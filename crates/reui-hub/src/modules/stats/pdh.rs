//! What only performance counters (PDH) know: the GPU's engines and memory, and how busy the
//! disks are. Counters are added by their English names (`PdhAddEnglishCounterW`), so a
//! German or Japanese Windows finds them too. A counter Windows doesn't have (no GPU
//! driver counters on a VM, the disk counters disabled with `diskperf -n`) is just left out.
//!
//! The GPU engine counter is the expensive one: it has an instance per process per engine,
//! re-listed on every collection, so it costs milliseconds where the rest cost microseconds
//! (see the PRD). It's only added when the card shows the GPU.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Performance::*;

/// Doubles, with `PDH_FMT_NOCAP100` (0x8000): values may pass 100 before we clamp them
/// ourselves, after summing.
const FORMAT: PDH_FMT = PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000);
const ERROR_SUCCESS: u32 = 0;

const GPU_ENGINES: &str = r"\GPU Engine(*)\Utilization Percentage";
const GPU_DEDICATED: &str = r"\GPU Adapter Memory(*)\Dedicated Usage";
/// Active time is what's left of idle time: `% Disk Time` can pass 100 on a queue of
/// requests, `% Idle Time` can't go below 0.
const DISK_IDLE: &str = r"\PhysicalDisk(_Total)\% Idle Time";

pub struct Query {
    query: PDH_HQUERY,
    gpu: Option<PDH_HCOUNTER>,
    gpu_memory: Option<PDH_HCOUNTER>,
    disk: Option<PDH_HCOUNTER>,
}

// The handles are only used from the thread that owns the query.
unsafe impl Send for Query {}

/// One collection: (instance, value) per GPU engine and per adapter's dedicated memory, and
/// the disks' active share (0..=1).
#[derive(Debug, Default)]
pub struct Values {
    pub engines: Vec<(String, f64)>,
    pub dedicated: Vec<(String, f64)>,
    pub disk: Option<f32>,
}

impl Query {
    /// `None` if PDH itself is unavailable or none of the wanted counters exist.
    pub fn open(gpu: bool, disk: bool) -> Option<Self> {
        let mut query = PDH_HQUERY::default();
        if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } != ERROR_SUCCESS {
            return None;
        }
        let add = |path: &str| {
            let mut counter = PDH_HCOUNTER::default();
            (unsafe { PdhAddEnglishCounterW(query, &HSTRING::from(path), 0, &mut counter) } == ERROR_SUCCESS).then_some(counter)
        };
        let q = Query { query, gpu: gpu.then(|| add(GPU_ENGINES)).flatten(), gpu_memory: gpu.then(|| add(GPU_DEDICATED)).flatten(), disk: disk.then(|| add(DISK_IDLE)).flatten() };
        if q.gpu.is_none() && q.gpu_memory.is_none() && q.disk.is_none() {
            return None;
        }
        // Rate counters need a first collection to have something to compare with.
        unsafe { PdhCollectQueryData(query) };
        Some(q)
    }

    pub fn collect(&mut self) -> Values {
        if unsafe { PdhCollectQueryData(self.query) } != ERROR_SUCCESS {
            return Values::default();
        }
        Values {
            engines: self.gpu.map(array).unwrap_or_default(),
            dedicated: self.gpu_memory.map(array).unwrap_or_default(),
            disk: self.disk.and_then(single).map(|idle| (1.0 - idle / 100.0).clamp(0.0, 1.0) as f32),
        }
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

fn single(counter: PDH_HCOUNTER) -> Option<f64> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    let status = unsafe { PdhGetFormattedCounterValue(counter, FORMAT, None, &mut value) };
    (status == ERROR_SUCCESS && valid(value.CStatus)).then_some(unsafe { value.Anonymous.doubleValue })
}

/// A wildcard counter's instances and their values.
fn array(counter: PDH_HCOUNTER) -> Vec<(String, f64)> {
    let format = FORMAT;
    let (mut size, mut count) = (0u32, 0u32);
    let status = unsafe { PdhGetFormattedCounterArrayW(counter, format, &mut size, &mut count, None) };
    if status != PDH_MORE_DATA || size == 0 {
        return Vec::new();
    }
    // The buffer holds the items followed by their names; u64s keep it aligned for the f64s.
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
    if unsafe { PdhGetFormattedCounterArrayW(counter, format, &mut size, &mut count, Some(items)) } != ERROR_SUCCESS {
        return Vec::new();
    }
    let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
    items
        .iter()
        .filter(|item| valid(item.FmtValue.CStatus))
        .map(|item| (unsafe { item.szName.to_string() }.unwrap_or_default(), unsafe { item.FmtValue.Anonymous.doubleValue }))
        .collect()
}

fn valid(status: u32) -> bool {
    status == PDH_CSTATUS_VALID_DATA || status == PDH_CSTATUS_NEW_DATA
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which PDH CPU counter `GetSystemTimes` agrees with: `cargo test -p reui-hub
    /// stats_cpu_counters -- --ignored --nocapture`. Load the machine meanwhile for a
    /// difference to show (Utility scales by frequency).
    #[test]
    #[ignore]
    fn stats_cpu_counters() {
        let mut query = PDH_HQUERY::default();
        assert_eq!(unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) }, ERROR_SUCCESS);
        let add = |path: &str| {
            let mut counter = PDH_HCOUNTER::default();
            assert_eq!(unsafe { PdhAddEnglishCounterW(query, &HSTRING::from(path), 0, &mut counter) }, ERROR_SUCCESS, "{path}");
            counter
        };
        let time = add(r"\Processor Information(_Total)\% Processor Time");
        let utility = add(r"\Processor Information(_Total)\% Processor Utility");
        unsafe { PdhCollectQueryData(query) };
        let mut before = crate::modules::stats::os::cpu().unwrap();
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            unsafe { PdhCollectQueryData(query) };
            let after = crate::modules::stats::os::cpu().unwrap();
            let ours = crate::modules::stats::maths::cpu_busy(before, after).unwrap() * 100.0;
            before = after;
            println!("GetSystemTimes {ours:5.1}   % Processor Time {:5.1}   % Processor Utility {:5.1}", single(time).unwrap_or(-1.0), single(utility).unwrap_or(-1.0));
        }
        unsafe { PdhCloseQuery(query) };
    }
}
