//! The island as an OLE drop target (docs/PRD-file-shelf.md §5.1). Windows calls it only
//! while the island is hit-testable under a drag, which the pill arranges near its shape;
//! this side answers with an effect (Copy, else Link, never Move), keeps the shell's drag
//! image going over the island (`IDropTargetHelper`), and on the drop reads the data once
//! and hands it on. What the island does with each step is the caller's (`On`), so the
//! plumbing is tested by calling it directly.

use std::cell::Cell;

use reui_protocol::Dropped;
use windows::core::{implement, Ref, Result};
use windows::Win32::Foundation::{HWND, POINT, POINTL};
use windows::Win32::System::Com::{CoCreateInstance, IDataObject, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Ole::{IDropTarget, IDropTarget_Impl, DROPEFFECT};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{CLSID_DragDropHelper, IDropTargetHelper};

use super::data;
use crate::features::drop;

/// What a drag over the island is doing, in screen pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A drag with something the shelf can hold came over the island.
    Enter(POINT),
    Over(POINT),
    Leave,
    Dropped(Vec<Dropped>),
}

/// The island's answer to each step: whether it takes the drag at that point.
pub type On = Box<dyn Fn(Event) -> bool>;

#[implement(IDropTarget)]
pub struct Target {
    hwnd: HWND,
    helper: Option<IDropTargetHelper>,
    on: On,
    /// The drag offers something holdable (decided once, on entering).
    holdable: Cell<bool>,
}

impl Target {
    /// `helper`: show the shell's drag image over the island (off in tests: no window).
    pub fn new(hwnd: HWND, on: On, helper: bool) -> IDropTarget {
        let helper = helper.then(|| unsafe { CoCreateInstance::<_, IDropTargetHelper>(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok() }).flatten();
        Target { hwnd, helper, on, holdable: Cell::new(false) }.into()
    }

    fn answer(&self, accept: bool, allowed: *mut DROPEFFECT) -> DROPEFFECT {
        let effect = DROPEFFECT(if accept && !allowed.is_null() { drop::effect(unsafe { (*allowed).0 }) } else { 0 });
        if !allowed.is_null() {
            unsafe { *allowed = effect };
        }
        effect
    }
}

fn point(pt: &POINTL) -> POINT {
    POINT { x: pt.x, y: pt.y }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let holdable = data.ok().is_ok_and(data::holdable);
        self.holdable.set(holdable);
        let accept = holdable && (self.on)(Event::Enter(point(pt)));
        let shown = self.answer(accept, effect);
        if let (Some(helper), Ok(data)) = (&self.helper, data.ok()) {
            unsafe { _ = helper.DragEnter(self.hwnd, data, &point(pt), shown) };
        }
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let accept = self.holdable.get() && (self.on)(Event::Over(point(pt)));
        let shown = self.answer(accept, effect);
        if let Some(helper) = &self.helper {
            unsafe { _ = helper.DragOver(&point(pt), shown) };
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        if self.holdable.replace(false) {
            (self.on)(Event::Leave);
        }
        if let Some(helper) = &self.helper {
            unsafe { _ = helper.DragLeave() };
        }
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let accept = self.holdable.replace(false) && (self.on)(Event::Over(point(pt)));
        let items = match data.ok() {
            Ok(data) if accept => drop::dropped(data::read(data)),
            _ => Vec::new(),
        };
        let taken = !items.is_empty() && (self.on)(Event::Dropped(items));
        if !taken {
            // Nothing came of it: the target folds as on leaving.
            (self.on)(Event::Leave);
        }
        let shown = self.answer(taken, effect);
        if let (Some(helper), Ok(data)) = (&self.helper, data.ok()) {
            unsafe { _ = helper.Drop(data, &point(pt), shown) };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::cell::RefCell;

    use reui_protocol::Drag;
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};

    use super::*;
    use crate::dragdrop::data::tests::{com, temp_files};

    fn target(accept_at: i32) -> (IDropTarget, Rc<RefCell<Vec<Event>>>) {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        let on: On = Box::new(move |e| {
            let ok = match &e {
                Event::Enter(p) | Event::Over(p) => p.x < accept_at,
                _ => true,
            };
            log.borrow_mut().push(e);
            ok
        });
        (Target::new(HWND::default(), on, false), seen)
    }

    #[test]
    fn files_dropped_on_the_island_are_sent_and_never_moved() {
        com();
        let (dir, paths) = temp_files("target", &["a.txt", r"other\b.txt"]);
        let data = data::build(&Drag::Files(paths.clone())).unwrap();
        let (t, seen) = target(100);
        let mut effect = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
        unsafe { t.DragEnter(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 10, y: 5 }, &mut effect).unwrap() };
        assert_eq!(effect, DROPEFFECT_COPY);
        let mut effect = DROPEFFECT_MOVE | DROPEFFECT_LINK;
        unsafe { t.DragOver(MODIFIERKEYS_FLAGS(0), POINTL { x: 20, y: 5 }, &mut effect).unwrap() };
        assert_eq!(effect, DROPEFFECT_LINK, "never Move");
        let mut effect = DROPEFFECT_COPY;
        unsafe { t.Drop(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 20, y: 5 }, &mut effect).unwrap() };
        assert_eq!(effect, DROPEFFECT_COPY);
        let events = seen.borrow();
        assert_eq!(events[0], Event::Enter(POINT { x: 10, y: 5 }));
        assert_eq!(events.last(), Some(&Event::Dropped(vec![Dropped::Files(paths)])));
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn away_from_the_target_and_reuis_own_drags_are_refused() {
        com();
        let data = data::build(&Drag::Text("hello".into())).unwrap();
        let (t, seen) = target(100);
        let mut effect = DROPEFFECT_COPY;
        unsafe { t.DragEnter(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 500, y: 5 }, &mut effect).unwrap() };
        assert_eq!(effect, DROPEFFECT(0), "outside the target");
        let mut effect = DROPEFFECT_COPY;
        unsafe { t.Drop(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 500, y: 5 }, &mut effect).unwrap() };
        assert_eq!(effect, DROPEFFECT(0));
        assert!(!seen.borrow().iter().any(|e| matches!(e, Event::Dropped(_))));

        data::mark(&data).unwrap();
        let (t, seen) = target(100);
        let mut effect = DROPEFFECT_COPY;
        unsafe { t.DragEnter(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 10, y: 5 }, &mut effect).unwrap() };
        assert_eq!(effect, DROPEFFECT(0));
        assert!(seen.borrow().is_empty(), "the island never hears of its own drag");
    }

    #[test]
    fn text_dropped_is_sent_as_text() {
        com();
        let data = data::build(&Drag::Text("notes".into())).unwrap();
        let (t, seen) = target(100);
        let mut effect = DROPEFFECT_COPY;
        unsafe {
            t.DragEnter(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 1, y: 1 }, &mut effect).unwrap();
            t.Drop(&data, MODIFIERKEYS_FLAGS(0), POINTL { x: 1, y: 1 }, &mut effect).unwrap();
        }
        assert_eq!(seen.borrow().last(), Some(&Event::Dropped(vec![Dropped::Text("notes".into())])));
    }
}
