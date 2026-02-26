// ---------------------------------------------------------------------------
// Diff view integration — key handling + open helpers
// ---------------------------------------------------------------------------

use crate::input::{Key, KeyEvent};
use crate::keybindings::EditorAction;

use super::{Editor, FileChangeNotice, MessageType};

impl Editor {
    /// Open a diff comparing the buffer at `buf_idx` against its current on-disk version.
    pub(super) fn open_diff_vs_disk(&mut self, buf_idx: usize) {
        let path = match self.buffers[buf_idx].buffer.file_path() {
            Some(p) => p.to_path_buf(),
            None => {
                self.set_message("Buffer has no file path", MessageType::Warning);
                return;
            }
        };
        let line_count = self.buffers[buf_idx].buffer.line_count();
        let lines: Vec<String> = (0..line_count)
            .map(|i| self.buffers[buf_idx].buffer.get_line(i).unwrap_or_default())
            .collect();
        match crate::diff_view::DiffView::open_vs_disk(&path, lines) {
            Ok(mut dv) => {
                dv.from_file_change = Some(buf_idx);
                let hunk_count = dv.hunks.len();
                self.diff_view = Some(dv);
                self.set_message(
                    &format!(
                        "Diff (buffer vs disk): {} hunk(s) — n/N navigate, Esc: back to options",
                        hunk_count
                    ),
                    MessageType::Info,
                );
            }
            Err(e) => self.set_message(&format!("Diff failed: {}", e), MessageType::Error),
        }
    }

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
                let from_fc = self.diff_view.as_ref().and_then(|d| d.from_file_change);
                self.diff_view = None;
                if let Some(buf_idx) = from_fc {
                    let modified = self.buffers[buf_idx].buffer.is_modified();
                    self.file_change_notice = Some(FileChangeNotice {
                        buf_idx,
                        buffer_modified: modified,
                    });
                    self.set_message(
                        "File changed on disk  [R] Reload   [I] Ignore",
                        MessageType::Warning,
                    );
                }
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
