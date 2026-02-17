use crate::input::{MouseButton, MouseEvent};
use crate::syntax::highlight;
use crate::undo::{GroupContext, Operation};

use super::*;

impl Editor {
    // -----------------------------------------------------------------------
    // Basic text operations
    // -----------------------------------------------------------------------

    pub(super) fn insert_char(&mut self, ch: char) {
        let before = self.cursor_state();
        let b = self.buf();
        let pos = b.cursor.byte_offset(&b.buffer);
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let s_owned = s.to_string();
        let b = self.buf_mut();
        b.buffer.insert(pos, &s_owned);
        b.undo_stack.record(
            Operation::Insert { pos, text: s_owned },
            before,
            GroupContext::Typing,
        );
        b.cursor.move_right(&b.buffer);
        self.invalidate_highlight();
    }

    pub(super) fn insert_newline(&mut self) {
        let before = self.cursor_state();
        let b = self.buf();
        let pos = b.cursor.byte_offset(&b.buffer);

        let insert_text = if self.config().auto_indent {
            // Auto-indent: capture leading whitespace from current line
            let indent = b
                .buffer
                .get_line(b.cursor.line)
                .unwrap_or_default()
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>();
            format!("\n{}", indent)
        } else {
            "\n".to_string()
        };

        let b = self.buf_mut();
        b.buffer.insert(pos, &insert_text);
        b.undo_stack.record(
            Operation::Insert {
                pos,
                text: insert_text.clone(),
            },
            before,
            GroupContext::Other,
        );
        // Move past \n + indent chars
        for _ in insert_text.chars() {
            b.cursor.move_right(&b.buffer);
        }
        self.invalidate_highlight();
    }

    pub(super) fn insert_tab(&mut self) {
        let use_spaces = self.config().use_spaces;
        let tab_size = self.config().tab_size;
        let insert_text = if use_spaces {
            " ".repeat(tab_size)
        } else {
            "\t".to_string()
        };
        let before = self.cursor_state();
        let b = self.buf();
        let pos = b.cursor.byte_offset(&b.buffer);
        let b = self.buf_mut();
        b.buffer.insert(pos, &insert_text);
        b.undo_stack.record(
            Operation::Insert {
                pos,
                text: insert_text.clone(),
            },
            before,
            GroupContext::Other,
        );
        for _ in insert_text.chars() {
            b.cursor.move_right(&b.buffer);
        }
        self.invalidate_highlight();
    }

    pub(super) fn backspace(&mut self) {
        let b = self.buf();
        let pos = b.cursor.byte_offset(&b.buffer);
        if pos == 0 {
            return;
        }
        let before = self.cursor_state();
        let b = self.buf_mut();
        // Move cursor left first (handles UTF-8 boundaries)
        b.cursor.move_left(&b.buffer);
        let new_pos = b.cursor.byte_offset(&b.buffer);
        let delete_len = pos - new_pos;
        let deleted = b.buffer.slice(new_pos, pos);
        b.buffer.delete(new_pos, delete_len);
        b.undo_stack.record(
            Operation::Delete {
                pos: new_pos,
                text: deleted,
            },
            before,
            GroupContext::Deleting,
        );
        self.invalidate_highlight();
    }

    pub(super) fn delete_at_cursor(&mut self) {
        let b = self.buf();
        let pos = b.cursor.byte_offset(&b.buffer);
        if pos >= b.buffer.len() {
            return;
        }
        // Find the length of the character at cursor position
        if let Some(ch) = self.buf().buffer.char_at(pos) {
            let before = self.cursor_state();
            let char_len = ch.len_utf8();
            let b = self.buf_mut();
            let deleted = b.buffer.slice(pos, pos + char_len);
            b.buffer.delete(pos, char_len);
            b.undo_stack.record(
                Operation::Delete { pos, text: deleted },
                before,
                GroupContext::Deleting,
            );
            b.cursor.clamp(&b.buffer);
            self.invalidate_highlight();
        }
    }

    // -----------------------------------------------------------------------
    // Line operations
    // -----------------------------------------------------------------------

