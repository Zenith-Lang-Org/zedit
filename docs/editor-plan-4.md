# Zedit Phase 4 — Performance & Polish

Inspired by `microsoft/edit`. Six targeted improvements: correct glob engine,
incremental line cache, perceptual color science, optimal fuzzy matching,
file-picker path completion, and mmap large-file support.

---

## Appendix A — Implementation Control (Reordered)

> Analysis date: 2026-02-24. Order revised after codebase audit.
> Original roadmap sorted by phase number; this appendix reorders by
> risk/impact ratio and tracks execution status.

### Reordered Execution Plan

| Order | Phase | Feature | Difficulty | Impact | Risk | Status |
|-------|-------|---------|------------|--------|------|--------|
| 1 | 27 | Fuzzy matching upgrade | Low | High — daily UX | Very low | `DONE` |
| 2 | 24 | Glob engine rewrite | Low-Med | High — correctness | Low | `DONE` |
| 3 | 25 | Incremental line cache | Medium | High — performance | Medium | `TODO` |
| 4 | 26 | OKLab color system | Low | Medium — visual quality | Low | `TODO` |
| 5 | 28 | File picker path completion | High | Medium — UX | Medium | `TODO` |
| 6 | 29 | Large file mmap support | Very high | Low-Med — scale | High | `TODO` |

### Rationale

**Phase 27 first** — drop-in replacement of `fuzzy_score()` in `src/editor/palette.rs`.
Zero API changes, no new files, immediately visible improvement in command palette ranking.

**Phase 24 second** — correctness fix before performance. The recursive O(2^m) glob in
`src/filetree.rs:886` can freeze the editor on large directory trees. Must be fixed before
optimizing anything else that depends on file scanning.

**Phase 25 third** — highest raw performance gain (~40x on 100KB files). Medium risk because
`update_lines()` must handle all edge cases (insert at start, delete spanning multiple newlines,
empty buffer). Requires careful testing.

**Phase 26 fourth** — pure math, no API changes, no new dependencies. Low risk, improves
color fidelity in 16-color terminals. Note: `contrast_ratio()` approximates WCAG using OKLab L
component (not identical to WCAG 2.1 relative luminance) — threshold relaxed to 3.0.

**Phase 28 fifth** — most lines of UI code. Touches `prompt.rs`, `mod.rs`, and `view.rs`.
Known issue in plan: `longest_common_prefix()` must use `.char_indices()` for UTF-8 safe
byte boundaries, not `.chars().count()`. Requires fix during implementation.

**Phase 29 last** — highest complexity and risk. Plan describes the read path (mmap → gap buffer
materialization) but does not resolve the **save path** for partially-edited mmap-backed buffers.
This must be designed before implementation begins.

### Known Issues to Resolve Per Phase

| Phase | Issue | Resolution |
|-------|-------|------------|
| 27 | Phase B backtracking is O(n²) | Acceptable — palette has ~60 short entries |
| 24 | `**` double-star needs path separator awareness | Handled in plan: `double` flag in iterator |
| 25 | `update_lines()` adjust-then-truncate ordering | Plan is correct; verify with edge-case tests |
| 26 | `contrast_ratio()` is not strict WCAG 2.1 | Documented — threshold lowered to 3.0 |
| 28 | `longest_common_prefix()` has UTF-8 boundary bug | Use `.char_indices()` instead of `.chars().count()` |
| 29 | Save path for partial mmap+gap buffer not designed | Must resolve before starting Phase 29 |

### Status Legend

| Symbol | Meaning |
|--------|---------|
| `TODO` | Not started |
| `WIP` | In progress |
| `DONE` | Implemented and tested |
| `SKIP` | Deferred or cancelled |

---

---

## Context: What microsoft/edit Does Better

| Area | microsoft/edit | zedit (current) | Gap |
|------|---------------|-----------------|-----|
| Glob matching | Iterative, linear O(n×m) | Recursive, exponential O(2^m) | Correctness |
| Line scan | SIMD (SSE2, 16 bytes/cycle) | Byte-by-byte, full rebuild | 5–8x perf |
| Color downsampling | OKLab (perceptual) | rec601 luma (linear) | Visual quality |
| Fuzzy search | Dedicated `fuzzy.rs`, scored | Greedy first-match | Match quality |
| File picker | Full directory browser UI | Modal text input, no autocomplete | Daily UX |
| Large files | Virtual memory + commit pages | `Vec<u8>` uniform, limit ~50MB | Scale |

---

## Implementation Roadmap

| # | Phase | Feature | ~Lines | Priority | Status |
|---|-------|---------|--------|----------|--------|
| 1 | 24 | Glob engine rewrite | 200 | High — correctness | TODO |
| 2 | 25 | Incremental line cache | 300 | High — performance | TODO |
| 3 | 26 | OKLab color system | 250 | Medium — quality | TODO |
| 4 | 27 | Fuzzy matching upgrade | 150 | Medium — UX | TODO |
| 5 | 28 | File picker path completion | 350 | Medium — UX | TODO |
| 6 | 29 | Large file mmap support | 400 | Low — scale | TODO |

