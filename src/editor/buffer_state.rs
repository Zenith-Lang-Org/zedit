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
// CursorSelection — one cursor + optional selection
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(super) struct CursorSelection {
    pub(super) cursor: Cursor,
    pub(super) selection: Option<Selection>,
}

// ---------------------------------------------------------------------------
// Per-buffer state
// ---------------------------------------------------------------------------

pub(super) struct BufferState {
    pub(super) buffer: Buffer,
    pub(super) cursors: Vec<CursorSelection>,
    pub(super) primary: usize,
    pub(super) scroll_row: usize,
    pub(super) scroll_col: usize,
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
            cursors: vec![CursorSelection {
                cursor: Cursor::new(),
                selection: None,
            }],
            primary: 0,
            scroll_row: 0,
            scroll_col: 0,
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
            cursors: vec![CursorSelection {
                cursor: Cursor::new(),
                selection: None,
            }],
            primary: 0,
            scroll_row: 0,
            scroll_col: 0,
            undo_stack: UndoStack::new(),
            search: None,
            highlighter,
            gutter_width,
        })
    }

    // -- Convenience accessors for primary cursor --

    pub(super) fn cursor(&self) -> &Cursor {
        &self.cursors[self.primary].cursor
    }

    pub(super) fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursors[self.primary].cursor
    }

    pub(super) fn selection(&self) -> Option<Selection> {
        self.cursors[self.primary].selection
    }

    pub(super) fn set_selection(&mut self, sel: Option<Selection>) {
        self.cursors[self.primary].selection = sel;
    }

    pub(super) fn is_multi(&self) -> bool {
        self.cursors.len() > 1
    }

    /// Collapse to primary cursor only, removing all secondary cursors.
    pub(super) fn collapse_to_primary(&mut self) {
        let primary_cs = self.cursors[self.primary].clone();
        self.cursors = vec![primary_cs];
        self.primary = 0;
    }

    /// Sort cursors by byte offset and merge overlapping ones.
    pub(super) fn sort_and_merge(&mut self) {
        if self.cursors.len() <= 1 {
            return;
        }

        // Remember the primary cursor's byte offset to re-find it after sorting
        let primary_offset = self.cursors[self.primary]
            .cursor
            .byte_offset(&self.buffer);

        // Sort by byte offset
        self.cursors.sort_by_key(|cs| cs.cursor.byte_offset(&self.buffer));

        // Merge overlapping cursors (same byte offset)
        let mut merged = Vec::with_capacity(self.cursors.len());
        for cs in self.cursors.drain(..) {
            if let Some(last) = merged.last() {
                let last_cs: &CursorSelection = last;
                let last_off = last_cs.cursor.byte_offset(&self.buffer);
                let this_off = cs.cursor.byte_offset(&self.buffer);
                if last_off == this_off {
                    // Skip duplicate — keep the earlier one (already in merged)
                    continue;
                }
                // Also merge if selections overlap
                if let (Some(last_sel), Some(this_sel)) = (last_cs.selection, cs.selection) {
                    let last_start = last_sel.anchor.min(last_sel.head);
                    let last_end = last_sel.anchor.max(last_sel.head);
                    let this_start = this_sel.anchor.min(this_sel.head);
                    let this_end = this_sel.anchor.max(this_sel.head);
                    if this_start <= last_end && last_start <= this_end {
                        // Overlapping selections — merge by extending the last one
                        let merged_start = last_start.min(this_start);
                        let merged_end = last_end.max(this_end);
                        let last_mut: &mut CursorSelection = merged.last_mut().unwrap();
                        last_mut.selection = Some(Selection {
                            anchor: merged_start,
                            head: merged_end,
                        });
                        // Move cursor to the end of merged selection
                        let line = self.buffer.byte_to_line(merged_end);
                        let line_start = self.buffer.line_start(line).unwrap_or(0);
                        last_mut
                            .cursor
                            .set_position(line, merged_end - line_start, &self.buffer);
                        continue;
                    }
                }
            }
            merged.push(cs);
        }
        self.cursors = merged;

        // Re-find primary by closest byte offset
        self.primary = self
            .cursors
            .iter()
            .enumerate()
            .min_by_key(|(_, cs)| {
                let off = cs.cursor.byte_offset(&self.buffer);
                (off as isize - primary_offset as isize).unsigned_abs()
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
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
