use crate::buffer::Buffer;
use crate::config::builtin_languages;
use crate::cursor::Cursor;
use crate::syntax::highlight;

use super::*;

#[test]
fn test_compute_gutter_width() {
    assert_eq!(compute_gutter_width(1), 4); // 1 digit + 2 = 3, min 4
    assert_eq!(compute_gutter_width(9), 4); // 1 digit + 2 = 3, min 4
    assert_eq!(compute_gutter_width(10), 4); // 2 digits + 2 = 4
    assert_eq!(compute_gutter_width(99), 4); // 2 digits + 2 = 4
    assert_eq!(compute_gutter_width(100), 5); // 3 digits + 2 = 5
    assert_eq!(compute_gutter_width(999), 5);
    assert_eq!(compute_gutter_width(1000), 6); // 4 digits + 2 = 6
}

#[test]
fn test_shorten_path() {
    use std::path::Path;
    // Path outside home stays as-is
    assert_eq!(shorten_path(Path::new("/etc/config")), "/etc/config");

    // Home itself becomes ~
    if let Some(home) = std::env::var_os("HOME") {
        let home_str = home.to_string_lossy().to_string();
        assert_eq!(shorten_path(Path::new(&home_str)), "~");

        // Subpath under home gets ~ prefix
        let sub = format!("{}/projects/zedit", home_str);
        assert_eq!(shorten_path(Path::new(&sub)), "~/projects/zedit");
    }
}

#[test]
fn test_byte_col_to_display_col() {
    assert_eq!(byte_col_to_display_col("hello", 0), 0);
    assert_eq!(byte_col_to_display_col("hello", 3), 3);
    assert_eq!(byte_col_to_display_col("hello", 5), 5);

    // "café" = c(1) a(1) f(1) é(2) = 5 bytes
    assert_eq!(byte_col_to_display_col("café", 0), 0);
    assert_eq!(byte_col_to_display_col("café", 3), 3); // before 'é'
    assert_eq!(byte_col_to_display_col("café", 5), 4); // after 'é'
}

#[test]
fn test_display_col_to_byte_col() {
    assert_eq!(display_col_to_byte_col("hello", 0), 0);
    assert_eq!(display_col_to_byte_col("hello", 3), 3);
    assert_eq!(display_col_to_byte_col("hello", 5), 5);

    // "café" = c(1) a(1) f(1) é(2) = 5 bytes
    assert_eq!(display_col_to_byte_col("café", 3), 3); // before 'é'
    assert_eq!(display_col_to_byte_col("café", 4), 5); // after 'é'
}

// -- Selection tests --

#[test]
fn test_selection_range_ordering() {
    // anchor < head
    let sel = Selection {
        anchor: 5,
        head: 10,
    };
    let (start, end) = {
        let s = sel.anchor.min(sel.head);
        let e = sel.anchor.max(sel.head);
        (s, e)
    };
    assert_eq!(start, 5);
    assert_eq!(end, 10);

    // anchor > head (backwards selection)
    let sel2 = Selection {
        anchor: 10,
        head: 5,
    };
    let (start2, end2) = {
        let s = sel2.anchor.min(sel2.head);
        let e = sel2.anchor.max(sel2.head);
        (s, e)
    };
    assert_eq!(start2, 5);
    assert_eq!(end2, 10);
}

#[test]
fn test_delete_selection_repositions_cursor() {
    let mut buf = Buffer::new();
    buf.insert(0, "hello world");
    let mut cursor = Cursor::new();
    cursor.set_position(0, 5, &buf);

    // Simulate selection of " world" (bytes 5..11)
    let sel = Selection {
        anchor: 5,
        head: 11,
    };
    let (start, end) = (sel.anchor.min(sel.head), sel.anchor.max(sel.head));
    let deleted = buf.slice(start, end);
    buf.delete(start, end - start);
    let line = buf.byte_to_line(start);
    let line_start = buf.line_start(line).unwrap_or(0);
    let col = start - line_start;
    cursor.set_position(line, col, &buf);

    assert_eq!(deleted, " world");
    assert_eq!(buf.text(), "hello");
    assert_eq!(cursor.line, 0);
    assert_eq!(cursor.col, 5);
}

