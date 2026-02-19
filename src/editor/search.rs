use crate::undo::{GroupContext, Operation};

use super::*;

// ---------------------------------------------------------------------------
// Search state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SearchMode {
    Substring,
    Regex,
}

pub(super) struct SearchState {
    pub(super) pattern: String,
    pub(super) matches: Vec<(usize, usize)>, // (byte_start, byte_end)
    pub(super) current: Option<usize>,       // index into matches
    pub(super) mode: SearchMode,
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

pub(super) fn find_all_matches(
    text: &str,
    pattern: &str,
    mode: &SearchMode,
) -> Vec<(usize, usize)> {
    match mode {
        SearchMode::Substring => find_all_matches_substring(text, pattern),
        SearchMode::Regex => find_all_matches_regex(text, pattern),
    }
}

/// Case-insensitive substring search. Returns non-overlapping byte ranges.
fn find_all_matches_substring(text: &str, pattern: &str) -> Vec<(usize, usize)> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let text_lower = text.to_lowercase();
    let pattern_lower = pattern.to_lowercase();
    let pat_len = pattern_lower.len();
    let mut results = Vec::new();
    let mut start = 0;
    while start + pat_len <= text_lower.len() {
        if let Some(pos) = text_lower[start..].find(&pattern_lower) {
            let abs_pos = start + pos;
            results.push((abs_pos, abs_pos + pat_len));
            start = abs_pos + pat_len; // non-overlapping
        } else {
            break;
        }
    }
    results
}

fn find_all_matches_regex(text: &str, pattern: &str) -> Vec<(usize, usize)> {
    use crate::syntax::regex::Regex;

    if pattern.is_empty() {
        return Vec::new();
    }
    let regex = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return Vec::new(), // Silent during incremental typing
    };
    let mut results = Vec::new();
    let mut start = 0;
    while start <= text.len() {
        match regex.find(text, start) {
            Some(m) if m.start < m.end => {
                results.push((m.start, m.end));
                start = m.end;
            }
            Some(m) => {
                // zero-length match — advance past one char
                start = text[m.end..]
                    .chars()
                    .next()
                    .map_or(text.len() + 1, |c| m.end + c.len_utf8());
            }
            None => break,
        }
    }
    results
}

// ---------------------------------------------------------------------------
// Editor methods
// ---------------------------------------------------------------------------

impl Editor {
    pub(super) fn open_find_prompt(&mut self, action: PromptAction) {
        // Pre-fill with selection text (if short, single-line) or last search pattern
        let prefill = self.prefill_search_text();
        let mode = self
            .buf()
            .search
            .as_ref()
            .map_or(SearchMode::Substring, |s| s.mode);
        let label = if mode == SearchMode::Regex {
            "Find (regex): "
        } else {
            "Find: "
        };
        self.prompt = Some(Prompt {
            label: label.to_string(),
            input: prefill.clone(),
            cursor_pos: prefill.len(),
            action,
        });
        self.message = None;
        // Trigger incremental search if prefill is non-empty
        if !prefill.is_empty() {
            self.update_search(&prefill);
        }
    }

    fn prefill_search_text(&self) -> String {
        // Use selection if it's short and single-line
        if let Some((start, end)) = self.selection_range()
            && start != end
        {
            let text = self.buf().buffer.slice(start, end);
            if !text.contains('\n') && text.len() <= 100 {
                return text;
            }
        }
        // Fall back to last search pattern
        if let Some(ref search) = self.buf().search {
            return search.pattern.clone();
        }
        String::new()
    }

    pub(super) fn update_search(&mut self, pattern: &str) {
        if pattern.is_empty() {
            self.buf_mut().search = None;
            return;
        }
        let mode = self
            .buf()
            .search
            .as_ref()
            .map_or(SearchMode::Substring, |s| s.mode);
        let text = self.buf().buffer.text();
        let matches = find_all_matches(&text, pattern, &mode);
        let b = self.buf();
        let cursor_byte = b.cursor().byte_offset(&b.buffer);

        // Find nearest match at or after cursor
        let current = if matches.is_empty() {
            None
        } else {
            let idx = matches
                .iter()
                .position(|(start, _)| *start >= cursor_byte)
                .unwrap_or(0);
            // Jump cursor to this match
            self.jump_to_byte(matches[idx].0);
            Some(idx)
        };

        self.buf_mut().search = Some(SearchState {
            pattern: pattern.to_string(),
            matches,
            current,
            mode,
        });
    }

