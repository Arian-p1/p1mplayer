//! Derive a cohesive dark UI theme from album cover art.
//!
//! The goal is to *tint* the existing Gruvbox-style dark look toward the cover
//! rather than reproduce it literally: backgrounds stay dark and low-saturation
//! (so text remains readable), while the accent is pulled from the cover's most
//! vivid hue. Cover-less tracks fall back to the base Gruvbox palette.

use std::path::Path;

/// A full set of UI colours, mirroring the fields of the Slint `Theme` global.
/// Each colour is plain `(r, g, b)`; alpha is always opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    pub bg: (u8, u8, u8),
    pub bg_alt: (u8, u8, u8),
    pub sidebar: (u8, u8, u8),
    pub card: (u8, u8, u8),
    pub accent: (u8, u8, u8),
    pub accent_soft: (u8, u8, u8),
    pub text: (u8, u8, u8),
    pub text_dim: (u8, u8, u8),
    pub hover: (u8, u8, u8),
}

impl ThemeColors {
    /// The default theme: Gruvbox dark. Used as the baseline and as the fallback
    /// whenever a cover can't be read or has no usable colour.
    pub const fn gruvbox() -> Self {
        ThemeColors {
            bg: (0x28, 0x28, 0x28),
            bg_alt: (0x32, 0x30, 0x2f),
            sidebar: (0x1d, 0x20, 0x21),
            card: (0x3c, 0x38, 0x36),
            accent: (0xfe, 0x80, 0x19),
            accent_soft: (0x5e, 0x3e, 0x24),
            text: (0xeb, 0xdb, 0xb2),
            text_dim: (0x92, 0x83, 0x74),
            hover: (0x50, 0x49, 0x45),
        }
    }
}

/// Build a theme from a cover image on disk. Returns `None` if the image can't be
/// decoded or contains no pixels; callers should fall back to `ThemeColors::gruvbox()`.
pub fn from_cover(path: &Path) -> Option<ThemeColors> {
    // Covers are already small cached thumbnails, so this is cheap.
    let img = image::open(path).ok()?.to_rgb8();
    if img.width() == 0 || img.height() == 0 {
        return None;
    }

    // Hue histogram (15° per bucket) for picking the accent, plus a running
    // average for the overall tint.
    const BUCKETS: usize = 24;
    let mut score = [0f32; BUCKETS];
    let mut rsum = [0f32; BUCKETS];
    let mut gsum = [0f32; BUCKETS];
    let mut bsum = [0f32; BUCKETS];
    let mut wsum = [0f32; BUCKETS];

    let (mut sr, mut sg, mut sb, mut n) = (0f64, 0f64, 0f64, 0f64);

    for px in img.pixels() {
        let [r, g, b] = px.0;
        sr += r as f64;
        sg += g as f64;
        sb += b as f64;
        n += 1.0;

        let (h, s, v) = rgb_to_hsv(r, g, b);
        // Only reasonably saturated, mid-bright pixels are accent candidates.
        if s > 0.30 && v > 0.25 && v < 0.98 {
            let idx = ((h / 360.0 * BUCKETS as f32) as usize).min(BUCKETS - 1);
            let w = s * v; // weight vivid colours more
            score[idx] += w;
            rsum[idx] += r as f32 * w;
            gsum[idx] += g as f32 * w;
            bsum[idx] += b as f32 * w;
            wsum[idx] += w;
        }
    }

    if n == 0.0 {
        return None;
    }

    // Overall average → the hue/saturation the dark backgrounds get tinted with.
    let avg = ((sr / n) as u8, (sg / n) as u8, (sb / n) as u8);
    let (base_h, base_s, _) = rgb_to_hsv(avg.0, avg.1, avg.2);

    // Accent = weighted-average colour of the most vivid hue bucket, intensified
    // so it stands out against the dark UI. Grayscale covers fall back to a light
    // tint of the base hue.
    let best = (0..BUCKETS)
        .filter(|&i| wsum[i] > 0.0)
        .max_by(|&a, &b| score[a].partial_cmp(&score[b]).unwrap());
    let accent = match best {
        Some(i) => {
            let (h, s, v) = rgb_to_hsv(
                (rsum[i] / wsum[i]) as u8,
                (gsum[i] / wsum[i]) as u8,
                (bsum[i] / wsum[i]) as u8,
            );
            hsv_to_rgb(h, s.max(0.55), v.max(0.80))
        }
        None => hsv_to_rgb(base_h, 0.12, 0.85),
    };
    let (acc_h, acc_s, _) = rgb_to_hsv(accent.0, accent.1, accent.2);

    // Dark, low-saturation tint of the base hue keeps depth + readability.
    let bg_s = (base_s * 0.5).clamp(0.0, 0.28);
    let bg = hsv_to_rgb(base_h, bg_s, 0.16);
    let bg_alt = hsv_to_rgb(base_h, bg_s, 0.19);
    let sidebar = hsv_to_rgb(base_h, bg_s, 0.11);
    let card = hsv_to_rgb(base_h, bg_s, 0.25);
    let hover = hsv_to_rgb(base_h, bg_s, 0.32);

    // Muted, dark version of the accent for selected/idle button backgrounds.
    let accent_soft = hsv_to_rgb(acc_h, (acc_s * 0.9).min(0.85), 0.26);

    // Warm near-white text tinted by the base hue; dim is a mid tone.
    let text = hsv_to_rgb(base_h, 0.08, 0.95);
    let text_dim = hsv_to_rgb(base_h, 0.12, 0.62);

    Some(ThemeColors {
        bg,
        bg_alt,
        sidebar,
        card,
        accent,
        accent_soft,
        text,
        text_dim,
        hover,
    })
}

/// RGB (0–255) → HSV with hue in degrees [0,360), s and v in [0,1].
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let d = max - min;

    let h = if d == 0.0 {
        0.0
    } else if max == rf {
        60.0 * (((gf - bf) / d).rem_euclid(6.0))
    } else if max == gf {
        60.0 * (((bf - rf) / d) + 2.0)
    } else {
        60.0 * (((rf - gf) / d) + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

/// HSV (hue degrees, s/v in [0,1]) → RGB (0–255).
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let c = v * s;
    let h2 = h / 60.0;
    let x = c * (1.0 - (h2.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h2 as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to_u8 = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r), to_u8(g), to_u8(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: u8, b: u8) -> bool {
        (a as i16 - b as i16).abs() <= 1
    }

    #[test]
    fn hsv_round_trips_primary_colors() {
        for &(r, g, b) in &[
            (255u8, 0u8, 0u8),
            (0, 255, 0),
            (0, 0, 255),
            (128, 64, 32),
            (10, 200, 150),
            (0, 0, 0),
            (255, 255, 255),
        ] {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let (r2, g2, b2) = hsv_to_rgb(h, s, v);
            assert!(
                close(r, r2) && close(g, g2) && close(b, b2),
                "{:?} -> {:?}",
                (r, g, b),
                (r2, g2, b2)
            );
        }
    }

    #[test]
    fn gruvbox_is_dark_and_distinct() {
        let g = ThemeColors::gruvbox();
        // Backgrounds darker than text.
        assert!(g.bg.0 < g.text.0);
        assert_ne!(g.accent, g.bg);
    }
}
