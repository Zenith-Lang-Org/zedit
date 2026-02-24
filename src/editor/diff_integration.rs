// ---------------------------------------------------------------------------
// Diff view integration — key handling + open helpers
// ---------------------------------------------------------------------------

use crate::input::{Key, KeyEvent};
use crate::keybindings::EditorAction;

use super::{Editor, MessageType};

impl Editor {
    /// Open a diff of the active buffer vs its HEAD version.
    pub(super) fn open_diff_vs_head(&mut self) {
        let buf_idx = self.active_buffer_index();
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_path_buf(),
            None => {
                self.set_message("Diff: buffer has no file path", MessageType::Warning);
                return;
            }
        };

        // Collect current buffer lines
        let line_count = self.buffers[buf_idx].buffer.line_count();
        let lines: Vec<String> = (0..line_count)
            .map(|i| self.buffers[buf_idx].buffer.get_line(i).unwrap_or_default())
            .collect();

        match crate::diff_view::DiffView::open_vs_head(&path, lines) {
            Some(dv) => {
                let hunk_count = dv.hunks.len();
                self.diff_view = Some(dv);
                if hunk_count == 0 {
                    self.set_message("Diff: no changes vs HEAD", MessageType::Info);
                } else {
                    let msg = format!(
                        "Diff: {} hunk(s) — n/N navigate hunks, Esc close",
                        hunk_count
                    );
                    self.set_message(&msg, MessageType::Info);
                }
            }
            None => {
                self.set_message("Diff: file not tracked in HEAD", MessageType::Warning);
            }
        }
    }

    /// Handle a key event when the diff view is open.
    /// Always consumes the event (diff view is modal).
    pub(super) fn handle_diff_key(&mut self, ke: KeyEvent) {
        // Check keybindings first
        if let Some(action) = self.config.keybindings.lookup(&ke) {
            match action {
                EditorAction::DiffNextHunk => {
                    if let Some(ref mut dv) = self.diff_view {
                        dv.next_hunk();
                    }
                    return;
                }
                EditorAction::DiffPrevHunk => {
                    if let Some(ref mut dv) = self.diff_view {
                        dv.prev_hunk();
                    }
                    return;
                }
                EditorAction::DiffOpenVsHead => {
                    // Toggle: close if already open
                    self.diff_view = None;
                    return;
                }
                _ => {}
            }
        }

        match ke.key {
            Key::Escape | Key::Char('q') => {
                self.diff_view = None;
            }
            Key::Up => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.scroll_up(1);
                }
            }
            Key::Down => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.scroll_down(1);
                }
            }
            Key::PageUp => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.scroll_up(20);
                }
            }
            Key::PageDown => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.scroll_down(20);
                }
            }
            Key::Home => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.scroll = 0;
                }
            }
            Key::End => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.scroll = dv.rows.len().saturating_sub(1);
                }
            }
            Key::Char('n') => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.next_hunk();
                }
            }
            Key::Char('N') => {
                if let Some(ref mut dv) = self.diff_view {
                    dv.prev_hunk();
                }
            }
            _ => {}
        }
    }
}
