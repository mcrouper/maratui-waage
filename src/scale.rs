//! Pure calibration math for the HX711 load-cell amplifier (scale).
//!
//! Kept hardware-agnostic (no GPIO/NVS access) so it can be unit tested on the host and
//! reused identically by the device driver (`hx711.rs`) and the simulator.

/// Reference weight (in grams) used by the calibration wizard.
pub const CALIBRATION_REFERENCE_G: f32 = 500.0;
const MIN_WEIGHT_DG: i32 = -500;
const MAX_WEIGHT_DG: i32 = 30_000;

/// Converts raw HX711 ADC counts into grams.
///
/// `offset` is the raw reading with an empty scale (the zero-point / tare from the
/// factory `tare` calibration step). `scale` is raw counts per gram above `offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaleCalibration {
    pub offset: i32,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DualScaleCalibration {
    pub left: ScaleCalibration,
    pub right: ScaleCalibration,
}

impl Default for ScaleCalibration {
    fn default() -> Self {
        // Zero marks the scale as uncalibrated until `tare` + `calibrate` have been run.
        Self {
            offset: 0,
            scale: 0.0,
        }
    }
}

impl Default for DualScaleCalibration {
    fn default() -> Self {
        Self {
            left: ScaleCalibration::default(),
            right: ScaleCalibration::default(),
        }
    }
}

impl ScaleCalibration {
    /// Returns whether the stored calibration can produce meaningful readings.
    pub fn is_valid(self) -> bool {
        self.scale.is_finite() && self.scale > f32::EPSILON
    }

    /// Convert a raw ADC reading to grams (tenths of a gram, as an `i32`) using this calibration.
    pub fn raw_to_decigrams(&self, raw: i32) -> i32 {
        let grams = (raw - self.offset) as f32 / self.scale;
        (grams * 10.0).round() as i32
    }

    /// Derive a new calibration from a raw reading taken with `reference_g` grams resting
    /// on the scale, given the current zero-point `offset`.
    pub fn calibrate(offset: i32, raw_with_reference: i32, reference_g: f32) -> Self {
        let delta = (raw_with_reference - offset) as f32;
        let scale = if delta.abs() > f32::EPSILON && reference_g > f32::EPSILON {
            delta / reference_g
        } else {
            0.0
        };
        Self { offset, scale }
    }

    /// Serialize to 8 bytes (offset: i32 LE, scale: f32 LE) for NVS storage.
    pub fn to_bytes(self) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[0..4].copy_from_slice(&self.offset.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.scale.to_le_bytes());
        bytes
    }

    /// Deserialize from 8 bytes produced by [`Self::to_bytes`].
    pub fn from_bytes(bytes: [u8; 8]) -> Self {
        let offset = i32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let scale = f32::from_le_bytes(bytes[4..8].try_into().unwrap());
        Self { offset, scale }
    }
}

impl DualScaleCalibration {
    /// Convert a left and right raw reading into the summed weight in decigrams.
    pub fn raw_to_decigrams(self, left_raw: i32, right_raw: i32) -> i32 {
        let left_weight_dg = self.left.raw_to_decigrams(left_raw);
        let right_weight_dg = self.right.raw_to_decigrams(right_raw);
        left_weight_dg + right_weight_dg
    }

    /// Serialize left/right calibrations for NVS storage.
    pub fn to_bytes(self) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[0..8].copy_from_slice(&self.left.to_bytes());
        bytes[8..16].copy_from_slice(&self.right.to_bytes());
        bytes
    }

    /// Deserialize left/right calibrations from NVS bytes.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        let left = ScaleCalibration::from_bytes(bytes[0..8].try_into().unwrap());
        let right = ScaleCalibration::from_bytes(bytes[8..16].try_into().unwrap());
        Self { left, right }
    }
}

// Rolling-window trimmed-mean parameters, matching the `HX711_ADC` Arduino library's proven
// defaults (as used by e.g. the CleverCoffee espresso-PID firmware): average the last
// `WINDOW_SAMPLES` raw readings, but drop the highest `TRIM_HIGH` and lowest `TRIM_LOW` of
// them first so a single spike doesn't skew the result.
//
// Deliberately NOT a step-limiting filter (reject-if-too-different-from-last-accepted-value):
// that design can get permanently stuck — once a real, fast change (weight added/removed) gets
// rejected as "implausible", every later sample is still compared against the same stale
// reference and rejected too, so the old value never updates. A rolling window has no such
// failure mode: it always reflects the most recent samples, converging on a real change within
// one window's worth of readings instead of staying stuck indefinitely.
const WINDOW_SAMPLES: usize = 32;
const TRIM_HIGH: usize = 1;
const TRIM_LOW: usize = 1;

