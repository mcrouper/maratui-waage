//! Start screen animation, looping while the ESP connects and waits for the machine: a Mara X
//! overheats and explodes, a red sports car pulls up through the smoke, its driver gets out and
//! waves with her coffee to go, and they drive off — then it starts over.
//!
//! The scene is painted at half resolution into a small RGB565 canvas (pixel-art look, ~27 KB
//! instead of a 107 KB full-size frame) and streamed to the display doubled, straight past
//! Ratatui like the smooth digits. Only canvas rows that changed since the last frame are sent,
//! so the static sky and street cost nothing while the sprites move.

use std::f32::consts::FRAC_PI_2;
use std::time::{Duration, Instant};

use mousefood::embedded_graphics::pixelcolor::{IntoStorage, Rgb565};
use mousefood::embedded_graphics::prelude::{DrawTarget, Point, RgbColor, Size};
use mousefood::embedded_graphics::primitives::Rectangle;

/// Canvas size in scene pixels; each one is drawn as a `SCALE`x`SCALE` block.
const WIDTH: usize = 160;
const HEIGHT: usize = 84;
const SCALE: usize = 2;
/// Display y of the scene's top edge: just below the version line (cell row 0). The scene ends
/// at y 182, where the loading block starts.
const TOP_PX: i32 = 14;
const FRAME_INTERVAL: Duration = Duration::from_millis(66);
const LOOP_SECS: f32 = 10.0;

// ── Timeline (seconds into the loop) ─────────────────────────────────────────────────────────
const SHAKE_START: f32 = 0.9;
const BOOM: f32 = 2.4;
const CAR_ARRIVE: (f32, f32) = (3.2, 5.0);
const DRIVER_OUT: (f32, f32) = (5.2, 6.0);
const DRIVER_BACK: (f32, f32) = (8.2, 8.9);
const CAR_LEAVE: (f32, f32) = (8.9, 10.0);

// ── Layout (scene pixels) ────────────────────────────────────────────────────────────────────
const SIDEWALK_Y: i32 = 60;
const ROAD_Y: i32 = 64;
const MACHINE_X: i32 = 14;
const MACHINE_Y: i32 = 36;
const CAR_Y: i32 = 63;
const CAR_PARK_X: f32 = 78.0;
const CAR_ENTRY_X: f32 = 165.0;
const DRIVER_Y: i32 = 56;

pub struct BootAnimation {
    canvas: Vec<Rgb565>,
    /// Hash of each canvas row as currently on the display (`None`: unknown, resend).
    shown_rows: Vec<Option<u32>>,
    started_at: Option<Instant>,
    last_frame_at: Option<Instant>,
}

impl Default for BootAnimation {
    fn default() -> Self {
        Self {
            canvas: vec![Rgb565::BLACK; WIDTH * HEIGHT],
            shown_rows: vec![None; HEIGHT],
            started_at: None,
            last_frame_at: None,
        }
    }
}

impl BootAnimation {
    /// Repaint the whole scene on the next `render` (another screen or a full clear drew over it).
    pub fn invalidate(&mut self) {
        self.shown_rows.fill(None);
        self.last_frame_at = None;
    }

    /// Draw the frame for `now`. Cheap to call every loop tick: frames are paced to
    /// `FRAME_INTERVAL`, and calls in between return right away.
    pub fn render<D>(&mut self, display: &mut D, now: Instant) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        if self
            .last_frame_at
            .is_some_and(|last| now.saturating_duration_since(last) < FRAME_INTERVAL)
        {
            return Ok(());
        }
        self.last_frame_at = Some(now);
        let started_at = *self.started_at.get_or_insert(now);
        let t = now.saturating_duration_since(started_at).as_secs_f32() % LOOP_SECS;

