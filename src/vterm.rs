// ---------------------------------------------------------------------------
// VTerm — VT100/xterm terminal emulator state machine
// ---------------------------------------------------------------------------
//
// Supports cursor movement, erase, scroll regions, SGR (bold, italic,
// underline, inverse, ANSI/256/RGB colors), alternate screen buffer,
// DECSET/DECRST modes, OSC title, DSR 6n, and scrollback.

use crate::render::Color;

// ---------------------------------------------------------------------------
// VTermCell — single cell in the grid
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct VTermCell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub underline: bool,
    pub inverse: bool,
    pub italic: bool,
}

impl Default for VTermCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            underline: false,
            inverse: false,
            italic: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Parser state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum ParseState {
    Normal,
    Escape,
    Csi(Vec<u8>),
    Osc(Vec<u8>),
}

// ---------------------------------------------------------------------------
// VTerm
// ---------------------------------------------------------------------------

pub struct VTerm {
    cols: u16,
    rows: u16,
    cells: Vec<VTermCell>,
    alt_cells: Option<Vec<VTermCell>>,
    alt_active: bool,
    cursor_col: u16,
    cursor_row: u16,
    saved_cursor: (u16, u16),
    // Current SGR attributes
    cur_fg: Color,
    cur_bg: Color,
    cur_bold: bool,
    cur_underline: bool,
    cur_inverse: bool,
    cur_italic: bool,
    // Scroll region
    scroll_top: u16,
    scroll_bot: u16,
    // Autowrap mode
    autowrap: bool,
    // Pending wrap (cursor at right edge, next printable wraps)
    pending_wrap: bool,
    // Parser state
    state: ParseState,
    // Scrollback
    scrollback: Vec<Vec<VTermCell>>,
    scrollback_max: usize,
    scroll_offset: usize,
    // Metadata
    pub title: String,
    pub cursor_visible: bool,
    // Response queue (for DSR 6n)
    responses: Vec<Vec<u8>>,
    // UTF-8 multi-byte decode buffer
    utf8_buf: Vec<u8>,
    utf8_remaining: u8,
    // Mouse text selection (pane-local display coordinates)
    pub sel_anchor: Option<(u16, u16)>, // (local_row, local_col) where drag started
    pub sel_active: Option<(u16, u16)>, // current drag endpoint
}

