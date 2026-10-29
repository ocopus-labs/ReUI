//! What `--shots` needs from the window: drawing offscreen, and putting it in the states
//! the screenshots show (a page at rest, a pointer resting, a control used, a search
//! typed, a scroll).

use reui_design::tokens::prefs;
use reui_design::Color;

use super::motion::Motion;
use super::App;
use crate::pages::Page;
use crate::render;
use crate::ui::layout::Rect;
use crate::ui::{chrome, stage, Act, Focus, Hover, Preview, Target};

impl App {
    /// The frame as it is now, offscreen: BGRA pixels of the window's size.
    pub fn snapshot(&mut self) -> windows::core::Result<Vec<u8>> {
        let size = self.size;
        self.with_view(|renderer, view| renderer.draw_offscreen(size.0, size.1, prefs::color::CONTENT, |p| render::frame::draw(p, view)))
    }

    /// For screenshots: one stage alone, `width` DIPs wide and as tall as its island
    /// needs, the island settled. The clear colour is the stage's own (backdrop under its
    /// dim), so its rounded top corners disappear. Pixels, and their width and height.
    pub fn stage_snapshot(&mut self, preview: Preview, width: f32) -> windows::core::Result<(u32, u32, Vec<u8>)> {
        let height = stage::height(preview, width);
        let (w, h) = ((width * self.scale()).round() as u32, (height * self.scale()).round() as u32);
        let backdrop = crate::wallpaper::current();
        let under = backdrop.as_ref().map_or(prefs::color::STAGE, |b| b.color);
        let k = 1.0 - prefs::opacity::STAGE_DIM;
        let clear = Color { r: under.r * k, g: under.g * k, b: under.b * k, a: 1.0 };
        let pixels = self.renderer.draw_offscreen(w, h, clear, |p| render::preview::draw(p, preview, Rect::new(0.0, 0.0, width, height), backdrop.as_deref(), 1.0))?;
        Ok((w, h, pixels))
    }

    /// For screenshots: the page as it rests (the preview settled, no hover).
    pub fn show(&mut self, page: Page) {
        self.set_page(page);
        self.motion = Motion::new(true);
        self.hover = Hover::None;
    }

    /// For screenshots: the pointer resting at (`x`, `y`), its fill settled.
    pub fn rest_pointer(&mut self, x: f32, y: f32) {
        self.mouse_move(x, y);
        self.motion.step(&self.hover);
    }

    /// For screenshots: a control used.
    pub fn act_for_shot(&mut self, target: &Target, act: Act) {
        self.act(target, act);
    }

    /// For screenshots: `query` typed into the search field.
    pub fn type_search(&mut self, query: &str) {
        self.focus = Some(Focus::Search);
        self.set_query(query.to_string());
    }

    /// For screenshots: scroll to `y`.
    pub fn scroll_shot(&mut self, y: f32) {
        let max = chrome::max_scroll(self.layout.height, self.dips().1);
        self.motion.jump(y.min(max));
    }
}