        paint_scene(&mut Canvas(&mut self.canvas), t);
        self.flush(display)
    }

    /// Send every band of rows that differs from what is on the display, pixel-doubled.
    fn flush<D>(&mut self, display: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let hashes: Vec<u32> = self.canvas.chunks(WIDTH).map(row_hash).collect();
        let mut y = 0;
        while y < HEIGHT {
            if self.shown_rows[y] == Some(hashes[y]) {
                y += 1;
                continue;
            }
            let start = y;
            while y < HEIGHT && self.shown_rows[y] != Some(hashes[y]) {
                self.shown_rows[y] = Some(hashes[y]);
                y += 1;
            }
            let area = Rectangle::new(
                Point::new(0, TOP_PX + (start * SCALE) as i32),
                Size::new((WIDTH * SCALE) as u32, ((y - start) * SCALE) as u32),
            );
            let pixels = self.canvas[start * WIDTH..y * WIDTH]
                .chunks(WIDTH)
                .flat_map(|row| std::iter::repeat_n(row, SCALE))
                .flat_map(|row| row.iter().flat_map(|&c| std::iter::repeat_n(c, SCALE)));
            display.fill_contiguous(&area, pixels)?;
        }
        Ok(())
    }
}

fn row_hash(row: &[Rgb565]) -> u32 {
    row.iter().fold(0x811C_9DC5, |hash, &c| {
        (hash ^ u32::from(c.into_storage())).wrapping_mul(0x0100_0193)
    })
}

// ── Scene ────────────────────────────────────────────────────────────────────────────────────

fn paint_scene(c: &mut Canvas, t: f32) {
    paint_backdrop(c, t);
    paint_title(c);

    if t < BOOM {
        paint_machine(c, t);
    } else {
        paint_wreck(c, t);
        paint_debris(c, t - BOOM);
        paint_smoke(c, t - BOOM);
        paint_fireball(c, t - BOOM);
    }

    if let Some(car_x) = car_x(t) {
        paint_car(c, car_x, t);
        if let Some((x, walking)) = driver_x(t, car_x) {
            let fade = phase(t, DRIVER_OUT.0, DRIVER_OUT.0 + 0.3)
                .min(1.0 - phase(t, DRIVER_BACK.1 - 0.2, DRIVER_BACK.1));
            paint_driver(c, x, t, walking, fade);
        }
    }

    // White flash of the explosion over everything.
    let flash = 1.0 - phase(t, BOOM, BOOM + 0.35);
    if t >= BOOM && flash > 0.0 {
        c.wash(rgb(0xFFF8E8), flash * 0.85);
    }
}

fn paint_backdrop(c: &mut Canvas, t: f32) {
    // Dusk sky.
    const SKY: [(i32, u32); 4] = [
        (0, 0x0A0E2A),
        (28, 0x3A2350),
        (46, 0x9A4A5A),
        (60, 0xF09050),
    ];
    for y in 0..SIDEWALK_Y {
        let i = SKY
            .iter()
            .rposition(|&(sy, _)| sy <= y)
            .unwrap_or(0)
            .min(SKY.len() - 2);
        let ((y0, a), (y1, b)) = (SKY[i], SKY[i + 1]);
        let color = mix(rgb(a), rgb(b), (y - y0) as f32 / (y1 - y0) as f32);
        c.rect(0, y, WIDTH as i32, 1, color);
    }

    // Twinkling stars.
    for i in 0..26 {
        let (x, y) = ((noise(i) * 160.0) as i32, (noise(i + 100) * 30.0) as i32);
        let dim = ((t * 3.0 + noise(i + 200) * 10.0) as i32) % 5 == 0;
        c.set(x, y, if dim { rgb(0x6A6A8A) } else { rgb(0xE8E8FF) });
    }

    // City skyline with lit windows.
    let mut x = 0;
    let mut i = 0;
    while x < WIDTH as i32 {
        let width = 6 + (noise(i + 300) * 9.0) as i32;
        let height = 5 + (noise(i + 400) * 16.0) as i32;
        let top = SIDEWALK_Y - height;
        c.rect(x, top, width, height, rgb(0x1C1430));
        for wy in (top + 2..SIDEWALK_Y - 1).step_by(3) {
            for wx in (x + 1..x + width - 1).step_by(2) {
                if noise((wx * 7 + wy * 131) as u32) > 0.72 {
                    c.set(wx, wy, rgb(0xE8B850));
                }
            }
        }
        x += width + 1;
        i += 1;
    }

    // Sidewalk and road with a dashed center line.
    c.rect(
        0,
        SIDEWALK_Y,
        WIDTH as i32,
        ROAD_Y - SIDEWALK_Y,
        rgb(0x5A5060),
    );
    c.rect(0, SIDEWALK_Y, WIDTH as i32, 1, rgb(0x7A7080));
    c.rect(
        0,
        ROAD_Y,
        WIDTH as i32,
        HEIGHT as i32 - ROAD_Y,
        rgb(0x2A2830),
    );
    c.rect(0, ROAD_Y, WIDTH as i32, 1, rgb(0x1A1820));
    for x in (0..WIDTH as i32).step_by(14) {
        c.rect(x, 75, 8, 1, rgb(0xC8B060));
    }
}