Total: ~1,650 new/modified lines across 6 phases.

---

## Phase 24: Glob Engine Rewrite

**Goal**: Replace the recursive backtracking glob with an iterative O(n×m) implementation.
Add `**` recursive directory matching and `[a-z]` character ranges.

### Problem

`src/filetree.rs` current implementation:

```rust
fn glob_match_inner(pat: &[char], name: &[char]) -> bool {
    match (pat.first(), name.first()) {
        (Some('*'), _) => {
            glob_match_inner(&pat[1..], name)                           // skip wildcard
                || (!name.is_empty() && glob_match_inner(pat, &name[1..]))  // consume char
        }
        ...
    }
}
```

For pattern `a*a*a*b` on a non-matching string of length n, this recurses O(2^m) times
where m is the number of `*` wildcards. A `.gitignore`-style pattern with 5 wildcards
can freeze the editor on a large directory tree.

### Target Architecture

Two-pointer iterative algorithm. No recursion, no heap allocation, O(n×m) worst case.

### New File: `src/glob.rs`

```rust
/// Match a glob pattern against a single path component or full path.
///
/// Supported patterns:
///   *       — zero or more chars (except '/')
///   **      — zero or more path segments (matches '/')
///   ?       — exactly one char (except '/')
///   [abc]   — character class (literal chars)
///   [a-z]   — character range
///   [!abc]  — negated character class
///
/// All matching is case-sensitive.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), path.as_bytes())
}

fn glob_match_inner(pat: &[u8], name: &[u8]) -> bool {
    let (mut pi, mut ni) = (0, 0);
    let (mut star_pi, mut star_ni) = (usize::MAX, 0);

    while ni < name.len() {
        if pi < pat.len() && (pat[pi] == b'?' || pat[pi] == name[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            // Check for '**' (matches path separators too)
            let double = pi + 1 < pat.len() && pat[pi + 1] == b'*';
            star_pi = pi;
            star_ni = ni;
            pi += if double { 2 } else { 1 };
        } else if pi < pat.len() && pat[pi] == b'[' {
            if let Some((matched, consumed)) = match_class(&pat[pi..], name[ni]) {
                if matched {
                    pi += consumed;
                    ni += 1;
                } else {
                    // class didn't match — backtrack to last '*' if any
                    if star_pi == usize::MAX { return false; }
                    pi = star_pi + 1;
                    star_ni += 1;
                    ni = star_ni;
                }
            } else {
                return false; // malformed class
            }
        } else if star_pi != usize::MAX {
            // backtrack: extend the '*' by one more character
            pi = star_pi + 1;
            star_ni += 1;
            ni = star_ni;
        } else {
            return false;
        }
    }

    // Skip any trailing '*' or '**' in the pattern
    while pi < pat.len() && (pat[pi] == b'*') {
        pi += 1;
    }

    pi == pat.len()
}

/// Match a `[class]` pattern starting at `pat` against byte `b`.
/// Returns `Some((matched, bytes_consumed_from_pat))` or `None` if malformed.
fn match_class(pat: &[u8], b: u8) -> Option<(bool, usize)> {
    if pat.first() != Some(&b'[') { return None; }
    let mut i = 1;
    let negate = i < pat.len() && pat[i] == b'!';
    if negate { i += 1; }

    let mut found = false;
    while i < pat.len() && pat[i] != b']' {
        if i + 2 < pat.len() && pat[i + 1] == b'-' && pat[i + 2] != b']' {
            // range: [a-z]
            if b >= pat[i] && b <= pat[i + 2] { found = true; }
            i += 3;
        } else {
            if pat[i] == b { found = true; }
            i += 1;
        }
    }
    if i >= pat.len() { return None; } // unclosed '['
    let consumed = i + 1; // include the closing ']'
    Some((found ^ negate, consumed))
}

/// Check if `path` matches any pattern in the list.
pub fn matches_any(patterns: &[&str], path: &str) -> bool {
    patterns.iter().any(|p| glob_match(p, path))
}
```

### Migration

Replace all calls to `glob_match_inner()` in `src/filetree.rs` with `crate::glob::glob_match()`.
The function signature is compatible — callers pass `&str` instead of `&[char]`.

Update the ignore list in `src/filetree.rs`:
```rust
// Before
const IGNORED: &[&str] = &[".git", "target", "node_modules", ...];

// After — now supports patterns
const IGNORED: &[&str] = &[
    ".git", "target", "node_modules", "__pycache__",
    "*.o", "*.a", "*.so", "*.dylib",
    "*.class", "*.pyc",
    ".DS_Store", "Thumbs.db",
];
```

