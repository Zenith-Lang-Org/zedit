use std::fs;
use std::path::{Path, PathBuf};

const INITIAL_GAP: usize = 1024;

// ---------------------------------------------------------------------------
// Sparse line-start cache (Phase 35)
// ---------------------------------------------------------------------------

/// One checkpoint: byte offset and line index at that position.
#[derive(Clone, Copy)]
struct LineCheckpoint {
    byte_off: usize,
    line: usize,
}

/// How many lines between consecutive checkpoints.
const CHECKPOINT_STRIDE: usize = 8_192;

/// Sparse line cache: one `LineCheckpoint` every `CHECKPOINT_STRIDE` lines.
/// Between checkpoints, linear byte scan finds the exact position.
struct LineCache {
    checkpoints: Vec<LineCheckpoint>,
    /// Total number of lines in the buffer (maintained incrementally).
    total_lines: usize,
}

impl LineCache {
    fn new() -> Self {
        Self {
            checkpoints: vec![LineCheckpoint {
                byte_off: 0,
                line: 0,
            }],
            total_lines: 1,
        }
    }

    /// Full rebuild from a contiguous byte slice.  Called once at file load.
    fn rebuild(&mut self, data: &[u8]) {
        self.checkpoints.clear();
        self.checkpoints.push(LineCheckpoint {
            byte_off: 0,
            line: 0,
        });
        self.total_lines = 1;

        let mut line = 0usize;
        for (i, &b) in data.iter().enumerate() {
            if b == b'\n' {
                line += 1;
                if line % CHECKPOINT_STRIDE == 0 {
                    self.checkpoints.push(LineCheckpoint {
                        byte_off: i + 1,
                        line,
                    });
                }
            }
        }
        self.total_lines = line + 1;
    }

    /// Incremental update after an edit at `edit_byte`.
    /// `byte_delta` is positive for insert, negative for delete.
    /// `newlines_delta` is the net change in newline count.
    fn update(&mut self, edit_byte: usize, byte_delta: isize, newlines_delta: isize) {
        // Adjust all checkpoints strictly after the edit point.
        let pivot = self
            .checkpoints
            .partition_point(|cp| cp.byte_off <= edit_byte);
        for cp in &mut self.checkpoints[pivot..] {
            cp.byte_off = cp.byte_off.wrapping_add_signed(byte_delta);
            cp.line = cp.line.wrapping_add_signed(newlines_delta);
        }
        self.total_lines = self.total_lines.wrapping_add_signed(newlines_delta);
        // NOTE: Does not re-insert checkpoints for new content.  A periodic
        // compaction pass handles checkpoint density degradation (deferred).
    }

    /// Return the byte offset of `target_line`, or `None` if out of range.
    /// Binary-searches to the nearest checkpoint, then calls `scan_fn` to
    /// walk forward.  `scan_fn(from_byte, from_line, to_line) -> (byte, line)`.
    fn line_to_byte<F>(&self, target_line: usize, scan_fn: F) -> Option<usize>
    where
        F: Fn(usize, usize, usize) -> (usize, usize),
    {
        if target_line >= self.total_lines {
            return None;
        }
        let idx = self
            .checkpoints
            .partition_point(|cp| cp.line <= target_line);
        let cp = if idx == 0 {
            self.checkpoints[0]
        } else {
            self.checkpoints[idx - 1]
        };
        if cp.line == target_line {
            return Some(cp.byte_off);
        }
        let (byte_off, _) = scan_fn(cp.byte_off, cp.line, target_line);
        Some(byte_off)
    }

    /// Convert a byte offset to its line number.
    /// `count_fn(from_byte, to_byte) -> newline count`.
    fn byte_to_line<F>(&self, target_byte: usize, count_fn: F) -> usize
    where
        F: Fn(usize, usize) -> usize,
    {
        let idx = self
            .checkpoints
            .partition_point(|cp| cp.byte_off <= target_byte);
        let cp = if idx == 0 {
            self.checkpoints[0]
        } else {
            self.checkpoints[idx - 1]
        };
        cp.line + count_fn(cp.byte_off, target_byte)
    }
}

// ---------------------------------------------------------------------------
// Buffer helper: walk gap-buffer bytes to find a target line
// ---------------------------------------------------------------------------

