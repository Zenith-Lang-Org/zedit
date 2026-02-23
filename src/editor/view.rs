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

        if self.buf().wrap_map.is_some() {
            self.adjust_viewport_wrapped(h);
        } else {
            self.adjust_viewport_unwrapped(h, w);
        }
    }

    fn adjust_viewport_unwrapped(&mut self, h: usize, w: usize) {
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

    fn adjust_viewport_wrapped(&mut self, h: usize) {
        if h == 0 {
            return;
        }

        // No horizontal scrolling in wrap mode
        self.buf_mut().scroll_col = 0;

        let b = self.buf();
        let cursor_line = b.cursor().line;
        let cursor_col = b.cursor().col;
        let scroll_row = b.scroll_row;
        let scroll_visual_offset = b.scroll_visual_offset;

        let line_text = b.buffer.get_line(cursor_line).unwrap_or_default();

        // Get cursor's visual row
        let cursor_visual_row = if let Some(ref wm) = b.wrap_map {
            let (vr, _) = wm.buffer_to_visual(cursor_line, cursor_col, &line_text);
            vr
        } else {
            return;
        };

        // Get scroll start visual row
        let scroll_visual_row = if let Some(ref wm) = b.wrap_map {
            if scroll_row < wm.total_visual_rows() {
                let base = if let Some(ref wm2) = b.wrap_map {
                    // visual_offsets for scroll_row
                    let (vr, _) = wm2.buffer_to_visual(scroll_row, 0, "");
                    vr
                } else {
                    0
                };
                base + scroll_visual_offset
            } else {
                0
            }
        } else {
            return;
        };

        if cursor_visual_row < scroll_visual_row {
            // Cursor above viewport: scroll up
            let (line, seg) = self
                .buf()
                .wrap_map
                .as_ref()
                .unwrap()
                .visual_to_buffer(cursor_visual_row);
            self.buf_mut().scroll_row = line;
            self.buf_mut().scroll_visual_offset = seg;
        } else if cursor_visual_row >= scroll_visual_row + h {
            // Cursor below viewport: scroll down
            let target_start = cursor_visual_row - h + 1;
            let (line, seg) = self
                .buf()
                .wrap_map
                .as_ref()
                .unwrap()
                .visual_to_buffer(target_start);
            self.buf_mut().scroll_row = line;
            self.buf_mut().scroll_visual_offset = seg;
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
        if !self.scroll_only_mode {
            self.adjust_viewport();
        }

        let screen_width = self.screen.width();
        let screen_height = self.screen.height();

        // Render tab bar at row 0
        self.render_tab_bar(screen_width);

        // Render file tree sidebar if visible
        if let Some(ref mut ft) = self.filetree {
            let sidebar_height =
                screen_height.saturating_sub(self.status_height + self.tab_bar_height);
            ft.render(
                &mut self.screen,
                sidebar_height,
                self.filetree_focused,
                self.tab_bar_height,
            );
        }

        // Render each pane
        let panes: Vec<_> = self.layout.panes().to_vec();
        for pane_info in &panes {
            match pane_info.content {
                crate::layout::PaneContent::Buffer(bi) => {
                    // Update gutter width for this pane's buffer
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
                    self.render_editor_pane(pane_info.rect, bi, pane_info.id);
                }
                crate::layout::PaneContent::Terminal(ti) => {
                    self.render_terminal_pane(pane_info.rect, ti, pane_info.id);
                }
            }
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
            let buf_idx = self.active_buffer_index();
            let filename = self.buffer_display_name(buf_idx);
            let b = self.buf();
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

            // File tree indicator
            let tree_indicator = if self.filetree.is_some() {
                "[Tree] "
            } else {
                ""
            };

            // LSP diagnostic count + build problem panel counts
            let diag_indicator = {
                let bs = &self.buffers[buf_idx];
                let lsp_errors = bs
                    .diagnostics
                    .iter()
                    .filter(|(_, s, _)| *s == crate::lsp::protocol::DiagnosticSeverity::Error)
                    .count();
                let lsp_warnings = bs
                    .diagnostics
                    .iter()
                    .filter(|(_, s, _)| *s == crate::lsp::protocol::DiagnosticSeverity::Warning)
                    .count();
                let build_errors = self.problem_panel.error_count();
                let build_warnings = self.problem_panel.warning_count();
                let total_errors = lsp_errors + build_errors;
                let total_warnings = lsp_warnings + build_warnings;
                if total_errors > 0 || total_warnings > 0 {
                    format!(" E:{} W:{}", total_errors, total_warnings)
                } else {
                    String::new()
                }
            };

            let left = format!(
                " {}{}{}{}{}{}{}{}",
                tree_indicator,
                pane_indicator,
                buf_indicator,
                filename,
                modified_marker,
                regex_indicator,
                multi_cursor_indicator,
                diag_indicator
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

        // Minimap sidebar overlay — always on the buffer pane (not the terminal pane)
        if self.minimap.visible && self.diff_view.is_none() {
            self.render_minimap();
        }

        // Diff view full-screen overlay (drawn before help so Esc closes it first)
        if self.diff_view.is_some() {
            self.render_diff_view();
        }

        // Problem panel overlay (bottom panel)
        if self.problem_panel.visible {
            self.render_problem_panel();
        }

        // Help overlay (drawn on top of everything)
        if self.help_visible {
            self.render_help();
        }

        // Command palette overlay
        if self.palette.is_some() {
            self.render_palette();
        }

        // LSP overlays
        if self.completion_menu.is_some() {
            self.render_completion_menu();
        }
        if self.hover_popup.is_some() {
            self.render_hover_popup();
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
        } else if let Some(crate::layout::PaneContent::Terminal(ti)) =
            self.layout.pane_content(self.active_pane)
        {
            // Terminal pane cursor: offset by however many scrollback lines are
            // shown above the live buffer. If the cursor would land below the
            // bottom of the pane (scrolled too far up) we simply don't move it.
            if let Some(rect) = self.layout.pane_rect(self.active_pane)
                && ti < self.vterms.len()
                && self.vterms[ti].cursor_visible()
            {
                let pane_h = rect.height as usize;
                let scroll_off = self.vterms[ti].scroll_offset();
                let sb_lines = scroll_off
                    .min(self.vterms[ti].scrollback().len())
                    .min(pane_h);
                let (vt_row, vt_col) = self.vterms[ti].cursor_pos();
                let screen_row = rect.y as usize + sb_lines + vt_row as usize;
                if screen_row < rect.y as usize + pane_h {
                    let screen_col = rect.x as usize + vt_col as usize;
                    terminal::move_cursor((screen_row + 1) as u16, (screen_col + 1) as u16);
                }
                // If screen_row >= pane bottom the cursor is off-screen (viewing
                // old scrollback). Don't move the hardware cursor there.
            }
        } else if let Some(rect) = self.layout.pane_rect(self.active_pane) {
            let b = self.buf();
            if b.wrap_map.is_some() {
                // Wrapped mode: compute visual position
                let cursor_line = b.cursor().line;
                let cursor_col = b.cursor().col;
                let line_text = b.buffer.get_line(cursor_line).unwrap_or_default();

                let (cursor_visual_row, cursor_visual_col) = b
                    .wrap_map
                    .as_ref()
                    .map(|wm| wm.buffer_to_visual(cursor_line, cursor_col, &line_text))
                    .unwrap_or((0, 0));

                // Compute scroll start visual row
                let scroll_visual_row = b
                    .wrap_map
                    .as_ref()
                    .map(|wm| {
                        let (vr, _) = wm.buffer_to_visual(b.scroll_row, 0, "");
                        vr + b.scroll_visual_offset
                    })
                    .unwrap_or(0);

                let screen_row =
                    cursor_visual_row.saturating_sub(scroll_visual_row) + rect.y as usize;
                let screen_col = cursor_visual_col + b.gutter_width + rect.x as usize;

                terminal::move_cursor((screen_row + 1) as u16, (screen_col + 1) as u16);
            } else {
                let cursor_screen_row =
                    b.cursor().line.saturating_sub(b.scroll_row) + rect.y as usize;
                let pane_bottom = rect.y as usize + rect.height as usize;
                // Only position the caret when the cursor is inside the visible pane.
                // During scroll-only mode the cursor may be off-screen.
                if cursor_screen_row < pane_bottom {
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
            }
        }
        terminal::flush();
    }

    /// Render the tab bar at row 0 with scroll arrow support.
    fn render_tab_bar(&mut self, screen_width: usize) {
        let tab_bg = Color::Color256(236);
        let active_fg = Color::Ansi(0); // black
        let active_bg = Color::Ansi(7); // white
        let inactive_fg = Color::Color256(250); // dim
        let arrow_fg = Color::Ansi(6); // cyan for arrows
        let separator = " \u{2502} "; // " │ "

        // Fill row 0 with background
        for col in 0..screen_width {
            self.screen
                .put_char(0, col, ' ', inactive_fg, tab_bg, false);
        }

        let active_buf = self.active_buffer_index();
        let buf_count = self.buffers.len();

        // Pre-compute all tab labels and widths
        let mut tab_labels: Vec<(String, usize, usize)> = Vec::with_capacity(buf_count);
        for i in 0..buf_count {
            let name = self.buffer_display_name(i);
            let modified = self.buffers[i].buffer.is_modified();
            let label = if modified {
                format!(" {} [+] ", name)
            } else {
                format!(" {} ", name)
            };
            let width = crate::unicode::str_width(&label);
            tab_labels.push((label, width, i));
        }

        // Auto-scroll to ensure active buffer is visible
        if active_buf < self.tab_scroll_offset {
            self.tab_scroll_offset = active_buf;
        }

        // Clamp scroll offset
        if self.tab_scroll_offset >= buf_count {
            self.tab_scroll_offset = buf_count.saturating_sub(1);
        }

        // Try rendering; if active tab doesn't fit, increment offset and retry
        loop {
            let has_left_arrow = self.tab_scroll_offset > 0;
            let left_arrow_width: usize = if has_left_arrow { 3 } else { 0 };

            let mut col = left_arrow_width;
            let mut active_rendered = false;

            // Check if tabs starting from tab_scroll_offset fit the active tab
            for (i, &(_, tw, _)) in tab_labels.iter().enumerate().skip(self.tab_scroll_offset) {
                if i > self.tab_scroll_offset {
                    col += 3; // separator
                }
                // Reserve space for right arrow if there are more tabs after this
                let remaining_tabs = i + 1 < buf_count;
                let right_reserve = if remaining_tabs { 3 } else { 0 };

                if col + tw + right_reserve > screen_width && remaining_tabs {
                    // This tab doesn't fit and there are more
                    break;
                }
                if col + tw > screen_width {
                    break;
                }
                col += tw;
                if i == active_buf {
                    active_rendered = true;
                }
            }

            if active_rendered || self.tab_scroll_offset >= active_buf {
                break;
            }
            // Active tab wasn't rendered, scroll right
            self.tab_scroll_offset += 1;
        }

        let has_left_arrow = self.tab_scroll_offset > 0;

        let mut col = 0;
        let mut regions = Vec::new();

        // Render left arrow if scrolled
        if has_left_arrow {
            self.screen.put_str(0, 0, " < ", arrow_fg, tab_bg, true);
            regions.push((0, 3, usize::MAX)); // sentinel for left arrow
            col = 3;
        }

        let mut has_right_arrow = false;

        for (i, (label, label_width, buf_idx)) in
            tab_labels.iter().enumerate().skip(self.tab_scroll_offset)
        {
            let label_width = *label_width;
            let buf_idx = *buf_idx;
            // Add separator between tabs
            if i > self.tab_scroll_offset {
                if col + 3 <= screen_width {
                    self.screen
                        .put_str(0, col, separator, Color::Color256(240), tab_bg, false);
                    col += 3;
                } else {
                    break;
                }
            }

            // Check if this tab fits; if not and there are more tabs, show right arrow
            let remaining_tabs = i + 1 < buf_count;
            if col + label_width > screen_width.saturating_sub(if remaining_tabs { 3 } else { 0 })
                && remaining_tabs
            {
                // Doesn't fit, render right arrow
                has_right_arrow = true;
                break;
            }

            if col + label_width > screen_width {
                // Last tab, truncate what fits
                let available = screen_width.saturating_sub(col);
                if available == 0 {
                    break;
                }
                let truncated: String = label.chars().take(available).collect();
                let start_col = col;
                if buf_idx == active_buf {
                    self.screen
                        .put_str(0, col, &truncated, active_fg, active_bg, true);
                } else {
                    self.screen
                        .put_str(0, col, &truncated, inactive_fg, tab_bg, false);
                }
                col += available;
                regions.push((start_col, col, buf_idx));
                break;
            }

            let start_col = col;
            if buf_idx == active_buf {
                self.screen
                    .put_str(0, col, label, active_fg, active_bg, true);
            } else {
                self.screen
                    .put_str(0, col, label, inactive_fg, tab_bg, false);
            }
            col += label_width;
            regions.push((start_col, col, buf_idx));
        }

        // Render right arrow if there are more tabs
        if has_right_arrow {
            let arrow_start = screen_width.saturating_sub(3);
            // Clear any partial tab content under the arrow
            for c in arrow_start..screen_width {
                self.screen.put_char(0, c, ' ', arrow_fg, tab_bg, false);
            }
            self.screen
                .put_str(0, arrow_start, " > ", arrow_fg, tab_bg, true);
            regions.push((arrow_start, screen_width, usize::MAX - 1)); // sentinel for right arrow
        }

        self.tab_regions = regions;
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

        let has_wrap = self.buffers[buffer_idx].wrap_map.is_some();
        if has_wrap {
            self.render_editor_pane_wrapped(rect, buffer_idx, pane_id);
        } else {
            self.render_editor_pane_unwrapped(rect, buffer_idx, pane_id);
        }
    }

    /// Render a pane without word wrap (original behavior).
    fn render_editor_pane_unwrapped(&mut self, rect: Rect, buffer_idx: usize, pane_id: PaneId) {
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

                let semantic_spans = &self.buffers[buffer_idx].semantic_spans;
                let mut char_col: u32 = 0;

                // Collect diagnostic severity for this line
                let line_diag_severity = self.buffers[buffer_idx]
                    .diagnostics
                    .iter()
                    .filter(|(r, _, _)| {
                        r.start.line as usize <= file_line && r.end.line as usize >= file_line
                    })
                    .map(|(_, sev, _)| *sev)
                    .min_by_key(|s| *s as u8); // Error < Warning < Info < Hint

                // Override gutter indicator if there's a diagnostic on this line
                if let Some(sev) = line_diag_severity {
                    let (marker, marker_fg) = match sev {
                        crate::lsp::protocol::DiagnosticSeverity::Error => ('E', Color::Ansi(1)),
                        crate::lsp::protocol::DiagnosticSeverity::Warning => ('W', Color::Ansi(3)),
                        _ => ('I', Color::Ansi(6)),
                    };
                    // Put diagnostic marker at the leftmost gutter position
                    if gutter_width > 0 {
                        self.screen.put_char(
                            screen_row,
                            pane_x,
                            marker,
                            marker_fg,
                            Color::Default,
                            true,
                        );
                    }
                }

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
                        // String/comment regions: TextMate context is authoritative.
                        // Semantic tokens from the LSP must not override string/comment
                        // coloring because the LSP may emit tokens for identifiers inside
                        // string literals (e.g., 'HELLO' → LSP says HELLO is a variable).
                        let is_str_or_comment = spans
                            .as_ref()
                            .map(|s| {
                                highlight::is_in_string_or_comment(s, byte_offset_in_line)
                            })
                            .unwrap_or(false);

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
                            } else if !is_str_or_comment {
                                // Semantic tokens override TextMate for Zenith multi-language accuracy
                                if let Some((sem_fg, sem_bold)) = highlight::lookup_semantic_span(
                                    semantic_spans,
                                    file_line as u32,
                                    char_col,
                                ) {
                                    (sem_fg, Color::Default, sem_bold)
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
                            }
                        } else if !is_str_or_comment {
                            // Semantic tokens override TextMate for Zenith multi-language accuracy
                            if let Some((sem_fg, sem_bold)) = highlight::lookup_semantic_span(
                                semantic_spans,
                                file_line as u32,
                                char_col,
                            ) {
                                (sem_fg, Color::Default, sem_bold)
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

                        // Check if this character is within a diagnostic range
                        let diag_underline = self.buffers[buffer_idx]
                            .diagnostics
                            .iter()
                            .find(|(r, _, _)| {
                                let in_start = file_line > r.start.line as usize
                                    || (file_line == r.start.line as usize
                                        && byte_offset_in_line >= r.start.character as usize);
                                let in_end = file_line < r.end.line as usize
                                    || (file_line == r.end.line as usize
                                        && byte_offset_in_line < r.end.character as usize);
                                in_start && in_end
                            })
                            .map(|(_, sev, _)| *sev);

                        if let Some(sev) = diag_underline {
                            let underline_fg = match sev {
                                crate::lsp::protocol::DiagnosticSeverity::Error => Color::Ansi(1),
                                crate::lsp::protocol::DiagnosticSeverity::Warning => Color::Ansi(3),
                                _ => fg,
                            };
                            self.screen.put_cell_styled(
                                screen_row,
                                screen_col,
                                ch,
                                crate::render::CellStyle {
                                    fg: if is_selected { fg } else { underline_fg },
                                    bg,
                                    bold,
                                    underline: true,
                                    inverse: false,
                                    italic: false,
                                },
                            );
                        } else {
                            self.screen
                                .put_char(screen_row, screen_col, ch, fg, bg, bold);
                        }
                    }
                    byte_offset_in_line += ch.len_utf8();
                    char_col += 1;
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

    /// Render a pane with word wrap enabled.
    fn render_editor_pane_wrapped(&mut self, rect: Rect, buffer_idx: usize, pane_id: PaneId) {
        let pane_x = rect.x as usize;
        let pane_y = rect.y as usize;
        let pane_w = rect.width as usize;
        let pane_h = rect.height as usize;

        let bs = &self.buffers[buffer_idx];
        let scroll_row = bs.scroll_row;
        let scroll_visual_offset = bs.scroll_visual_offset;
        let gutter_width = bs.gutter_width;
        let line_count = bs.buffer.line_count();
        let has_git = bs.git_info.is_some();

        // Collect selection ranges
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

        // Walk from scroll_row, skipping scroll_visual_offset segments
        let mut file_line = scroll_row;
        let mut segment = scroll_visual_offset;
        let git_col_width = if has_git { 2 } else { 0 };

        for local_row in 0..pane_h {
            let screen_row = pane_y + local_row;

            if file_line >= line_count {
                // Tilde line
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
                continue;
            }

            let visual_rows_this_line = self.buffers[buffer_idx]
                .wrap_map
                .as_ref()
                .map(|wm| wm.visual_rows_for(file_line))
                .unwrap_or(1);

            let is_first_segment = segment == 0;

            // Git gutter (only on first segment)
            if has_git {
                if is_first_segment {
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
                } else {
                    self.screen.put_char(
                        screen_row,
                        pane_x,
                        ' ',
                        Color::Default,
                        Color::Default,
                        false,
                    );
                }
                self.screen.put_char(
                    screen_row,
                    pane_x + 1,
                    ' ',
                    Color::Default,
                    Color::Default,
                    false,
                );
            }

            // Gutter
            let line_num_width = gutter_width.saturating_sub(git_col_width);
            if line_num_width > 0 && self.config.line_numbers {
                let gutter_fg = Color::Color256(240);
                let gutter_bg = Color::Default;

                if is_first_segment {
                    // Show line number
                    let num_str = format!("{}", file_line + 1);
                    let pad = line_num_width.saturating_sub(num_str.len() + 1);
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
                } else {
                    // Continuation: show ↪ indicator
                    for col in 0..line_num_width.saturating_sub(2) {
                        self.screen.put_char(
                            screen_row,
                            pane_x + git_col_width + col,
                            ' ',
                            gutter_fg,
                            gutter_bg,
                            false,
                        );
                    }
                    let arrow_col = pane_x + git_col_width + line_num_width.saturating_sub(2);
                    self.screen.put_char(
                        screen_row, arrow_col, '\u{21AA}', gutter_fg, gutter_bg, false,
                    );
                    if line_num_width >= 1 {
                        self.screen.put_char(
                            screen_row,
                            arrow_col + 1,
                            ' ',
                            gutter_fg,
                            gutter_bg,
                            false,
                        );
                    }
                }
            }

            // Segment content
            let (seg_start, seg_end) = self.buffers[buffer_idx]
                .wrap_map
                .as_ref()
                .map(|wm| wm.segment_byte_range(file_line, segment))
                .unwrap_or((0, usize::MAX));

            let line_text = self.buffers[buffer_idx]
                .buffer
                .get_line(file_line)
                .unwrap_or_default();
            let line_start_byte = self.buffers[buffer_idx]
                .buffer
                .line_start(file_line)
                .unwrap_or(0);

            let seg_end_clamped = seg_end.min(line_text.len());

            // Syntax highlighting spans
            let spans = {
                let bs = &mut self.buffers[buffer_idx];
                bs.highlighter.as_mut().map(|hl| {
                    let buf_ref = &bs.buffer;
                    hl.style_line(file_line, &line_text, |l| buf_ref.get_line(l))
                })
            };

            let semantic_spans = &self.buffers[buffer_idx].semantic_spans;
            let mut char_col: u32 = 0;

            let mut display_col: usize = 0;
            let mut byte_offset_in_line: usize = 0;
            let text_start_col = pane_x + gutter_width;

            for ch in line_text.chars() {
                let char_len = ch.len_utf8();
                let cw = crate::unicode::char_width(ch);

                // Only render chars within this segment
                if byte_offset_in_line >= seg_start && byte_offset_in_line < seg_end_clamped {
                    let screen_col = text_start_col + display_col;
                    if screen_col >= pane_x + pane_w {
                        break;
                    }

                    let char_byte = line_start_byte + byte_offset_in_line;
                    let is_selected = sel_ranges
                        .iter()
                        .any(|(s, e)| char_byte >= *s && char_byte < *e);
                    let is_secondary_cursor = secondary_cursor_offsets.contains(&char_byte);

                    // String/comment regions: TextMate context is authoritative.
                    // Semantic tokens from the LSP must not override string/comment
                    // coloring because the LSP may emit tokens for identifiers inside
                    // string literals (e.g., 'HELLO' → LSP says HELLO is a variable).
                    let is_str_or_comment = spans
                        .as_ref()
                        .map(|s| {
                            highlight::is_in_string_or_comment(s, byte_offset_in_line)
                        })
                        .unwrap_or(false);

                    let (fg, bg, bold) = if is_selected {
                        (Color::Ansi(0), Color::Ansi(7), true)
                    } else if is_secondary_cursor {
                        (Color::Ansi(0), Color::Color256(246), true)
                    } else if pane_id == self.active_pane {
                        if let Some(is_current) = self.match_at_byte(char_byte) {
                            if is_current {
                                (Color::Ansi(0), Color::Ansi(6), true)
                            } else {
                                (Color::Ansi(0), Color::Ansi(3), false)
                            }
                        } else if !is_str_or_comment {
                            // Semantic tokens override TextMate for Zenith multi-language accuracy
                            if let Some((sem_fg, sem_bold)) = highlight::lookup_semantic_span(
                                semantic_spans,
                                file_line as u32,
                                char_col,
                            ) {
                                (sem_fg, Color::Default, sem_bold)
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
                        }
                    } else if !is_str_or_comment {
                        // Semantic tokens override TextMate for Zenith multi-language accuracy
                        if let Some((sem_fg, sem_bold)) = highlight::lookup_semantic_span(
                            semantic_spans,
                            file_line as u32,
                            char_col,
                        ) {
                            (sem_fg, Color::Default, sem_bold)
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
                    display_col += cw;
                }

                byte_offset_in_line += char_len;
                char_col += 1;
            }

            // Fill remaining with spaces
            let start_fill = text_start_col + display_col;
            let line_end_byte = line_start_byte + line_text.len();
            // Show trailing selection/cursor only on the last segment of the line
            let is_last_segment = segment + 1 >= visual_rows_this_line;
            for col in start_fill..(pane_x + pane_w) {
                let is_trailing_selected = is_last_segment
                    && sel_ranges
                        .iter()
                        .any(|(s, e)| line_end_byte >= *s && line_end_byte < *e)
                    && col == start_fill;
                let is_secondary_cursor_trail = is_last_segment
                    && secondary_cursor_offsets.contains(&line_end_byte)
                    && col == start_fill;
                let (fg, bg, bold) = if is_trailing_selected {
                    (Color::Ansi(0), Color::Ansi(7), true)
                } else if is_secondary_cursor_trail {
                    (Color::Ansi(0), Color::Color256(246), true)
                } else {
                    (Color::Default, Color::Default, false)
                };
                self.screen.put_char(screen_row, col, ' ', fg, bg, bold);
            }

            // Advance to next segment or next line
            segment += 1;
            if segment >= visual_rows_this_line {
                file_line += 1;
                segment = 0;
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

            // Check if there's a pane below (sharing a border row)
            let bottom_border_row = (r.y + r.height) as usize;
            let has_bottom_neighbor = panes
                .iter()
                .any(|other| other.id != pane.id && other.rect.y as usize == bottom_border_row + 1);

            if has_bottom_neighbor {
                // Draw horizontal border ─ on the row below this pane
                let start_col = r.x as usize;
                let end_col = (r.x + r.width) as usize;
                for col in start_col..end_col {
                    // Check if a vertical border already exists here → draw ┼
                    let has_vertical = panes.iter().any(|other| {
                        let rc = other.rect;
                        let right_col = (rc.x + rc.width) as usize;
                        right_col == col
                            && panes.iter().any(|adj| {
                                adj.id != other.id && adj.rect.x as usize == right_col + 1
                            })
                            && (rc.y as usize) <= bottom_border_row
                            && ((rc.y + rc.height) as usize) > bottom_border_row
                    });
                    let ch = if has_vertical {
                        '\u{253C}' // ┼
                    } else {
                        '\u{2500}' // ─
                    };
                    self.screen
                        .put_char(bottom_border_row, col, ch, fg, border_bg, false);
                }
            }
        }
    }

    pub(super) fn render_help(&mut self) {
        const HELP_LINES: &[&str] = &[
            "              \u{2500} Zedit Help \u{2500}               ",
            "                                              ",
            "  FILE               NAVIGATION              ",
            "  Ctrl+S   Save      \u{2191}\u{2193}\u{2190}\u{2192}   Move cursor      ",
            "  Ctrl+\u{21e7}S  Save As   Home/End Line start/end  ",
            "  Ctrl+O   Open      PgUp/Dn Page scroll     ",
            "  Ctrl+Q   Quit      Ctrl+G  Go to line      ",
            "  Ctrl+N   New buf   Ctrl+F  Find            ",
            "  Ctrl+W   Close buf Ctrl+H  Replace         ",
            "  Ctrl+PgDn/PgUp Next/Prev  F3/\u{21e7}F3  Matches   ",
            "                                              ",
            "  EDIT               SELECTION / MULTICURSOR ",
            "  Ctrl+Z/Y Undo/Redo Shift+\u{2190}\u{2192}\u{2191}\u{2193} Extend sel  ",
            "  Ctrl+C   Copy      Ctrl+A   Select all     ",
            "  Ctrl+X   Cut       Ctrl+L   Select line    ",
            "  Ctrl+V   Paste     Ctrl+D   Next match     ",
            "  Ctrl+\u{21e7}D  Dup line  Ctrl+\u{21e7}L  All matches    ",
            "  Ctrl+\u{21e7}K  Del line  Alt+Click Add cursor    ",
            "  Tab/\u{21e7}Tab Indent/Un Escape   One cursor     ",
            "  Ctrl+/   Comment                           ",
            "                                              ",
            "  PANES               VIEW                   ",
            "  Ctrl+\\   Horiz     Ctrl+B  File tree       ",
            "  Ctrl+\u{21e7}\\  Vert      Ctrl+P  Palette        ",
            "  Ctrl+\u{21e7}W  Close     Ctrl+T  Terminal       ",
            "  Alt+\u{2190}\u{2192}\u{2191}\u{2193}  Focus     Alt+Z   Word wrap      ",
            "  Alt+\u{21e7}\u{2190}\u{2192}  Resize    Alt+M   Minimap        ",
            "                                              ",
            "  LSP                 DIFF (vs HEAD)         ",
            "  Ctrl+Spc Complete  F7      Open diff       ",
            "  Alt+K    Hover doc F8/\u{21e7}F8  Next/Prev hunk  ",
            "  F12      Go to def Escape  Close diff      ",
            "                                              ",
            "  TASKS              REPL / PROBLEMS         ",
            "  F5       Run       F6    Problem panel     ",
            "  Ctrl+F5  Build     Alt+\u{21b5} Send to REPL  ",
            "  \u{21e7}F5      Test task                        ",
            "  Alt+F5   Stop task                         ",
            "                                              ",
            "       Press Esc or F1 to close              ",
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
                let shortcut_width = crate::unicode::str_width(&entry.shortcut);
                let shortcut_start = start_col + box_width - 1 - shortcut_width - 1;
                self.screen.put_str(
                    row,
                    shortcut_start,
                    &entry.shortcut,
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

    /// Render a terminal pane within the given rectangle.
    fn render_terminal_pane(&mut self, rect: Rect, term_idx: usize, _pane_id: PaneId) {
        let pane_x = rect.x as usize;
        let pane_y = rect.y as usize;
        let pane_w = rect.width as usize;
        let pane_h = rect.height as usize;

        if term_idx >= self.vterms.len() {
            // Terminal not found — fill with blank
            for local_row in 0..pane_h {
                for col in 0..pane_w {
                    self.screen.put_char(
                        pane_y + local_row,
                        pane_x + col,
                        ' ',
                        Color::Default,
                        Color::Default,
                        false,
                    );
                }
            }
            return;
        }

        let vt = &self.vterms[term_idx];
        let vt_cols = vt.cols() as usize;
        let vt_rows = vt.rows() as usize;
        let scroll_offset = vt.scroll_offset();
        let scrollback = vt.scrollback();

        // Number of scrollback lines to show at the top of the pane
        let scrollback_lines = scroll_offset.min(scrollback.len()).min(pane_h);
        // Remaining rows show the live screen buffer
        // Remaining rows show the live screen buffer

        for local_row in 0..pane_h {
            for local_col in 0..pane_w {
                let screen_row = pane_y + local_row;
                let screen_col = pane_x + local_col;
                let selected = vt.is_cell_selected(local_row as u16, local_col as u16);

                if local_row < scrollback_lines {
                    // Render from scrollback
                    let sb_idx = scrollback.len() - scroll_offset + local_row;
                    if sb_idx < scrollback.len() && local_col < scrollback[sb_idx].len() {
                        let cell = &scrollback[sb_idx][local_col];
                        let style = crate::render::CellStyle {
                            fg: if selected { cell.bg } else { cell.fg },
                            bg: if selected { cell.fg } else { cell.bg },
                            bold: cell.bold,
                            underline: cell.underline,
                            inverse: cell.inverse != selected,
                            italic: cell.italic,
                        };
                        self.screen
                            .put_cell_styled(screen_row, screen_col, cell.ch, style);
                    } else {
                        let style = crate::render::CellStyle {
                            fg: if selected { Color::Default } else { Color::Default },
                            bg: if selected { Color::Ansi(7) } else { Color::Default },
                            bold: false,
                            underline: false,
                            inverse: false,
                            italic: false,
                        };
                        self.screen.put_cell_styled(screen_row, screen_col, ' ', style);
                    }
                } else {
                    // Render from live screen buffer
                    let vt_row = local_row - scrollback_lines;
                    if vt_row < vt_rows && local_col < vt_cols {
                        let cell = &vt.cells()[vt_row * vt_cols + local_col];
                        let style = crate::render::CellStyle {
                            fg: if selected { cell.bg } else { cell.fg },
                            bg: if selected { cell.fg } else { cell.bg },
                            bold: cell.bold,
                            underline: cell.underline,
                            inverse: cell.inverse != selected,
                            italic: cell.italic,
                        };
                        self.screen
                            .put_cell_styled(screen_row, screen_col, cell.ch, style);
                    } else {
                        let style = crate::render::CellStyle {
                            fg: Color::Default,
                            bg: if selected { Color::Ansi(7) } else { Color::Default },
                            bold: false,
                            underline: false,
                            inverse: false,
                            italic: false,
                        };
                        self.screen.put_cell_styled(screen_row, screen_col, ' ', style);
                    }
                }
            }
        }

        // Show scrollback indicator when scrolled up
        if scroll_offset > 0 {
            let indicator = format!("[Scrollback: -{} lines]", scroll_offset);
            let ind_col = pane_x + pane_w.saturating_sub(indicator.len()) / 2;
            self.screen.put_str(
                pane_y,
                ind_col,
                &indicator,
                Color::Ansi(0),
                Color::Ansi(3),
                true,
            );
        }

        // Show "[Process exited]" if PTY is dead
        if term_idx < self.ptys.len() && self.ptys[term_idx].is_dead() {
            let msg = "[Process exited]";
            let msg_row = pane_y + pane_h / 2;
            let msg_col = pane_x + pane_w.saturating_sub(msg.len()) / 2;
            self.screen
                .put_str(msg_row, msg_col, msg, Color::Ansi(3), Color::Default, true);
        }
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
            let bi = match pane_info.content {
                crate::layout::PaneContent::Buffer(bi) => bi,
                crate::layout::PaneContent::Terminal(_) => continue,
            };
            if bi >= self.buffers.len() {
                continue;
            }

            let local_row = (row - r.y) as usize;
            let local_col = (col - r.x) as usize;

            let bs = &self.buffers[bi];

            if bs.wrap_map.is_some() {
                return self
                    .screen_to_buffer_wrapped(local_row, local_col, bi)
                    .map(|(line, byte_col)| (line, byte_col, pane_info.id));
            }

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

    /// Convert local pane coordinates to buffer position when wrapping is active.
    fn screen_to_buffer_wrapped(
        &self,
        local_row: usize,
        local_col: usize,
        buffer_idx: usize,
    ) -> Option<(usize, usize)> {
        let bs = &self.buffers[buffer_idx];
        let wm = bs.wrap_map.as_ref()?;
        let gutter_width = bs.gutter_width;

        // Walk from scroll position to find the file_line and segment
        let mut file_line = bs.scroll_row;
        let mut segment = bs.scroll_visual_offset;
        let line_count = bs.buffer.line_count();

        for row_i in 0..=local_row {
            if file_line >= line_count {
                return None;
            }
            if row_i == local_row {
                // This is the target row
                let display_col = local_col.saturating_sub(gutter_width);

                let line_text = bs.buffer.get_line(file_line).unwrap_or_default();
                let (seg_start, seg_end) = wm.segment_byte_range(file_line, segment);
                let seg_end_clamped = seg_end.min(line_text.len());
                let seg_text = &line_text[seg_start..seg_end_clamped];
                let byte_in_seg = display_col_to_byte_col(seg_text, display_col);
                return Some((file_line, seg_start + byte_in_seg));
            }

            // Advance
            let visual_rows = wm.visual_rows_for(file_line);
            segment += 1;
            if segment >= visual_rows {
                file_line += 1;
                segment = 0;
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // LSP overlay renderers
    // -----------------------------------------------------------------------

    pub(super) fn render_completion_menu(&mut self) {
        // Extract the data we need from the menu before drawing (releases borrow).
        let (items_data, selected, scroll_offset, anchor_row, anchor_col) = {
            let menu = match self.completion_menu.as_ref() {
                Some(m) => m,
                None => return,
            };
            let data: Vec<(String, &'static str)> = menu
                .items
                .iter()
                .map(|i| (i.label.clone(), i.kind_str))
                .collect();
            (
                data,
                menu.selected,
                menu.scroll_offset,
                menu.anchor_screen_row,
                menu.anchor_screen_col,
            )
        };

        if items_data.is_empty() {
            return;
        }

        let screen_h = self.screen.height();
        let screen_w = self.screen.width();
        // Reserve status bar rows
        let usable_h = screen_h.saturating_sub(self.status_height);

        const MAX_VISIBLE: usize = 10;
        let visible_count = items_data.len().min(MAX_VISIBLE);

        // Compute menu width
        let max_label = items_data.iter().map(|(l, _)| l.len()).max().unwrap_or(4);
        let max_label = max_label.max(4);
        let kind_w = 3;
        let inner_w = max_label + 1 + kind_w; // label + space + kind
        let total_w = (inner_w + 2).min(screen_w); // +2 for borders
        let total_h = visible_count + 2; // +2 for borders

        // Position: below cursor by default
        let mut start_row = anchor_row + 1;
        let mut start_col = anchor_col;

        // Clamp: flip above if menu doesn't fit below
        if start_row + total_h > usable_h {
            start_row = anchor_row.saturating_sub(total_h);
        }
        // Clamp right
        if start_col + total_w > screen_w {
            start_col = screen_w.saturating_sub(total_w);
        }

        let border_fg = Color::Ansi(6); // cyan
        let bg = Color::Color256(235); // dark
        let selected_bg = Color::Ansi(4); // blue
        let selected_fg = Color::Ansi(7); // white
        let normal_fg = Color::Ansi(7);
        let kind_fg = Color::Color256(240); // dim grey

        // Top border
        if start_row < screen_h {
            self.screen
                .put_char(start_row, start_col, '\u{256d}', border_fg, bg, false);
            for c in 1..total_w.saturating_sub(1) {
                self.screen
                    .put_char(start_row, start_col + c, '\u{2500}', border_fg, bg, false);
            }
            self.screen.put_char(
                start_row,
                start_col + total_w - 1,
                '\u{256e}',
                border_fg,
                bg,
                false,
            );
        }

        // Items
        for i in 0..visible_count {
            let idx = scroll_offset + i;
            if idx >= items_data.len() {
                break;
            }
            let row = start_row + 1 + i;
            if row >= usable_h {
                break;
            }

            let (ref label, kind_str) = items_data[idx];
            let is_selected = idx == selected;
            let item_bg = if is_selected { selected_bg } else { bg };
            let item_fg = if is_selected { selected_fg } else { normal_fg };

            // Fill the row
            for c in 0..total_w {
                self.screen
                    .put_char(row, start_col + c, ' ', item_fg, item_bg, false);
            }

            // Left border
            self.screen
                .put_char(row, start_col, '\u{2502}', border_fg, bg, false);

            // Label (truncated to fit)
            let avail = total_w.saturating_sub(2 + 1 + kind_w); // inner - separator - kind
            let label_truncated = if label.len() > avail {
                &label[..avail]
            } else {
                label.as_str()
            };
            self.screen
                .put_str(row, start_col + 1, label_truncated, item_fg, item_bg, false);

            // Kind hint right-aligned (before right border)
            let kind_col = start_col + total_w - 1 - kind_str.len();
            self.screen
                .put_str(row, kind_col, kind_str, kind_fg, item_bg, false);

            // Right border
            self.screen.put_char(
                row,
                start_col + total_w - 1,
                '\u{2502}',
                border_fg,
                bg,
                false,
            );
        }

        // Scroll indicators
        if scroll_offset > 0 && start_row + 1 < usable_h {
            self.screen.put_char(
                start_row + 1,
                start_col + total_w - 1,
                '\u{25b4}', // ▴
                border_fg,
                bg,
                false,
            );
        }
        if scroll_offset + visible_count < items_data.len() {
            let last_item_row = start_row + visible_count;
            if last_item_row < usable_h {
                self.screen.put_char(
                    last_item_row,
                    start_col + total_w - 1,
                    '\u{25be}', // ▾
                    border_fg,
                    bg,
                    false,
                );
            }
        }

        // Bottom border
        let bot_row = start_row + visible_count + 1;
        if bot_row < screen_h {
            self.screen
                .put_char(bot_row, start_col, '\u{2570}', border_fg, bg, false);
            for c in 1..total_w.saturating_sub(1) {
                self.screen
                    .put_char(bot_row, start_col + c, '\u{2500}', border_fg, bg, false);
            }
            self.screen.put_char(
                bot_row,
                start_col + total_w - 1,
                '\u{256f}',
                border_fg,
                bg,
                false,
            );
        }
    }

    pub(super) fn render_hover_popup(&mut self) {
        let (lines, anchor_row, anchor_col) = {
            let popup = match self.hover_popup.as_ref() {
                Some(p) => p,
                None => return,
            };
            (
                popup.lines.clone(),
                popup.anchor_screen_row,
                popup.anchor_screen_col,
            )
        };

        if lines.is_empty() {
            return;
        }

        let screen_h = self.screen.height();
        let screen_w = self.screen.width();
        let usable_h = screen_h.saturating_sub(self.status_height);

        let max_line_w = lines.iter().map(|l| l.len()).max().unwrap_or(0);
        // Header row adds " hover " label
        let inner_w = max_line_w.max(7);
        let total_w = (inner_w + 4).min(screen_w); // 4 = 2 border + 2 padding
        let total_h = lines.len() + 2; // +2 for top/bottom border

        // Position: above cursor by default
        let mut start_row = anchor_row.saturating_sub(total_h);
        // If no room above, place below
        if anchor_row < total_h {
            start_row = anchor_row + 1;
        }

        let mut start_col = anchor_col;
        if start_col + total_w > screen_w {
            start_col = screen_w.saturating_sub(total_w);
        }

        let border_fg = Color::Ansi(6); // cyan
        let bg = Color::Color256(235); // dark
        let text_fg = Color::Ansi(7); // white

        // Top border: ╭─ hover ─────╮
        if start_row < usable_h {
            self.screen
                .put_char(start_row, start_col, '\u{256d}', border_fg, bg, false);
            let header = " hover ";
            let dash_total = total_w.saturating_sub(2 + header.len());
            let dash_left = dash_total / 2;
            let dash_right = dash_total - dash_left;
            let mut col = start_col + 1;
            for _ in 0..dash_left {
                self.screen
                    .put_char(start_row, col, '\u{2500}', border_fg, bg, false);
                col += 1;
            }
            self.screen
                .put_str(start_row, col, header, border_fg, bg, false);
            col += header.len();
            for _ in 0..dash_right {
                self.screen
                    .put_char(start_row, col, '\u{2500}', border_fg, bg, false);
                col += 1;
            }
            self.screen
                .put_char(start_row, col, '\u{256e}', border_fg, bg, false);
        }

        // Content rows
        for (i, line) in lines.iter().enumerate() {
            let row = start_row + 1 + i;
            if row >= usable_h {
                break;
            }
            // Fill row
            for c in 0..total_w {
                self.screen
                    .put_char(row, start_col + c, ' ', text_fg, bg, false);
            }
            // Borders
            self.screen
                .put_char(row, start_col, '\u{2502}', border_fg, bg, false);
            self.screen.put_char(
                row,
                start_col + total_w - 1,
                '\u{2502}',
                border_fg,
                bg,
                false,
            );
            // Text (left-padded by 1)
            let avail = total_w.saturating_sub(4);
            let display = if line.len() > avail {
                &line[..avail]
            } else {
                line.as_str()
            };
            self.screen
                .put_str(row, start_col + 2, display, text_fg, bg, false);
        }

        // Bottom border
        let bot_row = start_row + lines.len() + 1;
        if bot_row < screen_h {
            self.screen
                .put_char(bot_row, start_col, '\u{2570}', border_fg, bg, false);
            for c in 1..total_w.saturating_sub(1) {
                self.screen
                    .put_char(bot_row, start_col + c, '\u{2500}', border_fg, bg, false);
            }
            self.screen.put_char(
                bot_row,
                start_col + total_w - 1,
                '\u{256f}',
                border_fg,
                bg,
                false,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Minimap renderer — right-edge overlay on the active buffer pane
    // -----------------------------------------------------------------------

    pub(super) fn render_minimap(&mut self) {
        use minimap::{MINIMAP_WIDTH, build_minimap};

        // Find the pane that hosts the active buffer.
        // When a terminal pane is active we fall back to the first buffer pane
        // so the minimap stays visible and doesn't flicker.
        let buf_idx = self.active_buffer_index();
        let minimap_pane = self
            .layout
            .panes()
            .iter()
            .find(|p| p.content == crate::layout::PaneContent::Buffer(buf_idx))
            .map(|p| p.id)
            .unwrap_or(self.active_pane);

        let rect = match self.layout.pane_rect(minimap_pane) {
            Some(r) => r,
            None => return,
        };

        let pane_x = rect.x as usize;
        let pane_y = rect.y as usize;
        let pane_w = rect.width as usize;
        let pane_h = rect.height as usize;

        // Need at least MINIMAP_WIDTH + a few columns for actual text
        if pane_w < MINIMAP_WIDTH + 8 {
            return;
        }

        let buf_idx = self.active_buffer_index();
        let buf = &self.buffers[buf_idx];
        let line_count = buf.buffer.line_count();
        let scroll_row = buf.scroll_row;
        let visible_rows = pane_h;

        // Minimap sits in the rightmost MINIMAP_WIDTH columns of the pane
        let minimap_col_start = pane_x + pane_w - MINIMAP_WIDTH;
        let minimap_row_start = pane_y;
        let minimap_rows = pane_h;

        // Build minimap data
        let mm_lines = {
            let buf_ref = &self.buffers[buf_idx].buffer;
            build_minimap(
                line_count,
                |i| buf_ref.get_line(i).unwrap_or_default(),
                scroll_row,
                visible_rows,
                minimap_rows,
            )
        };

        // Draw a thin separator line on the left edge of the minimap
        let sep_col = minimap_col_start.saturating_sub(1);
        for r in 0..minimap_rows {
            let screen_row = minimap_row_start + r;
            self.screen.put_char(
                screen_row,
                sep_col,
                '\u{2502}', // │
                Color::Color256(240),
                Color::Default,
                false,
            );
        }

        // Draw each minimap row
        for (r, mm_line) in mm_lines.iter().enumerate() {
            let screen_row = minimap_row_start + r;
            for (c, cell) in mm_line.cells.iter().enumerate() {
                let screen_col = minimap_col_start + c;
                self.screen
                    .put_char(screen_row, screen_col, cell.ch, cell.fg, cell.bg, false);
            }
        }

        // Fill any remaining rows past EOF with blanks
        let mm_line_count = mm_lines.len();
        if mm_line_count < minimap_rows {
            let blank_bg = Color::Color256(236);
            for r in mm_line_count..minimap_rows {
                let screen_row = minimap_row_start + r;
                for c in 0..MINIMAP_WIDTH {
                    self.screen.put_char(
                        screen_row,
                        minimap_col_start + c,
                        '\u{2800}',
                        Color::Color256(237),
                        blank_bg,
                        false,
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Diff view renderer — full-screen side-by-side overlay
    // -----------------------------------------------------------------------

    pub(super) fn render_diff_view(&mut self) {
        use crate::diff_view::RowKind;
        use crate::unicode::str_width;

        let screen_w = self.screen.width();
        let screen_h = self.screen.height();
        if screen_w < 8 || screen_h < 4 {
            return;
        }

        // Pull out what we need from diff_view (borrow checker)
        let dv = match self.diff_view.as_ref() {
            Some(d) => d,
            None => return,
        };

        let left_label = dv.left.label.clone();
        let right_label = dv.right.label.clone();
        let total_rows = dv.row_count();
        let scroll = dv.scroll;
        let current_hunk = dv.current_hunk;
        let hunk_count = dv.hunks.len();

        // Build display lines before we borrow screen
        // Each entry: (kind, left_text, right_text)
        let visible_h = screen_h.saturating_sub(3); // header + separator + footer
        let col_w = screen_w / 2;

        // Collect visible row data
        struct RowData {
            kind: RowKind,
            left: String,
            right: String,
        }
        let mut visible_rows: Vec<RowData> = Vec::with_capacity(visible_h);
        let dv = self.diff_view.as_ref().unwrap();
        for i in 0..visible_h {
            let ri = scroll + i;
            if ri >= total_rows {
                visible_rows.push(RowData {
                    kind: RowKind::Equal,
                    left: String::new(),
                    right: String::new(),
                });
                continue;
            }
            let row = &dv.rows[ri];
            let left_text = match row.left {
                Some(li) => dv.left.lines.get(li).cloned().unwrap_or_default(),
                None => String::new(),
            };
            let right_text = match row.right {
                Some(ri2) => dv.right.lines.get(ri2).cloned().unwrap_or_default(),
                None => String::new(),
            };
            visible_rows.push(RowData {
                kind: row.kind,
                left: left_text,
                right: right_text,
            });
        }

        // Colors
        let bg_default = Color::Color256(235);
        let fg_default = Color::Ansi(7);
        let bg_added = Color::Color256(22); // dark green
        let fg_added = Color::Ansi(2); // green
        let bg_deleted = Color::Color256(52); // dark red
        let fg_deleted = Color::Ansi(1); // red
        let bg_modified = Color::Color256(58); // dark yellow
        let fg_modified = Color::Ansi(3); // yellow
        let border_color = Color::Ansi(6); // cyan

        // --- Header row (row 0) ---
        // Fill header background
        for c in 0..screen_w {
            self.screen
                .put_char(0, c, ' ', fg_default, Color::Ansi(4), false);
        }
        // Left label
        let left_trunc = if left_label.len() > col_w.saturating_sub(4) {
            &left_label[..col_w.saturating_sub(4)]
        } else {
            &left_label
        };
        self.screen
            .put_str(0, 2, left_trunc, Color::Ansi(7), Color::Ansi(4), true);
        // Divider
        if col_w < screen_w {
            self.screen
                .put_char(0, col_w, '\u{2502}', border_color, Color::Ansi(4), false);
        }
        // Right label
        let right_trunc = if right_label.len() > col_w.saturating_sub(4) {
            &right_label[..col_w.saturating_sub(4)]
        } else {
            &right_label
        };
        self.screen.put_str(
            0,
            col_w + 2,
            right_trunc,
            Color::Ansi(7),
            Color::Ansi(4),
            true,
        );

        // --- Content rows ---
        for (i, row) in visible_rows.iter().enumerate() {
            let screen_row = i + 1; // row 0 is header
            if screen_row >= screen_h.saturating_sub(1) {
                break;
            }

            let (row_bg, row_fg) = match row.kind {
                RowKind::Equal => (bg_default, fg_default),
                RowKind::Added => (bg_added, fg_added),
                RowKind::Deleted => (bg_deleted, fg_deleted),
                RowKind::Modified => (bg_modified, fg_modified),
            };

            // Fill entire row with background
            for c in 0..screen_w {
                self.screen
                    .put_char(screen_row, c, ' ', row_fg, row_bg, false);
            }

            // Left cell — blank for Added rows
            if row.kind != RowKind::Added {
                let avail = col_w.saturating_sub(2);
                let display = truncate_str(&row.left, avail);
                let w = str_width(display);
                if w > 0 {
                    self.screen
                        .put_str(screen_row, 1, display, row_fg, row_bg, false);
                }
            }

            // Center divider
            if col_w < screen_w {
                self.screen
                    .put_char(screen_row, col_w, '\u{2502}', border_color, row_bg, false);
            }

            // Right cell — blank for Deleted rows
            if row.kind != RowKind::Deleted {
                let avail = screen_w.saturating_sub(col_w + 2);
                let display = truncate_str(&row.right, avail);
                let w = str_width(display);
                if w > 0 {
                    self.screen
                        .put_str(screen_row, col_w + 1, display, row_fg, row_bg, false);
                }
            }
        }

        // --- Footer ---
        let footer_row = screen_h.saturating_sub(1);
        for c in 0..screen_w {
            self.screen
                .put_char(footer_row, c, ' ', fg_default, Color::Ansi(0), false);
        }
        let hunk_str = if hunk_count == 0 {
            "No changes".to_string()
        } else {
            format!("Hunk {}/{}", current_hunk + 1, hunk_count)
        };
        let footer_text = format!(
            " {} | F8 next  Shift+F8 prev  Up/Down scroll  Esc close",
            hunk_str
        );
        let footer_trunc = if footer_text.len() > screen_w {
            &footer_text[..screen_w]
        } else {
            &footer_text
        };
        self.screen.put_str(
            footer_row,
            0,
            footer_trunc,
            Color::Ansi(7),
            Color::Ansi(0),
            false,
        );
    }

    // -----------------------------------------------------------------------
    // Problem panel renderer — bottom overlay panel
    // -----------------------------------------------------------------------

    pub(super) fn render_problem_panel(&mut self) {
        use crate::problem_panel::Severity;

        let screen_w = self.screen.width();
        let screen_h = self.screen.height();
        if screen_w < 10 || screen_h < 5 {
            return;
        }

        // Panel geometry: occupy up to 1/3 of screen height, min 5 rows.
        let pp = &self.problem_panel;
        let item_count = pp.items.len();
        let visible_items = (screen_h / 3).max(5).min(item_count + 1);
        // rows: 1 title + visible_items rows + 1 footer = visible_items + 2
        let panel_h = (visible_items + 2).min(screen_h);
        let panel_y = screen_h.saturating_sub(panel_h + self.status_height);

        let title_bg = Color::Color256(25); // dark blue
        let title_fg = Color::Ansi(15); // white
        let item_bg = Color::Color256(235); // near-black
        let item_bg_sel = Color::Color256(24); // selected: blue
        let item_fg = Color::Ansi(15);
        let err_fg = Color::Ansi(9); // bright red
        let warn_fg = Color::Ansi(11); // yellow
        let info_fg = Color::Ansi(12); // blue
        let note_fg = Color::Ansi(14); // cyan
        let footer_bg = Color::Color256(238);
        let footer_fg = Color::Color256(250);

        // Clamp panel scroll
        let selected = pp.selected;
        let scroll = {
            let list_rows = panel_h.saturating_sub(2); // excluding title + footer
            let mut sc = pp.scroll;
            if selected >= sc + list_rows && list_rows > 0 {
                sc = selected + 1 - list_rows;
            }
            if selected < sc {
                sc = selected;
            }
            sc
        };

        // -- Title row --
        let source = pp.source_cmd.as_deref().unwrap_or("Build Output");
        let errors = pp.error_count();
        let warnings = pp.warning_count();
        let focus_marker = if pp.focused { "●" } else { "○" };
        let title = format!(
            " {} Problems [{}]  E:{} W:{}  {}",
            focus_marker,
            source,
            errors,
            warnings,
            if pp.focused {
                "↑↓ navigate  Enter jump  Esc close"
            } else {
                "F6 focus"
            }
        );
        for c in 0..screen_w {
            self.screen
                .put_char(panel_y, c, ' ', title_fg, title_bg, false);
        }
        let title_trunc = truncate_str(&title, screen_w);
        self.screen
            .put_str(panel_y, 0, title_trunc, title_fg, title_bg, false);

        // -- Item rows --
        let list_rows = panel_h.saturating_sub(2);
        for row in 0..list_rows {
            let item_row = panel_y + 1 + row;
            if item_row >= screen_h {
                break;
            }
            let item_idx = scroll + row;
            let (bg, fg_sev, text) = if item_idx < pp.items.len() {
                let p = &pp.items[item_idx];
                let bg = if item_idx == selected {
                    item_bg_sel
                } else {
                    item_bg
                };
                let fg_sev = match p.severity {
                    Severity::Error => err_fg,
                    Severity::Warning => warn_fg,
                    Severity::Info => info_fg,
                    Severity::Note => note_fg,
                };
                let code_part = p
                    .code
                    .as_ref()
                    .map(|c| format!("[{}] ", c))
                    .unwrap_or_default();
                let text = format!(
                    " [{}] {}{}:{}  {}{}",
                    p.severity.label(),
                    code_part,
                    p.file,
                    p.line,
                    p.message,
                    " ".repeat(screen_w)
                );
                (bg, fg_sev, text)
            } else {
                // Empty row
                (item_bg, item_fg, " ".repeat(screen_w))
            };

            // Fill row background
            for c in 0..screen_w {
                self.screen.put_char(item_row, c, ' ', item_fg, bg, false);
            }
            // Write severity indicator in its color
            let text_trunc = truncate_str(&text, screen_w);
            // Write severity char with color
            if !text_trunc.is_empty() {
                // First char is space, then [E] / [W] etc — write whole line with item_fg
                // but the severity letter gets fg_sev color
                // Split: " [X]" = 4 bytes, rest is normal
                if text_trunc.len() >= 4 {
                    self.screen
                        .put_str(item_row, 0, &text_trunc[..1], item_fg, bg, false);
                    self.screen
                        .put_str(item_row, 1, &text_trunc[1..4], fg_sev, bg, false);
                    self.screen
                        .put_str(item_row, 4, &text_trunc[4..], item_fg, bg, false);
                } else {
                    self.screen
                        .put_str(item_row, 0, text_trunc, item_fg, bg, false);
                }
            }
        }

        // -- Footer row --
        let footer_row = panel_y + 1 + list_rows;
        if footer_row < screen_h {
            for c in 0..screen_w {
                self.screen
                    .put_char(footer_row, c, ' ', footer_fg, footer_bg, false);
            }
            let footer = format!(
                " {}/{} items",
                if item_count == 0 { 0 } else { selected + 1 },
                item_count
            );
            let footer_trunc = truncate_str(&footer, screen_w);
            self.screen
                .put_str(footer_row, 0, footer_trunc, footer_fg, footer_bg, false);
        }
    }
}

/// Truncate a string to at most `max_cols` display columns (ASCII-safe approximation).
fn truncate_str(s: &str, max_cols: usize) -> &str {
    if max_cols == 0 {
        return "";
    }
    let mut width = 0;
    let mut byte_pos = 0;
    for ch in s.chars() {
        let cw = if ch == '\t' {
            4
        } else {
            crate::unicode::char_width(ch)
        };
        if width + cw > max_cols {
            break;
        }
        width += cw;
        byte_pos += ch.len_utf8();
    }
    &s[..byte_pos]
}
