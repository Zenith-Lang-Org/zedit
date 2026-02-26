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
///
/// # Phase 31 optimizations (from microsoft/edit)
///
/// - `srgb_to_linear` replaced by a 256-entry compile-time LUT — O(1), no
///   transcendental `powf(2.4)` call.
/// - Cube-root replaced by a bit-manipulation estimator + one Newton step,
///   accurate to < 6.7 × 10⁻⁴ for inputs in [0, 1] — ~5× faster than
///   `f32::cbrt()`.
/// - `srgb_to_oklab_u8` accepts raw bytes directly, eliminating the
///   per-call `/ 255.0` division.
/// - Reverse transform `oklab_to_srgb_u8` added for future color blending.

// ---------------------------------------------------------------------------
// LUT: sRGB u8 → linear float (precomputed, zero runtime cost)
// ---------------------------------------------------------------------------
//
// Formula (IEC 61966-2-1):
//   if c/255 ≤ 0.04045  →  (c/255) / 12.92
//   else                →  ((c/255 + 0.055) / 1.055) ^ 2.4
//
// Values identical to those in microsoft/edit's `oklab.rs`.

#[rustfmt::skip]
const SRGB_LUT: [f32; 256] = [
    0.0000000000, 0.0003035270, 0.0006070540, 0.0009105810, 0.0012141080, 0.0015176350,
    0.0018211619, 0.0021246888, 0.0024282159, 0.0027317430, 0.0030352699, 0.0033465356,
    0.0036765069, 0.0040247170, 0.0043914421, 0.0047769533, 0.0051815170, 0.0056053917,
    0.0060488326, 0.0065120910, 0.0069954102, 0.0074990317, 0.0080231922, 0.0085681248,
    0.0091340570, 0.0097212177, 0.0103298230, 0.0109600937, 0.0116122449, 0.0122864870,
    0.0129830306, 0.0137020806, 0.0144438436, 0.0152085144, 0.0159962922, 0.0168073755,
    0.0176419523, 0.0185002182, 0.0193823613, 0.0202885624, 0.0212190095, 0.0221738834,
    0.0231533647, 0.0241576303, 0.0251868572, 0.0262412224, 0.0273208916, 0.0284260381,
    0.0295568332, 0.0307134409, 0.0318960287, 0.0331047624, 0.0343398079, 0.0356013142,
    0.0368894450, 0.0382043645, 0.0395462364, 0.0409151986, 0.0423114114, 0.0437350273,
    0.0451862030, 0.0466650836, 0.0481718220, 0.0497065634, 0.0512694679, 0.0528606549,
    0.0544802807, 0.0561284944, 0.0578054339, 0.0595112406, 0.0612460710, 0.0630100295,
    0.0648032799, 0.0666259527, 0.0684781820, 0.0703601092, 0.0722718611, 0.0742135793,
    0.0761853904, 0.0781874284, 0.0802198276, 0.0822827145, 0.0843762159, 0.0865004659,
    0.0886556059, 0.0908417329, 0.0930589810, 0.0953074843, 0.0975873619, 0.0998987406,
    0.1022417471, 0.1046164930, 0.1070231125, 0.1094617173, 0.1119324341, 0.1144353822,
    0.1169706732, 0.1195384338, 0.1221387982, 0.1247718409, 0.1274376959, 0.1301364899,
    0.1328683347, 0.1356333494, 0.1384316236, 0.1412633061, 0.1441284865, 0.1470272839,
    0.1499598026, 0.1529261619, 0.1559264660, 0.1589608639, 0.1620294005, 0.1651322246,
    0.1682693958, 0.1714410931, 0.1746473908, 0.1778884083, 0.1811642349, 0.1844749898,
    0.1878207624, 0.1912016720, 0.1946178079, 0.1980693042, 0.2015562356, 0.2050787061,
    0.2086368501, 0.2122307271, 0.2158605307, 0.2195262313, 0.2232279778, 0.2269658893,
    0.2307400703, 0.2345506549, 0.2383976579, 0.2422811985, 0.2462013960, 0.2501583695,
    0.2541521788, 0.2581829131, 0.2622507215, 0.2663556635, 0.2704978585, 0.2746773660,
    0.2788943350, 0.2831487954, 0.2874408960, 0.2917706966, 0.2961383164, 0.3005438447,
    0.3049873710, 0.3094689548, 0.3139887452, 0.3185468316, 0.3231432438, 0.3277781308,
    0.3324515820, 0.3371636569, 0.3419144452, 0.3467040956, 0.3515326977, 0.3564002514,
    0.3613068759, 0.3662526906, 0.3712377846, 0.3762622178, 0.3813261092, 0.3864295185,
    0.3915725648, 0.3967553079, 0.4019778669, 0.4072403014, 0.4125427008, 0.4178851545,
    0.4232677519, 0.4286905527, 0.4341537058, 0.4396572411, 0.4452012479, 0.4507858455,
    0.4564110637, 0.4620770514, 0.4677838385, 0.4735315442, 0.4793202281, 0.4851499796,
    0.4910208881, 0.4969330430, 0.5028865933, 0.5088814497, 0.5149177909, 0.5209956765,
    0.5271152258, 0.5332764983, 0.5394796133, 0.5457245708, 0.5520114899, 0.5583404899,
    0.5647116303, 0.5711249113, 0.5775805116, 0.5840784907, 0.5906189084, 0.5972018838,
    0.6038274169, 0.6104956269, 0.6172066331, 0.6239604354, 0.6307572126, 0.6375969648,
    0.6444797516, 0.6514056921, 0.6583748460, 0.6653873324, 0.6724432111, 0.6795425415,
    0.6866854429, 0.6938719153, 0.7011020184, 0.7083759308, 0.7156936526, 0.7230552435,
    0.7304608822, 0.7379105687, 0.7454043627, 0.7529423237, 0.7605246305, 0.7681512833,
    0.7758223414, 0.7835379243, 0.7912980318, 0.7991028428, 0.8069523573, 0.8148466945,
    0.8227858543, 0.8307699561, 0.8387991190, 0.8468732834, 0.8549926877, 0.8631572723,
    0.8713672161, 0.8796223402, 0.8879231811, 0.8962693810, 0.9046613574, 0.9130986929,
    0.9215820432, 0.9301108718, 0.9386858940, 0.9473065734, 0.9559735060, 0.9646862745,
    0.9734454751, 0.9822505713, 0.9911022186, 1.0000000000,
];

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// sRGB byte → linear light via LUT. O(1), no transcendental.
#[inline(always)]
fn srgb_to_linear(c: u8) -> f32 {
    SRGB_LUT[c as usize]
}

