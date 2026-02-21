// ---------------------------------------------------------------------------
// Hover popup — floating tooltip shown on Alt+K
// ---------------------------------------------------------------------------

pub(super) struct HoverPopup {
    pub lines: Vec<String>,
    pub anchor_screen_row: usize,
    pub anchor_screen_col: usize,
}

impl HoverPopup {
    /// Word-wrap `content` at `max_width` characters, truncate at 12 lines.
    /// Strips simple markdown formatting (code fences, header markers).
    pub fn new(content: &str, anchor_row: usize, anchor_col: usize, max_width: usize) -> Self {
        let mut lines: Vec<String> = Vec::new();
        let max_w = max_width.max(10);

        for raw_line in content.lines() {
            let cleaned = strip_markdown(raw_line);

            if cleaned.is_empty() {
                // Keep a blank line to separate paragraphs, but not at start
                if !lines.is_empty()
                    && lines
                        .last()
                        .map(|l: &String| !l.is_empty())
                        .unwrap_or(false)
                {
                    lines.push(String::new());
                }
                if lines.len() >= 12 {
                    break;
                }
                continue;
            }

            // Word-wrap
            let mut remaining = cleaned.as_str();
            while !remaining.is_empty() {
                if remaining.chars().count() <= max_w {
                    lines.push(remaining.to_string());
                    break;
                }
                // Find a good break point within max_w characters
                let char_boundary = char_index_to_byte(remaining, max_w);
                let break_at = remaining[..char_boundary]
                    .rfind(' ')
                    .unwrap_or(char_boundary);
                lines.push(remaining[..break_at].to_string());
                remaining = remaining[break_at..].trim_start();
                if lines.len() >= 12 {
                    break;
                }
            }

            if lines.len() >= 12 {
                break;
            }
        }

        // Remove trailing blank lines
        while lines.last().map(|l: &String| l.is_empty()).unwrap_or(false) {
            lines.pop();
        }

        lines.truncate(12);

        HoverPopup {
            lines,
            anchor_screen_row: anchor_row,
            anchor_screen_col: anchor_col,
        }
    }
}

/// Strip common markdown formatting for plain-text display.
fn strip_markdown(s: &str) -> String {
    let s = s.trim();

    // Skip code fence delimiters
    if s.starts_with("```") || s.starts_with("~~~") {
        return String::new();
    }

    // Strip leading header markers
    let s = s.trim_start_matches('#').trim_start();

    // Inline code: replace backtick spans with their contents
    let mut result = String::with_capacity(s.len());
    let mut in_code = false;
    for ch in s.chars() {
        if ch == '`' {
            in_code = !in_code;
        } else {
            result.push(ch);
        }
    }

    result.trim().to_string()
}

/// Convert a character index to a byte index in a UTF-8 string.
fn char_index_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}
