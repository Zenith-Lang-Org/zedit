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
/// Tabs are expanded to the next tab stop (`TAB_WIDTH` columns).
pub(super) fn byte_col_to_display_col(line: &str, byte_col: usize) -> usize {
    let clamped = byte_col.min(line.len());
    let mut col = 0usize;
    for ch in line[..clamped].chars() {
        if ch == '\t' {
            let tw = crate::unicode::TAB_WIDTH;
            col = (col / tw + 1) * tw;
        } else {
            col += crate::unicode::char_width(ch);
        }
    }
    col
}

/// Convert a display column (visual column) back to a byte offset.
/// Tabs are expanded to the next tab stop (`TAB_WIDTH` columns).
pub(super) fn display_col_to_byte_col(line: &str, display_col: usize) -> usize {
    let mut byte_offset = 0;
    let mut visual_col = 0;
    for ch in line.chars() {
        if visual_col >= display_col {
            break;
        }
        let cw = if ch == '\t' {
            let tw = crate::unicode::TAB_WIDTH;
            (visual_col / tw + 1) * tw - visual_col
        } else {
            crate::unicode::char_width(ch)
        };
        byte_offset += ch.len_utf8();
        visual_col += cw;
    }
    byte_offset
}
