use crate::terminal::{self, ColorMode};

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    Default,
    Ansi(u8),
    Color256(u8),
    Rgb(u8, u8, u8),
}

// ---------------------------------------------------------------------------
// Cell
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub underline: bool,
    pub inverse: bool,
    pub italic: bool,
    /// True for the second column of a double-width character.
    pub wide_cont: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            underline: false,
            inverse: false,
            italic: false,
            wide_cont: false,
        }
    }
}

/// Style attributes for a cell (used by put_cell_styled).
#[derive(Clone, Copy, Debug)]
pub struct CellStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub underline: bool,
    pub inverse: bool,
    pub italic: bool,
}

// ---------------------------------------------------------------------------
// Screen — flat Vec<Cell> with double-buffer swap
// ---------------------------------------------------------------------------

pub struct Screen {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
    prev_cells: Vec<Cell>,
    first_frame: bool,
}

impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        let size = width * height;
        Self {
            width,
            height,
            cells: vec![Cell::default(); size],
            prev_cells: vec![Cell::default(); size],
            first_frame: true,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    #[inline]
    fn idx(&self, row: usize, col: usize) -> usize {
        row * self.width + col
    }

    // -- Building frames ---------------------------------------------------

    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            *cell = Cell::default();
        }
    }

    pub fn put_cell(&mut self, row: usize, col: usize, cell: Cell) {
        if row < self.height && col < self.width {
            let i = self.idx(row, col);
            self.cells[i] = cell;
        }
    }

    pub fn put_char(&mut self, row: usize, col: usize, ch: char, fg: Color, bg: Color, bold: bool) {
        let w = crate::unicode::char_width(ch);
        if w == 2 {
            if col + 1 < self.width {
                self.put_cell(
                    row,
                    col,
                    Cell {
                        ch,
                        fg,
                        bg,
                        bold,
                        underline: false,
                        inverse: false,
                        italic: false,
                        wide_cont: false,
                    },
                );
                self.put_cell(
                    row,
                    col + 1,
                    Cell {
                        ch: ' ',
                        fg,
                        bg,
                        bold,
                        underline: false,
                        inverse: false,
                        italic: false,
                        wide_cont: true,
                    },
                );
            } else {
                // Wide char doesn't fit at right edge — put a space instead
                self.put_cell(
                    row,
                    col,
                    Cell {
                        ch: ' ',
                        fg,
                        bg,
                        bold,
                        underline: false,
                        inverse: false,
                        italic: false,
                        wide_cont: false,
                    },
                );
            }
        } else {
            self.put_cell(
                row,
                col,
                Cell {
                    ch,
                    fg,
                    bg,
                    bold,
                    underline: false,
                    inverse: false,
                    italic: false,
                    wide_cont: false,
                },
            );
        }
    }

    pub fn put_cell_styled(&mut self, row: usize, col: usize, ch: char, style: CellStyle) {
        let w = crate::unicode::char_width(ch);
        if w == 2 {
            if col + 1 < self.width {
                self.put_cell(
                    row,
                    col,
                    Cell {
                        ch,
                        fg: style.fg,
                        bg: style.bg,
                        bold: style.bold,
                        underline: style.underline,
                        inverse: style.inverse,
                        italic: style.italic,
                        wide_cont: false,
                    },
                );
                self.put_cell(
                    row,
                    col + 1,
                    Cell {
                        ch: ' ',
                        fg: style.fg,
                        bg: style.bg,
                        bold: style.bold,
                        underline: style.underline,
                        inverse: style.inverse,
                        italic: style.italic,
                        wide_cont: true,
                    },
                );
            } else {
                self.put_cell(
                    row,
                    col,
                    Cell {
                        ch: ' ',
                        fg: style.fg,
                        bg: style.bg,
                        bold: style.bold,
                        underline: style.underline,
                        inverse: style.inverse,
                        italic: style.italic,
                        wide_cont: false,
                    },
                );
            }
        } else {
            self.put_cell(
                row,
                col,
                Cell {
                    ch,
                    fg: style.fg,
                    bg: style.bg,
                    bold: style.bold,
                    underline: style.underline,
                    inverse: style.inverse,
                    italic: style.italic,
                    wide_cont: false,
                },
            );
        }
    }

    pub fn put_str(
        &mut self,
        row: usize,
        col: usize,
        text: &str,
        fg: Color,
        bg: Color,
        bold: bool,
    ) {
        if row >= self.height {
            return;
        }
        let mut c = col;
        for ch in text.chars() {
            if c >= self.width {
                break;
            }
            self.put_char(row, c, ch, fg, bg, bold);
            c += crate::unicode::char_width(ch).max(1);
        }
    }

    // -- Rendering ---------------------------------------------------------

    pub fn flush(&mut self, color_mode: &ColorMode) {
        let buf = self.build_diff_output(color_mode);
        if !buf.is_empty() {
            terminal::hide_cursor();
            terminal::write_all(&buf);
            terminal::show_cursor();
            terminal::flush();
        }
        self.first_frame = false;
        // Zero-alloc swap: prev gets current, then clear current for next frame
        std::mem::swap(&mut self.cells, &mut self.prev_cells);
        self.clear();
    }

    // -- Resize ------------------------------------------------------------

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        let size = width * height;
        self.cells = vec![Cell::default(); size];
        self.prev_cells = vec![Cell::default(); size];
        self.first_frame = true; // force full redraw
    }

    // -- Internal ----------------------------------------------------------

    fn build_diff_output(&self, color_mode: &ColorMode) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4096);
        let mut cur_fg = Color::Default;
        let mut cur_bg = Color::Default;
        let mut cur_bold = false;
        let mut cur_underline = false;
        let mut cur_inverse = false;
        let mut cur_italic = false;
        let full_redraw = self.first_frame;

        for row in 0..self.height {
            for col in 0..self.width {
                let i = self.idx(row, col);
                let cell = &self.cells[i];

                // Skip continuation cells — the terminal advances 2 cols for wide chars
                if cell.wide_cont {
                    continue;
                }

                let changed = if full_redraw {
                    true
                } else {
                    self.prev_cells[i] != *cell
                };
                if !changed {
                    continue;
                }

                // Position cursor (1-based)
                write_cursor_pos(&mut buf, row, col);

                // Apply style changes
                if cell.bold != cur_bold {
                    if cell.bold {
                        buf.extend_from_slice(b"\x1b[1m");
                    } else {
                        buf.extend_from_slice(b"\x1b[22m");
                    }
                    cur_bold = cell.bold;
                }
                if cell.italic != cur_italic {
                    if cell.italic {
                        buf.extend_from_slice(b"\x1b[3m");
                    } else {
                        buf.extend_from_slice(b"\x1b[23m");
                    }
                    cur_italic = cell.italic;
                }
                if cell.underline != cur_underline {
                    if cell.underline {
                        buf.extend_from_slice(b"\x1b[4m");
                    } else {
                        buf.extend_from_slice(b"\x1b[24m");
                    }
                    cur_underline = cell.underline;
                }
                if cell.inverse != cur_inverse {
                    if cell.inverse {
                        buf.extend_from_slice(b"\x1b[7m");
                    } else {
                        buf.extend_from_slice(b"\x1b[27m");
                    }
                    cur_inverse = cell.inverse;
                }
                if cell.fg != cur_fg {
                    write_fg_color(&mut buf, cell.fg, color_mode);
                    cur_fg = cell.fg;
                }
                if cell.bg != cur_bg {
                    write_bg_color(&mut buf, cell.bg, color_mode);
                    cur_bg = cell.bg;
                }

                // Write character
                write_char(&mut buf, cell.ch);
            }
        }

        // Reset attributes if we emitted anything
        if !buf.is_empty() {
            buf.extend_from_slice(b"\x1b[0m");
        }

        buf
    }

    // -- Test helpers (cfg(test) only) -------------------------------------

    #[cfg(test)]
    fn cell_at(&self, row: usize, col: usize) -> &Cell {
        &self.cells[self.idx(row, col)]
    }
}

