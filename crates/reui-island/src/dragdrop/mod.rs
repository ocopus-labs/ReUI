//! OLE drag and drop for the file shelf (docs/PRD-file-shelf.md): the island as a drop
//! target (`target.rs`) and as a drag source, and the data objects both ways (`data.rs`).
//! The rules (when, which effect, what a drop becomes) are `features::drop`'s.

mod data;
pub mod target;

use reui_protocol::Drag;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Ole::{OleInitialize, RegisterDragDrop, RevokeDragDrop, DROPEFFECT};
use windows::Win32::UI::Shell::SHDoDragDrop;

use crate::features::drop::OFFERED;

/// Makes `hwnd` a drop target answering through `on`. OLE needs initialising on this
/// thread first (the composition apartment is already single-threaded, so this joins it).
pub fn register(hwnd: HWND, on: target::On) -> bool {
    unsafe {
        if let Err(e) = OleInitialize(None) {
            eprintln!("island: no drag and drop ({e})");
            return false;
        }
        match RegisterDragDrop(hwnd, &target::Target::new(hwnd, on, true)) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("island: can't take drops ({e})");
                false
            }
        }
    }
}

pub fn unregister(hwnd: HWND) {
    unsafe { _ = RevokeDragDrop(hwnd) };
}

/// Drags `drag` out of the island until the button comes up (Windows runs the loop; the
/// shell draws the files' image). Copy or link only: whatever it's dropped on gets a copy
/// or a shortcut, and the original stays where it is. Must not run while the pill is
/// borrowed: the island's own drop target and timers are called meanwhile.
pub fn drag_out(hwnd: HWND, drag: &Drag) {
    let data = match data::build(drag) {
        Ok(data) => data,
        Err(e) => return eprintln!("island: can't drag that out ({e})"),
    };
    _ = data::mark(&data);
    unsafe {
        if let Err(e) = SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT(OFFERED)) {
            eprintln!("island: the drag ended with {e}");
        }
    }
}
