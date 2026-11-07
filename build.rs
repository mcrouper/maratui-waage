use std::collections::HashMap;

fn main() {
    let dotenv_vars = export_dotenv_to_rustc_env();
    render_smooth_fonts();

    let target = std::env::var("TARGET").unwrap_or_default();
    let is_esp32 = target.contains("xtensa") || target.contains("espidf");
    let has_simulator = std::env::var("CARGO_FEATURE_SIMULATOR").is_ok();

    if has_simulator && is_esp32 {
        panic!(
            "Simulator is for host only. Use: cargo sim --target <host>\n\
             Example: cargo sim --target x86_64-unknown-linux-gnu"
        );
    }

    if !has_simulator {
        validate_device_env(&dotenv_vars);
    }

    #[cfg(feature = "device")]
    {
        embuild::espidf::sysenv::output();
    }
}

// Require SSID (non-empty) and PASSWORD key presence (may be empty for open networks).
// This mirrors the runtime check in config.rs but fails at compile time instead.
fn validate_device_env(dotenv_vars: &HashMap<String, String>) {
    let from_process = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    let from_dotenv = |key: &str| dotenv_vars.get(key).cloned().filter(|v| !v.is_empty());

    let ssid_present = from_process("MARATUI_WIFI_SSID")
        .or_else(|| from_dotenv("MARATUI_WIFI_SSID"))
        .is_some();

    // PASSWORD may be empty (open network) — we only require the key to exist.
    let password_present = std::env::var("MARATUI_WIFI_PASSWORD").is_ok()
        || dotenv_vars.contains_key("MARATUI_WIFI_PASSWORD");

    if !ssid_present || !password_present {
        let mut missing = Vec::new();
        if !ssid_present {
            missing.push("MARATUI_WIFI_SSID");
        }
        if !password_present {
            missing.push("MARATUI_WIFI_PASSWORD");
        }
        panic!(
            "\n\nMissing required Wi-Fi settings for device build: {}\n\
             Create a .env file in the project root:\n\n\
             MARATUI_WIFI_SSID=<your-network>\n\
             MARATUI_WIFI_PASSWORD=<your-password>\n",
            missing.join(", ")
        );
    }
}

fn export_dotenv_to_rustc_env() -> HashMap<String, String> {
    let dotenv_path = std::path::Path::new(".env");
    println!("cargo:rerun-if-changed={}", dotenv_path.display());

    let mut vars = HashMap::new();

    let Ok(content) = std::fs::read_to_string(dotenv_path) else {
        return vars;
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if !key.starts_with("MARATUI_") {
            continue;
        }

        let mut value = value.trim().to_string();
        if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            value = value[1..value.len() - 1].to_string();
        }

        println!("cargo:rustc-env={}={}", key, value);
        vars.insert(key.to_string(), value);
    }

    vars
}

const SMOOTH_FONT_PATH: &str = "assets/fonts/JetBrainsMono-Bold.ttf";

/// One pre-rendered glyph set for `src/smooth_digits.rs`. Every glyph shares the same box
/// `height`; `unit_chars` are rasterized at the smaller `unit_px` and top-aligned with the
/// digits (e.g. a raised "°C" next to the number).
struct GlyphSetSpec {
    name: &'static str,
    /// Font size of the digits in pixels (em size, not digit height — JetBrains Mono digits are
    /// ~0.73 em tall).
    px: f32,
    /// Height of every glyph box in pixels; digits are vertically centered in it.
    height: usize,
    /// Full-cell characters. The font is monospaced, so digits are tabular and values never
    /// shift sideways.
    chars: &'static str,
    /// Characters that get a tight box instead of a full digit cell (decimal comma).
    narrow_chars: &'static str,
    unit_px: f32,
    unit_chars: &'static str,
}

const GLYPH_SETS: &[GlyphSetSpec] = &[
    // Dashboard Timer / Weight readouts: 175x56 px fields.
    GlyphSetSpec {
        name: "LARGE_DIGITS",
        px: 56.0,
        height: 56,
        chars: "0123456789-",
        narrow_chars: ",",
        unit_px: 0.0,
        unit_chars: "",
    },
    // HX temperature in the Dashboard's info column: a single 49x14 px row.
    GlyphSetSpec {
        name: "SMALL_DIGITS",
        px: 17.0,
        height: 14,
        chars: "0123456789-",
        narrow_chars: "",
        unit_px: 12.0,
        unit_chars: "°C",
    },
    // Remaining tank water in the Dashboard banner, e.g. "1,85L": a single 42x14 px row.
    GlyphSetSpec {
        name: "WATER_DIGITS",
        px: 14.0,
        height: 14,
        chars: "0123456789",
        narrow_chars: ",",
        unit_px: 12.0,
        unit_chars: "L",
    },
];

