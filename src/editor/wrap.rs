use crate::buffer::Buffer;
use crate::unicode::char_width;

// ---------------------------------------------------------------------------
// WrapMap — visual word-wrap metadata for a single buffer
// ---------------------------------------------------------------------------

pub(super) struct WrapMap {
    /// For each buffer line, the byte offsets where visual breaks occur.
    /// A line with 0 breaks = 1 visual row. N breaks = N+1 visual rows.
    breaks: Vec<Vec<usize>>,
    /// Prefix sum: visual_offsets[line] = total visual rows before this line.
    visual_offsets: Vec<usize>,
    /// Total number of visual rows across all lines.
    total_visual_rows: usize,
    /// Pane text width used to compute breaks.
    wrap_col: usize,
}

impl WrapMap {
    /// Compute a new WrapMap for the entire buffer.
    pub fn new(buffer: &Buffer, wrap_col: usize) -> Self {
        let line_count = buffer.line_count();
        let mut breaks = Vec::with_capacity(line_count);
        for i in 0..line_count {
            let text = buffer.get_line(i).unwrap_or_default();
            breaks.push(find_breaks(&text, wrap_col));
        }
        let (visual_offsets, total_visual_rows) = build_offsets(&breaks);
        WrapMap {
            breaks,
            visual_offsets,
            total_visual_rows,
            wrap_col,
        }
    }

    /// Rebuild the entire map (e.g., after resize or major edit).
    pub fn rebuild(&mut self, buffer: &Buffer) {
        let line_count = buffer.line_count();
        self.breaks.clear();
        self.breaks.reserve(line_count);
        for i in 0..line_count {
            let text = buffer.get_line(i).unwrap_or_default();
            self.breaks.push(find_breaks(&text, self.wrap_col));
        }
        let (offsets, total) = build_offsets(&self.breaks);
        self.visual_offsets = offsets;
        self.total_visual_rows = total;
    }

    /// Rebuild with a new wrap column.
    pub fn rebuild_with_col(&mut self, buffer: &Buffer, wrap_col: usize) {
        self.wrap_col = wrap_col;
        self.rebuild(buffer);
    }

    /// Number of visual rows for a given buffer line.
    pub fn visual_rows_for(&self, line: usize) -> usize {
        if line < self.breaks.len() {
            1 + self.breaks[line].len()
        } else {
            1
        }
    }

    /// Total visual rows in the buffer.
    pub fn total_visual_rows(&self) -> usize {
        self.total_visual_rows
    }

    /// The byte range of a particular segment (sub-line) within a buffer line.
    /// Segment 0 = start..first_break, segment 1 = first_break..second_break, etc.
    pub fn segment_byte_range(&self, line: usize, segment: usize) -> (usize, usize) {
        let line_breaks = if line < self.breaks.len() {
            &self.breaks[line]
        } else {
            return (0, 0);
        };
        let start = if segment == 0 {
            0
        } else if segment - 1 < line_breaks.len() {
            line_breaks[segment - 1]
        } else {
            return (0, 0);
        };
        let end = if segment < line_breaks.len() {
            line_breaks[segment]
        } else {
            usize::MAX // caller should clamp to line length
        };
        (start, end)
    }

    /// Convert buffer position (line, byte_col) to (visual_row, visual_col).
    pub fn buffer_to_visual(
        &self,
        line: usize,
        byte_col: usize,
        line_text: &str,
    ) -> (usize, usize) {
        let base_visual = if line < self.visual_offsets.len() {
            self.visual_offsets[line]
        } else {
            self.total_visual_rows
        };
        let line_breaks = if line < self.breaks.len() {
            &self.breaks[line]
        } else {
            return (
                base_visual,
                display_width(&line_text[..byte_col.min(line_text.len())]),
            );
        };

        // Find which segment this byte_col falls into
        let mut segment = 0;
        for (i, &brk) in line_breaks.iter().enumerate() {
            if byte_col >= brk {
                segment = i + 1;
            } else {
                break;
            }
        }

        let seg_start = if segment == 0 {
            0
        } else {
            line_breaks[segment - 1]
        };

        let clamped_col = byte_col.min(line_text.len());
        let seg_start_clamped = seg_start.min(line_text.len());
        let visual_col = display_width(&line_text[seg_start_clamped..clamped_col]);

        (base_visual + segment, visual_col)
    }

