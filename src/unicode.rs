/// Number of columns per tab stop (used for `\t` rendering).
pub const TAB_WIDTH: usize = 4;

/// Return the display width (in terminal columns) of a single character.
///
/// Note: `\t` returns 1 here; callers that track the current column should
/// use `tab_stop_width(current_col)` instead for accurate tab expansion.
///
/// - 0 for combining marks, zero-width chars (ZWJ, ZWNJ, ZWS, BOM, etc.)
/// - 2 for CJK ideographs, Hangul syllables, fullwidth forms, and related blocks
/// - 1 for everything else
pub fn char_width(ch: char) -> usize {
    let cp = ch as u32;

    // Zero-width characters
    if is_zero_width(cp) {
        return 0;
    }

    // Wide (2-column) characters
    if is_wide(cp) {
        return 2;
    }

    1
}

/// Return the display width of a string (sum of char widths).
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn is_zero_width(cp: u32) -> bool {
    // Combining diacritical marks and related combining blocks
    matches!(cp,
        // Combining Diacritical Marks
        0x0300..=0x036F |
        // Combining Diacritical Marks Extended
        0x1AB0..=0x1AFF |
        // Combining Diacritical Marks Supplement
        0x1DC0..=0x1DFF |
        // Combining Diacritical Marks for Symbols
        0x20D0..=0x20FF |
        // Combining Half Marks
        0xFE20..=0xFE2F |
        // Zero-width space, ZWNJ, ZWJ
        0x200B..=0x200D |
        // Word joiner
        0x2060 |
        // BOM / ZWNBSP
        0xFEFF |
        // Soft hyphen
        0x00AD |
        // Variation selectors
        0xFE00..=0xFE0F |
        // Variation selectors supplement
        0xE0100..=0xE01EF |
        // Zero-width no-break space (interlinear annotation)
        0xFFF9..=0xFFFB |
        // Tags block (used in emoji sequences)
        0xE0001 |
        0xE0020..=0xE007F
    )
}

fn is_wide(cp: u32) -> bool {
    matches!(cp,
        // CJK Radicals Supplement
        0x2E80..=0x2EFF |
        // Kangxi Radicals
        0x2F00..=0x2FDF |
        // Ideographic Description Characters
        0x2FF0..=0x2FFF |
        // CJK Symbols and Punctuation
        0x3000..=0x303F |
        // Hiragana
        0x3040..=0x309F |
        // Katakana
        0x30A0..=0x30FF |
        // Bopomofo
        0x3100..=0x312F |
        // Hangul Compatibility Jamo
        0x3130..=0x318F |
        // Kanbun
        0x3190..=0x319F |
        // Bopomofo Extended
        0x31A0..=0x31BF |
        // CJK Strokes
        0x31C0..=0x31EF |
        // Katakana Phonetic Extensions
        0x31F0..=0x31FF |
        // Enclosed CJK Letters and Months
        0x3200..=0x32FF |
        // CJK Compatibility
        0x3300..=0x33FF |
        // CJK Unified Ideographs Extension A
        0x3400..=0x4DBF |
        // CJK Unified Ideographs
        0x4E00..=0x9FFF |
        // Yi Syllables + Radicals
        0xA000..=0xA4CF |
        // Hangul Syllables
        0xAC00..=0xD7AF |
        // CJK Compatibility Ideographs
        0xF900..=0xFAFF |
        // Fullwidth Forms (excluding halfwidth)
        0xFF01..=0xFF60 |
        0xFFE0..=0xFFE6 |
        // CJK Unified Ideographs Extension B
        0x20000..=0x2A6DF |
        // CJK Unified Ideographs Extension C
        0x2A700..=0x2B73F |
        // CJK Unified Ideographs Extension D
        0x2B740..=0x2B81F |
        // CJK Unified Ideographs Extension E
        0x2B820..=0x2CEAF |
        // CJK Unified Ideographs Extension F
        0x2CEB0..=0x2EBEF |
        // CJK Compatibility Ideographs Supplement
        0x2F800..=0x2FA1F |
        // CJK Unified Ideographs Extension G
        0x30000..=0x3134F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_chars_width_1() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('Z'), 1);
        assert_eq!(char_width(' '), 1);
        assert_eq!(char_width('!'), 1);
    }

    #[test]
    fn multibyte_narrow_chars() {
        // Latin accented characters are narrow
        assert_eq!(char_width('é'), 1);
        assert_eq!(char_width('ñ'), 1);
        assert_eq!(char_width('ü'), 1);
    }

    #[test]
    fn cjk_ideographs_width_2() {
        assert_eq!(char_width('中'), 2);
        assert_eq!(char_width('文'), 2);
        assert_eq!(char_width('字'), 2);
    }

    #[test]
    fn hangul_syllables_width_2() {
        assert_eq!(char_width('한'), 2);
        assert_eq!(char_width('글'), 2);
    }

    #[test]
    fn japanese_kana_width_2() {
        assert_eq!(char_width('あ'), 2); // Hiragana
        assert_eq!(char_width('カ'), 2); // Katakana
    }

    #[test]
    fn fullwidth_forms_width_2() {
        assert_eq!(char_width('\u{FF01}'), 2); // Fullwidth exclamation mark
        assert_eq!(char_width('\u{FF21}'), 2); // Fullwidth Latin A
    }

    #[test]
    fn zero_width_chars() {
        assert_eq!(char_width('\u{200B}'), 0); // Zero-width space
        assert_eq!(char_width('\u{200D}'), 0); // ZWJ
        assert_eq!(char_width('\u{FEFF}'), 0); // BOM
        assert_eq!(char_width('\u{0300}'), 0); // Combining grave accent
    }

    #[test]
    fn str_width_ascii() {
        assert_eq!(str_width("hello"), 5);
    }

    #[test]
    fn str_width_cjk() {
        assert_eq!(str_width("你好"), 4);
        assert_eq!(str_width("你好世界"), 8);
    }

    #[test]
    fn str_width_mixed() {
        assert_eq!(str_width("hi你好"), 6); // 2 + 4
    }

    #[test]
    fn str_width_empty() {
        assert_eq!(str_width(""), 0);
    }

    #[test]
    fn str_width_with_combining() {
        // 'e' (1) + combining accent (0) = 1
        assert_eq!(str_width("e\u{0301}"), 1);
    }
}
