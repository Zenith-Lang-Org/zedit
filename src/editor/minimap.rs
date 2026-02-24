// ---------------------------------------------------------------------------
// Minimap — zoomed-out braille-encoded code overview sidebar
// ---------------------------------------------------------------------------

use crate::render::Color;

// ---------------------------------------------------------------------------
// Minimap state
// ---------------------------------------------------------------------------

/// Width of the minimap overlay in terminal columns.
pub const MINIMAP_WIDTH: usize = 10;

pub(super) struct Minimap {
    pub visible: bool,
}

impl Minimap {
    pub fn new() -> Self {
        Self { visible: false }
    }
}

// ---------------------------------------------------------------------------
// Braille encoding
// ---------------------------------------------------------------------------

/// Encode a 2-column × 4-row pixel grid into a Unicode braille character.
///
/// `dots[row][col]` — true = dot present, false = blank.
/// The braille bit layout (U+2800 base):
///   col 0 col 1
///   bit0  bit3   (row 0)
///   bit1  bit4   (row 1)
///   bit2  bit5   (row 2)
///   bit6  bit7   (row 3)
pub(super) fn encode_braille(dots: [[bool; 2]; 4]) -> char {
    let mut code: u32 = 0x2800;
    if dots[0][0] {
        code |= 0x01;
    }
    if dots[1][0] {
        code |= 0x02;
    }
    if dots[2][0] {
        code |= 0x04;
    }
    if dots[0][1] {
        code |= 0x08;
    }
    if dots[1][1] {
        code |= 0x10;
    }
    if dots[2][1] {
        code |= 0x20;
    }
    if dots[3][0] {
        code |= 0x40;
    }
    if dots[3][1] {
        code |= 0x80;
    }
    char::from_u32(code).unwrap_or('\u{2800}')
}

// ---------------------------------------------------------------------------
// MinimapCell — one rendered cell
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(super) struct MinimapCell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
}

// ---------------------------------------------------------------------------
// MinimapLine — one row of minimap cells (MINIMAP_WIDTH cells wide)
// ---------------------------------------------------------------------------

pub(super) struct MinimapLine {
    pub cells: Vec<MinimapCell>,
    #[allow(dead_code)]
    pub in_viewport: bool,
}

// ---------------------------------------------------------------------------
// Minimap render data builder
// ---------------------------------------------------------------------------

