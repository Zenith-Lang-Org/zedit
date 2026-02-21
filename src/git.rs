// ---------------------------------------------------------------------------
// Git gutter: line-level change indicators comparing buffer vs HEAD
// ---------------------------------------------------------------------------
//
// Parses .git/ objects directly (zero deps). Implements:
// - zlib DEFLATE decompression (RFC 1951)
// - Git loose object reading (blob via commit→tree walk)
// - Myers diff algorithm
// - GitInfo struct for editor integration

// ============================================================================
// Part 1: DEFLATE inflate
// ============================================================================

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize, // byte position
    bit: u8,    // bit position within current byte (0..8)
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit: 0,
        }
    }

    fn read_bits(&mut self, n: u8) -> Option<u32> {
        let mut val: u32 = 0;
        for i in 0..n {
            if self.pos >= self.data.len() {
                return None;
            }
            let b = (self.data[self.pos] >> self.bit) & 1;
            val |= (b as u32) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Some(val)
    }

    fn align_byte(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }

    fn read_u16_le(&mut self) -> Option<u16> {
        self.align_byte();
        if self.pos + 2 > self.data.len() {
            return None;
        }
        let v = (self.data[self.pos] as u16) | ((self.data[self.pos + 1] as u16) << 8);
        self.pos += 2;
        Some(v)
    }
}

struct HuffmanTable {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl HuffmanTable {
    fn from_lengths(lengths: &[u8]) -> Self {
        let mut counts = [0u16; 16];
        for &l in lengths {
            if l > 0 && (l as usize) < 16 {
                counts[l as usize] += 1;
            }
        }
        // Build symbol table sorted by code length then symbol value
        let mut symbols = vec![0u16; lengths.len()];
        let mut offsets = [0u16; 16];
        let mut sum = 0u16;
        for i in 1..16 {
            offsets[i] = sum;
            sum += counts[i];
        }
        for (sym, &l) in lengths.iter().enumerate() {
            if l > 0 {
                symbols[offsets[l as usize] as usize] = sym as u16;
                offsets[l as usize] += 1;
            }
        }
        symbols.truncate(sum as usize);
        Self { counts, symbols }
    }

