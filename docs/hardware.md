# Hardware Wiring

Target board: **ESP32 Type-C** (generic) with external **ILI9341** 240x320 TFT display.

| ESP32 Type-C | ILI9341 Display |
|:---:|:---:|
| ![ESP32](assets/esp32.jpg) | ![ILI9341](assets/ili9341.jpg) |

## Pinout

### SPI Display (ILI9341)

| Display Pin | ESP32 GPIO | Notes |
|-------------|------------|-------|
| SCK / CLK   | GPIO18     | VSPI CLK |
| SDA / MOSI  | GPIO21     | VSPI MOSI |
| DC / RS     | GPIO23     | Data/Command select |
| CS          | GPIO19     | Chip Select |
| RST / RESET | GPIO22     | Hardware reset |
| LED / BLK   | GPIO14     | Backlight (driven HIGH = on) |
| VCC         | 3.3V       | |
| GND         | GND        | |

### UART (Lelit Mara X telemetry)

| Function | ESP32 GPIO | Notes |
|----------|------------|-------|
| TX       | GPIO17     | UART1 TX |
| RX       | GPIO16     | UART1 RX |

### Button

| Button   | ESP32 GPIO | Notes |
|----------|------------|-------|
| Button 1 | GPIO12     | Internal pull-up enabled |

Button connects GPIO to **GND** through a tactile switch (active LOW, NegEdge detection).
Short press (< 500 ms): toggle Dashboard ↔ Graphs. Long press (≥ 500 ms, < 3 s): toggle
Debug screen. Hold ≥ 3 s: start the scale calibration wizard (see below).

### Scale (HX711 load-cell amplifier)

| HX711 Pin | ESP32 GPIO | Notes |
|-----------|------------|-------|
| DOUT      | GPIO25     | Data (input on ESP32) |
| SCK       | GPIO26     | Clock (output from ESP32) |
| VCC       | 3.3V / 5V  | Per HX711 board spec |
| GND       | GND        | |

The load cell itself connects to the HX711 board's E+/E-/A+/A- terminals per the load
cell's datasheet. The driver reads channel A at gain 128 (the HX711 default).

#### Calibrating the scale

Calibration data (zero-point offset + counts-per-gram factor) is stored in NVS flash and
survives reboots. To (re-)calibrate:

1. Hold **Button 1** for 3 seconds to enter the calibration wizard.
2. **Step 1/2 — Zero point:** remove everything from the scale, then short-press Button 1
   to confirm. The device tares (averages ~10 raw samples as the zero-point).
3. **Step 2/2 — Reference weight:** place a **500 g** reference weight on the scale, then
   short-press Button 1 to confirm. The device computes the counts-per-gram scale factor
   from this reading and saves both values to NVS.
4. A result screen confirms success (or reports a read failure — check wiring and retry).
   Press either button to dismiss and return to the previous screen.

A long press at any step during the wizard cancels without changing the stored
calibration. The Dashboard shows live weight once calibrated; it auto-tares (zeroes) the
cup weight the moment a shot starts, so it displays the net extracted weight.
