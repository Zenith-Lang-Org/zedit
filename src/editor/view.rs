use crate::layout::{PaneId, Rect};
use crate::render::Color;
use crate::syntax::highlight;
use crate::terminal::{self, ColorMode};

use super::*;

impl Editor {
    /// Height of the active pane's text area.
    pub(super) fn text_area_height(&self) -> usize {
        if let Some(rect) = self.layout.pane_rect(self.active_pane) {
            rect.height as usize
        } else {
            self.screen.height().saturating_sub(self.status_height)
        }
    }

    /// Width of the active pane's text area (minus gutter).
    pub(super) fn text_area_width(&self) -> usize {
        let pane_width = if let Some(rect) = self.layout.pane_rect(self.active_pane) {
            rect.width as usize
        } else {
            self.screen.width()
        };
        pane_width.saturating_sub(self.buf().gutter_width)
    }

    pub(super) fn adjust_viewport(&mut self) {
        let h = self.text_area_height();
        let w = self.text_area_width();

        let b = self.buf();
        let cursor_line = b.cursor().line;
        let scroll_row = b.scroll_row;

        // Vertical scrolling
        if h > 0 {
            if cursor_line < scroll_row {
                self.buf_mut().scroll_row = cursor_line;
            } else if cursor_line >= scroll_row + h {
                self.buf_mut().scroll_row = cursor_line - h + 1;
            }
        }

        // Horizontal scrolling
        let display_col = self.cursor_display_col();
        let scroll_col = self.buf().scroll_col;
        if w > 0 {
            if display_col < scroll_col {
                self.buf_mut().scroll_col = display_col;
            } else if display_col >= scroll_col + w {
                self.buf_mut().scroll_col = display_col - w + 1;
            }
        }
    }

    pub(super) fn cursor_display_col(&self) -> usize {
        let b = self.buf();
        let line_text = b.buffer.get_line(b.cursor().line).unwrap_or_default();
        byte_col_to_display_col(&line_text, b.cursor().col)
    }