/// Smooths HX711 samples with a rolling trimmed-mean window and rejects disconnected, corrupt,
/// or wildly out-of-range readings before they ever enter the window.
#[derive(Debug, Default)]
pub struct ScaleReadingFilter {
    window: std::collections::VecDeque<i32>,
}

impl ScaleReadingFilter {
    /// Accept a raw sample and return the trimmed-mean calibrated decigrams, or ignore the
    /// sample entirely if it's out of range (before it can pollute the window).
    pub fn accept(&mut self, raw: i32, calibration: ScaleCalibration) -> Option<i32> {
        if !(-8_388_608..=8_388_607).contains(&raw) || !calibration.is_valid() {
            return None;
        }

        let weight_dg = calibration.raw_to_decigrams(raw);
        if !(MIN_WEIGHT_DG..=MAX_WEIGHT_DG).contains(&weight_dg) {
            return None;
        }

        self.window.push_back(raw);
        if self.window.len() > WINDOW_SAMPLES {
            self.window.pop_front();
        }

        let mut sorted: Vec<i32> = self.window.iter().copied().collect();
        sorted.sort_unstable();
        let (lo, hi) = if sorted.len() > TRIM_LOW + TRIM_HIGH {
            (TRIM_LOW, sorted.len() - TRIM_HIGH)
        } else {
            (0, sorted.len())
        };
        let trimmed = &sorted[lo..hi];
        let sum: i64 = trimmed.iter().map(|&v| i64::from(v)).sum();
        let mean_raw = (sum as f64 / trimmed.len() as f64).round() as i32;

        Some(calibration.raw_to_decigrams(mean_raw))
    }
}

/// Exponential-moving-average damper for raw (uncalibrated) ADC counts, used by the
/// `scale-test` build variant so on-screen jumps between samples stay small even without
/// `ScaleReadingFilter`'s calibration-based step limiting.
#[derive(Debug, Default)]
pub struct RawSignalSmoother {
    smoothed: Option<f32>,
}