// ---------------------------------------------------------------------------
// ANSI output helpers
// ---------------------------------------------------------------------------

fn write_cursor_pos(buf: &mut Vec<u8>, row: usize, col: usize) {
    // CSI row;col H  (1-based)
    buf.extend_from_slice(b"\x1b[");
    write_usize(buf, row + 1);
    buf.push(b';');
    write_usize(buf, col + 1);
    buf.push(b'H');
}

fn write_usize(buf: &mut Vec<u8>, n: usize) {
    if n == 0 {
        buf.push(b'0');
        return;
    }
    let start = buf.len();
    let mut v = n;
    while v > 0 {
        buf.push(b'0' + (v % 10) as u8);
        v /= 10;
    }
    buf[start..].reverse();
}

fn write_char(buf: &mut Vec<u8>, ch: char) {
    let mut tmp = [0u8; 4];
    buf.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
}

fn write_fg_color(buf: &mut Vec<u8>, color: Color, mode: &ColorMode) {
    match effective_color(color, mode) {
        Color::Default => buf.extend_from_slice(b"\x1b[39m"),
        Color::Ansi(n) => {
            let code = if n < 8 { 30 + n } else { 90 + n - 8 };
            buf.extend_from_slice(b"\x1b[");
            write_usize(buf, code as usize);
            buf.push(b'm');
        }
        Color::Color256(n) => {
            buf.extend_from_slice(b"\x1b[38;5;");
            write_usize(buf, n as usize);
            buf.push(b'm');
        }
        Color::Rgb(r, g, b) => {
            buf.extend_from_slice(b"\x1b[38;2;");
            write_usize(buf, r as usize);
            buf.push(b';');
            write_usize(buf, g as usize);
            buf.push(b';');
            write_usize(buf, b as usize);
            buf.push(b'm');
        }
    }
}

