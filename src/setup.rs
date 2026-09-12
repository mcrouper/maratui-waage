use crate::app::MaraUiApp;
use crate::button::{Button, ButtonState};
use crate::config::AppConfig;
use crate::hx711::Hx711;
#[cfg(feature = "scale-test")]
use crate::scale::RawSignalSmoother;
use crate::scale::{
    CALIBRATION_REFERENCE_G, DualScaleCalibration, ScaleCalibration, ScaleReadingFilter,
};
use crate::state::global_state::MqttOutboundMessage;
use crate::state::{AppEvent, CalibrationStep, ConnectionStatus, DeviceInfo};
use crate::telemetry::TelemetryFrame;
use mousefood::embedded_graphics::prelude::{DrawTarget, RgbColor};
use mousefood::fonts::*;
use mousefood::prelude::*;
use ratatui::Terminal;

use crate::uart_reader::UartReader;
use display_interface_spi::SPIInterface;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::{AnyIOPin, Gpio22, Gpio23, InterruptType, Output, PinDriver, Pull};
use esp_idf_svc::hal::prelude::*;
use esp_idf_svc::hal::spi::{SPI2, SpiConfig, SpiDeviceDriver, SpiDriver};
use esp_idf_svc::ipv4::{
    ClientConfiguration as IpClientConfiguration, Configuration as IpConfiguration,
    DHCPClientSettings,
};
use esp_idf_svc::mqtt::client::{
    EspMqttClient, EventPayload, MqttClientConfiguration, MqttProtocolVersion, QoS,
};
use esp_idf_svc::netif::{EspNetif, NetifConfiguration, NetifStack};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::wifi::{
    AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi, WifiDriver,
};
use ili9341::{DisplaySize240x320, Ili9341, Orientation};
use log::{info, warn};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

type DisplayResult<'a> = anyhow::Result<
    Ili9341<
        SPIInterface<SpiDeviceDriver<'a, SpiDriver<'a>>, PinDriver<'a, Gpio23, Output>>,
        PinDriver<'a, Gpio22, Output>,
    >,
>;

fn get_ili9341<'a>(
    spi_p: SPI2,
    dc: esp_idf_svc::hal::gpio::Gpio23,
    mosi: esp_idf_svc::hal::gpio::Gpio21,
    sclk: esp_idf_svc::hal::gpio::Gpio18,
    cs: Option<esp_idf_svc::hal::gpio::Gpio19>,
    rst: esp_idf_svc::hal::gpio::Gpio22,
) -> DisplayResult<'a> {
    let sdi = Option::<AnyIOPin>::None; // MISO not used for display

    let rst = PinDriver::output(rst).unwrap();
    let dc = PinDriver::output(dc).unwrap();
    let driver_config = Default::default();
    let spi_config = SpiConfig::new()
        .baudrate(Hertz(20_000_000))
        .duplex(esp_idf_svc::hal::spi::config::Duplex::Half)
        .into();
    let spi = SpiDeviceDriver::new_single(spi_p, sclk, mosi, sdi, cs, &driver_config, &spi_config)
        .unwrap();

    let di = SPIInterface::new(spi, dc);

    let mut display = Ili9341::new(
        di,
        rst,
        &mut esp_idf_svc::hal::delay::FreeRtos,
        Orientation::LandscapeFlipped,
        DisplaySize240x320,
    )
    .unwrap();

    display.clear(Rgb565::BLACK).unwrap();
    Ok(display)
}

pub fn run_app(app: impl MaraUiApp) {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    run_app_hardware(app);
}

