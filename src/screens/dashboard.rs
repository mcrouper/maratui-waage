use std::time::{Duration, Instant};

use mousefood::embedded_graphics::pixelcolor::Rgb565;
use mousefood::embedded_graphics::prelude::{DrawTarget, OriginDimensions};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};
#[cfg(feature = "scale-test")]
use tui_widgets::big_text::{BigText, PixelSize};

use crate::screens::screen::{Board, cells_to_pixels, display_cell_area};
use crate::smooth_digits::{
    LARGE_DIGITS, SMALL_DIGITS, SmoothField, WATER_DIGITS, rgb565_from_tui,
};
use crate::state::GlobalAppState;
use crate::telemetry::{MachineMode, TelemetryFrame};
use crate::water::TANK_CAPACITY_ML;

// Dashboard palette. RGB values are rounded to the display's RGB565 when drawn; black is the
// display background, so `STYLE_HIDDEN` makes the empty part of a bar invisible.
const STYLE_BORDER: Style = Style::new().fg(Color::Rgb(0x50, 0x50, 0x50));
const STYLE_TITLE: Style = Style::new().fg(Color::Rgb(0xFF, 0xF4, 0xE0));
const STYLE_HEADER: Style = Style::new().fg(Color::Rgb(0xE8, 0xC9, 0xA0));
const STYLE_INDICATOR: Style = Style::new().fg(Color::Rgb(0x4D, 0x7F, 0xB0));
const STYLE_HIDDEN: Style = Style::new().fg(Color::Rgb(0x00, 0x00, 0x00));
const STYLE_TIMER: Style = Style::new().fg(Color::Rgb(0xD8, 0xC3, 0xA5));
const STYLE_WEIGHT: Style = Style::new().fg(Color::Rgb(0xD8, 0xC3, 0xA5));
/// Timer before the first shot, weight placeholder.
const STYLE_INACTIVE: Style = Style::new().fg(Color::Rgb(0xAA, 0xAA, 0xAA));
const STYLE_SHOT_GAUGE: Style = Style::new().fg(Color::Rgb(0xE8, 0xC9, 0xA0));
const COLOR_HX_VALUE: Color = Color::Rgb(0x3C, 0x9A, 0x50);
const STYLE_HX_COLD: Style = Style::new().fg(Color::Rgb(0xA6, 0xB9, 0x8A));
const STYLE_HX_IDEAL: Style = Style::new().fg(Color::Rgb(0xA6, 0xB9, 0x8A));
const STYLE_HX_WARM: Style = Style::new().fg(Color::Rgb(0xA6, 0xB9, 0x8A));
const STYLE_HX_HOT: Style = Style::new().fg(Color::Rgb(0xA6, 0xB9, 0x8A));
const STYLE_HX_IDEAL_ZONE: Style = Style::new().fg(Color::Rgb(0x00, 0x00, 0x00));
const STYLE_BOILER_HEATING_LABEL: Style = Style::new().fg(Color::Rgb(0xFF, 0xF4, 0xE0));
const STYLE_BOILER_HEATING_FILL: Style = Style::new().fg(Color::Rgb(0x7A, 0x7F, 0x4A));
const STYLE_BOILER_READY: Style = Style::new().fg(Color::Rgb(0x5E, 0x6E, 0x45));
const STYLE_BOILER_TARGET: Style = Style::new().fg(Color::Rgb(0x3E, 0x5B, 0x4A));
const STYLE_NO_WATER: Style = Style::new().fg(Color::Rgb(0x4F, 0x6D, 0x8A));
const STYLE_WATER_VALUE: Style = Style::new().fg(Color::Rgb(0xE8, 0xC9, 0xA0));
const STYLE_WATER_BAR: Style = Style::new().fg(Color::Rgb(0x4D, 0x7F, 0xB0));
const STYLE_WATER_BAR_EMPTY: Style = Style::new().fg(Color::Rgb(0x50, 0x50, 0x50));
/// Remaining water value and bar once the tank is low, and the popup's frame and heading.
const STYLE_WATER_LOW: Style = Style::new().fg(Color::Rgb(0xFF, 0x5A, 0x30));
/// Low-water popup body. Its background must differ from the display's black so Ratatui
/// repaints every cell it covers, including those under the smooth Timer/Weight digits.
const COLOR_WATER_POPUP_BG: Color = Color::Rgb(0x3A, 0x0C, 0x08);
const STYLE_WATER_POPUP: Style = Style::new()
    .fg(Color::Rgb(0xFF, 0xE0, 0xC8))
    .bg(COLOR_WATER_POPUP_BG);

