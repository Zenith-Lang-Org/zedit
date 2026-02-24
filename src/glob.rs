/// Iterative O(n×m) glob matching engine.
///
/// Supported patterns:
///   `*`       — zero or more chars (does NOT cross `/`)
///   `**`      — zero or more chars including `/` (crosses path segments)
///   `?`       — exactly one char (does NOT cross `/`)
///   `[abc]`   — character class (literal chars, case-sensitive)
///   `[a-z]`   — character range
///   `[!abc]`  — negated character class
///
/// All matching is case-sensitive and operates on raw bytes (UTF-8 safe for
/// ASCII patterns, which covers all practical glob use-cases).

/// Match a glob pattern against a path component or full path.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), path.as_bytes())
}

/// Check if `path` matches any pattern in the slice.
pub fn matches_any(patterns: &[&str], path: &str) -> bool {
    patterns.iter().any(|p| glob_match(p, path))
}

// ---------------------------------------------------------------------------
// Core iterative algorithm
// ---------------------------------------------------------------------------
//
// Two independent backtrack points are maintained:
//
//   star1 — most recent single `*`:  cannot consume `/`
//   star2 — most recent `**`:        can consume any character, including `/`
//
// When a single `*` exhausts its options (would need to consume `/`), control
// falls back to `star2`, which extends by one character and retries from there.
// This handles patterns like `**/*.rs` across arbitrary numbers of slashes.
//
// When `**` is followed by `/` in the pattern (e.g., `src/**/mod.rs`), the
// resume point is set to *after* the `/` so that the zero-segment case
// (`src/mod.rs`) is attempted first before letting `**` consume more path.

fn glob_match_inner(pat: &[u8], name: &[u8]) -> bool {
    let (mut pi, mut ni) = (0usize, 0usize);

    // Single-`*` backtrack: position after the `*` in pat, start of consumption.
    let (mut s1_pi, mut s1_ni) = (usize::MAX, 0usize);
    // Double-`**` backtrack: resume position in pat, start of consumption.
    let (mut s2_pi, mut s2_ni) = (usize::MAX, 0usize);

    'outer: while ni < name.len() {
        if pi < pat.len() {
            match pat[pi] {
                // ── `**` wildcard ─────────────────────────────────────────
                b'*' if pi + 1 < pat.len() && pat[pi + 1] == b'*' => {
                    // Resume after `**/` if present (handles zero-segment case:
                    // `src/**/foo` must match `src/foo` directly).
                    let resume = if pi + 2 < pat.len() && pat[pi + 2] == b'/' {
                        pi + 3
                    } else {
                        pi + 2
                    };
                    s2_pi = resume;
                    s2_ni = ni;
                    pi = resume;
                    s1_pi = usize::MAX; // ** subsumes any pending single *
                    continue 'outer;
                }

                // ── `*` wildcard ──────────────────────────────────────────
                b'*' => {
                    s1_pi = pi + 1;
                    s1_ni = ni;
                    pi = s1_pi;
                    continue 'outer;
                }

                // ── `?` matches any single char except `/` ─────────────────
                b'?' if name[ni] != b'/' => {
                    pi += 1;
                    ni += 1;
                    continue 'outer;
                }

                // ── character class `[...]` ────────────────────────────────
                b'[' => {
                    match match_class(&pat[pi..], name[ni]) {
                        Some((true, consumed)) => {
                            pi += consumed;
                            ni += 1;
                            continue 'outer;
                        }
                        Some((false, _)) => {} // didn't match → backtrack
                        None => return false,  // malformed class
                    }
                }

                // ── exact byte match ──────────────────────────────────────
                c if c == name[ni] => {
                    pi += 1;
                    ni += 1;
                    continue 'outer;
                }

                _ => {} // no match → backtrack
            }
        }

        // ── backtrack ─────────────────────────────────────────────────────
        if s1_pi != usize::MAX {
            s1_ni += 1;
            if name[s1_ni - 1] == b'/' {
                // Single `*` cannot cross `/`: discard it and try `**`.
                s1_pi = usize::MAX;
                if s2_pi != usize::MAX {
                    s2_ni += 1;
                    ni = s2_ni;
                    pi = s2_pi;
                    continue 'outer;
                }
                return false;
            }
            ni = s1_ni;
            pi = s1_pi;
        } else if s2_pi != usize::MAX {
            // `**` extends by one character (can cross `/`).
            s2_ni += 1;
            ni = s2_ni;
            pi = s2_pi;
        } else {
            return false;
        }
    }

    // Skip any trailing `*` / `**` (they match empty string).
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }

    pi == pat.len()
}