fn run_app_hardware(mut app: impl MaraUiApp) {
    let app_config = AppConfig::from_env().expect("Invalid MARATUI_* configuration");
    app.set_mqtt_prefix(&app_config.mqtt.topic_prefix);
    let peripherals = Peripherals::take().unwrap();
    let modem = peripherals.modem;
    let spi_p = peripherals.spi2;
    let dc = peripherals.pins.gpio23;
    let mosi = peripherals.pins.gpio21;
    let sclk = peripherals.pins.gpio18;
    let cs = Some(peripherals.pins.gpio19);
    let rst = peripherals.pins.gpio22;
    let uart1 = peripherals.uart1;
    let uart_tx = peripherals.pins.gpio17;
    let uart_rx = peripherals.pins.gpio16;
    let button1_pin = peripherals.pins.gpio12;
    let hx711_dout_left = peripherals.pins.gpio25;
    let hx711_dout_right = peripherals.pins.gpio32;
    let hx711_sck_left = peripherals.pins.gpio26;
    let hx711_sck_right = peripherals.pins.gpio27;

    let mut display =
        get_ili9341(spi_p, dc, mosi, sclk, cs, rst).expect("Failed to initialize display");

    let mut backlight = PinDriver::output(peripherals.pins.gpio14).unwrap();
    backlight.set_high().unwrap();

    let mut button1 = PinDriver::input(button1_pin).unwrap();
    button1.set_interrupt_type(InterruptType::NegEdge).unwrap();
    let mut button1_state = ButtonState::default();

    button1.set_pull(Pull::Up).unwrap();

    let config = EmbeddedBackendConfig {
        font_regular: MONO_7X14,
        font_bold: Some(MONO_7X14_BOLD),
        font_italic: Some(MONO_7X14),
        ..Default::default()
    };

    // Create the Ratatui terminal first — loading stages render through it
    let backend = EmbeddedBackend::new(&mut display, config);
    let mut terminal = Terminal::new(backend).unwrap();

    // ── Scale (HX711) ──────────────────────────────────────────────────────
    // NVS is also used by Wi-Fi below; clone the partition so both can hold a handle.
    let nvs_partition = EspDefaultNvsPartition::take().ok();
    let mut scale_nvs: Option<EspNvs<NvsDefault>> = nvs_partition
        .clone()
        .and_then(|p| EspNvs::new(p, "scale", true).ok());
    let mut calibration = load_calibration(scale_nvs.as_ref());
    let mut hx711 = Hx711::new(
        hx711_dout_left,
        hx711_dout_right,
        hx711_sck_left,
        hx711_sck_right,
    )
    .expect("Failed to initialize HX711");
    let mut left_filter = ScaleReadingFilter::default();
    let mut right_filter = ScaleReadingFilter::default();
    #[cfg(feature = "scale-test")]
    let (mut left_raw_smoother, mut right_raw_smoother) =
        (RawSignalSmoother::default(), RawSignalSmoother::default());
    #[cfg(feature = "scale-test")]
    const RAW_SMOOTHING_ALPHA: f32 = 0.2;

    // ── WiFi ────────────────────────────────────────────────────────────────
    app.handle_event(AppEvent::WifiStatusChanged(ConnectionStatus::Connecting));
    app.handle_event(AppEvent::MqttStatusChanged(ConnectionStatus::Connecting));
    app.handle_event(AppEvent::LoadingStage {
        message: "connecting wifi...",
        progress: 20,
    });
    app.render_image(terminal.backend_mut().display_mut());
    terminal.draw(|f| app.draw(f)).unwrap();

    let (mut wifi, _) = init_wifi(modem, &app_config, nvs_partition);
    app.handle_event(AppEvent::WifiStatusChanged(ConnectionStatus::Connected));

    // ── MQTT ────────────────────────────────────────────────────────────────
    let (mut mqtt_client, mut cup_counter_rx, mut mqtt_status_rx) = if app_config.mqtt.enabled {
        app.handle_event(AppEvent::LoadingStage {
            message: "putting on hat...",
            progress: 55,
        });
        app.render_image(terminal.backend_mut().display_mut());
        terminal.draw(|f| app.draw(f)).unwrap();

        let stage_start = Instant::now();
        let (client, counter_rx, status_rx) = init_mqtt(&app_config);
        min_stage_delay(stage_start);
        (Some(client), Some(counter_rx), Some(status_rx))
    } else {
        app.handle_event(AppEvent::MqttStatusChanged(ConnectionStatus::Disabled));
        (None, None, None)
    };

    // ── UART ────────────────────────────────────────────────────────────────
    app.handle_event(AppEvent::LoadingStage {
        message: "heating up the machine...",
        progress: 85,
    });
    app.render_image(terminal.backend_mut().display_mut());
    terminal.draw(|f| app.draw(f)).unwrap();

    let stage_start = Instant::now();
    let (tx, rx) = std::sync::mpsc::channel::<TelemetryFrame>();

    info!("Initializing UART1: TX=GPIO17, RX=GPIO16, baud=9600");
    let uart_reader = match UartReader::new(uart1, uart_tx, uart_rx) {
        Ok(reader) => {
            info!("UART1 initialized successfully");
            reader
        }
        Err(e) => {
            warn!("Failed to initialize UART1: {:?}", e);
            panic!("UART initialization failed");
        }
    };

    info!("Spawning UART task");
    match uart_reader.spawn_uart_task(tx) {
        Ok(()) => info!("UART task spawn command completed"),
        Err(e) => {
            warn!("Failed to spawn UART task: {:?}", e);
            panic!("UART task spawn failed");
        }
    }
    min_stage_delay(stage_start);

    // Freeze the bar at 100% with "waiting for machine..." until first UART frame
    app.handle_event(AppEvent::LoadingComplete);
    let standalone_dashboard_at = Instant::now() + Duration::from_secs(5);
    let mut entered_offline_mode = false;

    let boot_time = Instant::now();
    let mut last_status_at: Option<Instant> = None;
    let mut last_wifi_check_at: Option<Instant> = None;
    let mut wifi_reconnect_at: Option<Instant> = None;
    let mut wifi_was_connected = true;
    // The two HX711s run on independent, unsynchronized conversion cycles, so they're almost
    // never "ready" on the same loop iteration. Track each cell's latest known reading here so
    // the combined total always reflects both cells, rather than treating whichever cell isn't
    // ready *this exact tick* as if it were reading 0.
    let mut last_left_weight_dg: Option<i32> = None;
    let mut last_right_weight_dg: Option<i32> = None;
    // TEMPORARY: verifying the trimmed-mean scale filter over serial — remove once confirmed.
    let mut last_logged_weight_dg: Option<i32> = None;

    loop {
        app.tick();

        button1_state.update(button1.is_low(), |press_type| {
            app.handle_press(Button::Button1(press_type));
        });

        // Holding Button1 for 3s (while not already in the wizard) starts scale calibration.
        if button1_state.held_for(button1.is_low(), CALIBRATION_HOLD_DURATION) {
            app.handle_event(AppEvent::StartCalibration);
        }

        while let Ok(telemetry) = rx.try_recv() {
            app.update_telemetry(telemetry);
        }

        if !entered_offline_mode
            && app.last_uart_frame_at().is_none()
            && Instant::now() >= standalone_dashboard_at
        {
            app.handle_event(AppEvent::EnterOfflineMode);
            entered_offline_mode = true;
        }

        // Non-blocking scale sample: read both HX711s independently and sum the calibrated
        // weights so the Dashboard shows the total load on both cells. Read as often as the
        // HX711 has new data (its own conversion rate paces this, not the loop) — the rolling
        // trimmed-mean window in `ScaleReadingFilter` does the smoothing, so there's no need to
        // artificially space samples out in time.
        let left_raw = if hx711.is_left_ready() { hx711.read_left_raw() } else { None };
        let right_raw = if hx711.is_right_ready() { hx711.read_right_raw() } else { None };

        #[cfg(feature = "scale-test")]
        if left_raw.is_some() || right_raw.is_some() {
            // Damp with an EMA so the on-screen readout doesn't jump around on raw ADC noise
            // (there's no calibration yet to run the normal step-size filter against).
            let left_smoothed =
                left_raw.map(|raw| left_raw_smoother.smooth(raw, RAW_SMOOTHING_ALPHA));
            let right_smoothed =
                right_raw.map(|raw| right_raw_smoother.smooth(raw, RAW_SMOOTHING_ALPHA));
            app.handle_event(AppEvent::RawWeightUpdated {
                left: left_smoothed,
                right: right_smoothed,
            });
        }

        let left_weight = left_raw.and_then(|raw| left_filter.accept(raw, calibration.left));
        let right_weight = right_raw.and_then(|raw| right_filter.accept(raw, calibration.right));
        if let Some(w) = left_weight {
            last_left_weight_dg = Some(w);
        }
        if let Some(w) = right_weight {
            last_right_weight_dg = Some(w);
        }
        let total_weight_dg = last_left_weight_dg.unwrap_or(0) + last_right_weight_dg.unwrap_or(0);
        if left_weight.is_some() || right_weight.is_some() {
            if last_logged_weight_dg != Some(total_weight_dg) {
                info!(
                    "Scale: {:.1}g (left={:?}dg right={:?}dg)",
                    total_weight_dg as f32 / 10.0,
                    last_left_weight_dg,
                    last_right_weight_dg
                );
                last_logged_weight_dg = Some(total_weight_dg);
            }
            app.handle_event(AppEvent::WeightUpdated { weight_dg: total_weight_dg });
        }

        // The wizard's transient steps (Taring/Calibrating) are performed here with a
        // short blocking HX711 read on the relevant channel, then reported back so the FSM
        // can advance to the next channel or finish.
        match app.calibration_step() {
            Some(CalibrationStep::TaringLeft) => {
                let success = match hx711.read_average_blocking_left(10, Duration::from_secs(3)) {
                    Some(raw) => {
                        calibration.left.offset = raw;
                        save_calibration(&mut scale_nvs, &calibration);
                        info!("Left scale tared: offset={}", raw);
                        true
                    }
                    None => {
                        warn!("Left scale tare failed: no reading from HX711");
                        false
                    }
                };
                app.handle_event(AppEvent::CalibrationStepResult { success });
            }
            Some(CalibrationStep::CalibratingLeft) => {
                let success = match hx711.read_average_blocking_left(10, Duration::from_secs(3)) {
                    Some(raw) => {
                        let candidate = ScaleCalibration::calibrate(
                            calibration.left.offset,
                            raw,
                            CALIBRATION_REFERENCE_G,
                        );
                        if candidate.is_valid() {
                            calibration.left = candidate;
                            save_calibration(&mut scale_nvs, &calibration);
                            info!("Left scale calibrated: scale={}", calibration.left.scale);
                            true
                        } else {
                            warn!("Left scale calibration rejected: reference signal was invalid");
                            false
                        }
                    }
                    None => {
                        warn!("Left scale calibration failed: no reading from HX711");
                        false
                    }
                };
                app.handle_event(AppEvent::CalibrationStepResult { success });
            }
            Some(CalibrationStep::TaringRight) => {
                let success = match hx711.read_average_blocking_right(10, Duration::from_secs(3)) {
                    Some(raw) => {
                        calibration.right.offset = raw;
                        save_calibration(&mut scale_nvs, &calibration);
                        info!("Right scale tared: offset={}", raw);
                        true
                    }
                    None => {
                        warn!("Right scale tare failed: no reading from HX711");
                        false
                    }
                };
                app.handle_event(AppEvent::CalibrationStepResult { success });
            }
            Some(CalibrationStep::CalibratingRight) => {
                let success = match hx711.read_average_blocking_right(10, Duration::from_secs(3)) {
                    Some(raw) => {
                        let candidate = ScaleCalibration::calibrate(
                            calibration.right.offset,
                            raw,
                            CALIBRATION_REFERENCE_G,
                        );
                        if candidate.is_valid() {
                            calibration.right = candidate;
                            save_calibration(&mut scale_nvs, &calibration);
                            info!("Right scale calibrated: scale={}", calibration.right.scale);
                            true
                        } else {
                            warn!("Right scale calibration rejected: reference signal was invalid");
                            false
                        }
                    }
                    None => {
                        warn!("Right scale calibration failed: no reading from HX711");
                        false
                    }
                };
                app.handle_event(AppEvent::CalibrationStepResult { success });
            }
            _ => {}
        }

        if let Some(cup_counter_rx) = cup_counter_rx.as_mut() {
            while let Ok(cups) = cup_counter_rx.try_recv() {
                app.handle_event(AppEvent::CupCounterUpdated { cups });
            }
        }

        if let Some(status_rx) = mqtt_status_rx.as_mut() {
            while let Ok(status) = status_rx.try_recv() {
                if status == ConnectionStatus::Connected {
                    let topic = format!("{}/cup_counter", app_config.mqtt.topic_prefix);
                    if let Some(client) = mqtt_client.as_mut() {
                        if let Err(e) = client.subscribe(&topic, QoS::AtMostOnce) {
                            warn!("Failed to re-subscribe to {}: {:?}", topic, e);
                        }
                    }
                    #[cfg(feature = "home-assistant")]
                    app.enqueue_home_assistant(&app_config.mqtt.topic_prefix);
                }
                app.handle_event(AppEvent::MqttStatusChanged(status));
            }
        }

        let now = Instant::now();
        let should_publish_status = last_status_at
            .map(|t| now.saturating_duration_since(t) >= STATUS_INTERVAL)
            .unwrap_or(true);
        if should_publish_status {
            last_status_at = Some(now);
            let ssid = app_config
                .wifi
                .as_ref()
                .map(|w| w.ssid.clone())
                .unwrap_or_default();
            app.handle_event(AppEvent::DeviceInfoUpdated(DeviceInfo {
                wifi_ssid: ssid,
                wifi_rssi: query_wifi_rssi(),
                ip: wifi
                    .sta_netif()
                    .get_ip_info()
                    .ok()
                    .map(|i| i.ip.to_string()),
                uptime_s: now.saturating_duration_since(boot_time).as_secs(),
                free_heap_b: Some(query_free_heap()),
                last_telemetry_age_s: app
                    .last_uart_frame_at()
                    .map(|t| now.saturating_duration_since(t).as_secs()),
            }));
        }

        let should_check_wifi = last_wifi_check_at
            .map(|t| now.saturating_duration_since(t) >= WIFI_CHECK_INTERVAL)
            .unwrap_or(true);
        if should_check_wifi {
            last_wifi_check_at = Some(now);
            let is_connected = query_wifi_rssi().is_some();
            if is_connected != wifi_was_connected {
                wifi_was_connected = is_connected;
                if is_connected {
                    info!("Wi-Fi reconnected");
                    app.handle_event(AppEvent::WifiStatusChanged(ConnectionStatus::Connected));
                } else {
                    info!("Wi-Fi disconnected");
                    app.handle_event(AppEvent::WifiStatusChanged(ConnectionStatus::Disconnected));
                }
            }
            if !is_connected {
                let should_reconnect = wifi_reconnect_at
                    .map(|t| now.saturating_duration_since(t) >= WIFI_RECONNECT_INTERVAL)
                    .unwrap_or(true);
                if should_reconnect {
                    wifi_reconnect_at = Some(now);
                    info!("Wi-Fi disconnected, attempting reconnect");
                    if let Err(e) = wifi.connect() {
                        warn!("Wi-Fi reconnect failed: {:?}", e);
                    }
                }
            } else {
                wifi_reconnect_at = None;
            }
        }

        for msg in app.take_outbound_mqtt_messages() {
            publish_mqtt_message(&mut mqtt_client, &app_config, &msg);
        }

        if app.backlight_active() {
            backlight.set_high().unwrap();
        } else {
            backlight.set_low().unwrap();
        }

        // Apply any pending full clear requested by the state machine (new session, Debug
        // toggle) so accumulated display artifacts are wiped before the next frame.
        if app.take_redraw_request() {
            terminal.clear().unwrap();
        }

        app.render_image(terminal.backend_mut().display_mut());

        terminal
            .draw(|f| {
                app.draw(f);
            })
            .unwrap();

        // Yield to the FreeRTOS scheduler so the UART task and Wi-Fi/MQTT stack
        // get CPU time between render frames.
        esp_idf_svc::hal::delay::FreeRtos::delay_ms(0);
    }
}

