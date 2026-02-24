/// OKLab perceptual color space utilities.
///
/// Reference: Björn Ottosson (2020) — <https://bottosson.github.io/posts/oklab/>
///
/// OKLab is a perceptually uniform color space: equal Euclidean distances
/// correspond to approximately equal *perceived* color differences.  This
/// makes it superior to rec601 luma for tasks like:
///
///   1. Finding the nearest palette color to an arbitrary RGB value.
///   2. Computing contrast ratios between foreground / background pairs.
///
/// All functions operate on `f32` and require no external dependencies.

// ---------------------------------------------------------------------------
// Forward transform: sRGB → OKLab
// ---------------------------------------------------------------------------

/// Convert sRGB (each channel in 0.0–1.0) to OKLab (L, a, b).
///
/// - `L` ∈ [0, 1]: perceptual lightness (0 = black, 1 = white).
/// - `a`, `b`: chromatic axes (green↔red, blue↔yellow).
pub fn srgb_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    // Step 1: sRGB gamma expansion → linear light
    let r = srgb_to_linear(r);
    let g = srgb_to_linear(g);
    let b = srgb_to_linear(b);

    // Step 2: linear RGB → LMS (Hunt-Pointer-Estévez adapted matrix)
    let l = 0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_99 * b;
    let m = 0.211_903_50 * r + 0.680_699_54 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_84 * g + 0.629_978_70 * b;

    // Step 3: LMS → LMS^(1/3) for perceptual uniformity
    let l = l.cbrt();
    let m = m.cbrt();
    let s = s.cbrt();

    // Step 4: LMS^(1/3) → OKLab
    let lab_l = 0.210_454_26 * l + 0.793_617_78 * m - 0.004_072_05 * s;
    let lab_a = 1.977_998_50 * l - 2.428_592_21 * m + 0.450_593_71 * s;
    let lab_b = 0.025_904_04 * l + 0.782_771_77 * m - 0.808_675_77 * s;

    (lab_l, lab_a, lab_b)
}

/// sRGB gamma expansion (IEC 61966-2-1).
#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

// ---------------------------------------------------------------------------
// Distance & contrast
// ---------------------------------------------------------------------------

/// Perceptual Euclidean distance between two sRGB colors (0–255 per channel).
///
/// Lower values mean more similar colors in human perception.
pub fn perceptual_distance(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f32 {
    let (l1, a1, b1) =
        srgb_to_oklab(r1 as f32 / 255.0, g1 as f32 / 255.0, b1 as f32 / 255.0);
    let (l2, a2, b2) =
        srgb_to_oklab(r2 as f32 / 255.0, g2 as f32 / 255.0, b2 as f32 / 255.0);
    let dl = l1 - l2;
    let da = a1 - a2;
    let db = b1 - b2;
    (dl * dl + da * da + db * db).sqrt()
}

/// Approximate contrast ratio between two sRGB colors via OKLab lightness.
///
/// Returns a value in [1.0, 21.0].  WCAG 2.1 AA normal text requires ≥ 4.5;
/// zedit uses a relaxed threshold of 3.0 to accommodate artistic themes.
///
/// Note: this is an *approximation* — WCAG 2.1 defines relative luminance via
/// the linear-light `Y` channel, not OKLab `L`.  The approximation is close
/// enough for editor use and avoids a separate luminance computation.
pub fn contrast_ratio(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f32 {
    let (l1, _, _) =
        srgb_to_oklab(r1 as f32 / 255.0, g1 as f32 / 255.0, b1 as f32 / 255.0);
    let (l2, _, _) =
        srgb_to_oklab(r2 as f32 / 255.0, g2 as f32 / 255.0, b2 as f32 / 255.0);
    let lighter = l1.max(l2) + 0.05;
    let darker = l1.min(l2) + 0.05;
    lighter / darker
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-3;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < EPSILON
    }

    // ── srgb_to_oklab ───────────────────────────────────────────────────────

    #[test]
    fn white_has_maximum_lightness() {
        let (l, _, _) = srgb_to_oklab(1.0, 1.0, 1.0);
        assert!(approx_eq(l, 1.0), "white L = {l}");
    }

    #[test]
    fn black_has_zero_lightness() {
        let (l, _, _) = srgb_to_oklab(0.0, 0.0, 0.0);
        assert!(approx_eq(l, 0.0), "black L = {l}");
    }

    #[test]
    fn gray_is_achromatic() {
        // A neutral gray should have a ≈ 0 and b ≈ 0.
        let (_, a, b) = srgb_to_oklab(0.5, 0.5, 0.5);
        assert!(a.abs() < 0.01, "gray a = {a}");
        assert!(b.abs() < 0.01, "gray b = {b}");
    }

    #[test]
    fn red_has_positive_a_axis() {
        // In OKLab the `a` axis runs green↔red; red should have a > 0.
        let (_, a, _) = srgb_to_oklab(1.0, 0.0, 0.0);
        assert!(a > 0.0, "red a = {a}");
    }

    // ── perceptual_distance ─────────────────────────────────────────────────

    #[test]
    fn identical_colors_have_zero_distance() {
        assert!(approx_eq(
            perceptual_distance(128, 64, 200, 128, 64, 200),
            0.0
        ));
    }

    #[test]
    fn black_white_has_maximum_distance() {
        let d = perceptual_distance(0, 0, 0, 255, 255, 255);
        // Black vs white: L = 0 vs L = 1, distance ≈ 1.0
        assert!(d > 0.9, "black-white distance = {d}");
    }

    #[test]
    fn perceptual_distance_is_symmetric() {
        let d1 = perceptual_distance(255, 0, 0, 0, 0, 255);
        let d2 = perceptual_distance(0, 0, 255, 255, 0, 0);
        assert!(approx_eq(d1, d2));
    }

    // ── contrast_ratio ──────────────────────────────────────────────────────

    #[test]
    fn same_color_has_unit_contrast() {
        let r = contrast_ratio(100, 100, 100, 100, 100, 100);
        assert!(approx_eq(r, 1.0), "same-color contrast = {r}");
    }

    #[test]
    fn black_white_contrast_near_maximum() {
        let r = contrast_ratio(0, 0, 0, 255, 255, 255);
        // Should be near 21 (WCAG max); OKLab approximation gives ~21.
        assert!(r > 18.0, "black-white contrast = {r}");
    }

    #[test]
    fn readable_pair_meets_threshold() {
        // Dark background (#1e1e2e) vs light text (#cdd6f4) — typical dark theme.
        let r = contrast_ratio(0x1e, 0x1e, 0x2e, 0xcd, 0xd6, 0xf4);
        assert!(r >= 3.0, "dark-theme contrast = {r}");
    }
}