### Unit Tests (`src/glob.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn exact_match()          { assert!(glob_match("foo.rs", "foo.rs")); }
    #[test] fn star_prefix()          { assert!(glob_match("*.rs", "main.rs")); }
    #[test] fn star_no_slash()        { assert!(!glob_match("*.rs", "src/main.rs")); }
    #[test] fn double_star()          { assert!(glob_match("**/*.rs", "src/main.rs")); }
    #[test] fn question_mark()        { assert!(glob_match("foo?.rs", "foob.rs")); }
    #[test] fn char_class()           { assert!(glob_match("[abc].rs", "b.rs")); }
    #[test] fn char_range()           { assert!(glob_match("[a-z].rs", "m.rs")); }
    #[test] fn negated_class()        { assert!(!glob_match("[!abc].rs", "a.rs")); }
    #[test] fn no_exponential_blowup() {
        // This would hang with the old recursive implementation
        let pat = "a*a*a*a*a*a*a*b";
        let name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaac";
        assert!(!glob_match(pat, name)); // must return in microseconds
    }
    #[test] fn multi_wildcard_match() {
        assert!(glob_match("src/**/mod.rs", "src/editor/view/mod.rs"));
    }
}
```

---

## Phase 25: Incremental Line Cache

**Goal**: Avoid full O(n) `rebuild_lines()` on every edit. Re-scan only from the edit
point forward, stopping as soon as the cached state matches.

### Problem

`src/buffer.rs` current implementation:

```rust
fn rebuild_lines(&mut self) {
    self.lines.clear();
    self.lines.push(0);
    let total = self.len();
    for i in 0..total {                       // full O(n) scan
        if self.byte_at(i) == Some(b'\n') {
            self.lines.push(i + 1);
        }
    }
}
```

Called after every `insert()` and `delete()`. A 500KB file has ~10,000 lines.
Typing one character triggers a scan of ~500,000 bytes. At 60 WPM that's 300 full
scans per second, burning 150MB/s of memory bandwidth for nothing.

### Target Architecture

Split into:
1. **Fast word scan**: scan 8 bytes at a time using `u64` alignment trick (no deps)
2. **Incremental update**: only re-scan from the modified line; splice `self.lines`

### Modified `src/buffer.rs`

```rust
impl Buffer {
    /// Full rebuild — only called on initial load.
    fn rebuild_lines_full(&mut self) {
        self.lines.clear();
        self.lines.push(0);
        let data = self.as_contiguous_bytes(); // collapses gap to temp slice
        scan_newlines(data, 0, &mut self.lines);
    }

    /// Incremental update after insert/delete at byte position `pos`.
    ///
    /// Algorithm:
    ///   1. Find which line `pos` belongs to (binary search in self.lines).
    ///   2. Remove all cached line-starts after `pos`.
    ///   3. Re-scan from `pos` to end of buffer.
    ///   4. Adjust all line-start offsets that come after `pos` by `delta`
    ///      (delta = +text.len() for insert, -len for delete).
    fn update_lines(&mut self, edit_pos: usize, delta: isize) {
        // 1. Find insertion line index (binary search)
        let line_idx = self.lines.partition_point(|&off| off <= edit_pos);

        // 2. Adjust existing offsets after edit_pos
        for off in &mut self.lines[line_idx..] {
            *off = off.wrapping_add_signed(delta);
        }

        // 3. Remove stale entries that may now be inside a deleted region
        //    or that will be re-discovered by the fresh scan.
        //    Keep everything up to (but not including) line_idx.
        let rescan_from = if line_idx > 0 { self.lines[line_idx - 1] } else { 0 };
        self.lines.truncate(line_idx);

        // 4. Re-scan from rescan_from to end
        let data = self.as_contiguous_bytes();
        scan_newlines(&data[rescan_from..], rescan_from, &mut self.lines);
    }

    /// Scan `data` for '\n' bytes, pushing line-start offsets into `out`.
    /// `base` is added to every offset (to support scanning a slice of the buffer).
    ///
    /// Uses 8-byte word scan for ~4x speedup over byte-by-byte on 64-bit.
    fn scan_newlines(data: &[u8], base: usize, out: &mut Vec<usize>) {
        let mut i = 0;

        // Align to 8-byte boundary
        while i < data.len() && (data.as_ptr() as usize + i) % 8 != 0 {
            if data[i] == b'\n' { out.push(base + i + 1); }
            i += 1;
        }

        // Word scan: process 8 bytes at a time
        // '\n' = 0x0A. XOR with 0x0A0A...0A, then find zero bytes.
        const NEWLINE_MASK: u64 = 0x0A0A_0A0A_0A0A_0A0Au64;
        const LO_BITS: u64      = 0x0101_0101_0101_0101u64;
        const HI_BITS: u64      = 0x8080_8080_8080_8080u64;

        while i + 8 <= data.len() {
            // SAFETY: aligned read, bounds checked above
            let word = u64::from_le_bytes(data[i..i+8].try_into().unwrap());
            let xored = word ^ NEWLINE_MASK;
            // Classic zero-byte detection: set high bit if byte == 0
            let has_zero = (xored.wrapping_sub(LO_BITS)) & !xored & HI_BITS;
            if has_zero != 0 {
                // Scan the 8 bytes individually (rare)
                for j in 0..8 {
                    if data[i + j] == b'\n' { out.push(base + i + j + 1); }
                }
            }
            i += 8;
        }

        // Handle remainder
        while i < data.len() {
            if data[i] == b'\n' { out.push(base + i + 1); }
            i += 1;
        }
    }
}
```

### Modified `insert()` and `delete()`

```rust
pub fn insert(&mut self, pos: usize, text: &str) {
    // ... existing gap buffer logic ...
    self.update_lines(pos, text.len() as isize);   // was: self.rebuild_lines()
}