const STATUS_INTERVAL: Duration = Duration::from_secs(30);
const WIFI_CHECK_INTERVAL: Duration = Duration::from_secs(10);
const WIFI_RECONNECT_INTERVAL: Duration = Duration::from_secs(15);
const MIN_STAGE_MS: u64 = 800;
/// How long Button1 must be held to start the scale calibration wizard.
const CALIBRATION_HOLD_DURATION: Duration = Duration::from_secs(3);

fn min_stage_delay(started_at: Instant) {
    let elapsed = started_at.elapsed();
    let min = Duration::from_millis(MIN_STAGE_MS);
    if elapsed < min {
        esp_idf_svc::hal::delay::FreeRtos::delay_ms((min - elapsed).as_millis() as u32);
    }
}

fn query_wifi_rssi() -> Option<i32> {
    let mut ap_info: esp_idf_svc::sys::wifi_ap_record_t = unsafe { core::mem::zeroed() };
    if unsafe { esp_idf_svc::sys::esp_wifi_sta_get_ap_info(&mut ap_info) } == 0 {
        Some(i32::from(ap_info.rssi))
    } else {
        None
    }
}

fn query_free_heap() -> u32 {
    unsafe { esp_idf_svc::sys::esp_get_free_heap_size() }
}

// TEMPORARY wiring sanity-check values — NOT a real calibration. `offset` is each cell's
// actual empty-scale raw reading, measured live over serial (left ≈ -206,400, right ≈
// -431,000 — the two cells are nowhere near each other, so they need separate offsets).
// `scale` is still a rough ballpark (counts/gram) for a small HX711 module at gain 128, so
// displayed grams are still wrong — but readings now correctly return to ~0g when the scale
// is empty instead of getting stuck (an all-zero offset put the "empty" reading outside the
// plausible-weight sanity range, so it was silently rejected and the last loaded reading
// never got overwritten). Run the real calibration wizard (hold Button1 3s) and remove this
// fallback once the cells are confirmed working.
const ASSUMED_CALIBRATION_LEFT: ScaleCalibration = ScaleCalibration {
    offset: -206_400,
    scale: 400.0,
};
const ASSUMED_CALIBRATION_RIGHT: ScaleCalibration = ScaleCalibration {
    offset: -431_000,
    scale: 400.0,
};

