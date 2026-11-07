use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Gauge, Paragraph, Widget};

use crate::screens::screen::Board;
use crate::state::GlobalAppState;

// Start screen palette. RGB values are rounded to the display's RGB565 when drawn.
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

        // Version in the top-right corner, above the animation.
        let [ver_area, _] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(top);

        Paragraph::new(concat!("v", env!("CARGO_PKG_VERSION")))
            .alignment(Alignment::Right)
            .style(Style::new().fg(COLOR_VERSION))
            .render(ver_area, frame.buffer_mut());

        // The rest of the top area stays blank: `BootAnimation` plays there.

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

/// Top 13 rows (182 px): version line, then the boot animation drawn directly onto the
/// display by `BootAnimation` via `render_overlay`. The loading block below.
fn top_and_bottom(area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Length(13), Constraint::Fill(1)]).areas(area)
}
