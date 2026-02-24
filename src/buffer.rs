use std::fs;
use std::path::{Path, PathBuf};

const INITIAL_GAP: usize = 1024;

/// Normalize `path` for storage: if the path is absolute and is located under
/// the current working directory, strip the CWD prefix so it is stored as a
/// relative path (e.g. `src/main.rs` instead of `/home/user/project/src/main.rs`).
///
/// This ensures that buffers opened from the shell (absolute paths), from the
/// file-picker prompt, from the file-tree, and from the Problems/Diagnostics
/// panels all use the same canonical relative path, preventing duplicate buffers
/// for the same file.
///
/// Paths that are already relative, or that point outside the CWD, are returned
/// unchanged.
fn normalize_to_relative(path: &Path) -> PathBuf {
    if path.is_absolute() {
        if let Ok(cwd) = std::env::current_dir() {
            if let Ok(rel) = path.strip_prefix(&cwd) {
                return rel.to_path_buf();
            }
        }
    }
    path.to_path_buf()
}

pub struct Buffer {
    data: Vec<u8>,
    gap_start: usize,
    gap_end: usize,
    lines: Vec<usize>,
    modified: bool,
    file_path: Option<PathBuf>,
}

impl Buffer {
    pub fn new() -> Buffer {
        let data = vec![0u8; INITIAL_GAP];
        Buffer {
            data,
            gap_start: 0,
            gap_end: INITIAL_GAP,
            lines: vec![0],
            modified: false,
            file_path: None,
        }
    }

    pub fn from_file(path: &Path) -> Result<Buffer, String> {
        let content = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
        let content_len = content.len();
        let gap_size = INITIAL_GAP.max(content_len / 4);
        let mut data = Vec::with_capacity(content_len + gap_size);
        data.extend_from_slice(&content);
        data.resize(content_len + gap_size, 0);

        let mut buf = Buffer {
            data,
            gap_start: content_len,
            gap_end: content_len + gap_size,
            lines: Vec::new(),
            modified: false,
            file_path: Some(normalize_to_relative(path)),
        };
        buf.rebuild_lines();
        Ok(buf)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = self
            .file_path
            .as_ref()
            .ok_or_else(|| "No file path set".to_string())?;
        fs::write(path, self.text_bytes()).map_err(|e| format!("Failed to write file: {}", e))
    }

    pub fn save_to(&mut self, path: &Path) -> Result<(), String> {
        fs::write(path, self.text_bytes()).map_err(|e| format!("Failed to write file: {}", e))?;
        self.file_path = Some(normalize_to_relative(path));
        self.modified = false;
        Ok(())
    }

    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn mark_saved(&mut self) {
        self.modified = false;
    }

    // --- Text access ---

    pub fn len(&self) -> usize {
        self.data.len() - self.gap_len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get_line(&self, line: usize) -> Option<String> {
        if line >= self.lines.len() {
            return None;
        }
        let start = self.lines[line];
        let end = if line + 1 < self.lines.len() {
            // Line ends just before the '\n' that starts the next line
            self.lines[line + 1] - 1
        } else {
            self.len()
        };
        let mut result = Vec::with_capacity(end - start);
        for i in start..end {
            if let Some(b) = self.byte_at(i) {
                result.push(b);
            }
        }
        Some(String::from_utf8_lossy(&result).into_owned())
    }

    /// Extract text between byte offsets `[start, end)` without modifying the buffer.
    pub fn slice(&self, start: usize, end: usize) -> String {
        let start = start.min(self.len());
        let end = end.min(self.len());
        if start >= end {
            return String::new();
        }
        let mut result = Vec::with_capacity(end - start);
        for i in start..end {
            if let Some(b) = self.byte_at(i) {
                result.push(b);
            }
        }
        String::from_utf8_lossy(&result).into_owned()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.text_bytes()).into_owned()
    }

    pub fn char_at(&self, byte_pos: usize) -> Option<char> {
        if byte_pos >= self.len() {
            return None;
        }
        let first = self.byte_at(byte_pos)?;
        let char_len = utf8_char_len(first);
        if char_len == 1 {
            return Some(first as char);
        }
        let mut bytes = [first, 0, 0, 0];
        for (i, slot) in bytes[1..char_len].iter_mut().enumerate() {
            *slot = self.byte_at(byte_pos + 1 + i)?;
        }
        std::str::from_utf8(&bytes[..char_len])
            .ok()
            .and_then(|s| s.chars().next())
    }