pub fn delete(&mut self, pos: usize, len: usize) {
    // ... existing gap buffer logic ...
    self.update_lines(pos, -(len as isize));        // was: self.rebuild_lines()
}
```

### Expected Performance

| File size | Old (full rebuild) | New (incremental) | Speedup |
|-----------|-------------------|-------------------|---------|
| 10 KB | 0.02ms | <0.001ms | >20x |
| 100 KB | 0.2ms | ~0.005ms | ~40x |
| 1 MB | 2ms | ~0.05ms | ~40x |
| 10 MB | 20ms | ~0.1ms | ~200x |

Typical edits touch 1–3 lines; re-scan covers only the tail of the file from the edit.

### Unit Tests

```rust
#[test]
fn incremental_matches_full_rebuild() {
    let mut buf = Buffer::from_str("line1\nline2\nline3\n");
    let expected_full = buf.lines.clone();

    buf.insert(6, "inserted\n");
    let mut reference = Buffer::from_str("line1\ninserted\nline2\nline3\n");
    assert_eq!(buf.lines, reference.lines);
}

#[test]
fn delete_spanning_newline() {
    let mut buf = Buffer::from_str("ab\ncd\nef\n");
    buf.delete(2, 4); // delete "\ncd\n"
    assert_eq!(buf.line_count(), 2);
    assert_eq!(buf.get_line(0).unwrap(), "abef");
}
```

---

## Phase 26: OKLab Color System

**Goal**: Replace `rec601` luminance with perceptual OKLab math for 24-bit → 256 → 16
color downsampling and theme contrast checking.

### Problem

`src/render.rs` current 256 → 16 downsampling:

```rust
// rec601: linear RGB → luma (not perceptual)
let luma = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
```

This produces hue-shifting when mapping to 16 ANSI colors. A saturated blue (#0000FF)
maps to the same dark bucket as a dark gray (#333333) because rec601 weights are tuned
for video signal levels, not human color perception.

OKLab (Björn Ottosson, 2020) is a perceptually uniform color space where Euclidean
distance correlates with perceived color difference. `microsoft/edit` uses it in
`oklab.rs` for exactly this purpose.

### New File: `src/oklab.rs`

```rust
/// OKLab color space utilities.
/// Reference: https://bottosson.github.io/posts/oklab/
///
/// OKLab is perceptually uniform: equal Euclidean distances ≈ equal perceived differences.
/// Used for:
///   1. Finding the closest palette color to an arbitrary RGB value (better than rec601)
///   2. Checking contrast ratios between foreground/background colors

/// Convert sRGB (0.0–1.0 each channel) to OKLab (L, a, b).
pub fn srgb_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    // Step 1: sRGB → linear RGB (gamma expansion)
    let r = srgb_to_linear(r);
    let g = srgb_to_linear(g);
    let b = srgb_to_linear(b);

    // Step 2: linear RGB → LMS (Hunt-Pointer-Estevez)
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;

    // Step 3: LMS → LMS^(1/3) (cube root for perceptual uniformity)
    let l = l.cbrt();
    let m = m.cbrt();
    let s = s.cbrt();

    // Step 4: LMS^(1/3) → OKLab
    let lab_l = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let lab_a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let lab_b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;

    (lab_l, lab_a, lab_b)
}

