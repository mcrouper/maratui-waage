//! Water tank tracking: estimates how much water the machine draws from its tank, so the
//! Dashboard can show what's left and warn before it runs dry.
//!
//! Every pump run is booked once it's over, as one of three cases:
//! - **A — pump only** (boiler refill, rinsing): no back pressure, so the pump moves
//!   [`FREE_FLOW_ML_PER_S`] for as long as it runs.
//! - **B — shot measured**: the scale (tared at every pump start) shows what landed in the cup,
//!   1 g = 1 ml, read once it has settled after the pump stops, plus [`SHOT_CORRECTION_ML`] for
//!   water that never reaches the cup.
//! - **C — shot estimated**: no scale, so a run of at least [`MIN_SHOT_MS`] counts as a shot at
//!   [`ESTIMATED_SHOT_ML_PER_S`]; flagged as estimated ([`DrawKind::ShotEstimated`]).
//!
//! The Mara X only reports pump on/off, so whether a run was a shot is decided afterwards: with
//! the scale, by at least [`MIN_SHOT_WEIGHT_DG`] in the cup; without, by the run lasting at
//! least [`MIN_SHOT_MS`] (the shot timer's rule). Hardware-agnostic — millisecond timestamps
//! in, millilitres out — so it runs in host unit tests.

/// Tank capacity; a confirmed refill resets the level to this.
pub const TANK_CAPACITY_ML: u32 = 2300;
/// Remaining water at or below which the refill warning shows.
pub const LOW_WATER_ML: u32 = 150;
/// Case A: pump flow without back pressure (boiler refill, rinsing).
pub const FREE_FLOW_ML_PER_S: f32 = 5.0;
/// Case C: assumed shot flow when no scale is available.
pub const ESTIMATED_SHOT_ML_PER_S: f32 = 1.0;
/// Case B: water a shot draws beyond what lands in the cup — held back in the coffee puck and
/// released into the drip tray through the valve after the shot.
pub const SHOT_CORRECTION_ML: u32 = 25;
/// With the scale: net cup weight (decigrams) from which a pump run counts as a shot.
pub const MIN_SHOT_WEIGHT_DG: i32 = 30;
/// Without the scale: pump run from which it counts as a shot (the shot timer's 15 s rule).
pub const MIN_SHOT_MS: u64 = 15_000;
/// After the pump stops, the cup weight counts as settled once it stays within
/// `SETTLE_TOLERANCE_DG` for `SETTLE_WINDOW_MS`, or at the latest after `SETTLE_TIMEOUT_MS`.
pub const SETTLE_WINDOW_MS: u64 = 2_000;
pub const SETTLE_TOLERANCE_DG: i32 = 5;
pub const SETTLE_TIMEOUT_MS: u64 = 20_000;
/// A weight drop this far below the peak while settling means the cup was lifted off: book
/// the peak right away.
pub const CUP_REMOVED_DROP_DG: i32 = 50;
/// Save the level to flash once it moved this far from the last saved value.
pub const PERSIST_STEP_ML: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawKind {
    /// Case A: pump ran without a shot.
    PumpOnly,
    /// Case B: shot weighed on the scale.
    ShotMeasured,
    /// Case C: shot without scale, volume estimated from its duration.
    ShotEstimated,
}

impl DrawKind {
    /// Snake-case name used in the `<prefix>/water` MQTT payload.
    pub fn as_str(self) -> &'static str {
        match self {
            DrawKind::PumpOnly => "pump_only",
            DrawKind::ShotMeasured => "shot_measured",
            DrawKind::ShotEstimated => "shot_estimated",
        }
    }
}

/// Water booked off the tank for one pump run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Draw {
    pub ml: u32,
    pub kind: DrawKind,
}

impl Draw {
    pub fn is_estimated(&self) -> bool {
        self.kind == DrawKind::ShotEstimated
    }
}

#[derive(Debug)]
pub struct WaterTank {
    remaining_ml: u32,
    shot_correction_ml: u32,
    pump_started_ms: Option<u64>,
    /// Pump has stopped, waiting for the cup weight to settle before booking.
    settling: Option<Settling>,
    last_draw: Option<Draw>,
    persisted_ml: u32,
    persist_now: bool,
}

