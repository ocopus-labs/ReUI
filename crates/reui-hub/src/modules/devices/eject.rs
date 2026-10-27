//! Safe removal: volume → disk number → the disk's device node → its first removable
//! ancestor (a card reader ejects as a whole, D-10) → `CM_Request_Device_EjectW` with a veto
//! buffer, so Windows shows no dialog of its own and tells us who's in the way. Windows
//! names the blocking app in the veto only for some veto kinds; for open handles (the usual
//! case) it logs the process in Kernel-PnP event 225, which is read back for "In use by …".
//! Blocking; runs on the blocking pool, only when the user presses Eject.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_PropertyW, CM_Request_Device_EjectW, CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CR_SUCCESS, DN_REMOVABLE,
    PNP_VETO_TYPE, PNP_VetoTypeUnknown, PNP_VetoWindowsApp, PNP_VetoWindowsService,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_InstanceId, DEVPROPTYPE};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::EventLog::{EvtClose, EvtNext, EvtQuery, EvtRender, EVT_HANDLE};
use windows::Win32::System::Ioctl::{GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER};
use windows::Win32::System::IO::DeviceIoControl;

use super::drives::open_volume;
use super::pnp;

/// Device nodes, as the ancestor walk sees them (the real tree, or a fake one in tests).
pub trait Tree {
    fn status(&self, node: u32) -> Option<u32>;
    fn parent(&self, node: u32) -> Option<u32>;
}

struct DevTree;

impl Tree for DevTree {
    fn status(&self, node: u32) -> Option<u32> {
        pnp::status(node)
    }
    fn parent(&self, node: u32) -> Option<u32> {
        pnp::parent(node)
    }
}

/// The node itself or its nearest ancestor Windows can eject.
pub fn removable_ancestor(tree: &impl Tree, node: u32) -> Option<u32> {
    let mut at = node;
    // A device tree is shallow; the bound only guards against a cycle in a broken one.
    for _ in 0..32 {
        if tree.status(at).is_some_and(|s| s & DN_REMOVABLE.0 != 0) {
            return Some(at);
        }
        at = tree.parent(at)?;
    }
    None
}

/// Why an eject didn't happen, as the card says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// "In use by Explorer", or "In use" when Windows doesn't say by what.
    InUse(Option<String>),
    /// The drive can't be found or ejected at all.
    Failed,
}

impl Refused {
    pub fn text(&self) -> String {
        match self {
            Refused::InUse(Some(app)) => format!("In use by {app}"),
            Refused::InUse(None) => "In use".into(),
            Refused::Failed => "Can't eject".into(),
        }
    }
}

/// "\Device\HarddiskVolume3\Windows\explorer.exe" → "Explorer".
pub fn app_name(path: &str) -> Option<String> {
    let file = path.rsplit(['\\', '/']).next()?.trim();
    let stem = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file);
    let mut chars = stem.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

/// The process an event-225 record names: the first `<Data>` value that is an executable.
pub fn app_from_event_xml(xml: &str) -> Option<String> {
    xml.split("<Data").skip(1).filter_map(|part| part.split_once('>').and_then(|(_, rest)| rest.split_once("</Data>")).map(|(value, _)| value.trim())).find(|v| v.to_ascii_lowercase().ends_with(".exe")).and_then(app_name)
}

/// What a veto says about who's in the way.
pub fn veto_reason(kind: PNP_VETO_TYPE, name: &str, logged_app: Option<String>) -> Refused {
    if kind == PNP_VetoWindowsApp || kind == PNP_VetoWindowsService {
        if let Some(app) = app_name(name) {
            return Refused::InUse(Some(app));
        }
    }
    Refused::InUse(logged_app)
}

pub fn eject(letter: char) -> Result<(), Refused> {
    let number = disk_number(letter).ok_or(Refused::Failed)?;
    let instance = disk_instance(number).ok_or(Refused::Failed)?;
    let node = pnp::locate(&instance).ok_or(Refused::Failed)?;
    let target = removable_ancestor(&DevTree, node).ok_or(Refused::Failed)?;
    let mut kind = PNP_VetoTypeUnknown;
    let mut name = [0u16; 260];
    let result = unsafe { CM_Request_Device_EjectW(target, Some(&mut kind), Some(&mut name), 0) };
    if result == CR_SUCCESS && kind == PNP_VetoTypeUnknown {
        return Ok(());
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    Err(veto_reason(kind, &String::from_utf16_lossy(&name[..len]), logged_blocker()))
}

fn device_number(handle: HANDLE) -> Option<u32> {
    let mut number = STORAGE_DEVICE_NUMBER::default();
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(handle, IOCTL_STORAGE_GET_DEVICE_NUMBER, None, 0, Some((&mut number as *mut STORAGE_DEVICE_NUMBER).cast()), size_of::<STORAGE_DEVICE_NUMBER>() as u32, Some(&mut returned), None)
    };
    _ = unsafe { CloseHandle(handle) };
    ok.is_ok().then_some(number.DeviceNumber)
}

fn disk_number(letter: char) -> Option<u32> {
    device_number(open_volume(letter)?)
}

