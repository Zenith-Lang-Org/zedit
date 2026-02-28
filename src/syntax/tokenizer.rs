/// Stateful line-by-line tokenizer for TextMate grammars.
/// Produces `ScopeToken` spans by matching grammar patterns against each line,
/// carrying `LineState` across lines for multi-line constructs (strings, comments).
use std::cell::RefCell;
use std::collections::HashMap;
use super::grammar::{Grammar, IncludeTarget, Pattern};
use super::regex::{Captures, Regex};

// Thread-local cache of compiled end-pattern regexes.
// End patterns are short strings (e.g. `"`, `-->`, heredoc delimiters).
// Caching avoids recompiling the same pattern on every line / every pos.
thread_local! {
    static END_REGEX_CACHE: RefCell<HashMap<String, Regex>> =
        RefCell::new(HashMap::new());
}

// ── Public types ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeToken {
    pub start: usize,
    pub end: usize,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineState {
    pub stack: Vec<ActiveRegion>,
}

#[derive(Debug, Clone)]
pub struct ActiveRegion {
    pub scope_name: Option<String>,
    pub content_name: Option<String>,
    pub end_pattern: String,
    pub region_id: Option<usize>,
    /// Byte offset (in the current line) where the begin match ended.
    /// Used as the `\G` anchor when searching for the end pattern.
    /// For regions that carry over from a previous line this is the value
    /// from when the region was originally opened; since it refers to a
    /// different line it will never equal any `pos` on the new line,
    /// so `(?!\G)` will match immediately at position 0 — the correct
    /// behaviour (the region closes at the start of the continuation line).
    pub begin_end_pos: usize,
    /// Capture-level scopes applied to the end delimiter token.
    pub end_captures: Vec<(usize, String)>,
}

impl PartialEq for ActiveRegion {
    fn eq(&self, other: &Self) -> bool {
        self.scope_name == other.scope_name
            && self.end_pattern == other.end_pattern
            && self.region_id == other.region_id
    }
}

impl LineState {
    pub fn initial() -> Self {
        LineState { stack: Vec::new() }
    }
}

// ── Tokenizer ─────────────────────────────────────────────────

pub struct Tokenizer<'a> {
    grammar: &'a Grammar,
}

struct MatchCandidate {
    pos: usize,
    len: usize,
    kind: MatchKind,
}

enum MatchKind {
    Match {
        name: Option<String>,
        captures: Option<Captures>,
        capture_scopes: Vec<(usize, String)>,
    },
    RegionBegin {
        name: Option<String>,
        content_name: Option<String>,
        end_pattern_raw: String,
        region_id: usize,
        captures: Option<Captures>,
        capture_scopes: Vec<(usize, String)>,
        end_capture_scopes: Vec<(usize, String)>,
    },
}

impl<'a> Tokenizer<'a> {
    pub fn new(grammar: &'a Grammar) -> Self {
        Tokenizer { grammar }
    }