/// Scan gap-buffer bytes from logical `from_byte` (start of `from_line`)
/// until `(to_line - from_line)` newlines have been counted.
/// Returns `(byte_offset_of_to_line, to_line)`.
fn buf_scan_to_line(
    data: &[u8],
    mmap: &Option<crate::mmap::Mmap>,
    gap_start: usize,
    gap_end: usize,
    from_byte: usize,
    from_line: usize,
    to_line: usize,
) -> (usize, usize) {
    let need = to_line - from_line;
    let mut counted = 0usize;
    let mut pos = from_byte;

    if let Some(map) = mmap {
        let bytes = map.as_bytes();
        while pos < bytes.len() && counted < need {
            if bytes[pos] == b'\n' {
                counted += 1;
            }
            pos += 1;
        }
    } else {
        let total_len = gap_start + (data.len() - gap_end);
        while pos < total_len && counted < need {
            let phys = if pos < gap_start {
                pos
            } else {
                gap_end + (pos - gap_start)
            };
            if data[phys] == b'\n' {
                counted += 1;
            }
            pos += 1;
        }
    }
    (pos, from_line + counted)
}

/// Count newlines in the logical byte range `[from_byte, to_byte)`.
fn buf_count_newlines(
    data: &[u8],
    mmap: &Option<crate::mmap::Mmap>,
    gap_start: usize,
    gap_end: usize,
    from_byte: usize,
    to_byte: usize,
) -> usize {
    let mut count = 0usize;
    if let Some(map) = mmap {
        let bytes = map.as_bytes();
        let lo = from_byte.min(bytes.len());
        let hi = to_byte.min(bytes.len());
        count = bytes[lo..hi].iter().filter(|b| **b == b'\n').count();
    } else {
        for pos in from_byte..to_byte {
            let phys = if pos < gap_start {
                pos
            } else {
                gap_end + (pos - gap_start)
            };
            if phys < data.len() && data[phys] == b'\n' {
                count += 1;
            }
        }
    }
    count
}

/// Files larger than this threshold are opened via `mmap(2)` instead of being
/// read fully into the gap buffer. The OS handles paging so only accessed pages
/// consume RAM. On first edit the entire content is materialized to the gap buffer.
pub const MMAP_THRESHOLD: usize = 1024 * 1024; // 1 MB

// ---------------------------------------------------------------------------
// Gap buffer backing store (Phase 36)
// ---------------------------------------------------------------------------

/// Backing storage for the gap buffer.
///
/// * `Heap`    — heap-allocated `Vec<u8>`.  Used for small files (< 128 KB).
/// * `Virtual` — virtual-memory region that never reallocates.  Used for
///   large files: committing new pages via `mprotect` replaces `Vec::resize`.
enum GapStorage {
    Heap(Vec<u8>),
    Virtual(crate::vmem::VirtualRegion),
}

impl GapStorage {
    /// Return a read-only slice of the first `len` bytes.
    ///
    /// SAFETY (Virtual arm): caller must ensure `len <= committed`.
    #[inline]
    fn as_slice(&self, len: usize) -> &[u8] {
        match self {
            GapStorage::Heap(v) => &v[..len.min(v.len())],
            GapStorage::Virtual(r) => unsafe {
                std::slice::from_raw_parts(r.as_ptr() as *const u8, len)
            },
        }
    }

    /// Raw mutable pointer to byte 0 (for direct writes and `ptr::copy`).
    #[inline]
    fn as_raw_ptr(&mut self) -> *mut u8 {
        match self {
            GapStorage::Heap(v) => v.as_mut_ptr(),
            GapStorage::Virtual(r) => r.as_ptr(),
        }
    }

    /// Physical capacity: total bytes currently usable (text + gap).
    /// For Heap this is `Vec::len()`; for Virtual it is `committed()`.
    #[allow(dead_code)]
    #[inline]
    fn capacity(&self) -> usize {
        match self {
            GapStorage::Heap(v) => v.len(),
            GapStorage::Virtual(r) => r.committed(),
        }
    }
}

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
    /// Backing store for the gap buffer: heap Vec for small files,
    /// virtual memory region for large files (>= VMEM_THRESHOLD).
    storage: GapStorage,
    gap_start: usize,
    gap_end: usize,
    /// Logical text length (bytes), excluding the gap.
    /// Invariant: text_len == storage.capacity() - gap_len()  (when mmap is None)
    text_len: usize,
    line_cache: LineCache,
    modified: bool,
    file_path: Option<PathBuf>,
    /// Non-None while the buffer is backed by a memory-mapped file.
    /// Cleared (and content materialized into `storage`) on first edit.
    mmap: Option<crate::mmap::Mmap>,
}

impl Buffer {
    pub fn new() -> Buffer {
        Buffer {
            storage: GapStorage::Heap(vec![0u8; INITIAL_GAP]),
            gap_start: 0,
            gap_end: INITIAL_GAP,
            text_len: 0,
            line_cache: LineCache::new(),
            modified: false,
            file_path: None,
            mmap: None,
        }
    }

