//! The expanded presentation: the session strip, the header (leading, text column,
//! trailing, pager) and the dispatch of each full-width row to its drawer in `rows`.


use reui_design::layout::header::{self, Hero};
use reui_design::layout::{self, rows};
use reui_design::tokens::{component, island, opacity, radius, space};
use reui_design::Color;
use crate::features::clock;
use reui_protocol::{Button, ButtonContent, CardSize, Element, ExpandedView, Headline, Row};
use windows::core::Result;
use windows::UI::Composition::CompositionDrawingSurface;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows_numerics::{Matrix3x2, Vector2};

use crate::render::{card, d2d, Align, ExpandedLayout, Hit, Interaction, Renderer, Slot, Thumb};
use crate::render::expanded_surface;
use crate::render::text::{clock, clock_left};
use crate::render::elements::shown_level;
use crate::render::rows::Meter;

impl Renderer {
    /// Draws the expanded presentation and returns its layout: height, pressable regions,
    /// and where live bars go (the pill draws those).
    pub fn draw_expanded(&self, surface: &CompositionDrawingSurface, view: Option<&ExpandedView>, accent: Color, with: &Interaction) -> Result<ExpandedLayout> {
        let mut height = island::expanded::MIN_HEIGHT;
        let mut hits = Vec::new();
        let mut bars = None;
        let mut width = island::expanded::WIDTH;
        let mut block_max = 0.0;
        let mut choice_spans = None;
        let max_height = card(view.map_or(CardSize::Regular, |v| v.size)).max_height(&self.look);
        self.paint(surface, |rt| {
            let Some(view) = view else { return Ok(()) };
            let size = card(view.size);
            let w = size.width();
            width = w;
            self.width.set(w);
            self.size.set(view.size);
            let roomy = size.roomy();
            // Normal views sit centred in the wide surface.
            let dx = (expanded_surface(&self.look).0 - w) / 2.0;
            unsafe {
                let mut transform = Matrix3x2::default();
                rt.GetTransform(&mut transform);
                rt.SetTransform(&Matrix3x2 { M31: transform.M31 + dx * self.scale, ..transform });
            }
            let inset = self.look.inset();
            if view.tint {
                self.wash(rt, w, accent)?;
            }

            // The session strip comes first, and the pager sits on it; the header follows.
            let mut top = inset;
            if let Some(context) = &view.context {
                let h = self.context_height(context)?;
                let right = match &view.pager {
                    Some(pager) => self.pager(rt, pager, w - inset, inset + h / 2.0, &mut hits)? - space::SM,
                    None => w - inset,
                };
                self.context(rt, context, inset, right, accent)?;
                top += h + island::expanded::CONTEXT_GAP;
            }

            let (leading, trailing) = (shown_level(&view.leading, with.level), shown_level(&view.trailing, with.level));
            let (lw, lh) = self.measure(&leading, Slot::Leading)?;

            // A lone row of glyph buttons belongs beside the hero, not under it. So does a
            // moment's lone text button (iOS's Silent Mode: "Unmute" beside the text), when
            // the hero fits beside it; otherwise it's a full-width capsule under the text.
            let mut inline: Option<&[Button]> = match view.rows.as_slice() {
                [Row::Buttons(b)] if b.iter().all(|b| matches!(b.content, ButtonContent::Glyph(_))) => Some(b),
                [Row::Buttons(b)] if view.context.is_some() && b.len() == 1 && matches!(b[0].content, ButtonContent::Text(_)) => Some(b),
                _ => None,
            };
            let mut inline_text = match inline {
                Some([Button { content: ButtonContent::Text(t), .. }]) => Some(header::inline_text_width(self.text_width(t, &self.fonts.button)?)),
                _ => None,
            };
            if let (Some(width), Headline::Text(hero)) = (inline_text, &view.headline) {
                if self.text_width(hero, &self.fonts.headline)? > header::columns(&self.look, w, lw, width).1 {
                    (inline, inline_text) = (None, None);
                }
            }
            let rows: &[Row] = if inline.is_some() { &[] } else { &view.rows };

            let (tw, th) = match (inline, inline_text) {
                (_, Some(width)) => (width, component::button::INLINE),
                (Some(b), None) => (header::inline_width(b.len()), component::button::INLINE),
                (None, None) => self.measure(&trailing, Slot::Trailing)?,
            };
            let (text_left, text_w) = header::columns(&self.look, w, lw, tw);

            let line_gap = island::expanded::LINE_GAP;
            let hero = match &view.headline {
                Headline::Value(_) | Headline::Clock { .. } => Hero::Value,
                Headline::Text(_) => Hero::Text,
                Headline::Code(_) => Hero::Code,
            };
            // Only a roomy card's detail may wrap, so only its width is worth measuring.
            let measured = match &view.detail {
                Some(d) if roomy => Some(self.text_width(d, &self.fonts.detail)?),
                Some(_) => Some(0.0),
                None => None,
            };
            let detail_lines = header::detail_lines(measured, roomy, text_w);
            let text = header::Text::new(view.eyebrow.is_some(), hero, detail_lines);
            let (eyebrow_h, hero_h, detail_h) = (text.eyebrow, text.hero, text.detail);
            // A view of only rows (e.g. clipboard cards) has no header at all.
            let headless = view.context.is_none()
                && matches!(view.leading, Element::Empty)
                && matches!(view.trailing, Element::Empty)
                && inline.is_none()
                && view.eyebrow.is_none()
                && view.detail.is_none()
                && view.pager.is_none()
                && matches!(&view.headline, Headline::Text(t) if t.is_empty());
            let head = header::header(top, lh, th, text.height(), headless);
            let (header_h, mid, text_top) = (head.height, head.mid, head.text_top);

            self.element(rt, &leading, Slot::Leading, inset, mid, Align::Start, accent)?;
            if let Some(open) = view.open.as_deref().filter(|_| lw > 0.0) {
                hits.push(Hit::new(inset, mid - lh / 2.0, inset + lw, mid + lh / 2.0, open));
            }
            bars = match (&view.leading, &view.trailing, inline) {
                (Element::Bars { live }, _, _) => Some((inset, mid, *live)),
                (_, Element::Bars { live }, None) => Some((w - inset - tw, mid, *live)),
                _ => None,
            };
            match inline {
                Some([button]) if inline_text.is_some() => {
                    let size = component::button::INLINE;
                    self.button(rt, button, w - inset - tw, mid - size / 2.0, tw, size, accent, &mut hits)?;
                }
                Some(buttons) => {
                    let size = component::button::INLINE;
                    for (i, button) in buttons.iter().enumerate() {
                        let x = header::inline_x(&self.look, w, buttons.len(), i);
                        self.button(rt, button, x, mid - size / 2.0, size, size, accent, &mut hits)?;
                    }
                }
                None => self.element(rt, &trailing, Slot::Trailing, w - inset, mid, Align::End, accent)?,
            }

            // The source switcher sits at the top right, on the first line of text, which
            // makes room for it.
            let first_line = text.first_line();
            let first_w = match view.pager.as_ref().filter(|_| view.context.is_none()) {
                Some(pager) => self.pager(rt, pager, w - inset, text_top + first_line / 2.0, &mut hits)? - space::SM - text_left,
                None => text_w,
            };

            // Text column: eyebrow (accent) → hero (primary) → detail (secondary).
            let mut y = text_top;
            let mut line_w = first_w.min(text_w);
            if let Some(eyebrow) = &view.eyebrow {
                self.text(rt, eyebrow, &self.fonts.eyebrow, text_left, y, line_w, Align::Start, accent)?;
                y += eyebrow_h;
                line_w = text_w;
            }
            match &view.headline {
                Headline::Value(s) => self.text(rt, s, &self.fonts.value, text_left, y, line_w, Align::Start, self.pal().label_primary)?,
                Headline::Clock { clock: c, tenths } => {
                    let tone = if c.running { self.pal().label_primary } else { self.pal().label_secondary };
                    self.text(rt, &clock::text(c, reui_protocol::unix_ms(), *tenths), &self.fonts.value, text_left, y, line_w, Align::Start, tone)?
                }
                Headline::Text(s) => self.text(rt, s, &self.fonts.headline, text_left, y, line_w, Align::Start, self.pal().label_primary)?,
                Headline::Code(s) => {
                    let (px, py) = (component::chip::PADDING_X, component::chip::PADDING_Y);
                    let chip_w = (self.text_width(s, &self.fonts.code)? + 2.0 * px).min(line_w);
                    unsafe { self.round_rect(rt, text_left, y, text_left + chip_w, y + hero_h, radius::CHIP.min(self.concentric()), self.pal().fill_tertiary)? };
                    self.text(rt, s, &self.fonts.code, text_left + px, y + py, chip_w - 2.0 * px, Align::Start, self.pal().label_primary)?;
                }
            }
            y += hero_h;
            if let Some(detail) = &view.detail {
                if detail_lines > 1.0 {
                    self.text_block(rt, detail, &self.fonts.detail_wrap, text_left, y + line_gap, text_w, detail_h - line_gap, self.pal().label_secondary)?;
                } else {
                    self.text(rt, detail, &self.fonts.detail, text_left, y + line_gap, text_w, Align::Start, self.pal().label_secondary)?;
                }
            }

            // Where the timeline is now as shown (a drag's preview or a held seek win), for
            // the bar and the lyrics.
            let now = reui_protocol::unix_ms();
            let shown_at = |position_ms: u64, length_ms: u64, playing: bool, at_unix_ms: u64, rate: f32| {
                let reported = || Row::timeline_position(position_ms, length_ms, playing, at_unix_ms, rate, now);
                with.scrub.or_else(|| with.hold.map(|h| h.position(length_ms, playing, rate, now))).unwrap_or_else(reported).min(length_ms)
            };
            let track_at = rows.iter().find_map(|r| match r {
                Row::Timeline { position_ms, length_ms, playing, at_unix_ms, rate, .. } if *length_ms > 0 => Some(shown_at(*position_ms, *length_ms, *playing, *at_unix_ms, *rate)),
                _ => None,
            });

            // Full-width rows. A progress bar with times followed by icon-only controls shares
            // one line under the bar when the controls fit between the times; otherwise the
            // times flank the bar.
            let mut y = top + header_h;
            let mut i = 0;
            while i < rows.len() {
                y += layout::row_gap(&self.look, i, headless);
                let meter = match &rows[i] {
                    Row::Progress { fraction, leading, trailing } => Some(Meter { fraction: Some(*fraction), leading: leading.clone(), trailing: trailing.clone(), seek: None }),
                    // Live: no bar, just a quiet fact where the times would be.
                    Row::Timeline { length_ms: 0, .. } => Some(Meter { fraction: None, leading: Some("LIVE".into()), trailing: None, seek: None }),
                    Row::Timeline { position_ms, length_ms, playing, at_unix_ms, seekable, rate } => {
                        let at = shown_at(*position_ms, *length_ms, *playing, *at_unix_ms, *rate);
                        Some(Meter {
                            fraction: Some(at as f32 / *length_ms as f32),
                            leading: Some(clock(at)),
                            trailing: Some(format!("-{}", clock_left(length_ms - at))),
                            seek: seekable.then_some(*length_ms),
                        })
                    }
                    Row::Buttons(_) | Row::Items(_) | Row::Search { .. } | Row::Tabs(_) | Row::Note(_) | Row::Slider { .. } | Row::Lyrics(_) | Row::Choices { .. } | Row::Block { .. } => None,
                };
                let controls = match rows.get(i + 1) {
                    Some(Row::Buttons(b)) if b.iter().all(|b| matches!(b.content, ButtonContent::Glyph(_))) => Some(b),
                    _ => None,
                };
                let thumb = with.thumb == Some(Thumb::Seek);
                match (meter, controls) {
                    (Some(m), Some(buttons)) if m.has_times() && self.fits_between(buttons, &m)? => {
                        if let Some(fraction) = m.fraction {
                            y += self.meter(rt, (inset, w - inset), y, fraction, m.seek, thumb, accent, &mut hits)? + rows::UNDER_BAR;
                        }
                        let row_h = self.button_row(rt, buttons, y, accent, &mut hits)?;
                        self.captions(rt, m.leading.as_deref(), m.trailing.as_deref(), y + row_h / 2.0)?;
                        y += row_h;
                        i += 2;
                        continue;
                    }
                    (Some(m @ Meter { fraction: Some(fraction), .. }), Some(_)) if m.has_times() => {
                        // Times at the bar's ends, on its line.
                        let line = self.fonts.caption.style.line_height;
                        let lead = self.caption_width(m.leading.as_deref())?;
                        let trail = self.caption_width(m.trailing.as_deref())?;
                        let span = rows::bar_between_times(&self.look, w, lead, trail);
                        self.meter(rt, span, y + (line - component::progress::HEIGHT) / 2.0, fraction, m.seek, thumb, accent, &mut hits)?;
                        self.captions(rt, m.leading.as_deref(), m.trailing.as_deref(), y + line / 2.0)?;
                        y += line;
                    }
                    (Some(m), _) => {
                        if let Some(fraction) = m.fraction {
                            y += self.meter(rt, (inset, w - inset), y, fraction, m.seek, thumb, accent, &mut hits)?;
                            if m.has_times() {
                                y += rows::UNDER_BAR;
                            }
                        }
                        if m.has_times() {
                            let line = self.fonts.caption.style.line_height;
                            self.captions(rt, m.leading.as_deref(), m.trailing.as_deref(), y + line / 2.0)?;
                            y += line;
                        }
                    }
                    (None, _) => match &rows[i] {
                        Row::Buttons(buttons) => y += self.button_row(rt, buttons, y, accent, &mut hits)?,
                        Row::Items(items) => y += self.cards(rt, items, y, accent, with, &mut hits)?,
                        Row::Search { query, buttons, placeholder, glyph } => y += self.search(rt, with.query.unwrap_or(query), placeholder.as_deref(), *glyph, with.keyboard, buttons, y, accent, &mut hits)?,
                        Row::Tabs(tabs) => y += self.tabs(rt, tabs, y, &mut hits)?,
                        // Standing in for the cards under a search field, it keeps their
                        // height, so the panel doesn't jump while typing.
                        Row::Note(text) => {
                            let searchable = rows.iter().any(|r| matches!(r, Row::Search { .. }));
                            y += self.note(rt, text, y, rows::note_height(searchable))?
                        }
                        Row::Slider { leading, value, action, trailing } => {
                            let set = with.slider.filter(|(a, ..)| a == action);
                            let value = set.map_or(*value, |(_, v, _)| v);
                            let dragging = set.is_some_and(|(.., dragging)| dragging);
                            let thumb = with.thumb == Some(Thumb::Slider);
                            y += self.slider(rt, leading, value, action, trailing.as_ref(), y, thumb, dragging, accent, &mut hits)?
                        }
                        Row::Lyrics(lines) => y += self.lyrics(rt, lines, track_at, y, with.lyric)?,
                        Row::Choices { options, multi } => {
                            let list = self.choices(rt, options, *multi, with.choice, y, with.block_scroll, accent, &mut hits)?;
                            block_max = list.max_scroll;
                            choice_spans = Some((list.spans, list.height));
                            y += list.height;
                        }
                        Row::Block { text, kind } => {
                            let (h, max) = self.block(rt, text, *kind, y, with.block_scroll, accent, &mut hits)?;
                            block_max = max;
                            y += h;
                        }
                        Row::Progress { .. } | Row::Timeline { .. } => {}
                    },
                }
                i += 1;
            }
            // Alert-style buttons at the bottom keep the same distance from the bottom edge
            // as from the sides.
            let text_buttons_last = matches!(rows.last(), Some(Row::Buttons(b)) if b.iter().any(|b| matches!(b.content, ButtonContent::Text(_))));
            let natural = y + size.bottom(&self.look, text_buttons_last);
            if natural > max_height {
                // Clamping would eat the bottom padding; make it visible instead.
                eprintln!("island: expanded content is {natural} tall, over max-height {max_height}");
            }
            height = size.height(&self.look, natural);
            // Hits and bar positions are in surface coordinates.
            for hit in hits.iter_mut() {
                hit.left += dx;
                hit.right += dx;
                if let Some((l, r, length)) = hit.seek {
                    hit.seek = Some((l + dx, r + dx, length));
                }
                if let Some((l, r)) = hit.slider {
                    hit.slider = Some((l + dx, r + dx));
                }
            }
            bars = bars.map(|(x, mid, live)| (x + dx, mid, live));
            Ok(())
        })?;
        let radius = self.card_radius(view.map_or(CardSize::Regular, |v| v.size));
        Ok(ExpandedLayout { width, height, radius, hits, bars, block_max, choices: choice_spans })
    }

    /// The accent washed in from the top-left corner, fading out by `tint-extent` of the
    /// width (a gradient, no blur). Drawn over the whole surface; the pill's shape clips it.
    fn wash(&self, rt: &ID2D1RenderTarget, w: f32, accent: Color) -> Result<()> {
        let radius = island::expanded::TINT_EXTENT * w * self.scale;
        let a = opacity::TINT_WASH;
        unsafe {
            // A softened falloff, so the wash has no visible edge.
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: d2d(accent.with_alpha(a)) },
                D2D1_GRADIENT_STOP { position: 0.45, color: d2d(accent.with_alpha(a * 0.5)) },
                D2D1_GRADIENT_STOP { position: 1.0, color: d2d(accent.with_alpha(0.0)) },
            ];
            let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: self.point(0.0, 0.0), gradientOriginOffset: Vector2::new(0.0, 0.0), radiusX: radius, radiusY: radius };
            let brush = rt.CreateRadialGradientBrush(&props, None, &collection)?;
            rt.FillRectangle(&self.rect(0.0, 0.0, w, expanded_surface(&self.look).1), &brush);
        }
        Ok(())
    }
}