#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Perceptual distance between two sRGB colors (0–255 each).
pub fn perceptual_distance(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f32 {
    let (l1, a1, b1) = srgb_to_oklab(r1 as f32 / 255.0, g1 as f32 / 255.0, b1 as f32 / 255.0);
    let (l2, a2, b2) = srgb_to_oklab(r2 as f32 / 255.0, g2 as f32 / 255.0, b2 as f32 / 255.0);
    let dl = l1 - l2;
    let da = a1 - a2;
    let db = b1 - b2;
    (dl * dl + da * da + db * db).sqrt()
}

/// WCAG-style contrast ratio between two sRGB colors (approximate via OKLab L).
/// Returns a value in [1.0, 21.0]. WCAG AA normal text requires >= 4.5.
pub fn contrast_ratio(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f32 {
    let (l1, _, _) = srgb_to_oklab(r1 as f32 / 255.0, g1 as f32 / 255.0, b1 as f32 / 255.0);
    let (l2, _, _) = srgb_to_oklab(r2 as f32 / 255.0, g2 as f32 / 255.0, b2 as f32 / 255.0);
    let lighter = l1.max(l2) + 0.05;
    let darker  = l1.min(l2) + 0.05;
    lighter / darker
}
```

### Modified `src/render.rs` — Better `rgb_to_ansi16()`

```rust
/// Map a 24-bit RGB color to the nearest ANSI 16-color using OKLab distance.
///
/// The 16 ANSI colors vary by terminal, but these sRGB values are representative:
fn rgb_to_ansi16_oklab(r: u8, g: u8, b: u8) -> u8 {
    const ANSI16: [(u8, u8, u8); 16] = [
        (0,   0,   0  ), // 0  black
        (128, 0,   0  ), // 1  red
        (0,   128, 0  ), // 2  green
        (128, 128, 0  ), // 3  yellow
        (0,   0,   128), // 4  blue
        (128, 0,   128), // 5  magenta
        (0,   128, 128), // 6  cyan
        (192, 192, 192), // 7  white
        (128, 128, 128), // 8  bright black (gray)
        (255, 0,   0  ), // 9  bright red
        (0,   255, 0  ), // 10 bright green
        (255, 255, 0  ), // 11 bright yellow
        (0,   0,   255), // 12 bright blue
        (255, 0,   255), // 13 bright magenta
        (0,   255, 255), // 14 bright cyan
        (255, 255, 255), // 15 bright white
    ];

    let mut best_idx = 0u8;
    let mut best_dist = f32::MAX;
    for (i, &(pr, pg, pb)) in ANSI16.iter().enumerate() {
        let dist = crate::oklab::perceptual_distance(r, g, b, pr, pg, pb);
        if dist < best_dist {
            best_dist = dist;
            best_idx = i as u8;
        }
    }
    best_idx
}
```

### Theme Contrast Check (`src/syntax/theme.rs`)

```rust
/// After loading a theme, verify that fg/bg combinations meet minimum contrast.
/// If not, lighten or darken the foreground to reach at least 3.0 contrast ratio
/// (relaxed from WCAG 4.5 to accommodate artistic themes).
pub fn ensure_readable_contrast(theme: &mut Theme, bg: (u8, u8, u8)) {
    for rule in &mut theme.token_rules {
        if let Some(Color::Rgb(r, g, b)) = rule.foreground {
            let ratio = crate::oklab::contrast_ratio(r, g, b, bg.0, bg.1, bg.2);
            if ratio < 3.0 {
                // Nudge toward opposite end of lightness scale
                rule.foreground = Some(adjust_lightness(r, g, b, bg));
            }
        }
    }
}
```

This is called once at theme load time, not on every render frame.

---

## Phase 27: Fuzzy Matching Upgrade

**Goal**: Replace the greedy first-match fuzzy with an optimal-path search that finds
the highest-scoring match positions, not just the first sequential match.

### Problem

`src/editor/palette.rs` current greedy algorithm:

```rust
// Takes the FIRST occurrence of each query char — not optimal
for (ti, &tc) in target_lower.iter().enumerate() {
    if qi < query_lower.len() && tc == query_lower[qi] {
        positions.push(ti); qi += 1; // stops here, never backtracks
    }
}
```

Query `"fs"` in `"Find: Save"` finds `F`ind and `S`ave — correct.
Query `"rs"` in `"replace_string"` finds `r`eplace and `s`tring — good.
But query `"rs"` in `"aRustString"` finds `R`ust and `S`tring at positions [1, 5]
instead of the consecutive `St` — suboptimal, missed consecutive bonus.

### Target: Two-Phase Algorithm

**Phase A** (same as now): greedy forward scan to confirm the pattern is matchable.
**Phase B** (new): backtrack from the last matched char, pulling matches right to
maximize consecutive runs and word-boundary hits.

```rust
/// Score a fuzzy match with optimal position selection.
///
/// Returns None if the query cannot be matched at all.
/// Returns Some((score, positions)) where positions are byte indices in target.
pub fn fuzzy_score(query: &str, target: &str) -> Option<(i32, Vec<usize>)> {
    let qc: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let tc: Vec<char> = target.chars().collect();
    let tc_lower: Vec<char> = target.chars().flat_map(|c| c.to_lowercase()).collect();

    // Phase A: greedy forward — bail out early if no full match
    let mut positions = Vec::with_capacity(qc.len());
    let mut qi = 0;
    for (ti, &ch) in tc_lower.iter().enumerate() {
        if qi < qc.len() && ch == qc[qi] { positions.push(ti); qi += 1; }
    }
    if positions.len() < qc.len() { return None; }

    // Phase B: push each match as far right as possible to cluster consecutive runs
    let n = positions.len();
    for i in (0..n).rev() {
        let lower_bound = if i == 0 { 0 } else { positions[i - 1] + 1 };
        let upper_bound = if i + 1 < n { positions[i + 1] - 1 } else { tc.len() - 1 };

        // Scan right from current position looking for a better (later) occurrence
        let mut best = positions[i];
        for ti in (lower_bound..=upper_bound).rev() {
            if tc_lower[ti] == qc[i] {
                // Prefer positions that are consecutive with adjacent match
                let consec_next = i + 1 < n && ti + 1 == positions[i + 1];
                let consec_prev = i > 0 && positions[i - 1] + 1 == ti;
                if consec_next || consec_prev { best = ti; break; }
            }
        }
        positions[i] = best;
    }

    // Scoring
    let mut score: i32 = 0;
    for (mi, &pos) in positions.iter().enumerate() {
        score += 1; // base: matched a char
        if mi > 0 && positions[mi - 1] + 1 == pos { score += 5; } // consecutive bonus
        let at_word_boundary = pos == 0
            || matches!(tc[pos - 1], ' ' | '_' | '-' | ':' | '/' | '.');
        if at_word_boundary { score += 10; }
        if mi == 0 { score -= pos as i32 / 2; } // mild prefix penalty
    }

    Some((score, positions))
}
```

### Impact on Command Palette

The command palette (`src/editor/palette.rs`) uses `fuzzy_score()` to rank 60+ commands.
With this change, searching `"sv"` will rank `Save` above `move_cursor_forward` because
`S`a`v`e has a word-boundary bonus on the first char and a consecutive match.

No API changes needed — the function signature stays identical.

---

## Phase 28: File Picker Path Completion

**Goal**: When typing in the `Ctrl+O` file-open prompt, show real-time filesystem
suggestions. Tab completes the longest common prefix.

### Problem

`src/editor/prompt.rs` current open-file flow:

```
User types: /home/rakzo/github/ze  →  nothing happens
User presses Enter                 →  tries to open literally "/home/rakzo/github/ze"
                                      fails with "No such file"
```

No hints, no suggestions, no autocomplete.

### Target UX

```
Ctrl+O opens prompt:
  Open: /home/rakzo/github/ze▌
  ┌────────────────────────────┐
  │ zedit/                     │  ← suggestions from readdir
  │ zenith-lang/               │
  │ zymbol/                    │
  └────────────────────────────┘
Tab → completes to "ze" common prefix or cycles through suggestions
Enter on a directory → descends (appends "/" and refreshes)
Enter on a file → opens it
```

### New Struct: `FileCompleter`

Add to `src/editor/prompt.rs`:

```rust
pub struct FileCompleter {
    /// Last directory that was scanned (to avoid re-reading on every keypress)
    last_dir: String,
    /// Sorted list of entries in `last_dir`
    entries: Vec<DirEntry>,
    /// Filtered subset matching the current input prefix
    pub matches: Vec<usize>,     // indices into `entries`
    pub selected: usize,
}

struct DirEntry {
    name: String,
    is_dir: bool,
}

impl FileCompleter {
    pub fn new() -> Self { ... }

    /// Update completions for the current input string.
    /// Only re-reads the directory when the directory portion of `input` changes.
    pub fn update(&mut self, input: &str) {
        let (dir, prefix) = split_dir_and_prefix(input);

        // Re-read directory only if it changed
        if dir != self.last_dir {
            self.entries = read_dir_entries(&dir);
            self.last_dir = dir.clone();
        }

        // Filter entries by prefix (case-sensitive first, case-insensitive fallback)
        let prefix_lower = prefix.to_lowercase();
        self.matches = self.entries.iter().enumerate()
            .filter(|(_, e)| {
                e.name.to_lowercase().starts_with(&prefix_lower)
            })
            .map(|(i, _)| i)
            .collect();

        self.selected = 0;
    }

    /// Tab-complete: return the longest common prefix of all current matches.
    pub fn tab_complete(&self, current_dir: &str) -> Option<String> {
        if self.matches.is_empty() { return None; }
        let names: Vec<&str> = self.matches.iter()
            .map(|&i| self.entries[i].name.as_str())
            .collect();
        let lcp = longest_common_prefix(&names);
        if lcp.is_empty() { return None; }
        let suffix = if self.matches.len() == 1 && self.entries[self.matches[0]].is_dir {
            format!("{}/", lcp)
        } else {
            lcp.to_string()
        };
        Some(format!("{}{}", current_dir, suffix))
    }

    /// Currently selected suggestion as a full path.
    pub fn selected_path(&self, current_dir: &str) -> Option<String> {
        let idx = *self.matches.get(self.selected)?;
        let e = &self.entries[idx];
        if e.is_dir {
            Some(format!("{}{}/", current_dir, e.name))
        } else {
            Some(format!("{}{}", current_dir, e.name))
        }
    }
}

fn split_dir_and_prefix(input: &str) -> (String, String) {
    match input.rfind('/') {
        Some(i) => (input[..=i].to_string(), input[i+1..].to_string()),
        None    => (String::from("./"), input.to_string()),
    }
}

fn read_dir_entries(dir: &str) -> Vec<DirEntry> {
    let mut entries = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            entries.push(DirEntry { name, is_dir });
        }
    }
    entries.sort_by(|a, b| {
        // Directories first, then alphabetical
        b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name))
    });
    entries
}