    // --- Editing ---

    pub fn insert(&mut self, pos: usize, text: &str) {
        let pos = pos.min(self.len());
        let bytes = text.as_bytes();
        self.ensure_gap(bytes.len());
        self.move_gap(pos);
        self.data[self.gap_start..self.gap_start + bytes.len()].copy_from_slice(bytes);
        self.gap_start += bytes.len();
        self.modified = true;
        self.update_lines(pos);
    }

    pub fn delete(&mut self, pos: usize, len: usize) -> String {
        if len == 0 || pos >= self.len() {
            return String::new();
        }
        let len = len.min(self.len() - pos);
        // Collect the bytes being deleted
        let mut deleted = Vec::with_capacity(len);
        for i in pos..pos + len {
            if let Some(b) = self.byte_at(i) {
                deleted.push(b);
            }
        }
        self.move_gap(pos);
        self.gap_end += len;
        self.modified = true;
        self.update_lines(pos);
        String::from_utf8_lossy(&deleted).into_owned()
    }

    // --- Line info ---

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.lines.get(line).copied()
    }

    pub fn line_end(&self, line: usize) -> Option<usize> {
        if line >= self.lines.len() {
            return None;
        }
        if line + 1 < self.lines.len() {
            Some(self.lines[line + 1] - 1)
        } else {
            Some(self.len())
        }
    }

    pub fn byte_to_line(&self, byte_pos: usize) -> usize {
        // Binary search: find the last line whose start <= byte_pos
        match self.lines.binary_search(&byte_pos) {
            Ok(i) => i,
            Err(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
        }
    }

    // --- Internal ---

    fn gap_len(&self) -> usize {
        self.gap_end - self.gap_start
    }

    fn logical_to_physical(&self, pos: usize) -> usize {
        if pos < self.gap_start {
            pos
        } else {
            pos + self.gap_len()
        }
    }

    fn byte_at(&self, pos: usize) -> Option<u8> {
        if pos >= self.len() {
            return None;
        }
        Some(self.data[self.logical_to_physical(pos)])
    }

    fn move_gap(&mut self, pos: usize) {
        let pos = pos.min(self.len());
        if pos == self.gap_start {
            return;
        }
        if pos < self.gap_start {
            let count = self.gap_start - pos;
            let src = pos;
            let dst = self.gap_end - count;
            self.data.copy_within(src..src + count, dst);
            self.gap_start = pos;
            self.gap_end = dst;
        } else {
            // pos > gap_start
            let count = pos - self.gap_start;
            let src = self.gap_end;
            let dst = self.gap_start;
            self.data.copy_within(src..src + count, dst);
            self.gap_start = pos;
            self.gap_end = src + count;
        }
    }

    fn ensure_gap(&mut self, needed: usize) {
        if self.gap_len() >= needed {
            return;
        }
        let grow = INITIAL_GAP.max(self.len() / 4).max(needed);
        let old_len = self.data.len();
        let after_gap = old_len - self.gap_end;
        self.data.resize(old_len + grow, 0);
        // Move post-gap data to end
        if after_gap > 0 {
            let new_after_start = self.data.len() - after_gap;
            self.data
                .copy_within(self.gap_end..self.gap_end + after_gap, new_after_start);
            self.gap_end = new_after_start;
        } else {
            self.gap_end = self.data.len();
        }
    }

    /// Full rebuild — called only on initial file load.
    /// Scans both gap-buffer segments via `scan_newlines`.
    fn rebuild_lines(&mut self) {
        self.lines.clear();
        self.lines.push(0);
        scan_newlines(&self.data[..self.gap_start], 0, &mut self.lines);
        scan_newlines(&self.data[self.gap_end..], self.gap_start, &mut self.lines);
    }

    /// Incremental update after an insert or delete at logical byte position
    /// `edit_pos`.  Only re-scans from the start of the edited line to end
    /// of buffer — O(tail) instead of O(total).
    ///
    /// The gap buffer has already been updated before this method is called,
    /// so `self.gap_start` / `self.gap_end` reflect the post-edit layout.
    fn update_lines(&mut self, edit_pos: usize) {
        // First cached line-start that is strictly after the edit point.
        // Lines 0..line_idx are unaffected (their byte offsets didn't change).
        let line_idx = self.lines.partition_point(|&off| off <= edit_pos);

        // The line that *contains* edit_pos starts here.
        let rescan_from = if line_idx > 0 { self.lines[line_idx - 1] } else { 0 };

        // Drop everything from line_idx onwards — it will be re-discovered.
        self.lines.truncate(line_idx);

        // Re-scan from rescan_from to end of buffer, respecting the gap split.
        if rescan_from < self.gap_start {
            // edit_pos is in the pre-gap segment; scan both segments.
            scan_newlines(
                &self.data[rescan_from..self.gap_start],
                rescan_from,
                &mut self.lines,
            );
            scan_newlines(&self.data[self.gap_end..], self.gap_start, &mut self.lines);
        } else {
            // edit_pos is at or after the gap; only the post-gap segment needs scanning.
            let physical = rescan_from + self.gap_end - self.gap_start;
            scan_newlines(&self.data[physical..], rescan_from, &mut self.lines);
        }
    }

    pub fn text_bytes(&self) -> Vec<u8> {
        let total = self.len();
        let mut result = Vec::with_capacity(total);
        result.extend_from_slice(&self.data[..self.gap_start]);
        result.extend_from_slice(&self.data[self.gap_end..]);
        result
    }
}