    pub(super) fn render(&mut self) {
        // Update gutter width for active buffer
        let buf_idx = self.active_buffer_index();
        let has_git = self.buffers[buf_idx].git_info.is_some();
        self.buffers[buf_idx].gutter_width = if self.config.line_numbers {
            let base = compute_gutter_width(self.buffers[buf_idx].buffer.line_count());
            if has_git { base + 2 } else { base }
        } else if has_git {
            2
        } else {
            0
        };
        self.adjust_viewport();

        let screen_width = self.screen.width();
        let screen_height = self.screen.height();

        // Render each pane
        let panes: Vec<_> = self.layout.panes().to_vec();
        for pane_info in &panes {
            // Update gutter width for this pane's buffer
            let bi = pane_info.buffer_index;
            if bi < self.buffers.len() {
                let has_git = self.buffers[bi].git_info.is_some();
                self.buffers[bi].gutter_width = if self.config.line_numbers {
                    let base = compute_gutter_width(self.buffers[bi].buffer.line_count());
                    if has_git { base + 2 } else { base }
                } else if has_git {
                    2
                } else {
                    0
                };
            }
            self.render_editor_pane(pane_info.rect, pane_info.buffer_index, pane_info.id);
        }

        // Draw pane borders
        self.render_pane_borders(&panes);

        // -- Status bar (inverted colors) --
        let h = screen_height.saturating_sub(self.status_height);
        let status_row = h;
        if status_row < screen_height {
            let status_fg = Color::Ansi(0); // black
            let status_bg = Color::Ansi(7); // white

            // Build status text from active pane's buffer
            let b = self.buf();
            let filename = b
                .buffer
                .file_path()
                .map(shorten_path)
                .unwrap_or_else(|| "[No Name]".to_string());
            let modified_marker = if b.buffer.is_modified() { " [+]" } else { "" };
            let color_str = match self.color_mode {
                ColorMode::TrueColor => "TrueColor",
                ColorMode::Color256 => "256color",
                ColorMode::Color16 => "16color",
            };
            let position = format!(
                "Ln {}, Col {}",
                b.cursor().line + 1,
                self.cursor_display_col() + 1,
            );

            // Buffer indicator
            let buf_count = self.buffers.len();
            let active_buf_idx = self.active_buffer_index();
            let buf_indicator = if buf_count > 1 {
                format!("[{}/{}] ", active_buf_idx + 1, buf_count)
            } else {
                String::new()
            };

            // Pane indicator
            let pane_count = self.layout.pane_count();
            let pane_indicator = if pane_count > 1 {
                format!("P{} ", self.active_pane.0 + 1)
            } else {
                String::new()
            };

            // Regex mode indicator
            let regex_indicator = if self
                .buf()
                .search
                .as_ref()
                .is_some_and(|s| s.mode == SearchMode::Regex)
            {
                " [.*]"
            } else {
                ""
            };

            // Multi-cursor indicator
            let multi_cursor_indicator = if self.buf().is_multi() {
                format!(" [{} cursors]", self.buf().cursors.len())
            } else {
                String::new()
            };

            let left = format!(
                " {}{}{}{}{}{}",
                pane_indicator,
                buf_indicator,
                filename,
                modified_marker,
                regex_indicator,
                multi_cursor_indicator
            );
            let right = format!("{} | {} ", position, color_str);

            // Fill status bar
            for col in 0..screen_width {
                self.screen
                    .put_char(status_row, col, ' ', status_fg, status_bg, true);
            }
            // Left side
            self.screen
                .put_str(status_row, 0, &left, status_fg, status_bg, true);
            // Right side
            let right_start = screen_width.saturating_sub(right.len());
            self.screen
                .put_str(status_row, right_start, &right, status_fg, status_bg, true);
        }

        // -- Message line --
        let msg_row = h + 1;
        if msg_row < screen_height {
            // Fill with spaces first
            for col in 0..screen_width {
                self.screen
                    .put_char(msg_row, col, ' ', Color::Default, Color::Default, false);
            }

            if let Some(ref prompt) = self.prompt {
                // Render prompt: label (yellow) + input (default)
                let label_fg = Color::Ansi(3); // yellow
                self.screen
                    .put_str(msg_row, 1, &prompt.label, label_fg, Color::Default, false);
                let input_start = 1 + crate::unicode::str_width(&prompt.label);
                self.screen.put_str(
                    msg_row,
                    input_start,
                    &prompt.input,
                    Color::Default,
                    Color::Default,
                    false,
                );

                // Show error message after the input if present
                if let Some(ref msg) = self.message {
                    let msg_fg = match self.message_type {
                        MessageType::Error => Color::Ansi(1),
                        MessageType::Warning => Color::Ansi(3),
                        _ => Color::Ansi(2),
                    };
                    let err_start = input_start + crate::unicode::str_width(&prompt.input) + 2;
                    if err_start < screen_width {
                        self.screen
                            .put_str(msg_row, err_start, msg, msg_fg, Color::Default, false);
                    }
                }
            } else if let Some(ref msg) = self.message {
                let msg_fg = match self.message_type {
                    MessageType::Info => Color::Ansi(2),    // green
                    MessageType::Error => Color::Ansi(1),   // red
                    MessageType::Warning => Color::Ansi(3), // yellow
                };
                self.screen
                    .put_str(msg_row, 1, msg, msg_fg, Color::Default, false);
            }
        }

        // Help overlay (drawn on top of everything)
        if self.help_visible {
            self.render_help();
        }

        // Command palette overlay
        if self.palette.is_some() {
            self.render_palette();
        }

        // Flush the screen
        self.screen.flush(&self.color_mode);

        // Position the hardware cursor
        if let Some(ref palette) = self.palette {
            // Cursor in the palette input field
            let screen_w = self.screen.width();
            let palette_width = (screen_w * 60 / 100).clamp(40, 80).min(screen_w);
            let start_col = (screen_w - palette_width) / 2;
            let input_col =
                start_col + 3 + crate::unicode::str_width(&palette.input[..palette.cursor_pos]);
            terminal::move_cursor(2, (input_col + 1) as u16); // row 1 (0-indexed) = input row
            terminal::flush();
            return;
        } else if let Some(ref prompt) = self.prompt {
            // Cursor on message line within prompt input
            let prompt_cursor_col = 1
                + crate::unicode::str_width(&prompt.label)
                + crate::unicode::str_width(&prompt.input[..prompt.cursor_pos]);
            let msg_row_1based = (h + 1 + 1) as u16; // h+1 is msg_row, +1 for 1-based
            terminal::move_cursor(msg_row_1based, (prompt_cursor_col + 1) as u16);
        } else if let Some(rect) = self.layout.pane_rect(self.active_pane) {
            let b = self.buf();
            let cursor_screen_row = b.cursor().line.saturating_sub(b.scroll_row) + rect.y as usize;
            let cursor_display = self.cursor_display_col();
            let cursor_screen_col = cursor_display
                .saturating_sub(b.scroll_col)
                .saturating_add(b.gutter_width)
                + rect.x as usize;

            terminal::move_cursor(
                (cursor_screen_row + 1) as u16,
                (cursor_screen_col + 1) as u16,
            );
        }
        terminal::flush();
    }

