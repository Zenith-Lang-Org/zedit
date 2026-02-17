use std::path::Path;

use crate::buffer::Buffer;
use crate::config::LanguageDef;
use crate::cursor::Cursor;
use crate::syntax::highlight::{self, Highlighter};
use crate::undo::UndoStack;

use super::SearchState;

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(super) struct Selection {
    pub(super) anchor: usize, // byte offset where selection started
    pub(super) head: usize,   // byte offset at cursor end
}

// ---------------------------------------------------------------------------
// Per-buffer state
// ---------------------------------------------------------------------------

pub(super) struct BufferState {
    pub(super) buffer: Buffer,
    pub(super) cursor: Cursor,
    pub(super) scroll_row: usize,
    pub(super) scroll_col: usize,
    pub(super) selection: Option<Selection>,
    pub(super) undo_stack: UndoStack,
    pub(super) search: Option<SearchState>,
    pub(super) highlighter: Option<Highlighter>,
    pub(super) gutter_width: usize,
}

impl BufferState {
    pub(super) fn new_empty(line_numbers: bool) -> Self {
        let buffer = Buffer::new();
        let gutter_width = if line_numbers {
            compute_gutter_width(buffer.line_count())
        } else {
            0
        };
        BufferState {
            buffer,
            cursor: Cursor::new(),
            scroll_row: 0,
            scroll_col: 0,
            selection: None,
            undo_stack: UndoStack::new(),
            search: None,
            highlighter: None,
            gutter_width,
        }
    }

    pub(super) fn from_file(
        path: &Path,
        line_numbers: bool,
        theme_name: &str,
        languages: &[LanguageDef],
    ) -> Result<Self, String> {
        let buffer = Buffer::from_file(path)?;
        let gutter_width = if line_numbers {
            compute_gutter_width(buffer.line_count())
        } else {
            0
        };
        let highlighter = highlight::detect_language(path, languages).and_then(|lang| {
            highlight::load_grammar(&lang, languages).map(|grammar| {
                let theme = highlight::load_theme(theme_name);
                Highlighter::new(grammar, theme).with_lang(&lang)
            })
        });
        Ok(BufferState {
            buffer,
            cursor: Cursor::new(),
            scroll_row: 0,
            scroll_col: 0,
            selection: None,
            undo_stack: UndoStack::new(),
            search: None,
            highlighter,
            gutter_width,
        })
    }
}

pub(super) fn compute_gutter_width(line_count: usize) -> usize {
    let digits = if line_count == 0 {
        1
    } else {
        let mut n = line_count;
        let mut d = 0;
        while n > 0 {
            d += 1;
            n /= 10;
        }
        d
    };
    // digits + 2 (one space before, one after), minimum 4
    (digits + 2).max(4)
}