#[derive(Debug)]
struct Settling {
    pump_ms: u64,
    peak_dg: i32,
    /// Weight and time the current stability window started at.
    window_dg: i32,
    window_start_ms: u64,
    deadline_ms: u64,
}

impl Default for WaterTank {
    fn default() -> Self {
        Self::new(TANK_CAPACITY_ML)
    }
}

impl WaterTank {
    /// A tank holding `remaining_ml` (e.g. restored from flash), clamped to the capacity.
    pub fn new(remaining_ml: u32) -> Self {
        let remaining_ml = remaining_ml.min(TANK_CAPACITY_ML);
        Self {
            remaining_ml,
            shot_correction_ml: SHOT_CORRECTION_ML,
            pump_started_ms: None,
            settling: None,
            last_draw: None,
            persisted_ml: remaining_ml,
            persist_now: false,
        }
    }

    pub fn with_shot_correction(self, shot_correction_ml: u32) -> Self {
        Self {
            shot_correction_ml,
            ..self
        }
    }

    pub fn remaining_ml(&self) -> u32 {
        self.remaining_ml
    }

    /// `true` while the refill warning should show.
    pub fn is_low(&self) -> bool {
        self.remaining_ml <= LOW_WATER_ML
    }

    pub fn last_draw(&self) -> Option<Draw> {
        self.last_draw
    }

    /// Pump turned on. A previous run still settling is booked now with its peak weight.
    pub fn pump_started(&mut self, now_ms: u64) -> Option<Draw> {
        let booked = self.finish_settling();
        self.pump_started_ms = Some(now_ms);
        booked
    }

    /// Pump turned off. `cup_weight_dg` is the net scale weight if the scale is available:
    /// then the run is booked once the weight has settled (see [`Self::weight_sample`]),
    /// otherwise right away from its duration.
    pub fn pump_stopped(&mut self, now_ms: u64, cup_weight_dg: Option<i32>) -> Option<Draw> {
        let pump_ms = now_ms.saturating_sub(self.pump_started_ms.take()?);
        match cup_weight_dg {
            Some(weight_dg) => {
                self.settling = Some(Settling {
                    pump_ms,
                    peak_dg: weight_dg,
                    window_dg: weight_dg,
                    window_start_ms: now_ms,
                    deadline_ms: now_ms + SETTLE_TIMEOUT_MS,
                });
                None
            }
            None if pump_ms >= MIN_SHOT_MS => Some(self.book(Draw {
                ml: volume_ml(pump_ms, ESTIMATED_SHOT_ML_PER_S),
                kind: DrawKind::ShotEstimated,
            })),
            None => Some(self.book(pump_only(pump_ms))),
        }
    }

    /// New net scale reading; books a settling run once the weight is stable or the cup is
    /// lifted off.
    pub fn weight_sample(&mut self, now_ms: u64, weight_dg: i32) -> Option<Draw> {
        let settling = self.settling.as_mut()?;
        settling.peak_dg = settling.peak_dg.max(weight_dg);
        if weight_dg <= settling.peak_dg - CUP_REMOVED_DROP_DG {
            return self.finish_settling();
        }
        if (weight_dg - settling.window_dg).abs() > SETTLE_TOLERANCE_DG {
            settling.window_dg = weight_dg;
            settling.window_start_ms = now_ms;
        } else if now_ms.saturating_sub(settling.window_start_ms) >= SETTLE_WINDOW_MS {
            return self.finish_settling();
        }
        self.poll(now_ms)
    }

    /// Book a settling run whose timeout passed (e.g. the scale stopped reporting).
    pub fn poll(&mut self, now_ms: u64) -> Option<Draw> {
        if self
            .settling
            .as_ref()
            .is_some_and(|s| now_ms >= s.deadline_ms)
        {
            self.finish_settling()
        } else {
            None
        }
    }

