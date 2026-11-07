use crate::state::GlobalAppState;
use mousefood::embedded_graphics::prelude::{OriginDimensions, Point, Size};
use mousefood::embedded_graphics::primitives::Rectangle;
use ratatui::Frame;
use ratatui::layout::Rect;
use strum::{Display, EnumIter, FromRepr};

/// Available screens in the application
#[derive(Clone, Copy, Default, Display, FromRepr, PartialEq, Eq, Debug, EnumIter)]
pub enum Screen {
    #[default]
    #[strum(to_string = "Dashboard")]
    Dashboard,
    #[strum(to_string = "Graphs")]
    Graphs,
    #[strum(to_string = "Debug")]
    Debug,
}

impl Screen {
    /// Toggle between Dashboard and Graphs (single-button navigation)
    pub fn next(self) -> Self {
        match self {
            Screen::Dashboard => Screen::Graphs,
            Screen::Graphs => Screen::Dashboard,
            Screen::Debug => Screen::Debug,
        }
    }

    /// Alias for next — with one button there is no separate "previous"
    pub fn previous(self) -> Self {
        self.next()
    }
}

/// Trait for rendering screen content
pub trait Board {
    /// Render the screen with the given application state
    fn render(state: &GlobalAppState, area: Rect, frame: &mut Frame);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screen_next_cycles_dashboard_graphs() {
        assert_eq!(Screen::Dashboard.next(), Screen::Graphs);
        assert_eq!(Screen::Graphs.next(), Screen::Dashboard);
    }

    #[test]
    fn test_screen_next_debug_stays() {
        assert_eq!(Screen::Debug.next(), Screen::Debug);
    }

    #[test]
    fn test_screen_previous_mirrors_next() {
        assert_eq!(Screen::Dashboard.previous(), Screen::Dashboard.next());
        assert_eq!(Screen::Graphs.previous(), Screen::Graphs.next());
        assert_eq!(Screen::Debug.previous(), Screen::Debug.next());
    }

    #[test]
    fn test_screen_default_is_dashboard() {
        assert_eq!(Screen::default(), Screen::Dashboard);
    }
}

/// Pixel size of one Ratatui cell — the backend's `MONO_7X14` font (see `setup.rs`).
const CELL_WIDTH_PX: u32 = 7;
const CELL_HEIGHT_PX: u32 = 14;

/// The Ratatui frame area (in cells) that Mousefood lays out on `display`.
pub fn display_cell_area(display: &impl OriginDimensions) -> Rect {
    let size = display.size();
    Rect::new(
        0,
        0,
        (size.width / CELL_WIDTH_PX) as u16,
        (size.height / CELL_HEIGHT_PX) as u16,
    )
}

/// Display pixels covered by a cell `area`, for drawing straight onto the display over it.
pub fn cells_to_pixels(area: Rect) -> Rectangle {
    Rectangle::new(
        Point::new(
            (u32::from(area.x) * CELL_WIDTH_PX) as i32,
            (u32::from(area.y) * CELL_HEIGHT_PX) as i32,
        ),
        Size::new(
            u32::from(area.width) * CELL_WIDTH_PX,
            u32::from(area.height) * CELL_HEIGHT_PX,
        ),
    )
}