const BOILER_SCALE_MAX: u16 = 160;
// Shot gauge ("%") tracks extracted weight, not elapsed time: 40g (400 decigrams) == 100%.
const WEIGHT_GAUGE_MAX_DG: i32 = 400;
const HX_SCALE_MIN: f64 = 60.0;
const HX_SCALE_MAX: f64 = 110.0;
const HX_IDEAL_LOW: f64 = 90.0;
const HX_IDEAL_HIGH: f64 = 95.0;
/// Width of the tank bar in the banner, in cells.
const WATER_BAR_CELLS: u16 = 4;

#[derive(Default)]
pub struct Dashboard;

impl Board for Dashboard {
    fn render(state: &GlobalAppState, area: Rect, frame: &mut Frame) {
        let buf = frame.buffer_mut();

        let Some(t_frame) = state.machine_state.last_frame.as_ref() else {
            return;
        };

        let [banner_area, content_area, boiler_area] = dashboard_rows(area);

        let [mode_area, counter_area] = banner_halves(banner_area);
        let [cups_area, _, water_bar_area] = water_banner_areas(counter_area);

        render_mode_banner(t_frame, mode_area, buf);
        render_cup_counter(state.cup_counter, cups_area, buf);
        render_water_bar(state, water_bar_area, buf);
        render_boiler_gauge(t_frame, boiler_area, buf);

        render_main_content(state, t_frame, content_area, frame);

        // Low-water popup over the Timer/Weight column (never during a shot).
        if state.water_popup_visible(Instant::now()) {
            let [_, timer_col, _] = content_columns(content_area);
            render_water_popup(state, timer_col, frame.buffer_mut());
        }
    }
}

/// Anti-aliased Timer, Weight and HX temperature readouts. The Dashboard's Ratatui render
/// leaves those areas blank and this draws them straight onto the display after each
/// `terminal.draw`, so Ratatui never paints over them (see `MaraUiApp::render_overlay`).
pub struct DashboardDigits {
    timer: SmoothField,
    weight: SmoothField,
    hx: SmoothField,
    water: SmoothField,
}

// `LARGE_DIGITS` boxes keep ~11 px below the digits free for the comma's descender, so
// centered digits sit high. Nudge them down: the Timer (no comma) low in its block, ~2 px above
// the bottom border with the assessment label, the Weight only as far as its comma still fits.
const TIMER_DIGITS_NUDGE_PX: i32 = 16;
const WEIGHT_DIGITS_NUDGE_PX: i32 = 2;

impl Default for DashboardDigits {
    fn default() -> Self {
        Self {
            timer: SmoothField::new(&LARGE_DIGITS).nudged_down(TIMER_DIGITS_NUDGE_PX),
            weight: SmoothField::new(&LARGE_DIGITS)
                .nudged_down(WEIGHT_DIGITS_NUDGE_PX)
                .with_sign_slot(),
            hx: SmoothField::new(&SMALL_DIGITS),
            water: SmoothField::new(&WATER_DIGITS),
        }
    }
}

impl DashboardDigits {
    /// Repaint every readout on the next `render` (another screen or a full clear drew over it).
    pub fn invalidate(&mut self) {
        self.timer.invalidate();
        self.weight.invalidate();
        self.hx.invalidate();
        self.water.invalidate();
    }

