use crate::screens::Screen;
use crate::telemetry::MachineState;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

// TODO: if the device is powered directly from the espresso machine, the backlight
// timeout is unnecessary — consider removing BACKLIGHT_TIMEOUT and backlight_should_be_on entirely.
pub const BACKLIGHT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long telemetry may be absent before the machine is considered offline.
///
/// Once this elapses with no UART frame, the UI falls back to the waiting screen and
/// the next frame to arrive starts a fresh session (terminal + graph buffers cleared).
pub const MACHINE_OFFLINE_TIMEOUT: Duration = Duration::from_secs(30);

/// Rolling window size for `GlobalAppState::flow_samples`, matching the existing 300-point
/// convention used by `MachineState`'s temperature graph buffers.
const FLOW_SAMPLES_CAP: usize = 300;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disabled,
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct MqttOutboundMessage {
    /// Topic suffix (combined with prefix by publish_mqtt_message) unless `absolute` is true.
    pub topic_suffix: String,
    pub payload: String,
    /// When `true`, the broker should store a retained copy of this message.
    pub retain: bool,
    /// When `true`, `topic_suffix` is used as-is (bypasses prefix construction).
    pub absolute: bool,
}

/// State of the coffee extraction process
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractionState {
    /// Pump is not running, no extraction in progress
    Idle {
        /// Duration of the last completed extraction
        last_extraction_duration: Option<Duration>,
    },
    /// Pump is running, extraction in progress
    Extracting { started_at: Instant },
}

impl Default for ExtractionState {
    fn default() -> Self {
        Self::Idle {
            last_extraction_duration: None,
        }
    }
}

impl ExtractionState {
    /// Get the duration of the current extraction (if in progress)
    pub fn elapsed(&self) -> Option<Duration> {
        match self {
            ExtractionState::Extracting { started_at } => Some(started_at.elapsed()),
            ExtractionState::Idle { .. } => None,
        }
    }

    /// Check if extraction is currently in progress
    pub fn is_extracting(&self) -> bool {
        matches!(self, ExtractionState::Extracting { .. })
    }

    /// Get the duration of the last completed extraction
    pub fn last_extraction_duration(&self) -> Option<Duration> {
        match self {
            ExtractionState::Idle {
                last_extraction_duration,
            } => *last_extraction_duration,
            ExtractionState::Extracting { .. } => None,
        }
    }
}

/// Board metadata published to MQTT and shown on the Debug screen
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    pub wifi_ssid: String,
    pub wifi_rssi: Option<i32>,
    pub ip: Option<String>,
    pub uptime_s: u64,
    pub free_heap_b: Option<u32>,
    /// Seconds since the last UART telemetry frame was received
    pub last_telemetry_age_s: Option<u64>,
}

/// Step of the on-screen HX711 scale calibration wizard, started by holding Button1 for 3s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationStep {
    /// Ask the user to empty the left scale; a short press confirms and starts taring.
    ConfirmEmptyLeft,
    /// Averaging raw samples for the left scale zero-point.
    TaringLeft,
    /// Ask the user to place the reference weight on the left scale.
    ConfirmReferenceLeft,
    /// Averaging raw samples with the left scale reference weight.
    CalibratingLeft,
    /// Ask the user to empty the right scale; a short press confirms and starts taring.
    ConfirmEmptyRight,
    /// Averaging raw samples for the right scale zero-point.
    TaringRight,
    /// Ask the user to place the reference weight on the right scale.
    ConfirmReferenceRight,
    /// Averaging raw samples with the right scale reference weight.
    CalibratingRight,
    /// Final result, shown briefly before returning to the previous screen.
    Done { success: bool },
}

/// Application errors
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppError {
    /// Water refill is needed
    WaterRefillNeeded { code: u16 },
    /// Machine is offline
    MachineOffline,
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::WaterRefillNeeded { code } => {
                write!(f, "Water refill needed (code: {})", code)
            }
            AppError::MachineOffline => write!(f, "Machine is offline"),
        }
    }
}