/// The instance id of the disk with this number.
fn disk_instance(number: u32) -> Option<String> {
    let mut len = 0u32;
    if unsafe { CM_Get_Device_Interface_List_SizeW(&mut len, &GUID_DEVINTERFACE_DISK, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) } != CR_SUCCESS || len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    if unsafe { CM_Get_Device_Interface_ListW(&GUID_DEVINTERFACE_DISK, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) } != CR_SUCCESS {
        return None;
    }
    pnp::multi_sz(&buf).into_iter().find_map(|path| {
        let path = HSTRING::from(path);
        let handle = unsafe { CreateFileW(&path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }.ok()?;
        (device_number(handle)? == number).then(|| interface_instance(&path)).flatten()
    })
}

fn interface_instance(path: &HSTRING) -> Option<String> {
    let mut kind = DEVPROPTYPE::default();
    let mut buf = [0u16; 512];
    let mut size = size_of_val(&buf) as u32;
    let ok = unsafe { CM_Get_Device_Interface_PropertyW(path, &DEVPKEY_Device_InstanceId, &mut kind, Some(buf.as_mut_ptr().cast()), &mut size, 0) } == CR_SUCCESS;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    (ok && len > 0).then(|| String::from_utf16_lossy(&buf[..len]))
}

/// The app Windows logged as blocking a removal in the last few seconds, if it's readable.
fn logged_blocker() -> Option<String> {
    let query = HSTRING::from("*[System[(EventID=225) and TimeCreated[timediff(@SystemTime) <= 10000]]]");
    // EvtQueryChannelPath | EvtQueryReverseDirection: newest first.
    let results = unsafe { EvtQuery(None, &HSTRING::from("Microsoft-Windows-Kernel-PnP/Configuration"), &query, 0x1 | 0x200) }.ok()?;
    let mut events = [0isize; 1];
    let mut returned = 0u32;
    let xml = match unsafe { EvtNext(results, &mut events, 1000, 0, &mut returned) } {
        Ok(()) if returned == 1 => {
            let event = EVT_HANDLE(events[0]);
            let xml = render_xml(event);
            _ = unsafe { EvtClose(event) };
            xml
        }
        _ => None,
    };
    _ = unsafe { EvtClose(results) };
    app_from_event_xml(&xml?)
}

fn render_xml(event: EVT_HANDLE) -> Option<String> {
    let (mut used, mut count) = (0u32, 0u32);
    // EvtRenderEventXml; the first call sizes.
    _ = unsafe { EvtRender(None, event, 1, 0, None, &mut used, &mut count) };
    if used == 0 || used > 256 * 1024 {
        return None;
    }
    let mut buf = vec![0u16; (used as usize).div_ceil(2)];
    unsafe { EvtRender(None, event, 1, used, Some(buf.as_mut_ptr().cast()), &mut used, &mut count) }.ok()?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use windows::Win32::Devices::DeviceAndDriverInstallation::PNP_VetoOutstandingOpen;

    /// node → (status, parent).
    struct Fake(HashMap<u32, (u32, Option<u32>)>);

    impl Tree for Fake {
        fn status(&self, node: u32) -> Option<u32> {
            self.0.get(&node).map(|n| n.0)
        }
        fn parent(&self, node: u32) -> Option<u32> {
            self.0.get(&node).and_then(|n| n.1)
        }
    }

    #[test]
    fn devices_eject_walks_to_the_first_removable_ancestor() {
        let r = DN_REMOVABLE.0;
        // disk (5) → card reader slot (4) → reader (3, removable) → hub (2, removable) → root (1)
        let tree = Fake(HashMap::from([(5, (0x0A, Some(4))), (4, (0x0A, Some(3))), (3, (0x0A | r, Some(2))), (2, (r, Some(1))), (1, (0, None))]));
        assert_eq!(removable_ancestor(&tree, 5), Some(3));
        // A USB stick's disk node is itself removable.
        let stick = Fake(HashMap::from([(9, (r, Some(8))), (8, (0, None))]));
        assert_eq!(removable_ancestor(&stick, 9), Some(9));
        // Nothing removable up to the root, a missing node, and a cycle.
        let fixed = Fake(HashMap::from([(7, (0, Some(6))), (6, (0, None))]));
        assert_eq!(removable_ancestor(&fixed, 7), None);
        assert_eq!(removable_ancestor(&fixed, 42), None);
        let cycle = Fake(HashMap::from([(1, (0, Some(2))), (2, (0, Some(1)))]));
        assert_eq!(removable_ancestor(&cycle, 1), None);
    }

    #[test]
    fn devices_eject_veto_reasons() {
        assert_eq!(app_name(r"\Device\HarddiskVolume3\Windows\explorer.exe").as_deref(), Some("Explorer"));
        assert_eq!(app_name("Code.exe").as_deref(), Some("Code"));
        assert_eq!(app_name(""), None);
        let xml = "<Event><EventData><Data Name='DeviceInstance'>USBSTOR\\Disk&amp;Ven_X\\1</Data><Data Name='ProcessId'>9856</Data><Data Name='ProcessName'>\\Device\\HarddiskVolume3\\Windows\\explorer.exe</Data></EventData></Event>";
        assert_eq!(app_from_event_xml(xml).as_deref(), Some("Explorer"));
        assert_eq!(app_from_event_xml("<Event><EventData><Data Name='X'>1</Data></EventData></Event>"), None);

        assert_eq!(veto_reason(PNP_VetoWindowsApp, r"C:\Program Files\App\notepad.exe", None), Refused::InUse(Some("Notepad".into())));
        // An open handle names a device, not an app: the logged app, or just "In use".
        assert_eq!(veto_reason(PNP_VetoOutstandingOpen, r"STORAGE\Volume\{x}", Some("Explorer".into())).text(), "In use by Explorer");
        assert_eq!(veto_reason(PNP_VetoOutstandingOpen, r"STORAGE\Volume\{x}", None).text(), "In use");
        assert_eq!(Refused::Failed.text(), "Can't eject");
    }

    /// Reads the latest eject veto Windows logged (read only). Run with `--ignored`.
    #[test]
    #[ignore = "reads the machine's event log"]
    fn devices_eject_log_reads() {
        println!("{:?}", logged_blocker());
    }
}
