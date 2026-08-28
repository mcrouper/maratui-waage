//! Bit-banged driver for the HX711 24-bit ADC (load-cell amplifier).
//!
//! Protocol: `SCK` idles low. Once `DOUT` goes low, a conversion is ready; 24 clock
//! pulses shift out the result MSB-first, and one extra ("25th") pulse selects gain 128 /
//! channel A for the next conversion (the default and only mode used here).
//!
//! Each HX711 has its own `SCK` line (not shared) so the two ADCs can be clocked and read
//! fully independently.

use esp_idf_svc::hal::delay::Ets;
use esp_idf_svc::hal::gpio::{Gpio25, Gpio26, Gpio27, Gpio32, Input, Output, PinDriver};
use std::time::{Duration, Instant};

pub struct Hx711<'d> {
    dout_left: PinDriver<'d, Gpio25, Input>,
    dout_right: PinDriver<'d, Gpio32, Input>,
    sck_left: PinDriver<'d, Gpio26, Output>,
    sck_right: PinDriver<'d, Gpio27, Output>,
}

impl<'d> Hx711<'d> {
    pub fn new(
        dout_left: Gpio25,
        dout_right: Gpio32,
        sck_left: Gpio26,
        sck_right: Gpio27,
    ) -> anyhow::Result<Self> {
        let mut sck_left = PinDriver::output(sck_left)?;
        let mut sck_right = PinDriver::output(sck_right)?;
        let dout_left = PinDriver::input(dout_left)?;
        let dout_right = PinDriver::input(dout_right)?;
        sck_left.set_low()?;
        sck_right.set_low()?;
        Ok(Self {
            dout_left,
            dout_right,
            sck_left,
            sck_right,
        })
    }

    /// `true` once the left ADC has a conversion ready (`DOUT` idles low when ready).
    pub fn is_left_ready(&self) -> bool {
        self.dout_left.is_low()
    }

    /// `true` once the right ADC has a conversion ready (`DOUT` idles low when ready).
    pub fn is_right_ready(&self) -> bool {
        self.dout_right.is_low()
    }

    /// Read one 24-bit signed sample from the left HX711 (gain 128, channel A).
    pub fn read_left_raw(&mut self) -> Option<i32> {
        if !self.is_left_ready() {
            return None;
        }

        let mut value: u32 = 0;
        for _ in 0..24 {
            self.sck_left.set_high().ok()?;
            Ets::delay_us(1);
            value = (value << 1) | u32::from(self.dout_left.is_high());
            self.sck_left.set_low().ok()?;
            Ets::delay_us(1);
        }
        self.sck_left.set_high().ok()?;
        Ets::delay_us(1);
        self.sck_left.set_low().ok()?;
        Ets::delay_us(1);

        if value & 0x0080_0000 != 0 {
            value |= 0xFF00_0000;
        }
        Some(value as i32)
    }

    /// Read one 24-bit signed sample from the right HX711 (gain 128, channel A).
    pub fn read_right_raw(&mut self) -> Option<i32> {
        if !self.is_right_ready() {
            return None;
        }

        let mut value: u32 = 0;
        for _ in 0..24 {
            self.sck_right.set_high().ok()?;
            Ets::delay_us(1);
            value = (value << 1) | u32::from(self.dout_right.is_high());
            self.sck_right.set_low().ok()?;
            Ets::delay_us(1);
        }
        self.sck_right.set_high().ok()?;
        Ets::delay_us(1);
        self.sck_right.set_low().ok()?;
        Ets::delay_us(1);

        if value & 0x0080_0000 != 0 {
            value |= 0xFF00_0000;
        }
        Some(value as i32)
    }

    /// Block until `samples` raw readings have been collected for the left HX711 or
    /// `timeout` elapses.
    pub fn read_average_blocking_left(&mut self, samples: usize, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        let mut sum: i64 = 0;
        let mut count = 0usize;
        while count < samples && Instant::now() < deadline {
            if let Some(v) = self.read_left_raw() {
                sum += i64::from(v);
                count += 1;
            } else {
                Ets::delay_us(100);
            }
        }
        if count == 0 {
            None
        } else {
            Some((sum / count as i64) as i32)
        }
    }

    /// Block until `samples` raw readings have been collected for the right HX711 or
    /// `timeout` elapses.
    pub fn read_average_blocking_right(&mut self, samples: usize, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        let mut sum: i64 = 0;
        let mut count = 0usize;
        while count < samples && Instant::now() < deadline {
            if let Some(v) = self.read_right_raw() {
                sum += i64::from(v);
                count += 1;
            } else {
                Ets::delay_us(100);
            }
        }
        if count == 0 {
            None
        } else {
            Some((sum / count as i64) as i32)
        }
    }
}