    pub fn from_file(path: &Path) -> Result<Buffer, String> {
        // Large file: attempt mmap to avoid loading everything into RAM.
        let file_size = fs::metadata(path).map(|m| m.len() as usize).unwrap_or(0);

        if file_size > MMAP_THRESHOLD {
            if let Ok(map) = crate::mmap::Mmap::open(path) {
                let bytes = map.as_bytes();
                // Validate UTF-8 without copying (single scan).
                if std::str::from_utf8(bytes).is_err() {
                    return Err("File is not valid UTF-8".to_string());
                }
                // Build the line cache directly from the mapped bytes.
                let mut line_cache = LineCache::new();
                line_cache.rebuild(bytes);
                // Reserve virtual memory for future editing (committed lazily on first edit).
                let storage = if file_size >= crate::vmem::VMEM_THRESHOLD {
                    match crate::vmem::VirtualRegion::reserve(crate::vmem::VMEM_RESERVE) {
                        Ok(region) => GapStorage::Virtual(region),
                        Err(_) => GapStorage::Heap(vec![0u8; INITIAL_GAP]),
                    }
                } else {
                    GapStorage::Heap(vec![0u8; INITIAL_GAP])
                };
                return Ok(Buffer {
                    storage,
                    gap_start: 0,
                    gap_end: INITIAL_GAP,
                    text_len: 0, // still backed by mmap; gap buffer is empty
                    line_cache,
                    modified: false,
                    file_path: Some(normalize_to_relative(path)),
                    mmap: Some(map),
                });
                // If mmap fails, fall through to regular read below.
            }
        }

        // Small/medium file: read fully into gap buffer (Heap or Virtual).
        let content = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
        let content_len = content.len();
        let gap_size = INITIAL_GAP.max(content_len / 4);

        let mut storage = if content_len >= crate::vmem::VMEM_THRESHOLD {
            match crate::vmem::VirtualRegion::reserve(crate::vmem::VMEM_RESERVE) {
                Ok(mut region) => {
                    if region.grow(content_len + gap_size).is_err() {
                        // Fall back to heap if commit fails.
                        let mut v = Vec::with_capacity(content_len + gap_size);
                        v.extend_from_slice(&content);
                        v.resize(content_len + gap_size, 0);
                        GapStorage::Heap(v)
                    } else {
                        GapStorage::Virtual(region)
                    }
                }
                Err(_) => {
                    let mut v = Vec::with_capacity(content_len + gap_size);
                    v.extend_from_slice(&content);
                    v.resize(content_len + gap_size, 0);
                    GapStorage::Heap(v)
                }
            }
        } else {
            let mut v = Vec::with_capacity(content_len + gap_size);
            v.extend_from_slice(&content);
            v.resize(content_len + gap_size, 0);
            GapStorage::Heap(v)
        };

        // Copy content into Virtual storage (Heap was already filled above).
        if let GapStorage::Virtual(region) = &mut storage {
            // SAFETY: region.grow(content_len + gap_size) succeeded above.
            unsafe {
                std::ptr::copy_nonoverlapping(content.as_ptr(), region.as_ptr(), content_len);
            }
        }

        let mut buf = Buffer {
            storage,
            gap_start: content_len,
            gap_end: content_len + gap_size,
            text_len: content_len,
            line_cache: LineCache::new(),
            modified: false,
            file_path: Some(normalize_to_relative(path)),
            mmap: None,
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

    /// Returns `true` while the buffer content is backed by a memory-mapped
    /// file rather than the gap buffer.  Becomes `false` after the first edit.
    #[allow(dead_code)]
    pub fn is_mmap_backed(&self) -> bool {
        self.mmap.is_some()
    }

    // --- Text access ---

    pub fn len(&self) -> usize {
        if let Some(ref map) = self.mmap {
            map.len()
        } else {
            self.text_len
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get_line(&self, line: usize) -> Option<String> {
        let start = self.line_start(line)?;
        let end = if line + 1 < self.line_cache.total_lines {
            // Line ends just before the '\n' that starts the next line
            self.line_start(line + 1)? - 1
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

    /// Return the text of the line starting at `byte_offset` and the byte
    /// offset of the *next* line.  Runs in O(line_length) — no checkpoint
    /// scan — making it suitable for sequential line-by-line warmup loops.
    pub fn line_at_byte(&self, byte_offset: usize) -> Option<(String, usize)> {
        let tlen = self.len();
        if byte_offset >= tlen {
            return None;
        }
        let mut pos = byte_offset;
        while pos < tlen {
            if self.byte_at(pos) == Some(b'\n') {
                break;
            }
            pos += 1;
        }
        let text = self.slice(byte_offset, pos);
        let next = if pos < tlen { pos + 1 } else { tlen };
        Some((text, next))
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
        if self.mmap.is_some() {
            self.materialize_mmap();
        }
        let pos = pos.min(self.len());
        let bytes = text.as_bytes();
        self.ensure_gap(bytes.len());
        self.move_gap(pos);
        // SAFETY: ensure_gap guarantees gap_len >= bytes.len() bytes of committed storage.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.storage.as_raw_ptr().add(self.gap_start),
                bytes.len(),
            );
        }
        self.gap_start += bytes.len();
        self.text_len += bytes.len();
        self.modified = true;
        let nl_delta = bytes.iter().filter(|b| **b == b'\n').count() as isize;
        self.update_lines(pos, bytes.len() as isize, nl_delta);
    }

    pub fn delete(&mut self, pos: usize, len: usize) -> String {
        if self.mmap.is_some() {
            self.materialize_mmap();
        }
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
        self.text_len -= len;
        self.modified = true;
        let nl_delta = deleted.iter().filter(|b| **b == b'\n').count() as isize;
        self.update_lines(pos, -(len as isize), -nl_delta);
        String::from_utf8_lossy(&deleted).into_owned()
    }

    // --- Line info ---

    pub fn line_count(&self) -> usize {
        self.line_cache.total_lines
    }

    pub fn line_start(&self, line: usize) -> Option<usize> {
        let gs = self.gap_start;
        let ge = self.gap_end;
        // When mmap is Some, buf_scan_to_line uses the mmap path and never touches data.
        // Passing &[] avoids accessing uncommitted virtual memory.
        let data: &[u8] = if self.mmap.is_some() {
            &[]
        } else {
            // SAFETY: phys_cap == text_len + gap_len <= committed (invariant).
            self.storage.as_slice(self.text_len + (ge - gs))
        };
        let mmap = &self.mmap;
        let cache = &self.line_cache;
        cache.line_to_byte(line, |from_byte, from_line, to_line| {
            buf_scan_to_line(data, mmap, gs, ge, from_byte, from_line, to_line)
        })
    }

    pub fn line_end(&self, line: usize) -> Option<usize> {
        if line >= self.line_cache.total_lines {
            return None;
        }
        if line + 1 < self.line_cache.total_lines {
            Some(self.line_start(line + 1)? - 1)
        } else {
            Some(self.len())
        }
    }

    pub fn byte_to_line(&self, byte_pos: usize) -> usize {
        let gs = self.gap_start;
        let ge = self.gap_end;
        let data: &[u8] = if self.mmap.is_some() {
            &[]
        } else {
            // SAFETY: phys_cap == text_len + gap_len <= committed (invariant).
            self.storage.as_slice(self.text_len + (ge - gs))
        };
        let mmap = &self.mmap;
        let cache = &self.line_cache;
        cache.byte_to_line(byte_pos, |from_byte, to_byte| {
            buf_count_newlines(data, mmap, gs, ge, from_byte, to_byte)
        })
    }

    // --- Internal ---

    /// Materialize a mmap-backed buffer into the gap buffer.
    ///
    /// Copies the entire mmap content into `self.data`, releases the mmap,
    /// and positions the gap at the end of the content.  The line cache
    /// remains valid because logical byte offsets are unchanged.
    ///
    /// Must be called before any `insert`/`delete` on a mmap-backed buffer.
    fn materialize_mmap(&mut self) {
        let map = match self.mmap.take() {
            Some(m) => m,
            None => return,
        };
        let bytes = map.as_bytes();
        let content_len = bytes.len();
        let gap_size = INITIAL_GAP.max(content_len / 4);
        let total = content_len + gap_size;

        // Choose Heap vs Virtual for the materialized storage.
        let new_storage = if content_len >= crate::vmem::VMEM_THRESHOLD {
            match crate::vmem::VirtualRegion::reserve(crate::vmem::VMEM_RESERVE) {
                Ok(mut region) => {
                    if region.grow(total).is_err() {
                        // Fall back to heap on commit failure.
                        let mut v = Vec::with_capacity(total);
                        v.extend_from_slice(bytes);
                        v.resize(total, 0);
                        GapStorage::Heap(v)
                    } else {
                        // SAFETY: grow(total) succeeded; total bytes are committed.
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                bytes.as_ptr(),
                                region.as_ptr(),
                                content_len,
                            );
                        }
                        GapStorage::Virtual(region)
                    }
                }
                Err(_) => {
                    let mut v = Vec::with_capacity(total);
                    v.extend_from_slice(bytes);
                    v.resize(total, 0);
                    GapStorage::Heap(v)
                }
            }
        } else {
            let mut v = Vec::with_capacity(total);
            v.extend_from_slice(bytes);
            v.resize(total, 0);
            GapStorage::Heap(v)
        };

        self.storage = new_storage;
        self.gap_start = content_len;
        self.gap_end = total;
        self.text_len = content_len;
        // `self.line_cache` remains valid: logical byte offsets from the mmap
        // are identical to those in a gap-buffer with the gap at the end.
    }

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
        if let Some(ref map) = self.mmap {
            return map.as_bytes().get(pos).copied();
        }
        if pos >= self.text_len {
            return None;
        }
        let phys = self.logical_to_physical(pos);
        let phys_cap = self.text_len + self.gap_len();
        // SAFETY: phys < phys_cap <= committed (invariant).
        Some(self.storage.as_slice(phys_cap)[phys])
    }

    fn move_gap(&mut self, pos: usize) {
        let pos = pos.min(self.text_len);
        if pos == self.gap_start {
            return;
        }
        // SAFETY: all offsets are within committed storage; ptr::copy handles overlaps.
        let ptr = self.storage.as_raw_ptr();
        if pos < self.gap_start {
            let count = self.gap_start - pos;
            let src = pos;
            let dst = self.gap_end - count;
            unsafe {
                std::ptr::copy(ptr.add(src), ptr.add(dst), count);
            }
            self.gap_start = pos;
            self.gap_end = dst;
        } else {
            // pos > gap_start
            let count = pos - self.gap_start;
            let src = self.gap_end;
            let dst = self.gap_start;
            unsafe {
                std::ptr::copy(ptr.add(src), ptr.add(dst), count);
            }
            self.gap_start = pos;
            self.gap_end = src + count;
        }
    }

    fn ensure_gap(&mut self, needed: usize) {
        if self.gap_len() >= needed {
            return;
        }
        let grow = INITIAL_GAP.max(self.text_len / 4).max(needed);
        let after_gap = self.text_len - self.gap_start; // bytes of text after the gap
        let new_gap_end = self.gap_end + grow;
        let new_cap = new_gap_end + after_gap;

        match &mut self.storage {
            GapStorage::Heap(v) => {
                v.resize(new_cap, 0);
                if after_gap > 0 {
                    v.copy_within(self.gap_end..self.gap_end + after_gap, new_gap_end);
                }
            }
            GapStorage::Virtual(region) => {
                if region.grow(new_cap).is_err() {
                    return; // best-effort; gap stays at current size
                }
                if after_gap > 0 {
                    let ptr = region.as_ptr();
                    // SAFETY: grow(new_cap) succeeded; both regions are in committed memory.
                    // ptr::copy handles the potential overlap.
                    unsafe {
                        std::ptr::copy(ptr.add(self.gap_end), ptr.add(new_gap_end), after_gap);
                    }
                }
            }
        }
        self.gap_end = new_gap_end;
    }

    /// Full rebuild — called only on initial file load (gap is at the end).
    fn rebuild_lines(&mut self) {
        let phys_cap = self.text_len + self.gap_len();
        // SAFETY: phys_cap <= committed (invariant maintained by from_file / ensure_gap).
        let data = self.storage.as_slice(phys_cap);
        let seg1 = &data[..self.gap_start];
        let seg2 = &data[self.gap_end..];
        if seg2.is_empty() {
            self.line_cache.rebuild(seg1);
        } else {
            // Gap not at end — rebuild from both segments concatenated.
            let mut combined = Vec::with_capacity(seg1.len() + seg2.len());
            combined.extend_from_slice(seg1);
            combined.extend_from_slice(seg2);
            self.line_cache.rebuild(&combined);
        }
    }

    /// Incremental update after an insert or delete at logical byte position
    /// `edit_pos`.  `byte_delta` is positive for inserts, negative for deletes.
    /// `newlines_delta` is the net change in newline count.
    fn update_lines(&mut self, edit_pos: usize, byte_delta: isize, newlines_delta: isize) {
        self.line_cache.update(edit_pos, byte_delta, newlines_delta);
    }

    pub fn text_bytes(&self) -> Vec<u8> {
        if let Some(map) = &self.mmap {
            return map.as_bytes().to_vec();
        }
        let phys_cap = self.text_len + self.gap_len();
        // SAFETY: phys_cap <= committed (invariant).
        let data = self.storage.as_slice(phys_cap);
        let mut result = Vec::with_capacity(self.text_len);
        result.extend_from_slice(&data[..self.gap_start]);
        result.extend_from_slice(&data[self.gap_end..]);
        result
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

    /// Helper: collect all lines as a `Vec<String>` from a buffer.
    fn all_lines(buf: &Buffer) -> Vec<String> {
        (0..buf.line_count())
            .map(|i| buf.get_line(i).unwrap_or_default())
            .collect()
    }

    #[test]
    fn test_incremental_matches_full_rebuild_insert() {
        let mut buf = Buffer::new();
        buf.insert(0, "line1\nline2\nline3\n");
        buf.insert(6, "inserted\n");

        // Verify via public API: line count and content.
        assert_eq!(buf.line_count(), 5); // line1, inserted, line2, line3, ""
        assert_eq!(buf.get_line(0), Some("line1".into()));
        assert_eq!(buf.get_line(1), Some("inserted".into()));
        assert_eq!(buf.get_line(2), Some("line2".into()));
        assert_eq!(buf.get_line(3), Some("line3".into()));
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
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.get_line(0), Some("hello".into()));
        assert_eq!(buf.get_line(1), Some("world".into()));
        assert_eq!(buf.get_line(2), Some("".into()));
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
        assert_eq!(buf.text(), text);
        assert_eq!(buf.line_count(), 4);
        assert_eq!(all_lines(&buf), vec!["foo", "bar", "baz", ""]);
    }

    #[test]
    fn test_scan_newlines_word_alignment() {
        // Exercise various buffer lengths; line_count must match expected.
        for len in 0..64_usize {
            let text: String = (0..len)
                .map(|i| if i % 7 == 6 { '\n' } else { 'x' })
                .collect();
            let mut buf = Buffer::new();
            buf.insert(0, &text);

            let expected_nl = text.bytes().filter(|&b| b == b'\n').count();
            assert_eq!(
                buf.line_count(),
                expected_nl + 1,
                "mismatch for text length {len}: {:?}",
                text
            );
        }
    }

    // ── mmap integration tests ──────────────────────────────────────────────

    use std::sync::atomic::{AtomicU64, Ordering};
    static MMAP_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Write `content` to a uniquely-named temp file and return its path.
    fn write_tmp_file(content: &[u8]) -> std::path::PathBuf {
        let n = MMAP_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!("zedit_buf_mmap_{}_{}.bin", std::process::id(), n));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    #[test]
    fn test_small_file_uses_gap_buffer() {
        let path = write_tmp_file(b"hello\nworld\n");
        let buf = Buffer::from_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(!buf.is_mmap_backed(), "small file should use gap buffer");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.get_line(0), Some("hello".into()));
    }

