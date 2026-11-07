//! Anti-aliased digit readouts drawn straight onto the display, bypassing Ratatui (whose
//! `MONO_7X14` cell font and `BigText` block glyphs can only produce jagged pixel digits).
//!
//! Glyphs are rasterized from `assets/fonts/` at build time (see `render_smooth_fonts` in
//! `build.rs`) to 4-bit coverage bitmaps in flash, and blended between a foreground and the
//! background color while streaming each glyph box to the display. Every glyph box covers its
//! full cell including background, so a redraw never needs a separate clear (no flicker), and
//! [`SmoothField`] only redraws the cells whose character or color actually changed.

use mousefood::ColorTheme;
use mousefood::embedded_graphics::pixelcolor::{Rgb565, Rgb888};
use mousefood::embedded_graphics::prelude::{DrawTarget, Point, RgbColor, Size};
use mousefood::embedded_graphics::primitives::Rectangle;
use ratatui::style::Color;

include!(concat!(env!("OUT_DIR"), "/smooth_fonts.rs"));

/// Display background behind smooth digits — the same black Mousefood clears the screen to.
pub const BACKGROUND: Rgb565 = Rgb565::BLACK;

pub struct Glyph {
    pub ch: char,
    pub width: u16,
    /// `width * height` 4-bit coverage values, row-major, two pixels per byte (high nibble first).
    pub coverage: &'static [u8],
}

pub struct GlyphSet {
    pub height: u16,
    pub glyphs: &'static [Glyph],
}

impl GlyphSet {
    fn glyph(&'static self, ch: char) -> Option<&'static Glyph> {
        self.glyphs.iter().find(|g| g.ch == ch)
    }

    /// Box width of `ch`; characters missing from the set render as a blank digit-wide cell.
    fn width_of(&'static self, ch: char) -> u16 {
        self.glyph(ch)
            .or_else(|| self.glyph('0'))
            .map_or(0, |g| g.width)
    }
}

/// A fixed display area showing one centered line of smooth text, remembering what it last
/// drew so an update only touches the glyph cells that changed.
pub struct SmoothField {
    set: &'static GlyphSet,
    drawn: Option<DrawnText>,
}

struct DrawnText {
    area: Rectangle,
    color: Rgb565,
    /// Each drawn cell's character and left x coordinate.
    cells: Vec<(char, i32)>,
}

impl SmoothField {
    pub const fn new(set: &'static GlyphSet) -> Self {
        Self { set, drawn: None }
    }

    /// Forget what is on screen, so the next [`draw`](Self::draw) repaints the whole area —
    /// call whenever something else (screen switch, full clear) may have drawn over it.
    pub fn invalidate(&mut self) {
        self.drawn = None;
    }

    /// Show `text` centered in `area` (in display pixels) in `color`.
    pub fn draw<D>(
        &mut self,
        display: &mut D,
        area: Rectangle,
        text: &str,
        color: Rgb565,
    ) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let set = self.set;
        let text_width: i32 = text.chars().map(|ch| i32::from(set.width_of(ch))).sum();
        let text_x = area.top_left.x + (area.size.width as i32 - text_width).max(0) / 2;
        let glyph_y = area.top_left.y + (area.size.height as i32 - i32::from(set.height)) / 2;

        let mut cells = Vec::with_capacity(text.len());
        let mut x = text_x;
        for ch in text.chars() {
            cells.push((ch, x));
            x += i32::from(set.width_of(ch));
        }

        // Previously drawn text extent to clear wherever the new text no longer covers it.
        // A fresh or moved field clears its whole area instead.
        let previous = self.drawn.take().filter(|d| d.area == area);
        let (stale_left, stale_right) = match &previous {
            Some(d) => (
                d.cells.first().map_or(text_x, |c| c.1),
                d.cells.last().map_or(text_x, |&(ch, x)| x + i32::from(set.width_of(ch))),
            ),
            None => (area.top_left.x, area.top_left.x + area.size.width as i32),
        };
        if previous.is_none() {
            display.fill_solid(&area, BACKGROUND)?;
        } else {
            let band = |x0: i32, x1: i32| {
                Rectangle::new(
                    Point::new(x0, glyph_y),
                    Size::new((x1 - x0).max(0) as u32, u32::from(set.height)),
                )
            };
            display.fill_solid(&band(stale_left, text_x), BACKGROUND)?;
            display.fill_solid(&band(text_x + text_width, stale_right), BACKGROUND)?;
        }

