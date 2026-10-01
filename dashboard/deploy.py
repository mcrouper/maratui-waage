#!/usr/bin/env python3
"""Deploy the MaraTUI web dashboard to Node-RED as its own flow tab.

Builds a "MaraTUI Web" tab from index.html + reducer.js and creates or updates it through the
Node-RED admin API, leaving every other flow untouched:

    mqtt in (mara/#) -> function (reduce + keep state + persist shots) -> websocket out /ws/mara
    websocket in /ws/mara -> function (snapshot for the new client) -> websocket out
    http in GET /mara -> template (the page) -> http response
    http in GET /mara/shot/:ts -> function (read shots/<date>_<time>.jpg) -> http response

Shot persistence (survives Node-RED restarts and Pi power-off), in $HOME/maratui/ of the user
running Node-RED (override with --data-dir):
    shots.json    the last 10 shots incl. weight/HX curves, reloaded into the page on start
    shotdoku.txt  plain-text archive; each shot pushed out of the last 10 is appended here
    shots/*.jpg   one chart per shot (weight + HX), rendered on the Pi by render_shot.py (Pillow)
    cups.json     total cup count; +1 for every shot added to the shot list, also published
                  retained to mara/cup_counter (read by the ESP32 display)

Usage: ./deploy.py [--url http://192.168.178.162:1880] [--broker <mqtt-broker config node id>]
       ./deploy.py --dry-run   # print the flow JSON instead of deploying
"""

import argparse
import hashlib
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
TAB_LABEL = "MaraTUI Web"
PAGE_PATH = "/mara"
WS_PATH = "/ws/mara"
TOPIC = "mara/#"
CUP_TOPIC = "mara/cup_counter"

# Shared by the function node's "On Start" and message code (separate scopes in Node-RED).
# `fs` comes in through the node's `libs` (needs functionExternalModules, the default).
PERSIST_HELPERS = r"""
const DATA_DIR = "__DATA_DIR__" || (env.get("HOME") + "/maratui");
const SHOTS_FILE = DATA_DIR + "/shots.json";
const ARCHIVE_FILE = DATA_DIR + "/shotdoku.txt";
const CUPS_FILE = DATA_DIR + "/cups.json";
const IMG_DIR = DATA_DIR + "/shots";
const RENDER_SCRIPT = DATA_DIR + "/render_shot.py";

function persistCups(cups) {
    try {
        fs.mkdirSync(DATA_DIR, { recursive: true });
        const tmp = CUPS_FILE + ".tmp";
        fs.writeFileSync(tmp, JSON.stringify({ cups: cups }));
        fs.renameSync(tmp, CUPS_FILE);
    } catch (e) {
        node.error("Speichern von " + CUPS_FILE + " fehlgeschlagen: " + e.message);
    }
}

// Write + fsync + rename, so a power cut leaves either the old or the new file, never half of one
function persistShots(shots) {
    try {
        fs.mkdirSync(DATA_DIR, { recursive: true });
        const tmp = SHOTS_FILE + ".tmp";
        const fd = fs.openSync(tmp, "w");
        fs.writeSync(fd, JSON.stringify(shots));
        fs.fsyncSync(fd);
        fs.closeSync(fd);
        fs.renameSync(tmp, SHOTS_FILE);
    } catch (e) {
        node.error("Speichern von " + SHOTS_FILE + " fehlgeschlagen: " + e.message);
    }
}

const pad = (n) => String(n).padStart(2, "0");
const de1 = (v) => v.toFixed(1).replace(".", ",");
function shotText(s) {
    const d = new Date(s.ts);
    const when = d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate()) + " " +
        pad(d.getHours()) + ":" + pad(d.getMinutes()) + ":" + pad(d.getSeconds());
    const w = typeof s.weight_g === "number" ? s.weight_g : null;
    let line = when + "  Dauer " + s.duration + " s  Gewicht " + (w === null ? "-" : de1(w) + " g") +
        "  Fluss " + (w !== null && s.duration ? de1(w / s.duration) + " g/s" : "-");
    const hx = (s.hx || []).map((p) => p[1]);
    if (hx.length) line += "  HX " + Math.min(...hx) + "-" + Math.max(...hx) + " °C";
    if (s.aborted) line += "  (kurz)";
    // Weight once per second: the last sample at or before each whole second
    const curve = s.curve || [];
    const pts = [];
    for (let t = 0, i = 0, g = 0; t <= Math.ceil(s.duration || 0) && curve.length; t++) {
        while (i < curve.length && curve[i][0] <= t) g = curve[i++][1];
        pts.push(t + ":" + de1(g));
    }
    return line + "\n" + (pts.length ? "  Gewicht je Sekunde: " + pts.join(" ") + "\n" : "");
}

function shotImagePath(s) {
    const d = new Date(s.ts);
    return IMG_DIR + "/" + d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate()) + "_" +
        pad(d.getHours()) + "-" + pad(d.getMinutes()) + "-" + pad(d.getSeconds()) + ".jpg";
}

// Render the shot's chart to JPEG with render_shot.py (async; failures only logged)
function renderShot(s, onlyIfMissing) {
    try {
        fs.mkdirSync(IMG_DIR, { recursive: true });
        const out = shotImagePath(s);
        if (onlyIfMissing && fs.existsSync(out)) return;
        const proc = cp.spawn("python3", [RENDER_SCRIPT, out], { stdio: ["pipe", "ignore", "pipe"] });
        let err = "";
        proc.stderr.on("data", (c) => { err += c; });
        proc.on("error", (e) => node.error("JPG " + out + ": " + e.message));
        proc.on("close", (code) => { if (code) node.error("JPG " + out + " fehlgeschlagen: " + err.slice(-400)); });
        proc.stdin.end(JSON.stringify(s));
    } catch (e) {
        node.error("JPG-Erzeugung fehlgeschlagen: " + e.message);
    }
}

// Oldest first, so the archive reads chronologically
function archiveShots(shots) {
    if (!shots.length) return;
    try {
        fs.mkdirSync(DATA_DIR, { recursive: true });
        fs.appendFileSync(ARCHIVE_FILE, shots.slice().reverse().map(shotText).join(""));
    } catch (e) {
        node.error("Archivieren in " + ARCHIVE_FILE + " fehlgeschlagen: " + e.message);
    }
}
"""