fn paint_title(c: &mut Canvas) {
    let (x, y) = (WIDTH as i32 - 50, 4);
    c.text(x + 1, y + 1, "MAXI TUI", rgb(0x3A1A10));
    c.text(x, y, "MAXI TUI", rgb(0xFFB000));
}

/// The Mara X before it blows: shaking harder, glowing red and puffing steam as it overheats.
fn paint_machine(c: &mut Canvas, t: f32) {
    let heat = phase(t, SHAKE_START, BOOM);
    let jitter = if heat > 0.0 {
        ((t * 30.0) as i32 % 3) - 1
    } else {
        0
    };
    let (x, y) = (MACHINE_X + jitter, MACHINE_Y);

    let glow = rgb(0xFF3010);
    c.sprite(x, y, MARA_X, 1.0, |ch| {
        machine_color(ch).map(|color| match ch {
            'S' | 's' | 'c' => mix(color, glow, heat * 0.65),
            _ => color,
        })
    });

    // Steam wand and its knob.
    c.rect(x + 22, y + 8, 2, 1, rgb(0x101010));
    c.rect(x + 23, y + 9, 1, 10, rgb(0xF4F6F8));

    // Power LED blinking faster and faster, coffee dripping into the cup.
    let blink = (t * (2.0 + heat * 14.0)) as i32 % 2 == 0;
    c.set(
        x + 3,
        y + 9,
        if blink { rgb(0xFF2000) } else { rgb(0x502010) },
    );
    if (t * 4.0) as i32 % 2 == 0 {
        c.set(x + 11, y + 16, rgb(0x5A3418));
    }

    // Steam puffing out of the top, thicker as it heats up.
    for k in 0..3 {
        let age = (t * 1.3 + k as f32 / 3.0).fract();
        let px = x as f32 + 12.0 + (age * 9.0 + k as f32).sin() * 3.0;
        let py = y as f32 - age * 16.0;
        c.disc(
            px,
            py,
            1.0 + age * 3.0,
            rgb(0xE8E8F0),
            (1.0 - age) * (0.25 + heat * 0.6),
        );
    }
}

/// What's left of the machine: a charred heap with glowing embers.
fn paint_wreck(c: &mut Canvas, t: f32) {
    for dx in 0..22 {
        let height = 2 + (noise(dx as u32 + 500) * 5.0) as i32;
        c.rect(
            MACHINE_X + 1 + dx,
            SIDEWALK_Y - height,
            1,
            height,
            rgb(0x2A2220),
        );
    }
    for i in 0..6 {
        let x = MACHINE_X + 2 + (noise(i + 600) * 20.0) as i32;
        let y = SIDEWALK_Y - 1 - (noise(i + 700) * 3.0) as i32;
        let hot = ((t * 5.0 + noise(i + 800) * 4.0) as i32) % 3 != 0;
        c.set(x, y, if hot { rgb(0xFF7A1A) } else { rgb(0x8A2A10) });
    }
}

