// ---------------------------------------------------------------------------
// Diff/Merge View — side-by-side file comparison with hunk navigation
// ---------------------------------------------------------------------------

use std::path::Path;

use crate::git;

// ---------------------------------------------------------------------------
// Row classification
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowKind {
    Equal,
    Added,
    Deleted,
    Modified,
}

// ---------------------------------------------------------------------------
// AlignRow — one visual row in the aligned diff display
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct AlignRow {
    /// Index into left buffer's lines (None if this row has no left content).
    pub left: Option<usize>,
    /// Index into right buffer's lines (None if this row has no right content).
    pub right: Option<usize>,
    pub kind: RowKind,
}

// ---------------------------------------------------------------------------
// Hunk — a group of consecutive non-Equal rows
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Hunk {
    pub row_start: usize,
    #[allow(dead_code)]
    pub row_count: usize,
}

// ---------------------------------------------------------------------------
// DiffBuffer — one side of the diff
// ---------------------------------------------------------------------------

pub struct DiffBuffer {
    pub lines: Vec<String>,
    #[allow(dead_code)]
    pub path: Option<std::path::PathBuf>,
    pub label: String,
}

// ---------------------------------------------------------------------------
// Edit script computation — LCS-based line alignment
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditOp {
    Equal,
    Insert,
    Delete,
}

/// Compute an aligned edit script between old and new line sets.
/// If either side exceeds 500 lines, falls back to pairing as Modified.
fn compute_edits(old: &[String], new: &[String]) -> Vec<AlignRow> {
    let n = old.len();
    let m = new.len();

    if n == 0 && m == 0 {
        return Vec::new();
    }

    // Cap to avoid O(n*m) explosion on large files
    if n > 500 || m > 500 {
        let pairs = n.min(m);
        let mut rows = Vec::new();
        for i in 0..pairs {
            rows.push(AlignRow {
                left: Some(i),
                right: Some(i),
                kind: RowKind::Modified,
            });
        }
        for i in pairs..n {
            rows.push(AlignRow {
                left: Some(i),
                right: None,
                kind: RowKind::Deleted,
            });
        }
        for j in pairs..m {
            rows.push(AlignRow {
                left: None,
                right: Some(j),
                kind: RowKind::Added,
            });
        }
        return rows;
    }

    // LCS DP on lines
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    // Backtrack
    let mut ops: Vec<EditOp> = Vec::new();
    let mut old_indices: Vec<Option<usize>> = Vec::new();
    let mut new_indices: Vec<Option<usize>> = Vec::new();

    let mut i = 0;
    let mut j = 0;
    while i < n || j < m {
        if i < n && j < m && old[i] == new[j] {
            ops.push(EditOp::Equal);
            old_indices.push(Some(i));
            new_indices.push(Some(j));
            i += 1;
            j += 1;
        } else if j < m && (i >= n || dp[i][j + 1] > dp[i + 1][j]) {
            // Strictly prefer Delete when tied, so Delete precedes Insert
            // (enabling Delete+Insert → Modified merging below)
            ops.push(EditOp::Insert);
            old_indices.push(None);
            new_indices.push(Some(j));
            j += 1;
        } else {
            ops.push(EditOp::Delete);
            old_indices.push(Some(i));
            new_indices.push(None);
            i += 1;
        }
    }

    // Convert ops to AlignRows, merging adjacent Delete+Insert into Modified
    let mut rows: Vec<AlignRow> = Vec::with_capacity(ops.len());
    let mut idx = 0;
    while idx < ops.len() {
        match ops[idx] {
            EditOp::Equal => {
                rows.push(AlignRow {
                    left: old_indices[idx],
                    right: new_indices[idx],
                    kind: RowKind::Equal,
                });
                idx += 1;
            }
            EditOp::Delete => {
                // Merge Delete+Insert into Modified pair
                if idx + 1 < ops.len() && ops[idx + 1] == EditOp::Insert {
                    rows.push(AlignRow {
                        left: old_indices[idx],
                        right: new_indices[idx + 1],
                        kind: RowKind::Modified,
                    });
                    idx += 2;
                } else {
                    rows.push(AlignRow {
                        left: old_indices[idx],
                        right: None,
                        kind: RowKind::Deleted,
                    });
                    idx += 1;
                }
            }
            EditOp::Insert => {
                rows.push(AlignRow {
                    left: None,
                    right: new_indices[idx],
                    kind: RowKind::Added,
                });
                idx += 1;
            }
        }
    }
    rows
}

// ---------------------------------------------------------------------------
// DiffView
// ---------------------------------------------------------------------------

pub struct DiffView {
    pub left: DiffBuffer,
    pub right: DiffBuffer,
    pub rows: Vec<AlignRow>,
    pub hunks: Vec<Hunk>,
    pub current_hunk: usize,
    pub scroll: usize,
    /// Set to Some(buf_idx) when this diff was opened from disk-change detection.
    /// Esc/q will restore the FileChangeNotice overlay instead of dismissing entirely.
    pub from_file_change: Option<usize>,
}

impl DiffView {
    /// Open a diff of the current buffer vs HEAD.
    pub fn open_vs_head(path: &Path, current_lines: Vec<String>) -> Option<Self> {
        let head_lines = git::head_lines(path)?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let left = DiffBuffer {
            lines: head_lines,
            path: Some(path.to_path_buf()),
            label: format!("{} (HEAD)", filename),
        };
        let right = DiffBuffer {
            lines: current_lines,
            path: Some(path.to_path_buf()),
            label: format!("{} (current)", filename),
        };
        Some(Self::build(left, right))
    }