// ---------------------------------------------------------------------------
// Newline scanner
// ---------------------------------------------------------------------------

/// Scan `data` for `\n` bytes, pushing the byte offset of each *subsequent
/// line start* (i.e. `base + i + 1`) into `out`.
///
/// `base` is the logical offset of `data[0]` within the full buffer — used
/// when scanning only one segment of the gap buffer.
///
/// Uses an 8-byte SWAR word scan for ~4× throughput over byte-by-byte on
/// 64-bit targets, without any platform-specific intrinsics.
fn scan_newlines(data: &[u8], base: usize, out: &mut Vec<usize>) {
    // '\n' = 0x0A. XOR each byte with 0x0A; a zero byte signals a newline.
    const NL_MASK: u64 = 0x0A0A_0A0A_0A0A_0A0A_u64;
    // Zero-byte detection: (word - 0x01..01) & ~word & 0x80..80 is non-zero
    // iff any byte in `word` is zero.
    const LO_BITS: u64 = 0x0101_0101_0101_0101_u64;
    const HI_BITS: u64 = 0x8080_8080_8080_8080_u64;

    let mut i = 0;

    // Walk byte-by-byte until the pointer is 8-byte aligned.
    while i < data.len() && (data[i..].as_ptr() as usize) % 8 != 0 {
        if data[i] == b'\n' {
            out.push(base + i + 1);
        }
        i += 1;
    }

    // 8-byte word scan.
    while i + 8 <= data.len() {
        let word = u64::from_le_bytes(data[i..i + 8].try_into().unwrap());
        let has_zero = (word ^ NL_MASK).wrapping_sub(LO_BITS) & !(word ^ NL_MASK) & HI_BITS;
        if has_zero != 0 {
            // At least one '\n' in this word — find it exactly.
            for j in 0..8_usize {
                if data[i + j] == b'\n' {
                    out.push(base + i + j + 1);
                }
            }
        }
        i += 8;
    }

    // Remaining tail bytes.
    while i < data.len() {
        if data[i] == b'\n' {
            out.push(base + i + 1);
        }
        i += 1;
    }
}

