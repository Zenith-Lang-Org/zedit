use std::collections::HashMap;

use crate::input::{Key, KeyEvent};
use crate::syntax::json_parser::JsonValue;

// ---------------------------------------------------------------------------
// EditorAction — every bindable action
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorAction {
    Save,
    SaveAs,
    OpenFile,
    Quit,
    NewBuffer,
    CloseBuffer,
    Undo,
    Redo,
    DuplicateLine,
    DeleteLine,
    ToggleComment,
    Unindent,
    Copy,
    Cut,
    Paste,
    SelectAll,
    SelectLine,
    SelectNextOccurrence,
    SelectAllOccurrences,
    Find,
    Replace,
    FindNext,
    FindPrev,
    GoToLine,
    NextBuffer,
    PrevBuffer,
    SplitHorizontal,
    SplitVertical,
    ClosePane,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    ResizePaneLeft,
    ResizePaneRight,
    ResizePaneUp,
    ResizePaneDown,
    ToggleHelp,
    ToggleWrap,
    ToggleFileTree,
    FocusFileTree,
    CommandPalette,
    ToggleTerminal,
    NewTerminal,
    LspComplete,
    LspHover,
    LspGoToDef,
    DiffOpenVsHead,
    DiffNextHunk,
    DiffPrevHunk,
    ToggleMinimap,
    // Task runner
    TaskRun,
    TaskBuild,
    TaskTest,
    TaskStop,
}

// ---------------------------------------------------------------------------
// NormalizedKey + KeyBinding — hashable key representation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NormalizedKey {
    Char(char),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    pub key: NormalizedKey,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl KeyBinding {
    /// Convert a `KeyEvent` to a `KeyBinding` for lookup.
    pub fn from_key_event(ke: &KeyEvent) -> Self {
        let (key, shift) = match &ke.key {
            Key::Char(ch) => {
                // For Ctrl+letter: uppercase means shift was held, but terminals
                // absorb shift into case. We normalize: Ctrl+Shift+S arrives as
                // Char('S') ctrl=true. We store that as Char('S') ctrl=true shift=false.
                // Plain Ctrl+S arrives as Char('s') ctrl=true.
                (NormalizedKey::Char(*ch), ke.shift)
            }
            Key::Enter => (NormalizedKey::Enter, ke.shift),
            Key::Tab => (NormalizedKey::Tab, ke.shift),
            Key::BackTab => (NormalizedKey::BackTab, true),
            Key::Backspace => (NormalizedKey::Backspace, ke.shift),
            Key::Delete => (NormalizedKey::Delete, ke.shift),
            Key::Escape => (NormalizedKey::Escape, ke.shift),
            Key::Up => (NormalizedKey::Up, ke.shift),
            Key::Down => (NormalizedKey::Down, ke.shift),
            Key::Left => (NormalizedKey::Left, ke.shift),
            Key::Right => (NormalizedKey::Right, ke.shift),
            Key::Home => (NormalizedKey::Home, ke.shift),
            Key::End => (NormalizedKey::End, ke.shift),
            Key::PageUp => (NormalizedKey::PageUp, ke.shift),
            Key::PageDown => (NormalizedKey::PageDown, ke.shift),
            Key::F(n) => (NormalizedKey::F(*n), ke.shift),
        };
        KeyBinding {
            key,
            ctrl: ke.ctrl,
            alt: ke.alt,
            shift,
        }
    }

    /// Format for display in the palette (e.g. "Ctrl+Shift+S").
    pub fn to_display_string(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        let key_str = match &self.key {
            NormalizedKey::Char(ch) => {
                // For display: if ctrl and uppercase, show Shift+<lower>
                // But shift is already tracked separately for chars
                if self.ctrl && ch.is_ascii_uppercase() {
                    // Ctrl+Shift+S: ctrl=true, shift=false, Char('S')
                    // Display as Ctrl+Shift+S
                    // We need to add Shift if not already added
                    if !self.shift {
                        // Insert Shift before the key
                        let mut p = parts.clone();
                        // Check if Shift is already there
                        if !p.contains(&"Shift") {
                            p.insert(p.len(), "Shift");
                        }
                        p.push("_"); // placeholder
                        let key_part = ch.to_ascii_uppercase().to_string();
                        let mut result: Vec<String> =
                            p[..p.len() - 1].iter().map(|s| s.to_string()).collect();
                        result.push(key_part);
                        return result.join("+");
                    }
                }
                match *ch {
                    '`' => "`".to_string(),
                    '\\' => "\\".to_string(),
                    '/' => "/".to_string(),
                    _ => {
                        if ch.is_ascii_uppercase() && !self.ctrl {
                            ch.to_string()
                        } else {
                            ch.to_ascii_uppercase().to_string()
                        }
                    }
                }
            }
            NormalizedKey::Enter => "Enter".to_string(),
            NormalizedKey::Tab => "Tab".to_string(),
            NormalizedKey::BackTab => {
                // BackTab is Shift+Tab, but shift is already set
                // Remove Shift from parts and show as Shift+Tab
                "Tab".to_string()
            }
            NormalizedKey::Backspace => "Backspace".to_string(),
            NormalizedKey::Delete => "Delete".to_string(),
            NormalizedKey::Escape => "Escape".to_string(),
            NormalizedKey::Up => "Up".to_string(),
            NormalizedKey::Down => "Down".to_string(),
            NormalizedKey::Left => "Left".to_string(),
            NormalizedKey::Right => "Right".to_string(),
            NormalizedKey::Home => "Home".to_string(),
            NormalizedKey::End => "End".to_string(),
            NormalizedKey::PageUp => "PgUp".to_string(),
            NormalizedKey::PageDown => "PgDn".to_string(),
            NormalizedKey::F(n) => format!("F{}", n),
        };
        parts.push("_"); // placeholder for key
        let mut result: Vec<String> = parts[..parts.len() - 1]
            .iter()
            .map(|s| s.to_string())
            .collect();
        result.push(key_str);
        result.join("+")
    }
}