    #[test]
    fn test_large_file_uses_mmap() {
        let content = vec![b'a'; MMAP_THRESHOLD + 1];
        let path = write_tmp_file(&content);
        let buf = Buffer::from_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(buf.is_mmap_backed(), "large file should be mmap-backed");
        assert_eq!(buf.len(), MMAP_THRESHOLD + 1);
    }

    #[test]
    fn test_mmap_byte_at() {
        let mut content = vec![b'x'; MMAP_THRESHOLD + 4];
        content[0] = b'H';
        content[MMAP_THRESHOLD] = b'Z';
        let path = write_tmp_file(&content);
        let buf = Buffer::from_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(buf.is_mmap_backed());
        assert_eq!(buf.byte_at(0), Some(b'H'));
        assert_eq!(buf.byte_at(MMAP_THRESHOLD), Some(b'Z'));
        assert_eq!(buf.byte_at(MMAP_THRESHOLD + 4), None); // out of bounds
    }

    #[test]
    fn test_mmap_line_count() {
        let mut content = vec![b'a'; MMAP_THRESHOLD + 10];
        content[100] = b'\n';
        content[200] = b'\n';
        content[300] = b'\n';
        let path = write_tmp_file(&content);
        let buf = Buffer::from_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(buf.is_mmap_backed());
        assert_eq!(buf.line_count(), 4); // 3 newlines → 4 lines
    }