fn utf8_char_len(first_byte: u8) -> usize {
    if first_byte & 0x80 == 0 {
        1
    } else if first_byte & 0xE0 == 0xC0 {
        2
    } else if first_byte & 0xF0 == 0xE0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_empty_buffer() {
        let buf = Buffer::new();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert_eq!(buf.line_count(), 1);
        assert!(!buf.is_modified());
        assert_eq!(buf.text(), "");
        assert_eq!(buf.get_line(0), Some(String::new()));
        assert_eq!(buf.get_line(1), None);
    }

    #[test]
    fn test_insert_single_char() {
        let mut buf = Buffer::new();
        buf.insert(0, "a");
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.text(), "a");
        assert!(buf.is_modified());
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn test_insert_multiline() {
        let mut buf = Buffer::new();
        buf.insert(0, "hello\nworld\n");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.get_line(0), Some("hello".into()));
        assert_eq!(buf.get_line(1), Some("world".into()));
        assert_eq!(buf.get_line(2), Some(String::new()));
        assert_eq!(buf.text(), "hello\nworld\n");
    }

    #[test]
    fn test_insert_utf8() {
        let mut buf = Buffer::new();
        buf.insert(0, "café ñ 日本語");
        assert_eq!(buf.text(), "café ñ 日本語");
        assert_eq!(buf.char_at(0), Some('c'));
        assert_eq!(buf.char_at(3), Some('é'));
        // 'é' is 2 bytes, so 'café' = 5 bytes, then space at 5
        assert_eq!(buf.char_at(5), Some(' '));
    }

    #[test]
    fn test_delete_range() {
        let mut buf = Buffer::new();
        buf.insert(0, "hello world");
        let deleted = buf.delete(5, 6);
        assert_eq!(deleted, " world");
        assert_eq!(buf.text(), "hello");
    }

    #[test]
    fn test_delete_empty() {
        let mut buf = Buffer::new();
        buf.insert(0, "abc");
        let deleted = buf.delete(3, 5);
        assert_eq!(deleted, "");
        assert_eq!(buf.text(), "abc");
    }

    #[test]
    fn test_gap_movement() {
        let mut buf = Buffer::new();
        buf.insert(0, "abcdef");
        // Insert in middle forces gap move
        buf.insert(3, "XY");
        assert_eq!(buf.text(), "abcXYdef");
        // Insert at beginning
        buf.insert(0, "Z");
        assert_eq!(buf.text(), "ZabcXYdef");
        // Insert at end
        buf.insert(buf.len(), "!");
        assert_eq!(buf.text(), "ZabcXYdef!");
    }

    #[test]
    fn test_get_line() {
        let mut buf = Buffer::new();
        buf.insert(0, "first\nsecond\nthird");
        assert_eq!(buf.get_line(0), Some("first".into()));
        assert_eq!(buf.get_line(1), Some("second".into()));
        assert_eq!(buf.get_line(2), Some("third".into()));
        assert_eq!(buf.get_line(3), None);
    }

    #[test]
    fn test_byte_to_line() {
        let mut buf = Buffer::new();
        buf.insert(0, "ab\ncd\nef");
        // "ab\ncd\nef" -> lines at 0, 3, 6
        assert_eq!(buf.byte_to_line(0), 0); // 'a'
        assert_eq!(buf.byte_to_line(1), 0); // 'b'
        assert_eq!(buf.byte_to_line(2), 0); // '\n'
        assert_eq!(buf.byte_to_line(3), 1); // 'c'
        assert_eq!(buf.byte_to_line(5), 1); // '\n'
        assert_eq!(buf.byte_to_line(6), 2); // 'e'
    }

    #[test]
    fn test_line_start_end() {
        let mut buf = Buffer::new();
        buf.insert(0, "ab\ncd\nef");
        assert_eq!(buf.line_start(0), Some(0));
        assert_eq!(buf.line_end(0), Some(2)); // before '\n'
        assert_eq!(buf.line_start(1), Some(3));
        assert_eq!(buf.line_end(1), Some(5));
        assert_eq!(buf.line_start(2), Some(6));
        assert_eq!(buf.line_end(2), Some(8)); // end of buffer
    }

    #[test]
    fn test_file_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join("zedit_test_buffer.txt");
        let content = "Hello\nWorld\nTest\n";

        // Write test file
        {
            let mut f = fs::File::create(&path).unwrap();
            f.write_all(content.as_bytes()).unwrap();
        }

        let buf = Buffer::from_file(&path).unwrap();
        assert_eq!(buf.text(), content);
        assert_eq!(buf.line_count(), 4);
        assert!(!buf.is_modified());
        assert_eq!(buf.file_path(), Some(path.as_path()));

        // Save to different file
        let path2 = dir.join("zedit_test_buffer2.txt");
        let mut buf = buf;
        buf.save_to(&path2).unwrap();
        let buf2 = Buffer::from_file(&path2).unwrap();
        assert_eq!(buf2.text(), content);

        // Cleanup
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&path2);
    }

    #[test]
    fn test_modified_flag() {
        let mut buf = Buffer::new();
        assert!(!buf.is_modified());
        buf.insert(0, "x");
        assert!(buf.is_modified());
        buf.mark_saved();
        assert!(!buf.is_modified());
        buf.delete(0, 1);
        assert!(buf.is_modified());
    }

    #[test]
    fn test_large_insert() {
        let mut buf = Buffer::new();
        let large = "x".repeat(10240);
        buf.insert(0, &large);
        assert_eq!(buf.len(), 10240);
        assert_eq!(buf.text(), large);
        // Insert more at middle
        buf.insert(5000, "MIDDLE");
        assert_eq!(buf.len(), 10246);
        assert_eq!(buf.char_at(5000), Some('M'));
    }

    #[test]
    fn test_slice() {
        let mut buf = Buffer::new();
        buf.insert(0, "hello world");
        assert_eq!(buf.slice(0, 5), "hello");
        assert_eq!(buf.slice(6, 11), "world");
        assert_eq!(buf.slice(0, 11), "hello world");
        assert_eq!(buf.slice(5, 5), "");
        assert_eq!(buf.slice(0, 0), "");
        assert_eq!(buf.slice(0, 100), "hello world"); // clamped
    }

    #[test]
    fn test_slice_utf8() {
        let mut buf = Buffer::new();
        buf.insert(0, "café ñ");
        // c(1) a(1) f(1) é(2) = 5 bytes, then ' '(1) ñ(2) = 8 total
        assert_eq!(buf.slice(0, 5), "café");
        assert_eq!(buf.slice(6, 8), "ñ");
    }

    #[test]
    fn test_sequential_inserts() {
        let mut buf = Buffer::new();
        for c in "hello".chars() {
            let pos = buf.len();
            buf.insert(pos, &c.to_string());
        }
        assert_eq!(buf.text(), "hello");
    }

    // ── Phase 25: incremental line cache ────────────────────────────────────

    /// Helper: rebuild lines from scratch and return the vec.
    fn full_lines(text: &str) -> Vec<usize> {
        let mut buf = Buffer::new();
        buf.insert(0, text);
        // Force a full rebuild to compare against.
        buf.rebuild_lines();
        buf.lines.clone()
    }

    #[test]
    fn test_incremental_matches_full_rebuild_insert() {
        let mut buf = Buffer::new();
        buf.insert(0, "line1\nline2\nline3\n");

        buf.insert(6, "inserted\n");

        let expected = full_lines("line1\ninserted\nline2\nline3\n");
        assert_eq!(buf.lines, expected, "incremental insert must match full rebuild");
        assert_eq!(buf.get_line(1), Some("inserted".into()));
    }

    #[test]
    fn test_incremental_delete_spanning_newline() {
        let mut buf = Buffer::new();
        buf.insert(0, "ab\ncd\nef\n");
        // Delete "\ncd\n" at position 2, length 4.
        buf.delete(2, 4);
        assert_eq!(buf.line_count(), 2, "should have 2 lines after delete");
        assert_eq!(buf.get_line(0), Some("abef".into()));
        assert_eq!(buf.get_line(1), Some("".into()));
    }

    #[test]
    fn test_incremental_insert_at_start() {
        let mut buf = Buffer::new();
        buf.insert(0, "world\n");
        buf.insert(0, "hello\n");
        let expected = full_lines("hello\nworld\n");
        assert_eq!(buf.lines, expected);
        assert_eq!(buf.get_line(0), Some("hello".into()));
        assert_eq!(buf.get_line(1), Some("world".into()));
    }

    #[test]
    fn test_incremental_sequential_char_inserts() {
        // Simulate typing one character at a time; lines must stay consistent.
        let text = "foo\nbar\nbaz\n";
        let mut buf = Buffer::new();
        for (i, ch) in text.chars().enumerate() {
            let byte_pos: usize = text[..i].len(); // byte offset, not char index
            buf.insert(byte_pos, &ch.to_string());
        }
        let expected = full_lines(text);
        assert_eq!(buf.lines, expected);
    }

    #[test]
    fn test_scan_newlines_word_alignment() {
        // Exercise the 8-byte word path with various lengths to catch off-by-one
        // in the alignment prefix and suffix handling.
        for len in 0..64_usize {
            let text: String = (0..len)
                .map(|i| if i % 7 == 6 { '\n' } else { 'x' })
                .collect();
            let mut buf = Buffer::new();
            buf.insert(0, &text);

            let expected = full_lines(&text);
            assert_eq!(
                buf.lines, expected,
                "mismatch for text length {len}: {:?}",
                text
            );
        }
    }
}