# Runs on every Node-RED start / redeploy: merge the saved shots into the (possibly surviving)
# in-memory state, archive whatever no longer fits into the last 10, and save the result.
STATE_INIT = """
const S = flow.get("mara") || maraInitialState();
let saved = [];
try {
    if (fs.existsSync(SHOTS_FILE)) saved = JSON.parse(fs.readFileSync(SHOTS_FILE, "utf8"));
} catch (e) {
    node.error("Lesen von " + SHOTS_FILE + " fehlgeschlagen: " + e.message);
}
// Install the current JPEG renderer next to the data files
try {
    fs.mkdirSync(DATA_DIR, { recursive: true });
    fs.writeFileSync(RENDER_SCRIPT, __RENDER_PY__);
} catch (e) {
    node.error("Schreiben von " + RENDER_SCRIPT + " fehlgeschlagen: " + e.message);
}
const seen = new Set(S.shots.map((s) => s.ts));
const merged = S.shots.concat((Array.isArray(saved) ? saved : []).filter((s) => !seen.has(s.ts)))
    .sort((a, b) => b.ts - a.ts);
S.shots = merged.slice(0, MARA_MAX_SHOTS);
archiveShots(merged.slice(MARA_MAX_SHOTS));
// Shots recorded before HX was stored per shot: take it from the temperature history if it's
// still there, then render any missing JPEGs
S.shots.forEach((s) => {
    if (s.hx && s.hx.length) return;
    const end = s.ts + ((s.duration || 0) + 1) * 1000;
    s.hx = S.hist.filter((r) => r[0] >= s.ts && r[0] <= end && typeof r[3] === "number")
        .map((r) => [(r[0] - s.ts) / 1000, r[3]]);
});
S.shots.forEach((s) => renderShot(s, true));
delete S.evicted;
// Cup count: the saved value unless memory survived a redeploy; republish it retained so the
// broker (and the ESP32 display) have it again after a Pi / Mosquitto restart
try {
    if (S.cups == null && fs.existsSync(CUPS_FILE)) S.cups = JSON.parse(fs.readFileSync(CUPS_FILE, "utf8")).cups;
} catch (e) {
    node.error("Lesen von " + CUPS_FILE + " fehlgeschlagen: " + e.message);
}
if (typeof S.cups !== "number") S.cups = 0;
flow.set("mara", S);
persistShots(S.shots);
persistCups(S.cups);
setTimeout(() => node.send([null, { topic: "__CUP_TOPIC__", payload: String(S.cups), retain: true }]), 3000);
node.status({ text: S.shots.length + " Shots, " + S.cups + " Tassen geladen" });
"""