    /// The user confirmed a refill.
    pub fn refilled(&mut self) {
        self.remaining_ml = TANK_CAPACITY_ML;
        self.persist_now = true;
    }

    /// The machine's own sensor reports the tank empty.
    pub fn mark_empty(&mut self) {
        self.remaining_ml = 0;
        self.persist_now = true;
    }

    /// The level to save to flash now, if any: after a refill or empty report, or once it
    /// moved [`PERSIST_STEP_ML`] from the last saved value. Bookings only happen when a pump
    /// run ends, so this never fires mid-run.
    pub fn take_level_to_persist(&mut self) -> Option<u32> {
        let moved = self.remaining_ml.abs_diff(self.persisted_ml) >= PERSIST_STEP_ML;
        if !(self.persist_now || moved) {
            return None;
        }
        self.persist_now = false;
        self.persisted_ml = self.remaining_ml;
        Some(self.remaining_ml)
    }

    fn finish_settling(&mut self) -> Option<Draw> {
        let settling = self.settling.take()?;
        let draw = if settling.peak_dg >= MIN_SHOT_WEIGHT_DG {
            Draw {
                ml: ((settling.peak_dg + 5) / 10) as u32 + self.shot_correction_ml,
                kind: DrawKind::ShotMeasured,
            }
        } else {
            pump_only(settling.pump_ms)
        };
        Some(self.book(draw))
    }

    fn book(&mut self, draw: Draw) -> Draw {
        self.remaining_ml = self.remaining_ml.saturating_sub(draw.ml);
        self.last_draw = Some(draw);
        draw
    }
}

fn pump_only(pump_ms: u64) -> Draw {
    Draw {
        ml: volume_ml(pump_ms, FREE_FLOW_ML_PER_S),
        kind: DrawKind::PumpOnly,
    }
}