    pub(super) fn duplicate_line(&mut self) {
        let b = self.buf();
        let line = b.cursor.line;
        let line_text = b.buffer.get_line(line).unwrap_or_default();
        let line_end = b.buffer.line_end(line).unwrap_or(0);
        let insert_text = format!("\n{}", line_text);
        let before = self.cursor_state();
        let b = self.buf_mut();
        b.buffer.insert(line_end, &insert_text);
        b.undo_stack.record(
            Operation::Insert {
                pos: line_end,
                text: insert_text,
            },
            before,
            GroupContext::Other,
        );
        // Move cursor to duplicated line
        b.cursor.move_down(&b.buffer);
        self.invalidate_highlight();
        self.set_message("Line duplicated", MessageType::Info);
    }

    pub(super) fn delete_line(&mut self) {
        let before = self.cursor_state();
        let b = self.buf();
        let line = b.cursor.line;
        let line_start = b.buffer.line_start(line).unwrap_or(0);
        let line_end = b.buffer.line_end(line).unwrap_or(0);
        let line_count = b.buffer.line_count();
        // Include the newline if not the last line
        let end = if line + 1 < line_count {
            line_end + 1
        } else {
            line_end
        };
        if line_start == end {
            return; // empty buffer, nothing to delete
        }
        let b = self.buf_mut();
        let text = b.buffer.slice(line_start, end);
        b.buffer.delete(line_start, end - line_start);
        b.undo_stack.record(
            Operation::Delete {
                pos: line_start,
                text,
            },
            before,
            GroupContext::Other,
        );
        b.cursor.clamp(&b.buffer);
        b.cursor.col = 0;
        b.cursor.desired_col = 0;
        self.invalidate_highlight();
        self.set_message("Line deleted", MessageType::Info);
    }

    pub(super) fn unindent(&mut self) {
        if let Some((start, end)) = self.selection_range() {
            let b = self.buf();
            let start_line = b.buffer.byte_to_line(start);
            let end_line = b.buffer.byte_to_line(if end > 0 { end - 1 } else { 0 });
            for line in (start_line..=end_line).rev() {
                self.unindent_line(line);
            }
        } else {
            let line = self.buf().cursor.line;
            self.unindent_line(line);
        }
    }

    fn unindent_line(&mut self, line: usize) {
        let b = self.buf();
        let line_text = b.buffer.get_line(line).unwrap_or_default();
        let line_start = b.buffer.line_start(line).unwrap_or(0);
        // Count leading spaces (up to tab_size) or 1 tab
        let tab_size = self.config().tab_size;
        let remove_len = if line_text.starts_with('\t') {
            1
        } else {
            let spaces = line_text.bytes().take_while(|b| *b == b' ').count();
            spaces.min(tab_size)
        };
        if remove_len == 0 {
            return;
        }
        let before = self.cursor_state();
        let b = self.buf_mut();
        let removed = b.buffer.slice(line_start, line_start + remove_len);
        b.buffer.delete(line_start, remove_len);
        b.undo_stack.record(
            Operation::Delete {
                pos: line_start,
                text: removed,
            },
            before,
            GroupContext::Other,
        );
        // Adjust cursor if on this line
        if b.cursor.line == line {
            b.cursor.col = b.cursor.col.saturating_sub(remove_len);
            b.cursor.desired_col = b.cursor.col;
        }
        self.invalidate_highlight();
    }

    pub(super) fn select_line(&mut self) {
        let b = self.buf();
        let line = b.cursor.line;
        let line_start = b.buffer.line_start(line).unwrap_or(0);
        let line_end = if line + 1 < b.buffer.line_count() {
            b.buffer.line_start(line + 1).unwrap_or(b.buffer.len())
        } else {
            b.buffer.len()
        };
        self.buf_mut().selection = Some(Selection {
            anchor: line_start,
            head: line_end,
        });
        // Move cursor to end of selection
        self.jump_to_byte(line_end);
    }

