#!/usr/bin/env python3
"""Drag-and-drop color editor for the MaraTUI Dashboard and start screen.

Left: a preview of the display (2x scale) and a list of every color slot. Right: a palette of
muted colors. Drag a swatch onto an element in the preview (or onto a row of the list) to
recolor it. "Speichern" writes the colors back into the palette constants in
`src/screens/dashboard.rs` and `src/screens/connecting.rs`; then rebuild and flash.

The "Startvideo" page plays the boot animation as the firmware draws it: it runs
`examples/boot_frames.rs` (the firmware's own scene code, built for the host) and plays the
frames, so changes to `src/screens/boot_animation.rs` show up after "Neu rendern" — no flashing.

The preview is a close mock of the firmware's layout (45x17 cells of 7x14 px), not the
firmware's own renderer, so fonts and spacing differ slightly from the real display.

Run from anywhere:  python3 tools/color_editor.py
"""

import re
import subprocess
import tempfile
import threading
import tkinter as tk
from pathlib import Path
from tkinter import colorchooser, messagebox

REPO = Path(__file__).resolve().parent.parent
DASHBOARD_RS = REPO / "src/screens/dashboard.rs"
CONNECTING_RS = REPO / "src/screens/connecting.rs"
BOOT_FRAMES = Path(tempfile.gettempdir()) / "maratui_boot_frames.bin"
BOOT_FRAMES_CMD = ["cargo", "run", "-q", "--example", "boot_frames", "--no-default-features",
                   "--features", "simulator", "--target", "x86_64-unknown-linux-gnu", "--",
                   str(BOOT_FRAMES)]

SCALE = 2
CELL_W, CELL_H = 7 * SCALE, 14 * SCALE
COLS, ROWS = 45, 17
DISPLAY_W, DISPLAY_H = 320 * SCALE, 240 * SCALE
BACKGROUND = "#000000"
FONT = "DejaVu Sans Mono"

# How Mousefood's default ANSI theme renders named Ratatui colors.
NAMED_COLORS = {
    "Black": "#000000",
    "White": "#FFFFFF",
    "Red": "#FF0000",
    "Green": "#00FF00",
    "Yellow": "#FFFF00",
    "Blue": "#0000FF",
    "Magenta": "#FF00FF",
    "Cyan": "#00FFFF",
    "Gray": "#7F7F7F",
    "DarkGray": "#AAAAAA",
    "Reset": "#FFFFFF",
}

# (key = Rust constant, file, German label). Order = order of the element list.
ELEMENTS = [
    ("STYLE_BORDER", DASHBOARD_RS, "Rahmen der Kästen"),
    ("STYLE_TITLE", DASHBOARD_RS, "Kastentitel (Timer, Weight, Boiler, %)"),
    ("STYLE_HEADER", DASHBOARD_RS, "Kopfzeile (Modus, Tassenzähler)"),
    ("STYLE_INDICATOR", DASHBOARD_RS, "HX-Beschriftung, HEAT/PUMP"),
    ("COLOR_HX_VALUE", DASHBOARD_RS, "HX-Temperatur"),
    ("STYLE_TIMER", DASHBOARD_RS, "Timer + Bewertung"),
    ("STYLE_INACTIVE", DASHBOARD_RS, "Timer vor 1. Shot, Gewicht „--“"),
    ("STYLE_WEIGHT", DASHBOARD_RS, "Gewichtsziffern"),
    ("STYLE_SHOT_GAUGE", DASHBOARD_RS, "%-Balken Füllung"),
    ("STYLE_HX_COLD", DASHBOARD_RS, "HX-Balken < 88 °C"),
    ("STYLE_HX_IDEAL", DASHBOARD_RS, "HX-Balken 88–95 °C"),
    ("STYLE_HX_WARM", DASHBOARD_RS, "HX-Balken 96–100 °C"),
    ("STYLE_HX_HOT", DASHBOARD_RS, "HX-Balken > 100 °C"),
    ("STYLE_HX_IDEAL_ZONE", DASHBOARD_RS, "HX-Idealzone (Markierung)"),
    ("STYLE_HIDDEN", DASHBOARD_RS, "Leere Balkenteile (Schwarz = unsichtbar)"),
    ("STYLE_BOILER_HEATING_LABEL", DASHBOARD_RS, "Boiler-Text beim Aufheizen"),
    ("STYLE_BOILER_HEATING_FILL", DASHBOARD_RS, "Boiler-Füllung beim Aufheizen"),
    ("STYLE_BOILER_READY", DASHBOARD_RS, "Boiler bereit (Text + Füllung)"),
    ("STYLE_BOILER_TARGET", DASHBOARD_RS, "Boiler-Sollmarke"),
    ("STYLE_NO_WATER", DASHBOARD_RS, "NO WATER"),
    ("COLOR_VERSION", CONNECTING_RS, "Start: Version"),
    ("COLOR_STATUS", CONNECTING_RS, "Start: Statustext"),
    ("COLOR_PROGRESS", CONNECTING_RS, "Start: Ladebalken"),
    ("COLOR_PROGRESS_TRACK", CONNECTING_RS, "Start: Ladebalken leer"),
]
LABELS = {key: label for key, _, label in ELEMENTS}

