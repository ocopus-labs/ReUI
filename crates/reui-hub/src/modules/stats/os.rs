//! The cheap reads, each a single call that costs microseconds: CPU times
//! (`GetSystemTimes`), memory (`GlobalMemoryStatusEx`) and the network interfaces'
//! counters (`GetIfTable2`). Called only from the sampler's thread.

use windows::Win32::Foundation::FILETIME;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

use super::maths::{CpuTimes, Interface};

pub fn cpu() -> Option<CpuTimes> {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Some(CpuTimes { idle: ticks(idle), kernel: ticks(kernel), user: ticks(user) })
}

/// (in use, total) physical memory in bytes. "In use" is total minus available, Task
/// Manager's figure (the standby cache counts as available).
pub fn memory() -> Option<(u64, u64)> {
    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some((status.ullTotalPhys.saturating_sub(status.ullAvailPhys), status.ullTotalPhys))
}

/// Every interface's counters and what decides whether it counts.
pub fn interfaces() -> Option<Vec<Interface>> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    unsafe {
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return None;
        }
        let count = (*table).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), count);
        let list = rows
            .iter()
            .map(|r| {
                // The flags are a C bitfield: HardwareInterface is bit 0, FilterInterface bit 1.
                let flags = r.InterfaceAndOperStatusFlags._bitfield;
                Interface {
                    luid: r.InterfaceLuid.Value,
                    kind: r.Type,
                    up: r.OperStatus == IfOperStatusUp,
                    hardware: flags & 1 != 0,
                    filter: flags & 2 != 0,
                    rx: r.InOctets,
                    tx: r.OutOctets,
                    link_bps: r.ReceiveLinkSpeed.max(r.TransmitLinkSpeed),
                }
            })
            .collect();
        FreeMibTable(table as *const _);
        Some(list)
    }
}

/// Task Manager, from the card's header: everything the card leaves out (which process,
/// per core, history) is one click away. Through the shell, not `CreateProcess`, because
/// Task Manager asks to run elevated and only the shell handles that. Blocking; best effort.
pub fn open_task_manager() {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    unsafe {
        _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from("taskmgr.exe"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if result.0 as isize <= 32 {
            eprintln!("stats: can't open Task Manager ({})", result.0 as isize);
        }
    }
}
