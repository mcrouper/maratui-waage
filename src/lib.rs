/// Application setup.
pub mod app;
pub mod config;
#[cfg(feature = "home-assistant")]
pub mod home_assistant;
#[cfg(not(feature = "simulator"))]
mod setup;
#[cfg(not(feature = "simulator"))]
pub use setup::run_app;
#[cfg(feature = "simulator")]
mod setup_simulator;
#[cfg(feature = "simulator")]
pub use setup_simulator::run_app;

/// Button handling.
pub mod button;

/// HX711 load-cell amplifier driver (device only — bit-bangs a shared SCK with two DOUT pins).
#[cfg(not(feature = "simulator"))]
pub mod hx711;

/// Scale calibration math (offset/scale conversion, NVS byte encoding). Hardware-agnostic.
pub mod scale;

pub mod telemetry;

pub mod screens;

/// State management (FSM).
pub mod state;

#[cfg(not(feature = "simulator"))]
pub mod uart_reader;