impl VTerm {
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = cols as usize * rows as usize;
        Self {
            cols,
            rows,
            cells: vec![VTermCell::default(); size],
            alt_cells: None,
            alt_active: false,
            cursor_col: 0,
            cursor_row: 0,
            saved_cursor: (0, 0),
            cur_fg: Color::Default,
            cur_bg: Color::Default,
            cur_bold: false,
            cur_underline: false,
            cur_inverse: false,
            cur_italic: false,
            scroll_top: 0,
            scroll_bot: rows.saturating_sub(1),
            autowrap: true,
            pending_wrap: false,
            state: ParseState::Normal,
            scrollback: Vec::new(),
            scrollback_max: 1000,
            scroll_offset: 0,
            title: String::new(),
            cursor_visible: true,
            responses: Vec::new(),
            utf8_buf: Vec::new(),
            utf8_remaining: 0,
            sel_anchor: None,
            sel_active: None,
        }
    }

    // -- Public API --

    pub fn feed(&mut self, data: &[u8]) {
        for &byte in data {
            self.process_byte(byte);
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let old_cols = self.cols;
        let old_rows = self.rows;
        let new_size = cols as usize * rows as usize;
        let mut new_cells = vec![VTermCell::default(); new_size];
        let copy_cols = old_cols.min(cols) as usize;

        if rows < old_rows {
            // Shrinking: preserve the BOTTOM `rows` rows.
            // Push the top `delta` rows into scrollback so they are not lost.
            let delta = (old_rows - rows) as usize;
            for r in 0..delta {
                let row_start = r * old_cols as usize;
                let row: Vec<VTermCell> = self.cells[row_start..row_start + old_cols as usize].to_vec();
                self.scrollback.push(row);
                if self.scrollback.len() > self.scrollback_max {
                    self.scrollback.remove(0);
                }
            }
            // Copy rows delta..old_rows → 0..rows in the new buffer.
            for r in 0..rows as usize {
                for c in 0..copy_cols {
                    new_cells[r * cols as usize + c] =
                        self.cells[(r + delta) * old_cols as usize + c];
                }
            }
            // Adjust cursor: same logical position, shifted up by delta.
            self.cursor_row = self.cursor_row.saturating_sub(delta as u16);
        } else if rows > old_rows {
            // Growing: pull lines from scrollback into the top of the new buffer
            // so existing content stays and scrollback history becomes visible.
            let delta = (rows - old_rows) as usize;
            let pull = delta.min(self.scrollback.len());
            // Shift existing content down by `pull` rows.
            for r in (0..old_rows as usize).rev() {
                for c in 0..copy_cols {
                    new_cells[(r + pull) * cols as usize + c] =
                        self.cells[r * old_cols as usize + c];
                }
            }
            // Fill the top `pull` rows from the end of scrollback.
            let sb_base = self.scrollback.len() - pull;
            for i in 0..pull {
                let row = &self.scrollback[sb_base + i];
                for c in 0..row.len().min(cols as usize) {
                    new_cells[i * cols as usize + c] = row[c];
                }
            }
            self.scrollback.truncate(sb_base);
            // Adjust cursor: moves down by the number of pulled rows.
            self.cursor_row = (self.cursor_row as usize + pull)
                .min(rows as usize - 1) as u16;
        } else {
            // Same height: just reflow columns.
            for r in 0..old_rows as usize {
                for c in 0..copy_cols {
                    new_cells[r * cols as usize + c] = self.cells[r * old_cols as usize + c];
                }
            }
        }

        self.cells = new_cells;
        self.cols = cols;
        self.rows = rows;
        self.scroll_bot = rows.saturating_sub(1);
        if self.scroll_top >= rows {
            self.scroll_top = 0;
        }
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));

        // Resize alt buffer if active (simple copy from top, shell redraws anyway).
        if let Some(ref mut alt) = self.alt_cells {
            let copy_rows = old_rows.min(rows) as usize;
            let mut new_alt = vec![VTermCell::default(); new_size];
            for r in 0..copy_rows {
                for c in 0..copy_cols {
                    new_alt[r * cols as usize + c] = alt[r * old_cols as usize + c];
                }
            }
            *alt = new_alt;
        }
    }

    pub fn cells(&self) -> &[VTermCell] {
        &self.cells
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cursor_pos(&self) -> (u16, u16) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn scroll_view(&mut self, delta: isize) {
        let max = self.scrollback.len();
        if delta < 0 {
            self.scroll_offset = self
                .scroll_offset
                .saturating_add((-delta) as usize)
                .min(max);
        } else {
            self.scroll_offset = self.scroll_offset.saturating_sub(delta as usize);
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn scrollback(&self) -> &[Vec<VTermCell>] {
        &self.scrollback
    }

    pub fn take_responses(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.responses)
    }

    // -- Text selection --

    pub fn set_sel_anchor(&mut self, row: u16, col: u16) {
        self.sel_anchor = Some((row, col));
        self.sel_active = None;
    }

    pub fn set_sel_active(&mut self, row: u16, col: u16) {
        self.sel_active = Some((row, col));
    }

    pub fn clear_selection(&mut self) {
        self.sel_anchor = None;
        self.sel_active = None;
    }

    pub fn has_selection(&self) -> bool {
        self.sel_anchor.is_some() && self.sel_active.is_some()
    }

    /// Return normalized selection range ((r1,c1),(r2,c2)) with r1<=r2.
    pub fn sel_range(&self) -> Option<((u16, u16), (u16, u16))> {
        let a = self.sel_anchor?;
        let b = self.sel_active?;
        if a.0 < b.0 || (a.0 == b.0 && a.1 <= b.1) {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }

    /// True if the cell at (local_row, local_col) falls inside the selection.
    pub fn is_cell_selected(&self, local_row: u16, local_col: u16) -> bool {
        let ((r1, c1), (r2, c2)) = match self.sel_range() {
            Some(r) => r,
            None => return false,
        };
        if local_row < r1 || local_row > r2 {
            return false;
        }
        if local_row == r1 && local_col < c1 {
            return false;
        }
        if local_row == r2 && local_col > c2 {
            return false;
        }
        true
    }

    /// Extract the selected text from the visible display (scrollback + live cells).
    /// `pane_h` and `pane_w` describe the pane dimensions used during rendering.
    pub fn selection_text(&self, pane_h: usize, pane_w: usize) -> Option<String> {
        let ((r1, c1), (r2, c2)) = self.sel_range()?;
        let scrollback_lines = self.scroll_offset.min(self.scrollback.len()).min(pane_h);
        let mut text = String::new();
        for row in r1..=r2 {
            let col_start = if row == r1 { c1 } else { 0 };
            let col_end = if row == r2 { c2 + 1 } else { pane_w as u16 };
            let row_chars: String = if (row as usize) < scrollback_lines {
                let sb_idx = self.scrollback.len().saturating_sub(self.scroll_offset) + row as usize;
                (col_start..col_end)
                    .map(|c| {
                        if sb_idx < self.scrollback.len()
                            && (c as usize) < self.scrollback[sb_idx].len()
                        {
                            self.scrollback[sb_idx][c as usize].ch
                        } else {
                            ' '
                        }
                    })
                    .collect()
            } else {
                let vt_row = (row as usize).saturating_sub(scrollback_lines);
                (col_start..col_end)
                    .map(|c| {
                        if vt_row < self.rows as usize && (c as usize) < self.cols as usize {
                            self.cells[vt_row * self.cols as usize + c as usize].ch
                        } else {
                            ' '
                        }
                    })
                    .collect()
            };
            let trimmed = row_chars.trim_end_matches(' ');
            text.push_str(trimmed);
            if row < r2 {
                text.push('\n');
            }
        }
        if text.trim().is_empty() { None } else { Some(text) }
    }

    // -- Internal byte processing --

    fn process_byte(&mut self, byte: u8) {
        match self.state.clone() {
            ParseState::Normal => self.process_normal(byte),
            ParseState::Escape => self.process_escape(byte),
            ParseState::Csi(ref buf) => {
                let mut buf = buf.clone();
                self.process_csi(byte, &mut buf);
            }
            ParseState::Osc(ref buf) => {
                let mut buf = buf.clone();
                self.process_osc(byte, &mut buf);
            }
        }
    }

    fn process_normal(&mut self, byte: u8) {
        match byte {
            0x1b => self.state = ParseState::Escape,
            0x07 => {} // BEL — ignore
            0x08 => {
                // BS — backspace
                self.pending_wrap = false;
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
            }
            0x09 => {
                // HT — horizontal tab
                self.pending_wrap = false;
                let next_tab = ((self.cursor_col / 8) + 1) * 8;
                self.cursor_col = next_tab.min(self.cols.saturating_sub(1));
            }
            0x0a..=0x0c => {
                // LF, VT, FF — line feed
                self.pending_wrap = false;
                self.line_feed();
            }
            0x0d => {
                // CR — carriage return
                self.pending_wrap = false;
                self.cursor_col = 0;
            }
            0x00..=0x1f => {} // Other control chars — ignore
            0x20..=0x7e => {
                // ASCII printable
                self.put_char(byte as char);
            }
            0x7f => {} // DEL — ignore
            0xc0..=0xff => {
                // UTF-8 lead byte: start new sequence
                self.utf8_buf.clear();
                self.utf8_buf.push(byte);
                self.utf8_remaining = if byte >= 0xf0 { 3 } else if byte >= 0xe0 { 2 } else { 1 };
            }
            _ => {
                // UTF-8 continuation byte (0x80-0xBF)
                if !self.utf8_buf.is_empty() && self.utf8_remaining > 0 {
                    self.utf8_buf.push(byte);
                    self.utf8_remaining -= 1;
                    if self.utf8_remaining == 0 {
                        let ch = std::str::from_utf8(&self.utf8_buf)
                            .ok()
                            .and_then(|s| s.chars().next())
                            .unwrap_or('\u{FFFD}');
                        self.utf8_buf.clear();
                        self.put_char(ch);
                    }
                }
                // Stray continuation with no lead: ignore
            }
        }
    }

    fn put_char(&mut self, ch: char) {
        if self.pending_wrap && self.autowrap {
            self.pending_wrap = false;
            self.cursor_col = 0;
            self.line_feed();
        }

        let idx = self.cell_idx(self.cursor_row, self.cursor_col);
        self.cells[idx] = VTermCell {
            ch,
            fg: self.cur_fg,
            bg: self.cur_bg,
            bold: self.cur_bold,
            underline: self.cur_underline,
            inverse: self.cur_inverse,
            italic: self.cur_italic,
        };

        if self.cursor_col + 1 >= self.cols {
            self.pending_wrap = true;
        } else {
            self.cursor_col += 1;
        }
    }

    fn process_escape(&mut self, byte: u8) {
        self.state = ParseState::Normal;
        match byte {
            b'[' => {
                self.state = ParseState::Csi(Vec::new());
            }
            b']' => {
                self.state = ParseState::Osc(Vec::new());
            }
            b'D' => {
                // IND — index (move down)
                self.line_feed();
            }
            b'E' => {
                // NEL — next line
                self.cursor_col = 0;
                self.line_feed();
            }
            b'M' => {
                // RI — reverse index (move up)
                self.reverse_index();
            }
            b'7' => {
                // DECSC — save cursor
                self.saved_cursor = (self.cursor_row, self.cursor_col);
            }
            b'8' => {
                // DECRC — restore cursor
                self.cursor_row = self.saved_cursor.0.min(self.rows.saturating_sub(1));
                self.cursor_col = self.saved_cursor.1.min(self.cols.saturating_sub(1));
            }
            b'c' => {
                // RIS — full reset
                self.full_reset();
            }
            _ => {} // Unknown escape
        }
    }

    fn process_csi(&mut self, byte: u8, buf: &mut Vec<u8>) {
        match byte {
            // Parameter bytes
            b'0'..=b'9' | b';' | b'?' | b'>' | b'!' | b' ' => {
                buf.push(byte);
                self.state = ParseState::Csi(buf.clone());
            }
            // Final byte
            0x40..=0x7e => {
                self.state = ParseState::Normal;
                self.execute_csi(byte, buf);
            }
            _ => {
                self.state = ParseState::Normal;
            }
        }
    }

    fn process_osc(&mut self, byte: u8, buf: &mut Vec<u8>) {
        match byte {
            0x07 => {
                // BEL terminates OSC
                self.state = ParseState::Normal;
                self.execute_osc(buf);
            }
            0x1b => {
                // ESC might start ST (\x1b\\)
                // For simplicity, treat as end of OSC
                self.state = ParseState::Normal;
                self.execute_osc(buf);
            }
            _ => {
                if buf.len() < 4096 {
                    buf.push(byte);
                }
                self.state = ParseState::Osc(buf.clone());
            }
        }
    }

    // -- CSI execution --

    fn execute_csi(&mut self, final_byte: u8, buf: &[u8]) {
        // Check for private mode prefix
        let is_private = buf.first() == Some(&b'?');
        let param_buf = if is_private { &buf[1..] } else { buf };

        // Parse parameters
        let params = self.parse_params(param_buf);

        match final_byte {
            b'A' => {
                // CUU — cursor up
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor_row = self.cursor_row.saturating_sub(n);
                self.pending_wrap = false;
            }
            b'B' => {
                // CUD — cursor down
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor_row = (self.cursor_row + n).min(self.rows.saturating_sub(1));
                self.pending_wrap = false;
            }
            b'C' => {
                // CUF — cursor forward
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor_col = (self.cursor_col + n).min(self.cols.saturating_sub(1));
                self.pending_wrap = false;
            }
            b'D' => {
                // CUB — cursor back
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor_col = self.cursor_col.saturating_sub(n);
                self.pending_wrap = false;
            }
            b'E' => {
                // CNL — cursor next line
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor_row = (self.cursor_row + n).min(self.rows.saturating_sub(1));
                self.cursor_col = 0;
                self.pending_wrap = false;
            }
            b'F' => {
                // CPL — cursor previous line
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor_row = self.cursor_row.saturating_sub(n);
                self.cursor_col = 0;
                self.pending_wrap = false;
            }
            b'G' => {
                // CHA — cursor character absolute (column)
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor_col = (n - 1).min(self.cols.saturating_sub(1));
                self.pending_wrap = false;
            }
            b'H' | b'f' => {
                // CUP — cursor position
                let row = params.first().copied().unwrap_or(1).max(1);
                let col = params.get(1).copied().unwrap_or(1).max(1);
                self.cursor_row = (row - 1).min(self.rows.saturating_sub(1));
                self.cursor_col = (col - 1).min(self.cols.saturating_sub(1));
                self.pending_wrap = false;
            }
            b'J' => {
                // ED — erase in display
                let mode = params.first().copied().unwrap_or(0);
                self.erase_display(mode);
            }
            b'K' => {
                // EL — erase in line
                let mode = params.first().copied().unwrap_or(0);
                self.erase_line(mode);
            }
            b'L' => {
                // IL — insert lines
                let n = params.first().copied().unwrap_or(1).max(1);
                self.insert_lines(n);
            }
            b'M' => {
                // DL — delete lines
                let n = params.first().copied().unwrap_or(1).max(1);
                self.delete_lines(n);
            }
            b'P' => {
                // DCH — delete characters
                let n = params.first().copied().unwrap_or(1).max(1);
                self.delete_chars(n);
            }
            b'@' => {
                // ICH — insert characters
                let n = params.first().copied().unwrap_or(1).max(1);
                self.insert_chars(n);
            }
            b'X' => {
                // ECH — erase characters
                let n = params.first().copied().unwrap_or(1).max(1);
                self.erase_chars(n);
            }
            b'S' => {
                // SU — scroll up
                let n = params.first().copied().unwrap_or(1).max(1);
                for _ in 0..n {
                    self.scroll_up();
                }
            }
            b'T' => {
                // SD — scroll down
                let n = params.first().copied().unwrap_or(1).max(1);
                for _ in 0..n {
                    self.scroll_down();
                }
            }
            b'd' => {
                // VPA — vertical position absolute (row)
                let n = params.first().copied().unwrap_or(1).max(1);
                self.cursor_row = (n - 1).min(self.rows.saturating_sub(1));
                self.pending_wrap = false;
            }
            b'r' => {
                // DECSTBM — set scrolling region
                let top = params.first().copied().unwrap_or(1).max(1);
                let bot = params.get(1).copied().unwrap_or(self.rows).max(1);
                self.scroll_top = (top - 1).min(self.rows.saturating_sub(1));
                self.scroll_bot = (bot - 1).min(self.rows.saturating_sub(1));
                if self.scroll_top >= self.scroll_bot {
                    self.scroll_top = 0;
                    self.scroll_bot = self.rows.saturating_sub(1);
                }
                // Home cursor
                self.cursor_row = 0;
                self.cursor_col = 0;
                self.pending_wrap = false;
            }
            b'm' => {
                // SGR — select graphic rendition
                self.execute_sgr(&params);
            }
            b'h' if is_private => {
                // DECSET
                for &p in &params {
                    self.decset(p, true);
                }
            }
            b'l' if is_private => {
                // DECRST
                for &p in &params {
                    self.decset(p, false);
                }
            }
            b'n' => {
                // DSR — device status report
                if params.first() == Some(&6) {
                    // CPR — cursor position report
                    let response = format!("\x1b[{};{}R", self.cursor_row + 1, self.cursor_col + 1);
                    self.responses.push(response.into_bytes());
                }
            }
            b'c' => {
                // DA — device attributes
                // Respond as VT100
                self.responses.push(b"\x1b[?1;2c".to_vec());
            }
            _ => {} // Unknown CSI
        }
    }

    // -- SGR (Select Graphic Rendition) --

    fn execute_sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            self.reset_sgr();
            return;
        }

        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => self.reset_sgr(),
                1 => self.cur_bold = true,
                3 => self.cur_italic = true,
                4 => self.cur_underline = true,
                7 => self.cur_inverse = true,
                22 => self.cur_bold = false,
                23 => self.cur_italic = false,
                24 => self.cur_underline = false,
                27 => self.cur_inverse = false,
                // Standard foreground colors
                30..=37 => self.cur_fg = Color::Ansi((params[i] - 30) as u8),
                38 => {
                    // Extended foreground
                    if let Some(color) = self.parse_extended_color(params, &mut i) {
                        self.cur_fg = color;
                    }
                }
                39 => self.cur_fg = Color::Default,
                // Standard background colors
                40..=47 => self.cur_bg = Color::Ansi((params[i] - 40) as u8),
                48 => {
                    // Extended background
                    if let Some(color) = self.parse_extended_color(params, &mut i) {
                        self.cur_bg = color;
                    }
                }
                49 => self.cur_bg = Color::Default,
                // Bright foreground
                90..=97 => self.cur_fg = Color::Ansi((params[i] - 90 + 8) as u8),
                // Bright background
                100..=107 => self.cur_bg = Color::Ansi((params[i] - 100 + 8) as u8),
                _ => {} // Unknown SGR
            }
            i += 1;
        }
    }

    fn parse_extended_color(&self, params: &[u16], i: &mut usize) -> Option<Color> {
        if *i + 1 >= params.len() {
            return None;
        }
        match params[*i + 1] {
            5 => {
                // 256-color: 38;5;N or 48;5;N
                if *i + 2 < params.len() {
                    *i += 2;
                    Some(Color::Color256(params[*i] as u8))
                } else {
                    *i += 1;
                    None
                }
            }
            2 => {
                // RGB: 38;2;R;G;B or 48;2;R;G;B
                if *i + 4 < params.len() {
                    let r = params[*i + 2] as u8;
                    let g = params[*i + 3] as u8;
                    let b = params[*i + 4] as u8;
                    *i += 4;
                    Some(Color::Rgb(r, g, b))
                } else {
                    *i += 1;
                    None
                }
            }
            _ => {
                *i += 1;
                None
            }
        }
    }

    fn reset_sgr(&mut self) {
        self.cur_fg = Color::Default;
        self.cur_bg = Color::Default;
        self.cur_bold = false;
        self.cur_italic = false;
        self.cur_underline = false;
        self.cur_inverse = false;
    }

    // -- DECSET/DECRST --

    fn decset(&mut self, mode: u16, enable: bool) {
        match mode {
            7 => self.autowrap = enable,
            25 => self.cursor_visible = enable,
            1049 => {
                // Alternate screen buffer
                if enable && !self.alt_active {
                    let size = self.cols as usize * self.rows as usize;
                    let main_cells =
                        std::mem::replace(&mut self.cells, vec![VTermCell::default(); size]);
                    self.alt_cells = Some(main_cells);
                    self.alt_active = true;
                    self.saved_cursor = (self.cursor_row, self.cursor_col);
                    self.cursor_row = 0;
                    self.cursor_col = 0;
                } else if !enable && self.alt_active {
                    if let Some(main) = self.alt_cells.take() {
                        self.cells = main;
                    }
                    self.alt_active = false;
                    self.cursor_row = self.saved_cursor.0.min(self.rows.saturating_sub(1));
                    self.cursor_col = self.saved_cursor.1.min(self.cols.saturating_sub(1));
                }
            }
            _ => {} // Unknown mode
        }
    }

    // -- OSC execution --

    fn execute_osc(&mut self, buf: &[u8]) {
        let text = String::from_utf8_lossy(buf);
        // OSC 0;title or OSC 2;title
        if let Some(rest) = text.strip_prefix("0;").or_else(|| text.strip_prefix("2;")) {
            self.title = rest.to_string();
        }
    }

    // -- Screen operations --

    fn line_feed(&mut self) {
        if self.cursor_row == self.scroll_bot {
            self.scroll_up();
        } else if self.cursor_row < self.rows - 1 {
            self.cursor_row += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.cursor_row == self.scroll_top {
            self.scroll_down();
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
        }
    }

    fn scroll_up(&mut self) {
        let top = self.scroll_top as usize;
        let bot = self.scroll_bot as usize;
        let cols = self.cols as usize;

        // Save top line to scrollback (only for main screen, full scroll region)
        if !self.alt_active && top == 0 {
            let row_start = 0;
            let row_end = cols;
            let line: Vec<VTermCell> = self.cells[row_start..row_end].to_vec();
            self.scrollback.push(line);
            if self.scrollback.len() > self.scrollback_max {
                self.scrollback.remove(0);
            }
        }

        // Shift rows up
        for r in top..bot {
            let dst = r * cols;
            let src = (r + 1) * cols;
            for c in 0..cols {
                self.cells[dst + c] = self.cells[src + c];
            }
        }

        // Clear bottom row
        let bot_start = bot * cols;
        for c in 0..cols {
            self.cells[bot_start + c] = VTermCell::default();
        }

        // New output arrived: snap scrollback view to bottom so the new
        // content (prompt, command output) is immediately visible.
        self.scroll_offset = 0;
    }

    fn scroll_down(&mut self) {
        let top = self.scroll_top as usize;
        let bot = self.scroll_bot as usize;
        let cols = self.cols as usize;

        // Shift rows down
        for r in (top + 1..=bot).rev() {
            let dst = r * cols;
            let src = (r - 1) * cols;
            for c in 0..cols {
                self.cells[dst + c] = self.cells[src + c];
            }
        }

        // Clear top row
        let top_start = top * cols;
        for c in 0..cols {
            self.cells[top_start + c] = VTermCell::default();
        }
    }

    fn erase_display(&mut self, mode: u16) {
        let cols = self.cols as usize;
        match mode {
            0 => {
                // Erase from cursor to end
                let start = self.cursor_row as usize * cols + self.cursor_col as usize;
                for i in start..self.cells.len() {
                    self.cells[i] = VTermCell::default();
                }
            }
            1 => {
                // Erase from start to cursor
                let end = self.cursor_row as usize * cols + self.cursor_col as usize + 1;
                for i in 0..end.min(self.cells.len()) {
                    self.cells[i] = VTermCell::default();
                }
            }
            2 | 3 => {
                // Erase entire display (3 also clears scrollback)
                for cell in &mut self.cells {
                    *cell = VTermCell::default();
                }
                if mode == 3 {
                    self.scrollback.clear();
                }
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let row = self.cursor_row as usize;
        let cols = self.cols as usize;
        let row_start = row * cols;

        match mode {
            0 => {
                // Erase from cursor to end of line
                let start = row_start + self.cursor_col as usize;
                let end = row_start + cols;
                for i in start..end {
                    self.cells[i] = VTermCell::default();
                }
            }
            1 => {
                // Erase from start of line to cursor
                let end = row_start + self.cursor_col as usize + 1;
                for i in row_start..end.min(row_start + cols) {
                    self.cells[i] = VTermCell::default();
                }
            }
            2 => {
                // Erase entire line
                for i in row_start..row_start + cols {
                    self.cells[i] = VTermCell::default();
                }
            }
            _ => {}
        }
    }

    fn insert_lines(&mut self, n: u16) {
        let row = self.cursor_row as usize;
        let bot = self.scroll_bot as usize;
        let cols = self.cols as usize;
        let n = (n as usize).min(bot.saturating_sub(row) + 1);

        // Shift rows down
        for r in (row + n..=bot).rev() {
            let dst = r * cols;
            let src = (r - n) * cols;
            for c in 0..cols {
                self.cells[dst + c] = self.cells[src + c];
            }
        }

        // Clear inserted rows
        for r in row..row + n {
            let start = r * cols;
            for c in 0..cols {
                self.cells[start + c] = VTermCell::default();
            }
        }
    }

    fn delete_lines(&mut self, n: u16) {
        let row = self.cursor_row as usize;
        let bot = self.scroll_bot as usize;
        let cols = self.cols as usize;
        let n = (n as usize).min(bot.saturating_sub(row) + 1);

        // Shift rows up
        for r in row..=bot.saturating_sub(n) {
            let dst = r * cols;
            let src = (r + n) * cols;
            for c in 0..cols {
                self.cells[dst + c] = self.cells[src + c];
            }
        }

        // Clear bottom rows
        for r in (bot + 1 - n)..=bot {
            let start = r * cols;
            for c in 0..cols {
                self.cells[start + c] = VTermCell::default();
            }
        }
    }

    fn delete_chars(&mut self, n: u16) {
        let row = self.cursor_row as usize;
        let col = self.cursor_col as usize;
        let cols = self.cols as usize;
        let n = (n as usize).min(cols.saturating_sub(col));
        let row_start = row * cols;

        for c in col..cols.saturating_sub(n) {
            self.cells[row_start + c] = self.cells[row_start + c + n];
        }
        for c in cols.saturating_sub(n)..cols {
            self.cells[row_start + c] = VTermCell::default();
        }
    }

    fn insert_chars(&mut self, n: u16) {
        let row = self.cursor_row as usize;
        let col = self.cursor_col as usize;
        let cols = self.cols as usize;
        let n = (n as usize).min(cols.saturating_sub(col));
        let row_start = row * cols;

        for c in (col + n..cols).rev() {
            self.cells[row_start + c] = self.cells[row_start + c - n];
        }
        for c in col..col + n {
            self.cells[row_start + c] = VTermCell::default();
        }
    }

    fn erase_chars(&mut self, n: u16) {
        let row = self.cursor_row as usize;
        let col = self.cursor_col as usize;
        let cols = self.cols as usize;
        let n = (n as usize).min(cols.saturating_sub(col));
        let row_start = row * cols;

        for c in col..col + n {
            self.cells[row_start + c] = VTermCell::default();
        }
    }

    fn full_reset(&mut self) {
        let size = self.cols as usize * self.rows as usize;
        self.cells = vec![VTermCell::default(); size];
        self.alt_cells = None;
        self.alt_active = false;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.saved_cursor = (0, 0);
        self.reset_sgr();
        self.scroll_top = 0;
        self.scroll_bot = self.rows.saturating_sub(1);
        self.autowrap = true;
        self.pending_wrap = false;
        self.state = ParseState::Normal;
        self.cursor_visible = true;
        self.title.clear();
    }

    // -- Helpers --

    fn cell_idx(&self, row: u16, col: u16) -> usize {
        row as usize * self.cols as usize + col as usize
    }

    fn parse_params(&self, buf: &[u8]) -> Vec<u16> {
        if buf.is_empty() {
            return Vec::new();
        }
        let s = String::from_utf8_lossy(buf);
        s.split(';')
            .map(|part| part.parse::<u16>().unwrap_or(0))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_vterm() {
        let vt = VTerm::new(80, 24);
        assert_eq!(vt.cols(), 80);
        assert_eq!(vt.rows(), 24);
        assert_eq!(vt.cursor_pos(), (0, 0));
        assert_eq!(vt.cells().len(), 80 * 24);
    }

    #[test]
    fn test_printable_char() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"A");
        assert_eq!(vt.cells()[0].ch, 'A');
        assert_eq!(vt.cursor_pos(), (0, 1));
    }

    #[test]
    fn test_line_feed() {
        let mut vt = VTerm::new(80, 24);
        // LF doesn't reset column (unlike CR+LF)
        vt.feed(b"A\nB");
        assert_eq!(vt.cursor_pos(), (1, 2));
        assert_eq!(vt.cells()[81].ch, 'B'); // row 1, col 1
    }

    #[test]
    fn test_carriage_return() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"ABC\r");
        assert_eq!(vt.cursor_pos(), (0, 0));
    }

    #[test]
    fn test_cursor_movement() {
        let mut vt = VTerm::new(80, 24);
        // CUP(5,10)
        vt.feed(b"\x1b[5;10H");
        assert_eq!(vt.cursor_pos(), (4, 9));
    }

    #[test]
    fn test_erase_display() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"Hello");
        vt.feed(b"\x1b[2J");
        assert_eq!(vt.cells()[0].ch, ' ');
    }

    #[test]
    fn test_sgr_bold() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"\x1b[1mX");
        assert!(vt.cells()[0].bold);
    }

    #[test]
    fn test_sgr_color() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"\x1b[31mR");
        assert_eq!(vt.cells()[0].fg, Color::Ansi(1));
    }

    #[test]
    fn test_sgr_256color() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"\x1b[38;5;196mR");
        assert_eq!(vt.cells()[0].fg, Color::Color256(196));
    }

    #[test]
    fn test_sgr_rgb() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"\x1b[38;2;255;128;0mO");
        assert_eq!(vt.cells()[0].fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn test_sgr_reset() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"\x1b[1;31mA\x1b[0mB");
        assert!(vt.cells()[0].bold);
        assert!(!vt.cells()[1].bold);
        assert_eq!(vt.cells()[1].fg, Color::Default);
    }

    #[test]
    fn test_alt_screen() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"Main");
        vt.feed(b"\x1b[?1049h"); // enter alt
        assert!(vt.alt_active);
        assert_eq!(vt.cells()[0].ch, ' '); // alt screen is blank
        vt.feed(b"Alt");
        vt.feed(b"\x1b[?1049l"); // leave alt
        assert!(!vt.alt_active);
        assert_eq!(vt.cells()[0].ch, 'M'); // main restored
    }

    #[test]
    fn test_scroll_region() {
        let mut vt = VTerm::new(80, 5);
        // Set scroll region to rows 2-4 (1-based)
        vt.feed(b"\x1b[2;4r");
        assert_eq!(vt.scroll_top, 1);
        assert_eq!(vt.scroll_bot, 3);
    }

    #[test]
    fn test_dsr_response() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"\x1b[3;5H"); // Move to row 3, col 5
        vt.feed(b"\x1b[6n"); // Request cursor position
        let responses = vt.take_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0], b"\x1b[3;5R");
    }

    #[test]
    fn test_erase_line() {
        let mut vt = VTerm::new(10, 1);
        vt.feed(b"ABCDEFGHIJ");
        vt.feed(b"\x1b[1;5H"); // col 5
        vt.feed(b"\x1b[0K"); // erase from cursor to end
        assert_eq!(vt.cells()[0].ch, 'A');
        assert_eq!(vt.cells()[4].ch, ' '); // erased
    }

    #[test]
    fn test_osc_title() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"\x1b]0;My Title\x07");
        assert_eq!(vt.title, "My Title");
    }

    #[test]
    fn test_autowrap() {
        let mut vt = VTerm::new(5, 3);
        vt.feed(b"ABCDE");
        assert_eq!(vt.cursor_pos(), (0, 4)); // pending_wrap set
        vt.feed(b"F");
        assert_eq!(vt.cursor_pos(), (1, 1)); // wrapped
        assert_eq!(vt.cells()[5].ch, 'F'); // row 1, col 0
    }

    #[test]
    fn test_resize() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"Hello");
        vt.resize(40, 12);
        assert_eq!(vt.cols(), 40);
        assert_eq!(vt.rows(), 12);
        // Shrinking 24→12 pushes the top 12 rows to scrollback so the bottom
        // (cursor) region is preserved.  "Hello" was on row 0, which is now
        // the first row in the scrollback buffer.
        assert_eq!(vt.scrollback()[0][0].ch, 'H');
    }

    #[test]
    fn test_backspace() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"AB\x08");
        assert_eq!(vt.cursor_pos(), (0, 1));
    }

    #[test]
    fn test_tab() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"\t");
        assert_eq!(vt.cursor_pos(), (0, 8));
    }

    #[test]
    fn test_insert_delete_chars() {
        let mut vt = VTerm::new(10, 1);
        vt.feed(b"ABCDE");
        vt.feed(b"\x1b[1;3H"); // col 3
        vt.feed(b"\x1b[2P"); // delete 2 chars
        assert_eq!(vt.cells()[2].ch, 'E');
        assert_eq!(vt.cells()[3].ch, ' ');
    }

    #[test]
    fn test_cursor_save_restore() {
        let mut vt = VTerm::new(80, 24);
        vt.feed(b"\x1b[5;10H"); // move
        vt.feed(b"\x1b7"); // save
        vt.feed(b"\x1b[1;1H"); // move home
        vt.feed(b"\x1b8"); // restore
        assert_eq!(vt.cursor_pos(), (4, 9));
    }
}
