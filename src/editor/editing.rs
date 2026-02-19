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
        let pos = b.cursor().byte_offset(&b.buffer);
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
        b.cursors[b.primary].cursor.move_right(&b.buffer);
        self.invalidate_highlight();
        self.invalidate_git();
    }

    pub(super) fn insert_newline(&mut self) {
        let before = self.cursor_state();
        let b = self.buf();
        let pos = b.cursor().byte_offset(&b.buffer);

        let insert_text = if self.config().auto_indent {
            // Auto-indent: capture leading whitespace from current line
            let indent = b
                .buffer
                .get_line(b.cursor().line)
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
            b.cursors[b.primary].cursor.move_right(&b.buffer);
        }
        self.invalidate_highlight();
        self.invalidate_git();
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
        let pos = b.cursor().byte_offset(&b.buffer);
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
            b.cursors[b.primary].cursor.move_right(&b.buffer);
        }
        self.invalidate_highlight();
        self.invalidate_git();
    }

    pub(super) fn backspace(&mut self) {
        let b = self.buf();
        let pos = b.cursor().byte_offset(&b.buffer);
        if pos == 0 {
            return;
        }
        let before = self.cursor_state();
        let b = self.buf_mut();
        // Move cursor left first (handles UTF-8 boundaries)
        b.cursors[b.primary].cursor.move_left(&b.buffer);
        let new_pos = b.cursor().byte_offset(&b.buffer);
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
        self.invalidate_git();
    }

    pub(super) fn delete_at_cursor(&mut self) {
        let b = self.buf();
        let pos = b.cursor().byte_offset(&b.buffer);
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
            b.cursors[b.primary].cursor.clamp(&b.buffer);
            self.invalidate_highlight();
            self.invalidate_git();
        }
    }

    // -----------------------------------------------------------------------
    // Line operations
    // -----------------------------------------------------------------------

    pub(super) fn duplicate_line(&mut self) {
        let b = self.buf();
        let line = b.cursor().line;
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
        b.cursors[b.primary].cursor.move_down(&b.buffer);
        self.invalidate_highlight();
        self.invalidate_git();
        self.set_message("Line duplicated", MessageType::Info);
    }

    pub(super) fn delete_line(&mut self) {
        let before = self.cursor_state();
        let b = self.buf();
        let line = b.cursor().line;
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
        b.cursors[b.primary].cursor.clamp(&b.buffer);
        b.cursors[b.primary].cursor.col = 0;
        b.cursors[b.primary].cursor.desired_col = 0;
        self.invalidate_highlight();
        self.invalidate_git();
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
            let line = self.buf().cursor().line;
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
        if b.cursor().line == line {
            let new_col = b.cursor().col.saturating_sub(remove_len);
            b.cursors[b.primary].cursor.col = new_col;
            b.cursors[b.primary].cursor.desired_col = new_col;
        }
        self.invalidate_highlight();
        self.invalidate_git();
    }

    pub(super) fn select_line(&mut self) {
        let b = self.buf();
        let line = b.cursor().line;
        let line_start = b.buffer.line_start(line).unwrap_or(0);
        let line_end = if line + 1 < b.buffer.line_count() {
            b.buffer.line_start(line + 1).unwrap_or(b.buffer.len())
        } else {
            b.buffer.len()
        };
        self.buf_mut().set_selection(Some(Selection {
            anchor: line_start,
            head: line_end,
        }));
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
            let line = self.buf().cursor().line;
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
        b.cursors[b.primary].cursor.clamp(&b.buffer);
        self.invalidate_highlight();
        self.invalidate_git();
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
                // Reload HEAD content and refresh git diff after save
                if let Some(path) = b.buffer.file_path().map(|p| p.to_path_buf())
                    && let Some(gi) = &mut b.git_info
                {
                    gi.reload_head(&path);
                }
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
        let new_idx = self.buffers.len() - 1;
        self.layout.set_pane_buffer(self.active_pane, new_idx);
        self.active_buffer = new_idx;
        self.set_message("New buffer", MessageType::Info);
    }

    pub(super) fn close_buffer(&mut self) {
        let buf_idx = self.active_buffer_index();
        if self.buffers[buf_idx].buffer.is_modified() && !self.quit_confirm {
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
            self.layout.set_pane_buffer(self.active_pane, 0);
            self.set_message("Buffer closed", MessageType::Info);
            return;
        }

        let removed = buf_idx;
        self.buffers.remove(removed);
        self.layout.adjust_buffer_indices_after_remove(removed);
        self.active_buffer = self.active_buffer_index();
        self.set_message("Buffer closed", MessageType::Info);
    }

    pub(super) fn next_buffer(&mut self) {
        if self.buffers.len() > 1 {
            let current = self.active_buffer_index();
            let next = (current + 1) % self.buffers.len();
            self.layout.set_pane_buffer(self.active_pane, next);
            self.active_buffer = next;
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
            let current = self.active_buffer_index();
            let prev = if current == 0 {
                self.buffers.len() - 1
            } else {
                current - 1
            };
            self.layout.set_pane_buffer(self.active_pane, prev);
            self.active_buffer = prev;
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
                        b.cursors[b.primary]
                            .cursor
                            .set_position(line, col, &b.buffer);
                        let head = b.cursor().byte_offset(&b.buffer);
                        if let Some(ref mut sel) = b.cursors[b.primary].selection {
                            sel.head = head;
                        }
                    }
                } else if me.pressed {
                    // Click: determine which pane and set cursor
                    if let Some((line, col, pane_id)) =
                        self.screen_to_buffer_with_pane(me.col, me.row)
                    {
                        // Switch active pane if clicking in a different one
                        if pane_id != self.active_pane {
                            self.active_pane = pane_id;
                            self.active_buffer = self.active_buffer_index();
                        }

                        // Alt+Click adds a cursor instead of replacing
                        if me.alt && !self.buf().cursors.is_empty() {
                            self.add_cursor_at(line, col);
                            self.mouse_dragging = false;
                            return;
                        }

                        // Normal click: collapse to single cursor
                        if self.buf().is_multi() {
                            self.buf_mut().collapse_to_primary();
                        }

                        let b = self.buf_mut();
                        b.cursors[b.primary]
                            .cursor
                            .set_position(line, col, &b.buffer);
                        let offset = b.cursor().byte_offset(&b.buffer);
                        b.set_selection(Some(Selection {
                            anchor: offset,
                            head: offset,
                        }));
                        self.mouse_dragging = true;
                    }
                } else {
                    // Release
                    self.mouse_dragging = false;
                    // Clear selection if anchor == head (just a click, no drag)
                    if let Some(sel) = self.buf().selection()
                        && sel.anchor == sel.head
                    {
                        self.buf_mut().set_selection(None);
                    }
                }
            }
            MouseButton::ScrollUp => {
                // Scroll the pane under the mouse pointer
                let target_pane = self.pane_at_mouse(me.col, me.row);
                let buf_idx = target_pane
                    .and_then(|p| self.layout.pane_buffer(p))
                    .unwrap_or(self.active_buffer_index());
                if buf_idx < self.buffers.len() {
                    self.buffers[buf_idx].scroll_row =
                        self.buffers[buf_idx].scroll_row.saturating_sub(3);
                }
                // Clamp cursor if scrolling the active pane
                if target_pane == Some(self.active_pane) || target_pane.is_none() {
                    let h = self.text_area_height();
                    let b = self.buf_mut();
                    if b.cursor().line >= b.scroll_row + h {
                        let target = b.scroll_row + h - 1;
                        let col = b.cursor().col;
                        b.cursors[b.primary]
                            .cursor
                            .set_position(target, col, &b.buffer);
                    }
                }
            }
            MouseButton::ScrollDown => {
                let target_pane = self.pane_at_mouse(me.col, me.row);
                let buf_idx = target_pane
                    .and_then(|p| self.layout.pane_buffer(p))
                    .unwrap_or(self.active_buffer_index());
                if buf_idx < self.buffers.len() {
                    let max_scroll = self.buffers[buf_idx].buffer.line_count().saturating_sub(1);
                    self.buffers[buf_idx].scroll_row =
                        (self.buffers[buf_idx].scroll_row + 3).min(max_scroll);
                }
                if target_pane == Some(self.active_pane) || target_pane.is_none() {
                    let b = self.buf_mut();
                    if b.cursor().line < b.scroll_row {
                        let target = b.scroll_row;
                        let col = b.cursor().col;
                        b.cursors[b.primary]
                            .cursor
                            .set_position(target, col, &b.buffer);
                    }
                }
            }
            _ => {}
        }
    }

    /// Find which pane a screen coordinate falls in.
    fn pane_at_mouse(&self, col: u16, row: u16) -> Option<crate::layout::PaneId> {
        for pane_info in self.layout.panes() {
            if pane_info.rect.contains(col, row) {
                return Some(pane_info.id);
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Paste
    // -----------------------------------------------------------------------

    pub(super) fn handle_paste(&mut self, text: &str) {
        let before = self.cursor_state();
        let b = self.buf();
        let pos = b.cursor().byte_offset(&b.buffer);
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
            b.cursors[b.primary].cursor.move_right(&b.buffer);
        }
        self.invalidate_highlight();
        self.invalidate_git();
    }

    // -----------------------------------------------------------------------
    // Undo / Redo
    // -----------------------------------------------------------------------

    pub(super) fn do_undo(&mut self) {
        if self.buf().is_multi() {
            self.buf_mut().collapse_to_primary();
        }
        self.buf_mut().set_selection(None);
        let cs = self.cursor_state();
        let b = self.buf_mut();
        if let Some(restored) = b.undo_stack.undo(&mut b.buffer, cs) {
            b.cursors[b.primary].cursor.line = restored.line;
            b.cursors[b.primary].cursor.col = restored.col;
            b.cursors[b.primary].cursor.desired_col = restored.desired_col;
            b.cursors[b.primary].cursor.clamp(&b.buffer);
            self.invalidate_highlight();
            self.invalidate_git();
            self.set_message("Undo", MessageType::Info);
        } else {
            self.set_message("Nothing to undo", MessageType::Warning);
        }
    }

    pub(super) fn do_redo(&mut self) {
        if self.buf().is_multi() {
            self.buf_mut().collapse_to_primary();
        }
        self.buf_mut().set_selection(None);
        let b = self.buf_mut();
        if let Some(restored) = b.undo_stack.redo(&mut b.buffer) {
            b.cursors[b.primary].cursor.line = restored.line;
            b.cursors[b.primary].cursor.col = restored.col;
            b.cursors[b.primary].cursor.desired_col = restored.desired_col;
            b.cursors[b.primary].cursor.clamp(&b.buffer);
            self.invalidate_highlight();
            self.invalidate_git();
            self.set_message("Redo", MessageType::Info);
        } else {
            self.set_message("Nothing to redo", MessageType::Warning);
        }
    }

    // -----------------------------------------------------------------------
    // Undo/highlight helpers
    // -----------------------------------------------------------------------

    pub(super) fn cursor_state(&self) -> CursorState {
        let b = self.buf();
        CursorState {
            line: b.cursor().line,
            col: b.cursor().col,
            desired_col: b.cursor().desired_col,
        }
    }

    pub(super) fn invalidate_highlight(&mut self) {
        let cursor_line = self.buf().cursor().line;
        if let Some(h) = &mut self.buf_mut().highlighter {
            h.invalidate_from(cursor_line);
        }
        self.invalidate_wrap();
    }

    pub(super) fn invalidate_wrap(&mut self) {
        let b = self.buf_mut();
        if let Some(ref mut wm) = b.wrap_map {
            wm.rebuild(&b.buffer);
        }
    }

    pub(super) fn invalidate_git(&mut self) {
        if let Some(gi) = &mut self.buf_mut().git_info {
            gi.mark_stale();
        }
    }

    // -----------------------------------------------------------------------
    // Multi-cursor editing operations
    // -----------------------------------------------------------------------

    /// Insert a character at all cursor positions (last-to-first).
    pub(super) fn insert_char_multi(&mut self, ch: char) {
        let before = self.cursor_state();
        let mut buf_str = [0u8; 4];
        let s = ch.encode_utf8(&mut buf_str);
        let text = s.to_string();
        let char_len = text.len();

        // Delete selections first (last-to-first)
        self.delete_all_selections();

        let b = self.buf_mut();
        // Collect positions (sorted last-to-first by byte offset)
        let mut positions: Vec<(usize, usize)> = b
            .cursors
            .iter()
            .enumerate()
            .map(|(i, cs)| (i, cs.cursor.byte_offset(&b.buffer)))
            .collect();
        positions.sort_by(|a, b_pos| b_pos.1.cmp(&a.1)); // reverse sort by offset

        // Insert last-to-first
        for &(_, pos) in &positions {
            b.buffer.insert(pos, &text);
        }

        // Record as single undo operation (using primary cursor state)
        // We record one compound insert for simplicity
        let primary_pos = b.cursors[b.primary].cursor.byte_offset(&b.buffer);
        b.undo_stack.record(
            Operation::Insert {
                pos: primary_pos.saturating_sub(char_len),
                text: text.clone(),
            },
            before,
            GroupContext::Typing,
        );

        // Update all cursor positions: move each right by one char
        for cs in &mut b.cursors {
            cs.cursor.move_right(&b.buffer);
        }

        // Invalidate from the earliest affected line
        let min_line = b.cursors.iter().map(|cs| cs.cursor.line).min().unwrap_or(0);
        if let Some(h) = &mut b.highlighter {
            h.invalidate_from(min_line.saturating_sub(1));
        }
        if let Some(gi) = &mut b.git_info {
            gi.mark_stale();
        }
    }

    /// Insert a newline at all cursor positions (last-to-first).
    pub(super) fn insert_newline_multi(&mut self) {
        let before = self.cursor_state();
        self.delete_all_selections();

        let auto_indent = self.config().auto_indent;
        let b = self.buf_mut();

        // Collect cursor info sorted last-to-first
        let mut cursor_info: Vec<(usize, usize, String)> = Vec::with_capacity(b.cursors.len());
        for (i, cs) in b.cursors.iter().enumerate() {
            let pos = cs.cursor.byte_offset(&b.buffer);
            let insert_text = if auto_indent {
                let indent = b
                    .buffer
                    .get_line(cs.cursor.line)
                    .unwrap_or_default()
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect::<String>();
                format!("\n{}", indent)
            } else {
                "\n".to_string()
            };
            cursor_info.push((i, pos, insert_text));
        }
        cursor_info.sort_by(|a, b_info| b_info.1.cmp(&a.1)); // reverse sort

        for (_, pos, insert_text) in &cursor_info {
            b.buffer.insert(*pos, insert_text);
        }

        // Record undo for primary
        let primary_pos = b.cursors[b.primary].cursor.byte_offset(&b.buffer);
        b.undo_stack.record(
            Operation::Insert {
                pos: primary_pos,
                text: "\n".to_string(),
            },
            before,
            GroupContext::Other,
        );

        // Move cursors past inserted text
        for cs in &mut b.cursors {
            // Each cursor needs to move right by the length of its insert
            // For simplicity, just recalculate based on current buffer state
            cs.cursor.move_right(&b.buffer); // past \n at minimum
        }

        // Re-position: since we inserted in reverse, byte offsets shifted
        // It's safest to sort and merge
        b.sort_and_merge();

        let min_line = b.cursors.iter().map(|cs| cs.cursor.line).min().unwrap_or(0);
        if let Some(h) = &mut b.highlighter {
            h.invalidate_from(min_line.saturating_sub(1));
        }
        if let Some(gi) = &mut b.git_info {
            gi.mark_stale();
        }
    }

    /// Insert a tab at all cursor positions (last-to-first).
    pub(super) fn insert_tab_multi(&mut self) {
        let before = self.cursor_state();
        let use_spaces = self.config().use_spaces;
        let tab_size = self.config().tab_size;
        let insert_text = if use_spaces {
            " ".repeat(tab_size)
        } else {
            "\t".to_string()
        };
        let text_len = insert_text.len();

        self.delete_all_selections();

        let b = self.buf_mut();
        let mut positions: Vec<usize> = b
            .cursors
            .iter()
            .map(|cs| cs.cursor.byte_offset(&b.buffer))
            .collect();
        positions.sort_unstable();
        positions.reverse();

        for &pos in &positions {
            b.buffer.insert(pos, &insert_text);
        }

        let primary_pos = b.cursors[b.primary].cursor.byte_offset(&b.buffer);
        b.undo_stack.record(
            Operation::Insert {
                pos: primary_pos,
                text: insert_text.clone(),
            },
            before,
            GroupContext::Other,
        );

        for cs in &mut b.cursors {
            for _ in 0..text_len {
                cs.cursor.move_right(&b.buffer);
            }
        }

        let min_line = b.cursors.iter().map(|cs| cs.cursor.line).min().unwrap_or(0);
        if let Some(h) = &mut b.highlighter {
            h.invalidate_from(min_line.saturating_sub(1));
        }
        if let Some(gi) = &mut b.git_info {
            gi.mark_stale();
        }
    }

    /// Backspace at all cursor positions (last-to-first).
    pub(super) fn backspace_multi(&mut self) {
        // If any cursor has a selection, delete selections instead
        if self.buf().cursors.iter().any(|cs| cs.selection.is_some()) {
            self.delete_all_selections();
            return;
        }

        let before = self.cursor_state();
        let b = self.buf_mut();

        // Collect (cursor_index, byte_offset) sorted last-to-first
        let mut positions: Vec<(usize, usize)> = b
            .cursors
            .iter()
            .enumerate()
            .map(|(i, cs)| (i, cs.cursor.byte_offset(&b.buffer)))
            .collect();
        positions.sort_by(|a, b_pos| b_pos.1.cmp(&a.1));

        for &(idx, pos) in &positions {
            if pos == 0 {
                continue;
            }
            // Move cursor left first
            b.cursors[idx].cursor.move_left(&b.buffer);
            let new_pos = b.cursors[idx].cursor.byte_offset(&b.buffer);
            let delete_len = pos - new_pos;
            b.buffer.delete(new_pos, delete_len);
        }

        let primary_pos = b.cursors[b.primary].cursor.byte_offset(&b.buffer);
        b.undo_stack.record(
            Operation::Delete {
                pos: primary_pos,
                text: String::new(), // simplified for multi-cursor undo
            },
            before,
            GroupContext::Deleting,
        );

        b.sort_and_merge();

        let min_line = b.cursors.iter().map(|cs| cs.cursor.line).min().unwrap_or(0);
        if let Some(h) = &mut b.highlighter {
            h.invalidate_from(min_line.saturating_sub(1));
        }
        if let Some(gi) = &mut b.git_info {
            gi.mark_stale();
        }
    }

    /// Delete at all cursor positions (last-to-first).
    pub(super) fn delete_at_multi(&mut self) {
        if self.buf().cursors.iter().any(|cs| cs.selection.is_some()) {
            self.delete_all_selections();
            return;
        }

        let before = self.cursor_state();
        let b = self.buf_mut();

        let mut positions: Vec<usize> = b
            .cursors
            .iter()
            .map(|cs| cs.cursor.byte_offset(&b.buffer))
            .collect();
        positions.sort_unstable();
        positions.reverse();

        for &pos in &positions {
            if pos >= b.buffer.len() {
                continue;
            }
            if let Some(ch) = b.buffer.char_at(pos) {
                let char_len = ch.len_utf8();
                b.buffer.delete(pos, char_len);
            }
        }

        let primary_pos = b.cursors[b.primary].cursor.byte_offset(&b.buffer);
        b.undo_stack.record(
            Operation::Delete {
                pos: primary_pos,
                text: String::new(),
            },
            before,
            GroupContext::Deleting,
        );

        // Clamp all cursors
        for cs in &mut b.cursors {
            cs.cursor.clamp(&b.buffer);
        }
        b.sort_and_merge();

        let min_line = b.cursors.iter().map(|cs| cs.cursor.line).min().unwrap_or(0);
        if let Some(h) = &mut b.highlighter {
            h.invalidate_from(min_line.saturating_sub(1));
        }
        if let Some(gi) = &mut b.git_info {
            gi.mark_stale();
        }
    }

    /// Delete selections at all cursors that have them (last-to-first by offset).
    fn delete_all_selections(&mut self) {
        let before = self.cursor_state();
        let b = self.buf_mut();

        // Collect selection ranges, sorted last-to-first
        let mut sel_info: Vec<(usize, usize, usize)> = Vec::new(); // (cursor_idx, start, end)
        for (i, cs) in b.cursors.iter().enumerate() {
            if let Some(sel) = cs.selection {
                let start = sel.anchor.min(sel.head);
                let end = sel.anchor.max(sel.head);
                if start != end {
                    sel_info.push((i, start, end));
                }
            }
        }
        sel_info.sort_by(|a, b_info| b_info.1.cmp(&a.1)); // reverse by start

        for &(idx, start, end) in &sel_info {
            b.buffer.delete(start, end - start);
            // Reposition cursor to selection start
            let line = b.buffer.byte_to_line(start);
            let line_start = b.buffer.line_start(line).unwrap_or(0);
            let col = start - line_start;
            b.cursors[idx].cursor.set_position(line, col, &b.buffer);
            b.cursors[idx].selection = None;
        }

        if !sel_info.is_empty() {
            let primary_pos = b.cursors[b.primary].cursor.byte_offset(&b.buffer);
            b.undo_stack.record(
                Operation::Delete {
                    pos: primary_pos,
                    text: String::new(),
                },
                before,
                GroupContext::Other,
            );
            b.sort_and_merge();
        }

        // Clear all remaining selections
        for cs in &mut b.cursors {
            cs.selection = None;
        }
    }
}