/// Load a persisted dual-scale calibration from NVS, falling back to `ASSUMED_CALIBRATION_*`
/// (see above) if none was ever saved or NVS is unavailable.
fn load_calibration(nvs: Option<&EspNvs<NvsDefault>>) -> DualScaleCalibration {
    let assumed = DualScaleCalibration {
        left: ASSUMED_CALIBRATION_LEFT,
        right: ASSUMED_CALIBRATION_RIGHT,
    };

    let Some(nvs) = nvs else {
        warn!("NVS unavailable — dual scale will use ASSUMED_CALIBRATION (wiring check only)");
        return assumed;
    };

    let mut buf = [0u8; 16];
    match nvs.get_raw("scale", &mut buf) {
        Ok(Some(_)) => DualScaleCalibration::from_bytes(buf),
        Ok(None) => {
            info!("No stored dual-scale calibration found, using ASSUMED_CALIBRATION (wiring check only)");
            assumed
        }
        Err(e) => {
            warn!("Failed to read stored dual-scale calibration: {:?}", e);
            assumed
        }
    }
}

fn save_calibration(nvs: &mut Option<EspNvs<NvsDefault>>, calibration: &DualScaleCalibration) {
    let Some(nvs) = nvs.as_mut() else {
        warn!("NVS unavailable — dual scale calibration will not persist across reboot");
        return;
    };
    if let Err(e) = nvs.set_raw("scale", &calibration.to_bytes()) {
        warn!("Failed to persist dual scale calibration: {:?}", e);
    }
}