fn paint_fireball(c: &mut Canvas, age: f32) {
    if age > 1.6 {
        return;
    }
    let grow = ease_out(phase(age, 0.0, 0.5));
    let fade = 1.0 - phase(age, 0.5, 1.6);
    let radius = 3.0 + grow * 21.0;
    let (cx, cy) = ((MACHINE_X + 12) as f32, (MACHINE_Y + 12) as f32);
    c.disc(cx, cy, radius, rgb(0xFF5A10), fade * 0.9);
    c.disc(cx, cy - 1.0, radius * 0.7, rgb(0xFFB030), fade);
    c.disc(
        cx,
        cy - 2.0,
        radius * 0.4,
        rgb(0xFFF0B0),
        fade * (1.0 - grow * 0.5),
    );
}

fn paint_smoke(c: &mut Canvas, age: f32) {
    for k in 0..8 {
        let puff_age = age - 0.3 - k as f32 * 0.15;
        if !(0.0..5.0).contains(&puff_age) {
            continue;
        }
        let x = (MACHINE_X + 12) as f32 + (noise(k + 900) - 0.5) * 24.0 + puff_age * 4.0;
        let y = (MACHINE_Y + 10) as f32 - puff_age * 7.0 - noise(k + 950) * 6.0;
        c.disc(
            x,
            y,
            3.0 + puff_age * 2.2,
            rgb(0x4A4450),
            0.55 * (1.0 - puff_age / 5.0),
        );
    }
}

/// Machine parts flying off in ballistic arcs and landing on the sidewalk.
fn paint_debris(c: &mut Canvas, age: f32) {
    const GRAVITY: f32 = 90.0;
    let (cx, cy) = ((MACHINE_X + 12) as f32, (MACHINE_Y + 12) as f32);
    for i in 0..22 {
        let vx = (noise(i) - 0.5) * 100.0;
        let vy = -30.0 - noise(i + 50) * 65.0;
        let ground = SIDEWALK_Y as f32 + noise(i + 70) * 3.0;
        // Time at which the arc reaches the ground; the part stays where it landed.
        let landing = (-vy + (vy * vy + 2.0 * GRAVITY * (ground - cy)).sqrt()) / GRAVITY;
        let t = age.min(landing);
        let (x, y) = (
            (cx + vx * t) as i32,
            (cy + vy * t + 0.5 * GRAVITY * t * t) as i32,
        );
        match i % 5 {
            // The pressure gauge.
            0 => {
                c.rect(x, y, 2, 2, rgb(0xF0F0F0));
                c.set(x + 1, y, rgb(0xD02020));
            }
            // Portafilter.
            1 => {
                c.rect(x, y, 3, 1, rgb(0x101010));
                c.rect(x + 3, y, 2, 2, rgb(0xF4F6F8));
            }
            // Panels.
            2 | 3 => c.rect(x, y, 2, 2, rgb(0xB8BCC4)),
            // Coffee drops, soaking away once they land.
            _ if age < landing => c.set(x, y, rgb(0x5A3418)),
            _ => {}
        }
    }
}

/// Car x position (sprite's left edge), or `None` when it's off stage.
fn car_x(t: f32) -> Option<f32> {
    if t < CAR_ARRIVE.0 {
        return None;
    }
    let arrive = ease_out(phase(t, CAR_ARRIVE.0, CAR_ARRIVE.1));
    let leave = ease_in(phase(t, CAR_LEAVE.0, CAR_LEAVE.1));
    Some(CAR_ENTRY_X - (CAR_ENTRY_X - CAR_PARK_X) * arrive - (CAR_PARK_X + 60.0) * leave)
}