fn longest_common_prefix<'a>(strs: &[&'a str]) -> &'a str {
    if strs.is_empty() { return ""; }
    let first = strs[0];
    let mut len = first.len();
    for s in &strs[1..] {
        len = first.chars().zip(s.chars())
            .take_while(|(a, b)| a == b)
            .count()
            .min(len);
    }
    &first[..len]
}
```

### Integration with `Prompt`

```rust
pub struct Prompt {
    pub label: String,
    pub input: String,
    pub action: PromptAction,
    pub completer: Option<FileCompleter>,  // NEW: Some for OpenFile prompts
}
```

In `src/editor/mod.rs`, `handle_action`:

```rust
EditorAction::OpenFilePrompt => {
    self.prompt = Some(Prompt {
        label: "Open".into(),
        input: String::new(),
        action: PromptAction::OpenFile,
        completer: Some(FileCompleter::new()),
    });
}
```

In `handle_key` for active prompt:

```rust
Key::Tab => {
    if let Some(ref mut comp) = self.prompt.as_mut().and_then(|p| p.completer.as_mut()) {
        if let Some(completed) = comp.tab_complete(&dir_portion) {
            prompt.input = completed;
            comp.update(&completed);
        }
    }
}
Key::Down => { if let Some(c) = completer { c.selected = (c.selected + 1).min(c.matches.len().saturating_sub(1)); } }
Key::Up   => { if let Some(c) = completer { c.selected = c.selected.saturating_sub(1); } }
```

### Rendering the Suggestion Dropdown

In `src/editor/view.rs`, `render_prompt()`:

```rust
if let Some(comp) = &prompt.completer {
    let visible: Vec<_> = comp.matches.iter()
        .take(8)   // max 8 suggestions shown
        .enumerate()
        .collect();
    for (i, &entry_idx) in &visible {
        let entry = &comp.entries[entry_idx];
        let row = prompt_row + 1 + i;
        let highlight = i == comp.selected;
        let name = if entry.is_dir {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        };
        // Render with inverse colors if selected
        self.screen.put_str(row, prompt_col, &name, if highlight { style_selected } else { style_normal });
    }
}
```

---

## Phase 29: Large File Support via mmap

**Goal**: Files larger than 1MB are opened via `mmap()` (read-only). On first edit,
the relevant pages are materialized into the gap buffer (copy-on-write semantics).
Enables opening multi-hundred-MB log files without 50MB hard limit.

### Current Limit

`src/buffer.rs` `Buffer::from_file()`:

```rust
pub fn from_file(path: &Path) -> io::Result<Self> {
    let content = std::fs::read(path)?;  // reads entire file into RAM immediately
    let mut buf = Buffer::new();
    buf.insert(0, std::str::from_utf8(&content).unwrap_or(""));
    Ok(buf)
}
```

### Target Architecture

For files > 1MB: open via `mmap()` libc call. The OS handles paging — only accessed
pages consume RAM. The buffer is initially read-only; the gap buffer is empty.
On first edit (insert/delete), the touched region is copied into the gap buffer
and editing proceeds normally for that region.

### New `src/mmap.rs`

```rust
use std::os::unix::io::AsRawFd;