    /// Render a single editor pane within the given rectangle.
    fn render_editor_pane(&mut self, rect: Rect, buffer_idx: usize, pane_id: PaneId) {
        if buffer_idx >= self.buffers.len() {
            return;
        }

        // Refresh git info if stale
        {
            let bs = &mut self.buffers[buffer_idx];
            let line_count = bs.buffer.line_count();
            if let Some(gi) = &mut bs.git_info {
                let buf_ref = &bs.buffer;
                gi.refresh_if_stale(line_count, |i| buf_ref.get_line(i).unwrap_or_default());
            }
        }

        let pane_x = rect.x as usize;
        let pane_y = rect.y as usize;
        let pane_w = rect.width as usize;
        let pane_h = rect.height as usize;

        let bs = &self.buffers[buffer_idx];
        let scroll_row = bs.scroll_row;
        let scroll_col = bs.scroll_col;
        let gutter_width = bs.gutter_width;
        let line_count = bs.buffer.line_count();
        let has_git = bs.git_info.is_some();

        // Collect all selection ranges for this pane
        let sel_ranges: Vec<(usize, usize)> = if pane_id == self.active_pane {
            bs.cursors
                .iter()
                .filter_map(|cs| {
                    cs.selection.map(|sel| {
                        let s = sel.anchor.min(sel.head);
                        let e = sel.anchor.max(sel.head);
                        (s, e)
                    })
                })
                .collect()
        } else {
            Vec::new()
        };

        // Collect secondary cursor byte offsets for rendering
        let secondary_cursor_offsets: Vec<usize> = if pane_id == self.active_pane && bs.is_multi() {
            bs.cursors
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != bs.primary)
                .map(|(_, cs)| cs.cursor.byte_offset(&bs.buffer))
                .collect()
        } else {
            Vec::new()
        };