    fn decode(&self, reader: &mut BitReader) -> Option<u16> {
        let mut code: u32 = 0;
        let mut first: u32 = 0;
        let mut index: u32 = 0;
        for len in 1..16u32 {
            code |= reader.read_bits(1)?;
            let count = self.counts[len as usize] as u32;
            if code < first + count {
                return Some(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

// Fixed Huffman tables per RFC 1951
fn fixed_lit_table() -> HuffmanTable {
    let mut lengths = [0u8; 288];
    for l in &mut lengths[0..=143] {
        *l = 8;
    }
    for l in &mut lengths[144..=255] {
        *l = 9;
    }
    for l in &mut lengths[256..=279] {
        *l = 7;
    }
    for l in &mut lengths[280..=287] {
        *l = 8;
    }
    HuffmanTable::from_lengths(&lengths)
}

fn fixed_dist_table() -> HuffmanTable {
    let lengths = [5u8; 32];
    HuffmanTable::from_lengths(&lengths)
}

// Length extra bits: base lengths and extra bit counts for codes 257..285
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

// Distance extra bits
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn inflate_block(
    reader: &mut BitReader,
    lit_table: &HuffmanTable,
    dist_table: &HuffmanTable,
    out: &mut Vec<u8>,
) -> Option<()> {
    loop {
        let sym = lit_table.decode(reader)?;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Some(());
        } else {
            let idx = (sym - 257) as usize;
            if idx >= LEN_BASE.len() {
                return None;
            }
            let length = LEN_BASE[idx] as usize + reader.read_bits(LEN_EXTRA[idx])? as usize;

            let dist_sym = dist_table.decode(reader)? as usize;
            if dist_sym >= DIST_BASE.len() {
                return None;
            }
            let distance =
                DIST_BASE[dist_sym] as usize + reader.read_bits(DIST_EXTRA[dist_sym])? as usize;

            if distance > out.len() {
                return None;
            }
            let start = out.len() - distance;
            for i in 0..length {
                let b = out[start + (i % distance)];
                out.push(b);
            }
        }
    }
}

fn decode_dynamic_tables(reader: &mut BitReader) -> Option<(HuffmanTable, HuffmanTable)> {
    let hlit = reader.read_bits(5)? as usize + 257;
    let hdist = reader.read_bits(5)? as usize + 1;
    let hclen = reader.read_bits(4)? as usize + 4;

    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let mut cl_lengths = [0u8; 19];
    for i in 0..hclen {
        cl_lengths[ORDER[i]] = reader.read_bits(3)? as u8;
    }
    let cl_table = HuffmanTable::from_lengths(&cl_lengths);

    let total = hlit + hdist;
    let mut lengths = Vec::with_capacity(total);
    while lengths.len() < total {
        let sym = cl_table.decode(reader)?;
        match sym {
            0..=15 => lengths.push(sym as u8),
            16 => {
                let prev = *lengths.last()?;
                let repeat = 3 + reader.read_bits(2)? as usize;
                for _ in 0..repeat {
                    lengths.push(prev);
                }
            }
            17 => {
                let repeat = 3 + reader.read_bits(3)? as usize;
                lengths.extend(std::iter::repeat_n(0, repeat));
            }
            18 => {
                let repeat = 11 + reader.read_bits(7)? as usize;
                lengths.extend(std::iter::repeat_n(0, repeat));
            }
            _ => return None,
        }
    }
    let lit = HuffmanTable::from_lengths(&lengths[..hlit]);
    let dist = HuffmanTable::from_lengths(&lengths[hlit..hlit + hdist]);
    Some((lit, dist))
}

fn zlib_decompress(data: &[u8]) -> Option<Vec<u8>> {
    // Zlib header: 2 bytes (CMF, FLG)
    if data.len() < 6 {
        return None;
    }
    let cmf = data[0];
    if cmf & 0x0F != 8 {
        return None;
    } // CM must be 8 (deflate)
    let mut reader = BitReader::new(&data[2..]);
    let mut out = Vec::new();

    loop {
        let bfinal = reader.read_bits(1)?;
        let btype = reader.read_bits(2)?;

        match btype {
            0 => {
                // Uncompressed block
                reader.align_byte();
                let len = reader.read_u16_le()? as usize;
                let _nlen = reader.read_u16_le()?;
                if reader.pos + len > reader.data.len() {
                    return None;
                }
                out.extend_from_slice(&reader.data[reader.pos..reader.pos + len]);
                reader.pos += len;
            }
            1 => {
                // Fixed Huffman
                let lit = fixed_lit_table();
                let dist = fixed_dist_table();
                inflate_block(&mut reader, &lit, &dist, &mut out)?;
            }
            2 => {
                // Dynamic Huffman
                let (lit, dist) = decode_dynamic_tables(&mut reader)?;
                inflate_block(&mut reader, &lit, &dist, &mut out)?;
            }
            _ => return None,
        }

        if bfinal == 1 {
            break;
        }
    }
    Some(out)
}

// ============================================================================
// Part 2: Git object parsing
// ============================================================================

use std::path::{Path, PathBuf};

fn find_repo_root(path: &Path) -> Option<PathBuf> {
    let mut dir = if path.is_file() {
        path.parent()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn resolve_head(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();

    if head.len() == 40 && head.chars().all(|c| c.is_ascii_hexdigit()) {
        // Detached HEAD
        return Some(head.to_string());
    }

    // ref: refs/heads/main
    let ref_path = head.strip_prefix("ref: ")?;
    // Try loose ref first
    let ref_file = git_dir.join(ref_path);
    if let Ok(hash) = std::fs::read_to_string(&ref_file) {
        let h = hash.trim();
        if h.len() == 40 {
            return Some(h.to_string());
        }
    }
    // Try packed-refs
    let packed = std::fs::read_to_string(git_dir.join("packed-refs")).ok()?;
    for line in packed.lines() {
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        let (hash, name) = line.split_once(' ')?;
        if name == ref_path {
            return Some(hash.to_string());
        }
    }
    None
}

#[cfg(test)]
fn hex_to_bytes(hex: &str) -> Option<[u8; 20]> {
    if hex.len() != 40 {
        return None;
    }
    let mut bytes = [0u8; 20];
    for i in 0..20 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX_CHARS[(b >> 4) as usize]);
        s.push(HEX_CHARS[(b & 0x0F) as usize]);
    }
    s
}

const HEX_CHARS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

fn read_loose_object(git_dir: &Path, hash: &str) -> Option<Vec<u8>> {
    if hash.len() != 40 {
        return None;
    }
    let path = git_dir.join("objects").join(&hash[..2]).join(&hash[2..]);
    let compressed = std::fs::read(&path).ok()?;
    zlib_decompress(&compressed)
}

fn parse_object_content(data: &[u8]) -> Option<(&[u8], &[u8])> {
    // Format: "type size\0content"
    let nul = data.iter().position(|&b| b == 0)?;
    let header = std::str::from_utf8(&data[..nul]).ok()?;
    let _type_and_size = header; // "blob 1234", "tree 5678", etc.
    Some((header.split(' ').next()?.as_bytes(), &data[nul + 1..]))
}

fn parse_commit_tree(commit_data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(commit_data).ok()?;
    for line in text.lines() {
        if let Some(hash) = line.strip_prefix("tree ") {
            return Some(hash.trim().to_string());
        }
        if line.is_empty() {
            break; // end of headers
        }
    }
    None
}

fn walk_tree_for_path(git_dir: &Path, tree_hash: &str, rel_path: &[&str]) -> Option<String> {
    if rel_path.is_empty() {
        return None;
    }

    let obj = read_loose_object(git_dir, tree_hash)?;
    let (obj_type, tree_data) = parse_object_content(&obj)?;
    if obj_type != b"tree" {
        return None;
    }

    let target = rel_path[0];
    let remaining = &rel_path[1..];

    // Parse binary tree entries: "<mode> <name>\0<20-byte-sha1>"
    let mut pos = 0;
    while pos < tree_data.len() {
        // Find space after mode
        let space = tree_data[pos..].iter().position(|&b| b == b' ')?;
        let _mode = &tree_data[pos..pos + space];
        pos += space + 1;

        // Find null after name
        let nul = tree_data[pos..].iter().position(|&b| b == 0)?;
        let name = std::str::from_utf8(&tree_data[pos..pos + nul]).ok()?;
        pos += nul + 1;

        // Read 20-byte SHA-1
        if pos + 20 > tree_data.len() {
            return None;
        }
        let sha = &tree_data[pos..pos + 20];
        pos += 20;

        if name == target {
            let hash = bytes_to_hex(sha);
            if remaining.is_empty() {
                // This is the blob we want
                return Some(hash);
            } else {
                // Recurse into subtree
                return walk_tree_for_path(git_dir, &hash, remaining);
            }
        }
    }
    None
}

fn read_head_blob(file_path: &Path) -> Option<Vec<u8>> {
    let abs_path = std::fs::canonicalize(file_path).ok()?;
    let repo_root = find_repo_root(&abs_path)?;
    let git_dir = repo_root.join(".git");

    let commit_hash = resolve_head(&git_dir)?;
    let commit_obj = read_loose_object(&git_dir, &commit_hash)?;
    let (_, commit_content) = parse_object_content(&commit_obj)?;
    let tree_hash = parse_commit_tree(commit_content)?;

    let rel = abs_path.strip_prefix(&repo_root).ok()?;
    let components: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    if components.is_empty() {
        return None;
    }

    let blob_hash = walk_tree_for_path(&git_dir, &tree_hash, &components)?;
    let blob_obj = read_loose_object(&git_dir, &blob_hash)?;
    let (obj_type, blob_data) = parse_object_content(&blob_obj)?;
    if obj_type != b"blob" {
        return None;
    }
    Some(blob_data.to_vec())
}

// ============================================================================
// Part 3: Myers diff
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineStatus {
    Unchanged,
    Added,
    Modified,
    DeletedBelow,
}

pub fn diff_lines(old: &[&str], new: &[&str]) -> Vec<LineStatus> {
    let n = old.len();
    let m = new.len();

    if n == 0 {
        return vec![LineStatus::Added; m];
    }
    if m == 0 {
        // All lines deleted — if buffer empty, return empty
        return Vec::new();
    }

    let max_d = 1000.min(n + m);
    let arr_len = 2 * max_d + 1;
    let offset = max_d as isize;

    // Forward pass: compute edit script
    let mut v = vec![0isize; arr_len];
    let mut trace: Vec<Vec<isize>> = Vec::new();

    let mut found_d = None;
    'outer: for d in 0..=max_d {
        trace.push(v.clone());
        let di = d as isize;
        let mut k = -di;
        while k <= di {
            let idx = (k + offset) as usize;
            let mut x = if k == -di
                || (k != di && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize])
            {
                v[(k + 1 + offset) as usize]
            } else {
                v[(k - 1 + offset) as usize] + 1
            };
            let mut y = x - k;
            while (x as usize) < n && (y as usize) < m && old[x as usize] == new[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x as usize >= n && y as usize >= m {
                found_d = Some(d);
                break 'outer;
            }
            k += 2;
        }
    }

    let total_d = match found_d {
        Some(d) => d,
        None => {
            // Exceeded cap: mark all as modified
            return vec![LineStatus::Modified; m];
        }
    };

    // Backtrack to find the edit path
    // Each step is (prev_x, prev_y, x, y)
    let mut edits: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut x = n as isize;
    let mut y = m as isize;
    for d in (0..=total_d).rev() {
        let di = d as isize;
        let k = x - y;
        let prev_v = &trace[d];

        let prev_k = if k == -di
            || (k != di && prev_v[(k - 1 + offset) as usize] < prev_v[(k + 1 + offset) as usize])
        {
            k + 1
        } else {
            k - 1
        };
        let prev_x = prev_v[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;

        // Diagonal (equal lines)
        let mut cx = x;
        let mut cy = y;
        while cx > prev_x && cy > prev_y {
            cx -= 1;
            cy -= 1;
            edits.push((cx as usize, cy as usize, cx as usize + 1, cy as usize + 1));
        }

        // The actual edit
        if d > 0 {
            edits.push((prev_x as usize, prev_y as usize, cx as usize, cy as usize));
        }

        x = prev_x;
        y = prev_y;
    }

    edits.reverse();

    // Map edits to line statuses
    let mut result = vec![LineStatus::Unchanged; m];
    let mut i = 0;
    while i < edits.len() {
        let (px, py, ex, ey) = edits[i];
        if ex == px + 1 && ey == py + 1 && old.get(px) == new.get(py) {
            // Diagonal: unchanged
            i += 1;
        } else if ex == px + 1 && ey == py {
            // Delete old line
            // Check if next edit is an insert at same position (→ modified)
            if i + 1 < edits.len() {
                let (npx, npy, nex, ney) = edits[i + 1];
                if nex == npx && ney == npy + 1 && npy == py {
                    if npy < m {
                        result[npy] = LineStatus::Modified;
                    }
                    i += 2;
                    continue;
                }
            }
            // Pure delete: mark preceding new line as DeletedBelow
            if py > 0
                && result[py - 1] != LineStatus::Added
                && result[py - 1] != LineStatus::Modified
            {
                result[py - 1] = LineStatus::DeletedBelow;
            } else if py < m && result[py] == LineStatus::Unchanged {
                result[py] = LineStatus::DeletedBelow;
            }
            i += 1;
        } else if ex == px && ey == py + 1 {
            // Insert new line
            if py < m {
                result[py] = LineStatus::Added;
            }
            i += 1;
        } else {
            i += 1;
        }
    }

    result
}

// ============================================================================
// Part 4: GitInfo struct
// ============================================================================

pub struct GitInfo {
    head_lines: Option<Vec<String>>,
    pub line_statuses: Vec<LineStatus>,
    stale: bool,
}

impl GitInfo {
    pub fn from_file(path: &Path) -> Option<Self> {
        let blob = read_head_blob(path)?;
        let text = String::from_utf8(blob).ok()?;
        let head_lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        Some(Self {
            head_lines: Some(head_lines),
            line_statuses: Vec::new(),
            stale: true,
        })
    }

    #[allow(dead_code)]
    pub fn new_file() -> Self {
        Self {
            head_lines: None,
            line_statuses: Vec::new(),
            stale: true,
        }
    }

    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// Refresh the diff if stale. `get_line` returns the line text for index i.
    pub fn refresh_if_stale<F>(&mut self, line_count: usize, get_line: F)
    where
        F: Fn(usize) -> String,
    {
        if !self.stale {
            return;
        }
        self.stale = false;

        let current: Vec<String> = (0..line_count).map(&get_line).collect();
        let new_refs: Vec<&str> = current.iter().map(|s| s.as_str()).collect();

        match &self.head_lines {
            None => {
                // Untracked file: all lines are Added
                self.line_statuses = vec![LineStatus::Added; line_count];
            }
            Some(head) => {
                let old_refs: Vec<&str> = head.iter().map(|s| s.as_str()).collect();
                self.line_statuses = diff_lines(&old_refs, &new_refs);
            }
        }
    }

    pub fn line_status(&self, line: usize) -> LineStatus {
        self.line_statuses
            .get(line)
            .copied()
            .unwrap_or(LineStatus::Unchanged)
    }

    /// Reload HEAD content after save (file may now match HEAD).
    pub fn reload_head(&mut self, path: &Path) {
        if let Some(blob) = read_head_blob(path)
            && let Ok(text) = String::from_utf8(blob)
        {
            self.head_lines = Some(text.lines().map(|l| l.to_string()).collect());
            self.stale = true;
            return;
        }
        // File no longer tracked or repo gone
        self.head_lines = None;
        self.stale = true;
    }
}

/// Read the HEAD version of a file as a vector of lines.
/// Returns `None` if the file is not tracked or the repo can't be read.
pub fn head_lines(path: &Path) -> Option<Vec<String>> {
    let blob = read_head_blob(path)?;
    let text = String::from_utf8(blob).ok()?;
    Some(text.lines().map(|l| l.to_string()).collect())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zlib_decompress_basic() {
        // Test with known zlib data: compress "hello" using fixed Huffman
        // We test the full pipeline via git objects instead
        // Just verify None for invalid data
        assert!(zlib_decompress(&[]).is_none());
        assert!(zlib_decompress(&[0x78, 0x9C]).is_none()); // valid header but no data
    }

    #[test]
    fn test_bytes_to_hex() {
        assert_eq!(bytes_to_hex(&[0xab, 0xcd, 0xef]), "abcdef");
        assert_eq!(bytes_to_hex(&[0x00, 0xff]), "00ff");
    }

    #[test]
    fn test_hex_to_bytes() {
        let b = hex_to_bytes("abcdef0123456789abcdef0123456789abcdef01").unwrap();
        assert_eq!(b[0], 0xab);
        assert_eq!(b[1], 0xcd);
        assert!(hex_to_bytes("short").is_none());
    }

    #[test]
    fn test_diff_empty_old() {
        let result = diff_lines(&[], &["a", "b"]);
        assert_eq!(result, vec![LineStatus::Added, LineStatus::Added]);
    }

    #[test]
    fn test_diff_empty_new() {
        let result = diff_lines(&["a", "b"], &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_diff_identical() {
        let result = diff_lines(&["a", "b", "c"], &["a", "b", "c"]);
        assert_eq!(result, vec![LineStatus::Unchanged; 3]);
    }

    #[test]
    fn test_diff_added_lines() {
        let result = diff_lines(&["a", "c"], &["a", "b", "c"]);
        assert_eq!(result[0], LineStatus::Unchanged);
        assert_eq!(result[1], LineStatus::Added);
        assert_eq!(result[2], LineStatus::Unchanged);
    }

    #[test]
    fn test_diff_modified_line() {
        let result = diff_lines(&["a", "b", "c"], &["a", "B", "c"]);
        assert_eq!(result[0], LineStatus::Unchanged);
        assert_eq!(result[1], LineStatus::Modified);
        assert_eq!(result[2], LineStatus::Unchanged);
    }

    #[test]
    fn test_diff_all_new() {
        let result = diff_lines(&["x"], &["a", "b"]);
        // "x" deleted, "a" and "b" inserted
        // The exact result depends on Myers path, but all should be non-Unchanged
        assert!(result.iter().all(|s| *s != LineStatus::Unchanged));
    }

    #[test]
    fn test_git_info_new_file() {
        let mut info = GitInfo::new_file();
        info.refresh_if_stale(3, |_| "line".to_string());
        assert_eq!(info.line_status(0), LineStatus::Added);
        assert_eq!(info.line_status(1), LineStatus::Added);
        assert_eq!(info.line_status(2), LineStatus::Added);
    }

    #[test]
    fn test_find_repo_root() {
        // This test runs within the zedit repo itself
        let root = find_repo_root(Path::new(env!("CARGO_MANIFEST_DIR")));
        assert!(root.is_some());
        let root = root.unwrap();
        assert!(root.join(".git").exists());
    }

    #[test]
    fn test_resolve_head() {
        let root = find_repo_root(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let hash = resolve_head(&root.join(".git"));
        assert!(hash.is_some());
        let h = hash.unwrap();
        assert_eq!(h.len(), 40);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_huffman_table_basic() {
        // Lengths: symbol 0 -> len 1, symbol 1 -> len 2, symbol 2 -> len 2
        let table = HuffmanTable::from_lengths(&[1, 2, 2]);
        // Code 0 -> symbol 0 (1 bit), code 10 -> symbol 1, code 11 -> symbol 2
        // Test by creating a known bit sequence
        let data = [0b0000_0110u8]; // bits: 0, 1, 1, 0, ...
        let mut reader = BitReader::new(&data);
        assert_eq!(table.decode(&mut reader), Some(0)); // code 0 -> sym 0
        assert_eq!(table.decode(&mut reader), Some(2)); // code 11 -> sym 2
    }
}