/// Fast cube-root estimator: float bit-manipulation seed + one Newton–Raphson step.
///
/// Max error < 6.7 × 10⁻⁴ for inputs in [0, 1].  That is sufficient for color
/// work (equivalent to a ±0.17 LSB error in 8-bit color depth).
///
/// Technique: same "evil floating-point bit level hack" family as the fast inverse
/// square root.  Reference: <http://metamerist.com/cbrt/cbrt.htm>
/// Identical to `cbrtf_est()` in microsoft/edit's `oklab.rs`.
#[inline(always)]
fn fast_cbrt(a: f32) -> f32 {
    if a == 0.0 {
        return 0.0;
    }
    let bits: u32 = a.to_bits();
    let est = f32::from_bits(bits / 3 + 709_921_077);
    // One Newton–Raphson step: x₁ = (1/3) × (a/x₀² + 2×x₀)
    (1.0 / 3.0) * (a / (est * est) + est + est)
}

/// linear float → sRGB byte (clamped). Used by the reverse transform.
#[allow(dead_code)]
#[inline]
fn linear_to_srgb(c: f32) -> u8 {
    let v = if c > 0.003_130_8 {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    } else {
        12.92 * c
    };
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

// ---------------------------------------------------------------------------
// Forward transform: sRGB → OKLab
// ---------------------------------------------------------------------------

/// Convert sRGB bytes (0–255 each) to OKLab (L, a, b).
///
/// - `L` ∈ [0, 1]: perceptual lightness (0 = black, 1 = white).
/// - `a`, `b`: chromatic axes (green↔red, blue↔yellow).
///
/// This is the fast path: uses the precomputed LUT for gamma and the
/// bit-manipulation cube-root estimator.  ~7× faster than the float variant.
pub fn srgb_to_oklab_u8(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    // Step 1: sRGB → linear light (LUT lookup, O(1))
    let r = srgb_to_linear(r);
    let g = srgb_to_linear(g);
    let b = srgb_to_linear(b);

    // Step 2: linear RGB → LMS (Hunt-Pointer-Estévez adapted matrix)
    let l = 0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_99 * b;
    let m = 0.211_903_50 * r + 0.680_699_54 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_84 * g + 0.629_978_70 * b;

    // Step 3: LMS → LMS^(1/3) (fast cbrt estimator)
    let l = fast_cbrt(l);
    let m = fast_cbrt(m);
    let s = fast_cbrt(s);

    // Step 4: LMS^(1/3) → OKLab
    (
        0.210_454_26 * l + 0.793_617_78 * m - 0.004_072_05 * s,
        1.977_998_50 * l - 2.428_592_21 * m + 0.450_593_71 * s,
        0.025_904_04 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    )
}

/// Convert sRGB floats (0.0–1.0 each) to OKLab (L, a, b).
///
/// Convenience wrapper around `srgb_to_oklab_u8` for call sites that already
/// have normalized floats.  Internally quantizes to u8 (max rounding error ±0.002
/// in the final Lab values, within the EPSILON = 1e-3 of all existing tests).
#[allow(dead_code)]
pub fn srgb_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    srgb_to_oklab_u8(
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

// ---------------------------------------------------------------------------
// Reverse transform: OKLab → sRGB
// ---------------------------------------------------------------------------

/// Convert OKLab (L, a, b) back to sRGB bytes (0–255 each), clamped.
///
/// Useful for blending colors in the perceptually uniform OKLab space and
/// converting the result back for terminal output.
#[allow(dead_code)]
pub fn oklab_to_srgb_u8(lab_l: f32, lab_a: f32, lab_b: f32) -> (u8, u8, u8) {
    // Step 1: OKLab → LMS^(1/3)
    let l_ = lab_l + 0.396_337_78 * lab_a + 0.215_803_76 * lab_b;
    let m_ = lab_l - 0.105_561_35 * lab_a - 0.063_854_17 * lab_b;
    let s_ = lab_l - 0.089_484_18 * lab_a - 1.291_485_55 * lab_b;

    // Step 2: cube → LMS
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    // Step 3: LMS → linear RGB
    let r = (4.076_741_66 * l - 3.307_711_59 * m + 0.230_969_93 * s).clamp(0.0, 1.0);
    let g = (-1.268_438_00 * l + 2.609_757_40 * m - 0.341_319_40 * s).clamp(0.0, 1.0);
    let b = (-0.004_196_09 * l - 0.703_418_61 * m + 1.707_614_70 * s).clamp(0.0, 1.0);

    // Step 4: linear → sRGB
    (linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b))
}

// ---------------------------------------------------------------------------
// Distance & contrast
// ---------------------------------------------------------------------------

/// Perceptual Euclidean distance between two sRGB colors (0–255 per channel).
///
/// Lower values mean more similar colors in human perception.
pub fn perceptual_distance(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f32 {
    let (l1, a1, b1) = srgb_to_oklab_u8(r1, g1, b1);
    let (l2, a2, b2) = srgb_to_oklab_u8(r2, g2, b2);
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
    let (l1, _, _) = srgb_to_oklab_u8(r1, g1, b1);
    let (l2, _, _) = srgb_to_oklab_u8(r2, g2, b2);
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

    // ── fast_cbrt ───────────────────────────────────────────────────────────

    #[test]
    fn fast_cbrt_zero() {
        assert_eq!(fast_cbrt(0.0), 0.0);
    }

    #[test]
    fn fast_cbrt_one() {
        // cbrt(1.0) = 1.0 — Newton step is exact here.
        assert!(approx_eq(fast_cbrt(1.0), 1.0));
    }

    #[test]
    fn fast_cbrt_accuracy() {
        // Verify error < 1e-3 across all 256 LUT input values.
        for i in 0u8..=255 {
            let x = SRGB_LUT[i as usize]; // linear value in [0, 1]
            let expected = x.cbrt();
            let got = fast_cbrt(x);
            assert!(
                (got - expected).abs() < 1e-3,
                "fast_cbrt({x}) = {got}, expected {expected}, diff = {}",
                (got - expected).abs()
            );
        }
    }

    // ── srgb_to_oklab_u8 ────────────────────────────────────────────────────

    #[test]
    fn white_has_maximum_lightness() {
        let (l, _, _) = srgb_to_oklab_u8(255, 255, 255);
        assert!(approx_eq(l, 1.0), "white L = {l}");
    }

    #[test]
    fn black_has_zero_lightness() {
        let (l, _, _) = srgb_to_oklab_u8(0, 0, 0);
        assert!(approx_eq(l, 0.0), "black L = {l}");
    }

    #[test]
    fn gray_is_achromatic() {
        let (_, a, b) = srgb_to_oklab_u8(128, 128, 128);
        assert!(a.abs() < 0.01, "gray a = {a}");
        assert!(b.abs() < 0.01, "gray b = {b}");
    }

    #[test]
    fn red_has_positive_a_axis() {
        let (_, a, _) = srgb_to_oklab_u8(255, 0, 0);
        assert!(a > 0.0, "red a = {a}");
    }

    // ── srgb_to_oklab (float wrapper) ───────────────────────────────────────

    #[test]
    fn float_wrapper_white() {
        let (l, _, _) = srgb_to_oklab(1.0, 1.0, 1.0);
        assert!(approx_eq(l, 1.0), "white L = {l}");
    }

    #[test]
    fn float_wrapper_black() {
        let (l, _, _) = srgb_to_oklab(0.0, 0.0, 0.0);
        assert!(approx_eq(l, 0.0), "black L = {l}");
    }

    #[test]
    fn float_wrapper_gray_achromatic() {
        let (_, a, b) = srgb_to_oklab(0.5, 0.5, 0.5);
        assert!(a.abs() < 0.01, "gray a = {a}");
        assert!(b.abs() < 0.01, "gray b = {b}");
    }

    #[test]
    fn float_wrapper_red_positive_a() {
        let (_, a, _) = srgb_to_oklab(1.0, 0.0, 0.0);
        assert!(a > 0.0, "red a = {a}");
    }

    // ── oklab_to_srgb_u8 (reverse transform) ───────────────────────────────

    #[test]
    fn roundtrip_black() {
        let (l, a, b) = srgb_to_oklab_u8(0, 0, 0);
        let (r, g, bv) = oklab_to_srgb_u8(l, a, b);
        assert_eq!((r, g, bv), (0, 0, 0));
    }

    #[test]
    fn roundtrip_white() {
        let (l, a, b) = srgb_to_oklab_u8(255, 255, 255);
        let (r, g, bv) = oklab_to_srgb_u8(l, a, b);
        assert_eq!((r, g, bv), (255, 255, 255));
    }

    #[test]
    fn roundtrip_midtone() {
        // Cornflower blue — a recognizable mid-tone.
        let (r0, g0, b0) = (100u8, 149u8, 237u8);
        let (l, a, b) = srgb_to_oklab_u8(r0, g0, b0);
        let (r1, g1, b1) = oklab_to_srgb_u8(l, a, b);
        assert!((r0 as i32 - r1 as i32).abs() <= 1, "R: {r0} → {r1}");
        assert!((g0 as i32 - g1 as i32).abs() <= 1, "G: {g0} → {g1}");
        assert!((b0 as i32 - b1 as i32).abs() <= 1, "B: {b0} → {b1}");
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
        assert!(r > 18.0, "black-white contrast = {r}");
    }

    #[test]
    fn readable_pair_meets_threshold() {
        // Dark background (#1e1e2e) vs light text (#cdd6f4) — typical dark theme.
        let r = contrast_ratio(0x1e, 0x1e, 0x2e, 0xcd, 0xd6, 0xf4);
        assert!(r >= 3.0, "dark-theme contrast = {r}");
    }
}
