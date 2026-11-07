use mousefood::embedded_graphics::pixelcolor::Rgb565;
use mousefood::embedded_graphics::prelude::{DrawTarget, OriginDimensions};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Gauge, Paragraph, Widget};

use crate::screens::screen::{Board, cells_to_pixels, display_cell_area};
use crate::smooth_digits::{SmoothField, TITLE_LETTERS, rgb565_from_tui};
use crate::state::GlobalAppState;

// Start screen palette. RGB values are rounded to the display's RGB565 when drawn.
const COLOR_TITLE: Color = Color::Rgb(0xC8, 0x75, 0x33);
const COLOR_VERSION: Color = Color::Rgb(0xFF, 0xB0, 0x00);
const COLOR_STATUS: Color = Color::Rgb(0xC8, 0x75, 0x33);
const COLOR_PROGRESS: Color = Color::Rgb(0xC8, 0x75, 0x33);
const COLOR_PROGRESS_TRACK: Color = Color::Rgb(0x7F, 0x7F, 0x7F);

#[derive(Default)]
pub struct Connecting;

impl Board for Connecting {
    fn render(state: &GlobalAppState, area: Rect, frame: &mut Frame) {
        Self::render_inner(state, area, frame);
    }
}

impl Connecting {
    fn render_inner(state: &GlobalAppState, area: Rect, frame: &mut Frame) {
        let [top, bottom] = top_and_bottom(area);

        // Version in the top-right corner (outside the image footprint which ends at ~col 27)
        let [ver_area, _] = version_and_title(top);

        Paragraph::new(concat!("v", env!("CARGO_PKG_VERSION")))
            .alignment(Alignment::Right)
            .style(Style::new().fg(COLOR_VERSION))
            .render(ver_area, frame.buffer_mut());

        // The title area stays blank: `ConnectingTitle` draws "MAXI / TUI" there.

        // Resolve current message and progress from state
        let (message, progress) = state.loading_status.unwrap_or(("starting up...", 0));
        let ratio = f64::from(progress) / 100.0;

        // Loading block: rounded yellow border, message + gauge inside
        let block = Block::bordered()
            .border_type(BorderType::Double)
            .border_style(Style::new().black());
        let inner = block.inner(bottom);
        block.render(bottom, frame.buffer_mut());

        let [msg_row, gauge_row] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);

        Paragraph::new(Line::styled(message, Style::new().fg(COLOR_STATUS)))
            .centered()
            .render(msg_row, frame.buffer_mut());

        Gauge::default()
            .gauge_style(Style::default().fg(COLOR_PROGRESS).bg(COLOR_PROGRESS_TRACK))
            .label(Span::raw(""))
            .ratio(ratio)
            .render(gauge_row, frame.buffer_mut());
    }
}

/// Top 13 rows (≈182 px) left empty — rat barista image rendered directly via render_image() —
/// and the loading block below.
fn top_and_bottom(area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Length(13), Constraint::Fill(1)]).areas(area)
}

/// Version line and title area, right of the rat barista image.
fn version_and_title(top: Rect) -> [Rect; 2] {
    let [_, label_render_area] =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Fill(1)]).areas(top);
    Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(label_render_area)
}

/// The "MAXI" / "TUI" title lines, each 4 rows tall, vertically centered in the title area.
fn title_line_areas(area: Rect) -> [Rect; 2] {
    let [top, _] = top_and_bottom(area);
    let [_, title_area] = version_and_title(top);
    let [_, centered_title_area, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(8),
        Constraint::Fill(1),
    ])
    .areas(title_area);
    Layout::vertical(Constraint::from_lengths([4, 4])).areas(centered_title_area)
}

/// Anti-aliased start screen title, drawn straight onto the display over the area the
/// Ratatui render leaves blank (see `MaraUiApp::render_overlay`).
pub struct ConnectingTitle {
    lines: [SmoothField; 2],
}

impl Default for ConnectingTitle {
    fn default() -> Self {
        Self {
            lines: [SmoothField::new(&TITLE_LETTERS), SmoothField::new(&TITLE_LETTERS)],
        }
    }
}

impl ConnectingTitle {
    /// Repaint the title on the next `render` (another screen or a full clear drew over it).
    pub fn invalidate(&mut self) {
        self.lines.iter_mut().for_each(SmoothField::invalidate);
    }

    pub fn render<D>(&mut self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565> + OriginDimensions,
    {
        let areas = title_line_areas(display_cell_area(display));
        let color = rgb565_from_tui(COLOR_TITLE);
        for ((field, area), text) in self.lines.iter_mut().zip(areas).zip(["MAXI", "TUI"]) {
            field.draw(display, cells_to_pixels(area), text, color)?;
        }
        Ok(())
    }
}
