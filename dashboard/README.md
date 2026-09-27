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
```

Node-RED keeps the state (last hour of temperatures, last 20 shots, last 30 events) in flow
context, so a freshly opened page is populated immediately; the browser never talks to the MQTT
broker itself, so no broker credentials end up in the page. The state lives in memory and is lost
when Node-RED restarts.

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