/// Rasterize the glyph sets in `GLYPH_SETS` to 4-bit coverage bitmaps (two pixels per byte,
/// high nibble first) and write them to `$OUT_DIR/smooth_fonts.rs`, which
/// `src/smooth_digits.rs` `include!`s. Doing this at build time keeps the firmware free of a
/// TTF rasterizer and needs no heap at runtime.
fn render_smooth_fonts() {
    use std::fmt::Write as _;

    println!("cargo:rerun-if-changed={SMOOTH_FONT_PATH}");
    let data = std::fs::read(SMOOTH_FONT_PATH)
        .unwrap_or_else(|e| panic!("Failed to read {SMOOTH_FONT_PATH}: {e}"));
    let font = fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
        .unwrap_or_else(|e| panic!("Failed to parse {SMOOTH_FONT_PATH}: {e}"));

    let mut out = format!("// @generated by build.rs from {SMOOTH_FONT_PATH} — do not edit.\n");
    for spec in GLYPH_SETS {
        // Baseline that vertically centers the set's ink (digits plus the comma's descender).
        let (ascent, descent) = spec
            .chars
            .chars()
            .chain(spec.narrow_chars.chars())
            .map(|ch| font.metrics(ch, spec.px))
            .fold((0, 0), |(a, d), m| (a.max(m.ymin + m.height as i32), d.min(m.ymin)));
        let baseline = (spec.height as i32 - (ascent - descent)) / 2 + ascent;
        let zero = font.metrics('0', spec.px);
        let digit_top = baseline - (zero.ymin + zero.height as i32);

        // Units sit on their own baseline so their top lines up with the digits' top.
        let unit_baseline = if spec.unit_chars.is_empty() {
            baseline
        } else {
            let (c, _) = font.rasterize('C', spec.unit_px);
            digit_top + c.ymin + c.height as i32
        };

        let glyphs = spec
            .chars
            .chars()
            .map(|ch| (ch, spec.px, baseline, false))
            .chain(spec.narrow_chars.chars().map(|ch| (ch, spec.px, baseline, true)))
            .chain(spec.unit_chars.chars().map(|ch| (ch, spec.unit_px, unit_baseline, false)));

        writeln!(out, "pub static {}: GlyphSet = GlyphSet {{", spec.name).unwrap();
        writeln!(out, "    height: {},\n    glyphs: &[", spec.height).unwrap();
        for (ch, px, baseline, narrow) in glyphs {
            let (m, bitmap) = font.rasterize(ch, px);
            // Narrow glyphs get 1 px padding on each side; the rest a full monospace cell.
            let width = if narrow { m.width + 2 } else { m.advance_width.round() as usize };
            let left = if narrow {
                1
            } else {
                ((width as f32 - m.advance_width) / 2.0).round() as i32 + m.xmin
            };
            let top = baseline - (m.ymin + m.height as i32);
            assert!(
                left >= 0
                    && top >= 0
                    && left as usize + m.width <= width
                    && top as usize + m.height <= spec.height,
                "{}: glyph {ch:?} at {px}px does not fit a {width}x{} box — lower `px` or raise `height`",
                spec.name,
                spec.height
            );

            let mut nibbles = vec![0u8; width * spec.height];
            for gy in 0..m.height {
                for gx in 0..m.width {
                    let coverage = bitmap[gy * m.width + gx] as u32;
                    nibbles[(top as usize + gy) * width + left as usize + gx] =
                        ((coverage * 15 + 127) / 255) as u8;
                }
            }
            let packed: Vec<String> = nibbles
                .chunks(2)
                .map(|p| (p[0] << 4 | p.get(1).copied().unwrap_or(0)).to_string())
                .collect();
            writeln!(
                out,
                "        Glyph {{ ch: {ch:?}, width: {width}, coverage: &[{}] }},",
                packed.join(",")
            )
            .unwrap();
        }
        writeln!(out, "    ],\n}};").unwrap();
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    std::fs::write(std::path::Path::new(&out_dir).join("smooth_fonts.rs"), out)
        .expect("Failed to write smooth_fonts.rs");
}
