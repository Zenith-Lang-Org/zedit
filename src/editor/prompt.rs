use std::path::Path;

use crate::input::{Key, KeyEvent};
use crate::syntax::highlight::{self, Highlighter};

use super::*;

// ---------------------------------------------------------------------------
// Prompt types
// ---------------------------------------------------------------------------

pub(super) enum PromptAction {
    OpenFile,
    Find,
    Replace,
    ReplaceWith(String),
    GoToLine,
    SaveAs,
}

pub(super) struct Prompt {
    pub(super) label: String,
    pub(super) input: String,
    pub(super) cursor_pos: usize, // byte offset within input
    pub(super) action: PromptAction,
}

// ---------------------------------------------------------------------------
// Editor methods
// ---------------------------------------------------------------------------

impl Editor {
    pub(super) fn start_prompt(&mut self, label: &str, action: PromptAction) {
        self.prompt = Some(Prompt {
            label: label.to_string(),
            input: String::new(),
            cursor_pos: 0,
            action,
        });
        self.message = None;
    }

    pub(super) fn handle_prompt_key(&mut self, ke: KeyEvent) {
        let mut input_changed = false;

        match (&ke.key, ke.ctrl, ke.alt) {
            (Key::Enter, false, false) => {
                // Take the prompt out to avoid borrow issues
                let prompt = self.prompt.take().unwrap();
                if prompt.input.is_empty() {
                    // Empty input — cancel
                    return;
                }
                self.execute_prompt(prompt);
                return;
            }
            (Key::Escape, _, _) => {
                // Keep search state so F3 still works
                self.prompt = None;
                return;
            }
            // Ctrl+R toggles regex mode in Find/Replace prompts
            (Key::Char('r'), true, false) => {
                if let Some(ref prompt) = self.prompt
                    && matches!(prompt.action, PromptAction::Find | PromptAction::Replace)
                {
                    // Toggle search mode
                    let current_mode = self
                        .buf()
                        .search
                        .as_ref()
                        .map_or(SearchMode::Substring, |s| s.mode);
                    let new_mode = match current_mode {
                        SearchMode::Substring => SearchMode::Regex,
                        SearchMode::Regex => SearchMode::Substring,
                    };

                    // Update existing search state mode or create placeholder
                    if let Some(ref mut search) = self.buf_mut().search {
                        search.mode = new_mode;
                    } else {
                        self.buf_mut().search = Some(SearchState {
                            pattern: String::new(),
                            matches: Vec::new(),
                            current: None,
                            mode: new_mode,
                        });
                    }

                    // Update prompt label
                    let label = if new_mode == SearchMode::Regex {
                        "Find (regex): "
                    } else {
                        "Find: "
                    };
                    if let Some(ref mut prompt) = self.prompt {
                        prompt.label = label.to_string();
                    }

                    // Re-run search with new mode
                    if let Some(ref prompt) = self.prompt {
                        let pattern = prompt.input.clone();
                        if !pattern.is_empty() {
                            self.update_search(&pattern);
                        }
                    }
                    return;
                }
            }
            (Key::Backspace, false, false) => {
                if let Some(ref mut prompt) = self.prompt
                    && prompt.cursor_pos > 0
                {
                    let before = &prompt.input[..prompt.cursor_pos];
                    if let Some(ch) = before.chars().next_back() {
                        let len = ch.len_utf8();
                        let new_pos = prompt.cursor_pos - len;
                        prompt.input.drain(new_pos..prompt.cursor_pos);
                        prompt.cursor_pos = new_pos;
                        input_changed = true;
                    }
                }
            }
            (Key::Delete, false, false) => {
                if let Some(ref mut prompt) = self.prompt
                    && prompt.cursor_pos < prompt.input.len()
                {
                    let after = &prompt.input[prompt.cursor_pos..];
                    if let Some(ch) = after.chars().next() {
                        let len = ch.len_utf8();
                        prompt
                            .input
                            .drain(prompt.cursor_pos..prompt.cursor_pos + len);
                        input_changed = true;
                    }
                }
            }
            (Key::Left, false, false) => {
                if let Some(ref mut prompt) = self.prompt
                    && prompt.cursor_pos > 0
                {
                    let before = &prompt.input[..prompt.cursor_pos];
                    if let Some(ch) = before.chars().next_back() {
                        prompt.cursor_pos -= ch.len_utf8();
                    }
                }
            }
            (Key::Right, false, false) => {
                if let Some(ref mut prompt) = self.prompt
                    && prompt.cursor_pos < prompt.input.len()
                {
                    let after = &prompt.input[prompt.cursor_pos..];
                    if let Some(ch) = after.chars().next() {
                        prompt.cursor_pos += ch.len_utf8();
                    }
                }
            }
            (Key::Home, false, false) => {
                if let Some(ref mut prompt) = self.prompt {
                    prompt.cursor_pos = 0;
                }
            }
            (Key::End, false, false) => {
                if let Some(ref mut prompt) = self.prompt {
                    prompt.cursor_pos = prompt.input.len();
                }
            }
            (Key::Char(ch), false, false) => {
                if let Some(ref mut prompt) = self.prompt {
                    let mut buf = [0u8; 4];
                    let s = ch.encode_utf8(&mut buf);
                    prompt.input.insert_str(prompt.cursor_pos, s);
                    prompt.cursor_pos += s.len();
                    input_changed = true;
                }
            }
            _ => {}
        }

        // Incremental search: update matches when input changes in Find/Replace prompts
        if input_changed && let Some(ref prompt) = self.prompt {
            let is_search_prompt =
                matches!(prompt.action, PromptAction::Find | PromptAction::Replace);
            if is_search_prompt {
                let pattern = prompt.input.clone();
                self.update_search(&pattern);
            }
        }
    }

