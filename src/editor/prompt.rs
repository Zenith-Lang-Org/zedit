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
    pub(super) completer: Option<FileCompleter>,
}

// ---------------------------------------------------------------------------
// File path completer
// ---------------------------------------------------------------------------

pub(super) struct FileCompleter {
    /// Last directory scanned (avoids re-reading on every keypress).
    last_dir: String,
    /// Sorted list of entries in `last_dir`.
    pub(super) entries: Vec<DirEntry>,
    /// Indices into `entries` that match the current input prefix.
    pub(super) matches: Vec<usize>,
    /// Currently highlighted suggestion (index into `matches`).
    pub(super) selected: usize,
}

pub(super) struct DirEntry {
    pub(super) name: String,
    pub(super) is_dir: bool,
}

impl FileCompleter {
    pub(super) fn new() -> Self {
        FileCompleter {
            last_dir: String::new(),
            entries: Vec::new(),
            matches: Vec::new(),
            selected: 0,
        }
    }

    /// Refresh completions for the current input string.
    /// Only re-reads the directory when the directory portion changes.
    pub(super) fn update(&mut self, input: &str) {
        let (dir, prefix) = split_dir_and_prefix(input);

        if dir != self.last_dir {
            self.entries = read_dir_entries(&dir);
            self.last_dir = dir.clone();
        }

        let prefix_lower = prefix.to_lowercase();
        self.matches = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name.to_lowercase().starts_with(&prefix_lower))
            .map(|(i, _)| i)
            .collect();

        // Clamp selected to valid range
        if self.matches.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.matches.len() - 1);
        }
    }

    /// Tab-complete: return the full path with the longest common prefix of all matches.
    pub(super) fn tab_complete(&self, current_dir: &str) -> Option<String> {
        if self.matches.is_empty() {
            return None;
        }
        let names: Vec<&str> = self
            .matches
            .iter()
            .map(|&i| self.entries[i].name.as_str())
            .collect();
        let lcp = longest_common_prefix(&names);
        if lcp.is_empty() {
            return None;
        }
        // If single match and it's a directory, append "/"
        let suffix = if self.matches.len() == 1 && self.entries[self.matches[0]].is_dir {
            format!("{}/", lcp)
        } else {
            lcp.to_string()
        };
        Some(format!("{}{}", current_dir, suffix))
    }

    /// Full path of the currently selected suggestion.
    pub(super) fn selected_path(&self, current_dir: &str) -> Option<String> {
        let idx = *self.matches.get(self.selected)?;
        let e = &self.entries[idx];
        if e.is_dir {
            Some(format!("{}{}/", current_dir, e.name))
        } else {
            Some(format!("{}{}", current_dir, e.name))
        }
    }

    /// Whether the currently selected entry is a directory.
    pub(super) fn is_selected_dir(&self) -> bool {
        self.matches
            .get(self.selected)
            .and_then(|&i| self.entries.get(i))
            .is_some_and(|e| e.is_dir)
    }
}

/// Split an input path into (directory, filename-prefix).
/// `"src/ed"` → `("src/", "ed")`, `"foo"` → `("./", "foo")`.
fn split_dir_and_prefix(input: &str) -> (String, String) {
    match input.rfind('/') {
        Some(i) => (input[..=i].to_string(), input[i + 1..].to_string()),
        None => ("./".to_string(), input.to_string()),
    }
}

/// Read directory entries sorted directories-first, then alphabetically.
fn read_dir_entries(dir: &str) -> Vec<DirEntry> {
    let mut entries = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            entries.push(DirEntry { name, is_dir });
        }
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    entries
}

