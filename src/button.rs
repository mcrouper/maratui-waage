use core::fmt;
use std::time::{Duration, Instant};

/// Type of button press: short or long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonPressType {
    Short,
    Long,
}

impl fmt::Display for ButtonPressType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ButtonPressType::Short => write!(f, "Short Press"),
            ButtonPressType::Long => write!(f, "Long Press"),
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
}

/// State of a button, tracking press duration.
#[derive(Default)]
pub struct ButtonState {
    pressed_at: Option<Instant>,
    /// Set once `held_for` has fired for the current press, so `update` doesn't also
    /// classify the eventual release as a Short/Long press.
    hold_consumed: bool,
}

impl ButtonState {
    /// Update the button state based on whether it is currently pressed.
    ///
    /// If the button was just released, it calls the `on_press` callback with the type of press
    /// detected. Suppressed if `held_for` already consumed this press.
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

            let duration = pressed_at.elapsed().as_millis() as u64;
            let press_type = if duration < 500 {
                Some(ButtonPressType::Short)
            } else {
                Some(ButtonPressType::Long)
            };

            if let Some(press_type) = press_type {
                on_press(press_type);
            }
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
    fn test_button_state_short_press() {
        let mut state = ButtonState::default();
        let mut received: Option<ButtonPressType> = None;

        // Press
        state.update(true, |_| {});
        // Release immediately → short press
        state.update(false, |t| received = Some(t));

        assert_eq!(received, Some(ButtonPressType::Short));
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
