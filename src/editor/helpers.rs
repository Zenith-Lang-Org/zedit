use std::path::Path;

/// Shorten a file path for display: replace $HOME prefix with `~`.
pub(super) fn shorten_path(path: &Path) -> String {
    let full = path.to_string_lossy();
    if let Some(home) = std::env::var_os("HOME") {
        let home_str = home.to_string_lossy();
        if let Some(rest) = full.strip_prefix(home_str.as_ref()) {
            if rest.is_empty() {
                return "~".to_string();
            }
            if rest.starts_with('/') {
                return format!("~{}", rest);
            }
        }
    }
    full.into_owned()
}

/// Convert a byte column offset into a display column (sum of char widths).
pub(super) fn byte_col_to_display_col(line: &str, byte_col: usize) -> usize {
    let clamped = byte_col.min(line.len());
    line[..clamped]
        .chars()
        .map(crate::unicode::char_width)
        .sum()
}

/// Convert a display column (visual column) back to a byte offset.
pub(super) fn display_col_to_byte_col(line: &str, display_col: usize) -> usize {
    let mut byte_offset = 0;
    let mut visual_col = 0;
    for ch in line.chars() {
        if visual_col >= display_col {
            break;
        }
        byte_offset += ch.len_utf8();
        visual_col += crate::unicode::char_width(ch);
    }
    byte_offset
}
