use crate::input::{Key, KeyEvent};

use super::*;

// ---------------------------------------------------------------------------
// Palette action — every command the palette can execute
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaletteAction {
    // File
    Save,
    SaveAs,
    OpenFile,
    Quit,
    NewBuffer,
    CloseBuffer,
    // Edit
    Undo,
    Redo,
    DuplicateLine,
    DeleteLine,
    ToggleComment,
    Unindent,
    // Selection
    Copy,
    Cut,
    Paste,
    SelectAll,
    SelectLine,
    // Search
    Find,
    Replace,
    FindNext,
    FindPrev,
    // Multi-cursor
    SelectNextOccurrence,
    SelectAllOccurrences,
    // Navigate
    GoToLine,
    NextBuffer,
    PrevBuffer,
    // Pane
    SplitHorizontal,
    SplitVertical,
    ClosePane,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    // View
    ToggleHelp,
    ToggleWrap,
    ToggleFileTree,
    FocusFileTree,
    CommandPalette,
}

// ---------------------------------------------------------------------------
// Palette entry — label + shortcut + action
// ---------------------------------------------------------------------------

pub(super) struct PaletteEntry {
    pub label: &'static str,
    pub shortcut: &'static str,
    pub action: PaletteAction,
}

// ---------------------------------------------------------------------------
// Palette state
// ---------------------------------------------------------------------------

pub(super) struct Palette {
    pub input: String,
    pub cursor_pos: usize,
    entries: Vec<PaletteEntry>,
    pub filtered: Vec<usize>, // indices into entries
    pub selected: usize,      // index into filtered
    pub scroll_offset: usize,
}