// ---------------------------------------------------------------------------
// Character class parser
// ---------------------------------------------------------------------------

/// Parse a `[class]` pattern starting at `pat` and test byte `b`.
///
/// Returns `Some((matched, bytes_consumed))` or `None` for a malformed class.
fn match_class(pat: &[u8], b: u8) -> Option<(bool, usize)> {
    if pat.first() != Some(&b'[') {
        return None;
    }
    let mut i = 1;
    let negate = i < pat.len() && pat[i] == b'!';
    if negate {
        i += 1;
    }

    let mut found = false;
    while i < pat.len() && pat[i] != b']' {
        // Range: `[a-z]`
        if i + 2 < pat.len() && pat[i + 1] == b'-' && pat[i + 2] != b']' {
            if b >= pat[i] && b <= pat[i + 2] {
                found = true;
            }
            i += 3;
        } else {
            if pat[i] == b {
                found = true;
            }
            i += 1;
        }
    }

    if i >= pat.len() {
        return None; // unclosed `[`
    }

    Some((found ^ negate, i + 1)) // i + 1 to include the closing `]`
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── basic matching ──────────────────────────────────────────────────────

    #[test]
    fn exact_match() {
        assert!(glob_match("foo.rs", "foo.rs"));
        assert!(!glob_match("foo.rs", "bar.rs"));
    }

    #[test]
    fn star_prefix() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(glob_match("*.rs", "lib.rs"));
    }

    #[test]
    fn star_does_not_cross_slash() {
        assert!(!glob_match("*.rs", "src/main.rs"));
    }

    #[test]
    fn star_matches_empty() {
        assert!(glob_match("*.rs", ".rs")); // zero chars before `.rs`
    }

    #[test]
    fn double_star_crosses_slash() {
        assert!(glob_match("**/*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "a/b/c/lib.rs"));
        assert!(!glob_match("**/*.rs", "src/main.txt"));
    }

    #[test]
    fn multi_segment_double_star() {
        assert!(glob_match("src/**/mod.rs", "src/editor/view/mod.rs"));
        assert!(glob_match("src/**/mod.rs", "src/mod.rs")); // zero segments
    }

    #[test]
    fn question_mark() {
        assert!(glob_match("foo?.rs", "foob.rs"));
        assert!(!glob_match("foo?.rs", "foo.rs")); // ? requires exactly one char
        assert!(!glob_match("foo?.rs", "foo/.rs")); // ? does not cross /
    }

    // ── character classes ───────────────────────────────────────────────────

    #[test]
    fn char_class_literal() {
        assert!(glob_match("[abc].rs", "b.rs"));
        assert!(!glob_match("[abc].rs", "d.rs"));
    }

    #[test]
    fn char_range() {
        assert!(glob_match("[a-z].rs", "m.rs"));
        assert!(!glob_match("[a-z].rs", "M.rs")); // case-sensitive
    }

    #[test]
    fn negated_class() {
        assert!(!glob_match("[!abc].rs", "a.rs"));
        assert!(glob_match("[!abc].rs", "d.rs"));
    }

    #[test]
    fn negated_range() {
        assert!(!glob_match("[!a-z].rs", "m.rs"));
        assert!(glob_match("[!a-z].rs", "M.rs"));
    }

    // ── performance / correctness ───────────────────────────────────────────

    #[test]
    fn no_exponential_blowup() {
        // This would hang for seconds with the old recursive O(2^m) algorithm.
        let pat = "a*a*a*a*a*a*a*b";
        let name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaac";
        assert!(!glob_match(pat, name)); // must return in microseconds
    }

    #[test]
    fn multiple_wildcards_match() {
        assert!(glob_match("s*c/**/m*.rs", "src/editor/main.rs"));
    }

    // ── practical ignore-list patterns ─────────────────────────────────────

    #[test]
    fn ignore_dotfiles() {
        assert!(glob_match(".*", ".gitignore"));
        assert!(glob_match(".*", ".DS_Store"));
        assert!(!glob_match(".*", "README.md"));
    }

    #[test]
    fn ignore_object_files() {
        assert!(glob_match("*.o", "main.o"));
        assert!(glob_match("*.so", "libfoo.so"));
        assert!(!glob_match("*.o", "main.rs"));
    }

    #[test]
    fn matches_any_helper() {
        let patterns = &["*.o", "*.pyc", ".git", "target"];
        assert!(matches_any(patterns, "main.o"));
        assert!(matches_any(patterns, "__pycache__.pyc"));
        assert!(matches_any(patterns, ".git"));
        assert!(matches_any(patterns, "target"));
        assert!(!matches_any(patterns, "main.rs"));
    }
}
