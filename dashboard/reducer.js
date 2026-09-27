// Shared state reducer for the MaraTUI web dashboard.
//
// Runs in two places, injected verbatim by deploy.py: the Node-RED function node (which keeps the
// authoritative state in flow context and sends it to newly connected browsers as a snapshot) and
// the browser page (which applies the same live MQTT messages to its copy of that snapshot).
// Keeping one implementation means the page and the server can't drift apart.

var MARA_HISTORY_MS = 60 * 60 * 1000; // temperature history kept for the chart
var MARA_MAX_SHOTS = 10; // shown on the page and persisted; older ones go to the text archive
var MARA_MAX_EVENTS = 30;
// The firmware reports pump runs under 15 s as `shot_aborted` (rinse / flush, but also a short
// test pour). Those still go into the shot list when something clearly landed in the cup.
var MARA_MIN_SHORT_SHOT_G = 5;

function maraInitialState() {
  return {
    tele: null, // last mara/telemetry payload + ts
    status: null, // last mara/status payload + ts
    cups: null, // last mara/cup_counter value
    weight: null, // { g, ts } from mara/scale
    hist: [], // [ts, boiler_now_c, boiler_target_c, hx_now_c, heating 0/1, pump 0/1]
    shot: null, // current or most recent pump run: { start, end, duration, weight_g, aborted, curve: [[s, g]], hx: [[s, °C]] }
    shots: [], // completed shots, newest first (short ones only with >= MARA_MIN_SHORT_SHOT_G)
    events: [], // mara/events payloads + ts, newest first
  };
}

// Apply one MQTT message (topic suffix `kind`, parsed payload `p`, server time `now`) to state `S`.
// Returns false for topics the dashboard doesn't use.
function maraReduce(S, kind, p, now) {
  switch (kind) {
    case "telemetry": {
      S.tele = Object.assign({}, p, { ts: now });
      S.hist.push([now, p.boiler_now_c, p.boiler_target_c, p.hx_now_c, p.heating_on ? 1 : 0, p.pump_on ? 1 : 0]);
      if (S.shot && !S.shot.end && typeof p.hx_now_c === "number") (S.shot.hx = S.shot.hx || []).push([(now - S.shot.start) / 1000, p.hx_now_c]);
      var cutoff = now - MARA_HISTORY_MS;
      while (S.hist.length && S.hist[0][0] < cutoff) S.hist.shift();
      return true;
    }
    case "status":
      S.status = Object.assign({}, p, { ts: now });
      return true;
    case "cup_counter": {
      var cups = Number(p);
      if (!isNaN(cups)) S.cups = cups;
      return true;
    }
    case "scale":
      if (!p || typeof p.weight_g !== "number") return false;
      S.weight = { g: p.weight_g, ts: now };
      if (S.shot && !S.shot.end) S.shot.curve.push([(now - S.shot.start) / 1000, p.weight_g]);
      return true;
    case "events": {
      if (!p || !p.type) return false;
      S.events.unshift(Object.assign({}, p, { ts: now }));
      if (S.events.length > MARA_MAX_EVENTS) S.events.length = MARA_MAX_EVENTS;
      if (p.type === "shot_started") {
        var hx0 = S.tele && typeof S.tele.hx_now_c === "number" ? [[0, S.tele.hx_now_c]] : [];
        S.shot = { start: now, end: null, duration: null, weight_g: null, aborted: false, curve: [[0, 0]], hx: hx0 };
      } else if (p.type === "shot_ended" || p.type === "shot_aborted") {
        var shot = S.shot && !S.shot.end ? S.shot : { start: now - (p.duration || 0) * 1000, curve: [], hx: [] };
        shot.end = now;
        shot.duration = p.duration;
        shot.weight_g = typeof p.weight_g === "number" ? p.weight_g : null;
        shot.aborted = p.type === "shot_aborted";
        if (shot.weight_g !== null && shot.curve.length) {
          // `duration` is whole seconds, so never place the end point before the last sample
          var lastT = shot.curve[shot.curve.length - 1][0];
          shot.curve.push([Math.max(lastT, p.duration), shot.weight_g]);
        }
        S.shot = shot;
        if (!shot.aborted || (shot.weight_g !== null && shot.weight_g >= MARA_MIN_SHORT_SHOT_G)) {
          S.shots.unshift({ ts: shot.start, duration: shot.duration, weight_g: shot.weight_g, aborted: shot.aborted, curve: shot.curve, hx: shot.hx });
          // Shots pushed out of the list are handed to the caller (Node-RED archives them)
          S.evicted = S.shots.splice(MARA_MAX_SHOTS);
        }
      }
      return true;
    }
    default:
      return false;
  }
}