fn volume_ml(duration_ms: u64, ml_per_s: f32) -> u32 {
    (duration_ms as f32 / 1000.0 * ml_per_s).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full tank without the shot correction, so drawn volumes are easy to read off.
    fn tank() -> WaterTank {
        WaterTank::default().with_shot_correction(0)
    }

    /// Feed `weight_dg` every 100 ms from `from_ms` for `for_ms`, returning the first booking.
    fn hold_weight(
        tank: &mut WaterTank,
        from_ms: u64,
        for_ms: u64,
        weight_dg: i32,
    ) -> Option<Draw> {
        (from_ms..=from_ms + for_ms)
            .step_by(100)
            .find_map(|t| tank.weight_sample(t, weight_dg))
    }

    #[test]
    fn pump_without_shot_draws_free_flow() {
        let mut tank = tank();
        tank.pump_started(0);
        let draw = tank.pump_stopped(10_000, None).unwrap();
        assert_eq!(
            draw,
            Draw {
                ml: 50,
                kind: DrawKind::PumpOnly
            }
        );
        assert_eq!(tank.remaining_ml(), TANK_CAPACITY_ML - 50);
    }

    #[test]
    fn shot_on_scale_books_settled_cup_weight() {
        let mut tank = tank();
        tank.pump_started(0);
        assert_eq!(tank.pump_stopped(25_000, Some(380)), None);
        // Still dripping: not booked yet.
        assert_eq!(tank.weight_sample(25_500, 395), None);
        let draw = hold_weight(&mut tank, 26_000, 3_000, 400).unwrap();
        assert_eq!(
            draw,
            Draw {
                ml: 40,
                kind: DrawKind::ShotMeasured
            }
        );
        assert_eq!(tank.remaining_ml(), TANK_CAPACITY_ML - 40);
    }

    #[test]
    fn shot_correction_is_added_to_measured_shots_only() {
        let mut tank = WaterTank::default().with_shot_correction(25);
        tank.pump_started(0);
        tank.pump_stopped(25_000, Some(400));
        assert_eq!(hold_weight(&mut tank, 25_000, 3_000, 400).unwrap().ml, 65);

        tank.pump_started(60_000);
        assert_eq!(tank.pump_stopped(85_000, None).unwrap().ml, 25);
    }

    #[test]
    fn shot_without_scale_is_estimated_from_duration() {
        let mut tank = tank();
        tank.pump_started(0);
        let draw = tank.pump_stopped(25_000, None).unwrap();
        assert_eq!(
            draw,
            Draw {
                ml: 25,
                kind: DrawKind::ShotEstimated
            }
        );
        assert!(draw.is_estimated());
    }

    #[test]
    fn short_shot_counts_when_the_cup_got_coffee() {
        let mut tank = tank();
        tank.pump_started(0);
        tank.pump_stopped(10_000, Some(200));
        let draw = hold_weight(&mut tank, 10_000, 3_000, 200).unwrap();
        assert_eq!(
            draw,
            Draw {
                ml: 20,
                kind: DrawKind::ShotMeasured
            }
        );
    }

    #[test]
    fn long_rinse_without_cup_is_pump_only() {
        let mut tank = tank();
        tank.pump_started(0);
        tank.pump_stopped(20_000, Some(2));
        let draw = hold_weight(&mut tank, 20_000, 3_000, 2).unwrap();
        assert_eq!(
            draw,
            Draw {
                ml: 100,
                kind: DrawKind::PumpOnly
            }
        );
    }

    #[test]
    fn lifting_the_cup_books_its_peak_weight() {
        let mut tank = tank();
        tank.pump_started(0);
        tank.pump_stopped(25_000, Some(400));
        assert_eq!(tank.weight_sample(25_500, 410), None);
        let draw = tank.weight_sample(26_000, -1_800).unwrap();
        assert_eq!(
            draw,
            Draw {
                ml: 41,
                kind: DrawKind::ShotMeasured
            }
        );
    }

    #[test]
    fn settling_times_out_without_readings() {
        let mut tank = tank();
        tank.pump_started(0);
        tank.pump_stopped(25_000, Some(400));
        assert_eq!(tank.poll(25_000 + SETTLE_TIMEOUT_MS - 1), None);
        assert_eq!(tank.poll(25_000 + SETTLE_TIMEOUT_MS).unwrap().ml, 40);
    }

    #[test]
    fn next_pump_start_books_a_run_still_settling() {
        let mut tank = tank();
        tank.pump_started(0);
        tank.pump_stopped(25_000, Some(400));
        assert_eq!(tank.pump_started(26_000).unwrap().ml, 40);
    }

    #[test]
    fn warning_shows_at_threshold_and_refill_resets() {
        let mut tank = WaterTank::new(LOW_WATER_ML + 5);
        assert!(!tank.is_low());
        tank.pump_started(0);
        tank.pump_stopped(1_000, None);
        assert!(tank.is_low());
        tank.refilled();
        assert_eq!(tank.remaining_ml(), TANK_CAPACITY_ML);
        assert!(!tank.is_low());
    }

    #[test]
    fn machine_empty_report_zeroes_the_tank() {
        let mut tank = tank();
        tank.mark_empty();
        assert_eq!(tank.remaining_ml(), 0);
        assert!(tank.is_low());
        // Draws never go below zero.
        tank.pump_started(0);
        tank.pump_stopped(10_000, None);
        assert_eq!(tank.remaining_ml(), 0);
    }

    #[test]
    fn persists_on_reset_and_every_ten_ml() {
        let mut tank = tank();
        assert_eq!(tank.take_level_to_persist(), None);
        tank.pump_started(0);
        tank.pump_stopped(1_000, None); // 5 ml
        assert_eq!(tank.take_level_to_persist(), None);
        tank.pump_started(2_000);
        tank.pump_stopped(3_000, None); // 10 ml in total
        assert_eq!(tank.take_level_to_persist(), Some(TANK_CAPACITY_ML - 10));
        assert_eq!(tank.take_level_to_persist(), None);
        tank.refilled();
        assert_eq!(tank.take_level_to_persist(), Some(TANK_CAPACITY_ML));
    }

    #[test]
    fn restored_level_is_clamped_to_capacity() {
        assert_eq!(WaterTank::new(9_999).remaining_ml(), TANK_CAPACITY_ML);
    }
}
