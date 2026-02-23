use crate::cursor::Cursor;
use crate::undo::{GroupContext, Operation};

use super::*;

impl Editor {
    pub(super) fn start_or_continue_selection(&mut self) {
        let b = self.buf();
        if b.selection().is_none() {
            let offset = b.cursor().byte_offset(&b.buffer);
            self.buf_mut().set_selection(Some(Selection {
                anchor: offset,
                head: offset,
            }));
        }
    }

    pub(super) fn extend_selection(&mut self) {
        let b = self.buf();
        let head = b.cursor().byte_offset(&b.buffer);
        if let Some(mut sel) = self.buf().selection() {
            sel.head = head;
            self.buf_mut().set_selection(Some(sel));
        }
    }

    pub(super) fn selection_range(&self) -> Option<(usize, usize)> {
        self.buf().selection().map(|sel| {
            let start = sel.anchor.min(sel.head);
            let end = sel.anchor.max(sel.head);
            (start, end)
        })
    }

    /// Delete the selected text, reposition cursor to selection start, clear selection.
    /// Returns the deleted text if there was a selection.
    pub(super) fn delete_selection(&mut self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        if start == end {
            self.buf_mut().set_selection(None);
            return None;
        }
        let before = self.cursor_state();
        let b = self.buf_mut();
        let deleted = b.buffer.slice(start, end);
        b.buffer.delete(start, end - start);
        b.undo_stack.record(
            Operation::Delete {
                pos: start,
                text: deleted.clone(),
            },
            before,
            GroupContext::Other,
        );
        // Reposition cursor to selection start
        let line = b.buffer.byte_to_line(start);
        let line_start = b.buffer.line_start(line).unwrap_or(0);
        let col = start - line_start;
        b.cursors[b.primary]
            .cursor
            .set_position(line, col, &b.buffer);
        b.set_selection(None);
        self.invalidate_highlight();
        Some(deleted)
    }

    pub(super) fn copy_selection(&mut self) {
        if let Some((start, end)) = self.selection_range() {
            if start == end {
                // No selection: copy current line
                self.copy_current_line();
                return;
            }
            let text = self.buf().buffer.slice(start, end);
            let len = text.chars().count();
            crate::clipboard::set(&text);
            self.sys_clip_set(&text);
            self.clipboard.set_text(text);
            self.set_message(&format!("Copied {} chars", len), MessageType::Info);
        } else {
            // No selection: copy current line
            self.copy_current_line();
        }
    }

    fn copy_current_line(&mut self) {
        let b = self.buf();
        let line_text = b.buffer.get_line(b.cursor().line).unwrap_or_default();
        let text = format!("{}\n", line_text);
        let len = line_text.chars().count();
        crate::clipboard::set(&text);
        self.sys_clip_set(&text);
        self.clipboard.set_line(text);
        self.set_message(&format!("Copied line ({} chars)", len), MessageType::Info);
    }

    pub(super) fn cut_selection(&mut self) {
        if let Some((start, end)) = self.selection_range() {
            if start == end {
                self.cut_current_line();
                return;
            }
            let text = self.delete_selection().unwrap_or_default();
            let len = text.chars().count();
            crate::clipboard::set(&text);
            self.sys_clip_set(&text);
            self.clipboard.set_text(text);
            self.set_message(&format!("Cut {} chars", len), MessageType::Info);
        } else {
            self.cut_current_line();
        }
    }

    fn cut_current_line(&mut self) {
        let before = self.cursor_state();
        let b = self.buf();
        let line = b.cursor().line;
        let line_start = b.buffer.line_start(line).unwrap_or(0);
        let line_end = b.buffer.line_end(line).unwrap_or(0);
        // Include the newline if not the last line
        let end = if line + 1 < b.buffer.line_count() {
            line_end + 1
        } else {
            line_end
        };
        let b = self.buf_mut();
        let text = b.buffer.slice(line_start, end);
        let len = text.chars().count();
        b.buffer.delete(line_start, end - line_start);
        b.undo_stack.record(
            Operation::Delete {
                pos: line_start,
                text: text.clone(),
            },
            before,
            GroupContext::Cut,
        );
        b.cursors[b.primary].cursor.clamp(&b.buffer);
        b.cursors[b.primary].cursor.col = 0;
        b.cursors[b.primary].cursor.desired_col = 0;
        crate::clipboard::set(&text);
        self.sys_clip_set(&text);
        self.clipboard.set_line(text);
        self.set_message(&format!("Cut line ({} chars)", len), MessageType::Info);
    }

    pub(super) fn paste_clipboard(&mut self) {
        // Prefer system clipboard; fall back to internal clipboard.
        let sys = self.sys_clip_get().filter(|s| !s.is_empty());
        let text = match sys {
            Some(t) => t,
            None => {
                if self.clipboard.is_empty() {
                    self.set_message("Clipboard is empty", MessageType::Warning);
                    return;
                }
                self.clipboard.text()
            }
        };
        // Delete selection if active
        self.delete_selection();
        self.handle_paste(&text);
    }

    pub(super) fn select_all(&mut self) {
        let len = self.buf().buffer.len();
        self.buf_mut().set_selection(Some(Selection {
            anchor: 0,
            head: len,
        }));
        let b = self.buf_mut();
        b.cursors[b.primary].cursor.move_to_end(&b.buffer);
    }

    // -----------------------------------------------------------------------
    // Multi-cursor: select next occurrence (Ctrl+D)
    // -----------------------------------------------------------------------

