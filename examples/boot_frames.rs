//! Render one loop of the boot animation to a file, for previewing it without flashing:
//!
//!     cargo run --example boot_frames --no-default-features --features simulator \
//!         --target x86_64-unknown-linux-gnu -- frames.bin
//!
//! Format: a text header line `<width> <height> <frames> <frame interval ms>\n`, then every
//! frame as RGB888 rows (scene pixels; the display shows each one as a 2x2 block).

use std::io::Write;

use maratui::screens::boot_animation::{FRAME_INTERVAL, HEIGHT, LOOP_SECS, WIDTH, preview_frame};
use mousefood::embedded_graphics::prelude::RgbColor;

fn main() -> std::io::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "boot_frames.bin".into());
    let interval = FRAME_INTERVAL.as_secs_f32();
    let frames = (LOOP_SECS / interval).round() as usize;

    let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
    writeln!(
        out,
        "{WIDTH} {HEIGHT} {frames} {}",
        FRAME_INTERVAL.as_millis()
    )?;
    for i in 0..frames {
        for px in preview_frame(i as f32 * interval) {
            // RGB565 channels scaled up to 8 bits.
            out.write_all(&[
                (u16::from(px.r()) * 255 / 31) as u8,
                (u16::from(px.g()) * 255 / 63) as u8,
                (u16::from(px.b()) * 255 / 31) as u8,
            ])?;
        }
    }
    out.flush()?;
    eprintln!("{frames} frames written to {path}");
    Ok(())
}