// ---------------------------------------------------------------------------
// Parse key string: "Ctrl+Shift+T" → KeyBinding
// ---------------------------------------------------------------------------

/// Parse a human-readable key binding string into a `KeyBinding`.
///
/// Format: `[Ctrl+][Alt+][Shift+]<key>`
/// Key names: single char, Enter, Tab, Backspace, Delete, Escape,
/// Up, Down, Left, Right, Home, End, PgUp, PgDn, PageUp, PageDown,
/// F1-F12, Backtick/`, Backslash/\, Slash//.
pub fn parse_key_string(s: &str) -> Option<KeyBinding> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let parts: Vec<&str> = s.split('+').collect();
    if parts.is_empty() {
        return None;
    }

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;

    // All parts except last are modifiers
    for &part in &parts[..parts.len() - 1] {
        match part.trim().to_lowercase().as_str() {
            "ctrl" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            _ => return None, // unknown modifier
        }
    }

    let key_part = parts.last()?.trim();

    let key = match key_part.to_lowercase().as_str() {
        "enter" | "return" => NormalizedKey::Enter,
        "tab" => {
            if shift {
                shift = true;
                NormalizedKey::BackTab
            } else {
                NormalizedKey::Tab
            }
        }
        "backspace" | "bs" => NormalizedKey::Backspace,
        "delete" | "del" => NormalizedKey::Delete,
        "escape" | "esc" => NormalizedKey::Escape,
        "up" => NormalizedKey::Up,
        "down" => NormalizedKey::Down,
        "left" => NormalizedKey::Left,
        "right" => NormalizedKey::Right,
        "home" => NormalizedKey::Home,
        "end" => NormalizedKey::End,
        "pageup" | "pgup" => NormalizedKey::PageUp,
        "pagedown" | "pgdn" => NormalizedKey::PageDown,
        "f1" => NormalizedKey::F(1),
        "f2" => NormalizedKey::F(2),
        "f3" => NormalizedKey::F(3),
        "f4" => NormalizedKey::F(4),
        "f5" => NormalizedKey::F(5),
        "f6" => NormalizedKey::F(6),
        "f7" => NormalizedKey::F(7),
        "f8" => NormalizedKey::F(8),
        "f9" => NormalizedKey::F(9),
        "f10" => NormalizedKey::F(10),
        "f11" => NormalizedKey::F(11),
        "f12" => NormalizedKey::F(12),
        "backtick" | "`" => NormalizedKey::Char('`'),
        "backslash" | "\\" => NormalizedKey::Char('\\'),
        "slash" | "/" => NormalizedKey::Char('/'),
        "space" => NormalizedKey::Char(' '),
        _ => {
            // Single character
            let chars: Vec<char> = key_part.chars().collect();
            if chars.len() == 1 {
                let ch = chars[0];
                if ctrl && shift && ch.is_ascii_alphabetic() {
                    // Ctrl+Shift+S → Char('S'), ctrl=true, shift=false
                    // because terminals send uppercase with ctrl
                    shift = false;
                    NormalizedKey::Char(ch.to_ascii_uppercase())
                } else if ctrl && ch.is_ascii_alphabetic() {
                    NormalizedKey::Char(ch.to_ascii_lowercase())
                } else if ch.is_ascii_alphabetic() && !shift {
                    // Non-ctrl, non-shift letter: lowercase (terminals send lowercase)
                    NormalizedKey::Char(ch.to_ascii_lowercase())
                } else {
                    NormalizedKey::Char(ch)
                }
            } else {
                return None;
            }
        }
    };

    Some(KeyBinding {
        key,
        ctrl,
        alt,
        shift,
    })
}