STATE_FUNC = """
const now = Date.now();
const kind = String(msg.topic || "").split("/").pop();
let p = msg.payload;
if (typeof p === "string" && kind !== "cup_counter") {
    try { p = JSON.parse(p); } catch (e) { return null; }
}
const S = flow.get("mara") || maraInitialState();
const newestBefore = S.shots[0];
if (!maraReduce(S, kind, p, now)) return null;
if (S.evicted) {
    archiveShots(S.evicted);
    delete S.evicted;
}
let cupMsg = null;
if (S.shots[0] !== newestBefore) {
    persistShots(S.shots);
    renderShot(S.shots[0], false);
    // Every shot that makes it into the shot list is a cup; the retained publish comes back
    // through mqtt in as cup_counter and updates the pages and the ESP32 display
    S.cups = (S.cups || 0) + 1;
    persistCups(S.cups);
    cupMsg = { topic: "__CUP_TOPIC__", payload: String(S.cups), retain: true };
    node.status({ text: "letzter Shot " + new Date(S.shots[0].ts).toLocaleTimeString("de-DE") + ", " + S.cups + " Tassen" });
} else if (kind === "cup_counter") {
    persistCups(S.cups); // a value set from outside (e.g. mosquitto_pub) becomes the new count
}
flow.set("mara", S);
return [{ payload: JSON.stringify({ kind: kind, data: p, ts: now }) }, cupMsg];
"""

# GET /mara/shot/<ts>.jpg — the shot's JPEG. The file name is derived from the shot's start
# timestamp here on the Pi, so it always matches renderShot() regardless of the browser's time zone.
IMAGE_FUNC = """
const ts = Number(String(msg.req.params.ts || "").replace(/\\.jpg$/, ""));
const file = Number.isFinite(ts) && ts > 0 ? shotImagePath({ ts: ts }) : null;
if (file && fs.existsSync(file)) {
    msg.payload = fs.readFileSync(file);
    msg.headers = { "content-type": "image/jpeg", "cache-control": "max-age=86400" };
    msg.statusCode = 200;
} else {
    msg.payload = "Kein Bild für diesen Shot";
    msg.headers = { "content-type": "text/plain; charset=utf-8" };
    msg.statusCode = 404;
}
return msg;
"""

SNAPSHOT_FUNC = """
if (msg.payload !== "hello") return null;
return {
    _session: msg._session,
    payload: JSON.stringify({ kind: "snapshot", data: flow.get("mara") || null, ts: Date.now() }),
};
"""


def nid(name: str) -> str:
    """Stable 16-hex node id, so redeploys update nodes in place instead of duplicating them."""
    return hashlib.sha1(f"maratui-web/{name}".encode()).hexdigest()[:16]


