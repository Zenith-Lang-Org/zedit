//! SIMD-accelerated newline scanning with runtime CPU dispatch.
//!
//! The public entry point is [`scan_newlines`].  On x86_64 a static function
//! pointer is set once on first call (after detecting CPU features) and reused
//! for every subsequent call — zero overhead after warm-up.
//!
//! Throughput targets per `scan_newlines` call:
//!   AVX2  (x86_64):  128 bytes / iteration  (~16× over byte-by-byte)
//!   NEON  (aarch64):  64 bytes / iteration   (~8×  over byte-by-byte)
//!   SWAR  (fallback):  8 bytes / iteration   (~4×  over byte-by-byte)

/// Scan `data` for `\n` bytes, pushing the logical byte offset of each
/// *following line start* (i.e. `base + i + 1`) into `out`.
///
/// `base` is the logical byte offset of `data[0]` within the full buffer —
/// used when scanning only one segment of a gap buffer (pre-gap or post-gap).
#[allow(dead_code)]
pub fn scan_newlines(data: &[u8], base: usize, out: &mut Vec<usize>) {
    // ── x86_64: set the function pointer once, then branch-free dispatch ──
    #[cfg(target_arch = "x86_64")]
    {
        use std::mem;
        use std::sync::atomic::{AtomicPtr, Ordering};

        type ScanFn = fn(&[u8], usize, &mut Vec<usize>);

        // Initialised to `detect`; replaced on first call with the best impl.
        static DISPATCH: AtomicPtr<()> = AtomicPtr::new(detect as *mut ());

        fn detect(data: &[u8], base: usize, out: &mut Vec<usize>) {
            let f: ScanFn = if is_x86_feature_detected!("avx2") {
                scan_avx2
            } else {
                scan_swar
            };
            // Relaxed: CPU features are fixed; all threads store the same value.
            DISPATCH.store(f as *mut (), Ordering::Relaxed);
            f(data, base, out);
        }

        let ptr = DISPATCH.load(Ordering::Relaxed);
        // SAFETY: DISPATCH only ever holds pointers to `detect`, `scan_avx2`,
        // or `scan_swar` — all of which have exactly the signature ScanFn.
        let f: ScanFn = unsafe { mem::transmute(ptr) };
        return f(data, base, out);
    }

    // ── aarch64: NEON is baseline — always available ──
    #[cfg(target_arch = "aarch64")]
    {
        return scan_neon(data, base, out);
    }

    // ── All other architectures: portable SWAR word scan ──
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    scan_swar(data, base, out);
}

// ---------------------------------------------------------------------------
// AVX2 — x86_64 with runtime feature check
// ---------------------------------------------------------------------------

/// Inner AVX2 implementation.  Must only be called when AVX2 is available.
/// `#[target_feature]` enables the compiler to emit AVX2 instructions.
///
/// Rust 2024 edition: unsafe operations inside `unsafe fn` still require
/// explicit `unsafe {}` blocks (`unsafe_op_in_unsafe_fn` is now deny-by-default).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_avx2_inner(data: &[u8], base: usize, out: &mut Vec<usize>) {
    use std::arch::x86_64::*;

    let lf = _mm256_set1_epi8(b'\n' as i8);
    let mut i = 0_usize;

    // ── Main loop: 128 bytes per iteration (4 × 32), loop-unrolled ──
    //
    // Strategy: OR the movemask results from four 32-byte comparisons.
    // Any set bit means at least one newline is present → fall to per-byte scan.
    // The common case for long lines (no newline in 128 bytes) is a single
    // branch-not-taken per iteration.
    while i + 128 <= data.len() {
        let (v0, v1, v2, v3) = unsafe {
            (
                _mm256_loadu_si256(data.as_ptr().add(i) as *const __m256i),
                _mm256_loadu_si256(data.as_ptr().add(i + 32) as *const __m256i),
                _mm256_loadu_si256(data.as_ptr().add(i + 64) as *const __m256i),
                _mm256_loadu_si256(data.as_ptr().add(i + 96) as *const __m256i),
            )
        };

        // movemask_epi8: MSB of each byte → 32-bit mask.
        // cmpeq_epi8 sets matched bytes to 0xFF (MSB = 1).
        let any = _mm256_movemask_epi8(_mm256_cmpeq_epi8(v0, lf))
            | _mm256_movemask_epi8(_mm256_cmpeq_epi8(v1, lf))
            | _mm256_movemask_epi8(_mm256_cmpeq_epi8(v2, lf))
            | _mm256_movemask_epi8(_mm256_cmpeq_epi8(v3, lf));

        if any != 0 {
            // At least one newline in this 128-byte chunk: find exact positions.
            for j in 0..128_usize {
                if data[i + j] == b'\n' {
                    out.push(base + i + j + 1);
                }
            }
        }
        i += 128;
    }

    // ── 32-byte tail: extract exact byte positions via bit scan ──
    while i + 32 <= data.len() {
        let v = unsafe { _mm256_loadu_si256(data.as_ptr().add(i) as *const __m256i) };
        let mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(v, lf)) as u32;
        if mask != 0 {
            let mut m = mask;
            while m != 0 {
                let bit = m.trailing_zeros() as usize;
                out.push(base + i + bit + 1);
                m &= m - 1; // clear lowest set bit
            }
        }
        i += 32;
    }

    // ── Scalar remainder (< 32 bytes) ──
    while i < data.len() {
        if data[i] == b'\n' {
            out.push(base + i + 1);
        }
        i += 1;
    }
}