    pub fn tokenize_line(&self, line: &str, state: &LineState) -> (Vec<ScopeToken>, LineState) {
        let mut tokens: Vec<ScopeToken> = Vec::new();
        let mut stack = state.stack.clone();
        let mut pos = 0;
        // Cache the end-pattern search to avoid O(n²) rescanning for long lines.
        // When many child-pattern matches occur before the end delimiter (e.g. escape
        // sequences inside a 14 000-char JSON string), every iteration would rescan
        // from `pos` to find the same end `"`.  Instead we reuse the cached result
        // whenever the cached match is still ahead of the current position.
        let mut end_cache: Option<(super::regex::Match, Option<Captures>)> = None;
        let mut end_cache_depth: usize = 0;
        let mut end_cache_pat = String::new();

        while pos < line.len() {
            // If inside a region, try end pattern first
            if let Some(region) = stack.last() {
                let cur_depth = stack.len();
                let g_anchor = region.begin_end_pos;
                let end_result = if cur_depth == end_cache_depth
                    && region.end_pattern == end_cache_pat
                {
                    match &end_cache {
                        // Cached match is still ahead — reuse it.
                        Some((m, _)) if m.start >= pos => end_cache.clone(),
                        // Cache miss: cached end is behind us or absent.
                        _ => {
                            let r =
                                try_compile_and_match(&region.end_pattern, line, pos, g_anchor);
                            end_cache = r.clone();
                            r
                        }
                    }
                } else {
                    // Region changed — recompute and prime the cache.
                    end_cache_depth = cur_depth;
                    end_cache_pat = region.end_pattern.clone();
                    let r = try_compile_and_match(&region.end_pattern, line, pos, g_anchor);
                    end_cache = r.clone();
                    r
                };

                // Also find earliest child/pattern match
                let child_patterns = self.active_patterns(&stack);
                let child_match: Option<MatchCandidate> = if !child_patterns.is_empty() {
                    self.find_earliest_match(&child_patterns, line, pos, 0)
                } else {
                    None
                };

                // Check if any child match comes before end
                let child_before_end = match (&child_match, &end_result) {
                    (Some(c), Some((end_m, _))) => c.pos < end_m.start,
                    _ => false,
                };

                if child_before_end {
                    // Child pattern wins — fall through to process it below
                } else if let Some((end_m, end_caps)) = &end_result {
                    let end_start = end_m.start;
                    let end_end = end_m.end;

                    // Emit gap before end marker
                    if pos < end_start {
                        let scopes = build_scopes_from_stack(&stack, None);
                        tokens.push(ScopeToken {
                            start: pos,
                            end: end_start,
                            scopes,
                        });
                    }

                    // Emit end marker token (skip zero-width matches like (?!\G) ends).
                    let region = stack.pop().unwrap();
                    if end_start < end_end {
                        if !region.end_captures.is_empty() {
                            if let Some(caps) = end_caps {
                                emit_capture_tokens(
                                    &mut tokens,
                                    caps,
                                    &region.end_captures,
                                    &stack,
                                    region.scope_name.as_deref(),
                                    end_start,
                                    end_end,
                                );
                            } else {
                                let scopes =
                                    build_scopes_from_stack_with_popped(&stack, &region);
                                tokens.push(ScopeToken {
                                    start: end_start,
                                    end: end_end,
                                    scopes,
                                });
                            }
                        } else {
                            let scopes = build_scopes_from_stack_with_popped(&stack, &region);
                            tokens.push(ScopeToken {
                                start: end_start,
                                end: end_end,
                                scopes,
                            });
                        }
                    }

                    pos = end_end;
                    continue;
                } else if child_match.is_none() {
                    // No end match and no child match — rest of line is region content
                    let scopes = build_scopes_from_stack(&stack, None);
                    tokens.push(ScopeToken {
                        start: pos,
                        end: line.len(),
                        scopes,
                    });
                    pos = line.len();
                    continue;
                }
                // If we get here, child_match is Some and either won or there's no end match
            }

            // Find earliest matching pattern (top-level or child from region)
            let patterns = self.active_patterns(&stack);
            let candidate = self.find_earliest_match(&patterns, line, pos, 0);

            match candidate {
                Some(cand) => {
                    // Emit gap before match
                    if pos < cand.pos {
                        let scopes = build_scopes_from_stack(&stack, None);
                        tokens.push(ScopeToken {
                            start: pos,
                            end: cand.pos,
                            scopes,
                        });
                    }

                    match cand.kind {
                        MatchKind::Match {
                            name,
                            captures,
                            capture_scopes,
                        } => {
                            if !capture_scopes.is_empty() {
                                if let Some(caps) = &captures {
                                    emit_capture_tokens(
                                        &mut tokens,
                                        caps,
                                        &capture_scopes,
                                        &stack,
                                        name.as_deref(),
                                        cand.pos,
                                        cand.pos + cand.len,
                                    );
                                } else {
                                    let scopes = build_scopes_from_stack(&stack, name.as_deref());
                                    tokens.push(ScopeToken {
                                        start: cand.pos,
                                        end: cand.pos + cand.len,
                                        scopes,
                                    });
                                }
                            } else {
                                let scopes = build_scopes_from_stack(&stack, name.as_deref());
                                tokens.push(ScopeToken {
                                    start: cand.pos,
                                    end: cand.pos + cand.len,
                                    scopes,
                                });
                            }
                            pos = cand.pos + cand.len;
                            // Zero-width match guard
                            if cand.len == 0 {
                                if pos < line.len() {
                                    let ch_len =
                                        line[pos..].chars().next().map_or(1, |c| c.len_utf8());
                                    let scopes = build_scopes_from_stack(&stack, None);
                                    tokens.push(ScopeToken {
                                        start: pos,
                                        end: pos + ch_len,
                                        scopes,
                                    });
                                    pos += ch_len;
                                } else {
                                    break;
                                }
                            }
                        }
                        MatchKind::RegionBegin {
                            name,
                            content_name,
                            end_pattern_raw,
                            region_id,
                            captures,
                            capture_scopes,
                            end_capture_scopes,
                        } => {
                            // Resolve end pattern with backref substitution
                            let resolved_end = match &captures {
                                Some(caps) => resolve_end_pattern(&end_pattern_raw, caps, line),
                                None => end_pattern_raw.clone(),
                            };

                            // Emit begin token
                            if !capture_scopes.is_empty() {
                                if let Some(caps) = &captures {
                                    emit_capture_tokens(
                                        &mut tokens,
                                        caps,
                                        &capture_scopes,
                                        &stack,
                                        name.as_deref(),
                                        cand.pos,
                                        cand.pos + cand.len,
                                    );
                                } else {
                                    let scopes = build_scopes_from_stack(&stack, name.as_deref());
                                    tokens.push(ScopeToken {
                                        start: cand.pos,
                                        end: cand.pos + cand.len,
                                        scopes,
                                    });
                                }
                            } else {
                                let scopes = build_scopes_from_stack(&stack, name.as_deref());
                                tokens.push(ScopeToken {
                                    start: cand.pos,
                                    end: cand.pos + cand.len,
                                    scopes,
                                });
                            }

                            let begin_end = cand.pos + cand.len;
                            stack.push(ActiveRegion {
                                scope_name: name,
                                content_name,
                                end_pattern: resolved_end,
                                region_id: Some(region_id),
                                begin_end_pos: begin_end,
                                end_captures: end_capture_scopes,
                            });
                            pos = begin_end;
                            // Zero-width guard
                            if cand.len == 0 {
                                if pos < line.len() {
                                    let ch_len =
                                        line[pos..].chars().next().map_or(1, |c| c.len_utf8());
                                    let scopes = build_scopes_from_stack(&stack, None);
                                    tokens.push(ScopeToken {
                                        start: pos,
                                        end: pos + ch_len,
                                        scopes,
                                    });
                                    pos += ch_len;
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
                None => {
                    // Rest of line is default scope
                    let scopes = build_scopes_from_stack(&stack, None);
                    tokens.push(ScopeToken {
                        start: pos,
                        end: line.len(),
                        scopes,
                    });
                    pos = line.len();
                }
            }
        }

        // Post-loop: close regions whose end pattern has a zero-width match at end-of-line.
        // This handles the TextMate `(?!\G)` idiom: the region stays open when its single
        // child token consumes the rest of the line, because the while loop above exits
        // before the end pattern is checked at pos == line.len().
        let eol = line.len();
        while !stack.is_empty() {
            let g_anchor = stack.last().unwrap().begin_end_pos;
            match try_compile_and_match(&stack.last().unwrap().end_pattern, line, eol, g_anchor) {
                Some((end_m, _)) if end_m.start == eol && end_m.end == eol => {
                    stack.pop();
                }
                _ => break,
            }
        }

        (tokens, LineState { stack })
    }

    /// Get the active patterns to scan — either from innermost region's children or top-level.
    fn active_patterns(&self, stack: &[ActiveRegion]) -> Vec<&Pattern> {
        if let Some(region) = stack.last() {
            if let Some(region_id) = region.region_id
                && let Some(children) = self.grammar.find_region_children(region_id)
            {
                return children.iter().collect();
            }
            // Region without id or children not found — nothing matches inside
            return Vec::new();
        }
        self.grammar.patterns.iter().collect()
    }

    fn find_earliest_match(
        &self,
        patterns: &[&Pattern],
        line: &str,
        pos: usize,
        depth: usize,
    ) -> Option<MatchCandidate> {
        if depth > 8 {
            return None; // Include cycle guard
        }

        let mut best: Option<MatchCandidate> = None;

        for pat in patterns {
            let candidate = match pat {
                Pattern::Match {
                    name,
                    regex,
                    captures,
                } => regex.find(line, pos).map(|m| MatchCandidate {
                    pos: m.start,
                    len: m.end - m.start,
                    kind: MatchKind::Match {
                        name: name.clone(),
                        captures: if captures.is_empty() {
                            None
                        } else {
                            regex.captures(line, pos)
                        },
                        capture_scopes: captures.clone(),
                    },
                }),
                Pattern::Region {
                    id,
                    name,
                    content_name,
                    begin,
                    end_pattern,
                    begin_captures,
                    end_captures,
                    patterns: _child_patterns,
                } => {
                    if let Some(m) = begin.find(line, pos) {
                        let caps = if begin_captures.is_empty() {
                            None
                        } else {
                            begin.captures(line, pos)
                        };
                        Some(MatchCandidate {
                            pos: m.start,
                            len: m.end - m.start,
                            kind: MatchKind::RegionBegin {
                                name: name.clone(),
                                content_name: content_name.clone(),
                                end_pattern_raw: end_pattern.clone(),
                                region_id: *id,
                                captures: caps,
                                capture_scopes: begin_captures.clone(),
                                end_capture_scopes: end_captures.clone(),
                            },
                        })
                    } else {
                        None
                    }
                }
                Pattern::Include(target) => {
                    let resolved = match target {
                        IncludeTarget::Repository(key) => self.grammar.find_repository(key),
                        IncludeTarget::SelfRef => Some(self.grammar.patterns.as_slice()),
                    };
                    if let Some(pats) = resolved {
                        let refs: Vec<&Pattern> = pats.iter().collect();
                        self.find_earliest_match(&refs, line, pos, depth + 1)
                    } else {
                        None
                    }
                }
            };

            if let Some(c) = candidate {
                let dominated = match &best {
                    Some(b) => c.pos < b.pos || (c.pos == b.pos && c.len > b.len),
                    None => true,
                };
                if dominated {
                    best = Some(c);
                }
            }
        }

        best
    }
}

// ── Helper functions ──────────────────────────────────────────

/// Push a TextMate `name` / `contentName` value into `scopes`.
///
/// In TextMate grammars a scope field may contain multiple space-separated
/// scope names (e.g. `"string.json support.type.property-name.json"`).
/// We split them so each scope is an independent entry that the theme
/// resolver can match individually.
#[inline]
fn push_scope(scopes: &mut Vec<String>, s: &str) {
    for part in s.split_whitespace() {
        scopes.push(part.to_string());
    }
}

fn build_scopes_from_stack(stack: &[ActiveRegion], leaf: Option<&str>) -> Vec<String> {
    let mut scopes = Vec::new();
    for region in stack {
        if let Some(s) = &region.scope_name {
            push_scope(&mut scopes, s);
        }
        if let Some(s) = &region.content_name {
            push_scope(&mut scopes, s);
        }
    }
    if let Some(leaf) = leaf {
        push_scope(&mut scopes, leaf);
    }
    scopes
}

fn build_scopes_from_stack_with_popped(
    stack: &[ActiveRegion],
    popped: &ActiveRegion,
) -> Vec<String> {
    let mut scopes = Vec::new();
    for region in stack {
        if let Some(s) = &region.scope_name {
            push_scope(&mut scopes, s);
        }
        if let Some(s) = &region.content_name {
            push_scope(&mut scopes, s);
        }
    }
    if let Some(s) = &popped.scope_name {
        push_scope(&mut scopes, s);
    }
    scopes
}

fn try_compile_and_match(
    pattern: &str,
    line: &str,
    pos: usize,
    g_anchor: usize,
) -> Option<(super::regex::Match, Option<Captures>)> {
    // Look up the compiled regex in the thread-local cache before compiling.
    // End patterns repeat across many lines (e.g. `"` for every JSON string),
    // so caching cuts Regex::new() calls from O(lines) to O(unique patterns).
    let re = END_REGEX_CACHE.with(|cache| -> Option<Regex> {
        let mut c = cache.borrow_mut();
        if let Some(existing) = c.get(pattern) {
            return Some(existing.clone());
        }
        let compiled = Regex::new(pattern).ok()?;
        c.insert(pattern.to_string(), compiled.clone());
        Some(compiled)
    })?;
    let m = re.find_with_anchor(line, pos, g_anchor)?;
    if m.start < pos {
        return None; // only accept matches at or after pos
    }
    let caps = re.captures_with_anchor(line, pos, g_anchor);
    Some((m, caps))
}

/// Substitute `\1`, `\2`, etc. in end_pattern with captured text from begin match.
fn resolve_end_pattern(raw: &str, captures: &Captures, text: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            let digit = (bytes[i + 1] - b'0') as usize;
            if let Some(Some(m)) = captures.groups.get(digit) {
                result.push_str(&regex_escape(&text[m.start..m.end]));
            }
            i += 2;
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if "\\.*+?()[]{}|^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn emit_capture_tokens(
    tokens: &mut Vec<ScopeToken>,
    captures: &Captures,
    capture_scopes: &[(usize, String)],
    stack: &[ActiveRegion],
    overall_name: Option<&str>,
    match_start: usize,
    match_end: usize,
) {
    // Collect capture regions that have scopes
    let mut regions: Vec<(usize, usize, &str)> = Vec::new();
    for (idx, scope) in capture_scopes {
        if *idx == 0 {
            // Group 0 scope is the overall name, handle via overall_name
            continue;
        }
        if let Some(Some(m)) = captures.groups.get(*idx)
            && m.start >= match_start
            && m.end <= match_end
        {
            regions.push((m.start, m.end, scope));
        }
    }
    regions.sort_by_key(|(start, _, _)| *start);

    // Check if there's a scope for group 0 in captures
    let cap0_scope = capture_scopes
        .iter()
        .find(|(idx, _)| *idx == 0)
        .map(|(_, s)| s.as_str())
        .or(overall_name);

    if regions.is_empty() {
        // No sub-captures, just emit the whole match
        let scopes = build_scopes_from_stack(stack, cap0_scope);
        tokens.push(ScopeToken {
            start: match_start,
            end: match_end,
            scopes,
        });
        return;
    }

    // Emit tokens with capture-level scopes
    let mut pos = match_start;
    for (cstart, cend, cscope) in &regions {
        if pos < *cstart {
            let scopes = build_scopes_from_stack(stack, cap0_scope);
            tokens.push(ScopeToken {
                start: pos,
                end: *cstart,
                scopes,
            });
        }
        let mut scopes = build_scopes_from_stack(stack, cap0_scope);
        push_scope(&mut scopes, cscope);
        tokens.push(ScopeToken {
            start: *cstart,
            end: *cend,
            scopes,
        });
        pos = *cend;
    }
    if pos < match_end {
        let scopes = build_scopes_from_stack(stack, cap0_scope);
        tokens.push(ScopeToken {
            start: pos,
            end: match_end,
            scopes,
        });
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::json_parser::JsonValue;

    fn make_grammar(json_str: &str) -> Grammar {
        let json = JsonValue::parse(json_str).unwrap();
        Grammar::from_json(&json).unwrap()
    }

    #[test]
    fn test_single_keyword() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "\\b(if|else|while)\\b", "name": "keyword.control"}
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);
        let (tokens, state) = t.tokenize_line("if true", &LineState::initial());
        assert!(state.stack.is_empty());
        // First token should be the keyword "if"
        assert_eq!(tokens[0].start, 0);
        assert_eq!(tokens[0].end, 2);
        assert!(tokens[0].scopes.contains(&"keyword.control".to_string()));
    }

    #[test]
    fn test_line_comment() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "//.*$", "name": "comment.line"}
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);
        let (tokens, _) = t.tokenize_line("x = 1; // note", &LineState::initial());
        // Should have gap + comment token
        let comment_tok = tokens
            .iter()
            .find(|t| t.scopes.contains(&"comment.line".to_string()));
        assert!(comment_tok.is_some());
        let ct = comment_tok.unwrap();
        assert_eq!(ct.start, 7);
        assert_eq!(ct.end, 14);
    }