    pub fn render<D>(&mut self, state: &GlobalAppState, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565> + OriginDimensions,
    {
        // The `scale-test` Dashboard replaces these readouts with its raw ADC view.
        if cfg!(feature = "scale-test") {
            return Ok(());
        }
        let Some(t_frame) = state.machine_state.last_frame.as_ref() else {
            return Ok(());
        };

        let areas = value_areas(display_cell_area(display));

        // The low-water popup covers the Timer and Weight; repaint them in full once it's gone.
        if state.water_popup_visible(Instant::now()) {
            self.timer.invalidate();
            self.weight.invalidate();
        } else {
            let secs = current_extraction_secs(state).to_string();
            let timer_color = style_color(timer_style(state));
            self.timer
                .draw(display, cells_to_pixels(areas.timer), &secs, timer_color)?;
            let weight_area = cells_to_pixels(areas.weight);
            let (mut weight, weight_style) = weight_text(state, true);
            // -100 g and below (or 1000 g and up) is too wide with the decimal: whole grams.
            if !self.weight.fits(weight_area, &weight) {
                weight = weight_text(state, false).0;
            }
            self.weight
                .draw(display, weight_area, &weight, style_color(weight_style))?;
        }
        let water_color = style_color(water_style(state));
        let water = format!("{}L", liters_text(state.water.remaining_ml()));
        self.water
            .draw(display, cells_to_pixels(areas.water), &water, water_color)?;
        let hx = format!("{}°C", t_frame.hx_now_c);
        self.hx
            .draw(display, cells_to_pixels(areas.hx), &hx, rgb565_from_tui(COLOR_HX_VALUE))
    }
}

/// Cell areas of the readouts `DashboardDigits` draws instead of Ratatui.
struct ValueAreas {
    timer: Rect,
    weight: Rect,
    hx: Rect,
    water: Rect,
}

fn value_areas(area: Rect) -> ValueAreas {
    let [banner_area, content_area, _] = dashboard_rows(area);
    let [_, counter_area] = banner_halves(banner_area);
    let [_, water_value_area, _] = water_banner_areas(counter_area);
    let [info_col, timer_col, _] = content_columns(content_area);
    let [timer_area, weight_area] = timer_column_halves(timer_col);
    let [hx_label_area, _, _] = info_rows(Block::bordered().inner(info_col));

    ValueAreas {
        timer: Block::bordered().inner(timer_area),
        weight: Block::bordered().inner(weight_area),
        // The line below the "HX" label.
        hx: Rect {
            y: hx_label_area.y + 1,
            height: 1,
            ..hx_label_area
        },
        water: water_value_area,
    }
}

fn style_color(style: Style) -> Rgb565 {
    rgb565_from_tui(style.fg.unwrap_or(Color::Reset))
}

/// Mode banner | cup counter and tank level.
fn banner_halves(banner_area: Rect) -> [Rect; 2] {
    Layout::horizontal(Constraint::from_fills([1, 1])).areas(banner_area)
}

/// Cup counter, remaining water value (drawn by `DashboardDigits`) and tank bar.
fn water_banner_areas(counter_area: Rect) -> [Rect; 3] {
    let [cups, _, value, _, bar] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(6),
        Constraint::Length(1),
        Constraint::Length(WATER_BAR_CELLS),
    ])
    .areas(counter_area);
    [cups, value, bar]
}

/// banner (1) | main (fill) | boiler (3)
fn dashboard_rows(area: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(3),
    ])
    .areas(area)
}

/// info column | timer | shot gauge, rule of thirds (1:3:1).
fn content_columns(content_area: Rect) -> [Rect; 3] {
    Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Fill(3),
        Constraint::Fill(1),
    ])
    .areas(content_area)
}

/// Timer on top, weight below.
fn timer_column_halves(timer_col: Rect) -> [Rect; 2] {
    Layout::vertical(Constraint::from_fills([1, 1])).areas(timer_col)
}

/// HX label + value | HX gauge | heat/pump status, inside the info column's border.
fn info_rows(inner: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(4),
    ])
    .areas(inner)
}

/// Normal build: info column | timer | shot gauge, rule of thirds (1:3:1).
#[cfg(not(feature = "scale-test"))]
fn render_main_content(
    state: &GlobalAppState,
    t_frame: &TelemetryFrame,
    content_area: Rect,
    frame: &mut Frame,
) {
    let buf = frame.buffer_mut();
    let [info_col, timer_col, gauge_col] = content_columns(content_area);

    render_info_col(t_frame, info_col, buf);
    render_shot_gauge(state, gauge_col, buf);
    render_timer(state, timer_col, frame);
}