    #[test]
    fn test_mmap_get_line() {
        // "abc\ndef\n" padded to > MMAP_THRESHOLD
        let header = b"abc\ndef\n";
        let mut content = Vec::with_capacity(MMAP_THRESHOLD + 10);
        content.extend_from_slice(header);
        content.resize(MMAP_THRESHOLD + 10, b'z');
        let path = write_tmp_file(&content);
        let buf = Buffer::from_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(buf.is_mmap_backed());
        assert_eq!(buf.get_line(0), Some("abc".into()));
        assert_eq!(buf.get_line(1), Some("def".into()));
    }

    #[test]
    fn test_mmap_materialize_on_insert() {
        let content = vec![b'a'; MMAP_THRESHOLD + 1];
        let path = write_tmp_file(&content);
        let mut buf = Buffer::from_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(buf.is_mmap_backed());

        buf.insert(0, "X");

        assert!(!buf.is_mmap_backed(), "should have materialized on insert");
        assert_eq!(buf.len(), MMAP_THRESHOLD + 2);
        assert_eq!(buf.byte_at(0), Some(b'X'));
        assert_eq!(buf.byte_at(1), Some(b'a'));
        assert!(buf.is_modified());
    }

    #[test]
    fn test_mmap_materialize_on_delete() {
        let content = vec![b'b'; MMAP_THRESHOLD + 5];
        let path = write_tmp_file(&content);
        let mut buf = Buffer::from_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(buf.is_mmap_backed());

        buf.delete(0, 3);

        assert!(!buf.is_mmap_backed(), "should have materialized on delete");
        assert_eq!(buf.len(), MMAP_THRESHOLD + 2);
        assert_eq!(buf.byte_at(0), Some(b'b'));
    }

