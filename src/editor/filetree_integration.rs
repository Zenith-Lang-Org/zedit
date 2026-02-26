use std::path::{Path, PathBuf};

use crate::filetree::{FileTree, TreeMode};
use crate::input::{Key, KeyEvent};

use super::*;

impl Editor {
    /// Toggle the file tree sidebar on/off.
    pub(super) fn toggle_filetree(&mut self) {
        if self.filetree.is_some() {
            self.filetree = None;
            self.filetree_focused = false;
            self.resolve_layout();
            self.recompute_all_wrap_maps();
            self.set_message("File tree closed", MessageType::Info);
        } else {
            let root = self.detect_project_root();
            let width = self.config.filetree_width;
            let ft = FileTree::new(root, width, &self.config.filetree_ignored);
            self.filetree = Some(ft);
            self.filetree_focused = true;
            self.resolve_layout();
            self.recompute_all_wrap_maps();
            self.set_message("File tree opened", MessageType::Info);
        }
    }

    /// Detect the project root by walking up from the first open file's directory
    /// (or cwd) looking for VCS/build markers.
    fn detect_project_root(&self) -> PathBuf {
        // Start from the first buffer's file path, or cwd.
        // Always resolve to an absolute path before walking so that relative
        // paths (e.g. "src/main.rs") don't produce an empty parent component
        // that makes FileTree::new() fail to scan the root directory.
        let start = self
            .buffers
            .iter()
            .find_map(|bs| {
                bs.buffer.file_path().map(|p| {
                    if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        std::env::current_dir()
                            .map(|cwd| cwd.join(p))
                            .unwrap_or_else(|_| p.to_path_buf())
                    }
                })
            })
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        let markers = [
            ".git",
            "Cargo.toml",
            "package.json",
            "go.mod",
            "Makefile",
            ".hg",
        ];

        let mut dir = start.as_path();
        loop {
            for marker in &markers {
                if dir.join(marker).exists() {
                    return dir.to_path_buf();
                }
            }
            match dir.parent() {
                Some(parent) if parent != dir => dir = parent,
                _ => break,
            }
        }