# Muted ("gedeckte") colors, grouped.
PALETTE = [
    ("Neutral", [
        ("Schwarz", "#000000"), ("Rußgrau", "#1A1A1A"), ("Anthrazit", "#2E2E2E"),
        ("Graphit", "#404040"), ("Schiefergrau", "#505050"), ("Steingrau", "#6B6B6B"),
        ("Mittelgrau", "#808080"), ("Silbergrau", "#A0A0A0"), ("Hellgrau", "#C4C4C4"),
        ("Elfenbein", "#F2EBDD"), ("Warmweiß", "#FFF4E0"), ("Weiß", "#FFFFFF"),
    ]),
    ("Kaffee", [
        ("Espresso", "#3B2A20"), ("Mokka", "#5C4033"), ("Haselnuss", "#8B6B4A"),
        ("Milchkaffee", "#A88B6E"), ("Karamell", "#B08055"), ("Kupfer", "#C87533"),
        ("Sand", "#D8C3A5"), ("Crema", "#E8C9A0"),
    ]),
    ("Rot / Rosé", [
        ("Weinrot", "#7A3B45"), ("Ziegel", "#9C4A3A"), ("Rosenholz", "#A0645F"),
        ("Terrakotta", "#C0654A"), ("Altrosa", "#C48B8B"), ("Koralle", "#D9776A"),
    ]),
    ("Gelb / Orange", [
        ("Ocker", "#C8963E"), ("Senf", "#C9A227"), ("Bernstein", "#D19A3A"),
        ("Honig", "#D6B25E"), ("Aprikose", "#D99A6C"), ("Safran", "#E0A458"),
    ]),
    ("Grün", [
        ("Tanne", "#3E5B4A"), ("Moos", "#5E6E45"), ("Olive", "#7A7F4A"),
        ("Gedecktes Grün", "#3C9A50"), ("Eukalyptus", "#6E9C8A"), ("Salbei", "#8FA88A"),
        ("Pistazie", "#A6B98A"),
    ]),
    ("Blau / Petrol", [
        ("Tiefsee", "#2E4A5E"), ("Petrol", "#3E7C7C"), ("Stahlblau", "#4F6D8A"),
        ("Schieferblau", "#5A6E85"), ("Jeansblau", "#5B7FA6"), ("Gedecktes Blau", "#4D7FB0"),
        ("Taubenblau", "#7A90A8"), ("Nebelblau", "#9FB3C8"),
    ]),
    ("Violett", [
        ("Pflaume", "#6E4A6B"), ("Heide", "#8A6A86"), ("Mauve", "#9C7A97"),
        ("Lavendel", "#A497B8"),
    ]),
]

CONST_RE = r"^(const {name}: (?:Style|Color) = )(.*);$"
COLOR_EXPR_RE = re.compile(
    r"Color::Rgb\(\s*0x([0-9A-Fa-f]{2}),\s*0x([0-9A-Fa-f]{2}),\s*0x([0-9A-Fa-f]{2})\s*\)"
    r"|Color::(\w+)"
)


def read_colors():
    colors = {}
    for key, path, _ in ELEMENTS:
        m = re.search(CONST_RE.format(name=key), path.read_text(), re.M)
        if not m:
            raise SystemExit(f"{key} not found in {path}")
        c = COLOR_EXPR_RE.search(m.group(2))
        if c.group(4):
            colors[key] = NAMED_COLORS[c.group(4)]
        else:
            colors[key] = "#" + "".join(c.group(i) for i in (1, 2, 3)).upper()
    return colors