fn paint_car(c: &mut Canvas, car_x: f32, t: f32) {
    let x = car_x.round() as i32;
    let moving = phase(t, CAR_ARRIVE.0, CAR_ARRIVE.1) < 1.0 || t > CAR_LEAVE.0;

    // Headlight beam ahead (the car faces left).
    for d in 1..34 {
        let spread = d as f32 * 0.3;
        let alpha = 0.3 * (1.0 - d as f32 / 34.0);
        for dy in -spread as i32..=spread as i32 {
            c.blend(x - d, CAR_Y + 8 + dy, rgb(0xFFF0B0), alpha);
        }
    }

    // Exhaust puffs while driving.
    if moving {
        for k in 0..4 {
            let age = (t * 3.0 + k as f32 / 4.0).fract();
            let px = car_x + 48.0 + age * 8.0;
            let py = (CAR_Y + 11) as f32 - age * 3.0;
            c.disc(px, py, 1.0 + age * 2.0, rgb(0x9A98A8), (1.0 - age) * 0.45);
        }
    }

    c.sprite(x, CAR_Y, SPORTS_CAR, 1.0, car_color);

    // Wheels, spokes turning with the distance rolled.
    let angle = car_x / 4.0;
    for wheel_x in [9, 37] {
        let (cx, cy) = ((x + wheel_x) as f32 + 0.5, (CAR_Y + 12) as f32 + 0.5);
        c.disc(cx, cy, 4.3, rgb(0x141414), 1.0);
        c.disc(cx, cy, 2.7, rgb(0xB8BCC4), 1.0);
        for spoke in 0..2 {
            let a = angle + spoke as f32 * FRAC_PI_2;
            let (dx, dy) = (a.cos() * 2.2, a.sin() * 2.2);
            c.line(cx - dx, cy - dy, cx + dx, cy + dy, rgb(0x5A5E66));
        }
    }
}

/// Driver x position and whether she is walking, or `None` while she's in the car.
fn driver_x(t: f32, car_x: f32) -> Option<(f32, bool)> {
    let door = car_x + 22.0;
    let spot = car_x + 52.0;
    if t < DRIVER_OUT.0 || t >= DRIVER_BACK.1 {
        None
    } else if t < DRIVER_OUT.1 {
        Some((
            door + (spot - door) * phase(t, DRIVER_OUT.0, DRIVER_OUT.1),
            true,
        ))
    } else if t < DRIVER_BACK.0 {
        Some((spot, false))
    } else {
        Some((
            spot - (spot - door) * phase(t, DRIVER_BACK.0, DRIVER_BACK.1),
            true,
        ))
    }
}

fn paint_driver(c: &mut Canvas, x: f32, t: f32, walking: bool, alpha: f32) {
    let step = (t * 7.0) as i32 % 2 == 1;
    let x = x.round() as i32;
    let y = DRIVER_Y - i32::from(walking && step);
    let skin = rgb(0xF5C8A8);

    c.sprite(x, y, DRIVER, alpha, driver_color);

    // Legs (stepping while walking) and shoes.
    let (left_lift, right_lift) = match (walking, step) {
        (false, _) => (0, 0),
        (true, false) => (1, 0),
        (true, true) => (0, 1),
    };
    for (leg_x, lift) in [(4, left_lift), (6, right_lift)] {
        let bottom = y + 22 - lift;
        c.blend_rect(x + leg_x, y + 17, 1, bottom - (y + 17), skin, alpha);
        c.blend_rect(
            x + leg_x - 1 + (leg_x / 6),
            bottom,
            2,
            1,
            rgb(0xC01030),
            alpha,
        );
    }

    // Left arm holds a coffee to go.
    c.blend_line(x + 1, y + 10, x, y + 14, skin, alpha);
    c.blend_rect(x - 2, y + 12, 2, 3, rgb(0xF4F0E8), alpha);
    c.blend_rect(x - 2, y + 13, 2, 1, rgb(0x8A5A30), alpha);

    // Right arm swings while walking and waves once she's standing.
    let wave = (t * 5.0) as i32 % 2 == 0;
    let hand = match (walking, wave) {
        (true, _) => (x + 10, y + 15 - i32::from(step)),
        (false, true) => (x + 12, y + 3),
        (false, false) => (x + 10, y + 2),
    };
    c.blend_line(x + 9, y + 10, hand.0, hand.1, skin, alpha);

    // Hair blowing in the wind.
    let flutter = (t * 6.0).sin() > 0.0;
    c.blend(x + 10, y + 4 + i32::from(flutter), rgb(0xF2D272), alpha);
    c.blend(x + 10, y + 7 - i32::from(flutter), rgb(0xC9A048), alpha);

    // A heart floating up while she waves.
    if !walking {
        let rise = phase(t, DRIVER_OUT.1 + 0.3, DRIVER_BACK.0);
        if rise > 0.0 && rise < 1.0 {
            let heart_y = y - 6 - (rise * 14.0) as i32;
            c.sprite(x + 3, heart_y, HEART, alpha * (1.0 - rise), |ch| {
                (ch == 'P').then(|| rgb(0xFF5A8A))
            });
        }
    }
}

