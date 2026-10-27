//! No opening on drag-over (boring.notch #1530): while a mouse button that went down away
//! from the island is held (dragging a browser tab, a file, a selection), passing over the
//! island must neither open it nor make it take clicks. Only after the button comes up,
//! and the pointer then rests on it, does hover intent count again.

/// Mouse buttons, as bits.
pub const LEFT: u8 = 1;
pub const RIGHT: u8 = 2;
pub const MIDDLE: u8 = 4;

#[derive(Default, Debug)]
pub struct DragGuard {
    /// Buttons held that went down away from the island.
    away: u8,
}

impl DragGuard {
    /// A button went down; `on_island` when that was on the pill or its bubble (the
    /// island's own clicks and drags).
    pub fn press(&mut self, button: u8, on_island: bool) {
        if on_island {
            self.away &= !button;
        } else {
            self.away |= button;
        }
    }

    pub fn release(&mut self, button: u8) {
        self.away &= !button;
    }

    /// The buttons actually down now (`held`): an up we never saw (a secure desktop, a
    /// hook that timed out) doesn't leave the island deaf for good.
    pub fn sync(&mut self, held: u8) {
        self.away &= held;
    }

    /// Something is being dragged over the screen: hover intent doesn't count.
    pub fn blocks(&self) -> bool {
        self.away != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_from_elsewhere_blocks_until_released() {
        let mut guard = DragGuard::default();
        guard.press(LEFT, false);
        assert!(guard.blocks());
        guard.press(RIGHT, false);
        guard.release(LEFT);
        assert!(guard.blocks(), "right still held");
        guard.release(RIGHT);
        assert!(!guard.blocks());
    }

    #[test]
    fn the_islands_own_presses_never_block() {
        let mut guard = DragGuard::default();
        guard.press(LEFT, true);
        assert!(!guard.blocks());
    }

    #[test]
    fn a_missed_release_is_caught_up() {
        let mut guard = DragGuard::default();
        guard.press(LEFT, false);
        guard.sync(LEFT);
        assert!(guard.blocks());
        guard.sync(0);
        assert!(!guard.blocks());
    }
}