    /// Open a diff comparing the current buffer contents against the on-disk file.
    pub fn open_vs_disk(path: &Path, buffer_lines: Vec<String>) -> Result<Self, String> {
        let disk_text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let disk_lines: Vec<String> = disk_text.lines().map(|l| l.to_string()).collect();
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let left = DiffBuffer {
            lines: disk_lines,
            path: Some(path.to_path_buf()),
            label: format!("{} (disk)", filename),
        };
        let right = DiffBuffer {
            lines: buffer_lines,
            path: Some(path.to_path_buf()),
            label: format!("{} (buffer)", filename),
        };
        Ok(Self::build(left, right))
    }

    fn build(left: DiffBuffer, right: DiffBuffer) -> Self {
        let rows = compute_edits(&left.lines, &right.lines);
        let hunks = Self::compute_hunks(&rows);
        let mut dv = DiffView {
            left,
            right,
            rows,
            hunks,
            current_hunk: 0,
            scroll: 0,
            from_file_change: None,
        };
        dv.jump_to_first_hunk();
        dv
    }

    pub fn compute_hunks(rows: &[AlignRow]) -> Vec<Hunk> {
        let mut hunks = Vec::new();
        let mut i = 0;
        while i < rows.len() {
            if rows[i].kind != RowKind::Equal {
                let start = i;
                while i < rows.len() && rows[i].kind != RowKind::Equal {
                    i += 1;
                }
                hunks.push(Hunk {
                    row_start: start,
                    row_count: i - start,
                });
            } else {
                i += 1;
            }
        }
        hunks
    }

    pub fn next_hunk(&mut self) {
        if self.hunks.is_empty() {
            return;
        }
        self.current_hunk = (self.current_hunk + 1) % self.hunks.len();
        self.scroll = self.hunks[self.current_hunk].row_start.saturating_sub(2);
    }

    pub fn prev_hunk(&mut self) {
        if self.hunks.is_empty() {
            return;
        }
        if self.current_hunk == 0 {
            self.current_hunk = self.hunks.len() - 1;
        } else {
            self.current_hunk -= 1;
        }
        self.scroll = self.hunks[self.current_hunk].row_start.saturating_sub(2);
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize) {
        let max_scroll = self.rows.len().saturating_sub(1);
        self.scroll = (self.scroll + n).min(max_scroll);
    }

    /// Jump scroll to the first hunk (if any).
    pub fn jump_to_first_hunk(&mut self) {
        if !self.hunks.is_empty() {
            self.current_hunk = 0;
            self.scroll = self.hunks[0].row_start.saturating_sub(2);
        }
    }

    /// Total number of aligned rows.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_edits_identical() {
        let a = vec!["a".to_string(), "b".to_string()];
        let rows = compute_edits(&a, &a);
        assert!(rows.iter().all(|r| r.kind == RowKind::Equal));
    }

    #[test]
    fn test_compute_edits_added() {
        let old = vec!["a".to_string()];
        let new = vec!["a".to_string(), "b".to_string()];
        let rows = compute_edits(&old, &new);
        assert_eq!(rows[0].kind, RowKind::Equal);
        assert_eq!(rows[1].kind, RowKind::Added);
    }

    #[test]
    fn test_compute_edits_deleted() {
        let old = vec!["a".to_string(), "b".to_string()];
        let new = vec!["a".to_string()];
        let rows = compute_edits(&old, &new);
        assert_eq!(rows[0].kind, RowKind::Equal);
        assert_eq!(rows[1].kind, RowKind::Deleted);
    }

    #[test]
    fn test_compute_edits_modified() {
        let old = vec!["hello".to_string()];
        let new = vec!["world".to_string()];
        let rows = compute_edits(&old, &new);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, RowKind::Modified);
    }

    #[test]
    fn test_compute_edits_empty() {
        let rows = compute_edits(&[], &[]);
        assert!(rows.is_empty());
    }

    #[test]
    fn test_compute_hunks_empty() {
        let rows: Vec<AlignRow> = Vec::new();
        let hunks = DiffView::compute_hunks(&rows);
        assert!(hunks.is_empty());
    }

    #[test]
    fn test_compute_hunks_all_equal() {
        let rows = vec![
            AlignRow {
                left: Some(0),
                right: Some(0),
                kind: RowKind::Equal,
            },
            AlignRow {
                left: Some(1),
                right: Some(1),
                kind: RowKind::Equal,
            },
        ];
        let hunks = DiffView::compute_hunks(&rows);
        assert!(hunks.is_empty());
    }

    #[test]
    fn test_compute_hunks_one_hunk() {
        let rows = vec![
            AlignRow {
                left: Some(0),
                right: Some(0),
                kind: RowKind::Equal,
            },
            AlignRow {
                left: Some(1),
                right: None,
                kind: RowKind::Deleted,
            },
            AlignRow {
                left: Some(2),
                right: Some(1),
                kind: RowKind::Equal,
            },
        ];
        let hunks = DiffView::compute_hunks(&rows);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].row_start, 1);
        assert_eq!(hunks[0].row_count, 1);
    }

    #[test]
    fn test_next_prev_hunk_wraps() {
        let old = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let new = vec!["a".to_string(), "X".to_string(), "c".to_string()];
        let rows = compute_edits(&old, &new);
        let hunks = DiffView::compute_hunks(&rows);
        let mut dv = DiffView {
            left: DiffBuffer {
                lines: old,
                path: None,
                label: "left".to_string(),
            },
            right: DiffBuffer {
                lines: new,
                path: None,
                label: "right".to_string(),
            },
            rows,
            hunks,
            current_hunk: 0,
            scroll: 0,
            from_file_change: None,
        };
        assert_eq!(dv.hunks.len(), 1);
        dv.next_hunk();
        assert_eq!(dv.current_hunk, 0); // wraps back
        dv.prev_hunk();
        assert_eq!(dv.current_hunk, 0); // still 0 with single hunk
    }
}
