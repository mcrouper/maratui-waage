use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};

use crate::scale::CALIBRATION_REFERENCE_G;
use crate::screens::screen::Board;
use crate::state::{CalibrationStep, GlobalAppState};

const STYLE_YELLOW: Style = Style::new().fg(Color::Yellow);
const STYLE_GREEN: Style = Style::new().fg(Color::Green);
const STYLE_RED: Style = Style::new().fg(Color::Red);
const STYLE_WHITE: Style = Style::new().fg(Color::White);
const STYLE_GRAY: Style = Style::new().fg(Color::Gray);

/// Full-screen step-by-step scale calibration wizard.
///
/// Shown whenever `GlobalAppState::calibration_step` is `Some` (entered by holding Button1
/// for 3s). A short press confirms/advances the current step; a long press cancels.
#[derive(Default)]
pub struct CalibrationWizard;

impl Board for CalibrationWizard {
    fn render(state: &GlobalAppState, area: Rect, frame: &mut Frame) {
        let Some(step) = state.calibration_step else {
            return;
        };

        let buf = frame.buffer_mut();
        let block = Block::bordered()
            .title("Scale Calibration")
            .border_type(BorderType::Rounded)
            .border_style(STYLE_YELLOW);
        let inner = block.inner(area);
        block.render(area, buf);

        let (title, lines, style) = wizard_content(step);

        let [title_area, body_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);

        Paragraph::new(Line::styled(title, style))
            .centered()
            .render(title_area, buf);

        Paragraph::new(lines).centered().style(style).render(body_area, buf);
    }
}

fn wizard_content(step: CalibrationStep) -> (&'static str, Vec<Line<'static>>, Style) {
    match step {
        CalibrationStep::ConfirmEmptyLeft => (
            "Left scale: zero point",
            vec![
                Line::from(""),
                Line::from("Remove everything from the left scale."),
                Line::from(""),
                Line::from("Short press: confirm & tare left cell"),
                Line::from("Long press: cancel"),
            ],
            STYLE_WHITE,
        ),
        CalibrationStep::TaringLeft => (
            "Taring left cell...",
            vec![Line::from(""), Line::from("Reading left zero point...")],
            STYLE_GRAY,
        ),
        CalibrationStep::ConfirmReferenceLeft => (
            "Left scale: reference weight",
            vec![
                Line::from(""),
                Line::from(format!(
                    "Place a {:.0}g reference weight on the left scale.",
                    CALIBRATION_REFERENCE_G
                )),
                Line::from(""),
                Line::from("Short press: confirm & calibrate left cell"),
                Line::from("Long press: cancel"),
            ],
            STYLE_WHITE,
        ),
        CalibrationStep::CalibratingLeft => (
            "Calibrating left cell...",
            vec![Line::from(""), Line::from("Reading left reference weight...")],
            STYLE_GRAY,
        ),
        CalibrationStep::ConfirmEmptyRight => (
            "Right scale: zero point",
            vec![
                Line::from(""),
                Line::from("Remove everything from the right scale."),
                Line::from(""),
                Line::from("Short press: confirm & tare right cell"),
                Line::from("Long press: cancel"),
            ],
            STYLE_WHITE,
        ),
        CalibrationStep::TaringRight => (
            "Taring right cell...",
            vec![Line::from(""), Line::from("Reading right zero point...")],
            STYLE_GRAY,
        ),
        CalibrationStep::ConfirmReferenceRight => (
            "Right scale: reference weight",
            vec![
                Line::from(""),
                Line::from(format!(
                    "Place a {:.0}g reference weight on the right scale.",
                    CALIBRATION_REFERENCE_G
                )),
                Line::from(""),
                Line::from("Short press: confirm & calibrate right cell"),
                Line::from("Long press: cancel"),
            ],
            STYLE_WHITE,
        ),
        CalibrationStep::CalibratingRight => (
            "Calibrating right cell...",
            vec![Line::from(""), Line::from("Reading right reference weight...")],
            STYLE_GRAY,
        ),
        CalibrationStep::Done { success: true } => (
            "Done",
            vec![
                Line::from(""),
                Line::from("Both scales calibrated successfully!"),
                Line::from(""),
                Line::from("Press button to continue"),
            ],
            STYLE_GREEN,
        ),
        CalibrationStep::Done { success: false } => (
            "Failed",
            vec![
                Line::from(""),
                Line::from("No reading from one of the scales — check wiring."),
                Line::from(""),
                Line::from("Press button to continue"),
            ],
            STYLE_RED,
        ),
    }
}