// ---------------------------------------------------------------------------
// Action name mapping
// ---------------------------------------------------------------------------

fn action_name_to_action(name: &str) -> Option<EditorAction> {
    match name {
        "save" => Some(EditorAction::Save),
        "save_as" => Some(EditorAction::SaveAs),
        "open_file" => Some(EditorAction::OpenFile),
        "quit" => Some(EditorAction::Quit),
        "new_buffer" => Some(EditorAction::NewBuffer),
        "close_buffer" => Some(EditorAction::CloseBuffer),
        "undo" => Some(EditorAction::Undo),
        "redo" => Some(EditorAction::Redo),
        "duplicate_line" => Some(EditorAction::DuplicateLine),
        "delete_line" => Some(EditorAction::DeleteLine),
        "toggle_comment" => Some(EditorAction::ToggleComment),
        "unindent" => Some(EditorAction::Unindent),
        "copy" => Some(EditorAction::Copy),
        "cut" => Some(EditorAction::Cut),
        "paste" => Some(EditorAction::Paste),
        "select_all" => Some(EditorAction::SelectAll),
        "select_line" => Some(EditorAction::SelectLine),
        "select_next_occurrence" => Some(EditorAction::SelectNextOccurrence),
        "select_all_occurrences" => Some(EditorAction::SelectAllOccurrences),
        "find" => Some(EditorAction::Find),
        "replace" => Some(EditorAction::Replace),
        "find_next" => Some(EditorAction::FindNext),
        "find_prev" => Some(EditorAction::FindPrev),
        "go_to_line" => Some(EditorAction::GoToLine),
        "next_buffer" => Some(EditorAction::NextBuffer),
        "prev_buffer" => Some(EditorAction::PrevBuffer),
        "split_horizontal" => Some(EditorAction::SplitHorizontal),
        "split_vertical" => Some(EditorAction::SplitVertical),
        "close_pane" => Some(EditorAction::ClosePane),
        "focus_left" => Some(EditorAction::FocusLeft),
        "focus_right" => Some(EditorAction::FocusRight),
        "focus_up" => Some(EditorAction::FocusUp),
        "focus_down" => Some(EditorAction::FocusDown),
        "resize_pane_left" => Some(EditorAction::ResizePaneLeft),
        "resize_pane_right" => Some(EditorAction::ResizePaneRight),
        "resize_pane_up" => Some(EditorAction::ResizePaneUp),
        "resize_pane_down" => Some(EditorAction::ResizePaneDown),
        "toggle_help" => Some(EditorAction::ToggleHelp),
        "toggle_wrap" => Some(EditorAction::ToggleWrap),
        "toggle_file_tree" => Some(EditorAction::ToggleFileTree),
        "focus_file_tree" => Some(EditorAction::FocusFileTree),
        "command_palette" => Some(EditorAction::CommandPalette),
        "toggle_terminal" => Some(EditorAction::ToggleTerminal),
        "new_terminal" => Some(EditorAction::NewTerminal),
        "lsp_complete" => Some(EditorAction::LspComplete),
        "lsp_hover" => Some(EditorAction::LspHover),
        "lsp_go_to_def" => Some(EditorAction::LspGoToDef),
        "diff_open_vs_head" => Some(EditorAction::DiffOpenVsHead),
        "diff_next_hunk" => Some(EditorAction::DiffNextHunk),
        "diff_prev_hunk" => Some(EditorAction::DiffPrevHunk),
        "toggle_minimap" => Some(EditorAction::ToggleMinimap),
        "task_run" => Some(EditorAction::TaskRun),
        "task_build" => Some(EditorAction::TaskBuild),
        "task_test" => Some(EditorAction::TaskTest),
        "task_stop" => Some(EditorAction::TaskStop),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// KeyMap
// ---------------------------------------------------------------------------

pub struct KeyMap {
    bindings: HashMap<KeyBinding, EditorAction>,
    labels: HashMap<EditorAction, String>,
}

impl KeyMap {
    /// Build the default key map.
    pub fn defaults() -> Self {
        let table: &[(&str, EditorAction)] = &[
            // File
            ("Ctrl+S", EditorAction::Save),
            ("Ctrl+Shift+S", EditorAction::SaveAs),
            ("Ctrl+O", EditorAction::OpenFile),
            ("Ctrl+Q", EditorAction::Quit),
            ("Ctrl+N", EditorAction::NewBuffer),
            ("Ctrl+W", EditorAction::CloseBuffer),
            // Edit
            ("Ctrl+Z", EditorAction::Undo),
            ("Ctrl+Y", EditorAction::Redo),
            ("Ctrl+Shift+D", EditorAction::DuplicateLine),
            ("Ctrl+Shift+K", EditorAction::DeleteLine),
            ("Ctrl+/", EditorAction::ToggleComment),
            ("Shift+Tab", EditorAction::Unindent),
            // Clipboard / Selection
            ("Ctrl+C", EditorAction::Copy),
            ("Ctrl+X", EditorAction::Cut),
            ("Ctrl+V", EditorAction::Paste),
            ("Ctrl+A", EditorAction::SelectAll),
            ("Ctrl+L", EditorAction::SelectLine),
            ("Ctrl+D", EditorAction::SelectNextOccurrence),
            ("Ctrl+Shift+L", EditorAction::SelectAllOccurrences),
            // Search
            ("Ctrl+F", EditorAction::Find),
            ("Ctrl+H", EditorAction::Replace),
            ("F3", EditorAction::FindNext),
            ("Shift+F3", EditorAction::FindPrev),
            // Navigate
            ("Ctrl+G", EditorAction::GoToLine),
            ("Ctrl+PgDn", EditorAction::NextBuffer),
            ("Ctrl+PgUp", EditorAction::PrevBuffer),
            // Pane
            ("Ctrl+\\", EditorAction::SplitHorizontal),
            ("Ctrl+Shift+\\", EditorAction::SplitVertical),
            ("Ctrl+Shift+W", EditorAction::ClosePane),
            ("Alt+Left", EditorAction::FocusLeft),
            ("Alt+Right", EditorAction::FocusRight),
            ("Alt+Up", EditorAction::FocusUp),
            ("Alt+Down", EditorAction::FocusDown),
            ("Alt+Shift+Left", EditorAction::ResizePaneLeft),
            ("Alt+Shift+Right", EditorAction::ResizePaneRight),
            ("Alt+Shift+Up", EditorAction::ResizePaneUp),
            ("Alt+Shift+Down", EditorAction::ResizePaneDown),
            // View
            ("F1", EditorAction::ToggleHelp),
            ("Alt+Z", EditorAction::ToggleWrap),
            ("Ctrl+B", EditorAction::ToggleFileTree),
            ("Ctrl+P", EditorAction::CommandPalette),
            // Terminal — NEW default: Ctrl+T instead of Ctrl+`
            ("Ctrl+T", EditorAction::ToggleTerminal),
            ("Ctrl+Shift+T", EditorAction::NewTerminal),
            // LSP interactive
            ("Ctrl+Space", EditorAction::LspComplete),
            ("Alt+K", EditorAction::LspHover),
            ("F12", EditorAction::LspGoToDef),
            // Diff view
            ("F7", EditorAction::DiffOpenVsHead),
            ("F8", EditorAction::DiffNextHunk),
            ("Shift+F8", EditorAction::DiffPrevHunk),
            // Minimap — Alt+M (Ctrl+Shift+M = 0x0D = Enter in all standard terminals)
            ("Alt+M", EditorAction::ToggleMinimap),
            // Task runner
            ("F5", EditorAction::TaskRun),
            ("Ctrl+F5", EditorAction::TaskBuild),
            ("Shift+F5", EditorAction::TaskTest),
            ("Alt+F5", EditorAction::TaskStop),
        ];

        let mut bindings = HashMap::new();
        let mut labels = HashMap::new();

        for &(key_str, action) in table {
            if let Some(kb) = parse_key_string(key_str) {
                labels
                    .entry(action)
                    .or_insert_with(|| kb.to_display_string());
                bindings.insert(kb, action);
            }
        }

        KeyMap { bindings, labels }
    }

    /// Build a key map from defaults + optional user overrides from config JSON.
    pub fn new(overrides: Option<&JsonValue>) -> Self {
        let mut km = Self::defaults();
        if let Some(obj) = overrides {
            km.apply_overrides(obj);
        }
        km
    }

    /// Apply user overrides: `{ "action_name": "Key+String", ... }`.
    fn apply_overrides(&mut self, obj: &JsonValue) {
        let pairs = match obj.as_object() {
            Some(p) => p,
            None => return,
        };
        for (action_name, key_val) in pairs {
            let key_str = match key_val.as_str() {
                Some(s) => s,
                None => continue,
            };
            let action = match action_name_to_action(action_name) {
                Some(a) => a,
                None => continue, // silently skip unknown actions
            };
            let kb = match parse_key_string(key_str) {
                Some(k) => k,
                None => continue, // silently skip unparseable key strings
            };
            // Remove old binding for this action (if any)
            self.bindings.retain(|_, a| *a != action);
            // Set new label and binding
            self.labels.insert(action, kb.to_display_string());
            self.bindings.insert(kb, action);
        }
    }

    /// Look up a key event → action.
    pub fn lookup(&self, ke: &KeyEvent) -> Option<EditorAction> {
        let kb = KeyBinding::from_key_event(ke);
        self.bindings.get(&kb).copied()
    }

    /// Get the display label for an action (for palette display).
    pub fn label(&self, action: EditorAction) -> &str {
        self.labels.get(&action).map(|s| s.as_str()).unwrap_or("")
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::defaults()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ctrl_s() {
        let kb = parse_key_string("Ctrl+S").unwrap();
        assert_eq!(kb.key, NormalizedKey::Char('s'));
        assert!(kb.ctrl);
        assert!(!kb.alt);
        assert!(!kb.shift);
    }

    #[test]
    fn test_parse_ctrl_shift_s() {
        let kb = parse_key_string("Ctrl+Shift+S").unwrap();
        assert_eq!(kb.key, NormalizedKey::Char('S'));
        assert!(kb.ctrl);
        assert!(!kb.alt);
        assert!(!kb.shift); // shift absorbed into case for Ctrl+letter
    }

    #[test]
    fn test_parse_alt_z() {
        let kb = parse_key_string("Alt+Z").unwrap();
        assert_eq!(kb.key, NormalizedKey::Char('z'));
        assert!(!kb.ctrl);
        assert!(kb.alt);
        assert!(!kb.shift);
    }

    #[test]
    fn test_parse_f3() {
        let kb = parse_key_string("F3").unwrap();
        assert_eq!(kb.key, NormalizedKey::F(3));
        assert!(!kb.ctrl);
        assert!(!kb.alt);
        assert!(!kb.shift);
    }

    #[test]
    fn test_parse_shift_f3() {
        let kb = parse_key_string("Shift+F3").unwrap();
        assert_eq!(kb.key, NormalizedKey::F(3));
        assert!(!kb.ctrl);
        assert!(!kb.alt);
        assert!(kb.shift);
    }

    #[test]
    fn test_parse_ctrl_pgdn() {
        let kb = parse_key_string("Ctrl+PgDn").unwrap();
        assert_eq!(kb.key, NormalizedKey::PageDown);
        assert!(kb.ctrl);
        assert!(!kb.alt);
        assert!(!kb.shift);
    }

    #[test]
    fn test_parse_ctrl_backtick() {
        let kb = parse_key_string("Ctrl+`").unwrap();
        assert_eq!(kb.key, NormalizedKey::Char('`'));
        assert!(kb.ctrl);
    }

    #[test]
    fn test_parse_shift_tab() {
        let kb = parse_key_string("Shift+Tab").unwrap();
        assert_eq!(kb.key, NormalizedKey::BackTab);
        assert!(kb.shift);
    }

    #[test]
    fn test_parse_ctrl_backslash() {
        let kb = parse_key_string("Ctrl+\\").unwrap();
        assert_eq!(kb.key, NormalizedKey::Char('\\'));
        assert!(kb.ctrl);
    }

    #[test]
    fn test_parse_alt_shift_left() {
        let kb = parse_key_string("Alt+Shift+Left").unwrap();
        assert_eq!(kb.key, NormalizedKey::Left);
        assert!(!kb.ctrl);
        assert!(kb.alt);
        assert!(kb.shift);
    }

    #[test]
    fn test_display_roundtrip_ctrl_s() {
        let kb = parse_key_string("Ctrl+S").unwrap();
        let display = kb.to_display_string();
        assert_eq!(display, "Ctrl+S");
        let parsed_back = parse_key_string(&display).unwrap();
        assert_eq!(parsed_back, kb);
    }

    #[test]
    fn test_display_roundtrip_ctrl_shift_s() {
        let kb = parse_key_string("Ctrl+Shift+S").unwrap();
        let display = kb.to_display_string();
        assert_eq!(display, "Ctrl+Shift+S");
    }

    #[test]
    fn test_display_roundtrip_f3() {
        let kb = parse_key_string("F3").unwrap();
        let display = kb.to_display_string();
        assert_eq!(display, "F3");
        let parsed_back = parse_key_string(&display).unwrap();
        assert_eq!(parsed_back, kb);
    }

    #[test]
    fn test_display_shift_f3() {
        let kb = parse_key_string("Shift+F3").unwrap();
        let display = kb.to_display_string();
        assert_eq!(display, "Shift+F3");
        let parsed_back = parse_key_string(&display).unwrap();
        assert_eq!(parsed_back, kb);
    }

    #[test]
    fn test_default_keymap_lookup() {
        let km = KeyMap::defaults();
        // Ctrl+S → Save
        let ke = KeyEvent {
            key: Key::Char('s'),
            ctrl: true,
            alt: false,
            shift: false,
        };
        assert_eq!(km.lookup(&ke), Some(EditorAction::Save));
    }

    #[test]
    fn test_default_keymap_toggle_terminal() {
        let km = KeyMap::defaults();
        // New default: Ctrl+T → ToggleTerminal
        let ke = KeyEvent {
            key: Key::Char('t'),
            ctrl: true,
            alt: false,
            shift: false,
        };
        assert_eq!(km.lookup(&ke), Some(EditorAction::ToggleTerminal));
    }

    #[test]
    fn test_default_keymap_label() {
        let km = KeyMap::defaults();
        let label = km.label(EditorAction::Save);
        assert_eq!(label, "Ctrl+S");
    }

    #[test]
    fn test_user_override() {
        let json = r#"{"toggle_terminal": "F12"}"#;
        let val = JsonValue::parse(json).unwrap();
        let km = KeyMap::new(Some(&val));
        // F12 should now map to ToggleTerminal
        let ke = KeyEvent {
            key: Key::F(12),
            ctrl: false,
            alt: false,
            shift: false,
        };
        assert_eq!(km.lookup(&ke), Some(EditorAction::ToggleTerminal));
        // Old Ctrl+T should no longer map to ToggleTerminal
        let ke_old = KeyEvent {
            key: Key::Char('t'),
            ctrl: true,
            alt: false,
            shift: false,
        };
        assert_eq!(km.lookup(&ke_old), None);
    }

    #[test]
    fn test_unknown_action_skipped() {
        let json = r#"{"nonexistent_action": "Ctrl+Z"}"#;
        let val = JsonValue::parse(json).unwrap();
        let km = KeyMap::new(Some(&val));
        // Ctrl+Z should still be Undo (not overridden by unknown action)
        let ke = KeyEvent {
            key: Key::Char('z'),
            ctrl: true,
            alt: false,
            shift: false,
        };
        assert_eq!(km.lookup(&ke), Some(EditorAction::Undo));
    }

    #[test]
    fn test_from_key_event_ctrl_shift() {
        // Terminal sends Ctrl+Shift+S as Char('S') with ctrl=true
        let ke = KeyEvent {
            key: Key::Char('S'),
            ctrl: true,
            alt: false,
            shift: false,
        };
        let kb = KeyBinding::from_key_event(&ke);
        // Should match our parsed Ctrl+Shift+S
        let expected = parse_key_string("Ctrl+Shift+S").unwrap();
        assert_eq!(kb, expected);
    }

    #[test]
    fn test_parse_case_insensitive_modifiers() {
        let kb = parse_key_string("ctrl+shift+s").unwrap();
        assert_eq!(kb.key, NormalizedKey::Char('S'));
        assert!(kb.ctrl);
    }

    #[test]
    fn test_ctrl_slash_lookup() {
        let km = KeyMap::defaults();
        let ke = KeyEvent {
            key: Key::Char('/'),
            ctrl: true,
            alt: false,
            shift: false,
        };
        assert_eq!(km.lookup(&ke), Some(EditorAction::ToggleComment));
    }
}