// -- Prompt tests --

#[test]
fn test_prompt_insert_char() {
    let mut prompt = Prompt {
        label: "Open: ".to_string(),
        input: String::new(),
        cursor_pos: 0,
        action: PromptAction::OpenFile,
    };

    // Insert 'a'
    prompt.input.insert_str(prompt.cursor_pos, "a");
    prompt.cursor_pos += 1;
    assert_eq!(prompt.input, "a");
    assert_eq!(prompt.cursor_pos, 1);

    // Insert 'b'
    prompt.input.insert_str(prompt.cursor_pos, "b");
    prompt.cursor_pos += 1;
    assert_eq!(prompt.input, "ab");
    assert_eq!(prompt.cursor_pos, 2);

    // Move cursor left, insert 'x' in the middle
    let before = &prompt.input[..prompt.cursor_pos];
    if let Some(ch) = before.chars().next_back() {
        prompt.cursor_pos -= ch.len_utf8();
    }
    prompt.input.insert_str(prompt.cursor_pos, "x");
    prompt.cursor_pos += 1;
    assert_eq!(prompt.input, "axb");
    assert_eq!(prompt.cursor_pos, 2);
}

#[test]
fn test_prompt_backspace() {
    let mut prompt = Prompt {
        label: "Open: ".to_string(),
        input: "hello".to_string(),
        cursor_pos: 5,
        action: PromptAction::OpenFile,
    };

    // Backspace at end
    let before = &prompt.input[..prompt.cursor_pos];
    if let Some(ch) = before.chars().next_back() {
        let len = ch.len_utf8();
        let new_pos = prompt.cursor_pos - len;
        prompt.input.drain(new_pos..prompt.cursor_pos);
        prompt.cursor_pos = new_pos;
    }
    assert_eq!(prompt.input, "hell");
    assert_eq!(prompt.cursor_pos, 4);
}

#[test]
fn test_prompt_delete() {
    let mut prompt = Prompt {
        label: "Open: ".to_string(),
        input: "hello".to_string(),
        cursor_pos: 0,
        action: PromptAction::OpenFile,
    };

    // Delete at start
    let after = &prompt.input[prompt.cursor_pos..];
    if let Some(ch) = after.chars().next() {
        let len = ch.len_utf8();
        prompt
            .input
            .drain(prompt.cursor_pos..prompt.cursor_pos + len);
    }
    assert_eq!(prompt.input, "ello");
    assert_eq!(prompt.cursor_pos, 0);
}

#[test]
fn test_prompt_cursor_movement() {
    let mut prompt = Prompt {
        label: "Open: ".to_string(),
        input: "abc".to_string(),
        cursor_pos: 0,
        action: PromptAction::OpenFile,
    };

    // Right
    let after = &prompt.input[prompt.cursor_pos..];
    if let Some(ch) = after.chars().next() {
        prompt.cursor_pos += ch.len_utf8();
    }
    assert_eq!(prompt.cursor_pos, 1);

    // End
    prompt.cursor_pos = prompt.input.len();
    assert_eq!(prompt.cursor_pos, 3);

    // Home
    prompt.cursor_pos = 0;
    assert_eq!(prompt.cursor_pos, 0);

    // Left at start — should stay at 0
    if prompt.cursor_pos > 0 {
        let before = &prompt.input[..prompt.cursor_pos];
        if let Some(ch) = before.chars().next_back() {
            prompt.cursor_pos -= ch.len_utf8();
        }
    }
    assert_eq!(prompt.cursor_pos, 0);
}