impl Palette {
    pub fn new() -> Self {
        let entries = vec![
            // File
            PaletteEntry {
                label: "File: Save",
                shortcut: "Ctrl+S",
                action: PaletteAction::Save,
            },
            PaletteEntry {
                label: "File: Save As",
                shortcut: "Ctrl+Shift+S",
                action: PaletteAction::SaveAs,
            },
            PaletteEntry {
                label: "File: Open",
                shortcut: "Ctrl+O",
                action: PaletteAction::OpenFile,
            },
            PaletteEntry {
                label: "File: Quit",
                shortcut: "Ctrl+Q",
                action: PaletteAction::Quit,
            },
            PaletteEntry {
                label: "File: New Buffer",
                shortcut: "Ctrl+N",
                action: PaletteAction::NewBuffer,
            },
            PaletteEntry {
                label: "File: Close Buffer",
                shortcut: "Ctrl+W",
                action: PaletteAction::CloseBuffer,
            },
            // Edit
            PaletteEntry {
                label: "Edit: Undo",
                shortcut: "Ctrl+Z",
                action: PaletteAction::Undo,
            },
            PaletteEntry {
                label: "Edit: Redo",
                shortcut: "Ctrl+Y",
                action: PaletteAction::Redo,
            },
            PaletteEntry {
                label: "Edit: Duplicate Line",
                shortcut: "Ctrl+Shift+D",
                action: PaletteAction::DuplicateLine,
            },
            PaletteEntry {
                label: "Edit: Delete Line",
                shortcut: "Ctrl+Shift+K",
                action: PaletteAction::DeleteLine,
            },
            PaletteEntry {
                label: "Edit: Toggle Comment",
                shortcut: "Ctrl+/",
                action: PaletteAction::ToggleComment,
            },
            PaletteEntry {
                label: "Edit: Unindent",
                shortcut: "Shift+Tab",
                action: PaletteAction::Unindent,
            },
            // Selection
            PaletteEntry {
                label: "Selection: Copy",
                shortcut: "Ctrl+C",
                action: PaletteAction::Copy,
            },
            PaletteEntry {
                label: "Selection: Cut",
                shortcut: "Ctrl+X",
                action: PaletteAction::Cut,
            },
            PaletteEntry {
                label: "Selection: Paste",
                shortcut: "Ctrl+V",
                action: PaletteAction::Paste,
            },
            PaletteEntry {
                label: "Selection: Select All",
                shortcut: "Ctrl+A",
                action: PaletteAction::SelectAll,
            },
            PaletteEntry {
                label: "Selection: Select Line",
                shortcut: "Ctrl+L",
                action: PaletteAction::SelectLine,
            },
            // Search
            PaletteEntry {
                label: "Search: Find",
                shortcut: "Ctrl+F",
                action: PaletteAction::Find,
            },
            PaletteEntry {
                label: "Search: Replace",
                shortcut: "Ctrl+H",
                action: PaletteAction::Replace,
            },
            PaletteEntry {
                label: "Search: Find Next",
                shortcut: "F3",
                action: PaletteAction::FindNext,
            },
            PaletteEntry {
                label: "Search: Find Previous",
                shortcut: "Shift+F3",
                action: PaletteAction::FindPrev,
            },
            // Multi-cursor
            PaletteEntry {
                label: "Multi-Cursor: Select Next Occurrence",
                shortcut: "Ctrl+D",
                action: PaletteAction::SelectNextOccurrence,
            },
            PaletteEntry {
                label: "Multi-Cursor: Select All Occurrences",
                shortcut: "Ctrl+Shift+L",
                action: PaletteAction::SelectAllOccurrences,
            },
            // Navigate
            PaletteEntry {
                label: "Navigate: Go to Line",
                shortcut: "Ctrl+G",
                action: PaletteAction::GoToLine,
            },
            PaletteEntry {
                label: "Navigate: Next Buffer",
                shortcut: "Ctrl+PgDn",
                action: PaletteAction::NextBuffer,
            },
            PaletteEntry {
                label: "Navigate: Previous Buffer",
                shortcut: "Ctrl+PgUp",
                action: PaletteAction::PrevBuffer,
            },
            // Pane
            PaletteEntry {
                label: "Pane: Split Horizontal",
                shortcut: "Ctrl+\\",
                action: PaletteAction::SplitHorizontal,
            },
            PaletteEntry {
                label: "Pane: Split Vertical",
                shortcut: "Ctrl+Shift+\\",
                action: PaletteAction::SplitVertical,
            },
            PaletteEntry {
                label: "Pane: Close Pane",
                shortcut: "Ctrl+Shift+W",
                action: PaletteAction::ClosePane,
            },
            PaletteEntry {
                label: "Pane: Focus Left",
                shortcut: "Alt+Left",
                action: PaletteAction::FocusLeft,
            },
            PaletteEntry {
                label: "Pane: Focus Right",
                shortcut: "Alt+Right",
                action: PaletteAction::FocusRight,
            },
            PaletteEntry {
                label: "Pane: Focus Up",
                shortcut: "Alt+Up",
                action: PaletteAction::FocusUp,
            },
            PaletteEntry {
                label: "Pane: Focus Down",
                shortcut: "Alt+Down",
                action: PaletteAction::FocusDown,
            },
            // View
            PaletteEntry {
                label: "View: Toggle Help",
                shortcut: "F1",
                action: PaletteAction::ToggleHelp,
            },
            PaletteEntry {
                label: "View: Toggle Word Wrap",
                shortcut: "Alt+Z",
                action: PaletteAction::ToggleWrap,
            },
            PaletteEntry {
                label: "View: Toggle File Tree",
                shortcut: "Ctrl+B",
                action: PaletteAction::ToggleFileTree,
            },
            PaletteEntry {
                label: "View: Focus File Tree",
                shortcut: "",
                action: PaletteAction::FocusFileTree,
            },
            PaletteEntry {
                label: "View: Command Palette",
                shortcut: "Ctrl+Shift+P",
                action: PaletteAction::CommandPalette,
            },
        ];
        let filtered: Vec<usize> = (0..entries.len()).collect();
        Palette {
            input: String::new(),
            cursor_pos: 0,
            entries,
            filtered,
            selected: 0,
            scroll_offset: 0,
        }
    }

    /// Re-filter and sort entries based on current input.
    pub fn update_filter(&mut self) {
        if self.input.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
        } else {
            let mut scored: Vec<(usize, i32)> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, entry)| {
                    fuzzy_score(&self.input, entry.label).map(|(score, _)| (i, score))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Get the currently selected entry, if any.
    pub fn selected_entry(&self) -> Option<&PaletteEntry> {
        self.filtered
            .get(self.selected)
            .map(|&idx| &self.entries[idx])
    }

    /// Get an entry by its index in the entries vec.
    pub fn entry(&self, idx: usize) -> &PaletteEntry {
        &self.entries[idx]
    }

    /// Get matched character positions for highlighting.
    pub fn match_positions(&self, entry_idx: usize) -> Vec<usize> {
        if self.input.is_empty() {
            return Vec::new();
        }
        fuzzy_score(&self.input, self.entries[entry_idx].label)
            .map(|(_, positions)| positions)
            .unwrap_or_default()
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll_offset {
                self.scroll_offset = self.selected;
            }
        }
    }

