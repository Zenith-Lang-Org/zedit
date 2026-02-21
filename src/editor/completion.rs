// ---------------------------------------------------------------------------
// Completion menu — floating list shown on Ctrl+Space
// ---------------------------------------------------------------------------

use crate::lsp::CompletionItem;

pub(super) struct CompletionMenuEntry {
    pub label: String,
    pub kind_str: &'static str,
    pub insert_text: String,
}

pub(super) struct CompletionMenu {
    pub items: Vec<CompletionMenuEntry>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub anchor_screen_row: usize,
    pub anchor_screen_col: usize,
}

impl CompletionMenu {
    pub fn new(raw: Vec<CompletionItem>, anchor_row: usize, anchor_col: usize) -> Self {
        let items: Vec<CompletionMenuEntry> = raw
            .into_iter()
            .map(|item| {
                let insert_text = item.insert_text.unwrap_or_else(|| item.label.clone());
                CompletionMenuEntry {
                    kind_str: kind_str(item.kind),
                    label: item.label,
                    insert_text,
                }
            })
            .collect();
        CompletionMenu {
            items,
            selected: 0,
            scroll_offset: 0,
            anchor_screen_row: anchor_row,
            anchor_screen_col: anchor_col,
        }
    }

    pub fn selected_insert_text(&self) -> &str {
        if self.items.is_empty() {
            return "";
        }
        &self.items[self.selected].insert_text
    }

    pub fn select_next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.items.len() - 1);
        // Scroll down if selected is past the visible window
        const VISIBLE: usize = 10;
        if self.selected >= self.scroll_offset + VISIBLE {
            self.scroll_offset = self.selected + 1 - VISIBLE;
        }
    }

    pub fn select_prev(&mut self) {
        if self.items.is_empty() || self.selected == 0 {
            return;
        }
        self.selected -= 1;
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }
    }
}

/// Map LSP completion item kind number → short display label.
fn kind_str(kind: Option<u32>) -> &'static str {
    match kind {
        Some(1) => "txt",
        Some(2) => "met",
        Some(3) => "fn ",
        Some(4) => "ctr",
        Some(5) => "fld",
        Some(6) => "var",
        Some(7) => "cls",
        Some(8) => "ifc",
        Some(9) => "mod",
        Some(10) => "prp",
        Some(11) => "unt",
        Some(12) => "val",
        Some(13) => "enm",
        Some(14) => "key",
        Some(15) => "snp",
        Some(16) => "clr",
        Some(17) => "fil",
        Some(18) => "ref",
        Some(19) => "fld",
        Some(20) => "evt",
        Some(21) => "op ",
        Some(22) => "typ",
        Some(23) => "par",
        Some(24) => "kw ",
        Some(25) => "  ",
        _ => "   ",
    }
}