fn write_bg_color(buf: &mut Vec<u8>, color: Color, mode: &ColorMode) {
    match effective_color(color, mode) {
        Color::Default => buf.extend_from_slice(b"\x1b[49m"),
        Color::Ansi(n) => {
            let code = if n < 8 { 40 + n } else { 100 + n - 8 };
            buf.extend_from_slice(b"\x1b[");
            write_usize(buf, code as usize);
            buf.push(b'm');
        }
        Color::Color256(n) => {
            buf.extend_from_slice(b"\x1b[48;5;");
            write_usize(buf, n as usize);
            buf.push(b'm');
        }
        Color::Rgb(r, g, b) => {
            buf.extend_from_slice(b"\x1b[48;2;");
            write_usize(buf, r as usize);
            buf.push(b';');
            write_usize(buf, g as usize);
            buf.push(b';');
            write_usize(buf, b as usize);
            buf.push(b'm');
        }
    }
}

// ---------------------------------------------------------------------------
// Color downgrade
// ---------------------------------------------------------------------------

fn effective_color(color: Color, mode: &ColorMode) -> Color {
    match (color, mode) {
        (Color::Rgb(r, g, b), ColorMode::Color256) => Color::Color256(rgb_to_ansi256(r, g, b)),
        // Use perceptual OKLab distance for all 24-bit → 16-color downsampling.
        (Color::Rgb(r, g, b), ColorMode::Color16) => Color::Ansi(rgb_to_ansi16_oklab(r, g, b)),
        (Color::Color256(n), ColorMode::Color16) => {
            let (r, g, b) = ansi256_to_rgb(n);
            Color::Ansi(rgb_to_ansi16_oklab(r, g, b))
        }
        _ => color,
    }
}

/// Map a 24-bit RGB color to the nearest ANSI 16-color index using OKLab
/// perceptual distance.
///
/// This replaces the old rec601-luma heuristic, which caused hue-shifting when
/// saturated colors (e.g. `#0000FF`) collapsed into the same dark bucket as
/// neutral grays because luma weights are tuned for video signal levels, not
/// human color perception.
fn rgb_to_ansi16_oklab(r: u8, g: u8, b: u8) -> u8 {
    // Representative sRGB values for the 16 standard ANSI colors.
    // These must match the `ANSI_BASIC` table in `ansi256_to_rgb` so that
    // Color256(0..15) round-trips correctly.
    const ANSI16: [(u8, u8, u8); 16] = [
        (0, 0, 0),       // 0  black
        (128, 0, 0),     // 1  red
        (0, 128, 0),     // 2  green
        (128, 128, 0),   // 3  yellow
        (0, 0, 128),     // 4  blue
        (128, 0, 128),   // 5  magenta
        (0, 128, 128),   // 6  cyan
        (192, 192, 192), // 7  white
        (128, 128, 128), // 8  bright black (gray)
        (255, 0, 0),     // 9  bright red
        (0, 255, 0),     // 10 bright green
        (255, 255, 0),   // 11 bright yellow
        (0, 0, 255),     // 12 bright blue
        (255, 0, 255),   // 13 bright magenta
        (0, 255, 255),   // 14 bright cyan
        (255, 255, 255), // 15 bright white
    ];

    let mut best_idx = 0u8;
    let mut best_dist = f32::MAX;
    for (i, &(pr, pg, pb)) in ANSI16.iter().enumerate() {
        let dist = crate::oklab::perceptual_distance(r, g, b, pr, pg, pb);
        if dist < best_dist {
            best_dist = dist;
            best_idx = i as u8;
        }
    }
    best_idx
}

