use core::fmt;
use std::time::{Duration, Instant};

/// Type of button press: short, long, or a fast double short-press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonPressType {
    Short,
    Long,
    /// Two Short presses released in quick succession (see `DOUBLE_PRESS_WINDOW_MS`).
    Double,
}

impl fmt::Display for ButtonPressType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ButtonPressType::Short => write!(f, "Short Press"),
            ButtonPressType::Long => write!(f, "Long Press"),
            ButtonPressType::Double => write!(f, "Double Press"),
        }
    }
}

/// Button enum representing different button actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Button1(ButtonPressType),
}

impl fmt::Display for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Button::Button1(press_type) => write!(f, "Button 1 ({press_type})"),
        }
    }
}

impl Button {
    /// Check if a short press was detected.
    pub fn is_short_press(&self) -> bool {
        matches!(self, Button::Button1(ButtonPressType::Short))
    }

    /// Check if a long press was detected.
    pub fn is_long_press(&self) -> bool {
        matches!(self, Button::Button1(ButtonPressType::Long))
    }

    /// Check if a fast double press was detected.
    pub fn is_double_press(&self) -> bool {
        matches!(self, Button::Button1(ButtonPressType::Double))
    }
}

/// Maximum gap between the release of one Short press and the release of the next for the
/// pair to be reclassified as a single `ButtonPressType::Double`.
const DOUBLE_PRESS_WINDOW_MS: u64 = 400;

/// State of a button, tracking press duration.
#[derive(Default)]
pub struct ButtonState {
    pressed_at: Option<Instant>,
    /// Set once `held_for` has fired for the current press, so `update` doesn't also
    /// classify the eventual release as a Short/Long press.
    hold_consumed: bool,
    /// A Short press release currently on hold, waiting to see whether a second Short
    /// follows within `DOUBLE_PRESS_WINDOW_MS` to pair into a `ButtonPressType::Double`.
    /// A lone Short must *not* fire the moment it's released — a caller reacting to it
    /// immediately (e.g. switching screens) would misfire on the first half of every
    /// double press. Resolved either by pairing (in `update`) or by timing out (in `poll`,
    /// which must be called regularly regardless of press state for this to ever fire).
    pending_short_at: Option<Instant>,
}

impl ButtonState {
    /// Update the button state based on whether it is currently pressed.
    ///
    /// A Long press (or a Short paired into a Double) calls `on_press` immediately on
    /// release. A lone Short is held back instead — call `poll` every tick to eventually
    /// dispatch it once the double-press window has passed without a pairing second press.
    pub fn update<F>(&mut self, is_pressed: bool, on_press: F)
    where
        F: FnOnce(ButtonPressType),
    {
        if is_pressed {
            // Button is currently down
            if self.pressed_at.is_none() {
                self.pressed_at = Some(Instant::now());
                self.hold_consumed = false;
            }
        } else if let Some(pressed_at) = self.pressed_at.take() {
            // Button just released
            let consumed = std::mem::take(&mut self.hold_consumed);
            if consumed {
                return;
            }

            let now = Instant::now();
            let duration = now.saturating_duration_since(pressed_at).as_millis() as u64;
            if duration < 500 {
                let is_double = self.pending_short_at.take().is_some_and(|pending| {
                    now.saturating_duration_since(pending).as_millis() as u64
                        <= DOUBLE_PRESS_WINDOW_MS
                });
                if is_double {
                    on_press(ButtonPressType::Double);
                } else {
                    // Hold it back; `poll` fires it as a plain Short if nothing pairs with it.
                    self.pending_short_at = Some(now);
                }
            } else {
                on_press(ButtonPressType::Long);
            }
        }
    }

    /// Dispatches a Short press that was held back by `update` once the double-press window
    /// has elapsed without a second Short arriving to pair it into a Double. Call this every
    /// main-loop tick, independent of the button's current pressed state — a lone Short only
    /// ever fires from here, never from `update` itself.
    pub fn poll<F>(&mut self, on_press: F)
    where
        F: FnOnce(ButtonPressType),
    {
        if let Some(pending) = self.pending_short_at
            && Instant::now().saturating_duration_since(pending).as_millis() as u64
                > DOUBLE_PRESS_WINDOW_MS
        {
            self.pending_short_at = None;
            on_press(ButtonPressType::Short);
        }
    }