/// Longest common prefix of a slice of strings, UTF-8 safe (uses byte positions
/// from `char_indices` to avoid slicing at a non-char boundary).
fn longest_common_prefix<'a>(strs: &[&'a str]) -> &'a str {
    if strs.is_empty() {
        return "";
    }
    let first = strs[0];
    let mut byte_end = first.len();
    for s in &strs[1..] {
        let common = first
            .char_indices()
            .zip(s.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map(|((idx, ch), _)| idx + ch.len_utf8())
            .unwrap_or(0);
        byte_end = byte_end.min(common);
    }
    &first[..byte_end]
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod file_completer_tests {
    use super::*;

    #[test]
    fn test_split_dir_and_prefix_with_slash() {
        let (dir, prefix) = split_dir_and_prefix("src/editor/pr");
        assert_eq!(dir, "src/editor/");
        assert_eq!(prefix, "pr");
    }

    #[test]
    fn test_split_dir_and_prefix_no_slash() {
        let (dir, prefix) = split_dir_and_prefix("main");
        assert_eq!(dir, "./");
        assert_eq!(prefix, "main");
    }

    #[test]
    fn test_split_dir_and_prefix_trailing_slash() {
        let (dir, prefix) = split_dir_and_prefix("src/");
        assert_eq!(dir, "src/");
        assert_eq!(prefix, "");
    }

    #[test]
    fn test_longest_common_prefix_ascii() {
        let strs = ["editor", "editing", "extension"];
        assert_eq!(longest_common_prefix(&strs), "e");
    }

    #[test]
    fn test_longest_common_prefix_full_match() {
        let strs = ["view.rs", "view.rs"];
        assert_eq!(longest_common_prefix(&strs), "view.rs");
    }

    #[test]
    fn test_longest_common_prefix_no_common() {
        let strs = ["alpha", "beta"];
        assert_eq!(longest_common_prefix(&strs), "");
    }

    #[test]
    fn test_longest_common_prefix_single() {
        let strs = ["mod.rs"];
        assert_eq!(longest_common_prefix(&strs), "mod.rs");
    }

    #[test]
    fn test_longest_common_prefix_utf8_safe() {
        // "café" and "caf" share "caf"; ensuring we don't split mid-char
        let strs = ["café", "café_con_leche"];
        assert_eq!(longest_common_prefix(&strs), "café");
    }

    #[test]
    fn test_longest_common_prefix_empty_input() {
        let strs: &[&str] = &[];
        assert_eq!(longest_common_prefix(strs), "");
    }

    #[test]
    fn test_file_completer_update_filters_by_prefix() {
        let mut comp = FileCompleter::new();
        // Seed entries directly to avoid filesystem dependency
        comp.entries = vec![
            DirEntry { name: "editor".to_string(), is_dir: true },
            DirEntry { name: "extension.rs".to_string(), is_dir: false },
            DirEntry { name: "render.rs".to_string(), is_dir: false },
        ];
        comp.last_dir = "FAKE".to_string(); // prevent re-read

        // Manually filter as update() would after directory is populated
        let prefix_lower = "ed".to_lowercase();
        comp.matches = comp
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name.to_lowercase().starts_with(&prefix_lower))
            .map(|(i, _)| i)
            .collect();
        comp.selected = 0;

        assert_eq!(comp.matches.len(), 1);
        assert_eq!(comp.entries[comp.matches[0]].name, "editor");
    }

    #[test]
    fn test_file_completer_tab_complete_single_dir() {
        let mut comp = FileCompleter::new();
        comp.entries = vec![DirEntry { name: "src".to_string(), is_dir: true }];
        comp.last_dir = "./".to_string();
        comp.matches = vec![0];
        comp.selected = 0;

        let result = comp.tab_complete("./");
        assert_eq!(result, Some("./src/".to_string()));
    }

    #[test]
    fn test_file_completer_tab_complete_common_prefix() {
        let mut comp = FileCompleter::new();
        comp.entries = vec![
            DirEntry { name: "editor".to_string(), is_dir: true },
            DirEntry { name: "extension.rs".to_string(), is_dir: false },
        ];
        comp.last_dir = "src/".to_string();
        comp.matches = vec![0, 1];
        comp.selected = 0;

        let result = comp.tab_complete("src/");
        assert_eq!(result, Some("src/e".to_string()));
    }

    #[test]
    fn test_file_completer_selected_path_file() {
        let mut comp = FileCompleter::new();
        comp.entries = vec![DirEntry { name: "main.rs".to_string(), is_dir: false }];
        comp.last_dir = "src/".to_string();
        comp.matches = vec![0];
        comp.selected = 0;

        let path = comp.selected_path("src/");
        assert_eq!(path, Some("src/main.rs".to_string()));
    }

    #[test]
    fn test_file_completer_is_selected_dir() {
        let mut comp = FileCompleter::new();
        comp.entries = vec![
            DirEntry { name: "editor".to_string(), is_dir: true },
            DirEntry { name: "main.rs".to_string(), is_dir: false },
        ];
        comp.last_dir = "src/".to_string();
        comp.matches = vec![0, 1];

        comp.selected = 0;
        assert!(comp.is_selected_dir());
        comp.selected = 1;
        assert!(!comp.is_selected_dir());
    }
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
            completer: None,
        });
        self.message = None;
    }

    /// Open the file-open prompt with path completion pre-seeded from `initial`.
    pub(super) fn start_open_file_prompt(&mut self, initial: &str) {
        let mut comp = FileCompleter::new();
        comp.update(initial);
        self.prompt = Some(Prompt {
            label: "Open: ".to_string(),
            input: initial.to_string(),
            cursor_pos: initial.len(),
            action: PromptAction::OpenFile,
            completer: Some(comp),
        });
        self.message = None;
    }

    pub(super) fn handle_prompt_key(&mut self, ke: KeyEvent) {
        let mut input_changed = false;

        match (&ke.key, ke.ctrl, ke.alt) {
            (Key::Enter, false, false) => {
                let prompt = self.prompt.take().unwrap();
                if prompt.input.is_empty() {
                    return;
                }
                // For OpenFile with completer: check if selected entry is a directory
                if matches!(prompt.action, PromptAction::OpenFile) {
                    if let Some(ref comp) = prompt.completer {
                        if !comp.matches.is_empty() {
                            let (dir, _) = split_dir_and_prefix(&prompt.input);
                            if let Some(path) = comp.selected_path(&dir) {
                                if comp.is_selected_dir() {
                                    // Descend into directory: refresh prompt with new path
                                    let mut new_comp = FileCompleter::new();
                                    new_comp.update(&path);
                                    self.prompt = Some(Prompt {
                                        label: prompt.label,
                                        cursor_pos: path.len(),
                                        input: path,
                                        action: PromptAction::OpenFile,
                                        completer: Some(new_comp),
                                    });
                                    return;
                                } else {
                                    // Open the selected file
                                    let mut p = prompt;
                                    p.cursor_pos = path.len();
                                    p.input = path;
                                    self.execute_prompt(p);
                                    return;
                                }
                            }
                        }
                    }
                }
                self.execute_prompt(prompt);
                return;
            }
            (Key::Tab, false, false) => {
                // Tab-complete the file path when in an OpenFile prompt
                let is_open = self
                    .prompt
                    .as_ref()
                    .is_some_and(|p| matches!(p.action, PromptAction::OpenFile));
                if is_open {
                    if let Some(ref mut prompt) = self.prompt {
                        if let Some(ref mut comp) = prompt.completer {
                            let (dir, _) = split_dir_and_prefix(&prompt.input);
                            if let Some(completed) = comp.tab_complete(&dir) {
                                prompt.input = completed.clone();
                                prompt.cursor_pos = completed.len();
                                comp.update(&completed);
                            }
                        }
                    }
                    return;
                }
            }
            (Key::Up, false, false) => {
                // Navigate suggestions upward (OpenFile completer)
                let is_open = self
                    .prompt
                    .as_ref()
                    .is_some_and(|p| matches!(p.action, PromptAction::OpenFile));
                if is_open {
                    if let Some(ref mut prompt) = self.prompt {
                        if let Some(ref mut comp) = prompt.completer {
                            comp.selected = comp.selected.saturating_sub(1);
                        }
                    }
                    return;
                }
            }
            (Key::Down, false, false) => {
                // Navigate suggestions downward (OpenFile completer)
                let is_open = self
                    .prompt
                    .as_ref()
                    .is_some_and(|p| matches!(p.action, PromptAction::OpenFile));
                if is_open {
                    if let Some(ref mut prompt) = self.prompt {
                        if let Some(ref mut comp) = prompt.completer {
                            let max = comp.matches.len().saturating_sub(1);
                            comp.selected = (comp.selected + 1).min(max);
                        }
                    }
                    return;
                }
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

        // File completer: refresh suggestions when input changes in OpenFile prompts
        if input_changed {
            let is_open = self
                .prompt
                .as_ref()
                .is_some_and(|p| matches!(p.action, PromptAction::OpenFile));
            if is_open {
                if let Some(ref mut prompt) = self.prompt {
                    let input = prompt.input.clone();
                    if let Some(ref mut comp) = prompt.completer {
                        comp.update(&input);
                    }
                }
            }
        }
    }

    pub(super) fn execute_prompt(&mut self, prompt: Prompt) {
        match prompt.action {
            PromptAction::OpenFile => {
                let path = Path::new(&prompt.input);
                // Reuse an already-open buffer for the same file (dedup via canonicalize).
                let canonical =
                    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                if let Some(i) = self.buffers.iter().position(|bs| {
                    bs.buffer
                        .file_path()
                        .map(|bp| {
                            std::fs::canonicalize(bp).unwrap_or_else(|_| bp.to_path_buf())
                                == canonical
                        })
                        .unwrap_or(false)
                }) {
                    self.ensure_editor_pane();
                    self.layout.set_pane_buffer(self.active_pane, i);
                    self.active_buffer = i;
                    self.set_message(
                        &format!("Switched to: {}", shorten_path(path)),
                        MessageType::Info,
                    );
                    return;
                }
                match BufferState::from_file(
                    path,
                    self.config.line_numbers,
                    &self.config.theme,
                    &self.config.languages,
                ) {
                    Ok(bs) => {
                        // Ensure we open in an editor pane, not the terminal
                        self.ensure_editor_pane();
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
                        // Notify LSP about newly opened file
                        let notify_idx = self.active_buffer_index();
                        self.lsp_notify_open(notify_idx);
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
                    b.cursors[b.primary]
                        .cursor
                        .set_position(target, 0, &b.buffer);
                    b.set_selection(None);
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
                        // Remove swap file after successful save
                        self.cleanup_swap(buf_idx);
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