    #[test]
    fn test_string_region() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"begin": "\"", "end": "\"", "name": "string.quoted"}
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);
        let (tokens, state) = t.tokenize_line("x = \"hello\"", &LineState::initial());
        assert!(state.stack.is_empty());
        // Should have: gap("x = "), begin("), content(hello), end(")
        let string_tokens: Vec<_> = tokens
            .iter()
            .filter(|t| t.scopes.contains(&"string.quoted".to_string()))
            .collect();
        assert!(!string_tokens.is_empty());
    }

    #[test]
    fn test_multiline_block_comment() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"begin": "/\\*", "end": "\\*/", "name": "comment.block"}
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);

        // Line 1: open comment
        let (tokens1, state1) = t.tokenize_line("x = 1; /* start", &LineState::initial());
        assert_eq!(state1.stack.len(), 1);
        assert_eq!(state1.stack[0].scope_name.as_deref(), Some("comment.block"));

        // Line 2: middle (all comment)
        let (tokens2, state2) = t.tokenize_line("  middle  ", &state1);
        assert_eq!(state2.stack.len(), 1); // still inside
        assert!(!tokens2.is_empty());

        // Line 3: close comment
        let (tokens3, state3) = t.tokenize_line("end */ y", &state2);
        assert!(state3.stack.is_empty()); // region closed
        let _ = tokens1;
        let _ = tokens3;
    }

    #[test]
    fn test_nested_pattern_in_region() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {
                        "begin": "\"",
                        "end": "\"",
                        "name": "string.quoted",
                        "patterns": [
                            {"match": "\\\\.", "name": "constant.character.escape"}
                        ]
                    }
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);

        // Test escape sequences inside strings get proper scope
        let (tokens, state) = t.tokenize_line(r#""hello\nworld""#, &LineState::initial());
        assert!(state.stack.is_empty());
        let escape_tok = tokens
            .iter()
            .find(|t| t.scopes.contains(&"constant.character.escape".to_string()));
        assert!(
            escape_tok.is_some(),
            "escape sequence should be highlighted"
        );
        let et = escape_tok.unwrap();
        assert_eq!(et.start, 6); // \n starts at byte 6
        assert_eq!(et.end, 8); // \n ends at byte 8
        // Should also have the parent string scope
        assert!(et.scopes.contains(&"string.quoted".to_string()));
    }

    #[test]
    fn test_keywords_not_matched_inside_string() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "\\b(if|else|while)\\b", "name": "keyword.control"},
                    {
                        "begin": "\"",
                        "end": "\"",
                        "name": "string.quoted",
                        "patterns": [
                            {"match": "\\\\.", "name": "constant.character.escape"}
                        ]
                    }
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);
        let (tokens, state) = t.tokenize_line(r#""if else while""#, &LineState::initial());
        assert!(state.stack.is_empty());
        // No token inside the string should have keyword.control scope
        let keyword_tok = tokens
            .iter()
            .find(|t| t.scopes.contains(&"keyword.control".to_string()));
        assert!(
            keyword_tok.is_none(),
            "keywords inside strings should NOT be highlighted"
        );
    }

    #[test]
    fn test_multiline_region_with_child_patterns() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {
                        "begin": "\"",
                        "end": "\"",
                        "name": "string.quoted",
                        "patterns": [
                            {"match": "\\\\.", "name": "constant.character.escape"}
                        ]
                    }
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);

        // Line 1: open string with escape
        let (tokens1, state1) = t.tokenize_line(r#""hello\n"#, &LineState::initial());
        assert_eq!(state1.stack.len(), 1); // still inside string
        let escape1 = tokens1
            .iter()
            .find(|t| t.scopes.contains(&"constant.character.escape".to_string()));
        assert!(escape1.is_some(), "escape on line 1 should be highlighted");

        // Line 2: continuation with escape and close
        let (tokens2, state2) = t.tokenize_line(r#"world\t""#, &state1);
        assert!(state2.stack.is_empty()); // string closed
        let escape2 = tokens2
            .iter()
            .find(|t| t.scopes.contains(&"constant.character.escape".to_string()));
        assert!(escape2.is_some(), "escape on line 2 should be highlighted");
    }

    #[test]
    fn test_region_with_no_child_patterns() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "\\b(if|else)\\b", "name": "keyword.control"},
                    {
                        "begin": "\"",
                        "end": "\"",
                        "name": "string.quoted"
                    }
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);
        let (tokens, state) = t.tokenize_line(r#""if else""#, &LineState::initial());
        assert!(state.stack.is_empty());
        // Nothing inside the string should match (no child patterns defined)
        let keyword_tok = tokens
            .iter()
            .find(|t| t.scopes.contains(&"keyword.control".to_string()));
        assert!(
            keyword_tok.is_none(),
            "keywords should NOT match inside region with no child patterns"
        );
        // String content should still have string.quoted scope
        let string_content = tokens
            .iter()
            .find(|t| t.start > 0 && t.end < 9 && t.scopes.contains(&"string.quoted".to_string()));
        assert!(string_content.is_some());
    }

    #[test]
    fn test_include_from_repository() {
        let g = make_grammar(
            r##"{
                "scopeName": "source.test",
                "patterns": [
                    {"include": "#keywords"}
                ],
                "repository": {
                    "keywords": {
                        "patterns": [
                            {"match": "\\b(let|const)\\b", "name": "keyword.declaration"}
                        ]
                    }
                }
            }"##,
        );
        let t = Tokenizer::new(&g);
        let (tokens, _) = t.tokenize_line("let x = 1", &LineState::initial());
        assert!(
            tokens[0]
                .scopes
                .contains(&"keyword.declaration".to_string())
        );
        assert_eq!(tokens[0].start, 0);
        assert_eq!(tokens[0].end, 3);
    }

    #[test]
    fn test_empty_line() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "\\w+", "name": "text"}
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);
        let (tokens, state) = t.tokenize_line("", &LineState::initial());
        assert!(tokens.is_empty());
        assert!(state.stack.is_empty());
    }

    #[test]
    fn test_line_state_equality() {
        let s1 = LineState {
            stack: vec![ActiveRegion {
                scope_name: Some("comment.block".to_string()),
                content_name: None,
                end_pattern: "\\*/".to_string(),
                region_id: None,
                begin_end_pos: 0,
                end_captures: vec![],
            }],
        };
        let s2 = LineState {
            stack: vec![ActiveRegion {
                scope_name: Some("comment.block".to_string()),
                content_name: None,
                end_pattern: "\\*/".to_string(),
                region_id: None,
                begin_end_pos: 0,
                end_captures: vec![],
            }],
        };
        let s3 = LineState {
            stack: vec![ActiveRegion {
                scope_name: Some("string.quoted".to_string()),
                content_name: None,
                end_pattern: "\"".to_string(),
                region_id: None,
                begin_end_pos: 0,
                end_captures: vec![],
            }],
        };
        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[test]
    fn test_first_match_wins() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "//.*$", "name": "comment.line"},
                    {"match": "\\w+", "name": "identifier"}
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);
        let (tokens, _) = t.tokenize_line("// hi", &LineState::initial());
        // The comment should win over the identifier
        assert!(tokens[0].scopes.contains(&"comment.line".to_string()));
    }

    /// Regression: compound keywords with hyphens (ELSE-IF, END-IF, …) must be
    /// tokenized as keyword.control, NOT as identifier.
    ///
    /// Root cause: `find_earliest_match` takes the LONGEST match at a given
    /// position.  When the keyword pattern tries alternatives left-to-right and
    /// matches `ELSE` (4 chars), but the identifier pattern `[a-zA-Z_][a-zA-Z0-9_-]*`
    /// matches the full `ELSE-IF` (7 chars), the identifier used to win.
    /// Fix: put compound keywords (`ELSE-IF`, `SINO-SI`, …) BEFORE their
    /// shorter prefix in the alternation so the keyword pattern also produces
    /// a 7-char match, tying the identifier — and the first (keyword) pattern wins.
    #[test]
    fn test_compound_keyword_beats_identifier() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "(?i)\\b(ELSE-IF|ELSE|IF|END-IF)\\b", "name": "keyword.control"},
                    {"match": "\\b[a-zA-Z_][a-zA-Z0-9_-]*\\b",      "name": "variable.other"}
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);

        // ELSE-IF must get keyword.control, not variable.other
        let (tokens, _) = t.tokenize_line("ELSE-IF x", &LineState::initial());
        let kw = tokens
            .iter()
            .find(|tok| tok.start == 0 && tok.end == 7)
            .expect("expected token at [0,7] for ELSE-IF");
        assert!(
            kw.scopes.contains(&"keyword.control".to_string()),
            "ELSE-IF should be keyword.control, got {:?}",
            kw.scopes
        );

        // Standalone ELSE still works
        let (tokens2, _) = t.tokenize_line("ELSE x", &LineState::initial());
        let kw2 = tokens2
            .iter()
            .find(|tok| tok.start == 0 && tok.end == 4)
            .expect("expected token at [0,4] for ELSE");
        assert!(kw2.scopes.contains(&"keyword.control".to_string()));

        // END-IF works (its prefix END is not in the alternation — no conflict)
        let (tokens3, _) = t.tokenize_line("END-IF", &LineState::initial());
        let kw3 = tokens3
            .iter()
            .find(|tok| tok.start == 0 && tok.end == 6)
            .expect("expected token at [0,6] for END-IF");
        assert!(kw3.scopes.contains(&"keyword.control".to_string()));
    }

    #[test]
    fn test_zero_width_match_no_infinite_loop() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "^", "name": "meta.start"}
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);
        let (tokens, _) = t.tokenize_line("abc", &LineState::initial());
        // Should not hang. We get some tokens and finish.
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_end_pattern_backref_resolution() {
        // Heredoc-style: begin captures delimiter, end uses \1
        let resolved = resolve_end_pattern(
            "\\1",
            &Captures {
                groups: vec![
                    Some(super::super::regex::Match { start: 0, end: 5 }),
                    Some(super::super::regex::Match { start: 2, end: 5 }),
                ],
            },
            "<<EOF",
        );
        assert_eq!(resolved, "EOF");
    }

    /// Regression: JSON number pattern uses (?x) extended mode.
    /// Before the fix, strip_extended_mode was not called and the whitespace/
    /// comment characters were compiled as literals → the pattern never matched.
    #[test]
    fn test_json_number_extended_mode() {
        // This is the exact (?x) number pattern from grammars/json.tmLanguage.json
        let json_number_pat = "(?x)        # turn on extended mode\n  -?        # an optional minus\n  (?:\n    0       # a zero\n    |       # ...or...\n    [1-9]   # a 1-9 character\n    \\d*     # followed by zero or more digits\n  )\n  (?:\n    (?:\n      \\.    # a period\n      \\d+   # followed by one or more digits\n    )?\n    (?:\n      [eE]  # an e character\n      [+-]? # followed by an option +/-\n      \\d+   # followed by one or more digits\n    )?      # make exponent optional\n  )?        # make decimal portion optional";
        let g = make_grammar(&format!(
            r#"{{
                "scopeName": "source.json",
                "patterns": [
                    {{"match": {}, "name": "constant.numeric.json"}},
                    {{"match": "\\btrue\\b", "name": "constant.language.json"}}
                ]
            }}"#,
            serde_json_string(json_number_pat)
        ));
        let t = Tokenizer::new(&g);

        // Integer
        let (tokens, _) = t.tokenize_line("42", &LineState::initial());
        let num_tok = tokens
            .iter()
            .find(|tok| tok.scopes.contains(&"constant.numeric.json".to_string()));
        assert!(num_tok.is_some(), "42 should match as constant.numeric.json");
        assert_eq!(num_tok.unwrap().start, 0);
        assert_eq!(num_tok.unwrap().end, 2);

        // Negative decimal
        let (tokens2, _) = t.tokenize_line("-3.14", &LineState::initial());
        let num_tok2 = tokens2
            .iter()
            .find(|tok| tok.scopes.contains(&"constant.numeric.json".to_string()));
        assert!(num_tok2.is_some(), "-3.14 should match as constant.numeric.json");

        // Boolean should NOT match number pattern
        let (tokens3, _) = t.tokenize_line("true", &LineState::initial());
        let lang_tok = tokens3
            .iter()
            .find(|tok| tok.scopes.contains(&"constant.language.json".to_string()));
        assert!(lang_tok.is_some(), "true should match constant.language.json");
        let num_in_bool = tokens3
            .iter()
            .any(|tok| tok.scopes.contains(&"constant.numeric.json".to_string()));
        assert!(!num_in_bool, "true should NOT match the number pattern");
    }

    /// JSON string escape pattern also uses (?x). Verify it compiles and matches.
    #[test]
    fn test_json_string_escape_extended_mode() {
        let escape_pat = "(?x)                # turn on extended mode\n  \\\\                # a literal backslash\n  (?:               # ...followed by...\n    [\"\\\\/bfnrt]     # one of these characters\n    |               # ...or...\n    u               # a u\n    [0-9a-fA-F]{4}) # and four hex digits";
        let g = make_grammar(&format!(
            r#"{{
                "scopeName": "source.json",
                "patterns": [
                    {{
                        "begin": "\"",
                        "end": "\"",
                        "name": "string.quoted.double.json",
                        "patterns": [
                            {{"match": {}, "name": "constant.character.escape.json"}}
                        ]
                    }}
                ]
            }}"#,
            serde_json_string(escape_pat)
        ));
        let t = Tokenizer::new(&g);

        let (tokens, state) = t.tokenize_line(r#""hello\nworld""#, &LineState::initial());
        assert!(state.stack.is_empty());
        let esc = tokens
            .iter()
            .find(|tok| tok.scopes.contains(&"constant.character.escape.json".to_string()));
        assert!(esc.is_some(), "\\n should be a constant.character.escape.json token");
    }

    /// Markdown heading pattern should produce markup.heading.markdown scope.
    #[test]
    fn test_markdown_heading_scope() {
        // Simplified version of the markdown heading pattern (from the grammar)
        let g = make_grammar(
            r##"{
                "scopeName": "text.html.markdown",
                "patterns": [
                    {
                        "match": "^(#{1,6})\\s+(.+?)$",
                        "name": "markup.heading.markdown",
                        "captures": {
                            "2": {"name": "entity.name.section.markdown"}
                        }
                    }
                ]
            }"##,
        );
        let t = Tokenizer::new(&g);

        let (tokens, _) = t.tokenize_line("## My Title", &LineState::initial());
        let heading_tok = tokens
            .iter()
            .find(|tok| tok.scopes.contains(&"markup.heading.markdown".to_string()));
        assert!(heading_tok.is_some(), "## should produce markup.heading.markdown");

        let section_tok = tokens
            .iter()
            .find(|tok| tok.scopes.contains(&"entity.name.section.markdown".to_string()));
        assert!(section_tok.is_some(), "heading text should have entity.name.section.markdown");
    }

    // Minimal JSON serializer for embedding a string literal inside a JSON grammar.
    fn serde_json_string(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for b in s.bytes() {
            match b {
                b'"' => out.push_str("\\\""),
                b'\\' => out.push_str("\\\\"),
                b'\n' => out.push_str("\\n"),
                b'\r' => out.push_str("\\r"),
                b'\t' => out.push_str("\\t"),
                _ => out.push(b as char),
            }
        }
        out.push('"');
        out
    }

    #[test]
    fn test_captures_with_numbered_groups() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {
                        "match": "(\\w+)\\.(\\w+)",
                        "captures": {
                            "1": {"name": "entity.name"},
                            "2": {"name": "support.function"}
                        }
                    }
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);
        let (tokens, _) = t.tokenize_line("foo.bar", &LineState::initial());
        // Should have capture-level tokens
        let has_entity = tokens
            .iter()
            .any(|t| t.scopes.contains(&"entity.name".to_string()));
        let has_function = tokens
            .iter()
            .any(|t| t.scopes.contains(&"support.function".to_string()));
        assert!(has_entity);
        assert!(has_function);
    }

    /// Regression: end-pattern cache must survive many child matches on a long line.
    ///
    /// A JSON-like grammar has a string region `"..."` with an escape child `\\.`.
    /// A line containing 200 escape sequences inside a string must produce the
    /// correct tokens and NOT hang (the cache avoids O(n²) rescanning).
    #[test]
    fn test_end_pattern_cache_long_line() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {
                        "begin": "\"",
                        "end": "\"",
                        "name": "string.quoted",
                        "patterns": [
                            {"match": "\\\\.", "name": "constant.character.escape"}
                        ]
                    }
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);

        // Build a string with 200 escape sequences: "\\a\\a\\a...\\a"
        let mut line = String::from("\"");
        for _ in 0..200 {
            line.push_str("\\a");
        }
        line.push('"');

        let (tokens, state) = t.tokenize_line(&line, &LineState::initial());
        // Region must be closed at end of line.
        assert!(state.stack.is_empty(), "string region should be closed");
        // All 200 escape sequences should be highlighted.
        let esc_count = tokens
            .iter()
            .filter(|tok| tok.scopes.contains(&"constant.character.escape".to_string()))
            .count();
        assert_eq!(esc_count, 200, "expected 200 escape tokens, got {}", esc_count);
    }

    /// Regression: `(?!\G)` end pattern must close the region immediately after
    /// the first child-pattern match — it must NOT leak to the next line.
    ///
    /// The R grammar uses this idiom for `$accessor` and `::namespace` regions:
    ///   begin: `(\$)(?=identifier)`  end: `(?!\G)`
    /// After the begin match at pos P, the child identifier is matched and the
    /// region must close before any subsequent characters.
    #[test]
    fn test_not_g_anchor_region_closes_after_child() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {
                        "begin": "(\\$)(?=[a-z])",
                        "beginCaptures": {"1": {"name": "punctuation.accessor"}},
                        "end": "(?!\\G)",
                        "patterns": [
                            {"match": "[a-z]+", "name": "entity.name.identifier"}
                        ]
                    }
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);

        // The identifier after $ should get entity.name.identifier scope.
        let (tokens, state) = t.tokenize_line("$foo", &LineState::initial());
        // Region must be CLOSED at end of line (no leak to next line).
        assert!(
            state.stack.is_empty(),
            "(?!\\G) region must close before end of line, stack={:?}",
            state.stack
        );
        // $ gets punctuation.accessor.
        let accessor_tok = tokens
            .iter()
            .find(|tok| tok.scopes.contains(&"punctuation.accessor".to_string()));
        assert!(accessor_tok.is_some(), "$ should be punctuation.accessor");
        // identifier gets entity.name.identifier.
        let ident_tok = tokens
            .iter()
            .find(|tok| tok.scopes.contains(&"entity.name.identifier".to_string()));
        assert!(ident_tok.is_some(), "foo should be entity.name.identifier");
        assert_eq!(ident_tok.unwrap().start, 1);
        assert_eq!(ident_tok.unwrap().end, 4);
    }

    /// Regression: `endCaptures` must be applied to the end delimiter token.
    #[test]
    fn test_end_captures_applied_to_closing_delimiter() {
        let g = make_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {
                        "begin": "\\(",
                        "beginCaptures": {"0": {"name": "punctuation.open"}},
                        "end": "\\)",
                        "endCaptures": {"0": {"name": "punctuation.close"}}
                    }
                ]
            }"#,
        );
        let t = Tokenizer::new(&g);

        let (tokens, state) = t.tokenize_line("(x)", &LineState::initial());
        assert!(state.stack.is_empty());

        // Opening ( must have punctuation.open.
        let open_tok = tokens
            .iter()
            .find(|tok| tok.start == 0 && tok.end == 1);
        assert!(open_tok.is_some());
        assert!(
            open_tok.unwrap().scopes.contains(&"punctuation.open".to_string()),
            "( should have punctuation.open, got {:?}",
            open_tok.unwrap().scopes
        );

        // Closing ) must have punctuation.close (endCaptures group 0).
        let close_tok = tokens
            .iter()
            .find(|tok| tok.start == 2 && tok.end == 3);
        assert!(close_tok.is_some(), "no token for closing )");
        assert!(
            close_tok.unwrap().scopes.contains(&"punctuation.close".to_string()),
            ") should have punctuation.close, got {:?}",
            close_tok.unwrap().scopes
        );
    }
}