    #[test]
    fn test_mmap_text_bytes_correct() {
        let mut content = vec![b'q'; MMAP_THRESHOLD + 1];
        content[0] = b'A';
        content[MMAP_THRESHOLD] = b'Z';
        let path = write_tmp_file(&content);
        let buf = Buffer::from_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(buf.is_mmap_backed());
        let bytes = buf.text_bytes();
        assert_eq!(bytes.len(), MMAP_THRESHOLD + 1);
        assert_eq!(bytes[0], b'A');
        assert_eq!(bytes[MMAP_THRESHOLD], b'Z');
    }

    // ── Phase 36: Virtual memory gap buffer ─────────────────────────────────

    impl Buffer {
        /// Create a new empty buffer backed by a `VirtualRegion` instead of a `Vec`.
        /// Only available in tests to exercise the Virtual storage path.
        #[cfg(test)]
        pub fn new_virtual() -> Buffer {
            let mut region = crate::vmem::VirtualRegion::reserve(crate::vmem::VMEM_RESERVE)
                .expect("vmem reserve failed in test");
            region.grow(INITIAL_GAP).expect("vmem grow failed in test");
            Buffer {
                storage: GapStorage::Virtual(region),
                gap_start: 0,
                gap_end: INITIAL_GAP,
                text_len: 0,
                line_cache: LineCache::new(),
                modified: false,
                file_path: None,
                mmap: None,
            }
        }
    }