impl RawSignalSmoother {
    /// Blend `raw` into the running average and return the smoothed value, rounded.
    ///
    /// `alpha` is the weight given to the new sample, in `(0.0, 1.0]`; smaller values damp
    /// harder (slower to move, smaller jumps) but lag more behind real changes.
    pub fn smooth(&mut self, raw: i32, alpha: f32) -> i32 {
        let raw = raw as f32;
        let next = match self.smoothed {
            Some(prev) => prev + alpha * (raw - prev),
            None => raw,
        };
        self.smoothed = Some(next);
        next.round() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_uncalibrated() {
        let cal = ScaleCalibration::default();
        assert_eq!(cal.offset, 0);
        assert_eq!(cal.scale, 0.0);
    }

    #[test]
    fn raw_to_decigrams_applies_offset_and_scale() {
        let cal = ScaleCalibration {
            offset: 1000,
            scale: 20.0,
        };
        // (3000 - 1000) / 20.0 = 100.0g -> 1000 decigrams
        assert_eq!(cal.raw_to_decigrams(3000), 1000);
        // at the zero point, weight is 0
        assert_eq!(cal.raw_to_decigrams(1000), 0);
    }

    #[test]
    fn calibrate_computes_scale_from_reference_weight() {
        let offset = 1000;
        let raw_with_500g = 11000; // 10000 counts above offset for 500g
        let cal = ScaleCalibration::calibrate(offset, raw_with_500g, CALIBRATION_REFERENCE_G);
        assert_eq!(cal.offset, offset);
        assert_eq!(cal.scale, 20.0);
        assert_eq!(cal.raw_to_decigrams(11000), 5000);
    }

    #[test]
    fn calibrate_avoids_division_by_zero() {
        let cal = ScaleCalibration::calibrate(1000, 1000, CALIBRATION_REFERENCE_G);
        assert_eq!(cal.scale, 0.0);
    }

    #[test]
    fn bytes_roundtrip() {
        let cal = ScaleCalibration {
            offset: -12345,
            scale: 420.5,
        };
        assert_eq!(ScaleCalibration::from_bytes(cal.to_bytes()), cal);
    }

    #[test]
    fn dual_scale_calibration_sums_left_and_right_weights() {
        let cal = DualScaleCalibration {
            left: ScaleCalibration {
                offset: 1000,
                scale: 20.0,
            },
            right: ScaleCalibration {
                offset: 2000,
                scale: 25.0,
            },
        };
        assert_eq!(cal.raw_to_decigrams(3000, 4500), 1000 + 1000);
    }

    #[test]
    fn dual_scale_bytes_roundtrip() {
        let cal = DualScaleCalibration {
            left: ScaleCalibration {
                offset: -12345,
                scale: 420.5,
            },
            right: ScaleCalibration {
                offset: 9876,
                scale: 77.5,
            },
        };
        assert_eq!(DualScaleCalibration::from_bytes(cal.to_bytes()), cal);
    }

    #[test]
    fn filter_ignores_invalid_calibration_and_out_of_range_values() {
        let mut filter = ScaleReadingFilter::default();
        assert_eq!(filter.accept(1000, ScaleCalibration::default()), None);
        let calibration = ScaleCalibration {
            offset: 1000,
            scale: 20.0,
        };
        assert_eq!(filter.accept(1000, calibration), Some(0));
        assert_eq!(filter.accept(1000 + 20 * 4000, calibration), None);
    }

    /// A single spike (e.g. one bad HX711 sample) is trimmed out of the window and barely
    /// moves the result, instead of being rejected-and-then-stuck like the old step filter.
    #[test]
    fn filter_trims_a_single_spike() {
        let mut filter = ScaleReadingFilter::default();
        let calibration = ScaleCalibration {
            offset: 1000,
            scale: 20.0,
        };
        for _ in 0..5 {
            filter.accept(1000, calibration);
        }
        // One wild outlier: gets trimmed as the window's single highest sample, so it has no
        // effect on the output at all.
        assert_eq!(filter.accept(1000 + 20 * 300, calibration), Some(0));
    }

    /// A sustained real change (e.g. lifting the cup off the scale) must fully take over the
    /// window, unlike the old step filter which could reject it forever.
    #[test]
    fn filter_converges_on_a_sustained_change() {
        let mut filter = ScaleReadingFilter::default();
        let calibration = ScaleCalibration {
            offset: 1000,
            scale: 20.0,
        };
        for _ in 0..WINDOW_SAMPLES {
            filter.accept(1000, calibration);
        }

        let new_raw = 1000 + 20 * 300; // a large, real change
        let mut last = None;
        for _ in 0..WINDOW_SAMPLES {
            last = filter.accept(new_raw, calibration);
        }
        // Once the old samples have fully aged out of the window, the reading matches the new
        // value exactly — it's not stuck at the old one.
        assert_eq!(last, Some(calibration.raw_to_decigrams(new_raw)));
    }

    /// Noise bouncing symmetrically around a steady weight averages back out, rather than
    /// drifting the reading away from the true value.
    #[test]
    fn filter_smooths_symmetric_noise() {
        let mut filter = ScaleReadingFilter::default();
        let calibration = ScaleCalibration {
            offset: 1000,
            scale: 20.0,
        };
        let mut last = None;
        for i in 0..WINDOW_SAMPLES {
            let jitter = if i % 2 == 0 { 5 } else { -5 };
            last = filter.accept(1000 + jitter, calibration);
        }
        assert_eq!(last, Some(0));
    }

    #[test]
    fn smoother_first_sample_passes_through() {
        let mut smoother = RawSignalSmoother::default();
        assert_eq!(smoother.smooth(1000, 0.2), 1000);
    }

    #[test]
    fn smoother_damps_a_sudden_jump() {
        let mut smoother = RawSignalSmoother::default();
        smoother.smooth(1000, 0.2);
        // A big jump only moves the smoothed value by `alpha` of the way there.
        let smoothed = smoother.smooth(2000, 0.2);
        assert_eq!(smoothed, 1200);
        assert!(smoothed < 2000);
    }

    #[test]
    fn smoother_converges_on_a_steady_input() {
        let mut smoother = RawSignalSmoother::default();
        let mut last = 0;
        for _ in 0..200 {
            last = smoother.smooth(5000, 0.2);
        }
        assert_eq!(last, 5000);
    }
}
