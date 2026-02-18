use crate::terminal;
use crate::undo::{GroupContext, Operation};

use super::*;

impl Editor {
    pub(super) fn start_or_continue_selection(&mut self) {
        let b = self.buf();
        if b.selection.is_none() {
            let offset = b.cursor.byte_offset(&b.buffer);
            self.buf_mut().selection = Some(Selection {
                anchor: offset,
                head: offset,
            });
        }
    }

    pub(super) fn extend_selection(&mut self) {
        let b = self.buf();
        let head = b.cursor.byte_offset(&b.buffer);
        if let Some(ref mut sel) = self.buf_mut().selection {
            sel.head = head;
        }
    }

    pub(super) fn selection_range(&self) -> Option<(usize, usize)> {
        self.buf().selection.map(|sel| {
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
            self.buf_mut().selection = None;
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
        b.cursor.set_position(line, col, &b.buffer);
        b.selection = None;
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
            terminal::set_clipboard_osc52(&text);
            self.clipboard.set_text(text);
            self.set_message(&format!("Copied {} chars", len), MessageType::Info);
        } else {
            // No selection: copy current line
            self.copy_current_line();
        }
    }

    fn copy_current_line(&mut self) {
        let b = self.buf();
        let line_text = b.buffer.get_line(b.cursor.line).unwrap_or_default();
        let text = format!("{}\n", line_text);
        let len = line_text.chars().count();
        terminal::set_clipboard_osc52(&text);
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
            terminal::set_clipboard_osc52(&text);
            self.clipboard.set_text(text);
            self.set_message(&format!("Cut {} chars", len), MessageType::Info);
        } else {
            self.cut_current_line();
        }
    }

    fn cut_current_line(&mut self) {
        let before = self.cursor_state();
        let b = self.buf();
        let line = b.cursor.line;
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
        b.cursor.clamp(&b.buffer);
        b.cursor.col = 0;
        b.cursor.desired_col = 0;
        terminal::set_clipboard_osc52(&text);
        self.clipboard.set_line(text);
        self.set_message(&format!("Cut line ({} chars)", len), MessageType::Info);
    }

    pub(super) fn paste_clipboard(&mut self) {
        if self.clipboard.is_empty() {
            self.set_message("Clipboard is empty", MessageType::Warning);
            return;
        }
        // Delete selection if active
        self.delete_selection();

        if self.clipboard.line_mode {
            // Line-mode paste: insert as a new line above current cursor position
            let text = self.clipboard.text();
            let before = self.cursor_state();
            let b = self.buf();
            let line_start = b.buffer.line_start(b.cursor.line).unwrap_or(0);
            let b = self.buf_mut();
            b.buffer.insert(line_start, &text);
            b.undo_stack.record(
                Operation::Insert {
                    pos: line_start,
                    text: text.clone(),
                },
                before,
                GroupContext::Paste,
            );
            // Position cursor at beginning of pasted content
            b.cursor.col = 0;
            b.cursor.desired_col = 0;
            self.invalidate_highlight();
        } else {
            let text = self.clipboard.text();
            self.handle_paste(&text);
        }
    }

    pub(super) fn select_all(&mut self) {
        let len = self.buf().buffer.len();
        self.buf_mut().selection = Some(Selection {
            anchor: 0,
            head: len,
        });
        let b = self.buf_mut();
        b.cursor.move_to_end(&b.buffer);
    }
}