def write_colors(colors):
    for path in {p for _, p, _ in ELEMENTS}:
        text = path.read_text()
        for key, p, _ in ELEMENTS:
            if p != path:
                continue
            r, g, b = (colors[key][i:i + 2].upper() for i in (1, 3, 5))
            rgb = f"Color::Rgb(0x{r}, 0x{g}, 0x{b})"
            pattern = re.compile(CONST_RE.format(name=key), re.M)
            m = pattern.search(text)
            value = rgb if m.group(1).endswith("Color = ") else f"Style::new().fg({rgb})"
            text = text[:m.start()] + m.group(1) + value + ";" + text[m.end():]
        path.write_text(text)


def rgb565(hex_color):
    """What the display actually shows: the color rounded to RGB565."""
    r, g, b = (int(hex_color[i:i + 2], 16) for i in (1, 3, 5))
    r5, g6, b5 = round(r * 31 / 255), round(g * 63 / 255), round(b * 31 / 255)
    return "#{:02X}{:02X}{:02X}".format(
        round(r5 * 255 / 31), round(g6 * 255 / 63), round(b5 * 255 / 31))


def text_color_on(hex_color):
    r, g, b = (int(hex_color[i:i + 2], 16) for i in (1, 3, 5))
    return "#000000" if 0.299 * r + 0.587 * g + 0.114 * b > 140 else "#FFFFFF"