        // Fallback: cwd or start dir
        std::env::current_dir().unwrap_or(start)
    }

    /// Handle a key event when the file tree sidebar is focused.
    /// Returns true if the key was consumed.
    pub(super) fn handle_filetree_key(&mut self, ke: KeyEvent) -> bool {
        let ft = match self.filetree.as_mut() {
            Some(ft) => ft,
            None => return false,
        };

        // Handle mode-specific input first
        match ft.mode {
            TreeMode::Filter => {
                return self.handle_filetree_filter_key(ke);
            }
            TreeMode::PromptNewFile | TreeMode::PromptNewDir | TreeMode::PromptRename => {
                return self.handle_filetree_prompt_key(ke);
            }
            TreeMode::ConfirmDelete => {
                return self.handle_filetree_confirm_delete(ke);
            }
            TreeMode::Normal => {}
        }

        match (&ke.key, ke.ctrl, ke.alt) {
            // Close sidebar
            (Key::Char('b'), true, false) => {
                self.toggle_filetree();
                true
            }
            // Navigation
            (Key::Up, false, false) | (Key::Char('k'), false, false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.move_up();
                }
                true
            }
            (Key::Down, false, false) | (Key::Char('j'), false, false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.move_down();
                }
                true
            }
            // Open file / expand dir
            (Key::Enter, false, false)
            | (Key::Right, false, false)
            | (Key::Char('l'), false, false) => {
                let result = if let Some(ft) = &mut self.filetree {
                    ft.enter()
                } else {
                    None
                };
                if let Some(path) = result {
                    self.open_file_from_tree(&path);
                }
                true
            }
            // Collapse / go parent
            (Key::Left, false, false) | (Key::Char('h'), false, false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.go_parent();
                }
                true
            }
            // Toggle expand
            (Key::Char(' '), false, false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.toggle_expand();
                }
                true
            }
            // New file
            (Key::Char('a'), false, false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.start_new_file();
                }
                true
            }
            // New directory
            (Key::Char('A'), false, false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.start_new_dir();
                }
                true
            }
            // Delete
            (Key::Char('d'), false, false) if !ke.ctrl => {
                if let Some(ft) = &mut self.filetree {
                    ft.start_delete();
                }
                true
            }
            // Rename
            (Key::Char('r'), false, false) if !ke.ctrl => {
                if let Some(ft) = &mut self.filetree {
                    ft.start_rename();
                }
                true
            }
            // Filter
            (Key::Char('/'), false, false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.start_filter();
                }
                true
            }
            // Refresh
            (Key::Char('R'), false, false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.refresh();
                }
                self.set_message("Tree refreshed", MessageType::Info);
                true
            }
            // Unfocus sidebar
            (Key::Escape, false, false) => {
                self.filetree_focused = false;
                true
            }
            // Alt+Right / Alt+Left/Up/Down: leave file tree, focus editor pane
            (Key::Right | Key::Left | Key::Up | Key::Down, false, true) => {
                self.filetree_focused = false;
                true
            }
            _ => false,
        }
    }

    fn handle_filetree_filter_key(&mut self, ke: KeyEvent) -> bool {
        match (&ke.key, ke.ctrl) {
            (Key::Escape, _) => {
                if let Some(ft) = &mut self.filetree {
                    ft.stop_filter();
                }
            }
            (Key::Enter, _) => {
                // Select filtered item and exit filter
                let path = if let Some(ft) = &mut self.filetree {
                    let result = ft.enter();
                    ft.stop_filter();
                    result
                } else {
                    None
                };
                if let Some(path) = path {
                    self.open_file_from_tree(&path);
                }
            }
            (Key::Backspace, _) => {
                if let Some(ft) = &mut self.filetree {
                    ft.filter_backspace();
                }
            }
            (Key::Char(ch), false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.filter_input(*ch);
                }
            }
            (Key::Up, _) => {
                if let Some(ft) = &mut self.filetree {
                    ft.move_up();
                }
            }
            (Key::Down, _) => {
                if let Some(ft) = &mut self.filetree {
                    ft.move_down();
                }
            }
            _ => {}
        }
        true
    }

    fn handle_filetree_prompt_key(&mut self, ke: KeyEvent) -> bool {
        match (&ke.key, ke.ctrl) {
            (Key::Escape, _) => {
                if let Some(ft) = &mut self.filetree {
                    ft.cancel_prompt();
                }
            }
            (Key::Enter, _) => {
                let ft = match self.filetree.as_mut() {
                    Some(ft) => ft,
                    None => return true,
                };
                let mode = ft.mode;
                match mode {
                    TreeMode::PromptNewFile => match ft.create_file() {
                        Ok(Some(path)) => {
                            self.open_file_from_tree(&path);
                            self.set_message("File created", MessageType::Info);
                        }
                        Ok(None) => {}
                        Err(e) => self.set_message(&e, MessageType::Error),
                    },
                    TreeMode::PromptNewDir => match ft.create_dir() {
                        Ok(()) => self.set_message("Directory created", MessageType::Info),
                        Err(e) => self.set_message(&e, MessageType::Error),
                    },
                    TreeMode::PromptRename => match ft.rename_node() {
                        Ok(()) => self.set_message("Renamed", MessageType::Info),
                        Err(e) => self.set_message(&e, MessageType::Error),
                    },
                    _ => {}
                }
            }
            (Key::Backspace, _) => {
                if let Some(ft) = &mut self.filetree {
                    ft.prompt_backspace();
                }
            }
            (Key::Char(ch), false) => {
                if let Some(ft) = &mut self.filetree {
                    ft.prompt_insert_char(*ch);
                }
            }
            _ => {}
        }
        true
    }

    fn handle_filetree_confirm_delete(&mut self, ke: KeyEvent) -> bool {
        match &ke.key {
            Key::Char('y') | Key::Char('Y') => {
                if let Some(ft) = &mut self.filetree {
                    match ft.delete_node() {
                        Ok(()) => self.set_message("Deleted", MessageType::Info),
                        Err(e) => self.set_message(&e, MessageType::Error),
                    }
                }
            }
            _ => {
                // Any other key cancels
                if let Some(ft) = &mut self.filetree {
                    ft.cancel_prompt();
                }
            }
        }
        true
    }

    /// Open a file from the tree in the active pane, reusing an existing buffer if possible.
    pub(super) fn open_file_from_tree(&mut self, path: &Path) {
        // Ensure we open in an editor pane, not the terminal
        self.ensure_editor_pane();

        // Canonicalize for comparison
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        // Check if file is already open in a buffer
        for (i, bs) in self.buffers.iter().enumerate() {
            if let Some(bp) = bs.buffer.file_path() {
                let bp_canon = std::fs::canonicalize(bp).unwrap_or_else(|_| bp.to_path_buf());
                if bp_canon == canonical {
                    // Switch to this buffer
                    self.layout.set_pane_buffer(self.active_pane, i);
                    self.active_buffer = i;
                    self.filetree_focused = false;
                    let display = shorten_path(path);
                    self.set_message(&format!("Switched to: {}", display), MessageType::Info);
                    return;
                }
            }
        }

        // Open new buffer
        match BufferState::from_file(
            path,
            self.config.line_numbers,
            &self.config.theme,
            &self.config.languages,
        ) {
            Ok(bs) => {
                let display = shorten_path(path);
                let buf_idx = self.active_buffer_index();
                let current_empty = self.buffers[buf_idx].buffer.is_empty()
                    && !self.buffers[buf_idx].buffer.is_modified()
                    && self.buffers[buf_idx].buffer.file_path().is_none();
                if current_empty {
                    self.buffers[buf_idx] = bs;
                } else {
                    self.buffers.push(bs);
                    let new_idx = self.buffers.len() - 1;
                    self.layout.set_pane_buffer(self.active_pane, new_idx);
                    self.active_buffer = new_idx;
                }
                self.filetree_focused = false;
                self.set_message(&format!("Opened: {}", display), MessageType::Info);
                // Notify LSP about newly opened file
                let notify_idx = self.active_buffer_index();
                self.lsp_notify_open(notify_idx);
            }
            Err(e) => {
                self.set_message(&format!("Error: {}", e), MessageType::Error);
            }
        }
    }
}