fn init_wifi(
    modem: esp_idf_svc::hal::modem::Modem,
    app_config: &AppConfig,
    nvs: Option<EspDefaultNvsPartition>,
) -> (EspWifi<'static>, Option<String>) {
    let sys_loop = EspSystemEventLoop::take().expect("Failed to take system event loop");

    let wifi_cfg = app_config
        .wifi
        .as_ref()
        .expect("Wi-Fi config is required on device");

    let wifi_driver =
        WifiDriver::new(modem, sys_loop.clone(), nvs).expect("Failed to create Wi-Fi driver");
    let mut esp_wifi = EspWifi::wrap_all(
        wifi_driver,
        EspNetif::new_with_conf(&NetifConfiguration {
            ip_configuration: Some(IpConfiguration::Client(IpClientConfiguration::DHCP(
                DHCPClientSettings {
                    hostname: Some("maratui".try_into().expect("hostname too long")),
                },
            ))),
            ..NetifConfiguration::wifi_default_client()
        })
        .expect("Failed to create STA netif"),
        EspNetif::new(NetifStack::Ap).expect("Failed to create AP netif"),
    )
    .expect("Failed to create Wi-Fi");

    {
        let mut wifi =
            BlockingWifi::wrap(&mut esp_wifi, sys_loop.clone()).expect("Failed to wrap Wi-Fi");

        let auth_method = if wifi_cfg.password.is_empty() {
            warn!(
                "MARATUI_WIFI_PASSWORD is empty — connecting to '{}' as an open network (no encryption).",
                wifi_cfg.ssid
            );
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        };

        wifi.set_configuration(&Configuration::Client(ClientConfiguration {
            ssid: wifi_cfg.ssid.as_str().try_into().expect("SSID too long"),
            password: wifi_cfg
                .password
                .as_str()
                .try_into()
                .expect("password too long"),
            auth_method,
            ..Default::default()
        }))
        .expect("Failed to set Wi-Fi config");

        wifi.start().expect("Failed to start Wi-Fi");
        wifi.connect().expect("Failed to connect Wi-Fi");
        wifi.wait_netif_up().expect("Failed to obtain IP");
    }

    let initial_ip = esp_wifi
        .sta_netif()
        .get_ip_info()
        .ok()
        .map(|info| info.ip.to_string());
    info!("Wi-Fi connected, IP: {:?}", initial_ip);

    (esp_wifi, initial_ip)
}

