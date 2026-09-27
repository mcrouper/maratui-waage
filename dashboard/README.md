# Web dashboard

A standalone live dashboard page for MaraTUI, served by Node-RED at `http://<node-red-host>:1880/mara`:
boiler / HX temperatures with history, heating / pump / water / boost state, live scale weight,
shot timer, shot weight curve, recent shots (duration, yield, flow), event log and device status.
No external dependencies (no CDN, no Dashboard nodes) — it works on an offline LAN.

## How it works

```
mqtt in (mara/#) → function "Zustand + Broadcast" → websocket out /ws/mara  (broadcast to all pages)
websocket in /ws/mara → function "Snapshot an neuen Client" → websocket out  (state for a new page)
http in GET /mara → template (index.html) → http response
http in GET /mara/shot/:ts → function → http response            (the shot's JPEG, linked in the table)
```

Node-RED keeps the state (last hour of temperatures, last 10 shots, last 30 events) in flow
context, so a freshly opened page is populated immediately; the browser never talks to the MQTT
broker itself, so no broker credentials end up in the page.

Shots survive Node-RED restarts and power cuts: the function node writes them to `$HOME/maratui/`
of the Node-RED user (`--data-dir` to change) and reloads them on start:

- `shots.json` — the last 10 shots incl. weight and HX curves (written atomically with fsync)
- `shotdoku.txt` — plain-text archive; every shot pushed out of the last 10 is appended, e.g.
  `2026-09-27 14:39:28  Dauer 11 s  Gewicht 41,9 g  Fluss 3,8 g/s  HX 95-104 °C  (kurz)` plus a
  weight-per-second line

- `shots/JJJJ-MM-TT_HH-MM-SS.jpg` — one chart per shot (weight curve, HX temperature behind it,
  duration / weight / flow in the title), rendered on the Node-RED host by `render_shot.py`
  (needs `python3` with Pillow and the DejaVu fonts, both on Raspberry Pi OS by default). The
  flow installs the script into the data dir on start and renders missing images for saved shots.
- `cups.json` — total cup count, +1 for every shot that enters the shot list (normal shots and
  short pours with >= 5 g, not rinses). Published retained to `mara/cup_counter`, so the ESP32
  display shows it too — this replaces the Home Assistant automation for setups without HA. To set
  a start value: `mosquitto_pub -r -t mara/cup_counter -m 1234` (plus `-h/-u/-P`); the flow adopts it.

Temperature history and events stay in memory only.

`reducer.js` is the single implementation of how MQTT messages update that state. `deploy.py`
injects it both into the Node-RED function node and into the page, so server and browser can't
drift apart.

## Deploy / update

```bash
python3 dashboard/deploy.py                      # default: http://192.168.178.162:1880
python3 dashboard/deploy.py --url http://host:1880 --broker <mqtt-broker config node id>
python3 dashboard/deploy.py --dry-run            # print the generated flow JSON
```

Creates the Node-RED tab **MaraTUI Web** on first run and updates it in place afterwards (stable
node ids); other flows are left untouched. It subscribes through the existing mqtt-broker config
node named `MaraTUI Broker` unless `--broker` is given. Edit the files here and redeploy rather than
editing the generated nodes in the Node-RED editor.

Requires the Node-RED admin API to be reachable without auth (or add auth handling to `deploy.py`).

## Topics used

`mara/telemetry`, `mara/status`, `mara/events` (incl. `weight_g` on `shot_ended`), `mara/scale`,
`mara/cup_counter` — see [docs/home-assistant.md](../docs/home-assistant.md#mqtt-topics-published-by-maratui).
The `mara` prefix is hardcoded in `deploy.py` (`TOPIC`); change it there if
`MARATUI_MQTT_TOPIC_PREFIX` differs.