/// Build the minimap render data for a buffer.
///
/// Arguments:
/// - `line_count`: total lines in the buffer
/// - `get_line`: closure returning the text of source line `i`
/// - `scroll_row`: first visible source line in the editor pane
/// - `visible_rows`: number of visible source rows in the pane
/// - `minimap_rows`: available terminal rows for the minimap
pub(super) fn build_minimap<F>(
    line_count: usize,
    get_line: F,
    scroll_row: usize,
    visible_rows: usize,
    minimap_rows: usize,
) -> Vec<MinimapLine>
where
    F: Fn(usize) -> String,
{
    if minimap_rows == 0 || line_count == 0 {
        return Vec::new();
    }

    // How many source lines does each minimap row represent?
    // Each braille char covers 4 "sub-rows", so each terminal row = 4 sub-slots.
    // We want to fit the whole file: total_sub_slots = minimap_rows * 4
    let total_sub_slots = minimap_rows * 4;
    // lines_per_sub = how many source lines one braille sub-row covers
    let lines_per_sub = (line_count + total_sub_slots - 1) / total_sub_slots;
    let lines_per_sub = lines_per_sub.max(1);

    let viewport_end = scroll_row + visible_rows;

    let bg_normal = Color::Color256(236);
    let bg_viewport = Color::Color256(240);
    let fg_text = Color::Color256(250); // light gray for non-whitespace dots
    let fg_empty = Color::Color256(237); // very dark for empty/whitespace

    let mut result = Vec::with_capacity(minimap_rows);

    for r in 0..minimap_rows {
        // Source line range for this minimap row
        let src_start = r * 4 * lines_per_sub;
        if src_start >= line_count {
            // Past end of file — blank row
            result.push(MinimapLine {
                cells: vec![
                    MinimapCell {
                        ch: '\u{2800}',
                        fg: fg_empty,
                        bg: bg_normal,
                    };
                    MINIMAP_WIDTH
                ],
                in_viewport: false,
            });
            continue;
        }

        // Is this row in the viewport?
        let row_src_end = (src_start + 4 * lines_per_sub).min(line_count);
        let in_vp = src_start < viewport_end && row_src_end > scroll_row;

        let bg = if in_vp { bg_viewport } else { bg_normal };

        // Pre-fetch the 4 representative source lines for this minimap row
        let mut src_lines: [String; 4] = Default::default();
        for sr in 0..4usize {
            let line_idx = (src_start + sr * lines_per_sub).min(line_count.saturating_sub(1));
            if src_start + sr * lines_per_sub < line_count {
                src_lines[sr] = get_line(line_idx);
            }
        }

        // Build MINIMAP_WIDTH braille chars for this row
        let mut cells = Vec::with_capacity(MINIMAP_WIDTH);
        let mut has_any_dot = false;

        for c in 0..MINIMAP_WIDTH {
            let char_base = c * 2; // source char offset for this minimap col

            let mut dots = [[false; 2]; 4];
            for sr in 0..4usize {
                let line = &src_lines[sr];
                // Left dot: char at char_base
                dots[sr][0] = line
                    .chars()
                    .nth(char_base)
                    .map(|ch| !ch.is_whitespace())
                    .unwrap_or(false);
                // Right dot: char at char_base + 1
                dots[sr][1] = line
                    .chars()
                    .nth(char_base + 1)
                    .map(|ch| !ch.is_whitespace())
                    .unwrap_or(false);
            }

            let any_dot = dots.iter().any(|row| row[0] || row[1]);
            has_any_dot |= any_dot;

            cells.push(MinimapCell {
                ch: encode_braille(dots),
                fg: if any_dot { fg_text } else { fg_empty },
                bg,
            });
        }

        // If no dots at all (blank line block), use a subtle empty braille
        if !has_any_dot {
            for cell in &mut cells {
                cell.ch = '\u{2800}'; // blank braille
            }
        }

        result.push(MinimapLine {
            cells,
            in_viewport: in_vp,
        });
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_braille_empty() {
        let dots = [[false; 2]; 4];
        assert_eq!(encode_braille(dots), '\u{2800}');
    }

    #[test]
    fn test_encode_braille_full() {
        let dots = [[true; 2]; 4];
        // All 8 bits set: 0x2800 | 0xFF = 0x28FF
        assert_eq!(encode_braille(dots) as u32, 0x28FF);
    }

    #[test]
    fn test_encode_braille_top_left() {
        let mut dots = [[false; 2]; 4];
        dots[0][0] = true; // bit 0
        assert_eq!(encode_braille(dots) as u32, 0x2801);
    }

    #[test]
    fn test_encode_braille_bottom_right() {
        let mut dots = [[false; 2]; 4];
        dots[3][1] = true; // bit 7
        assert_eq!(encode_braille(dots) as u32, 0x2880);
    }

    #[test]
    fn test_build_minimap_empty_buffer() {
        let lines = build_minimap(0, |_| String::new(), 0, 10, 20);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_build_minimap_basic() {
        let src = vec![
            "fn main() {".to_string(),
            "    let x = 1;".to_string(),
            "}".to_string(),
        ];
        let lines = build_minimap(src.len(), |i| src[i].clone(), 0, 3, 5);
        assert!(!lines.is_empty());
        assert_eq!(lines[0].cells.len(), MINIMAP_WIDTH);
        // First row should be in viewport (scroll_row=0, visible=3)
        assert!(lines[0].in_viewport);
    }

    #[test]
    fn test_build_minimap_viewport_highlight() {
        let src: Vec<String> = (0..40).map(|i| format!("line {}", i)).collect();
        // scroll_row=20, visible=10 → viewport is lines 20..30
        let lines = build_minimap(src.len(), |i| src[i].clone(), 20, 10, 40);
        // Row 0 covers lines 0..4 (approx), should not be in viewport
        assert!(!lines[0].in_viewport);
    }

    #[test]
    fn test_build_minimap_past_eof() {
        let src = vec!["hello".to_string()];
        let lines = build_minimap(src.len(), |i| src[i].clone(), 0, 1, 20);
        // Rows past line count should be blank braille
        for line in lines.iter().skip(1) {
            assert!(line.cells.iter().all(|c| c.ch == '\u{2800}'));
        }
    }
}
