use crate::button::Button;
use crate::run_app;
use crate::screens::{
    Board, BootAnimation, CalibrationWizard, Connecting, Dashboard, DashboardDigits, Debug, Graphs,
    Screen,
};
use crate::state::global_state::MqttOutboundMessage;
use crate::state::{AppEvent, AppStateMachine, CalibrationStep, ConnectionStatus, GlobalAppState};
use crate::telemetry::TelemetryFrame;
use mousefood::embedded_graphics::prelude::{DrawTarget, OriginDimensions};
use mousefood::prelude::Rgb565;
use ratatui::Frame;
use std::cell::Cell;
use std::time::Instant;

/// Application trait to be implemented by the user
pub trait MaraUiApp {
    /// Draw the UI frame
    fn draw(&self, frame: &mut Frame);

    /// Handle button press events
    fn handle_press(&mut self, button: Button);

    /// Update application state with telemetry data
    fn update_telemetry(&mut self, telemetry: TelemetryFrame);

    /// Handle generic application event
    fn handle_event(&mut self, event: AppEvent);

    /// Process time-based transitions (auto-return after extraction)
    fn tick(&mut self) {}

    /// Returns `true` if the backlight should be on
    fn backlight_active(&self) -> bool {
        true
    }

    /// Drain app-generated MQTT outbound messages
    fn take_outbound_mqtt_messages(&mut self) -> Vec<MqttOutboundMessage>;

    /// Returns `true` once if the render loop should clear the terminal this frame
    /// (new session or Debug toggle). Consumes the request.
    fn take_redraw_request(&mut self) -> bool;

    /// Mirror the MQTT topic prefix into app state (call right after AppConfig::from_env)
    fn set_mqtt_prefix(&mut self, prefix: &str);

    /// Enqueue HA MQTT Discovery configs (called on MQTT connect when home-assistant is enabled)
    #[cfg(feature = "home-assistant")]
    fn enqueue_home_assistant(&mut self, topic_prefix: &str);

    /// Read current network status from app state
    fn connection_statuses(&self) -> (ConnectionStatus, ConnectionStatus);

    /// Current active screen
    fn current_screen(&self) -> Screen;

    /// Current step of the scale calibration wizard, if active
    fn calibration_step(&self) -> Option<CalibrationStep>;

    /// Returns `true` once the first UART telemetry frame has arrived
    fn has_telemetry(&self) -> bool;

    /// Time of the last received UART telemetry frame
    fn last_uart_frame_at(&self) -> Option<Instant>;

    /// Draw anti-aliased readouts and the boot animation straight onto the display — call right after
    /// `terminal.draw`, so they land on top of the frame Ratatui just flushed. Pass
    /// `full_redraw` when the display was cleared this frame.
    fn render_overlay<D>(&mut self, display: &mut D, full_redraw: bool)
    where
        D: DrawTarget<Color = Rgb565> + OriginDimensions,
        D::Error: core::fmt::Debug;

    /// Run the application
    ///
    /// Default implementation provided. Do not override unless necessary.
    fn run(self)
    where
        Self: Sized,
    {
        run_app(self);
    }
}

/// Main application structure
#[derive(Default)]
pub struct MaraUi {
    /// Global application state
    pub state: GlobalAppState,
    /// Board the last `draw()` rendered, so `render_overlay` decorates exactly that frame
    drawn_board: Cell<DrawnBoard>,
    /// Board `render_overlay` last drew over
    overlay_board: DrawnBoard,
    /// Smooth Dashboard digits drawn by `render_overlay`
    dashboard_digits: DashboardDigits,
    /// Start screen animation drawn by `render_overlay`
    boot_animation: BootAnimation,
}

/// Boards that get smooth overlay text, as far as `render_overlay` is concerned.
#[derive(Clone, Copy, Default, PartialEq)]
enum DrawnBoard {
    #[default]
    Other,
    Connecting,
    Dashboard,
}

/// The main application implementation
impl MaraUiApp for MaraUi {
    /// Draw the UI frame
    /// This is being called in the main loop to render the UI
    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();