    /// Returns `true` exactly once per press, the moment `threshold` has elapsed while the
    /// button is still held down (e.g. "hold 3s to start scale calibration"). Once fired,
    /// the eventual release no longer triggers a Short/Long press via `update`.
    pub fn held_for(&mut self, is_pressed: bool, threshold: Duration) -> bool {
        if !is_pressed {
            return false;
        }
        let Some(pressed_at) = self.pressed_at else {
            return false;
        };
        if !self.hold_consumed && pressed_at.elapsed() >= threshold {
            self.hold_consumed = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_button_press_type_display() {
        assert_eq!(ButtonPressType::Short.to_string(), "Short Press");
        assert_eq!(ButtonPressType::Long.to_string(), "Long Press");
    }

    #[test]
    fn test_button_display() {
        assert_eq!(
            Button::Button1(ButtonPressType::Short).to_string(),
            "Button 1 (Short Press)"
        );
        assert_eq!(
            Button::Button1(ButtonPressType::Long).to_string(),
            "Button 1 (Long Press)"
        );
    }

    #[test]
    fn test_button_is_short_press() {
        assert!(Button::Button1(ButtonPressType::Short).is_short_press());
        assert!(!Button::Button1(ButtonPressType::Long).is_short_press());
    }

    #[test]
    fn test_button_is_long_press() {
        assert!(Button::Button1(ButtonPressType::Long).is_long_press());
        assert!(!Button::Button1(ButtonPressType::Short).is_long_press());
    }

    #[test]
    fn test_button_state_short_press_is_held_back_until_poll() {
        let mut state = ButtonState::default();
        let mut received: Option<ButtonPressType> = None;

        // Press and release: not dispatched yet from `update` — it might still pair into a
        // Double if a second Short follows within the window.
        state.update(true, |_| {});
        state.update(false, |t| received = Some(t));
        assert_eq!(received, None);

        // Nothing paired with it, but the window hasn't elapsed yet either.
        state.poll(|t| received = Some(t));
        assert_eq!(received, None);
    }

    #[test]
    #[ignore = "requires sleeping past the double-press window"]
    fn test_button_state_lone_short_press_fires_from_poll_after_the_window() {
        let mut state = ButtonState::default();
        let mut received: Option<ButtonPressType> = None;

        state.update(true, |_| {});
        state.update(false, |_| {});
        std::thread::sleep(Duration::from_millis(DOUBLE_PRESS_WINDOW_MS + 100));
        state.poll(|t| received = Some(t));

        assert_eq!(received, Some(ButtonPressType::Short));
    }

    #[test]
    fn test_button_is_double_press() {
        assert!(Button::Button1(ButtonPressType::Double).is_double_press());
        assert!(!Button::Button1(ButtonPressType::Short).is_double_press());
    }

    #[test]
    fn test_button_state_fast_second_short_press_is_a_double() {
        let mut state = ButtonState::default();
        let mut received: Vec<ButtonPressType> = Vec::new();

        // First short press: held back, not dispatched.
        state.update(true, |_| {});
        state.update(false, |t| received.push(t));
        // Second short press, immediately after (well within the double-press window): pairs
        // with the first and fires immediately as a Double — no need to wait for `poll`.
        state.update(true, |_| {});
        state.update(false, |t| received.push(t));

        assert_eq!(received, vec![ButtonPressType::Double]);
    }

    #[test]
    fn test_button_state_a_third_fast_press_starts_a_new_double_press_pair() {
        let mut state = ButtonState::default();
        let mut received: Vec<ButtonPressType> = Vec::new();

        for _ in 0..3 {
            state.update(true, |_| {});
            state.update(false, |t| received.push(t));
        }

        // Press 1+2 pair up into a Double, fired immediately. Press 3 has nothing left to pair
        // with, so it's held back the same way a lone Short always is — `poll` would eventually
        // dispatch it, but that's covered by the poll-specific tests above.
        assert_eq!(received, vec![ButtonPressType::Double]);
    }

    #[test]
    #[ignore = "requires sleeping past the double-press window"]
    fn test_button_state_slow_second_short_press_stays_two_shorts() {
        let mut state = ButtonState::default();
        let mut received: Vec<ButtonPressType> = Vec::new();

        state.update(true, |_| {});
        state.update(false, |_| {});
        std::thread::sleep(Duration::from_millis(DOUBLE_PRESS_WINDOW_MS + 100));
        // The first Short's window has elapsed: `poll` dispatches it before the second press
        // even starts, so the second press pairs with nothing and is held back in turn.
        state.poll(|t| received.push(t));
        state.update(true, |_| {});
        state.update(false, |t| received.push(t));

        assert_eq!(received, vec![ButtonPressType::Short]);
    }

    #[test]
    fn test_button_state_no_callback_when_not_pressed() {
        let mut state = ButtonState::default();
        let mut called = false;

        state.update(false, |_| called = true);

        assert!(!called);
    }

    #[test]
    fn test_button_state_held_does_not_fire() {
        let mut state = ButtonState::default();
        let mut count = 0;

        // Multiple held-down ticks without release
        state.update(true, |_| count += 1);
        state.update(true, |_| count += 1);
        state.update(true, |_| count += 1);

        assert_eq!(count, 0);
    }

    #[test]
    #[ignore = "requires 600ms sleep to cross the long-press threshold"]
    fn test_button_state_long_press() {
        let mut state = ButtonState::default();
        let mut received: Option<ButtonPressType> = None;

        state.update(true, |_| {});
        std::thread::sleep(std::time::Duration::from_millis(600));
        state.update(false, |t| received = Some(t));

        assert_eq!(received, Some(ButtonPressType::Long));
    }

    #[test]
    fn test_held_for_false_before_threshold() {
        let mut state = ButtonState::default();
        state.update(true, |_| {});
        assert!(!state.held_for(true, Duration::from_secs(3)));
    }

    #[test]
    fn test_held_for_false_when_not_pressed() {
        let mut state = ButtonState::default();
        assert!(!state.held_for(false, Duration::from_millis(0)));
    }

    #[test]
    #[ignore = "requires a real hold delay to cross the threshold"]
    fn test_held_for_fires_once_then_suppresses_release_classification() {
        let mut state = ButtonState::default();
        let mut released: Option<ButtonPressType> = None;

        state.update(true, |_| {});
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(state.held_for(true, Duration::from_millis(20)));
        // Fires only once even if still held and threshold keeps being exceeded.
        assert!(!state.held_for(true, Duration::from_millis(20)));

        state.update(false, |t| released = Some(t));
        // The hold already consumed this press, so release must not also fire Short/Long.
        assert_eq!(released, None);
    }
}