#[test]
fn test_prompt_utf8_navigation() {
    let mut prompt = Prompt {
        label: "Open: ".to_string(),
        input: "café".to_string(), // c(1) a(1) f(1) é(2) = 5 bytes
        cursor_pos: 5,             // at end
        action: PromptAction::OpenFile,
    };

    // Left from end — should move back over 'é' (2 bytes)
    let before = &prompt.input[..prompt.cursor_pos];
    if let Some(ch) = before.chars().next_back() {
        prompt.cursor_pos -= ch.len_utf8();
    }
    assert_eq!(prompt.cursor_pos, 3);

    // Backspace 'é' — should remove 2 bytes
    // Move cursor back to end to test backspace over 'é'
    prompt.cursor_pos = 5;
    let before3 = &prompt.input[..prompt.cursor_pos];
    if let Some(ch) = before3.chars().next_back() {
        let len = ch.len_utf8();
        let new_pos = prompt.cursor_pos - len;
        prompt.input.drain(new_pos..prompt.cursor_pos);
        prompt.cursor_pos = new_pos;
    }
    assert_eq!(prompt.input, "caf");
    assert_eq!(prompt.cursor_pos, 3);
}

// -- Search tests --

#[test]
fn test_find_all_matches_basic() {
    let matches = find_all_matches("hello hello", "hello", &SearchMode::Substring);
    assert_eq!(matches, vec![(0, 5), (6, 11)]);
}

#[test]
fn test_find_all_matches_case_insensitive() {
    let matches = find_all_matches("Hello HELLO", "hello", &SearchMode::Substring);
    assert_eq!(matches, vec![(0, 5), (6, 11)]);
}

#[test]
fn test_find_all_matches_empty_pattern() {
    let matches = find_all_matches("hello", "", &SearchMode::Substring);
    assert!(matches.is_empty());
}

#[test]
fn test_find_all_matches_no_overlap() {
    let matches = find_all_matches("aaa", "aa", &SearchMode::Substring);
    assert_eq!(matches, vec![(0, 2)]);
}

#[test]
fn test_find_all_matches_utf8() {
    let matches = find_all_matches("café café", "café", &SearchMode::Substring);
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0], (0, 5)); // "café" = 5 bytes
    assert_eq!(matches[1], (6, 11)); // after space
}

// -- BufferState tests --

#[test]
fn test_buffer_state_new_empty() {
    let bs = BufferState::new_empty(true);
    assert_eq!(bs.buffer.len(), 0);
    assert_eq!(bs.cursor.line, 0);
    assert_eq!(bs.scroll_row, 0);
    assert!(bs.selection.is_none());
    assert!(bs.highlighter.is_none());
}

// -- Comment prefix tests --

#[test]
fn test_comment_prefix() {
    let langs = builtin_languages();
    assert_eq!(
        highlight::comment_prefix("rust", &langs).as_deref(),
        Some("//")
    );
    assert_eq!(
        highlight::comment_prefix("python", &langs).as_deref(),
        Some("#")
    );
    assert_eq!(
        highlight::comment_prefix("javascript", &langs).as_deref(),
        Some("//")
    );
    assert_eq!(
        highlight::comment_prefix("shell", &langs).as_deref(),
        Some("#")
    );
    assert_eq!(highlight::comment_prefix("markdown", &langs), None);
}

// -- Regex search tests --

#[test]
fn test_find_all_matches_regex_basic() {
    let matches = find_all_matches("abc 123 def 456", r"\d+", &SearchMode::Regex);
    assert_eq!(matches, vec![(4, 7), (12, 15)]);
}

#[test]
fn test_find_all_matches_regex_invalid() {
    let matches = find_all_matches("hello", r"[invalid", &SearchMode::Regex);
    assert!(matches.is_empty());
}

#[test]
fn test_find_all_matches_regex_zero_length() {
    // ^ matches at start of string — should not infinite loop
    let matches = find_all_matches("hello", r"^", &SearchMode::Regex);
    assert_eq!(matches.len(), 0); // zero-length matches are skipped
}

#[test]
fn test_find_all_matches_regex_word_boundary() {
    let matches = find_all_matches("hello world hello", r"\bhello\b", &SearchMode::Regex);
    assert_eq!(matches, vec![(0, 5), (12, 17)]);
}

#[test]
fn test_find_all_matches_regex_char_class() {
    let matches = find_all_matches("abc 123 def", r"[a-z]+", &SearchMode::Regex);
    assert_eq!(matches, vec![(0, 3), (8, 11)]);
}