    #[test]
    fn virtual_gap_buffer_basic_edit() {
        let mut buf = Buffer::new_virtual();
        buf.insert(0, "hello\nworld\n");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.get_line(0), Some("hello".into()));
        assert_eq!(buf.get_line(1), Some("world".into()));
        // Delete '\n' between "hello" and "world"
        buf.delete(5, 1);
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.get_line(0), Some("helloworld".into()));
    }

    #[test]
    fn virtual_gap_buffer_insert_delete_matches_heap() {
        let text = "alpha\nbeta\ngamma\ndelta\n";
        let mut heap = Buffer::new();
        let mut virt = Buffer::new_virtual();
        heap.insert(0, text);
        virt.insert(0, text);
        assert_eq!(heap.text(), virt.text());
        assert_eq!(heap.line_count(), virt.line_count());

        // Delete from the middle
        heap.delete(6, 5); // remove "beta\n"
        virt.delete(6, 5);
        assert_eq!(heap.text(), virt.text());
        assert_eq!(heap.line_count(), virt.line_count());

        // Insert at the start
        heap.insert(0, "prefix\n");
        virt.insert(0, "prefix\n");
        assert_eq!(heap.text(), virt.text());
        assert_eq!(heap.line_count(), virt.line_count());
    }

    #[test]
    fn virtual_gap_buffer_no_relocation() {
        // The VirtualRegion's base pointer must not change across multiple inserts.
        let mut buf = Buffer::new_virtual();
        let ptr_before = match &buf.storage {
            GapStorage::Virtual(r) => r.as_ptr(),
            _ => panic!("expected Virtual"),
        };
        // Force several gap grows
        for _ in 0..10 {
            buf.insert(buf.len(), &"x".repeat(512));
        }
        let ptr_after = match &buf.storage {
            GapStorage::Virtual(r) => r.as_ptr(),
            _ => panic!("expected Virtual"),
        };
        assert_eq!(ptr_before, ptr_after, "VirtualRegion must not move");
    }

    #[test]
    fn gap_storage_heap_used_for_small_files() {
        let buf = Buffer::new();
        assert!(matches!(buf.storage, GapStorage::Heap(_)));
    }

    // ── Phase 35: sparse LineCache tests ────────────────────────────────────

    /// Simple scan helper for LineCache unit tests (not performance-sensitive).
    fn cache_scan(
        data: &[u8],
        from_byte: usize,
        from_line: usize,
        to_line: usize,
    ) -> (usize, usize) {
        let need = to_line - from_line;
        let mut counted = 0usize;
        let mut pos = from_byte;
        while pos < data.len() && counted < need {
            if data[pos] == b'\n' {
                counted += 1;
            }
            pos += 1;
        }
        (pos, from_line + counted)
    }

    fn cache_count(data: &[u8], from_byte: usize, to_byte: usize) -> usize {
        data[from_byte.min(data.len())..to_byte.min(data.len())]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
    }

    #[test]
    fn sparse_cache_new_has_one_checkpoint() {
        let cache = LineCache::new();
        assert_eq!(cache.total_lines, 1);
        assert_eq!(cache.checkpoints.len(), 1);
        assert_eq!(cache.checkpoints[0].byte_off, 0);
        assert_eq!(cache.checkpoints[0].line, 0);
    }

    #[test]
    fn sparse_cache_rebuild_small() {
        let data = b"a\nb\nc\nd\n"; // 4 newlines → 5 lines (incl. trailing empty)
        let mut cache = LineCache::new();
        cache.rebuild(data);
        assert_eq!(cache.total_lines, 5);
        assert_eq!(cache.checkpoints.len(), 1); // < CHECKPOINT_STRIDE lines

        // line_to_byte
        assert_eq!(
            cache.line_to_byte(0, |b, l, t| cache_scan(data, b, l, t)),
            Some(0)
        );
        assert_eq!(
            cache.line_to_byte(1, |b, l, t| cache_scan(data, b, l, t)),
            Some(2)
        );
        assert_eq!(
            cache.line_to_byte(3, |b, l, t| cache_scan(data, b, l, t)),
            Some(6)
        );
        assert_eq!(
            cache.line_to_byte(4, |b, l, t| cache_scan(data, b, l, t)),
            Some(8)
        );
        assert_eq!(
            cache.line_to_byte(5, |b, l, t| cache_scan(data, b, l, t)),
            None
        );

        // byte_to_line
        assert_eq!(cache.byte_to_line(0, |f, t| cache_count(data, f, t)), 0);
        assert_eq!(cache.byte_to_line(1, |f, t| cache_count(data, f, t)), 0);
        assert_eq!(cache.byte_to_line(2, |f, t| cache_count(data, f, t)), 1);
        assert_eq!(cache.byte_to_line(6, |f, t| cache_count(data, f, t)), 3);
    }

    #[test]
    fn sparse_cache_large_file() {
        // 100 000 lines of "x\n" — verify checkpoint count
        let data: Vec<u8> = b"x\n".iter().copied().cycle().take(200_000).collect();
        let mut cache = LineCache::new();
        cache.rebuild(&data);
        assert_eq!(cache.total_lines, 100_001); // 100000 newlines + 1

        let expected_extra = 100_000 / CHECKPOINT_STRIDE; // checkpoints beyond origin
        assert_eq!(cache.checkpoints.len(), expected_extra + 1);
    }

    #[test]
    fn sparse_cache_update_insert_newlines() {
        let mut cache = LineCache::new();
        cache.total_lines = 3; // pretend 3 lines: 0, 5, 10

        // Insert 2 newlines at byte 3 (delta = +8 bytes, +2 lines)
        cache.update(3, 8, 2);
        assert_eq!(cache.total_lines, 5);
        // Origin checkpoint (byte 0) untouched — it's at <= 3
        assert_eq!(cache.checkpoints[0].byte_off, 0);
        assert_eq!(cache.checkpoints[0].line, 0);
    }

    #[test]
    fn sparse_cache_buffer_integration() {
        // Full buffer round-trip with the sparse cache.
        let mut buf = Buffer::new();
        buf.insert(0, "alpha\nbeta\ngamma\n");
        assert_eq!(buf.line_count(), 4);
        assert_eq!(buf.line_start(0), Some(0));
        assert_eq!(buf.line_start(1), Some(6)); // "alpha\n" = 6 bytes
        assert_eq!(buf.line_start(2), Some(11)); // "beta\n" = 5 bytes → 11
        assert_eq!(buf.line_start(3), Some(17)); // "gamma\n" = 6 bytes → 17
        assert_eq!(buf.line_start(4), None); // out of range

        assert_eq!(buf.byte_to_line(0), 0);
        assert_eq!(buf.byte_to_line(5), 0); // '\n' is still line 0
        assert_eq!(buf.byte_to_line(6), 1);
        assert_eq!(buf.byte_to_line(10), 1);
        assert_eq!(buf.byte_to_line(11), 2);
    }
}