    pub(super) fn toggle_comment(&mut self) {
        let prefix = match self.buf().highlighter.as_ref().and_then(|h| h.language()) {
            Some(lang) => match highlight::comment_prefix(lang, &self.config.languages) {
                Some(p) => p,
                None => {
                    self.set_message("No comment style for this language", MessageType::Warning);
                    return;
                }
            },
            None => {
                self.set_message("No language detected", MessageType::Warning);
                return;
            }
        };

        let (start_line, end_line) = if let Some((start, end)) = self.selection_range() {
            let b = self.buf();
            let sl = b.buffer.byte_to_line(start);
            let el = b.buffer.byte_to_line(if end > 0 { end - 1 } else { 0 });
            (sl, el)
        } else {
            let line = self.buf().cursor.line;
            (line, line)
        };

        let comment_with_space = format!("{} ", prefix);

        // Determine action: if all lines are commented, uncomment; else comment
        let all_commented = (start_line..=end_line).all(|line| {
            let text = self.buf().buffer.get_line(line).unwrap_or_default();
            let trimmed = text.trim_start();
            trimmed.starts_with(&comment_with_space) || trimmed.starts_with(prefix.as_str())
        });

        // Apply in reverse order to preserve byte offsets
        for line in (start_line..=end_line).rev() {
            let line_text = self.buf().buffer.get_line(line).unwrap_or_default();
            let line_start = self.buf().buffer.line_start(line).unwrap_or(0);
            let first_non_ws = line_text
                .bytes()
                .position(|b| b != b' ' && b != b'\t')
                .unwrap_or(0);

            if all_commented {
                // Uncomment: remove prefix + optional space
                let after_ws = &line_text[first_non_ws..];
                let remove_len = if after_ws.starts_with(&comment_with_space) {
                    comment_with_space.len()
                } else if after_ws.starts_with(prefix.as_str()) {
                    prefix.len()
                } else {
                    continue;
                };
                let before = self.cursor_state();
                let pos = line_start + first_non_ws;
                let b = self.buf_mut();
                let removed = b.buffer.slice(pos, pos + remove_len);
                b.buffer.delete(pos, remove_len);
                b.undo_stack.record(
                    Operation::Delete { pos, text: removed },
                    before,
                    GroupContext::Other,
                );
            } else {
                // Comment: insert prefix + space at first non-whitespace
                let before = self.cursor_state();
                let pos = line_start + first_non_ws;
                let b = self.buf_mut();
                b.buffer.insert(pos, &comment_with_space);
                b.undo_stack.record(
                    Operation::Insert {
                        pos,
                        text: comment_with_space.clone(),
                    },
                    before,
                    GroupContext::Other,
                );
            }
        }

        let b = self.buf_mut();
        b.cursor.clamp(&b.buffer);
        self.invalidate_highlight();
    }

    // -----------------------------------------------------------------------
    // Commands
    // -----------------------------------------------------------------------

    pub(super) fn save(&mut self) {
        if self.buf().buffer.file_path().is_none() {
            self.start_prompt("Save as: ", PromptAction::SaveAs);
            return;
        }
        match self.buf().buffer.save() {
            Ok(()) => {
                let cs = self.cursor_state();
                let b = self.buf_mut();
                b.buffer.mark_saved();
                b.undo_stack.mark_saved(cs);
                self.set_message("Saved!", MessageType::Info);
            }
            Err(e) => {
                self.set_message(&format!("Save failed: {}", e), MessageType::Error);
            }
        }
    }

    pub(super) fn quit(&mut self) {
        // Check all buffers for unsaved changes
        let any_modified = self.buffers.iter().any(|b| b.buffer.is_modified());
        if any_modified && !self.quit_confirm {
            self.quit_confirm = true;
            self.set_message(
                "Unsaved changes! Press Ctrl+Q again to quit without saving.",
                MessageType::Warning,
            );
            return;
        }
        self.running = false;
    }

    // -----------------------------------------------------------------------
    // Multi-buffer commands
    // -----------------------------------------------------------------------

    pub(super) fn new_buffer(&mut self) {
        self.buffers
            .push(BufferState::new_empty(self.config.line_numbers));
        self.active_buffer = self.buffers.len() - 1;
        self.set_message("New buffer", MessageType::Info);
    }

    pub(super) fn close_buffer(&mut self) {
        if self.buf().buffer.is_modified() && !self.quit_confirm {
            self.quit_confirm = true;
            self.set_message(
                "Unsaved changes! Press Ctrl+W again to close without saving.",
                MessageType::Warning,
            );
            return;
        }
        self.quit_confirm = false;

        if self.buffers.len() == 1 {
            // Last buffer — just reset to empty
            self.buffers[0] = BufferState::new_empty(self.config.line_numbers);
            self.active_buffer = 0;
            self.set_message("Buffer closed", MessageType::Info);
            return;
        }

        self.buffers.remove(self.active_buffer);
        if self.active_buffer >= self.buffers.len() {
            self.active_buffer = self.buffers.len() - 1;
        }
        self.set_message("Buffer closed", MessageType::Info);
    }