    pub(super) fn search_next(&mut self) {
        let (total, next_idx, byte_pos) = {
            let search = match self.buf().search {
                Some(ref s) if !s.matches.is_empty() => s,
                _ => {
                    self.set_message("No search pattern", MessageType::Warning);
                    return;
                }
            };
            let total = search.matches.len();
            let next = match search.current {
                Some(i) => (i + 1) % total,
                None => 0,
            };
            (total, next, search.matches[next].0)
        };
        self.jump_to_byte(byte_pos);
        self.buf_mut().search.as_mut().unwrap().current = Some(next_idx);
        self.set_message(
            &format!("Match {} of {}", next_idx + 1, total),
            MessageType::Info,
        );
    }

    pub(super) fn search_prev(&mut self) {
        let (total, prev_idx, byte_pos) = {
            let search = match self.buf().search {
                Some(ref s) if !s.matches.is_empty() => s,
                _ => {
                    self.set_message("No search pattern", MessageType::Warning);
                    return;
                }
            };
            let total = search.matches.len();
            let prev = match search.current {
                Some(i) => {
                    if i == 0 {
                        total - 1
                    } else {
                        i - 1
                    }
                }
                None => total - 1,
            };
            (total, prev, search.matches[prev].0)
        };
        self.jump_to_byte(byte_pos);
        self.buf_mut().search.as_mut().unwrap().current = Some(prev_idx);
        self.set_message(
            &format!("Match {} of {}", prev_idx + 1, total),
            MessageType::Info,
        );
    }

    pub(super) fn jump_to_byte(&mut self, byte_pos: usize) {
        let b = self.buf();
        let line = b.buffer.byte_to_line(byte_pos);
        let line_start = b.buffer.line_start(line).unwrap_or(0);
        let col = byte_pos - line_start;
        let b = self.buf_mut();
        b.cursors[b.primary]
            .cursor
            .set_position(line, col, &b.buffer);
    }

    pub(super) fn execute_replace_all(&mut self, find_pattern: &str, replacement: &str) {
        let mode = self
            .buf()
            .search
            .as_ref()
            .map_or(SearchMode::Substring, |s| s.mode);
        let text = self.buf().buffer.text();
        let matches = find_all_matches(&text, find_pattern, &mode);
        if matches.is_empty() {
            self.set_message("No matches to replace", MessageType::Warning);
            return;
        }
        let count = matches.len();

        // Replace in reverse order to preserve byte offsets
        for &(start, end) in matches.iter().rev() {
            let before = self.cursor_state();
            let b = self.buf_mut();
            let deleted = b.buffer.slice(start, end);
            b.buffer.delete(start, end - start);
            b.undo_stack.record(
                Operation::Delete {
                    pos: start,
                    text: deleted,
                },
                before,
                GroupContext::Other,
            );
            let before2 = self.cursor_state();
            let b = self.buf_mut();
            b.buffer.insert(start, replacement);
            b.undo_stack.record(
                Operation::Insert {
                    pos: start,
                    text: replacement.to_string(),
                },
                before2,
                GroupContext::Other,
            );
        }

        // Clear search state after replace
        let b = self.buf_mut();
        b.search = None;
        b.cursors[b.primary].cursor.clamp(&b.buffer);
        self.set_message(
            &format!("Replaced {} occurrences", count),
            MessageType::Info,
        );
    }

    /// Check if a byte position falls within any search match.
    /// Returns Some(is_current_match) if in a match, None otherwise.
    pub(super) fn match_at_byte(&self, byte_pos: usize) -> Option<bool> {
        let search = self.buf().search.as_ref()?;
        for (i, &(start, end)) in search.matches.iter().enumerate() {
            if byte_pos >= start && byte_pos < end {
                let is_current = search.current == Some(i);
                return Some(is_current);
            }
            if start > byte_pos {
                break; // matches are sorted, no need to continue
            }
        }
        None
    }
}