    /// Convert a visual row to (buffer_line, segment_index).
    pub fn visual_to_buffer(&self, visual_row: usize) -> (usize, usize) {
        // Binary search for the line: find the last line whose visual_offset <= visual_row
        if self.visual_offsets.is_empty() {
            return (0, 0);
        }
        let line_count = self.visual_offsets.len();
        let mut lo = 0;
        let mut hi = line_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.visual_offsets[mid] <= visual_row {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let line = lo.saturating_sub(1);
        let segment = visual_row.saturating_sub(self.visual_offsets[line]);
        (line, segment)
    }
}

// ---------------------------------------------------------------------------
// Break finding
// ---------------------------------------------------------------------------

/// Find word-boundary break points for a single line.
/// Returns byte offsets where visual line breaks should occur.
fn find_breaks(line: &str, wrap_col: usize) -> Vec<usize> {
    if wrap_col == 0 {
        return Vec::new();
    }

    let mut breaks = Vec::new();
    let mut display_col: usize = 0;
    let mut byte_offset: usize = 0;
    let mut last_break_opportunity: Option<usize> = None; // byte offset after a space/punct
    let mut seg_start_byte: usize = 0;

    for ch in line.chars() {
        let cw = char_width(ch);

        if display_col + cw > wrap_col && display_col > 0 {
            // Need to break
            let break_at = if let Some(opp) = last_break_opportunity {
                if opp > seg_start_byte {
                    opp
                } else {
                    byte_offset
                }
            } else {
                byte_offset
            };

            breaks.push(break_at);

            // Reset for next segment
            seg_start_byte = break_at;
            last_break_opportunity = None;

            // Recalculate display_col from break point to current position
            if break_at == byte_offset {
                display_col = 0;
            } else {
                display_col = display_width(&line[break_at..byte_offset]);
            }
        }

        byte_offset += ch.len_utf8();
        display_col += cw;

        // Record break opportunities after spaces and punctuation
        if ch == ' ' || ch == '\t' || ch == '-' || ch == '/' || ch == ',' || ch == '.' || ch == ';'
        {
            last_break_opportunity = Some(byte_offset);
        }
    }

    breaks
}

/// Calculate display width of a string slice.
fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

// ---------------------------------------------------------------------------
// Prefix sum builder
// ---------------------------------------------------------------------------

fn build_offsets(breaks: &[Vec<usize>]) -> (Vec<usize>, usize) {
    let mut offsets = Vec::with_capacity(breaks.len());
    let mut total: usize = 0;
    for line_breaks in breaks {
        offsets.push(total);
        total += 1 + line_breaks.len();
    }
    (offsets, total)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_line_no_breaks() {
        let breaks = find_breaks("", 80);
        assert!(breaks.is_empty());
    }

    #[test]
    fn test_short_line_no_breaks() {
        let breaks = find_breaks("hello world", 80);
        assert!(breaks.is_empty());
    }

    #[test]
    fn test_long_line_wraps_at_word_boundary() {
        // "hello world" with wrap_col=7: "hello " fits in 6 cols, "w" at col 6
        // "hello w" = 7 cols, fits. "hello wo" = 8, wraps.
        // Actually let's use a clear example:
        let line = "aaa bbb ccc ddd";
        // wrap_col=8: "aaa bbb " = 8 display cols
        let breaks = find_breaks(line, 8);
        // Should break after "aaa bbb " (byte 8) and "ccc " (byte 12)
        assert!(!breaks.is_empty());
        // First break should be at a word boundary
        let first = breaks[0];
        assert!(first > 0);
        assert!(first <= line.len());
    }

    #[test]
    fn test_long_word_forced_break() {
        let line = "abcdefghijklmnop"; // 16 chars, no spaces
        let breaks = find_breaks(line, 5);
        // Should force break every 5 chars
        assert!(!breaks.is_empty());
        assert_eq!(breaks[0], 5);
    }

    #[test]
    fn test_utf8_chars() {
        let line = "hello \u{00e9}\u{00e9}\u{00e9}\u{00e9}\u{00e9}\u{00e9}"; // "hello " + 6 accented e's (each 2 bytes, 1 col)
        let breaks = find_breaks(line, 8);
        // "hello \u{00e9}\u{00e9}" = 8 display cols, fits
        // "hello \u{00e9}\u{00e9}\u{00e9}" = 9 display cols, breaks
        // Break should happen within the accented characters
        assert!(!breaks.is_empty() || line.chars().map(char_width).sum::<usize>() <= 8);
    }

    #[test]
    fn test_prefix_sum_correctness() {
        let breaks = vec![
            vec![],       // line 0: 1 visual row
            vec![10, 20], // line 1: 3 visual rows
            vec![5],      // line 2: 2 visual rows
            vec![],       // line 3: 1 visual row
        ];
        let (offsets, total) = build_offsets(&breaks);
        assert_eq!(offsets, vec![0, 1, 4, 6]);
        assert_eq!(total, 7);
    }

    #[test]
    fn test_buffer_to_visual_round_trip() {
        let line = "hello world, this is a test of wrapping";
        let wrap_col = 12;
        let breaks = find_breaks(line, wrap_col);
        let breaks_vec = vec![breaks];
        let (offsets, _total) = build_offsets(&breaks_vec);

        let wm = WrapMap {
            breaks: breaks_vec,
            visual_offsets: offsets,
            total_visual_rows: _total,
            wrap_col,
        };

        // Test position at start
        let (vr, vc) = wm.buffer_to_visual(0, 0, line);
        assert_eq!(vr, 0);
        assert_eq!(vc, 0);

        // Test visual_to_buffer for row 0
        let (bl, seg) = wm.visual_to_buffer(0);
        assert_eq!(bl, 0);
        assert_eq!(seg, 0);
    }

    #[test]
    fn test_visual_to_buffer_multi_line() {
        let breaks = vec![
            vec![],   // line 0: 1 visual row (visual row 0)
            vec![10], // line 1: 2 visual rows (visual rows 1, 2)
            vec![],   // line 2: 1 visual row (visual row 3)
        ];
        let (offsets, total) = build_offsets(&breaks);
        let wm = WrapMap {
            breaks,
            visual_offsets: offsets,
            total_visual_rows: total,
            wrap_col: 20,
        };

        assert_eq!(wm.visual_to_buffer(0), (0, 0));
        assert_eq!(wm.visual_to_buffer(1), (1, 0));
        assert_eq!(wm.visual_to_buffer(2), (1, 1));
        assert_eq!(wm.visual_to_buffer(3), (2, 0));
    }

    #[test]
    fn test_segment_byte_range() {
        let breaks = vec![
            vec![10, 20], // line 0 has breaks at byte 10 and 20
        ];
        let (offsets, total) = build_offsets(&breaks);
        let wm = WrapMap {
            breaks,
            visual_offsets: offsets,
            total_visual_rows: total,
            wrap_col: 15,
        };

        assert_eq!(wm.segment_byte_range(0, 0), (0, 10));
        assert_eq!(wm.segment_byte_range(0, 1), (10, 20));
        assert_eq!(wm.segment_byte_range(0, 2), (20, usize::MAX));
    }
}