def api(base: str, method: str, path: str, body=None):
    req = urllib.request.Request(
        base.rstrip("/") + path,
        method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Accept": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=15) as resp:
        raw = resp.read()
    return json.loads(raw) if raw else None


def find_broker(flows, wanted):
    brokers = [n for n in flows if n["type"] == "mqtt-broker"]
    if wanted:
        match = [b for b in brokers if b["id"] == wanted]
        if not match:
            sys.exit(f"mqtt-broker config node {wanted} not found")
        return match[0]["id"]
    # Prefer the broker the existing MaraTUI flows already use
    for b in brokers:
        if b.get("name") == "MaraTUI Broker":
            return b["id"]
    sys.exit("No 'MaraTUI Broker' mqtt-broker config node found; pass --broker <id>. Available: "
             + ", ".join(f"{b['id']} ({b.get('name')})" for b in brokers))


def build_flow(broker_id: str, data_dir: str = "") -> dict:
    reducer = (HERE / "reducer.js").read_text()
    page = (HERE / "index.html").read_text()
    if "/*__REDUCER__*/" not in page:
        sys.exit("index.html is missing the /*__REDUCER__*/ placeholder")
    page = page.replace("/*__REDUCER__*/", reducer)
    helpers = PERSIST_HELPERS.replace('"__DATA_DIR__"', json.dumps(data_dir))

    listener = nid("ws-listener")
    ws_out = nid("ws-out")
    nodes = [
        {"id": nid("comment"), "type": "comment", "name": f"MaraTUI Web-Dashboard → http://<host>:1880{PAGE_PATH}",
         "info": "Generated by maratui/dashboard/deploy.py — edit the files there and redeploy instead of editing here.",
         "x": 250, "y": 40, "wires": []},
        {"id": nid("mqtt-in"), "type": "mqtt in", "name": TOPIC, "topic": TOPIC, "qos": "0",
         "datatype": "utf8", "broker": broker_id, "nl": False, "rap": True, "rh": 0, "inputs": 0,
         "x": 130, "y": 100, "wires": [[nid("state")]]},
        {"id": nid("state"), "type": "function", "name": "Zustand + Broadcast",
         "func": reducer + helpers + STATE_FUNC.replace("__CUP_TOPIC__", CUP_TOPIC), "outputs": 2,
         "timeout": 0, "noerr": 0,
         "initialize": reducer + helpers + STATE_INIT.replace("__CUP_TOPIC__", CUP_TOPIC)
            .replace("__RENDER_PY__", json.dumps((HERE / "render_shot.py").read_text())), "finalize": "",
         "libs": [{"var": "fs", "module": "fs"}, {"var": "cp", "module": "child_process"}], "x": 360, "y": 100, "wires": [[ws_out], [nid("cup-out")]]},
        {"id": nid("cup-out"), "type": "mqtt out", "name": CUP_TOPIC, "topic": "", "qos": "1", "retain": "",
         "respTopic": "", "contentType": "", "userProps": "", "correl": "", "expiry": "", "broker": broker_id,
         "x": 610, "y": 60, "wires": []},
        {"id": nid("ws-in"), "type": "websocket in", "name": WS_PATH, "server": listener, "client": "",
         "x": 130, "y": 160, "wires": [[nid("snapshot")]]},
        {"id": nid("snapshot"), "type": "function", "name": "Snapshot an neuen Client",
         "func": SNAPSHOT_FUNC, "outputs": 1, "timeout": 0, "noerr": 0, "initialize": "",
         "finalize": "", "libs": [], "x": 370, "y": 160, "wires": [[ws_out]]},
        {"id": ws_out, "type": "websocket out", "name": WS_PATH, "server": listener, "client": "",
         "x": 600, "y": 130, "wires": []},
        {"id": nid("http-in"), "type": "http in", "name": f"GET {PAGE_PATH}", "url": PAGE_PATH,
         "method": "get", "upload": False, "swaggerDoc": "", "x": 130, "y": 220, "wires": [[nid("page")]]},
        {"id": nid("page"), "type": "template", "name": "index.html", "field": "payload",
         "fieldType": "msg", "format": "html", "syntax": "plain", "template": page, "output": "str",
         "x": 340, "y": 220, "wires": [[nid("http-out")]]},
        {"id": nid("img-in"), "type": "http in", "name": f"GET {PAGE_PATH}/shot/:ts", "url": f"{PAGE_PATH}/shot/:ts",
         "method": "get", "upload": False, "swaggerDoc": "", "x": 150, "y": 280, "wires": [[nid("img")]]},
        {"id": nid("img"), "type": "function", "name": "Shot-JPG ausliefern", "func": helpers + IMAGE_FUNC,
         "outputs": 1, "timeout": 0, "noerr": 0, "initialize": "", "finalize": "",
         "libs": [{"var": "fs", "module": "fs"}, {"var": "cp", "module": "child_process"}],
         "x": 370, "y": 280, "wires": [[nid("img-out")]]},
        {"id": nid("img-out"), "type": "http response", "name": "", "statusCode": "", "headers": {},
         "x": 570, "y": 280, "wires": []},
        {"id": nid("http-out"), "type": "http response", "name": "", "statusCode": "",
         "headers": {"content-type": "text/html; charset=utf-8", "cache-control": "no-cache"},
         "x": 530, "y": 220, "wires": []},
    ]
    configs = [
        {"id": listener, "type": "websocket-listener", "path": WS_PATH, "wholemsg": "false"},
    ]
    return {"label": TAB_LABEL, "nodes": nodes, "configs": configs}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--url", default="http://192.168.178.162:1880", help="Node-RED base URL")
    ap.add_argument("--broker", help="id of the mqtt-broker config node to subscribe with")
    ap.add_argument("--data-dir", default="", help="where shots.json / shotdoku.txt live on the Node-RED host "
                    "(default: $HOME/maratui of the Node-RED user)")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    flows = api(args.url, "GET", "/flows")
    broker = find_broker(flows, args.broker)
    flow = build_flow(broker, args.data_dir)
    if args.dry_run:
        print(json.dumps(flow, indent=2))
        return

    existing = [n for n in flows if n["type"] == "tab" and n.get("label") == TAB_LABEL]
    try:
        if existing:
            tab_id = existing[0]["id"]
            api(args.url, "PUT", f"/flow/{tab_id}", dict(flow, id=tab_id))
            print(f"Updated flow '{TAB_LABEL}' ({tab_id})")
        else:
            tab_id = api(args.url, "POST", "/flow", flow)["id"]
            print(f"Created flow '{TAB_LABEL}' ({tab_id})")
    except urllib.error.HTTPError as e:
        sys.exit(f"Node-RED rejected the flow: {e.code} {e.read().decode(errors='replace')}")
    print(f"Dashboard: {args.url.rstrip('/')}{PAGE_PATH}")


if __name__ == "__main__":
    main()