// ── Sprites ──────────────────────────────────────────────────────────────────────────────────

/// Lelit Mara X, front view: gauge, E61 group with portafilter, cup on the drip tray.
const MARA_X: &[&str] = &[
    "..dddddddddddddddddddd..",
    ".dccccccccccccccccccccd.",
    ".sSSSSSSSSSSSSSSSSSSSSs.",
    ".sSSSSSSSSkkkkSSSSSSSSs.",
    ".sSSSSSSSkggggkSSSSSSSs.",
    ".sSSSSSSSkggrgkSSSSSSSs.",
    ".sSSSSSSSkggggkSSSSSSSs.",
    ".sSSSSSSSSkkkkSSSSSSSSs.",
    ".sSSSSSSSSSSSSSSSSSSSSs.",
    ".sSSSSSSSSSSSSSSSSSSSSs.",
    ".sSSSSSSSccccccSSSSSSSs.",
    ".sSSSSSSccccccccSSSSSSs.",
    ".sSSSSSSSccccccSSSSSSSs.",
    ".sSSSSSSSSddddSSSSSSSSs.",
    ".sSSSSkkkkddddSSSSSSSSs.",
    ".sSSSSdddddccdddddSSSSs.",
    ".sSSSSddddddddddddSSSSs.",
    ".sSSSSddddddddddddSSSSs.",
    ".sSSSSddddwwwwddddSSSSs.",
    ".sSSSSddddwwwwddddSSSSs.",
    ".sSSSSccccccccccccSSSSs.",
    ".sSSSSkckckckckckcSSSSs.",
    ".sSSSSSSSSSSSSSSSSSSSSs.",
    ".ssssssssssssssssssssss.",
    "..kk................kk..",
];

fn machine_color(ch: char) -> Option<Rgb565> {
    Some(rgb(match ch {
        'S' => 0xC8CCD0,
        's' => 0x8A9096,
        'c' => 0xF4F6F8,
        'd' => 0x30343A,
        'k' => 0x101010,
        'g' => 0xF0F0F0,
        'r' => 0xD02020,
        'w' => 0xFFFFFF,
        _ => return None,
    }))
}