        for local_row in 0..pane_h {
            let screen_row = pane_y + local_row;
            let file_line = scroll_row + local_row;

            if file_line < line_count {
                // Git gutter column
                let git_col_width = if has_git { 2 } else { 0 };
                if has_git {
                    let status = self.buffers[buffer_idx]
                        .git_info
                        .as_ref()
                        .map(|gi| gi.line_status(file_line))
                        .unwrap_or(crate::git::LineStatus::Unchanged);
                    let (ch, fg) = match status {
                        crate::git::LineStatus::Added => ('+', Color::Ansi(2)),
                        crate::git::LineStatus::Modified => ('~', Color::Ansi(3)),
                        crate::git::LineStatus::DeletedBelow => ('\u{25B8}', Color::Ansi(1)),
                        crate::git::LineStatus::Unchanged => (' ', Color::Default),
                    };
                    self.screen
                        .put_char(screen_row, pane_x, ch, fg, Color::Default, false);
                    self.screen.put_char(
                        screen_row,
                        pane_x + 1,
                        ' ',
                        Color::Default,
                        Color::Default,
                        false,
                    );
                }

                // Gutter: right-aligned line number
                let line_num_width = gutter_width.saturating_sub(git_col_width);
                if line_num_width > 0 && self.config.line_numbers {
                    let num_str = format!("{}", file_line + 1);
                    let pad = line_num_width.saturating_sub(num_str.len() + 1);
                    let gutter_fg = Color::Color256(240);
                    let gutter_bg = Color::Default;

                    for col in 0..pad {
                        self.screen.put_char(
                            screen_row,
                            pane_x + git_col_width + col,
                            ' ',
                            gutter_fg,
                            gutter_bg,
                            false,
                        );
                    }
                    self.screen.put_str(
                        screen_row,
                        pane_x + git_col_width + pad,
                        &num_str,
                        gutter_fg,
                        gutter_bg,
                        false,
                    );
                    let sep_col = pad + num_str.len();
                    if sep_col < line_num_width {
                        self.screen.put_char(
                            screen_row,
                            pane_x + git_col_width + sep_col,
                            ' ',
                            gutter_fg,
                            gutter_bg,
                            false,
                        );
                    }
                }

                // Line content
                let line_text = self.buffers[buffer_idx]
                    .buffer
                    .get_line(file_line)
                    .unwrap_or_default();
                let line_start_byte = self.buffers[buffer_idx]
                    .buffer
                    .line_start(file_line)
                    .unwrap_or(0);

                // Syntax highlighting spans
                let spans = {
                    let bs = &mut self.buffers[buffer_idx];
                    bs.highlighter.as_mut().map(|hl| {
                        let buf_ref = &bs.buffer;
                        hl.style_line(file_line, &line_text, |l| buf_ref.get_line(l))
                    })
                };

                let mut display_col: usize = 0;
                let mut byte_offset_in_line: usize = 0;
                for ch in line_text.chars() {
                    let cw = crate::unicode::char_width(ch);
                    if display_col >= scroll_col {
                        let screen_col = pane_x + display_col - scroll_col + gutter_width;
                        if screen_col >= pane_x + pane_w {
                            break;
                        }
                        let char_byte = line_start_byte + byte_offset_in_line;
                        let is_selected = sel_ranges
                            .iter()
                            .any(|(s, e)| char_byte >= *s && char_byte < *e);
                        let is_secondary_cursor = secondary_cursor_offsets.contains(&char_byte);
                        let (fg, bg, bold) = if is_selected {
                            (Color::Ansi(0), Color::Ansi(7), true)
                        } else if is_secondary_cursor {
                            // Secondary cursor: inverted dimmed block
                            (Color::Ansi(0), Color::Color256(246), true)
                        } else if pane_id == self.active_pane {
                            if let Some(is_current) = self.match_at_byte(char_byte) {
                                if is_current {
                                    (Color::Ansi(0), Color::Ansi(6), true)
                                } else {
                                    (Color::Ansi(0), Color::Ansi(3), false)
                                }
                            } else {
                                match &spans {
                                    Some(s) => highlight::lookup_style(s, byte_offset_in_line),
                                    None => (Color::Default, Color::Default, false),
                                }
                            }
                        } else {
                            match &spans {
                                Some(s) => highlight::lookup_style(s, byte_offset_in_line),
                                None => (Color::Default, Color::Default, false),
                            }
                        };
                        self.screen
                            .put_char(screen_row, screen_col, ch, fg, bg, bold);
                    }
                    byte_offset_in_line += ch.len_utf8();
                    display_col += cw;
                }

                // Fill remaining with spaces
                let start_fill = pane_x
                    + display_col
                        .saturating_sub(scroll_col)
                        .saturating_add(gutter_width);
                let line_end_byte = line_start_byte + line_text.len();
                for col in start_fill..(pane_x + pane_w) {
                    let is_trailing_selected = sel_ranges
                        .iter()
                        .any(|(s, e)| line_end_byte >= *s && line_end_byte < *e)
                        && col == start_fill;
                    let is_secondary_cursor_trail =
                        secondary_cursor_offsets.contains(&line_end_byte) && col == start_fill;
                    let (fg, bg, bold) = if is_trailing_selected {
                        (Color::Ansi(0), Color::Ansi(7), true)
                    } else if is_secondary_cursor_trail {
                        (Color::Ansi(0), Color::Color256(246), true)
                    } else {
                        (Color::Default, Color::Default, false)
                    };
                    self.screen.put_char(screen_row, col, ' ', fg, bg, bold);
                }
            } else {
                // Tilde line (past end of file)
                self.screen.put_char(
                    screen_row,
                    pane_x,
                    '~',
                    Color::Color256(240),
                    Color::Default,
                    false,
                );
                for col in (pane_x + 1)..(pane_x + pane_w) {
                    self.screen.put_char(
                        screen_row,
                        col,
                        ' ',
                        Color::Default,
                        Color::Default,
                        false,
                    );
                }
            }
        }
    }

    /// Draw borders between panes.
    fn render_pane_borders(&mut self, panes: &[crate::layout::PaneInfo]) {
        if panes.len() <= 1 {
            return;
        }

        let border_fg = Color::Color256(240); // dim border
        let active_border_fg = Color::Ansi(6); // cyan for active pane
        let border_bg = Color::Default;

        // For each pane, draw a vertical border on its right edge if there's a pane to its right,
        // and a horizontal border on its bottom edge if there's a pane below.
        for pane in panes {
            let r = pane.rect;
            let fg = if pane.id == self.active_pane {
                active_border_fg
            } else {
                border_fg
            };

            // Check if there's a pane to the right (sharing a border column)
            let right_border_col = (r.x + r.width) as usize;
            let has_right_neighbor = panes
                .iter()
                .any(|other| other.id != pane.id && other.rect.x as usize == right_border_col + 1);

            if has_right_neighbor {
                // Draw vertical border │ on the column right after this pane
                for row in r.y..(r.y + r.height) {
                    self.screen.put_char(
                        row as usize,
                        right_border_col,
                        '\u{2502}',
                        fg,
                        border_bg,
                        false,
                    );
                }
            }
        }
    }

    pub(super) fn render_help(&mut self) {
        const HELP_LINES: &[&str] = &[
            "                  Zedit Help                  ",
            "                                              ",
            "  FILE              NAVIGATION                ",
            "  Ctrl+S  Save      \u{2191}\u{2193}\u{2190}\u{2192}    Move cursor        ",
            "  Ctrl+\u{21e7}S Save As   Home/End Line start/end  ",
            "  Ctrl+O  Open      Ctrl+Home File start      ",
            "  Ctrl+Q  Quit      Ctrl+End  File end        ",
            "                    PgUp/PgDn Page scroll     ",
            "  BUFFERS           Ctrl+G    Go to line      ",
            "  Ctrl+N  New       Ctrl+F    Find            ",
            "  Ctrl+W  Close     Ctrl+H    Replace         ",
            "  Ctrl+PgDn Next    F3/\u{21e7}F3   Next/prev match  ",
            "  Ctrl+PgUp Prev                              ",
            "                    PANES                     ",
            "  EDIT              Ctrl+\\    Split horiz     ",
            "  Ctrl+Z  Undo      Ctrl+\u{21e7}\\   Split vert      ",
            "  Ctrl+Y  Redo      Ctrl+\u{21e7}W  Close pane       ",
            "  Ctrl+C  Copy      Alt+\u{2190}\u{2192}\u{2191}\u{2193}  Focus pane      ",
            "  Ctrl+X  Cut       Alt+\u{21e7}\u{2190}\u{2192}  Resize pane     ",
            "  Ctrl+V  Paste                               ",
            "  Ctrl+D  Sel next MULTI-CURSOR                ",
            "  Ctrl+\u{21e7}D Dup line  Ctrl+\u{21e7}L  All occurrences ",
            "  Ctrl+\u{21e7}K Del line  Alt+Click Add cursor      ",
            "  Tab     Indent    Escape    Single cursor   ",
            "  \u{21e7}Tab    Unindent SELECTION                   ",
            "  Ctrl+/  Comment   Shift+\u{2190}\u{2192}\u{2191}\u{2193} Extend sel     ",
            "  Ctrl+L  Sel line  Ctrl+A    Select all      ",
            "  Ctrl+\u{21e7}P Palette                              ",
            "        Press Esc or F1 to close              ",
        ];

        let panel_width = 48;
        let panel_height = HELP_LINES.len();
        let box_width = panel_width + 2;
        let box_height = panel_height + 2;

        let screen_w = self.screen.width();
        let screen_h = self.screen.height();

        if box_width > screen_w || box_height > screen_h {
            return;
        }

        let start_col = (screen_w - box_width) / 2;
        let start_row = (screen_h - box_height) / 2;

        let border_fg = Color::Ansi(6);
        let bg = Color::Color256(235);
        let text_fg = Color::Ansi(7);
        let header_fg = Color::Ansi(6);

        // Top border
        self.screen
            .put_char(start_row, start_col, '\u{250c}', border_fg, bg, false);
        for col in 1..box_width - 1 {
            self.screen
                .put_char(start_row, start_col + col, '\u{2500}', border_fg, bg, false);
        }
        self.screen.put_char(
            start_row,
            start_col + box_width - 1,
            '\u{2510}',
            border_fg,
            bg,
            false,
        );

        // Content rows
        for (i, line) in HELP_LINES.iter().enumerate() {
            let row = start_row + 1 + i;
            self.screen
                .put_char(row, start_col, '\u{2502}', border_fg, bg, false);

            let mut col_offset = 0;
            let fg = if i == 0 { header_fg } else { text_fg };
            let bold = i == 0;

            for ch in line.chars() {
                let cw = crate::unicode::char_width(ch).max(1);
                if col_offset + cw <= panel_width {
                    self.screen
                        .put_char(row, start_col + 1 + col_offset, ch, fg, bg, bold);
                    col_offset += cw;
                } else {
                    break;
                }
            }
            while col_offset < panel_width {
                self.screen
                    .put_char(row, start_col + 1 + col_offset, ' ', fg, bg, false);
                col_offset += 1;
            }

            self.screen.put_char(
                row,
                start_col + box_width - 1,
                '\u{2502}',
                border_fg,
                bg,
                false,
            );
        }

        // Bottom border
        let bottom_row = start_row + box_height - 1;
        self.screen
            .put_char(bottom_row, start_col, '\u{2514}', border_fg, bg, false);
        for col in 1..box_width - 1 {
            self.screen.put_char(
                bottom_row,
                start_col + col,
                '\u{2500}',
                border_fg,
                bg,
                false,
            );
        }
        self.screen.put_char(
            bottom_row,
            start_col + box_width - 1,
            '\u{2518}',
            border_fg,
            bg,
            false,
        );
    }

    fn render_palette(&mut self) {
        let palette = match self.palette {
            Some(ref p) => p,
            None => return,
        };

        let screen_w = self.screen.width();
        let screen_h = self.screen.height();

        // Width: 60% of screen, clamped to [40, 80]
        let panel_width = (screen_w * 60 / 100).clamp(40, 80).min(screen_w);
        let box_width = panel_width;
        let max_visible: usize = 10;
        let visible_count = palette.filtered.len().min(max_visible);
        // box: 1 top border + 1 input row + 1 separator + visible_count result rows + 1 bottom border
        let box_height = 3 + visible_count + 1;

        if box_width > screen_w || box_height > screen_h {
            return;
        }

        let start_col = (screen_w - box_width) / 2;
        let start_row = 0; // top of screen

        let border_fg = Color::Ansi(6); // cyan
        let bg = Color::Color256(235); // dark bg
        let text_fg = Color::Ansi(7); // white
        let input_fg = Color::Default;
        let shortcut_fg = Color::Color256(240); // dim
        let highlight_fg = Color::Ansi(3); // yellow for matched chars
        let selected_bg = Color::Ansi(4); // blue for selected row

        // Top border: ┌───...───┐
        self.screen
            .put_char(start_row, start_col, '\u{250c}', border_fg, bg, false);
        for col in 1..box_width - 1 {
            self.screen
                .put_char(start_row, start_col + col, '\u{2500}', border_fg, bg, false);
        }
        self.screen.put_char(
            start_row,
            start_col + box_width - 1,
            '\u{2510}',
            border_fg,
            bg,
            false,
        );

        // Input row: │ > query... │
        let input_row = start_row + 1;
        self.screen
            .put_char(input_row, start_col, '\u{2502}', border_fg, bg, false);
        // Fill with bg
        for col in 1..box_width - 1 {
            self.screen
                .put_char(input_row, start_col + col, ' ', input_fg, bg, false);
        }
        self.screen.put_char(
            input_row,
            start_col + box_width - 1,
            '\u{2502}',
            border_fg,
            bg,
            false,
        );
        // "> " prefix
        self.screen
            .put_str(input_row, start_col + 1, "> ", border_fg, bg, true);
        // Query text
        let max_input_width = box_width.saturating_sub(4);
        let display_input: String = palette.input.chars().take(max_input_width).collect();
        self.screen.put_str(
            input_row,
            start_col + 3,
            &display_input,
            input_fg,
            bg,
            false,
        );

        // Separator: ├───...───┤
        let sep_row = start_row + 2;
        self.screen
            .put_char(sep_row, start_col, '\u{251c}', border_fg, bg, false);
        for col in 1..box_width - 1 {
            self.screen
                .put_char(sep_row, start_col + col, '\u{2500}', border_fg, bg, false);
        }
        self.screen.put_char(
            sep_row,
            start_col + box_width - 1,
            '\u{2524}',
            border_fg,
            bg,
            false,
        );

        // Result rows
        let content_width = box_width - 2; // inside borders
        for i in 0..visible_count {
            let row = start_row + 3 + i;
            let filter_idx = palette.scroll_offset + i;
            let is_selected = filter_idx == palette.selected;
            let row_bg = if is_selected { selected_bg } else { bg };

            // Left border
            self.screen
                .put_char(row, start_col, '\u{2502}', border_fg, bg, false);
            // Fill row
            for col in 1..box_width - 1 {
                self.screen
                    .put_char(row, start_col + col, ' ', text_fg, row_bg, false);
            }
            // Right border
            self.screen.put_char(
                row,
                start_col + box_width - 1,
                '\u{2502}',
                border_fg,
                bg,
                false,
            );

            if filter_idx < palette.filtered.len() {
                let entry_idx = palette.filtered[filter_idx];
                let entry = palette.entry(entry_idx);
                let matched_positions = palette.match_positions(entry_idx);

                // Draw label with highlighted match positions
                let label_chars: Vec<char> = entry.label.chars().collect();
                let max_label_width = content_width.saturating_sub(entry.shortcut.len() + 3);
                let mut col_offset = 0;
                for (ci, &ch) in label_chars.iter().enumerate() {
                    if col_offset >= max_label_width {
                        break;
                    }
                    let is_match = matched_positions.contains(&ci);
                    let fg = if is_match { highlight_fg } else { text_fg };
                    let bold = is_match;
                    self.screen
                        .put_char(row, start_col + 1 + col_offset, ch, fg, row_bg, bold);
                    col_offset += crate::unicode::char_width(ch);
                }

                // Right-aligned shortcut
                let shortcut_width = crate::unicode::str_width(entry.shortcut);
                let shortcut_start = start_col + box_width - 1 - shortcut_width - 1;
                self.screen.put_str(
                    row,
                    shortcut_start,
                    entry.shortcut,
                    shortcut_fg,
                    row_bg,
                    false,
                );
            }
        }

        // Bottom border: └───...───┘
        let bottom_row = start_row + 3 + visible_count;
        self.screen
            .put_char(bottom_row, start_col, '\u{2514}', border_fg, bg, false);
        for col in 1..box_width - 1 {
            self.screen.put_char(
                bottom_row,
                start_col + col,
                '\u{2500}',
                border_fg,
                bg,
                false,
            );
        }
        self.screen.put_char(
            bottom_row,
            start_col + box_width - 1,
            '\u{2518}',
            border_fg,
            bg,
            false,
        );
    }

    /// Convert screen coordinates to buffer (line, byte_col).
    /// Also returns the pane id that was hit. Returns None if out of any text area.
    pub(super) fn screen_to_buffer(&self, col: u16, row: u16) -> Option<(usize, usize)> {
        self.screen_to_buffer_with_pane(col, row)
            .map(|(line, byte_col, _pane_id)| (line, byte_col))
    }

    /// Convert screen coordinates to buffer (line, byte_col, pane_id).
    pub(super) fn screen_to_buffer_with_pane(
        &self,
        col: u16,
        row: u16,
    ) -> Option<(usize, usize, PaneId)> {
        // Find which pane contains this screen coordinate
        for pane_info in self.layout.panes() {
            let r = pane_info.rect;
            if !r.contains(col, row) {
                continue;
            }
            if pane_info.buffer_index >= self.buffers.len() {
                continue;
            }

            let local_row = (row - r.y) as usize;
            let local_col = (col - r.x) as usize;

            let bs = &self.buffers[pane_info.buffer_index];
            let file_line = bs.scroll_row + local_row;
            if file_line >= bs.buffer.line_count() {
                return None;
            }

            if local_col < bs.gutter_width {
                return Some((file_line, 0, pane_info.id));
            }
            let display_col = local_col - bs.gutter_width + bs.scroll_col;
            let line_text = bs.buffer.get_line(file_line).unwrap_or_default();
            let byte_col = display_col_to_byte_col(&line_text, display_col);
            return Some((file_line, byte_col, pane_info.id));
        }
        None
    }
}
