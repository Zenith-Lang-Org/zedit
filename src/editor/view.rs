use crate::render::Color;
use crate::syntax::highlight;
use crate::terminal::{self, ColorMode};

use super::*;

impl Editor {
    pub(super) fn text_area_height(&self) -> usize {
        self.screen.height().saturating_sub(self.status_height)
    }

    pub(super) fn text_area_width(&self) -> usize {
        self.screen.width().saturating_sub(self.buf().gutter_width)
    }

    pub(super) fn adjust_viewport(&mut self) {
        let h = self.text_area_height();
        let w = self.text_area_width();

        let b = self.buf();
        let cursor_line = b.cursor.line;
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
        let line_text = b.buffer.get_line(b.cursor.line).unwrap_or_default();
        byte_col_to_display_col(&line_text, b.cursor.col)
    }

    pub(super) fn render(&mut self) {
        let b = self.buf();
        self.buffers[self.active_buffer].gutter_width = if self.config.line_numbers {
            compute_gutter_width(b.buffer.line_count())
        } else {
            0
        };
        self.adjust_viewport();

        let h = self.text_area_height();
        let screen_width = self.screen.width();

        let b = self.buf();
        let scroll_row = b.scroll_row;
        let scroll_col = b.scroll_col;
        let gutter_width = b.gutter_width;
        let line_count = b.buffer.line_count();
        let sel_range = self.selection_range();

        // -- Text area + gutter --
        for screen_row in 0..h {
            let file_line = scroll_row + screen_row;

            if file_line < line_count {
                // Gutter: right-aligned line number (only if line numbers enabled)
                if gutter_width > 0 {
                    let num_str = format!("{}", file_line + 1);
                    let pad = gutter_width.saturating_sub(num_str.len() + 1);
                    let gutter_fg = Color::Color256(240); // dim gray
                    let gutter_bg = Color::Default;

                    // Pad
                    for col in 0..pad {
                        self.screen
                            .put_char(screen_row, col, ' ', gutter_fg, gutter_bg, false);
                    }
                    // Number
                    self.screen
                        .put_str(screen_row, pad, &num_str, gutter_fg, gutter_bg, false);
                    // Separator space
                    let sep_col = pad + num_str.len();
                    if sep_col < gutter_width {
                        self.screen
                            .put_char(screen_row, sep_col, ' ', gutter_fg, gutter_bg, false);
                    }
                }

                // Line content (with selection + syntax highlighting)
                let line_text = self.buf().buffer.get_line(file_line).unwrap_or_default();
                let line_start_byte = self.buf().buffer.line_start(file_line).unwrap_or(0);

                // Get syntax highlighting spans for this line
                let spans = {
                    let bs = &mut self.buffers[self.active_buffer];
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
                        let screen_col = display_col - scroll_col + gutter_width;
                        if screen_col >= screen_width {
                            break;
                        }
                        let char_byte = line_start_byte + byte_offset_in_line;
                        let is_selected =
                            sel_range.is_some_and(|(s, e)| char_byte >= s && char_byte < e);
                        let (fg, bg, bold) = if is_selected {
                            (Color::Ansi(0), Color::Ansi(7), true)
                        } else if let Some(is_current) = self.match_at_byte(char_byte) {
                            if is_current {
                                (Color::Ansi(0), Color::Ansi(6), true) // cyan bg
                            } else {
                                (Color::Ansi(0), Color::Ansi(3), false) // yellow bg
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
                // Fill remaining with spaces (selected if selection extends past EOL)
                let start_fill = display_col
                    .saturating_sub(scroll_col)
                    .saturating_add(gutter_width);
                let line_end_byte = line_start_byte + line_text.len();
                for col in start_fill..screen_width {
                    // Show selection highlight on trailing space if newline is selected
                    let is_trailing_selected = sel_range
                        .is_some_and(|(s, e)| line_end_byte >= s && line_end_byte < e)
                        && col == start_fill; // only first trailing cell
                    let (fg, bg, bold) = if is_trailing_selected {
                        (Color::Ansi(0), Color::Ansi(7), true)
                    } else {
                        (Color::Default, Color::Default, false)
                    };
                    self.screen.put_char(screen_row, col, ' ', fg, bg, bold);
                }
            } else {
                // Tilde line (past end of file)
                self.screen.put_char(
                    screen_row,
                    0,
                    '~',
                    Color::Color256(240),
                    Color::Default,
                    false,
                );
                for col in 1..screen_width {
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

        // -- Status bar (inverted colors) --
        let status_row = h;
        if status_row < self.screen.height() {
            let status_fg = Color::Ansi(0); // black
            let status_bg = Color::Ansi(7); // white

            // Build status text
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
                b.cursor.line + 1,
                self.cursor_display_col() + 1,
            );

            // Buffer indicator
            let buf_count = self.buffers.len();
            let buf_indicator = if buf_count > 1 {
                format!("[{}/{}] ", self.active_buffer + 1, buf_count)
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

            let left = format!(
                " {}{}{}{}",
                buf_indicator, filename, modified_marker, regex_indicator
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
        if msg_row < self.screen.height() {
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

        // Flush the screen
        self.screen.flush(&self.color_mode);

        // Position the hardware cursor
        if let Some(ref prompt) = self.prompt {
            // Cursor on message line within prompt input
            let prompt_cursor_col = 1
                + crate::unicode::str_width(&prompt.label)
                + crate::unicode::str_width(&prompt.input[..prompt.cursor_pos]);
            let msg_row_1based = (h + 1 + 1) as u16; // h+1 is msg_row, +1 for 1-based
            terminal::move_cursor(msg_row_1based, (prompt_cursor_col + 1) as u16);
        } else {
            let b = self.buf();
            let cursor_screen_row = b.cursor.line.saturating_sub(b.scroll_row);
            let cursor_display = self.cursor_display_col();
            let cursor_screen_col = cursor_display
                .saturating_sub(self.buf().scroll_col)
                .saturating_add(self.buf().gutter_width);

            terminal::move_cursor(
                (cursor_screen_row + 1) as u16,
                (cursor_screen_col + 1) as u16,
            );
        }
        terminal::flush();
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
            "                    SELECTION                  ",
            "  EDIT              Shift+\u{2190}\u{2192}\u{2191}\u{2193} Extend sel     ",
            "  Ctrl+Z  Undo      Ctrl+\u{21e7}\u{2190}\u{2192}  Select word    ",
            "  Ctrl+Y  Redo      Ctrl+A    Select all      ",
            "  Ctrl+C  Copy      Ctrl+L    Select line     ",
            "  Ctrl+X  Cut                                 ",
            "  Ctrl+V  Paste    MOUSE                      ",
            "  Ctrl+D  Dup line  Click     Position        ",
            "  Ctrl+\u{21e7}K Del line  Drag      Select          ",
            "  Tab     Indent    Scroll    Viewport        ",
            "  \u{21e7}Tab    Unindent                            ",
            "  Ctrl+/  Comment                             ",
            "                                              ",
            "        Press Esc or F1 to close              ",
        ];

        let panel_width = 48; // content width
        let panel_height = HELP_LINES.len();
        let box_width = panel_width + 2; // +2 for left/right border
        let box_height = panel_height + 2; // +2 for top/bottom border

        let screen_w = self.screen.width();
        let screen_h = self.screen.height();

        if box_width > screen_w || box_height > screen_h {
            return; // terminal too small
        }

        let start_col = (screen_w - box_width) / 2;
        let start_row = (screen_h - box_height) / 2;

        let border_fg = Color::Ansi(6); // cyan
        let bg = Color::Color256(235); // dark gray
        let text_fg = Color::Ansi(7); // white
        let header_fg = Color::Ansi(6); // cyan

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
            // Left border
            self.screen
                .put_char(row, start_col, '\u{2502}', border_fg, bg, false);

            // Line content
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
            // Fill remaining space
            while col_offset < panel_width {
                self.screen
                    .put_char(row, start_col + 1 + col_offset, ' ', fg, bg, false);
                col_offset += 1;
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

    /// Convert screen coordinates to buffer (line, byte_col). Returns None if out of text area.
    pub(super) fn screen_to_buffer(&self, col: u16, row: u16) -> Option<(usize, usize)> {
        let screen_row = row as usize;
        let screen_col = col as usize;

        let h = self.text_area_height();
        if screen_row >= h {
            return None;
        }

        let b = self.buf();
        let file_line = b.scroll_row + screen_row;
        if file_line >= b.buffer.line_count() {
            return None;
        }

        if screen_col < b.gutter_width {
            return Some((file_line, 0));
        }
        let display_col = screen_col - b.gutter_width + b.scroll_col;
        let line_text = b.buffer.get_line(file_line).unwrap_or_default();
        let byte_col = display_col_to_byte_col(&line_text, display_col);
        Some((file_line, byte_col))
    }
}