    pub(super) fn next_buffer(&mut self) {
        if self.buffers.len() > 1 {
            self.active_buffer = (self.active_buffer + 1) % self.buffers.len();
            let name = self
                .buf()
                .buffer
                .file_path()
                .map(shorten_path)
                .unwrap_or_else(|| "[No Name]".to_string());
            self.set_message(&format!("Buffer: {}", name), MessageType::Info);
        }
    }

    pub(super) fn prev_buffer(&mut self) {
        if self.buffers.len() > 1 {
            if self.active_buffer == 0 {
                self.active_buffer = self.buffers.len() - 1;
            } else {
                self.active_buffer -= 1;
            }
            let name = self
                .buf()
                .buffer
                .file_path()
                .map(shorten_path)
                .unwrap_or_else(|| "[No Name]".to_string());
            self.set_message(&format!("Buffer: {}", name), MessageType::Info);
        }
    }

    // -----------------------------------------------------------------------
    // Mouse
    // -----------------------------------------------------------------------

    pub(super) fn handle_mouse(&mut self, me: MouseEvent) {
        match me.button {
            MouseButton::Left => {
                if me.motion {
                    // Drag motion: extend selection
                    if self.mouse_dragging
                        && let Some((line, col)) = self.screen_to_buffer(me.col, me.row)
                    {
                        let b = self.buf_mut();
                        b.cursor.set_position(line, col, &b.buffer);
                        let head = b.cursor.byte_offset(&b.buffer);
                        if let Some(ref mut sel) = b.selection {
                            sel.head = head;
                        }
                    }
                } else if me.pressed {
                    // Click: set cursor, start selection anchor
                    if let Some((line, col)) = self.screen_to_buffer(me.col, me.row) {
                        let b = self.buf_mut();
                        b.cursor.set_position(line, col, &b.buffer);
                        let offset = b.cursor.byte_offset(&b.buffer);
                        b.selection = Some(Selection {
                            anchor: offset,
                            head: offset,
                        });
                        self.mouse_dragging = true;
                    }
                } else {
                    // Release
                    self.mouse_dragging = false;
                    // Clear selection if anchor == head (just a click, no drag)
                    if let Some(sel) = self.buf().selection
                        && sel.anchor == sel.head
                    {
                        self.buf_mut().selection = None;
                    }
                }
            }
            MouseButton::ScrollUp => {
                self.buf_mut().scroll_row = self.buf().scroll_row.saturating_sub(3);
                // Clamp cursor to visible area
                let h = self.text_area_height();
                let b = self.buf_mut();
                if b.cursor.line >= b.scroll_row + h {
                    let target = b.scroll_row + h - 1;
                    let col = b.cursor.col;
                    b.cursor.set_position(target, col, &b.buffer);
                }
            }
            MouseButton::ScrollDown => {
                let max_scroll = self.buf().buffer.line_count().saturating_sub(1);
                self.buf_mut().scroll_row = (self.buf().scroll_row + 3).min(max_scroll);
                // Clamp cursor to visible area
                let b = self.buf_mut();
                if b.cursor.line < b.scroll_row {
                    let target = b.scroll_row;
                    let col = b.cursor.col;
                    b.cursor.set_position(target, col, &b.buffer);
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Paste
    // -----------------------------------------------------------------------

    pub(super) fn handle_paste(&mut self, text: &str) {
        let before = self.cursor_state();
        let b = self.buf();
        let pos = b.cursor.byte_offset(&b.buffer);
        let b = self.buf_mut();
        b.buffer.insert(pos, text);
        b.undo_stack.record(
            Operation::Insert {
                pos,
                text: text.to_string(),
            },
            before,
            GroupContext::Paste,
        );
        // Advance cursor past inserted text
        for _ in text.chars() {
            b.cursor.move_right(&b.buffer);
        }
        self.invalidate_highlight();
    }

    // -----------------------------------------------------------------------
    // Undo/highlight helpers
    // -----------------------------------------------------------------------

    pub(super) fn cursor_state(&self) -> CursorState {
        let b = self.buf();
        CursorState {
            line: b.cursor.line,
            col: b.cursor.col,
            desired_col: b.cursor.desired_col,
        }
    }

    pub(super) fn invalidate_highlight(&mut self) {
        let cursor_line = self.buf().cursor.line;
        if let Some(h) = &mut self.buf_mut().highlighter {
            h.invalidate_from(cursor_line);
        }
    }
}