/// Global application state
/// Single point of state management for the entire application
#[derive(Debug)]
pub struct GlobalAppState {
    /// Currently displayed screen
    pub current_screen: Screen,
    /// State of the extraction process
    pub extraction_state: ExtractionState,
    /// Machine state (mode, temperatures, pump, etc.)
    pub machine_state: MachineState,
    pub events_log: VecDeque<String>,
    pub wifi_status: ConnectionStatus,
    pub mqtt_status: ConnectionStatus,
    pub cup_counter: Option<u64>,
    pub outbound_mqtt: VecDeque<MqttOutboundMessage>,
    /// Current error (if any)
    pub error: Option<AppError>,
    /// Screen to return to when toggling out of Debug
    pub screen_before_debug: Option<Screen>,
    /// Backlight toggle during the loading phase (short press on/off); ignored after first UART frame
    pub backlight_on: bool,
    /// Last time UART telemetry or a button press was received (for backlight timeout)
    pub last_activity_at: Option<Instant>,
    /// Last time a UART telemetry frame was received (for Debug screen activity marker)
    pub last_uart_frame_at: Option<Instant>,
    /// Time when the last shot ended (pump turned off after a valid extraction)
    pub last_shot_ended_at: Option<Instant>,
    /// MQTT topic prefix (e.g. "mara"), mirrored from AppConfig for use in HA state topics
    pub mqtt_topic_prefix: String,
    /// Board metadata (WiFi RSSI, IP, uptime, free heap)
    pub device_info: DeviceInfo,
    /// Current boot loading stage; `None` before first stage fires, frozen at 100% while waiting for machine
    pub loading_status: Option<(&'static str, u8)>,
    /// Whether the dashboard is being shown for standalone scale testing without Mara telemetry.
    pub offline_mode: bool,
    /// Set by the state machine to ask the render loop for a full `terminal.clear()`
    /// (new session, Debug toggle). Drained once per frame via `take_redraw_request`.
    pub needs_terminal_clear: bool,
    /// Latest raw scale reading in decigrams (0.1g), before the session tare is applied.
    pub last_raw_weight_dg: Option<i32>,
    /// Scale reading shown on screen: `last_raw_weight_dg` minus `scale_tare_dg`.
    pub weight_dg: Option<i32>,
    /// Raw reading captured at the moment the last shot started; subtracted from
    /// subsequent readings so the Dashboard shows net (cup-tared) extracted weight.
    pub scale_tare_dg: i32,
    /// Current step of the scale calibration wizard, if active (started by a 3s button hold).
    pub calibration_step: Option<CalibrationStep>,
    /// Screen to return to once calibration finishes or is cancelled.
    pub screen_before_calibration: Option<Screen>,
    /// Latest raw (uncalibrated) HX711 ADC count from the left cell. Only populated by the
    /// `scale-test` build variant, which bypasses calibration entirely.
    pub raw_weight_left: Option<i32>,
    /// Latest raw (uncalibrated) HX711 ADC count from the right cell. Only populated by the
    /// `scale-test` build variant.
    pub raw_weight_right: Option<i32>,
    /// Rolling `(elapsed_secs_since_shot_start, weight_dg)` samples captured while a shot is
    /// extracting, used to compute and render the live flow rate. Cleared on `ShotStarted`;
    /// left as-is once the shot ends, so the Dashboard keeps showing that shot's flow profile
    /// until the next one starts.
    pub flow_samples: VecDeque<(f32, i32)>,
}

impl Default for GlobalAppState {
    fn default() -> Self {
        Self {
            current_screen: Screen::default(),
            extraction_state: ExtractionState::default(),
            machine_state: MachineState::default(),
            events_log: VecDeque::with_capacity(10),
            wifi_status: ConnectionStatus::default(),
            mqtt_status: ConnectionStatus::default(),
            cup_counter: None,
            outbound_mqtt: VecDeque::with_capacity(32),
            error: None,
            screen_before_debug: None,
            backlight_on: true,
            last_activity_at: None,
            last_uart_frame_at: None,
            last_shot_ended_at: None,
            mqtt_topic_prefix: "mara".to_string(),
            device_info: DeviceInfo::default(),
            loading_status: None,
            offline_mode: false,
            needs_terminal_clear: false,
            last_raw_weight_dg: None,
            weight_dg: None,
            scale_tare_dg: 0,
            calibration_step: None,
            screen_before_calibration: None,
            raw_weight_left: None,
            raw_weight_right: None,
            flow_samples: VecDeque::with_capacity(FLOW_SAMPLES_CAP),
        }
    }
}

impl GlobalAppState {
    /// Get the duration of the current extraction in seconds
    pub fn extraction_duration_secs(&self) -> u64 {
        self.extraction_state
            .elapsed()
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Check if extraction is currently in progress
    pub fn is_extracting(&self) -> bool {
        self.extraction_state.is_extracting()
    }

    /// Check if there is an error
    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Clear the error
    pub fn clear_error(&mut self) {
        self.error = None;
    }

    /// Record a flow sample at `elapsed_s` seconds into the current shot, using the current
    /// `weight_dg`. No-op if the scale hasn't reported a weight yet.
    pub fn record_flow_sample(&mut self, elapsed_s: f32) {
        let Some(weight_dg) = self.weight_dg else {
            return;
        };
        self.flow_samples.push_back((elapsed_s, weight_dg));
        if self.flow_samples.len() > FLOW_SAMPLES_CAP {
            self.flow_samples.pop_front();
        }
    }

    /// Discard all recorded flow samples (called on `ShotStarted` so a new shot starts clean).
    pub fn clear_flow_samples(&mut self) {
        self.flow_samples.clear();
    }

    /// Live flow rate in g/s, averaged across the whole recorded sample window. `None` until
    /// at least two samples spanning a nontrivial time delta have been collected, so a couple
    /// of readings a few milliseconds apart can't produce a wildly inflated rate.
    pub fn flow_rate_g_per_s(&self) -> Option<f64> {
        let &(t0, w0) = self.flow_samples.front()?;
        let &(t1, w1) = self.flow_samples.back()?;
        let dt = (t1 - t0) as f64;
        if dt < 0.5 {
            return None;
        }
        let dw_g = (w1 - w0) as f64 / 10.0;
        Some((dw_g / dt).max(0.0))
    }

    pub fn enqueue_mqtt_message(
        &mut self,
        topic_suffix: impl Into<String>,
        payload: impl Into<String>,
    ) {
        self.outbound_mqtt.push_back(MqttOutboundMessage {
            topic_suffix: topic_suffix.into(),
            payload: payload.into(),
            retain: false,
            absolute: false,
        });
        while self.outbound_mqtt.len() > 64 {
            self.outbound_mqtt.pop_front();
        }
    }

    /// Enqueue a message with an absolute topic (not prefixed) and configurable retain flag.
    pub fn enqueue_absolute_mqtt_message(
        &mut self,
        topic: impl Into<String>,
        payload: impl Into<String>,
        retain: bool,
    ) {
        self.outbound_mqtt.push_back(MqttOutboundMessage {
            topic_suffix: topic.into(),
            payload: payload.into(),
            retain,
            absolute: true,
        });
        while self.outbound_mqtt.len() > 64 {
            self.outbound_mqtt.pop_front();
        }
    }

    pub fn take_outbound_mqtt_messages(&mut self) -> Vec<MqttOutboundMessage> {
        self.outbound_mqtt.drain(..).collect()
    }

    /// Returns `true` if the backlight should be on (activity within the last `BACKLIGHT_TIMEOUT`)
    pub fn backlight_should_be_on(&self, now: Instant) -> bool {
        match self.last_activity_at {
            Some(last) => now.saturating_duration_since(last) < BACKLIGHT_TIMEOUT,
            None => false,
        }
    }

    /// Returns `true` while telemetry is fresh enough to consider the machine online
    /// (a UART frame arrived within the last `MACHINE_OFFLINE_TIMEOUT`).
    pub fn machine_online(&self, now: Instant) -> bool {
        if self.offline_mode {
            return true;
        }
        match self.last_uart_frame_at {
            Some(last) => now.saturating_duration_since(last) < MACHINE_OFFLINE_TIMEOUT,
            None => false,
        }
    }

    /// Ask the render loop to perform a full `terminal.clear()` on the next frame.
    pub fn request_redraw(&mut self) {
        self.needs_terminal_clear = true;
    }

    /// Consume a pending redraw request, returning whether the terminal should be cleared.
    pub fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.needs_terminal_clear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extraction_state_idle() {
        let state = ExtractionState::Idle {
            last_extraction_duration: None,
        };
        assert!(!state.is_extracting());
        assert_eq!(state.elapsed(), None);
        assert_eq!(state.last_extraction_duration(), None);
    }

    #[test]
    fn test_extraction_state_idle_with_duration() {
        let duration = Duration::from_secs(30);
        let state = ExtractionState::Idle {
            last_extraction_duration: Some(duration),
        };
        assert!(!state.is_extracting());
        assert_eq!(state.last_extraction_duration(), Some(duration));
    }

    #[test]
    fn test_extraction_state_extracting() {
        let now = Instant::now();
        let state = ExtractionState::Extracting { started_at: now };
        assert!(state.is_extracting());
        assert!(state.elapsed().is_some());
        assert_eq!(state.last_extraction_duration(), None);
    }

    #[test]
    fn test_global_app_state_default() {
        let state = GlobalAppState::default();
        assert_eq!(state.current_screen, Screen::default());
        assert_eq!(
            state.extraction_state,
            ExtractionState::Idle {
                last_extraction_duration: None
            }
        );
        assert_eq!(state.machine_state.last_frame, None);
        assert_eq!(state.cup_counter, None);
        assert_eq!(state.error, None);
    }

    #[test]
    fn test_global_app_state_extraction_duration() {
        let mut state = GlobalAppState::default();
        assert_eq!(state.extraction_duration_secs(), 0);

        state.extraction_state = ExtractionState::Extracting {
            started_at: Instant::now(),
        };
        assert!(state.extraction_duration_secs() > 0 || state.extraction_duration_secs() == 0);
    }

    #[test]
    fn test_global_app_state_error() {
        let mut state = GlobalAppState::default();
        assert!(!state.has_error());

        state.error = Some(AppError::WaterRefillNeeded { code: 65 });
        assert!(state.has_error());

        state.clear_error();
        assert!(!state.has_error());
    }

    #[test]
    fn test_flow_rate_requires_at_least_two_samples_spanning_half_a_second() {
        let mut state = GlobalAppState::default();
        assert_eq!(state.flow_rate_g_per_s(), None);

        state.weight_dg = Some(50);
        state.record_flow_sample(0.0);
        assert_eq!(state.flow_rate_g_per_s(), None);

        // Same instant twice (dt == 0) must not be treated as an infinite rate.
        state.record_flow_sample(0.0);
        assert_eq!(state.flow_rate_g_per_s(), None);
    }

    #[test]
    fn test_flow_rate_averages_over_the_full_recorded_window() {
        let mut state = GlobalAppState::default();
        state.weight_dg = Some(0);
        state.record_flow_sample(0.0);
        state.weight_dg = Some(50); // +5.0g
        state.record_flow_sample(1.0);
        state.weight_dg = Some(150); // +10.0g more, but 2s elapsed this leg
        state.record_flow_sample(3.0);

        // Total: 15.0g over 3.0s = 5.0 g/s, regardless of the uneven step sizes in between.
        assert_eq!(state.flow_rate_g_per_s(), Some(5.0));
    }

    #[test]
    fn test_clear_flow_samples() {
        let mut state = GlobalAppState::default();
        state.weight_dg = Some(10);
        state.record_flow_sample(0.0);
        state.record_flow_sample(1.0);
        assert!(!state.flow_samples.is_empty());

        state.clear_flow_samples();
        assert!(state.flow_samples.is_empty());
    }

    #[test]
    fn test_backlight_off_by_default() {
        let state = GlobalAppState::default();
        assert!(!state.backlight_should_be_on(Instant::now()));
    }

    #[test]
    fn test_backlight_on_after_activity() {
        let mut state = GlobalAppState::default();
        state.last_activity_at = Some(Instant::now());
        assert!(state.backlight_should_be_on(Instant::now()));
    }

    #[test]
    fn test_backlight_off_after_timeout() {
        let mut state = GlobalAppState::default();
        let past = Instant::now() - BACKLIGHT_TIMEOUT - Duration::from_millis(1);
        state.last_activity_at = Some(past);
        assert!(!state.backlight_should_be_on(Instant::now()));
    }
}