        let palette = blend_palette(BACKGROUND, color);
        for &(ch, x) in &cells {
            let unchanged = previous
                .as_ref()
                .is_some_and(|d| d.color == color && d.cells.contains(&(ch, x)));
            if !unchanged {
                draw_glyph(display, set, ch, Point::new(x, glyph_y), &palette, area)?;
            }
        }

        self.drawn = Some(DrawnText { area, color, cells });
        Ok(())
    }
}

/// Stream one glyph box (glyph plus its background) to the display in a single window write.
/// Boxes that would spill outside `clip` (text too wide for its field) are skipped rather than
/// drawn over neighboring UI.
fn draw_glyph<D>(
    display: &mut D,
    set: &'static GlyphSet,
    ch: char,
    top_left: Point,
    palette: &[Rgb565; 16],
    clip: Rectangle,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let width = set.width_of(ch);
    let rect = Rectangle::new(top_left, Size::new(u32::from(width), u32::from(set.height)));
    if clip.intersection(&rect) != rect {
        return Ok(());
    }

    match set.glyph(ch) {
        Some(glyph) => {
            let pixels = (0..usize::from(width) * usize::from(set.height)).map(|i| {
                let byte = glyph.coverage[i / 2];
                let level = if i % 2 == 0 { byte >> 4 } else { byte & 0x0F };
                palette[usize::from(level)]
            });
            display.fill_contiguous(&rect, pixels)
        }
        None => display.fill_solid(&rect, BACKGROUND),
    }
}

/// The 16 colors between `bg` (coverage 0) and `fg` (coverage 15).
fn blend_palette(bg: Rgb565, fg: Rgb565) -> [Rgb565; 16] {
    core::array::from_fn(|level| {
        let a = level as u16;
        let mix = |b: u8, f: u8| ((u16::from(b) * (15 - a) + u16::from(f) * a + 7) / 15) as u8;
        Rgb565::new(mix(bg.r(), fg.r()), mix(bg.g(), fg.g()), mix(bg.b(), fg.b()))
    })
}

/// The display color Mousefood (default ANSI theme) renders a Ratatui `color` as, so smooth
/// digits match the TUI text styled with the same color.
pub fn rgb565_from_tui(color: Color) -> Rgb565 {
    let theme = ColorTheme::ansi();
    let rgb: Rgb888 = match color {
        Color::Reset | Color::White => theme.white,
        Color::Black => theme.black,
        Color::Red => theme.red,
        Color::Green => theme.green,
        Color::Yellow => theme.yellow,
        Color::Blue => theme.blue,
        Color::Magenta => theme.magenta,
        Color::Cyan => theme.cyan,
        Color::LightRed => theme.light_red,
        Color::LightGreen => theme.light_green,
        Color::LightYellow => theme.light_yellow,
        Color::LightBlue => theme.light_blue,
        Color::LightMagenta => theme.light_magenta,
        Color::LightCyan => theme.light_cyan,
        Color::Gray => theme.gray,
        Color::DarkGray => theme.dark_gray,
        Color::Rgb(r, g, b) => Rgb888::new(r, g, b),
        Color::Indexed(_) => theme.black,
    };
    rgb.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_sets_hold_complete_tabular_digits() {
        for set in [&LARGE_DIGITS, &SMALL_DIGITS] {
            let zero = set.width_of('0');
            for ch in "0123456789-".chars() {
                let glyph = set.glyph(ch).expect("digit missing from glyph set");
                assert_eq!(glyph.width, zero, "{ch:?} is not tabular");
                let pixels = usize::from(glyph.width) * usize::from(set.height);
                assert_eq!(glyph.coverage.len(), pixels.div_ceil(2));
            }
        }
    }

    #[test]
    fn palette_runs_from_background_to_foreground() {
        let palette = blend_palette(Rgb565::BLACK, Rgb565::WHITE);
        assert_eq!(palette[0], Rgb565::BLACK);
        assert_eq!(palette[15], Rgb565::WHITE);
        assert!(palette[7].g() > 0 && palette[7].g() < Rgb565::WHITE.g());
    }
}