    pub(super) fn select_next_occurrence(&mut self) {
        let b = self.buf();

        // If no selection, select word under cursor first
        if b.selection().is_none() {
            self.select_word_under_cursor();
            return;
        }

        // Get selected text
        let (start, end) = match self.selection_range() {
            Some(range) if range.0 != range.1 => range,
            _ => return,
        };
        let selected_text = self.buf().buffer.slice(start, end);
        if selected_text.is_empty() {
            return;
        }

        // Find all matches
        let full_text = self.buf().buffer.text();
        let matches = find_all_matches(&full_text, &selected_text, &SearchMode::Substring);
        if matches.is_empty() {
            return;
        }

        // Find existing cursor positions to avoid duplicates
        let existing_positions: Vec<usize> = self
            .buf()
            .cursors
            .iter()
            .filter_map(|cs| cs.selection.map(|s| s.anchor.min(s.head)))
            .collect();

        // Find next match after the last cursor's selection
        let last_end = {
            let b = self.buf();
            let mut max_end = 0usize;
            for cs in &b.cursors {
                if let Some(sel) = cs.selection {
                    let e = sel.anchor.max(sel.head);
                    if e > max_end {
                        max_end = e;
                    }
                }
            }
            max_end
        };

        // Search forward from last_end, then wrap
        let next_match = matches
            .iter()
            .find(|(s, _)| *s >= last_end && !existing_positions.contains(s))
            .or_else(|| {
                matches
                    .iter()
                    .find(|(s, _)| !existing_positions.contains(s))
            });

        match next_match {
            Some(&(m_start, m_end)) => {
                // Add a new cursor at this match
                let b = self.buf_mut();
                let line = b.buffer.byte_to_line(m_end);
                let line_start = b.buffer.line_start(line).unwrap_or(0);
                let col = m_end - line_start;
                let mut new_cursor = Cursor::new();
                new_cursor.set_position(line, col, &b.buffer);
                b.cursors.push(CursorSelection {
                    cursor: new_cursor,
                    selection: Some(Selection {
                        anchor: m_start,
                        head: m_end,
                    }),
                });
                b.sort_and_merge();
                let count = b.cursors.len();
                self.set_message(&format!("{} cursors", count), MessageType::Info);
            }
            None => {
                self.set_message("All occurrences selected", MessageType::Info);
            }
        }
    }

    /// Select all occurrences of the current selection (Ctrl+Shift+L).
    pub(super) fn select_all_occurrences(&mut self) {
        // If no selection, select word under cursor first
        if self.buf().selection().is_none() {
            self.select_word_under_cursor();
        }

        let (start, end) = match self.selection_range() {
            Some(range) if range.0 != range.1 => range,
            _ => return,
        };
        let selected_text = self.buf().buffer.slice(start, end);
        if selected_text.is_empty() {
            return;
        }

        let full_text = self.buf().buffer.text();
        let matches = find_all_matches(&full_text, &selected_text, &SearchMode::Substring);
        if matches.is_empty() {
            return;
        }

        // Build cursors for all matches
        let b = self.buf_mut();
        let mut new_cursors = Vec::with_capacity(matches.len());
        let mut primary_idx = 0;
        for (i, &(m_start, m_end)) in matches.iter().enumerate() {
            let line = b.buffer.byte_to_line(m_end);
            let line_start = b.buffer.line_start(line).unwrap_or(0);
            let col = m_end - line_start;
            let mut cursor = Cursor::new();
            cursor.set_position(line, col, &b.buffer);
            new_cursors.push(CursorSelection {
                cursor,
                selection: Some(Selection {
                    anchor: m_start,
                    head: m_end,
                }),
            });
            // Primary is the one containing the original selection start
            if m_start == start {
                primary_idx = i;
            }
        }
        let count = new_cursors.len();
        b.cursors = new_cursors;
        b.primary = primary_idx;
        self.set_message(
            &format!("{} occurrences selected", count),
            MessageType::Info,
        );
    }

    /// Select the word under the primary cursor.
    fn select_word_under_cursor(&mut self) {
        let b = self.buf();
        let line_text = b.buffer.get_line(b.cursor().line).unwrap_or_default();
        let col = b.cursor().col;
        let bytes = line_text.as_bytes();

        if bytes.is_empty() || col >= bytes.len() && col > 0 {
            return;
        }

        let col = col.min(bytes.len().saturating_sub(1));

        // Check if cursor is on a word character
        if !is_word_byte(bytes[col]) {
            return;
        }

        // Find word boundaries
        let mut word_start = col;
        while word_start > 0 && is_word_byte(bytes[word_start - 1]) {
            word_start -= 1;
        }
        let mut word_end = col;
        while word_end < bytes.len() && is_word_byte(bytes[word_end]) {
            word_end += 1;
        }

        let line_start_byte = b.buffer.line_start(b.cursor().line).unwrap_or(0);
        let anchor = line_start_byte + word_start;
        let head = line_start_byte + word_end;

        let b = self.buf_mut();
        b.set_selection(Some(Selection { anchor, head }));
        // Move cursor to end of word
        let line = b.cursor().line;
        b.cursors[b.primary]
            .cursor
            .set_position(line, word_end, &b.buffer);
    }

    /// Add a new cursor at (line, byte_col) — for Alt+Click.
    pub(super) fn add_cursor_at(&mut self, line: usize, col: usize) {
        let b = self.buf_mut();
        let mut new_cursor = Cursor::new();
        new_cursor.set_position(line, col, &b.buffer);
        b.cursors.push(CursorSelection {
            cursor: new_cursor,
            selection: None,
        });
        b.sort_and_merge();
        let count = b.cursors.len();
        self.set_message(&format!("{} cursors", count), MessageType::Info);
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
