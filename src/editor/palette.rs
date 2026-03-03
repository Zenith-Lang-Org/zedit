use crate::input::{Key, KeyEvent};
use crate::keybindings::{EditorAction, KeyMap};

use super::*;

// ---------------------------------------------------------------------------
// Palette action — every command the palette can execute
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
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
    // Terminal
    ToggleTerminal,
    NewTerminal,
    // Import a VS Code extension
    ImportExtension,
    // Plugin commands (command_id stored inline)
    PluginCommand(String),
}

// ---------------------------------------------------------------------------
// Palette entry — label + shortcut + action
// ---------------------------------------------------------------------------

pub(super) struct PaletteEntry {
    pub label: String,
    pub shortcut: String,
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
    pub fn new(keymap: &KeyMap) -> Self {
        // Each entry: (label, PaletteAction, EditorAction for shortcut lookup)
        let defs: &[(&str, PaletteAction, Option<EditorAction>)] = &[
            // File
            ("File: Save", PaletteAction::Save, Some(EditorAction::Save)),
            (
                "File: Save As",
                PaletteAction::SaveAs,
                Some(EditorAction::SaveAs),
            ),
            (
                "File: Open",
                PaletteAction::OpenFile,
                Some(EditorAction::OpenFile),
            ),
            ("File: Quit", PaletteAction::Quit, Some(EditorAction::Quit)),
            (
                "File: New Buffer",
                PaletteAction::NewBuffer,
                Some(EditorAction::NewBuffer),
            ),
            (
                "File: Close Buffer",
                PaletteAction::CloseBuffer,
                Some(EditorAction::CloseBuffer),
            ),
            // Edit
            ("Edit: Undo", PaletteAction::Undo, Some(EditorAction::Undo)),
            ("Edit: Redo", PaletteAction::Redo, Some(EditorAction::Redo)),
            (
                "Edit: Duplicate Line",
                PaletteAction::DuplicateLine,
                Some(EditorAction::DuplicateLine),
            ),
            (
                "Edit: Delete Line",
                PaletteAction::DeleteLine,
                Some(EditorAction::DeleteLine),
            ),
            (
                "Edit: Toggle Comment",
                PaletteAction::ToggleComment,
                Some(EditorAction::ToggleComment),
            ),
            (
                "Edit: Unindent",
                PaletteAction::Unindent,
                Some(EditorAction::Unindent),
            ),
            // Selection
            (
                "Selection: Copy",
                PaletteAction::Copy,
                Some(EditorAction::Copy),
            ),
            (
                "Selection: Cut",
                PaletteAction::Cut,
                Some(EditorAction::Cut),
            ),
            (
                "Selection: Paste",
                PaletteAction::Paste,
                Some(EditorAction::Paste),
            ),
            (
                "Selection: Select All",
                PaletteAction::SelectAll,
                Some(EditorAction::SelectAll),
            ),
            (
                "Selection: Select Line",
                PaletteAction::SelectLine,
                Some(EditorAction::SelectLine),
            ),
            // Search
            (
                "Search: Find",
                PaletteAction::Find,
                Some(EditorAction::Find),
            ),
            (
                "Search: Replace",
                PaletteAction::Replace,
                Some(EditorAction::Replace),
            ),
            (
                "Search: Find Next",
                PaletteAction::FindNext,
                Some(EditorAction::FindNext),
            ),
            (
                "Search: Find Previous",
                PaletteAction::FindPrev,
                Some(EditorAction::FindPrev),
            ),
            // Multi-cursor
            (
                "Multi-Cursor: Select Next Occurrence",
                PaletteAction::SelectNextOccurrence,
                Some(EditorAction::SelectNextOccurrence),
            ),
            (
                "Multi-Cursor: Select All Occurrences",
                PaletteAction::SelectAllOccurrences,
                Some(EditorAction::SelectAllOccurrences),
            ),
            // Navigate
            (
                "Navigate: Go to Line",
                PaletteAction::GoToLine,
                Some(EditorAction::GoToLine),
            ),
            (
                "Navigate: Next Buffer",
                PaletteAction::NextBuffer,
                Some(EditorAction::NextBuffer),
            ),
            (
                "Navigate: Previous Buffer",
                PaletteAction::PrevBuffer,
                Some(EditorAction::PrevBuffer),
            ),
            // Pane
            (
                "Pane: Split Horizontal",
                PaletteAction::SplitHorizontal,
                Some(EditorAction::SplitHorizontal),
            ),
            (
                "Pane: Split Vertical",
                PaletteAction::SplitVertical,
                Some(EditorAction::SplitVertical),
            ),
            (
                "Pane: Close Pane",
                PaletteAction::ClosePane,
                Some(EditorAction::ClosePane),
            ),
            (
                "Pane: Focus Left",
                PaletteAction::FocusLeft,
                Some(EditorAction::FocusLeft),
            ),
            (
                "Pane: Focus Right",
                PaletteAction::FocusRight,
                Some(EditorAction::FocusRight),
            ),
            (
                "Pane: Focus Up",
                PaletteAction::FocusUp,
                Some(EditorAction::FocusUp),
            ),
            (
                "Pane: Focus Down",
                PaletteAction::FocusDown,
                Some(EditorAction::FocusDown),
            ),
            // View
            (
                "View: Toggle Help",
                PaletteAction::ToggleHelp,
                Some(EditorAction::ToggleHelp),
            ),
            (
                "View: Toggle Word Wrap",
                PaletteAction::ToggleWrap,
                Some(EditorAction::ToggleWrap),
            ),
            (
                "View: Toggle File Tree",
                PaletteAction::ToggleFileTree,
                Some(EditorAction::ToggleFileTree),
            ),
            ("View: Focus File Tree", PaletteAction::FocusFileTree, None),
            (
                "View: Command Palette",
                PaletteAction::CommandPalette,
                Some(EditorAction::CommandPalette),
            ),
            // Terminal
            (
                "Terminal: Toggle Terminal",
                PaletteAction::ToggleTerminal,
                Some(EditorAction::ToggleTerminal),
            ),
            (
                "Terminal: New Terminal",
                PaletteAction::NewTerminal,
                Some(EditorAction::NewTerminal),
            ),
            // Extensions
            (
                "Extensions: Import VS Code Extension...",
                PaletteAction::ImportExtension,
                Some(EditorAction::ImportExtension),
            ),
        ];

        let entries: Vec<PaletteEntry> = defs
            .iter()
            .map(|&(label, ref action, editor_action)| {
                let shortcut = editor_action
                    .map(|ea| keymap.label(ea).to_string())
                    .unwrap_or_default();
                PaletteEntry {
                    label: label.to_string(),
                    shortcut,
                    action: action.clone(),
                }
            })
            .collect();

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

    /// Append plugin commands to the palette entries and re-filter.
    pub fn add_plugin_commands(&mut self, commands: &[(String, String, String)]) {
        // commands: Vec<(plugin_name, command_id, label)>
        for (plugin_name, cmd_id, label) in commands {
            let display = format!("Plugin ({}): {}", plugin_name, label);
            self.entries.push(PaletteEntry {
                label: display,
                shortcut: String::new(),
                action: PaletteAction::PluginCommand(cmd_id.clone()),
            });
        }
        // Re-build the full filter
        if self.input.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
        } else {
            self.update_filter();
        }
    }