/// Convert an RGB color to the nearest xterm-256 palette index.
///
/// The xterm-256 palette is:
///   0-7:     standard ANSI colors
///   8-15:    bright ANSI colors
///   16-231:  6x6x6 color cube
///   232-255: 24-step grayscale ramp
pub fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    // Check grayscale first
    if r == g && g == b {
        if r < 8 {
            return 16; // closest cube entry (black)
        }
        if r > 248 {
            return 231; // closest cube entry (white)
        }
        // Map to grayscale ramp 232-255 (values 8, 18, 28, ..., 238)
        return (((r as u16 - 8) * 24 / 240) as u8) + 232;
    }

    // Map to 6x6x6 color cube (indices 16-231)
    let ri = color_cube_index(r);
    let gi = color_cube_index(g);
    let bi = color_cube_index(b);
    16 + 36 * ri + 6 * gi + bi
}

fn color_cube_index(v: u8) -> u8 {
    // The 6 cube values are 0, 95, 135, 175, 215, 255
    // Midpoints: 48, 115, 155, 195, 235
    if v < 48 {
        0
    } else if v < 115 {
        1
    } else if v < 155 {
        2
    } else if v < 195 {
        3
    } else if v < 235 {
        4
    } else {
        5
    }
}


fn ansi256_to_rgb(n: u8) -> (u8, u8, u8) {
    static ANSI_BASIC: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];

    match n {
        0..=15 => ANSI_BASIC[n as usize],
        16..=231 => {
            let idx = n - 16;
            let ri = idx / 36;
            let gi = (idx % 36) / 6;
            let bi = idx % 6;
            let cube = |i: u8| -> u8 { if i == 0 { 0 } else { 55 + 40 * i } };
            (cube(ri), cube(gi), cube(bi))
        }
        _ => {
            // Grayscale ramp 232-255: 8, 18, 28, ..., 238
            let v = 8 + 10 * (n - 232);
            (v, v, v)
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
    fn new_screen_dimensions() {
        let s = Screen::new(80, 24);
        assert_eq!(s.width(), 80);
        assert_eq!(s.height(), 24);
        assert_eq!(s.cells.len(), 80 * 24);
    }

    #[test]
    fn new_screen_default_cells() {
        let s = Screen::new(3, 2);
        for cell in &s.cells {
            assert_eq!(*cell, Cell::default());
        }
    }

    #[test]
    fn put_char_populates_cell() {
        let mut s = Screen::new(10, 5);
        s.put_char(2, 3, 'A', Color::Rgb(255, 0, 0), Color::Default, true);
        assert_eq!(s.cell_at(2, 3).ch, 'A');
        assert_eq!(s.cell_at(2, 3).fg, Color::Rgb(255, 0, 0));
        assert_eq!(s.cell_at(2, 3).bold, true);
    }

    #[test]
    fn put_char_out_of_bounds() {
        let mut s = Screen::new(5, 5);
        // Should not panic
        s.put_char(10, 10, 'X', Color::Default, Color::Default, false);
    }

    #[test]
    fn put_str_populates_cells() {
        let mut s = Screen::new(10, 5);
        s.put_str(0, 0, "Hi!", Color::Default, Color::Default, false);
        assert_eq!(s.cell_at(0, 0).ch, 'H');
        assert_eq!(s.cell_at(0, 1).ch, 'i');
        assert_eq!(s.cell_at(0, 2).ch, '!');
        assert_eq!(s.cell_at(0, 3).ch, ' '); // untouched
    }

    #[test]
    fn put_str_truncates_at_edge() {
        let mut s = Screen::new(5, 1);
        s.put_str(0, 3, "Hello", Color::Default, Color::Default, false);
        assert_eq!(s.cell_at(0, 3).ch, 'H');
        assert_eq!(s.cell_at(0, 4).ch, 'e');
        // "llo" should be truncated
    }

    #[test]
    fn put_str_row_out_of_bounds() {
        let mut s = Screen::new(5, 2);
        s.put_str(5, 0, "nope", Color::Default, Color::Default, false);
        // Should not panic
    }

    #[test]
    fn clear_resets_cells() {
        let mut s = Screen::new(5, 3);
        s.put_char(1, 2, 'Z', Color::Ansi(1), Color::Ansi(2), true);
        s.clear();
        assert_eq!(*s.cell_at(1, 2), Cell::default());
    }

    #[test]
    fn resize_changes_dimensions() {
        let mut s = Screen::new(10, 5);
        s.resize(20, 10);
        assert_eq!(s.width(), 20);
        assert_eq!(s.height(), 10);
        assert_eq!(s.cells.len(), 20 * 10);
    }

    #[test]
    fn unchanged_screen_empty_diff() {
        let mut s = Screen::new(5, 3);
        // First flush: full draw (first_frame = true)
        let first = s.build_diff_output(&ColorMode::TrueColor);
        assert!(!first.is_empty());

        // Simulate flush: swap buffers
        s.first_frame = false;
        std::mem::swap(&mut s.cells, &mut s.prev_cells);
        s.clear();

        // Second flush with identical content: no diff
        let second = s.build_diff_output(&ColorMode::TrueColor);
        assert!(second.is_empty());
    }

    #[test]
    fn rgb_to_ansi256_black() {
        assert_eq!(rgb_to_ansi256(0, 0, 0), 16);
    }

    #[test]
    fn rgb_to_ansi256_white() {
        assert_eq!(rgb_to_ansi256(255, 255, 255), 231);
    }

    #[test]
    fn rgb_to_ansi256_gray() {
        let idx = rgb_to_ansi256(128, 128, 128);
        assert!((232..=255).contains(&idx));
    }

    #[test]
    fn rgb_to_ansi256_red() {
        let idx = rgb_to_ansi256(255, 0, 0);
        // Should map to the red region of the color cube
        // 255 → cube index 5, 0 → 0, 0 → 0 → 16 + 5*36 = 196
        assert_eq!(idx, 196);
    }

    #[test]
    fn rgb_to_ansi16_oklab_black() {
        // Pure black should map to ANSI color index 0 (black)
        assert_eq!(rgb_to_ansi16_oklab(0, 0, 0), 0);
    }

    #[test]
    fn rgb_to_ansi16_oklab_red() {
        // Bright red should map to red (1) or bright red (9)
        let n = rgb_to_ansi16_oklab(255, 0, 0);
        assert!(n == 1 || n == 9, "expected red index, got {n}");
    }

    #[test]
    fn cell_default_equality() {
        let a = Cell::default();
        let b = Cell {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            underline: false,
            inverse: false,
            italic: false,
            wide_cont: false,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn color_downgrade_rgb_to_256() {
        let c = effective_color(Color::Rgb(255, 0, 0), &ColorMode::Color256);
        assert_eq!(c, Color::Color256(196));
    }

    #[test]
    fn color_downgrade_rgb_to_16() {
        let c = effective_color(Color::Rgb(255, 0, 0), &ColorMode::Color16);
        if let Color::Ansi(n) = c {
            assert!(n <= 15);
        } else {
            panic!("Expected Ansi color");
        }
    }

    #[test]
    fn color_no_downgrade_in_truecolor() {
        let c = effective_color(Color::Rgb(42, 100, 200), &ColorMode::TrueColor);
        assert_eq!(c, Color::Rgb(42, 100, 200));
    }

    #[test]
    fn write_usize_zero() {
        let mut buf = Vec::new();
        write_usize(&mut buf, 0);
        assert_eq!(buf, b"0");
    }

    #[test]
    fn write_usize_multidigit() {
        let mut buf = Vec::new();
        write_usize(&mut buf, 123);
        assert_eq!(buf, b"123");
    }

    #[test]
    fn double_buffer_swap_no_alloc() {
        let mut s = Screen::new(10, 5);
        s.put_char(0, 0, 'X', Color::Default, Color::Default, false);

        // Simulate flush
        s.first_frame = false;
        let ptr_before = s.prev_cells.as_ptr();
        std::mem::swap(&mut s.cells, &mut s.prev_cells);
        let ptr_after = s.cells.as_ptr();

        // The old prev_cells buffer is now cells (same pointer, no realloc)
        assert_eq!(ptr_before, ptr_after);
    }

    #[test]
    fn flat_grid_indexing() {
        let mut s = Screen::new(10, 5);
        s.put_char(3, 7, 'Q', Color::Ansi(1), Color::Default, false);
        // Verify flat indexing: row 3, col 7 = index 3*10+7 = 37
        assert_eq!(s.cells[37].ch, 'Q');
        assert_eq!(s.cell_at(3, 7).ch, 'Q');
    }
}