/// Safe wrapper: calls `scan_avx2_inner` only after AVX2 was confirmed.
#[cfg(target_arch = "x86_64")]
fn scan_avx2(data: &[u8], base: usize, out: &mut Vec<usize>) {
    // SAFETY: only stored in DISPATCH after is_x86_feature_detected!("avx2").
    unsafe { scan_avx2_inner(data, base, out) }
}

// ---------------------------------------------------------------------------
// NEON — aarch64 (always available, baseline ABI)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
fn scan_neon(data: &[u8], base: usize, out: &mut Vec<usize>) {
    use std::arch::aarch64::*;

    let lf = unsafe { vdupq_n_u8(b'\n') };
    let mut i = 0_usize;

    // ── Main loop: 64 bytes per iteration (4 × 16), loop-unrolled ──
    //
    // Accumulate match counts per byte position across the 4 vectors.
    // `vceqq_u8` gives 0xFF for matches; `vsubq_u8(0, 0xFF)` = 1 (u8 wrap).
    // After 4 ops each byte in `sum` holds 0..4 (count of matching vectors).
    // `vaddvq_u8` gives the horizontal sum (0..64 — fits in u8).
    while i + 64 <= data.len() {
        let (v0, v1, v2, v3) = unsafe {
            (
                vld1q_u8(data.as_ptr().add(i)),
                vld1q_u8(data.as_ptr().add(i + 16)),
                vld1q_u8(data.as_ptr().add(i + 32)),
                vld1q_u8(data.as_ptr().add(i + 48)),
            )
        };

        let sum = unsafe {
            let mut s = vdupq_n_u8(0);
            s = vsubq_u8(s, vceqq_u8(v0, lf));
            s = vsubq_u8(s, vceqq_u8(v1, lf));
            s = vsubq_u8(s, vceqq_u8(v2, lf));
            s = vsubq_u8(s, vceqq_u8(v3, lf));
            s
        };

        if unsafe { vaddvq_u8(sum) } > 0 {
            for j in 0..64_usize {
                if data[i + j] == b'\n' {
                    out.push(base + i + j + 1);
                }
            }
        }
        i += 64;
    }

    // ── Scalar remainder (< 64 bytes) ──
    while i < data.len() {
        if data[i] == b'\n' {
            out.push(base + i + 1);
        }
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// SWAR — portable fallback, no platform intrinsics
// ---------------------------------------------------------------------------

/// 8-byte word scan.  Same algorithm that was previously inlined in
/// `src/buffer.rs` (Phase 25).  Moved here so it can serve as the fallback
/// for `scan_newlines` on non-AVX2 x86_64 and on architectures other than
/// x86_64 / aarch64.
fn scan_swar(data: &[u8], base: usize, out: &mut Vec<usize>) {
    // '\n' = 0x0A. XOR with NL_MASK turns every '\n' into 0x00.
    // Zero-byte detection: (word - 0x01..01) & ~word & 0x80..80 is non-zero
    // iff any byte in `word` is zero.
    const NL_MASK: u64 = 0x0A0A_0A0A_0A0A_0A0A_u64;
    const LO_BITS: u64 = 0x0101_0101_0101_0101_u64;
    const HI_BITS: u64 = 0x8080_8080_8080_8080_u64;

    let mut i = 0_usize;

    // Walk byte-by-byte until the pointer is 8-byte aligned.
    while i < data.len() && (data[i..].as_ptr() as usize) % 8 != 0 {
        if data[i] == b'\n' {
            out.push(base + i + 1);
        }
        i += 1;
    }

    // 8-byte word scan.
    while i + 8 <= data.len() {
        let word = u64::from_le_bytes(data[i..i + 8].try_into().unwrap());
        let xor = word ^ NL_MASK;
        let has_zero = xor.wrapping_sub(LO_BITS) & !xor & HI_BITS;
        if has_zero != 0 {
            for j in 0..8_usize {
                if data[i + j] == b'\n' {
                    out.push(base + i + j + 1);
                }
            }
        }
        i += 8;
    }

    // Remaining tail bytes (< 8).
    while i < data.len() {
        if data[i] == b'\n' {
            out.push(base + i + 1);
        }
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Ground-truth reference: simple byte-by-byte scan.
    fn scan_scalar(data: &[u8], base: usize) -> Vec<usize> {
        data.iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .map(|(i, _)| base + i + 1)
            .collect()
    }

    // ── SWAR unit tests ──────────────────────────────────────────────────

    #[test]
    fn swar_empty() {
        let mut out = Vec::new();
        scan_swar(b"", 0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn swar_no_newlines() {
        let mut out = Vec::new();
        scan_swar(b"hello world", 0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn swar_single_newline() {
        let mut out = Vec::new();
        scan_swar(b"hello\nworld", 0, &mut out);
        assert_eq!(out, vec![6]);
    }

    #[test]
    fn swar_trailing_newline() {
        let mut out = Vec::new();
        scan_swar(b"abc\n", 0, &mut out);
        assert_eq!(out, vec![4]);
    }

    #[test]
    fn swar_only_newlines() {
        let mut out = Vec::new();
        scan_swar(b"\n\n\n", 0, &mut out);
        assert_eq!(out, vec![1, 2, 3]);
    }

    #[test]
    fn swar_base_offset() {
        let mut out = Vec::new();
        scan_swar(b"a\nb\n", 100, &mut out);
        assert_eq!(out, vec![102, 104]);
    }

    #[test]
    fn swar_matches_scalar_lengths_0_to_64() {
        for len in 0..=64_usize {
            let text: Vec<u8> = (0..len)
                .map(|i| if i % 7 == 6 { b'\n' } else { b'x' })
                .collect();
            let expected = scan_scalar(&text, 0);
            let mut actual = Vec::new();
            scan_swar(&text, 0, &mut actual);
            assert_eq!(actual, expected, "swar mismatch at len={len}");
        }
    }

    // ── scan_newlines (dispatch) tests ───────────────────────────────────

    #[test]
    fn dispatch_empty() {
        let mut out = Vec::new();
        scan_newlines(b"", 0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn dispatch_matches_scalar() {
        let text = b"line1\nline2\nline3\nno newline at end";
        let expected = scan_scalar(text, 0);
        let mut actual = Vec::new();
        scan_newlines(text, 0, &mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn dispatch_base_offset() {
        let text = b"a\nb";
        let mut out = Vec::new();
        scan_newlines(text, 1000, &mut out);
        assert_eq!(out, vec![1002]);
    }

    /// Verifies correctness at every byte position in a 256-byte buffer.
    /// This catches off-by-one errors in alignment prefix / tail handling
    /// and any missed newlines in the SIMD main loop boundaries.
    #[test]
    fn dispatch_newline_at_every_position_256() {
        for pos in 0..256_usize {
            let mut data = vec![b'x'; 256];
            data[pos] = b'\n';
            let expected = vec![pos + 1];
            let mut actual = Vec::new();
            scan_newlines(&data, 0, &mut actual);
            assert_eq!(
                actual, expected,
                "newline at position {pos} was not detected"
            );
        }
    }

    #[test]
    fn dispatch_no_newlines_256_bytes() {
        let data = vec![b'a'; 256];
        let mut out = Vec::new();
        scan_newlines(&data, 0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn dispatch_large_buffer() {
        // 512 KB of "a\n" repeated — 262 144 newlines.
        let data: Vec<u8> = b"a\n".iter().cycle().take(524_288).copied().collect();
        let expected = scan_scalar(&data, 0);
        let mut actual = Vec::new();
        scan_newlines(&data, 0, &mut actual);
        assert_eq!(actual.len(), 262_144, "wrong newline count in large buffer");
        assert_eq!(actual, expected, "large buffer results diverge from scalar");
    }

    #[test]
    fn dispatch_matches_scalar_lengths_0_to_256() {
        for len in 0..=256_usize {
            let text: Vec<u8> = (0..len)
                .map(|i| if i % 11 == 10 { b'\n' } else { b'y' })
                .collect();
            let expected = scan_scalar(&text, 0);
            let mut actual = Vec::new();
            scan_newlines(&text, 0, &mut actual);
            assert_eq!(actual, expected, "dispatch mismatch at len={len}");
        }
    }

    // ── AVX2-specific tests (skipped gracefully on non-AVX2 hardware) ─────

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_matches_scalar_when_available() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        let text: Vec<u8> = (0..512_usize)
            .map(|i| if i % 13 == 0 { b'\n' } else { b'x' })
            .collect();
        let expected = scan_scalar(&text, 0);
        let mut actual = Vec::new();
        scan_avx2(&text, 0, &mut actual);
        assert_eq!(actual, expected, "avx2 results diverge from scalar");
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_newline_at_every_position_256() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        for pos in 0..256_usize {
            let mut data = vec![b'x'; 256];
            data[pos] = b'\n';
            let expected = vec![pos + 1];
            let mut actual = Vec::new();
            scan_avx2(&data, 0, &mut actual);
            assert_eq!(
                actual, expected,
                "avx2: newline at position {pos} was not detected"
            );
        }
    }
}