    /// Remove all plugin commands and replace with a new set.
    pub fn replace_plugin_commands(&mut self, commands: &[(String, String, String)]) {
        self.entries
            .retain(|e| !matches!(e.action, PaletteAction::PluginCommand(_)));
        self.add_plugin_commands(commands);
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
                    fuzzy_score(&self.input, &entry.label).map(|(score, _)| (i, score))
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
        fuzzy_score(&self.input, &self.entries[entry_idx].label)
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
///
/// Two-phase algorithm:
///   Phase A — greedy forward scan: confirms the query is matchable at all.
///   Phase B — rightward clustering: pushes each match position as far right
///              as possible to maximise consecutive runs and word-boundary hits.
pub fn fuzzy_score(query: &str, target: &str) -> Option<(i32, Vec<usize>)> {
    let qc: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let tc: Vec<char> = target.chars().collect();
    let tc_lower: Vec<char> = target.chars().flat_map(|c| c.to_lowercase()).collect();

    if qc.is_empty() {
        return Some((0, Vec::new()));
    }
    if qc.len() > tc_lower.len() {
        return None;
    }

    // Phase A: greedy forward — bail out early if no full match exists.
    let mut positions = Vec::with_capacity(qc.len());
    let mut qi = 0;
    for (ti, &ch) in tc_lower.iter().enumerate() {
        if qi < qc.len() && ch == qc[qi] {
            positions.push(ti);
            qi += 1;
        }
    }
    if positions.len() < qc.len() {
        return None;
    }

    // Phase B: push each match rightward to cluster consecutive runs.
    // Iterating in reverse ensures positions[i+1] is already finalised
    // when we process positions[i].
    let n = positions.len();
    for i in (0..n).rev() {
        let lower_bound = if i == 0 { 0 } else { positions[i - 1] + 1 };
        let upper_bound = if i + 1 < n {
            positions[i + 1].saturating_sub(1)
        } else {
            tc_lower.len().saturating_sub(1)
        };

        let mut best = positions[i];
        for ti in (lower_bound..=upper_bound).rev() {
            if tc_lower[ti] == qc[i] {
                let consec_next = i + 1 < n && ti + 1 == positions[i + 1];
                let consec_prev = i > 0 && positions[i - 1] + 1 == ti;
                if consec_next || consec_prev {
                    best = ti;
                    break;
                }
            }
        }
        positions[i] = best;
    }

    // Scoring
    let mut score: i32 = 0;
    for (mi, &pos) in positions.iter().enumerate() {
        // Base: +1 per matched char
        score += 1;

        // Consecutive bonus: +5 if previous match was immediately before this one
        if mi > 0 && positions[mi - 1] + 1 == pos {
            score += 5;
        }

        // Word boundary bonus: +10 if at start or after a delimiter
        let at_boundary = pos == 0 || matches!(tc[pos - 1], ' ' | ':' | '_' | '-' | '/' | '.');
        if at_boundary {
            score += 10;
        }

        // Mild prefix penalty: discourage matches that start far into the target
        if mi == 0 {
            score -= pos as i32 / 2;
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
                    let action = entry.action.clone();
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
            OpenFile => self.start_open_file_prompt(""),
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
                let mut p = Palette::new(&self.config.keybindings);
                if let Some(ref mgr) = self.plugin_manager {
                    let cmds: Vec<(String, String, String)> = mgr
                        .all_commands()
                        .iter()
                        .map(|(pname, cmd)| (pname.clone(), cmd.id.clone(), cmd.label.clone()))
                        .collect();
                    p.add_plugin_commands(&cmds);
                }
                self.palette = Some(p);
            }
            ToggleTerminal => self.toggle_terminal_panel(),
            NewTerminal => self.new_terminal(),
            ImportExtension => {
                self.start_prompt(
                    "Extension (ID like 'haskell.haskell', URL, or .vsix path): ",
                    PromptAction::ImportExtension,
                );
            }
            PluginCommand(cmd_id) => {
                if let Some(ref mut mgr) = self.plugin_manager {
                    mgr.invoke_command(&cmd_id);
                }
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
    fn test_fuzzy_phase_b_clusters_consecutive() {
        // "ab" in "xaabb": Phase A finds [1,3], Phase B should pull a to index 2
        // giving consecutive [2,3] which scores higher.
        let (score_new, positions) = fuzzy_score("ab", "xaabb").unwrap();
        assert_eq!(
            positions,
            vec![2, 3],
            "Phase B should cluster to consecutive positions"
        );
        // Verify score is higher than what Phase A alone would give ([1,3])
        // [1,3]: base 2, no consecutive bonus = 2;  [2,3]: base 2 + consecutive 5 = 7
        assert!(
            score_new > 2,
            "clustered match should outscore scattered match"
        );
    }

    #[test]
    fn test_fuzzy_slash_dot_word_boundary() {
        // '/' and '.' should trigger the word boundary bonus
        let score_slash = fuzzy_score("m", "src/main.rs").unwrap().0;
        let score_dot = fuzzy_score("r", "main.rs").unwrap().0;
        // 'm' after '/' and 'r' after '.' are both at boundaries
        let score_mid = fuzzy_score("a", "main.rs").unwrap().0;
        assert!(score_slash > score_mid, "'/' should confer boundary bonus");
        assert!(score_dot > score_mid, "'.' should confer boundary bonus");
    }

    #[test]
    fn test_fuzzy_prefix_penalty_halved() {
        // With the halved penalty (pos/2), "s" matching at index 6 costs 3, not 6.
        // The word boundary bonus (+10) should still dominate.
        let (score, positions) = fuzzy_score("s", "File: Save").unwrap();
        assert_eq!(positions, vec![6]);
        // score = 1 (base) + 10 (boundary) - 3 (penalty 6/2) = 8
        assert_eq!(score, 8);
    }

    #[test]
    fn test_palette_filter() {
        let mut p = Palette::new(&KeyMap::defaults());
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
        let p = Palette::new(&KeyMap::defaults());
        assert_eq!(p.filtered.len(), p.entries.len());
    }

    #[test]
    fn test_palette_navigation() {
        let mut p = Palette::new(&KeyMap::defaults());
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