class Preview(tk.Canvas):
    """Mock of the firmware screens, every colored item tagged `el:<CONST>`."""

    def __init__(self, master, app):
        super().__init__(master, width=DISPLAY_W, height=DISPLAY_H, bg=BACKGROUND,
                         highlightthickness=0)
        self.app = app

    def color(self, key):
        return rgb565(self.app.colors[key])

    # -- cell helpers ------------------------------------------------------------------
    def chars(self, col, row, text, key, color=None):
        """TUI text: one character per 7x14 cell, like Mousefood."""
        for i, ch in enumerate(text):
            x, y = (col + i + 0.5) * CELL_W, (row + 0.5) * CELL_H
            self.create_text(x, y, text=ch, fill=color or self.color(key),
                             font=(FONT, -22), tags=(f"el:{key}",))

    def centered(self, col0, width, row, text, key, color=None):
        self.chars(col0 + (width - len(text)) // 2, row, text, key, color)

    def box(self, col, row, w, h, title=None, title_left=False, bottom=None,
            bottom_key="STYLE_TITLE"):
        x0, y0 = (col + 0.5) * CELL_W, (row + 0.5) * CELL_H
        x1, y1 = (col + w - 0.5) * CELL_W, (row + h - 0.5) * CELL_H
        self.create_rectangle(x0, y0, x1, y1, outline=self.color("STYLE_BORDER"), width=2,
                              tags=("el:STYLE_BORDER",))
        for text, line_row, left, key in (
            (title, row, title_left, "STYLE_TITLE"),
            (bottom, row + h - 1, False, bottom_key),
        ):
            if not text:
                continue
            start = col + 1 if left else col + (w - len(text)) // 2
            self.create_rectangle(start * CELL_W, line_row * CELL_H + 6,
                                  (start + len(text)) * CELL_W, (line_row + 1) * CELL_H - 6,
                                  fill=BACKGROUND, outline="")
            self.chars(start, line_row, text, key)

    def cell_block(self, col, row, w, key, hatched=False):
        self.create_rectangle(col * CELL_W, row * CELL_H, (col + w) * CELL_W,
                              (row + 1) * CELL_H, fill=self.color(key), outline="",
                              stipple="gray50" if hatched else "", tags=(f"el:{key}",))

    def smooth(self, x, y, text, key, px):
        """Anti-aliased firmware digits (`px` = digit height on the real display)."""
        self.create_text(x * SCALE, y * SCALE, text=text, fill=self.color(key),
                         font=(FONT, -round(px / 0.73 * SCALE), "bold"), tags=(f"el:{key}",))

    # -- screens -----------------------------------------------------------------------
    def draw(self):
        self.delete("all")
        self.create_rectangle(0, 0, DISPLAY_W, DISPLAY_H, fill=BACKGROUND, outline="")
        if self.app.screen.get() == "start":
            self.draw_start()
        else:
            self.draw_dashboard()

    def draw_dashboard(self):
        s = self.app.state
        hx, shot_ran = s["hx"].get(), s["shot_ran"].get()
        weight = s["weight"].get() if s["has_weight"].get() else None

        self.centered(0, 23, 0, "═══ COFFEE MODE ═══", "STYLE_HEADER")
        self.centered(23, 22, 0, "Cups brewed: 169", "STYLE_HEADER")

        # Info column: HX label + value, HX gauge, HEAT/PUMP.
        self.box(0, 1, 9, 13)
        self.centered(1, 7, 2, "HX", "STYLE_INDICATOR")
        self.smooth(24, 49, f"{hx}", "COLOR_HX_VALUE", 12)
        self.create_text(47 * SCALE, 45 * SCALE, text="°C", fill=self.color("COLOR_HX_VALUE"),
                         font=(FONT, -9 * SCALE, "bold"), tags=("el:COLOR_HX_VALUE",))
        total = 5
        filled = round(max(0, min(1, (hx - 60) / 50)) * total)
        fill_key = ("STYLE_HX_COLD" if hx <= 87 else "STYLE_HX_IDEAL" if hx <= 95
                    else "STYLE_HX_WARM" if hx <= 100 else "STYLE_HX_HOT")
        for i in range(total):
            if i >= total - filled:
                self.cell_block(2, 4 + i, 5, fill_key)
            elif i == 1:
                self.cell_block(2, 4 + i, 5, "STYLE_HX_IDEAL_ZONE", hatched=True)
            else:
                self.cell_block(2, 4 + i, 5, "STYLE_HIDDEN", hatched=True)
        self.chars(1, 10, "● HEAT", "STYLE_INDICATOR")
        self.chars(1, 12, "○ PUMP", "STYLE_INDICATOR")

        # Timer and Weight.
        timer_key = "STYLE_TIMER" if shot_ran else "STYLE_INACTIVE"
        self.box(9, 1, 27, 7, "Timer", bottom="PERFECT" if shot_ran else None,
                 bottom_key=timer_key)
        self.smooth(157, 74, "27" if shot_ran else "0", timer_key, 43)
        self.box(9, 8, 27, 6, "Weight")
        if weight is None:
            self.smooth(157, 151, "--", "STYLE_INACTIVE", 43)
        else:
            self.smooth(157 + 8, 151, f"{weight:.1f}".replace(".", ","), "STYLE_WEIGHT", 43)

        # Shot gauge.
        self.box(36, 1, 9, 13, bottom="%")
        if shot_ran:
            rows = 11
            fill = round(min(1, max(0, (weight or 0) / 40)) * rows)
            for i in range(rows):
                if i >= rows - fill:
                    self.cell_block(37, 2 + i, 7, "STYLE_SHOT_GAUGE")
                else:
                    self.cell_block(37, 2 + i, 7, "STYLE_HIDDEN", hatched=True)

        # Boiler.
        self.box(0, 14, 45, 3, "Boiler", title_left=True)
        if s["no_water"].get():
            self.centered(1, 43, 15, "NO WATER", "STYLE_NO_WATER")
            return
        target, now = 120, (120 if s["boiler_ready"].get() else 98)
        ready = now / target >= 0.95
        self.chars(1, 15, f"{now:3}°/{target:3}° ",
                   "STYLE_BOILER_READY" if ready else "STYLE_BOILER_HEATING_LABEL")
        bar_w = 33
        pos = lambda t: round(t / 160 * bar_w)  # noqa: E731
        current, target_pos, ready_pos = min(pos(now), bar_w), pos(target), pos(target * 0.9)
        for i in range(bar_w):
            if i < current:
                key = "STYLE_BOILER_READY" if i >= ready_pos else "STYLE_BOILER_HEATING_FILL"
                self.cell_block(11 + i, 15, 1, key)
            elif i == target_pos:
                self.chars(11 + i, 15, "│", "STYLE_BOILER_TARGET")
            else:
                self.chars(11 + i, 15, "─", "STYLE_HIDDEN")

    def draw_start(self):
        # The boot animation's scene (a still from the Startvideo page, once rendered).
        video = getattr(self.app, "video_page", None)
        if video and not video.frames:
            video.load()
        if video and video.frames:
            self.create_image(0, VideoPage.TOP_PX * SCALE, anchor="nw",
                              image=video.frames[min(45, len(video.frames) - 1)])
        else:
            self.create_rectangle(0, 14 * SCALE, DISPLAY_W, 182 * SCALE, fill="#303030",
                                  outline="")
            self.create_text(DISPLAY_W // 2, 98 * SCALE, fill="#909090", font=(FONT, 12),
                             text="Animation: Seite Startvideo öffnen")
        self.chars(45 - 1 - len(read_version()), 0, "v" + read_version(), "COLOR_VERSION")
        self.centered(1, 43, 14, "connecting wifi...", "COLOR_STATUS")
        split = 1 + round(43 * 0.55)
        self.create_rectangle(CELL_W, 15 * CELL_H, split * CELL_W, 16 * CELL_H,
                              fill=self.color("COLOR_PROGRESS"), outline="",
                              tags=("el:COLOR_PROGRESS",))
        self.create_rectangle(split * CELL_W, 15 * CELL_H, 44 * CELL_W, 16 * CELL_H,
                              fill=self.color("COLOR_PROGRESS_TRACK"), outline="",
                              tags=("el:COLOR_PROGRESS_TRACK",))

    def element_at(self, x, y):
        """Topmost element under (x, y), with a few pixels of slack for thin lines."""
        for item in reversed(self.find_overlapping(x - 4, y - 4, x + 4, y + 4)):
            for tag in self.gettags(item):
                if tag.startswith("el:"):
                    return tag[3:]
        return None


class VideoPage(tk.Frame):
    """The boot animation, rendered by the firmware's own scene code and played in a loop."""

    TOP_PX = 14  # display y of the scene, below the version line (`TOP_PX` in boot_animation.rs)

    def __init__(self, master, app):
        super().__init__(master, bg="#202020")
        self.app = app
        self.frames = []
        self.interval = 66
        self.index = 0
        self.playing = True
        self.rendering = False
        self.tick_job = None

        self.canvas = tk.Canvas(self, width=DISPLAY_W, height=DISPLAY_H, bg=BACKGROUND,
                                highlightthickness=0)
        self.canvas.pack(pady=(6, 6))

        bar = tk.Frame(self, bg="#202020")
        bar.pack(fill="x")
        button = dict(bg="#303030", fg="#DDDDDD", activebackground="#404040",
                      activeforeground="#FFFFFF", relief="flat", padx=10, pady=3)
        self.play_button = tk.Button(bar, text="⏸ Pause", command=self.toggle, **button)
        self.play_button.pack(side="left")
        tk.Button(bar, text="⟳ Neu rendern", command=self.render, **button).pack(
            side="left", padx=6)
        self.time = tk.Scale(bar, from_=0, to=0, orient="horizontal", length=330,
                             showvalue=False, bg="#202020", troughcolor="#303030",
                             highlightthickness=0, command=self.seek)
        self.time.pack(side="left", padx=6)
        self.clock = tk.Label(bar, text="", width=12, bg="#202020", fg="#DDDDDD",
                              font=(FONT, 9), anchor="w")
        self.clock.pack(side="left")
        self.status = tk.Label(self, text="", bg="#202020", fg="#AAAAAA", anchor="w",
                               justify="left")
        self.status.pack(fill="x", pady=(6, 0))

    def show(self):
        if not self.frames and not self.load() and not self.rendering:
            self.render()
        self.schedule()

    def hide(self):
        if self.tick_job:
            self.after_cancel(self.tick_job)
            self.tick_job = None

    def render(self):
        """Build and run the frame dump in the background, then load the new frames."""
        if self.rendering:
            return
        self.rendering = True
        self.status.config(text="Rendere mit dem Firmware-Code (cargo run --example "
                                "boot_frames) …")

        def work():
            try:
                result = subprocess.run(BOOT_FRAMES_CMD, cwd=REPO, capture_output=True,
                                        text=True)
                error = result.stderr[-1500:] if result.returncode else None
            except OSError as e:
                error = str(e)
            self.after(0, lambda: self.rendered(error))

        threading.Thread(target=work, daemon=True).start()

    def rendered(self, error):
        self.rendering = False
        if error:
            self.status.config(text="Rendern fehlgeschlagen:\n" + error)
        elif self.load():
            self.status.config(text=f"{len(self.frames)} Bilder à {self.interval} ms, "
                                    "gerendert aus src/screens/boot_animation.rs")
            if self.winfo_ismapped():
                self.schedule()

    def load(self):
        try:
            data = BOOT_FRAMES.read_bytes()
            newline = data.index(b"\n")
            width, height, count, self.interval = map(int, data[:newline].split())
        except (OSError, ValueError):
            return False
        body = data[newline + 1:]
        size = width * height * 3
        header = b"P6 %d %d 255\n" % (width, height)
        self.frames = [
            tk.PhotoImage(data=header + body[i * size:(i + 1) * size], format="PPM")
            .zoom(2 * SCALE)
            for i in range(count)
        ]
        self.index = min(self.index, len(self.frames) - 1)
        self.time.config(to=len(self.frames) - 1)
        if not self.status.cget("text"):
            self.status.config(text=f"{len(self.frames)} Bilder geladen. Nach Änderungen an "
                                    "der Animation: Neu rendern.")
        self.draw()
        return True

    def toggle(self):
        self.playing = not self.playing
        self.play_button.config(text="⏸ Pause" if self.playing else "▶ Abspielen")
        self.schedule()

    def seek(self, value):
        if self.frames and int(value) != self.index:
            self.index = int(value)
            self.draw()

    def schedule(self):
        if self.tick_job:
            self.after_cancel(self.tick_job)
            self.tick_job = None
        if self.playing and self.frames:
            self.tick_job = self.after(self.interval, self.tick)

    def tick(self):
        self.tick_job = None
        if self.frames and self.playing:
            self.index = (self.index + 1) % len(self.frames)
            self.time.set(self.index)
            self.draw()
        self.schedule()

    def draw(self):
        """The start screen around the scene: version line on top, loading block below."""
        c = self.canvas
        c.delete("all")
        color = self.app.colors.get
        c.create_text(DISPLAY_W - 4, CELL_H // 2, text="v" + read_version(), anchor="e",
                      fill=color("COLOR_VERSION", "#FFB000"), font=(FONT, 13))
        if self.frames:
            c.create_image(0, self.TOP_PX * SCALE, image=self.frames[self.index], anchor="nw")
            seconds = self.index * self.interval / 1000
            self.clock.config(text=f"{seconds:5.2f} s")
        else:
            c.create_text(DISPLAY_W // 2, 98 * SCALE, text="noch nicht gerendert",
                          fill="#808080", font=(FONT, 12))
        c.create_text(DISPLAY_W // 2, 14 * CELL_H + CELL_H // 2, text="connecting wifi...",
                      fill=color("COLOR_STATUS", "#C87533"), font=(FONT, 13))
        split = CELL_W + round(43 * CELL_W * 0.2)
        c.create_rectangle(CELL_W, 15 * CELL_H, split, 16 * CELL_H,
                           fill=color("COLOR_PROGRESS", "#C87533"), outline="")
        c.create_rectangle(split, 15 * CELL_H, 44 * CELL_W, 16 * CELL_H,
                           fill=color("COLOR_PROGRESS_TRACK", "#7F7F7F"), outline="")


def read_version():
    match = re.search(r'^version\s*=\s*"([^"]+)"', (REPO / "Cargo.toml").read_text(), re.M)
    return match.group(1) if match else "?"


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("MaraTUI Farben")
        self.configure(bg="#202020")
        self.colors = read_colors()
        self.saved = dict(self.colors)
        self.undo_stack = []
        self.drag = None
        self.drop_rows = {}  # widget path -> element key
        self.list_swatches = {}  # element key -> swatch label

        self.screen = tk.StringVar(value="dashboard")
        self.state = {
            "hx": tk.IntVar(value=93),
            "shot_ran": tk.BooleanVar(value=True),
            "has_weight": tk.BooleanVar(value=True),
            "weight": tk.DoubleVar(value=36.4),
            "boiler_ready": tk.BooleanVar(value=True),
            "no_water": tk.BooleanVar(value=False),
        }

        left = tk.Frame(self, bg="#202020")
        left.pack(side="left", fill="both", padx=10, pady=10)
        right = tk.Frame(self, bg="#202020")
        right.pack(side="left", fill="both", expand=True, padx=(0, 10), pady=10)

        self.build_controls(left)
        self.editor_page = tk.Frame(left, bg="#202020")
        self.editor_page.pack(fill="both")
        self.preview = Preview(self.editor_page, self)
        self.preview.pack(pady=(6, 6))
        self.preview.bind("<Motion>", self.on_preview_hover)
        self.build_element_list(self.editor_page)
        self.video_page = VideoPage(left, self)
        self.build_palette(right)
        self.build_actions(right)

        for var in self.state.values():
            var.trace_add("write", lambda *_: self.redraw())
        self.screen.trace_add("write", lambda *_: self.switch_page())
        self.bind_all("<B1-Motion>", self.on_drag_motion)
        self.bind_all("<ButtonRelease-1>", self.on_drop)
        self.bind_all("<Control-z>", lambda _: self.undo())
        self.protocol("WM_DELETE_WINDOW", self.on_close)
        self.redraw()

    # -- layout ------------------------------------------------------------------------
    def label(self, master, text, **kw):
        return tk.Label(master, text=text, bg="#202020", fg="#DDDDDD", **kw)

    def build_controls(self, master):
        bar = tk.Frame(master, bg="#202020")
        bar.pack(fill="x")
        for value, text in (("dashboard", "Dashboard"), ("start", "Startbildschirm"),
                            ("video", "Startvideo")):
            tk.Radiobutton(bar, text=text, variable=self.screen, value=value,
                           indicatoron=False, padx=10, pady=3, bg="#303030", fg="#DDDDDD",
                           selectcolor="#505050", activebackground="#404040",
                           activeforeground="#FFFFFF").pack(side="left", padx=(0, 4))

        states = self.states_bar = tk.Frame(master, bg="#202020")
        states.pack(fill="x", pady=(6, 0))
        self.label(states, "Zustand:").pack(side="left")
        for key, text in (("shot_ran", "Shot gelaufen"), ("has_weight", "Waage hat Wert"),
                          ("boiler_ready", "Boiler bereit"), ("no_water", "No Water")):
            tk.Checkbutton(states, text=text, variable=self.state[key], bg="#202020",
                           fg="#DDDDDD", selectcolor="#303030", activebackground="#202020",
                           activeforeground="#FFFFFF").pack(side="left")

        sliders = self.sliders_bar = tk.Frame(master, bg="#202020")
        sliders.pack(fill="x")
        for key, text, lo, hi, res in (("hx", "HX °C", 60, 110, 1),
                                       ("weight", "Gewicht g", -5, 45, 0.1)):
            self.label(sliders, text).pack(side="left", padx=(0, 4))
            tk.Scale(sliders, variable=self.state[key], from_=lo, to=hi, resolution=res,
                     orient="horizontal", length=200, bg="#202020", fg="#DDDDDD",
                     troughcolor="#303030", highlightthickness=0).pack(side="left", padx=(0, 12))

    def build_element_list(self, master):
        self.label(master, "Elemente (auch hier kannst du Farben ablegen):",
                   anchor="w").pack(fill="x")
        grid = tk.Frame(master, bg="#202020")
        grid.pack(fill="x")
        per_column = (len(ELEMENTS) + 1) // 2
        for i, (key, _, text) in enumerate(ELEMENTS):
            row = tk.Frame(grid, bg="#202020")
            row.grid(row=i % per_column, column=i // per_column, sticky="w", padx=(0, 16))
            swatch = tk.Label(row, width=8, font=(FONT, 8), relief="solid", bd=1)
            swatch.pack(side="left")
            name = self.label(row, text, font=(FONT, 9), anchor="w")
            name.pack(side="left", padx=4)
            for widget in (row, swatch, name):
                self.drop_rows[str(widget)] = key
            self.list_swatches[key] = swatch

    def build_palette(self, master):
        self.label(master, "Gedeckte Farben – auf ein Element ziehen",
                   font=(FONT, 11, "bold"), anchor="w").pack(fill="x")
        for group, colors in PALETTE:
            self.label(master, group, anchor="w", font=(FONT, 9)).pack(fill="x", pady=(8, 2))
            row = tk.Frame(master, bg="#202020")
            row.pack(fill="x")
            for i, (name, hex_color) in enumerate(colors):
                self.make_swatch(row, name, hex_color).grid(row=i // 4, column=i % 4,
                                                            padx=2, pady=2)
        custom = tk.Button(master, text="Eigene Farbe wählen …", command=self.pick_custom,
                           bg="#303030", fg="#DDDDDD", activebackground="#404040")
        custom.pack(anchor="w", pady=(10, 2))
        self.custom_row = tk.Frame(master, bg="#202020")
        self.custom_row.pack(fill="x")

    def make_swatch(self, master, name, hex_color):
        swatch = tk.Label(master, text=f"{name}\n{hex_color}", width=13, height=2,
                          bg=hex_color, fg=text_color_on(hex_color), font=(FONT, 8),
                          relief="raised", bd=1, cursor="hand2")
        swatch.bind("<ButtonPress-1>", lambda e, c=hex_color, n=name: self.start_drag(e, c, n))
        return swatch

    def build_actions(self, master):
        self.status = self.label(master, "", anchor="w", justify="left", wraplength=420)
        self.status.pack(side="bottom", fill="x", pady=(8, 0))
        bar = tk.Frame(master, bg="#202020")
        bar.pack(side="bottom", fill="x")
        for text, command in (("Speichern", self.save), ("Rückgängig", self.undo),
                              ("Verwerfen", self.revert)):
            tk.Button(bar, text=text, command=command, bg="#303030", fg="#DDDDDD",
                      activebackground="#404040", padx=10).pack(side="left", padx=(0, 6))

    # -- drag and drop -----------------------------------------------------------------
    def start_drag(self, event, hex_color, name):
        ghost = tk.Toplevel(self)
        ghost.overrideredirect(True)
        ghost.attributes("-topmost", True)
        tk.Label(ghost, bg=hex_color, width=4, height=2, relief="solid", bd=1).pack()
        ghost.geometry(f"+{event.x_root + 8}+{event.y_root + 8}")
        self.drag = (hex_color, name, ghost)

    def target_at(self, x_root, y_root):
        widget = self.winfo_containing(x_root, y_root)
        if widget is self.preview:
            return self.preview.element_at(x_root - self.preview.winfo_rootx(),
                                           y_root - self.preview.winfo_rooty())
        return self.drop_rows.get(str(widget)) if widget else None

    def on_drag_motion(self, event):
        if not self.drag:
            return
        hex_color, name, ghost = self.drag
        ghost.geometry(f"+{event.x_root + 8}+{event.y_root + 8}")
        target = self.target_at(event.x_root, event.y_root)
        self.status.config(text=f"{name} {hex_color} → {LABELS[target]}" if target
                           else f"{name} {hex_color} – auf ein Element ziehen")

    def on_drop(self, event):
        if not self.drag:
            return
        hex_color, name, ghost = self.drag
        ghost.destroy()
        self.drag = None
        target = self.target_at(event.x_root, event.y_root)
        if target:
            self.set_color(target, hex_color, f"{LABELS[target]} = {name} {hex_color}")
        else:
            self.status.config(text="Nicht auf einem Element abgelegt.")

    def on_preview_hover(self, event):
        if self.drag:
            return
        key = self.preview.element_at(event.x, event.y)
        if key:
            self.status.config(text=f"{LABELS[key]}: {self.colors[key]}  ({key})")

    # -- color state -------------------------------------------------------------------
    def set_color(self, key, hex_color, message):
        if self.colors[key] == hex_color:
            return
        self.undo_stack.append((key, self.colors[key]))
        self.colors[key] = hex_color
        self.status.config(text=message)
        self.redraw()

    def pick_custom(self):
        result = colorchooser.askcolor(parent=self, title="Eigene Farbe")
        if result and result[1]:
            hex_color = result[1].upper()
            self.make_swatch(self.custom_row, "Eigene", hex_color).pack(side="left", padx=2)

    def undo(self):
        if self.undo_stack:
            key, previous = self.undo_stack.pop()
            self.colors[key] = previous
            self.status.config(text=f"Rückgängig: {LABELS[key]} = {previous}")
            self.redraw()

    def revert(self):
        self.colors = read_colors()
        self.saved = dict(self.colors)
        self.undo_stack.clear()
        self.status.config(text="Farben aus dem Code neu geladen.")
        self.redraw()

    def save(self):
        write_colors(self.colors)
        self.saved = dict(self.colors)
        self.status.config(text="Gespeichert in dashboard.rs / connecting.rs. Zum Übernehmen "
                                "neu bauen und flashen: cargo build --release, dann espflash.")
        self.redraw()

    def switch_page(self):
        """The color editor for Dashboard / start screen, or the boot animation player."""
        video = self.screen.get() == "video"
        self.states_bar.pack_forget()
        self.sliders_bar.pack_forget()
        if video:
            self.editor_page.pack_forget()
            self.video_page.pack(fill="both")
            self.video_page.show()
        else:
            self.video_page.hide()
            self.video_page.pack_forget()
            self.states_bar.pack(fill="x", pady=(6, 0))
            self.sliders_bar.pack(fill="x")
            self.editor_page.pack(fill="both")
            self.redraw()

    def redraw(self):
        if self.screen.get() == "video":
            self.video_page.draw()
        else:
            self.preview.draw()
        for key, _, _ in ELEMENTS:
            hex_color = self.colors[key]
            self.list_swatches[key].config(text=hex_color, bg=hex_color,
                                       fg=text_color_on(hex_color))
        dirty = self.colors != self.saved
        self.title("MaraTUI Farben" + (" – ungespeichert" if dirty else ""))

    def on_close(self):
        if self.colors != self.saved and not messagebox.askyesno(
                "Ungespeicherte Änderungen", "Ohne Speichern schließen?", parent=self):
            return
        self.destroy()


if __name__ == "__main__":
    App().mainloop()