    pub fn move_down(&mut self) {
        if !self.filtered.is_empty() && self.selected + 1 < self.filtered.len() {
            self.selected += 1;
            let max_visible = 10;
            if self.selected >= self.scroll_offset + max_visible {
                self.scroll_offset = self.selected - max_visible + 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

/// Case-insensitive fuzzy subsequence match with scoring.
/// Returns (score, matched_positions) or None if no match.
pub fn fuzzy_score(query: &str, target: &str) -> Option<(i32, Vec<usize>)> {
    let query_lower: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let target_chars: Vec<char> = target.chars().collect();
    let target_lower: Vec<char> = target.chars().flat_map(|c| c.to_lowercase()).collect();

    if query_lower.is_empty() {
        return Some((0, Vec::new()));
    }
    if query_lower.len() > target_lower.len() {
        return None;
    }

    // Find best match positions using greedy forward matching
    let mut positions = Vec::with_capacity(query_lower.len());
    let mut qi = 0;
    for (ti, &tc) in target_lower.iter().enumerate() {
        if qi < query_lower.len() && tc == query_lower[qi] {
            positions.push(ti);
            qi += 1;
        }
    }

    if qi < query_lower.len() {
        return None; // not all query chars matched
    }

    // Score the match
    let mut score: i32 = 0;
    for (match_idx, &pos) in positions.iter().enumerate() {
        // Base: +1 per matched char
        score += 1;

        // Consecutive bonus: +5 if previous match was at pos-1
        if match_idx > 0 && positions[match_idx - 1] + 1 == pos {
            score += 5;
        }

        // Word boundary bonus: +10 if at start or after space/colon/underscore
        if pos == 0 || matches!(target_chars[pos - 1], ' ' | ':' | '_' | '-') {
            score += 10;
        }

        // Penalty for distance from start
        if match_idx == 0 {
            score -= pos as i32;
        }
    }

    Some((score, positions))
}

// ---------------------------------------------------------------------------
// Input handling (on Editor)
// ---------------------------------------------------------------------------

impl Editor {
    pub(super) fn handle_palette_key(&mut self, ke: KeyEvent) {
        match (&ke.key, ke.ctrl, ke.alt) {
            (Key::Escape, _, _) => {
                self.palette = None;
            }
            (Key::Enter, false, false) => {
                if let Some(palette) = self.palette.take()
                    && let Some(entry) = palette.selected_entry()
                {
                    let action = entry.action;
                    self.execute_palette_action(action);
                }
            }
            (Key::Up, false, false) => {
                if let Some(ref mut palette) = self.palette {
                    palette.move_up();
                }
            }
            (Key::Down, false, false) => {
                if let Some(ref mut palette) = self.palette {
                    palette.move_down();
                }
            }
            (Key::Backspace, false, false) => {
                if let Some(ref mut palette) = self.palette
                    && palette.cursor_pos > 0
                {
                    let before = &palette.input[..palette.cursor_pos];
                    if let Some(ch) = before.chars().next_back() {
                        let len = ch.len_utf8();
                        let new_pos = palette.cursor_pos - len;
                        palette.input.drain(new_pos..palette.cursor_pos);
                        palette.cursor_pos = new_pos;
                        palette.update_filter();
                    }
                }
            }
            (Key::Delete, false, false) => {
                if let Some(ref mut palette) = self.palette
                    && palette.cursor_pos < palette.input.len()
                {
                    let after = &palette.input[palette.cursor_pos..];
                    if let Some(ch) = after.chars().next() {
                        let len = ch.len_utf8();
                        palette
                            .input
                            .drain(palette.cursor_pos..palette.cursor_pos + len);
                        palette.update_filter();
                    }
                }
            }
            (Key::Left, false, false) => {
                if let Some(ref mut palette) = self.palette
                    && palette.cursor_pos > 0
                {
                    let before = &palette.input[..palette.cursor_pos];
                    if let Some(ch) = before.chars().next_back() {
                        palette.cursor_pos -= ch.len_utf8();
                    }
                }
            }
            (Key::Right, false, false) => {
                if let Some(ref mut palette) = self.palette
                    && palette.cursor_pos < palette.input.len()
                {
                    let after = &palette.input[palette.cursor_pos..];
                    if let Some(ch) = after.chars().next() {
                        palette.cursor_pos += ch.len_utf8();
                    }
                }
            }
            (Key::Home, false, false) => {
                if let Some(ref mut palette) = self.palette {
                    palette.cursor_pos = 0;
                }
            }
            (Key::End, false, false) => {
                if let Some(ref mut palette) = self.palette {
                    palette.cursor_pos = palette.input.len();
                }
            }
            (Key::Char(ch), false, false) => {
                if let Some(ref mut palette) = self.palette {
                    let mut buf = [0u8; 4];
                    let s = ch.encode_utf8(&mut buf);
                    palette.input.insert_str(palette.cursor_pos, s);
                    palette.cursor_pos += s.len();
                    palette.update_filter();
                }
            }
            _ => {}
        }
    }

    pub(super) fn execute_palette_action(&mut self, action: PaletteAction) {
        use PaletteAction::*;
        match action {
            Save => self.save(),
            SaveAs => self.start_prompt("Save as: ", PromptAction::SaveAs),
            OpenFile => self.start_prompt("Open: ", PromptAction::OpenFile),
            Quit => self.quit(),
            NewBuffer => self.new_buffer(),
            CloseBuffer => self.close_buffer(),
            Undo => self.do_undo(),
            Redo => self.do_redo(),
            DuplicateLine => self.duplicate_line(),
            DeleteLine => self.delete_line(),
            ToggleComment => self.toggle_comment(),
            Unindent => self.unindent(),
            Copy => self.copy_selection(),
            Cut => self.cut_selection(),
            Paste => self.paste_clipboard(),
            SelectAll => self.select_all(),
            SelectLine => self.select_line(),
            Find => self.open_find_prompt(PromptAction::Find),
            Replace => self.open_find_prompt(PromptAction::Replace),
            FindNext => self.search_next(),
            FindPrev => self.search_prev(),
            SelectNextOccurrence => self.select_next_occurrence(),
            SelectAllOccurrences => self.select_all_occurrences(),
            GoToLine => self.start_prompt("Go to line: ", PromptAction::GoToLine),
            NextBuffer => self.next_buffer(),
            PrevBuffer => self.prev_buffer(),
            SplitHorizontal => self.split_pane_horizontal(),
            SplitVertical => self.split_pane_vertical(),
            ClosePane => self.close_active_pane(),
            FocusLeft => self.focus_pane(crate::layout::Direction::Left),
            FocusRight => self.focus_pane(crate::layout::Direction::Right),
            FocusUp => self.focus_pane(crate::layout::Direction::Up),
            FocusDown => self.focus_pane(crate::layout::Direction::Down),
            ToggleHelp => {
                self.help_visible = !self.help_visible;
            }
            ToggleWrap => self.toggle_word_wrap(),
            ToggleFileTree => self.toggle_filetree(),
            FocusFileTree => {
                if self.filetree.is_none() {
                    self.toggle_filetree();
                } else {
                    self.filetree_focused = true;
                }
            }
            CommandPalette => {
                // Re-open palette (already closed by taking it)
                self.palette = Some(Palette::new());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_exact_match() {
        let result = fuzzy_score("save", "File: Save");
        assert!(result.is_some());
        let (score, positions) = result.unwrap();
        assert!(score > 0);
        assert_eq!(positions.len(), 4);
    }

    #[test]
    fn test_fuzzy_no_match() {
        assert!(fuzzy_score("xyz", "File: Save").is_none());
    }

    #[test]
    fn test_fuzzy_case_insensitive() {
        let result = fuzzy_score("SAVE", "File: Save");
        assert!(result.is_some());
    }

    #[test]
    fn test_fuzzy_subsequence() {
        let result = fuzzy_score("fs", "File: Save");
        assert!(result.is_some());
        let (_, positions) = result.unwrap();
        assert_eq!(positions.len(), 2);
    }

    #[test]
    fn test_fuzzy_empty_query() {
        let result = fuzzy_score("", "File: Save");
        assert!(result.is_some());
        let (score, positions) = result.unwrap();
        assert_eq!(score, 0);
        assert!(positions.is_empty());
    }

    #[test]
    fn test_fuzzy_word_boundary_bonus() {
        // "s" at word boundary ("Save") should score higher than mid-word
        let score_boundary = fuzzy_score("s", "File: Save").unwrap().0;
        let score_mid = fuzzy_score("i", "File: Save").unwrap().0;
        assert!(score_boundary > score_mid);
    }

    #[test]
    fn test_fuzzy_consecutive_bonus() {
        // "sav" consecutive should score higher than "s_a_v" scattered
        let score_consec = fuzzy_score("sav", "File: Save").unwrap().0;
        let score_scatter = fuzzy_score("fae", "File: Save").unwrap().0;
        assert!(score_consec > score_scatter);
    }

    #[test]
    fn test_palette_filter() {
        let mut p = Palette::new();
        p.input = "save".to_string();
        p.cursor_pos = 4;
        p.update_filter();
        assert!(!p.filtered.is_empty());
        // First result should be Save or Save As
        let first = p.selected_entry().unwrap();
        assert!(first.label.contains("Save"));
    }

    #[test]
    fn test_palette_empty_filter_shows_all() {
        let p = Palette::new();
        assert_eq!(p.filtered.len(), p.entries.len());
    }

    #[test]
    fn test_palette_navigation() {
        let mut p = Palette::new();
        assert_eq!(p.selected, 0);
        p.move_down();
        assert_eq!(p.selected, 1);
        p.move_down();
        assert_eq!(p.selected, 2);
        p.move_up();
        assert_eq!(p.selected, 1);
        p.move_up();
        assert_eq!(p.selected, 0);
        p.move_up(); // should stay at 0
        assert_eq!(p.selected, 0);
    }
}
