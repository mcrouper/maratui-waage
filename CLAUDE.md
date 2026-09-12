# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Contributing Rules

- **All commit messages and pull request titles/descriptions must be written in English.**

## What This Is

MaraTUI is an embedded Rust TUI application for the **Lelit Mara X** espresso machine. It runs on an ESP32 microcontroller driving an ILI9341 320×240 display via SPI, reads telemetry from the machine over UART, and publishes events to MQTT. A host simulator mode exists for UI development without hardware.

## Build Targets and Features

The crate has two mutually exclusive base features:
- `device` (default) — targets `xtensa-esp32-espidf`, requires the `esp` toolchain channel
- `simulator` — targets the host OS, uses SDL2 for display emulation and `rumqttc` for MQTT

Two more features layer on top of either base feature:
- `home-assistant` — publishes MQTT Discovery configs directly from the firmware
- `scale-test` — bring-up build variant for the scale only: no Lelit Mara UART connection is
  expected, and the Dashboard's main content area is replaced by a raw (uncalibrated, untared)
  left/right/sum ADC-counts readout instead of the normal machine telemetry. See
  `docs/hardware.md`.

The toolchain is pinned in `rust-toolchain.toml` to `channel = "esp"` (Espressif's fork).

## Commands

**Run the simulator (recommended for UI work):**
```bash
make sim          # auto-detects OS
cargo sim         # Linux x86_64
cargo simmac      # macOS aarch64
cargo simwin      # Windows x86_64
```

**Flash to ESP32:**
```bash
cargo run --release
```
Uses `espflash flash --monitor` as the runner (configured in `.cargo/config.toml`).

**Run tests (host, no hardware needed):**
```bash
cargo test --no-default-features --features simulator --target x86_64-unknown-linux-gnu
```
Tests live inline in `src/state/fsm.rs`, `src/state/global_state.rs`, `src/state/app_events.rs`, and `src/telemetry/telemetry.rs`.

## Configuration

Create a `.env` file in the project root (git-ignored). `build.rs` reads it and embeds `MARATUI_*` values as compile-time env vars for the device build. The simulator reads them at runtime.

```
MARATUI_WIFI_SSID=...
MARATUI_WIFI_PASSWORD=...
MARATUI_MQTT_ENABLED=true
MARATUI_MQTT_URL=mqtt://broker.emqx.io:1883
MARATUI_MQTT_CLIENT_ID=maratui-dev
MARATUI_MQTT_USERNAME=
MARATUI_MQTT_PASSWORD=
MARATUI_MQTT_TOPIC_PREFIX=mara
```

## Architecture

### Feature-gated runtime (`src/lib.rs`)
`run_app` is re-exported from either `setup.rs` (device) or `setup_simulator.rs` (simulator). Both implement the same polling loop: read UART/keyboard input → update telemetry → handle button presses → drain outbound MQTT queue → render.

### `MaraUiApp` trait (`src/app.rs`)
The public interface all platform setups call. `MaraUi` is the concrete implementation. `draw()` dispatches to the active screen; `render_image()` blits a raw RGB565 asset directly to the embedded display (bypassing Ratatui), used for the rat barista sprite on the Main screen.

### State management (`src/state/`)
- `GlobalAppState` — single source of truth, passed by reference everywhere. Holds current screen, extraction state, machine state, connection statuses, MQTT outbound queue, and error.
- `AppStateMachine` — stateless struct; all methods take `&mut GlobalAppState`. Handles `AppEvent` variants and button presses.
- `ExtractionState` — `Idle { last_extraction_duration }` / `Extracting { started_at }`.

### Telemetry (`src/telemetry/`)
- `parse_uart_line()` parses the machine's UART format: `<ModeChar><Version>,<boiler_now>,<boiler_target_or_Lxx>,<hx_now>,<boost>,<heating>,<pump>`
- `update_state_with_events()` computes derived events (shot start/end, water refill, mode change) from frame-to-frame transitions.
- `MachineState` holds rolling `VecDeque<f64>` buffers (capped at 300 points each, pushed in triples) for the three temperature series shown in the Graphs screen.

### Screens (`src/screens/`)
Each screen is a zero-size struct implementing the `Board` trait (`fn render(state, area, frame)`). There is a **single physical button** (Button1). `Screen` (`src/screens/screen.rs`) is `{ Dashboard (default), Graphs, Debug }`; a short press toggles `Dashboard ↔ Graphs` (`Screen::next`/`previous` are the same swap — one button, no real "previous"), a long press enters/exits `Debug` (`screen_before_debug` restores whichever of the two it interrupted). The boot/loading screen (`Connecting`) is not a `Screen` variant — `MaraUi::draw()` shows it whenever `GlobalAppState::machine_state.last_frame` is still `None`, before any telemetry (or, on the simulator/`EnterOfflineMode`, a synthetic offline frame) has arrived. `CalibrationWizard` is not part of the rotation either — it overlays whatever screen is active whenever `GlobalAppState::calibration_step` is `Some`.

### Scale (`src/hx711.rs`, `src/scale.rs`)
- `hx711.rs` (device only) bit-bangs the HX711 protocol for two independent load cells, each with its own DOUT/SCK pair (left: GPIO25/GPIO26, right: GPIO32/GPIO27), gain 128 / channel A. See `docs/hardware.md` for the full pin mapping.
- `scale.rs` is hardware-agnostic (no GPIO/NVS access) so it's reused identically by the device driver, the simulator, and unit tests:
  - `ScaleCalibration { offset, scale }` converts raw ADC counts to decigrams; `DualScaleCalibration` holds one per cell plus NVS byte (de)serialization.
  - `ScaleReadingFilter` smooths each cell's raw samples with a **rolling trimmed-mean window** (`WINDOW_SAMPLES = 32`, drop the single highest and lowest sample) before calibrating them — the same approach the widely-used `HX711_ADC` Arduino library uses (e.g. in the CleverCoffee espresso-PID firmware). It deliberately does *not* reject a reading for being too different from the previous one: an early step-limiting design could get permanently stuck, since a real, fast weight change gets rejected as implausible and every later sample is still compared against the same stale reference.
- Calibration is triggered by holding Button1 for 3s, which starts the `CalibrationWizard` (see `GlobalAppState::calibration_step` / `CalibrationStep`). The device loop (`setup.rs`) performs the actual blocking HX711 reads for the `Taring`/`Calibrating` steps and reports back via `AppEvent::CalibrationStepResult`; calibration persists to NVS under the `"scale"` namespace.
- The Dashboard shows live weight (`GlobalAppState::weight_dg`, in decigrams) and auto-tares the moment a shot starts (`AppEvent::ShotStarted`) so it displays net extracted weight. The shot gauge ("%" column) tracks extracted weight, not elapsed time: `WEIGHT_GAUGE_MAX_DG` (40g) is 100%.
- Because the two HX711s free-run on independent, unsynchronized conversion cycles, they're essentially never "ready" on the same loop tick — `setup.rs` tracks each cell's *latest known* reading and sums those, rather than summing `left.unwrap_or(0) + right.unwrap_or(0)` (which would treat whichever cell isn't ready *this exact tick* as reading zero).

### Assets (`assets/`)
Raw RGB565 image files are `include_bytes!`-embedded at compile time. To regenerate from PNG:
```bash
ffmpeg -f lavfi -i color=black:s=180x240 -i rat_barista.png \
  -filter_complex "[1:v]scale=180:240[scaled];[0:v][scaled]overlay" \
  -f rawvideo -pix_fmt rgb565be -frames:v 1 rat_barista.raw
```

### Simulator keyboard controls
| Key | Action |
|-----|--------|
| Right / Left arrow | Button1 short (toggle Dashboard ↔ Graphs) |
| D | Button1 long (toggle Debug screen) |
| Up | Inject pump-on debug frame (also starts simulated scale weight rising) |
| Down | Inject normal debug frame (stops simulated scale weight) |
| Space | Inject no-water debug frame (stops simulated scale weight) |
| M | Publish manual MQTT event |
| C | Start the scale calibration wizard (real hardware needs a 3s Button1 hold instead) |

---

## Home Assistant Integration (`src/home_assistant.rs`)

Build with `--features home-assistant` to enable direct HA MQTT Discovery publishing from the firmware — no Node-RED needed. On every MQTT connect, the firmware publishes retained discovery configs to `homeassistant/{sensor,binary_sensor}/maratui_*/config` and streams state updates alongside the existing `mara/*` topics.

**Keep `home_assistant.rs` in sync when changing MQTT payloads:**

| Change | What to update |
|--------|----------------|
| Add/remove a field in `mara/status` payload (`device_status_payload` in `fsm.rs`) | `enqueue_status_states` in `home_assistant.rs` + sensor list in `enqueue_discovery_configs` |
| Add/remove a field in `mara/telemetry` payload (`telemetry_payload` in `fsm.rs`) | `enqueue_telemetry_states` in `home_assistant.rs` + sensor list in `enqueue_discovery_configs` |
| Add/remove an event type in `mara/events` | `enqueue_event_states` in `home_assistant.rs` |
| Rename MQTT topic prefix | Update `docs/ha-automation.yaml` topic strings |

The cup counter requires a one-time HA automation setup — see `docs/home-assistant.md` and `docs/ha-automation.yaml`.

The simulator connects with client ID `<MARATUI_MQTT_CLIENT_ID>-sim` (e.g. `maratui-dev-sim`) to avoid session conflicts when device and simulator run simultaneously.

---

## Security Considerations

### Credentials baked into the binary at compile time
Wi-Fi SSID/password and MQTT credentials are embedded via `option_env!()` in `build.rs` and stored as plaintext `&'static str` in the flash image. Anyone with physical access and an SPI flash reader can extract them with standard binary analysis tools (`strings`, `binwalk`). **Rotate credentials if the device is shared or its firmware is distributed.**

### Default MQTT broker is public and unauthenticated
The default `MARATUI_MQTT_URL=mqtt://broker.emqx.io:1883` publishes machine telemetry to a free public broker with no access control. Any client that knows or guesses the topic prefix (`mara/telemetry`) can read all extraction data. Always set a private broker with authentication before deploying. A `warn!` fires on every startup when the default broker is detected.

### No TLS for MQTT
The `mqtt://` scheme is plaintext TCP. The `esp-idf-svc` MQTT client supports TLS via `mqtts://` — switch to it and configure a CA certificate for the broker. Until then, credentials and telemetry travel unencrypted on the local network. A `warn!` fires on startup when credentials are configured over a plaintext connection.

### Open Wi-Fi fallback
If `MARATUI_WIFI_PASSWORD` is empty, `AuthMethod::None` is used (open network — no encryption). This is intentional for open-network environments. A `warn!` fires on startup naming the SSID so the choice is explicit in the logs.

### UART input is not sanitised beyond ASCII gating
`uart_reader.rs` only rejects non-ASCII bytes before appending to the line buffer. All ASCII (including control characters like `\t` or DEL) passes through to `parse_uart_line`. The parser is robust (returns `Err` on bad field counts or non-numeric values), but malformed input from the machine side can spam `warn!` log entries at up to one per byte if the machine sends garbage continuously.