    pub(super) fn execute_prompt(&mut self, prompt: Prompt) {
        match prompt.action {
            PromptAction::OpenFile => {
                let path = Path::new(&prompt.input);
                match BufferState::from_file(
                    path,
                    self.config.line_numbers,
                    &self.config.theme,
                    &self.config.languages,
                ) {
                    Ok(bs) => {
                        let display_name = shorten_path(path);
                        let buf_idx = self.active_buffer_index();
                        // Open in current slot if current buffer is empty, else new buffer
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
                        self.set_message(&format!("Opened: {}", display_name), MessageType::Info);
                    }
                    Err(e) => {
                        // Keep prompt open so user can fix the path
                        self.prompt = Some(prompt);
                        self.set_message(&format!("Error: {}", e), MessageType::Error);
                    }
                }
            }
            PromptAction::Find => {
                // Finalize search, jump to current match
                self.update_search(&prompt.input.clone());
                if let Some(ref search) = self.buf().search {
                    if search.matches.is_empty() {
                        self.set_message("No matches", MessageType::Warning);
                    } else {
                        let total = search.matches.len();
                        let current = search.current.map_or(0, |i| i + 1);
                        self.set_message(
                            &format!("Match {} of {}", current, total),
                            MessageType::Info,
                        );
                    }
                }
            }
            PromptAction::Replace => {
                // Save pattern, open "Replace with:" prompt
                let pattern = prompt.input;
                self.update_search(&pattern);
                if let Some(ref search) = self.buf().search
                    && search.matches.is_empty()
                {
                    self.set_message("No matches", MessageType::Warning);
                    return;
                }
                self.start_prompt("Replace with: ", PromptAction::ReplaceWith(pattern));
            }
            PromptAction::ReplaceWith(ref find_pattern) => {
                let replacement = prompt.input;
                let find_pattern = find_pattern.clone();
                self.execute_replace_all(&find_pattern, &replacement);
            }
            PromptAction::GoToLine => match prompt.input.trim().parse::<usize>() {
                Ok(n) if n > 0 => {
                    let max = self.buf().buffer.line_count().saturating_sub(1);
                    let target = (n - 1).min(max);
                    let b = self.buf_mut();
                    b.cursor.set_position(target, 0, &b.buffer);
                    b.selection = None;
                    self.set_message(&format!("Jumped to line {}", target + 1), MessageType::Info);
                }
                _ => {
                    self.set_message("Invalid line number", MessageType::Error);
                }
            },
            PromptAction::SaveAs => {
                let path = Path::new(&prompt.input);
                let buf_idx = self.active_buffer_index();
                match self.buf_mut().buffer.save_to(path) {
                    Ok(()) => {
                        let display_name = shorten_path(path);
                        let cs = self.cursor_state();
                        self.buf_mut().undo_stack.mark_saved(cs);
                        // Reload highlighter for new file extension
                        let theme_name = self.config.theme.clone();
                        let languages = &self.config.languages;
                        self.buffers[buf_idx].highlighter =
                            highlight::detect_language(path, languages).and_then(|lang| {
                                highlight::load_grammar(&lang, languages).map(|grammar| {
                                    let theme = highlight::load_theme(&theme_name);
                                    Highlighter::new(grammar, theme).with_lang(&lang)
                                })
                            });
                        self.set_message(&format!("Saved: {}", display_name), MessageType::Info);
                    }
                    Err(e) => {
                        self.prompt = Some(prompt);
                        self.set_message(&format!("Error: {}", e), MessageType::Error);
                    }
                }
            }
        }
    }
}