/// `scale-test` build: no Mara telemetry to show, so the whole content area becomes one big
/// raw (uncalibrated, untared) scale readout — right cell, left cell, and their sum, stacked.
#[cfg(feature = "scale-test")]
fn render_main_content(
    state: &GlobalAppState,
    _t_frame: &TelemetryFrame,
    content_area: Rect,
    frame: &mut Frame,
) {
    render_raw_weight(state, content_area, frame);
}

fn render_mode_banner(t_frame: &TelemetryFrame, area: Rect, buf: &mut Buffer) {
    let (label, style) = match t_frame.mode {
        MachineMode::Coffee => ("═══ COFFEE MODE ═══", STYLE_HEADER),
        MachineMode::SteamS | MachineMode::SteamV | MachineMode::SteamC => {
            ("═══ STEAM MODE ═══", STYLE_HEADER)
        }
        MachineMode::Offline => ("═══  OFFLINE  ═══", STYLE_HEADER),
        MachineMode::Unknown(_) => ("═══  UNKNOWN  ═══", STYLE_HEADER),
    };

    Paragraph::new(Line::from(label))
        .centered()
        .style(style)
        .render(area, buf);
}

fn render_cup_counter(count: Option<u64>, area: Rect, buf: &mut Buffer) {
    let Some(cups) = count else {
        return;
    };

    Paragraph::new(Line::from(format!("{cups} cups")))
        .centered()
        .style(STYLE_HEADER)
        .render(area, buf);
}

/// Litres with two decimals and a decimal comma, e.g. 1850 ml -> "1,85".
fn liters_text(ml: u32) -> String {
    let centiliters = (ml + 5) / 10;
    format!("{},{:02}", centiliters / 100, centiliters % 100)
}

fn water_style(state: &GlobalAppState) -> Style {
    if state.water.is_low() {
        STYLE_WATER_LOW
    } else {
        STYLE_WATER_VALUE
    }
}

/// Tank level bar next to the remaining water value.
fn render_water_bar(state: &GlobalAppState, area: Rect, buf: &mut Buffer) {
    let cells = u32::from(area.width);
    let filled = (state.water.remaining_ml() * cells + TANK_CAPACITY_ML / 2) / TANK_CAPACITY_ML;
    let fill_style = if state.water.is_low() {
        STYLE_WATER_LOW
    } else {
        STYLE_WATER_BAR
    };
    for i in 0..area.width {
        let (ch, style) = if u32::from(i) < filled {
            ("█", fill_style)
        } else {
            ("░", STYLE_WATER_BAR_EMPTY)
        };
        buf.set_string(area.x + i, area.y, ch, style);
    }
}

/// Refill prompt; confirmed with a short press (see `AppStateMachine::handle_button_press`).
fn render_water_popup(state: &GlobalAppState, area: Rect, buf: &mut Buffer) {
    Clear.render(area, buf);
    let block = Block::bordered()
        .border_type(BorderType::Double)
        .border_style(STYLE_WATER_LOW.bg(COLOR_WATER_POPUP_BG))
        .style(STYLE_WATER_POPUP);
    let inner = block.inner(area);
    block.render(area, buf);

    let rest = format!("Rest: {} L", liters_text(state.water.remaining_ml()));
    let lines = vec![
        Line::styled("WASSER FAST LEER", STYLE_WATER_LOW.bg(COLOR_WATER_POPUP_BG)),
        Line::raw(""),
        Line::raw(rest),
        Line::raw(""),
        Line::raw("Wasser nachgefüllt?"),
        Line::raw("Taste kurz drücken"),
    ];
    let [_, text_area, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(lines.len() as u16),
        Constraint::Fill(1),
    ])
    .areas(inner);
    Paragraph::new(lines)
        .centered()
        .style(STYLE_WATER_POPUP)
        .render(text_area, buf);
}