fn init_mqtt(
    app_config: &AppConfig,
) -> (
    EspMqttClient<'static>,
    Receiver<u64>,
    Receiver<ConnectionStatus>,
) {
    let cup_counter_topic = format!("{}/cup_counter", app_config.mqtt.topic_prefix);
    let callback_topic = cup_counter_topic.clone();
    let (cup_counter_tx, cup_counter_rx) = mpsc::channel::<u64>();
    let (connected_tx, connected_rx) = mpsc::sync_channel::<()>(1);
    let (mqtt_status_tx, mqtt_status_rx) = mpsc::channel::<ConnectionStatus>();

    info!("Starting MQTT client: {}", app_config.mqtt.url);
    let mut mqtt_client = EspMqttClient::new_cb(
        &app_config.mqtt.url,
        &MqttClientConfiguration {
            protocol_version: Some(MqttProtocolVersion::V3_1_1),
            client_id: Some(app_config.mqtt.client_id.as_str()),
            username: app_config.mqtt.username.as_deref(),
            password: app_config.mqtt.password.as_deref(),
            reconnect_timeout: Some(Duration::from_secs(2)),
            network_timeout: Duration::from_secs(5),
            keep_alive_interval: Some(Duration::from_secs(30)),
            ..Default::default()
        },
        move |event| match event.payload() {
            EventPayload::Connected(_) => {
                info!("MQTT connected");
                let _ = connected_tx.try_send(());
                let _ = mqtt_status_tx.send(ConnectionStatus::Connected);
            }
            EventPayload::Disconnected => {
                let _ = mqtt_status_tx.send(ConnectionStatus::Connecting);
            }
            EventPayload::Received {
                topic: Some(topic),
                data,
                ..
            } if topic == callback_topic => {
                match core::str::from_utf8(data)
                    .ok()
                    .map(str::trim)
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    Some(cups) => {
                        let _ = cup_counter_tx.send(cups);
                    }
                    None => {
                        warn!("Failed to parse cup counter payload: {:?}", data);
                    }
                }
            }
            payload => {
                info!("MQTT event: {}", payload);
            }
        },
    )
    .expect("Failed to create MQTT client");

    match connected_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(_) => {
            if let Err(e) = mqtt_client.subscribe(&cup_counter_topic, QoS::AtMostOnce) {
                warn!("Failed to subscribe to {}: {:?}", cup_counter_topic, e);
            }
        }
        Err(_) => {
            warn!("MQTT connection timeout after 10s, skipping subscribe");
        }
    }

    info!("MQTT started: {}", app_config.mqtt.url);
    (mqtt_client, cup_counter_rx, mqtt_status_rx)
}

fn publish_mqtt_message(
    client: &mut Option<EspMqttClient<'_>>,
    cfg: &AppConfig,
    msg: &MqttOutboundMessage,
) {
    let Some(client) = client.as_mut() else {
        return;
    };

    let topic = if msg.absolute {
        msg.topic_suffix.clone()
    } else {
        format!("{}/{}", cfg.mqtt.topic_prefix, msg.topic_suffix)
    };
    let qos = if msg.retain {
        QoS::AtLeastOnce
    } else {
        QoS::AtMostOnce
    };
    if let Err(e) = client.publish(&topic, qos, msg.retain, msg.payload.as_bytes()) {
        warn!("Failed to publish MQTT message: {:?}", e);
    }
}