/// Red rear-engined sports coupe, side view facing left. Wheels are drawn separately.
const SPORTS_CAR: &[&str] = &[
    "...................hhhhhhhhhhh..................",
    "..................hwwwwwRRWWWWh.................",
    ".................hwWWWWWRRWWWWWhhh..............",
    "................hwWWWWWWRRWWWWWWWWh.............",
    "...............hwWWWWWWWRRWWWWWWWWWhhh.......hhh",
    "..............hRRWWWWWWWRRWWWWWWWWWWRRhhh....RRR",
    "........hhhhhhRRRRRRRRRRRRrRRRRRRRRRRRRRRhhhhTTT",
    "..hhhhhhRRRRRRRRRRRRRRRRRRrRcRRRRRRRRRRRRRRRRTTT",
    "LLLhhhh.....hhhhhhhhhhhhhhrhhhhhhhh.....hhhhhRRR",
    "RRRRRR.......RRRRRRRRRRRRRrRRRRRRR.......RRRRRRR",
    "RRRRR.........RRRRRRRRRRRRrRRRRRR.........RRRRRR",
    "rrrrr.........rrrrrrrrrrrrrrrrrrr.........rrrrrr",
    "kkkkr.........rrrrrrrrrrrrrrrrrrr.........rrkkkk",
];

fn car_color(ch: char) -> Option<Rgb565> {
    Some(rgb(match ch {
        'R' => 0xD0201C,
        'r' => 0x8A1010,
        'h' => 0xFF6A5A,
        'W' => 0x2A3A5A,
        'w' => 0x6A8AB0,
        'c' => 0xE6E6E6,
        'L' => 0xFFF3B0,
        'T' => 0xFF3020,
        'k' => 0x101010,
        _ => return None,
    }))
}

/// The driver from the waist up plus her skirt; legs and arms are drawn separately to animate.
const DRIVER: &[&str] = &[
    "...yYYYy...",
    "..yYYYYYy..",
    ".yYYFFFYYy.",
    ".YYFEFEFYY.",
    ".YYFFFFFYY.",
    ".YYFFLFFYY.",
    ".YYfFFFfYY.",
    ".YYYYFYYYY.",
    ".YYDDDDDYY.",
    ".YyDDDDDyY.",
    "..yDDDDDy..",
    "...DdDdD...",
    "...DDDDD...",
    "..DDDDDDD..",
    "..DDDDDDD..",
    ".DDDDDDDDD.",
    ".dDDDDDDDd.",
];

fn driver_color(ch: char) -> Option<Rgb565> {
    Some(rgb(match ch {
        'Y' => 0xF2D272,
        'y' => 0xC9A048,
        'F' => 0xF5C8A8,
        'f' => 0xD9A080,
        'E' => 0x2A4060,
        'L' => 0xD04060,
        'D' => 0x2E5AAC,
        'd' => 0x1E3C78,
        _ => return None,
    }))
}

const HEART: &[&str] = &[".P.P.", "PPPPP", ".PPP.", "..P.."];

/// 5x7 pixel font, just the letters of the title.
fn glyph(ch: char) -> Option<[&'static str; 7]> {
    Some(match ch {
        'M' => [
            "X...X", "XX.XX", "X.X.X", "X.X.X", "X...X", "X...X", "X...X",
        ],
        'A' => [
            ".XXX.", "X...X", "X...X", "XXXXX", "X...X", "X...X", "X...X",
        ],
        'X' => [
            "X...X", "X...X", ".X.X.", "..X..", ".X.X.", "X...X", "X...X",
        ],
        'I' => [
            ".XXX.", "..X..", "..X..", "..X..", "..X..", "..X..", ".XXX.",
        ],
        'T' => [
            "XXXXX", "..X..", "..X..", "..X..", "..X..", "..X..", "..X..",
        ],
        'U' => [
            "X...X", "X...X", "X...X", "X...X", "X...X", "X...X", ".XXX.",
        ],
        _ => return None,
    })
}

// ── Canvas ───────────────────────────────────────────────────────────────────────────────────

struct Canvas<'a>(&'a mut [Rgb565]);