fn render_info_col(t_frame: &TelemetryFrame, area: Rect, buf: &mut Buffer) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(STYLE_BORDER);
    let inner = block.inner(area);
    block.render(area, buf);

    let [hx_label_area, hx_gauge_area, status_area] = info_rows(inner);

    // The value line below stays blank: `DashboardDigits` draws the HX temperature there.
    Paragraph::new(Line::styled("HX", STYLE_INDICATOR))
        .centered()
        .render(hx_label_area, buf);

    let [_, hx_gauge_area, _] = Layout::horizontal([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(hx_gauge_area);

    render_hx_vgauge(t_frame.hx_now_c, hx_gauge_area, buf);

    let heat_span = Span::styled(
        if t_frame.heating_on {
            "● HEAT"
        } else {
            "○ HEAT"
        },
        STYLE_INDICATOR,
    );
    let pump_span = Span::styled(
        if t_frame.pump_on {
            "● PUMP"
        } else {
            "○ PUMP"
        },
        STYLE_INDICATOR,
    );

    Paragraph::new(vec![
        Line::raw(""),
        Line::from(heat_span),
        Line::raw(""),
        Line::from(pump_span),
    ])
    .centered()
    .render(status_area, buf);
}

/// Full-width vertical gauge for HX temperature.
///
/// Scale 60–110°C. Ideal zone 90–95°C is marked with `▒` even when unfilled.
/// Colors: white (cold) → green (ideal 88–95°) → yellow (95–100°) → red (>100°).
fn render_hx_vgauge(temp: u16, area: Rect, buf: &mut Buffer) {
    let total = area.height as usize;
    if total == 0 || area.width == 0 {
        return;
    }

    let ratio = ((temp as f64 - HX_SCALE_MIN) / (HX_SCALE_MAX - HX_SCALE_MIN)).clamp(0.0, 1.0);
    let filled = (ratio * total as f64).round() as usize;

    // Convert temperature to row index (bar fills from bottom, so higher temp = lower row index)
    let temp_to_row = |t: f64| -> usize {
        let r = ((t - HX_SCALE_MIN) / (HX_SCALE_MAX - HX_SCALE_MIN)).clamp(0.0, 1.0);
        total.saturating_sub((r * total as f64).round() as usize)
    };
    let ideal_top_row = temp_to_row(HX_IDEAL_HIGH);
    let ideal_bot_row = temp_to_row(HX_IDEAL_LOW);

    let fill_style = match temp {
        0..=87 => STYLE_HX_COLD,
        88..=95 => STYLE_HX_IDEAL,
        96..=100 => STYLE_HX_WARM,
        _ => STYLE_HX_HOT,
    };

    let w = area.width as usize;
    let fill = "█".repeat(w);
    let empty = "░".repeat(w);
    let ideal = "▒".repeat(w);

    for row in 0..total {
        let y = area.y + row as u16;
        let is_filled = row >= total.saturating_sub(filled);
        let in_ideal = row >= ideal_top_row && row < ideal_bot_row;

        let (s, style) = if is_filled {
            (fill.as_str(), fill_style)
        } else if in_ideal {
            (ideal.as_str(), STYLE_HX_IDEAL_ZONE)
        } else {
            (empty.as_str(), STYLE_HIDDEN)
        };

        buf.set_string(area.x, y, s, style);
    }
}

/// Custom boiler bar with absolute temperature scale, color zones, and target marker.
///
/// Visual:  label | ████████████░░░│────────── |
///                  warm     ready  ^target
fn render_boiler_gauge(t_frame: &TelemetryFrame, area: Rect, buf: &mut Buffer) {
    let block = Block::bordered()
        .title("Boiler")
        .title_style(STYLE_TITLE)
        .border_type(BorderType::Rounded)
        .border_style(STYLE_BORDER);
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let Some(target) = t_frame.boiler_target_c else {
        Paragraph::new(Line::from("NO WATER"))
            .centered()
            .style(STYLE_NO_WATER)
            .render(inner, buf);
        return;
    };

    let now = t_frame.boiler_now_c;
    let at_temp = now as f64 / target as f64 >= 0.95;

    // Left label: "120°/138° "
    let label = format!("{:3}°/{:3}° ", now, target);
    let label_len = label.len() as u16; // all ASCII + °(1 display width each)
    let label_style = if at_temp {
        STYLE_BOILER_READY
    } else {
        STYLE_BOILER_HEATING_LABEL
    };
    buf.set_string(inner.x, inner.y, &label, label_style);

    let bar_x = inner.x + label_len;
    let bar_w = inner.width.saturating_sub(label_len) as usize;
    if bar_w == 0 {
        return;
    }

    // Map temperatures to bar positions on 0..BOILER_SCALE_MAX scale
    let pos_of = |temp: u16| -> usize {
        ((temp as f64 / BOILER_SCALE_MAX as f64) * bar_w as f64).round() as usize
    };
    let current_pos = pos_of(now).min(bar_w);
    let target_pos = pos_of(target).min(bar_w.saturating_sub(1));
    // Ready zone starts at 90% of target temperature
    let ready_pos = pos_of((target as f64 * 0.9) as u16);

    for i in 0..bar_w {
        let x = bar_x + i as u16;
        let (ch, style) = if i < current_pos {
            // Filled portion — two-tone: warming → ready
            if i >= ready_pos {
                ("█", STYLE_BOILER_READY)
            } else {
                ("█", STYLE_BOILER_HEATING_FILL)
            }
        } else if i == target_pos {
            // Target marker (current hasn't reached it yet)
            ("│", STYLE_BOILER_TARGET)
        } else {
            ("─", STYLE_HIDDEN)
        };
        buf.set_string(x, inner.y, ch, style);
    }
}

fn render_shot_gauge(state: &GlobalAppState, area: Rect, buf: &mut Buffer) {
    let has_data = state.extraction_state.is_extracting()
        || state.extraction_state.last_extraction_duration().is_some();

    let block = Block::bordered()
        .title_bottom("%")
        .title_alignment(ratatui::layout::HorizontalAlignment::Center)
        .title_style(STYLE_TITLE)
        .border_type(BorderType::Rounded)
        .border_style(STYLE_BORDER);
    let inner = block.inner(area);
    block.render(area, buf);

    if !has_data {
        return;
    }

    let total = inner.height as usize;
    let weight_dg = state.weight_dg.unwrap_or(0).max(0);
    let ratio = (weight_dg as f64 / WEIGHT_GAUGE_MAX_DG as f64).min(1.0);
    let filled = (ratio * total as f64).round() as usize;
    let fill: String = "█".repeat(inner.width as usize);
    let empty: String = "░".repeat(inner.width as usize);

    for row in 0..total {
        let y = inner.y + row as u16;
        let is_filled = row >= total.saturating_sub(filled);
        if is_filled {
            buf.set_string(inner.x, y, &fill, STYLE_SHOT_GAUGE);
        } else {
            buf.set_string(inner.x, y, &empty, STYLE_HIDDEN);
        }
    }
}

fn render_timer(state: &GlobalAppState, area: Rect, frame: &mut Frame) {
    let extraction_secs = current_extraction_secs(state);
    let timer_style = timer_style(state);

    // Split the column in half: timer on top, weight below, each labeled on top.
    let [timer_area, weight_area] = timer_column_halves(area);

    render_timer_half(timer_area, frame, extraction_secs, timer_style, state);
    render_weight_half(weight_area, frame);
}

fn timer_style(state: &GlobalAppState) -> Style {
    if state.extraction_state.is_extracting()
        || state.extraction_state.last_extraction_duration().is_some()
    {
        STYLE_TIMER
    } else {
        STYLE_INACTIVE
    }
}

/// The digits themselves are drawn by `DashboardDigits`; this renders only the frame, with the
/// post-shot assessment label set into its bottom border so the digits get the whole inside.
fn render_timer_half(
    area: Rect,
    frame: &mut Frame,
    extraction_secs: u64,
    timer_style: Style,
    state: &GlobalAppState,
) {
    let mut timer_block = Block::bordered()
        .title("Timer")
        .title_alignment(ratatui::layout::HorizontalAlignment::Center)
        .title_style(STYLE_TITLE)
        .border_type(BorderType::Rounded)
        .border_style(STYLE_BORDER);

    if !state.extraction_state.is_extracting()
        && state.extraction_state.last_extraction_duration().is_some()
    {
        timer_block = timer_block
            .title_bottom(Line::styled(shot_quality_label(extraction_secs), timer_style));
    }

    timer_block.render(area, frame.buffer_mut());
}

/// Only the frame — `DashboardDigits` draws the weight inside it.
fn render_weight_half(area: Rect, frame: &mut Frame) {
    Block::bordered()
        .title("Weight")
        .title_alignment(ratatui::layout::HorizontalAlignment::Center)
        .title_style(STYLE_TITLE)
        .border_type(BorderType::Rounded)
        .border_style(STYLE_BORDER)
        .render(area, frame.buffer_mut());
}

fn weight_text(state: &GlobalAppState, decimal: bool) -> (String, Style) {
    match state.weight_dg {
        // German decimal comma, not a period.
        Some(dg) if decimal => (format!("{:.1}", dg as f64 / 10.0).replace('.', ","), STYLE_WEIGHT),
        Some(dg) => (format!("{:.0}", dg as f64 / 10.0), STYLE_WEIGHT),
        None => ("--".to_string(), STYLE_INACTIVE),
    }
}

/// `scale-test` build only: raw (uncalibrated, untared, EMA-smoothed) HX711 counts for right
/// cell, left cell, and their sum — stacked in that order, filling the whole content area.
///
/// Uses `PixelSize::Quadrant`, same as the normal Weight/Timer display — that's the smallest
/// size confirmed to render correctly on this display's embedded font. The smaller
/// `ThirdHeight`/`Sextant`/etc. sizes pull glyphs from a much rarer Unicode block (Legacy
/// Computing Symbols) that this font doesn't have, and render as garbage on real hardware.
/// No bordered `Block` here (unlike the normal Weight box) — three Quadrant-height lines
/// (4 rows each = 12) barely fit the content area as it is; a border would push it over.
#[cfg(feature = "scale-test")]
const RAW_WEIGHT_PIXEL_SIZE: PixelSize = PixelSize::Quadrant;

#[cfg(feature = "scale-test")]
fn render_raw_weight(state: &GlobalAppState, area: Rect, frame: &mut Frame) {
    let buf = frame.buffer_mut();

    let [title_area, digits_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);

    Paragraph::new(Line::from("WEIGHT (raw, uncalibrated)"))
        .centered()
        .style(STYLE_INACTIVE)
        .render(title_area, buf);

    let fmt = |v: Option<i32>| v.map(|v| v.to_string()).unwrap_or_else(|| "--".to_string());
    let sum = match (state.raw_weight_left, state.raw_weight_right) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0) + right.unwrap_or(0)),
    };

    let big_text = BigText::builder()
        .pixel_size(RAW_WEIGHT_PIXEL_SIZE)
        .centered()
        .lines(vec![
            Line::styled(format!("R {}", fmt(state.raw_weight_right)), STYLE_TITLE),
            Line::styled(format!("L {}", fmt(state.raw_weight_left)), STYLE_WEIGHT),
            Line::styled(format!("S {}", fmt(sum)), STYLE_INDICATOR),
        ])
        .build();

    frame.render_widget(big_text, digits_area);
}

fn current_extraction_secs(state: &GlobalAppState) -> u64 {
    if state.extraction_state.is_extracting() {
        state
            .extraction_state
            .elapsed()
            .unwrap_or(Duration::ZERO)
            .as_secs()
    } else {
        state
            .extraction_state
            .last_extraction_duration()
            .unwrap_or(Duration::ZERO)
            .as_secs()
    }
}

fn shot_quality_label(secs: u64) -> &'static str {
    match secs {
        15..=19 => "UNDEREXTRACTED",
        20..=23 => "GOOD",
        24..=27 => "PERFECT",
        28..=31 => "LONG SHOT",
        32..=39 => "BLONDING",
        _ => "RIP SHOT",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_text_without_decimal_rounds_to_whole_grams() {
        let mut state = GlobalAppState::default();
        state.weight_dg = Some(-1376);
        assert_eq!(weight_text(&state, true).0, "-137,6");
        assert_eq!(weight_text(&state, false).0, "-138");
    }

    #[test]
    fn liters_text_rounds_to_ten_ml() {
        assert_eq!(liters_text(2300), "2,30");
        assert_eq!(liters_text(1855), "1,86");
        assert_eq!(liters_text(495), "0,50");
        assert_eq!(liters_text(0), "0,00");
    }
}