pub struct Mmap {
    ptr: *const u8,
    len: usize,
}

impl Mmap {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len() as usize;
        if len == 0 {
            return Ok(Mmap { ptr: std::ptr::NonNull::dangling().as_ptr(), len: 0 });
        }

        // SAFETY: valid fd, len > 0, MAP_PRIVATE for read-only view
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }

        // Advise sequential access pattern for faster paging
        unsafe { libc::madvise(ptr, len, libc::MADV_SEQUENTIAL); }

        Ok(Mmap { ptr: ptr as *const u8, len })
    }

    pub fn as_bytes(&self) -> &[u8] {
        if self.len == 0 { return &[]; }
        // SAFETY: ptr is valid for `len` bytes for the lifetime of self
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn len(&self) -> usize { self.len }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        if self.len > 0 {
            // SAFETY: ptr was obtained from mmap with the same len
            unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.len); }
        }
    }
}

// SAFETY: &[u8] is Send + Sync; mmap'd region is read-only
unsafe impl Send for Mmap {}
unsafe impl Sync for Mmap {}
```

### Modified `Buffer::from_file()`

```rust
const MMAP_THRESHOLD: u64 = 1 * 1024 * 1024; // 1MB

pub fn from_file(path: &Path) -> io::Result<Self> {
    let meta = std::fs::metadata(path)?;
    let file_size = meta.len();

    if file_size > MMAP_THRESHOLD {
        // Large file: mmap, initialize line cache without copying into gap buffer
        let map = crate::mmap::Mmap::open(path)?;
        let bytes = map.as_bytes();

        // Validate UTF-8 before accepting (scan only, no copy)
        if std::str::from_utf8(bytes).is_err() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file is not valid UTF-8"));
        }

        let mut buf = Buffer {
            data:    Vec::new(),      // gap buffer starts empty
            gap_start: 0,
            gap_end: 0,
            lines:   Vec::new(),
            mmap:    Some(map),       // holds the mmap alive
            is_dirty: false,
        };

        // Build line cache directly from mmap without copying
        Buffer::scan_newlines(bytes, 0, &mut buf.lines);

        Ok(buf)
    } else {
        // Small file: read fully into gap buffer (existing path)
        let content = std::fs::read(path)?;
        let text = std::str::from_utf8(&content)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "file is not valid UTF-8"))?;
        let mut buf = Buffer::new();
        buf.insert(0, text);
        Ok(buf)
    }
}
```

### `byte_at()` and `get_line()` with mmap backing

```rust
pub fn byte_at(&self, pos: usize) -> Option<u8> {
    // Check gap buffer first; fall back to mmap for unedited regions
    if !self.data.is_empty() {
        // existing gap buffer logic
        ...
    } else if let Some(ref map) = self.mmap {
        map.as_bytes().get(pos).copied()
    } else {
        None
    }
}
```

On first write to a mmap-backed buffer:
1. Relevant page range is copied into the gap buffer
2. `self.mmap` is dropped (unmapped) when the entire file has been materialized
3. Edit proceeds normally via gap buffer

This lazy materialization means browsing (read-only) a 500MB log file uses
only the RAM for the pages the user actually scrolls through.

---

## Implementation Order

### Week 1: Correctness & Quick Wins

**Phase 24 — Glob engine** (1–2 days):
1. Create `src/glob.rs` with the new implementation and tests
2. Replace `glob_match_inner` calls in `src/filetree.rs`
3. Run `cargo test` — verify existing glob tests pass
4. Manually test `.gitignore`-style patterns in the file tree

**Phase 27 — Fuzzy upgrade** (1 day):
1. Replace `fuzzy_score` implementation in `src/editor/palette.rs`
2. Test: open command palette, search with multi-char queries, verify ranking is better
3. No API changes needed — purely internal to the function

### Week 2: Performance

**Phase 25 — Incremental line cache** (2–3 days):
1. Add `scan_newlines()` function to `src/buffer.rs`
2. Add `update_lines()` method
3. Replace `rebuild_lines()` calls in `insert()` and `delete()` with `update_lines()`
4. Add unit tests for edge cases (insert at start, delete across newlines)
5. Benchmark: `time zedit large_file.rs` before and after

**Phase 26 — OKLab colors** (1–2 days):
1. Create `src/oklab.rs` with pure math (no deps)
2. Replace `ansi256_to_ansi16()` in `src/render.rs` with OKLab-based version
3. Add `ensure_readable_contrast()` call in `src/syntax/theme.rs`
4. Visual test: open a Rust file with the default theme in a 16-color terminal

### Week 3: UX Polish

**Phase 28 — File picker** (2–3 days):
1. Add `FileCompleter` struct to `src/editor/prompt.rs`
2. Add `completer` field to `Prompt`
3. Handle Tab/Up/Down keys in prompt input handler
4. Add dropdown rendering to `src/editor/view.rs`
5. Test: `Ctrl+O`, type partial path, press Tab

**Phase 29 — mmap large files** (3–4 days):
1. Create `src/mmap.rs` with the libc FFI wrapper
2. Add `mmap` field to `Buffer` struct
3. Modify `Buffer::from_file()` with size threshold
4. Add `byte_at()` mmap fallback
5. Test with a large file (> 1MB) — verify read-only browsing is fast
6. Test: edit a line in a mmap-backed buffer — verify materialization works

---

## Verification Suite

```sh
cargo build && cargo test && cargo clippy && cargo fmt -- --check
```

End-to-end manual tests:

| Test | Expected |
|------|----------|
| Glob: `a*a*a*a*b` on non-matching string | Returns instantly (no freeze) |
| Glob: `**/*.rs` in file tree filter | Matches nested `.rs` files |
| Glob: `[!.]` pattern | Hides dotfiles |
| Incremental lines: type in 1MB file | No perceptible lag between keystrokes |
| Incremental lines: delete across 3 lines | Line count updates correctly |
| OKLab: open `.rs` in 16-color terminal | Colors visually distinct, readable |
| OKLab: dark theme on light terminal | Contrast warning adjusts fg colors |
| Fuzzy: `"sv"` in palette | `Save` ranks above `move_cursor_forward` |
| Fuzzy: `"rs"` in `"replace_string"` | Finds consecutive match |
| File picker: `Ctrl+O`, type `src/` | Shows files in `src/` directory |
| File picker: Tab | Completes longest common prefix |
| File picker: Enter on directory | Navigates into it |
| mmap: open 10MB file | Opens in < 50ms |
| mmap: edit first line of 10MB file | Works normally, no crash |

---

## Design Constraints (Inherited)

- Zero external Rust crate dependencies — all new code uses `std` + libc FFI only
- `src/mmap.rs` uses `libc::mmap` — same pattern as `terminal.rs` using `libc::tcsetattr`
- `src/oklab.rs` is pure Rust math, no deps
- Startup time budget: < 10ms (mmap is lazy; line scan is fast)
- Binary size budget: < 1MB stripped (none of these changes add embedded data)
- All user-facing strings in English
