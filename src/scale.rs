//! Pure calibration math for the HX711 load-cell amplifier (scale).
//!
//! Kept hardware-agnostic (no GPIO/NVS access) so it can be unit tested on the host and
//! reused identically by the device driver (`hx711.rs`) and the simulator.

/// Reference weight (in grams) used by the calibration wizard.
pub const CALIBRATION_REFERENCE_G: f32 = 500.0;
const MIN_WEIGHT_DG: i32 = -500;
const MAX_WEIGHT_DG: i32 = 30_000;
const MAX_STEP_DG: i32 = 2_500;

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

/// Filters disconnected, corrupt, and implausibly jumping HX711 samples.
#[derive(Debug, Default)]
pub struct ScaleReadingFilter {
    last_raw: Option<i32>,
}

impl ScaleReadingFilter {
    /// Accept a raw sample and return calibrated decigrams, or ignore the sample.
    pub fn accept(&mut self, raw: i32, calibration: ScaleCalibration) -> Option<i32> {
        if !(-8_388_608..=8_388_607).contains(&raw) || !calibration.is_valid() {
            return None;
        }

        let weight_dg = calibration.raw_to_decigrams(raw);
        if !(MIN_WEIGHT_DG..=MAX_WEIGHT_DG).contains(&weight_dg) {
            return None;
        }

        if let Some(last_raw) = self.last_raw {
            let max_raw_step = (calibration.scale.abs() * MAX_STEP_DG as f32 / 10.0) as i32;
            if (i64::from(raw) - i64::from(last_raw)).abs() > i64::from(max_raw_step.max(1)) {
                return None;
            }
        }

        self.last_raw = Some(raw);
        Some(weight_dg)
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

    #[test]
    fn filter_ignores_large_jumps() {
        let mut filter = ScaleReadingFilter::default();
        let calibration = ScaleCalibration {
            offset: 1000,
            scale: 20.0,
        };
        assert_eq!(filter.accept(1000, calibration), Some(0));
        assert_eq!(filter.accept(1000 + 20 * 300, calibration), None);
    }
}