impl Canvas<'_> {
    fn index(x: i32, y: i32) -> Option<usize> {
        let in_bounds = (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y);
        in_bounds.then(|| y as usize * WIDTH + x as usize)
    }

    fn set(&mut self, x: i32, y: i32, color: Rgb565) {
        if let Some(i) = Self::index(x, y) {
            self.0[i] = color;
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: Rgb565, alpha: f32) {
        if let Some(i) = Self::index(x, y) {
            self.0[i] = mix(self.0[i], color, alpha);
        }
    }

    fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Rgb565) {
        self.blend_rect(x, y, width, height, color, 1.0);
    }

    fn blend_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Rgb565, alpha: f32) {
        for py in y..y + height {
            for px in x..x + width {
                self.blend(px, py, color, alpha);
            }
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, radius: f32, color: Rgb565, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let (x0, x1) = ((cx - radius).floor() as i32, (cx + radius).ceil() as i32);
        let (y0, y1) = ((cy - radius).floor() as i32, (cy + radius).ceil() as i32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= radius * radius {
                    self.blend(x, y, color, alpha.min(1.0));
                }
            }
        }
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Rgb565) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
        for s in 0..=steps {
            let f = s as f32 / steps as f32;
            self.set(
                (x0 + (x1 - x0) * f).floor() as i32,
                (y0 + (y1 - y0) * f).floor() as i32,
                color,
            );
        }
    }

    fn blend_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Rgb565, alpha: f32) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        for s in 0..=steps {
            let x = x0 + (x1 - x0) * s / steps;
            let y = y0 + (y1 - y0) * s / steps;
            self.blend(x, y, color, alpha);
        }
    }

    /// Draw ASCII-art `rows`, mapping each character through `color` (`None`: transparent).
    fn sprite(
        &mut self,
        x: i32,
        y: i32,
        rows: &[&str],
        alpha: f32,
        color: impl Fn(char) -> Option<Rgb565>,
    ) {
        for (dy, row) in rows.iter().enumerate() {
            for (dx, ch) in row.chars().enumerate() {
                if let Some(c) = color(ch) {
                    self.blend(x + dx as i32, y + dy as i32, c, alpha);
                }
            }
        }
    }

    fn text(&mut self, x: i32, y: i32, text: &str, color: Rgb565) {
        for (i, ch) in text.chars().enumerate() {
            if let Some(rows) = glyph(ch) {
                self.sprite(x + i as i32 * 6, y, &rows, 1.0, |p| {
                    (p == 'X').then_some(color)
                });
            }
        }
    }

    /// Blend the whole canvas toward `color`.
    fn wash(&mut self, color: Rgb565, alpha: f32) {
        for px in self.0.iter_mut() {
            *px = mix(*px, color, alpha);
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────────────────────

const fn rgb(hex: u32) -> Rgb565 {
    Rgb565::new(
        (hex >> 19) as u8 & 0x1F,
        (hex >> 10) as u8 & 0x3F,
        (hex >> 3) as u8 & 0x1F,
    )
}

fn mix(a: Rgb565, b: Rgb565, f: f32) -> Rgb565 {
    let f = f.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * f + 0.5) as u8;
    Rgb565::new(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Progress of `t` through `start..end`, clamped to `0..=1`.
fn phase(t: f32, start: f32, end: f32) -> f32 {
    ((t - start) / (end - start)).clamp(0.0, 1.0)
}

fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p).powi(3)
}

fn ease_in(p: f32) -> f32 {
    p.powi(3)
}

/// Deterministic pseudo-random value in `0..1` for seed `i`.
fn noise(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprites_are_rectangular() {
        for sprite in [MARA_X, SPORTS_CAR, DRIVER, HEART] {
            let width = sprite[0].len();
            assert!(sprite.iter().all(|row| row.len() == width));
        }
    }

    #[test]
    fn whole_loop_paints_without_panicking() {
        let mut canvas = vec![Rgb565::BLACK; WIDTH * HEIGHT];
        for frame in 0..(LOOP_SECS * 30.0) as usize {
            paint_scene(&mut Canvas(&mut canvas), frame as f32 / 30.0);
        }
    }
}