        // Show the waiting screen until the machine is online — both before the first UART
        // frame and after telemetry goes stale (machine switched off).
        // Debug screen is always accessible for diagnostics.
        if !self.state.machine_online(Instant::now()) && self.state.current_screen != Screen::Debug
        {
            self.drawn_board.set(DrawnBoard::Connecting);
            Connecting::render(&self.state, area, frame);
            return;
        }

        // Calibration wizard takes over the whole screen while active, regardless of the
        // screen underneath (it is restored once calibration finishes or is cancelled).
        if self.state.calibration_step.is_some() {
            self.drawn_board.set(DrawnBoard::Other);
            CalibrationWizard::render(&self.state, area, frame);
            return;
        }

        self.drawn_board.set(match self.state.current_screen {
            Screen::Dashboard => DrawnBoard::Dashboard,
            Screen::Graphs | Screen::Debug => DrawnBoard::Other,
        });
        match self.state.current_screen {
            Screen::Dashboard => Dashboard::render(&self.state, area, frame),
            Screen::Graphs => Graphs::render(&self.state, area, frame),
            Screen::Debug => Debug::render(&self.state, area, frame),
        }
    }

    /// Handle button press events
    fn handle_press(&mut self, button: Button) {
        AppStateMachine::handle_button_press(&mut self.state, button);
    }

    /// Update application state with telemetry data
    fn update_telemetry(&mut self, telemetry: TelemetryFrame) {
        let now = Instant::now();
        AppStateMachine::handle_telemetry_frame(&mut self.state, telemetry, now);
    }

    fn handle_event(&mut self, event: AppEvent) {
        AppStateMachine::handle_event(&mut self.state, event);
    }

    fn take_outbound_mqtt_messages(&mut self) -> Vec<MqttOutboundMessage> {
        self.state.take_outbound_mqtt_messages()
    }

    fn take_redraw_request(&mut self) -> bool {
        self.state.take_redraw_request()
    }

    fn set_mqtt_prefix(&mut self, prefix: &str) {
        self.state.mqtt_topic_prefix = prefix.to_string();
    }

    #[cfg(feature = "home-assistant")]
    fn enqueue_home_assistant(&mut self, topic_prefix: &str) {
        crate::home_assistant::enqueue_discovery_configs(&mut self.state, topic_prefix);
    }

    fn backlight_active(&self) -> bool {
        let is_loading = self.state.machine_state.last_frame.is_none();
        if is_loading {
            self.state.current_screen == Screen::Debug || self.state.backlight_on
        } else if self.state.offline_mode {
            true
        } else {
            self.state.current_screen == Screen::Debug
                || self.state.backlight_should_be_on(Instant::now())
        }
    }

    fn current_screen(&self) -> Screen {
        self.state.current_screen
    }

    fn calibration_step(&self) -> Option<CalibrationStep> {
        self.state.calibration_step
    }

    fn has_telemetry(&self) -> bool {
        self.state.machine_state.last_frame.is_some()
    }

    fn last_uart_frame_at(&self) -> Option<Instant> {
        self.state.last_uart_frame_at
    }

    fn connection_statuses(&self) -> (ConnectionStatus, ConnectionStatus) {
        (
            self.state.wifi_status.clone(),
            self.state.mqtt_status.clone(),
        )
    }

    fn render_overlay<D>(&mut self, display: &mut D, full_redraw: bool)
    where
        D: DrawTarget<Color = Rgb565> + OriginDimensions,
        D::Error: core::fmt::Debug,
    {
        // Whenever the board changes, Ratatui has just painted over the overlay areas, so they
        // must be repainted in full.
        let board = self.drawn_board.get();
        if full_redraw || board != self.overlay_board {
            self.dashboard_digits.invalidate();
            self.boot_animation.invalidate();
        }
        self.overlay_board = board;

        match board {
            DrawnBoard::Dashboard => self.dashboard_digits.render(&self.state, display).unwrap(),
            DrawnBoard::Connecting => self
                .boot_animation
                .render(display, Instant::now())
                .unwrap(),
            DrawnBoard::Other => {}
        }
    }
}
