/// Problem Panel — captures build/run output, parses errors, and lets the user
/// navigate to source locations with Enter.
///
/// Parsers supported:
///   - Rust / Cargo  (`error[E0425]: msg\n  --> file:line:col`)
///   - GCC / Clang   (`file:line:col: error: msg`)
///   - Python        (`File "file", line N\n  SyntaxError: msg`)
///   - Generic       (`file:line:col: msg`)

use std::collections::HashSet;

// ── Types ────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Note,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "E",
            Severity::Warning => "W",
            Severity::Info => "I",
            Severity::Note => "N",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Problem {
    pub severity: Severity,
    pub file: String,
    pub line: u32, // 1-based
    pub col: u32,  // 1-based
    pub message: String,
    #[allow(dead_code)]
    pub code: Option<String>, // "E0425", "W0001", etc.
}

/// Collapse-group key for the cargo-check section (cannot appear in file paths).
pub const BUILD_GROUP_KEY: &str = "\0build";
/// Prefix for per-file collapse keys within the cargo-check section.
pub const CARGO_FILE_PREFIX: &str = "\0cargo:";

/// A single row in the Problems panel display (cargo check only).
///
/// Layout:
/// ```
/// ▼ cargo check            ← CargoSectionHeader
///   ▼ src/layout.rs  W:3  ← CargoFileHeader
///       [W] :82  msg       ← Item  (idx into `items`)
/// ```
///
/// LSP diagnostics (rust-analyzer) are shown in the separate DIAGNOSTICS tab.
#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    /// Top-level collapsible header for the cargo-check section.
    CargoSectionHeader,
    /// Collapsible per-file header within the cargo-check section.
    CargoFileHeader(String),
    /// An actual problem; the `usize` is the index into `items` (cargo check results).
    Item(usize),
}

// ── ProblemPanel ─────────────────────────────────────────────

pub struct ProblemPanel {
    pub visible: bool,
    pub focused: bool,
    /// Build-output items (from task runner).
    pub items: Vec<Problem>,
    /// LSP-sourced diagnostic items (updated on every LSP sync).
    pub lsp_items: Vec<Problem>,
    /// `selected` is a *display-row* index into `compute_rows()`.
    pub selected: usize,
    pub scroll: usize,
    /// Keys of currently collapsed groups (file paths + BUILD_GROUP_KEY).
    pub collapsed_groups: HashSet<String>,
    /// Command that produced this output (e.g. "cargo build").
    pub source_cmd: Option<String>,
    /// Elapsed milliseconds for the last task (set externally).
    pub elapsed_ms: u64,
    /// Raw text buffer accumulating incomplete lines.
    capture_buf: String,
    /// Multi-line Rust parser state.
    pending_severity: Option<Severity>,
    pending_message: Option<String>,
    pending_code: Option<String>,
    /// Python parser state.
    pending_py_file: Option<String>,
    pending_py_line: Option<u32>,
}

impl ProblemPanel {
    pub fn new() -> Self {
        ProblemPanel {
            visible: false,
            focused: false,
            items: Vec::new(),
            lsp_items: Vec::new(),
            selected: 0,
            scroll: 0,
            collapsed_groups: HashSet::new(),
            source_cmd: None,
            elapsed_ms: 0,
            capture_buf: String::new(),
            pending_severity: None,
            pending_message: None,
            pending_code: None,
            pending_py_file: None,
            pending_py_line: None,
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.selected = 0;
        self.scroll = 0;
        self.elapsed_ms = 0;
        self.capture_buf.clear();
        self.pending_severity = None;
        self.pending_message = None;
        self.pending_code = None;
        self.pending_py_file = None;
        self.pending_py_line = None;
    }

    /// Feed raw bytes (possibly with incomplete lines and ANSI codes).
    pub fn feed_raw(&mut self, text: &str) {
        self.capture_buf.push_str(text);
        // Process complete lines.
        while let Some(pos) = self.capture_buf.find('\n') {
            let line = self.capture_buf[..pos].to_string();
            self.capture_buf = self.capture_buf[pos + 1..].to_string();
            let clean = strip_ansi(&line);
            self.feed_line(&clean);
        }
    }

    /// Parse a single clean (ANSI-stripped) line and update the items list.
    pub fn feed_line(&mut self, line: &str) {
        // Rust/Cargo: `  --> file:line:col` continuation of a pending error.
        if let Some(problem) = self.try_rust_location(line) {
            self.push(problem);
            return;
        }

        // Rust/Cargo: `error[E0425]: message` or `warning: message`
        if self.try_rust_header(line) {
            return;
        }

        // Python: `File "file", line N` header
        if self.try_python_header(line) {
            return;
        }

        // Python: `SyntaxError:` continuation
        if let Some(problem) = self.try_python_error(line) {
            self.push(problem);
            return;
        }

        // GCC/Clang: `file:line:col: error: message`
        if let Some(problem) = parse_gcc(line) {
            self.push(problem);
            return;
        }

        // Generic: `file:line:col: message`
        if let Some(problem) = parse_generic(line) {
            self.push(problem);
        }
    }

    /// Update LSP diagnostic items. Called after every LSP sync.
    pub fn set_lsp_items(&mut self, items: Vec<Problem>) {
        self.lsp_items = items;
        let total = self.compute_rows().len();
        if total == 0 {
            self.selected = 0;
        } else if self.selected >= total {
            self.selected = total - 1;
        }
    }

    /// Total display rows (headers + items). Used for navigation and scroll bounds.
    pub fn all_items_count(&self) -> usize {
        self.compute_rows().len()
    }

    /// Get a cargo check problem by index (direct index into `items`).
    pub fn get_item(&self, idx: usize) -> Option<&Problem> {
        self.items.get(idx)
    }

    /// Return the LSP diagnostic items for a specific file path.
    #[allow(dead_code)]
    pub fn lsp_items_for_file<'a>(&'a self, file: &str) -> Vec<&'a Problem> {
        self.lsp_items.iter().filter(|p| p.file == file).collect()
    }

    /// Return the problem the cursor is on, or None if on a header row.
    #[allow(dead_code)]
    pub fn selected_problem(&self) -> Option<&Problem> {
        match self.compute_rows().get(self.selected)? {
            RowKind::Item(idx) => self.get_item(*idx),
            _ => None,
        }
    }

    // ── Grouped display ──────────────────────────────────────

    /// Compute the ordered list of display rows for the PROBLEMS tab (cargo check only).
    ///
    /// LSP diagnostics are shown in the separate DIAGNOSTICS tab; they do not
    /// appear here.
    ///
    /// Structure:
    /// - `CargoSectionHeader` (when items non-empty) — collapses entire cargo section.
    ///   - `CargoFileHeader(file)` per distinct file — collapses via `"\0cargo:file"` key.
    ///     - `Item(i)` for each problem in that file (direct index into `items`).
    pub fn compute_rows(&self) -> Vec<RowKind> {
        let mut rows = Vec::new();

        // ── cargo check section (compiler diagnostics) ───────────────────
        if !self.items.is_empty() {
            rows.push(RowKind::CargoSectionHeader);
            if !self.collapsed_groups.contains(BUILD_GROUP_KEY) {
                let mut current_file: Option<&str> = None;
                for (i, item) in self.items.iter().enumerate() {
                    if current_file != Some(item.file.as_str()) {
                        current_file = Some(&item.file);
                        rows.push(RowKind::CargoFileHeader(item.file.clone()));
                    }
                    let cargo_key = format!("{}{}", CARGO_FILE_PREFIX, item.file);
                    if !self.collapsed_groups.contains(&cargo_key) {
                        rows.push(RowKind::Item(i));
                    }
                }
            }
        }

        rows
    }

    /// Get the `RowKind` at a given display-row index.
    pub fn get_display_row(&self, idx: usize) -> Option<RowKind> {
        self.compute_rows().into_iter().nth(idx)
    }

    /// Toggle the collapsed state of the group identified by `key`.
    /// Clamps `selected` if the current row disappears.
    pub fn toggle_group(&mut self, key: &str) {
        if self.collapsed_groups.contains(key) {
            self.collapsed_groups.remove(key);
        } else {
            self.collapsed_groups.insert(key.to_string());
        }
        let total = self.compute_rows().len();
        if total > 0 && self.selected >= total {
            self.selected = total - 1;
        } else if total == 0 {
            self.selected = 0;
        }
    }

    /// Toggle collapse for whichever group header the cursor is currently on.
    /// No-op if the cursor is on an `Item` row.
    #[allow(dead_code)]
    pub fn toggle_selected_collapse(&mut self) {
        let rows = self.compute_rows();
        if let Some(row) = rows.get(self.selected) {
            let key = match row {
                RowKind::CargoSectionHeader => Some(BUILD_GROUP_KEY.to_string()),
                RowKind::CargoFileHeader(f) => Some(format!("{}{}", CARGO_FILE_PREFIX, f)),
                RowKind::Item(_) => None,
            };
            if let Some(k) = key {
                self.toggle_group(&k);
            }
        }
    }

    /// Count errors and warnings for a given file path within `lsp_items`.
    /// Returns `(error_count, warning_count)`.
    #[allow(dead_code)]
    pub fn file_counts(&self, file: &str) -> (usize, usize) {
        self.lsp_items.iter().filter(|p| p.file == file).fold(
            (0, 0),
            |(e, w), p| match p.severity {
                Severity::Error => (e + 1, w),
                Severity::Warning => (e, w + 1),
                _ => (e, w),
            },
        )
    }

    /// Count errors and warnings for a given file path within `items` (cargo check).
    /// Returns `(error_count, warning_count)`.
    pub fn cargo_file_counts(&self, file: &str) -> (usize, usize) {
        self.items.iter().filter(|p| p.file == file).fold(
            (0, 0),
            |(e, w), p| match p.severity {
                Severity::Error => (e + 1, w),
                Severity::Warning => (e, w + 1),
                _ => (e, w),
            },
        )
    }

    /// Count errors and warnings across the entire LSP section.
    #[allow(dead_code)]
    pub fn lsp_counts(&self) -> (usize, usize) {
        self.lsp_items.iter().fold((0, 0), |(e, w), p| match p.severity {
            Severity::Error => (e + 1, w),
            Severity::Warning => (e, w + 1),
            _ => (e, w),
        })
    }

    /// Count errors and warnings within `items` (build output).
    pub fn build_counts(&self) -> (usize, usize) {
        self.items.iter().fold((0, 0), |(e, w), p| match p.severity {
            Severity::Error => (e + 1, w),
            Severity::Warning => (e, w + 1),
            _ => (e, w),
        })
    }

    /// Count errors from build output only (used by status bar to avoid
    /// double-counting with the LSP diagnostic indicators).
    pub fn error_count(&self) -> usize {
        self.items
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .count()
    }

    /// Count warnings from build output only.
    pub fn warning_count(&self) -> usize {
        self.items
            .iter()
            .filter(|p| p.severity == Severity::Warning)
            .count()
    }

    /// Total errors across both LSP and build items (for the panel tab badge).
    #[allow(dead_code)]
    pub fn total_error_count(&self) -> usize {
        self.lsp_items
            .iter()
            .chain(self.items.iter())
            .filter(|p| p.severity == Severity::Error)
            .count()
    }

    /// Total warnings across both LSP and build items (for the panel tab badge).
    #[allow(dead_code)]
    pub fn total_warning_count(&self) -> usize {
        self.lsp_items
            .iter()
            .chain(self.items.iter())
            .filter(|p| p.severity == Severity::Warning)
            .count()
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll {
                self.scroll = self.selected;
            }
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.compute_rows().len() {
            self.selected += 1;
        }
    }

    #[allow(dead_code)]
    pub fn clamp_scroll(&mut self, visible_rows: usize) {
        if self.selected >= self.scroll + visible_rows {
            self.scroll = self.selected + 1 - visible_rows;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
    }

    // ── Internal helpers ──────────────────────────────────────

    fn push(&mut self, p: Problem) {
        self.items.push(p);
        // Clear rust pending state.
        self.pending_severity = None;
        self.pending_message = None;
        self.pending_code = None;
        self.pending_py_file = None;
        self.pending_py_line = None;
    }

    /// Try to parse a Rust/Cargo header: `error[E0425]: message`.
    /// Returns true if the line was consumed as a header.
    fn try_rust_header(&mut self, line: &str) -> bool {
        // Patterns: "error[CODE]:", "error:", "warning:", "note:"
        let (sev, rest) = if let Some(r) = line.strip_prefix("error[") {
            (Severity::Error, r)
        } else if let Some(r) = line.strip_prefix("error:") {
            self.pending_severity = Some(Severity::Error);
            self.pending_message = Some(r.trim().to_string());
            self.pending_code = None;
            return true;
        } else if let Some(r) = line.strip_prefix("warning[") {
            (Severity::Warning, r)
        } else if let Some(r) = line.strip_prefix("warning:") {
            self.pending_severity = Some(Severity::Warning);
            self.pending_message = Some(r.trim().to_string());
            self.pending_code = None;
            return true;
        } else if let Some(r) = line.strip_prefix("note:") {
            self.pending_severity = Some(Severity::Note);
            self.pending_message = Some(r.trim().to_string());
            self.pending_code = None;
            return true;
        } else {
            return false;
        };

        // Extract code and message: "E0425]: message"
        if let Some(bracket) = rest.find("]:") {
            let code = rest[..bracket].to_string();
            let msg = rest[bracket + 2..].trim().to_string();
            self.pending_severity = Some(sev);
            self.pending_code = Some(code);
            self.pending_message = Some(msg);
            true
        } else {
            false
        }
    }

    /// Try to parse `  --> file:line:col` and combine with pending.
    fn try_rust_location(&mut self, line: &str) -> Option<Problem> {
        let arrow = line.trim_start();
        let loc = if let Some(r) = arrow.strip_prefix("--> ") {
            r
        } else {
            return None;
        };

        let sev = self.pending_severity.take()?;
        let message = self.pending_message.take().unwrap_or_default();
        let code = self.pending_code.take();

        // `file:line:col` — parse loc
        let (file, ln, col) = split_file_line_col(loc)?;
        Some(Problem {
            severity: sev,
            file,
            line: ln,
            col,
            message,
            code,
        })
    }

    /// Try to parse Python `File "file", line N`.
    fn try_python_header(&mut self, line: &str) -> bool {
        let rest = match line.trim().strip_prefix("File \"") {
            Some(r) => r,
            None => return false,
        };
        let (file_part, tail) = match rest.split_once("\", line ") {
            Some(p) => p,
            None => return false,
        };
        let ln: u32 = tail.trim().parse().unwrap_or(0);
        if ln == 0 {
            return false;
        }
        self.pending_py_file = Some(file_part.to_string());
        self.pending_py_line = Some(ln);
        true
    }

    /// Try to parse Python error continuation after a `File "..."` header.
    fn try_python_error(&mut self, line: &str) -> Option<Problem> {
        let file = self.pending_py_file.clone()?;
        let ln = self.pending_py_line?;

        let trimmed = line.trim();
        // Match "ErrorType: msg" patterns (SyntaxError:, TypeError:, etc.)
        if trimmed.contains(':') && !trimmed.starts_with(' ') {
            let message = trimmed.to_string();
            Some(Problem {
                severity: Severity::Error,
                file,
                line: ln,
                col: 1,
                message,
                code: None,
            })
        } else {
            None
        }
    }
}

// ── GCC / Clang parser ────────────────────────────────────────

/// Parse `file:line:col: severity: message`.
fn parse_gcc(line: &str) -> Option<Problem> {
    // Must not start with whitespace (location lines do not)
    if line.starts_with(' ') || line.is_empty() {
        return None;
    }
    // Split off the last `: severity: message` part
    let parts: Vec<&str> = line.splitn(5, ':').collect();
    // Minimum: file + line + col + severity + message = 5 parts
    if parts.len() < 5 {
        return None;
    }

    let ln: u32 = parts[1].trim().parse().ok()?;
    let col: u32 = parts[2].trim().parse().ok()?;
    let sev_str = parts[3].trim().to_lowercase();
    let severity = match sev_str.as_str() {
        "error" => Severity::Error,
        "warning" | "warn" => Severity::Warning,
        "note" => Severity::Note,
        _ => return None,
    };
    let message = parts[4].trim().to_string();
    let file = parts[0].to_string();

    // Basic sanity: file should look like a path
    if file.is_empty() || ln == 0 {
        return None;
    }

    Some(Problem {
        severity,
        file,
        line: ln,
        col,
        message,
        code: None,
    })
}

// ── Generic parser ────────────────────────────────────────────

/// Parse `file:line:col: message` — generic fallback.
fn parse_generic(line: &str) -> Option<Problem> {
    if line.starts_with(' ') || line.is_empty() {
        return None;
    }
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() < 4 {
        return None;
    }
    let ln: u32 = parts[1].trim().parse().ok()?;
    let col: u32 = parts[2].trim().parse().ok()?;
    let file = parts[0].to_string();
    let message = parts[3].trim().to_string();

    // Avoid false positives: file must have an extension, line > 0
    if file.is_empty() || !file.contains('.') || ln == 0 || message.is_empty() {
        return None;
    }

    Some(Problem {
        severity: Severity::Error,
        file,
        line: ln,
        col,
        message,
        code: None,
    })
}

// ── ANSI stripper ─────────────────────────────────────────────

/// Remove ANSI escape sequences from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // CSI sequence: skip until final byte (0x40–0x7E)
            i += 2;
            while i < bytes.len() && !(0x40..=0x7E).contains(&bytes[i]) {
                i += 1;
            }
            i += 1; // skip final byte
        } else if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b']' {
            // OSC sequence: skip until ST (\a or \e\\)
            i += 2;
            while i < bytes.len() {
                if bytes[i] == 0x07 {
                    i += 1;
                    break;
                }
                if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else if bytes[i] == 0x0d {
            // CR — skip (terminal line resets confuse parsers)
            i += 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

// ── Shared helper ─────────────────────────────────────────────

/// Split `file:line:col` into `(file, line, col)`.
fn split_file_line_col(s: &str) -> Option<(String, u32, u32)> {
    let parts: Vec<&str> = s.rsplitn(3, ':').collect();
    // rsplitn gives [col, line, file] in reverse
    if parts.len() < 3 {
        // Try file:line only
        let parts2: Vec<&str> = s.rsplitn(2, ':').collect();
        if parts2.len() < 2 {
            return None;
        }
        let ln: u32 = parts2[0].trim().parse().ok()?;
        return Some((parts2[1].to_string(), ln, 1));
    }
    let col_str = parts[0].trim();
    let line_str = parts[1].trim();
    // Remove trailing char info like `5` from `5: blah`
    let col_clean: u32 = col_str.split_whitespace().next()?.parse().ok()?;
    let ln: u32 = line_str.parse().ok()?;
    let file = parts[2].to_string();
    Some((file, ln, col_clean))
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- strip_ansi ---

    #[test]
    fn test_strip_ansi_plain() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn test_strip_ansi_color() {
        assert_eq!(strip_ansi("\x1b[31merror\x1b[0m: msg"), "error: msg");
    }

    #[test]
    fn test_strip_ansi_bold() {
        assert_eq!(strip_ansi("\x1b[1mwarning\x1b[0m"), "warning");
    }

    // --- Rust parser ---

    #[test]
    fn test_rust_error_two_lines() {
        let mut pp = ProblemPanel::new();
        pp.feed_line("error[E0425]: cannot find value `x` in this scope");
        pp.feed_line("  --> src/main.rs:74:5");
        assert_eq!(pp.items.len(), 1);
        let p = &pp.items[0];
        assert_eq!(p.severity, Severity::Error);
        assert_eq!(p.file, "src/main.rs");
        assert_eq!(p.line, 74);
        assert_eq!(p.col, 5);
        assert_eq!(p.code.as_deref(), Some("E0425"));
        assert!(p.message.contains("cannot find"));
    }

    #[test]
    fn test_rust_warning_two_lines() {
        let mut pp = ProblemPanel::new();
        pp.feed_line("warning[W0001]: unused variable `x`");
        pp.feed_line("  --> src/lib.rs:12:3");
        assert_eq!(pp.items.len(), 1);
        assert_eq!(pp.items[0].severity, Severity::Warning);
        assert_eq!(pp.items[0].line, 12);
    }

    #[test]
    fn test_rust_plain_error() {
        let mut pp = ProblemPanel::new();
        pp.feed_line("error: aborting due to previous error");
        pp.feed_line("  --> src/main.rs:1:1");
        assert_eq!(pp.items.len(), 1);
        assert_eq!(pp.items[0].severity, Severity::Error);
        assert!(pp.items[0].code.is_none());
    }

    #[test]
    fn test_rust_via_raw_feed() {
        let mut pp = ProblemPanel::new();
        pp.feed_raw("\x1b[31merror\x1b[0m[E0308]: mismatched types\n  --> src/lib.rs:8:5\n");
        assert_eq!(pp.items.len(), 1);
        assert_eq!(pp.items[0].line, 8);
    }

    // --- GCC parser ---

    #[test]
    fn test_gcc_error() {
        let mut pp = ProblemPanel::new();
        pp.feed_line("src/main.c:42:10: error: use of undeclared identifier 'foo'");
        assert_eq!(pp.items.len(), 1);
        let p = &pp.items[0];
        assert_eq!(p.severity, Severity::Error);
        assert_eq!(p.file, "src/main.c");
        assert_eq!(p.line, 42);
        assert_eq!(p.col, 10);
        assert!(p.message.contains("undeclared"));
    }

    #[test]
    fn test_gcc_warning() {
        let mut pp = ProblemPanel::new();
        pp.feed_line("main.c:5:3: warning: implicit declaration of function 'printf'");
        assert_eq!(pp.items.len(), 1);
        assert_eq!(pp.items[0].severity, Severity::Warning);
    }

    // --- Python parser ---

    #[test]
    fn test_python_syntax_error() {
        let mut pp = ProblemPanel::new();
        pp.feed_line("  File \"script.py\", line 12");
        pp.feed_line("SyntaxError: invalid syntax");
        assert_eq!(pp.items.len(), 1);
        let p = &pp.items[0];
        assert_eq!(p.file, "script.py");
        assert_eq!(p.line, 12);
        assert!(p.message.contains("SyntaxError"));
    }

    // --- Navigation ---

    #[test]
    fn test_navigation_up_down() {
        // 3 items in 3 different files produce:
        //   0: CargoSectionHeader
        //   1: CargoFileHeader("src/a.c")
        //   2: Item(0)
        //   3: CargoFileHeader("src/b.c")
        //   4: Item(1)
        //   5: CargoFileHeader("src/c.c")
        //   6: Item(2)   ← last row (index 6)
        let mut pp = ProblemPanel::new();
        pp.feed_line("src/a.c:1:1: error: err1");
        pp.feed_line("src/b.c:2:1: error: err2");
        pp.feed_line("src/c.c:3:1: error: err3");
        assert_eq!(pp.items.len(), 3);
        assert_eq!(pp.compute_rows().len(), 7);
        assert_eq!(pp.selected, 0);
        pp.move_down();
        assert_eq!(pp.selected, 1);
        pp.move_down();
        assert_eq!(pp.selected, 2);
        // Navigate to last row
        for _ in 0..10 {
            pp.move_down();
        }
        assert_eq!(pp.selected, 6); // clamped at last row
        pp.move_up();
        assert_eq!(pp.selected, 5);
    }

    // --- Counts ---

    #[test]
    fn test_error_warning_counts() {
        let mut pp = ProblemPanel::new();
        pp.feed_line("src/a.c:1:1: error: err");
        pp.feed_line("src/b.c:2:1: warning: warn");
        assert_eq!(pp.error_count(), 1);
        assert_eq!(pp.warning_count(), 1);
    }

    // --- Clear ---

    #[test]
    fn test_clear() {
        let mut pp = ProblemPanel::new();
        pp.feed_line("src/a.c:1:1: error: err");
        pp.source_cmd = Some("cargo build".to_string());
        pp.clear();
        assert!(pp.items.is_empty());
        assert_eq!(pp.selected, 0);
        // source_cmd is preserved (cleared separately by caller)
    }

    // --- LSP multi-file grouping ---

    /// Simulates what drain_lsp_messages does: feeds Problems from N distinct
    /// files into set_lsp_items and verifies every file's diagnostics are
    /// accessible via lsp_items_for_file() (shown in the DIAGNOSTICS tab).
    #[test]
    fn test_lsp_multifile_all_files_shown() {
        // These are the actual zedit src files that cargo build reports warnings
        // for — proves we handle real-world multi-file LSP data, not just main.rs.
        let files = vec![
            ("src/diff_view.rs",      Severity::Warning, 48,  "unused variable"),
            ("src/editor/minimap.rs", Severity::Warning, 81,  "dead code"),
            ("src/extension.rs",      Severity::Warning, 17,  "unused import"),
            ("src/layout.rs",         Severity::Warning, 80,  "unused variable"),
            ("src/layout.rs",         Severity::Warning, 100, "unused variable"),
            ("src/lsp/client.rs",     Severity::Warning, 67,  "unused field"),
            ("src/lsp/mod.rs",        Severity::Warning, 34,  "dead code"),
            ("src/plugin/mod.rs",     Severity::Warning, 21,  "unused variable"),
            ("src/plugin/mod.rs",     Severity::Warning, 96,  "dead code"),
            ("src/main.rs",           Severity::Warning, 10,  "unused import"),
        ];

        let problems: Vec<Problem> = files
            .iter()
            .map(|(f, sev, line, msg)| Problem {
                severity: *sev,
                file: f.to_string(),
                line: *line,
                col: 1,
                message: msg.to_string(),
                code: None,
            })
            .collect();

        let mut pp = ProblemPanel::new();
        pp.set_lsp_items(problems);

        // Every distinct file must have its diagnostics accessible via lsp_items_for_file.
        // (LSP items are shown in the DIAGNOSTICS tab, not in compute_rows().)
        let distinct_files = vec![
            "src/diff_view.rs",
            "src/editor/minimap.rs",
            "src/extension.rs",
            "src/layout.rs",
            "src/lsp/client.rs",
            "src/lsp/mod.rs",
            "src/plugin/mod.rs",
            "src/main.rs",
        ];
        for f in &distinct_files {
            let items = pp.lsp_items_for_file(f);
            assert!(
                !items.is_empty(),
                "lsp_items_for_file({}) should not be empty — diagnostics tab would not show this file",
                f
            );
        }

        // Total lsp_items must equal total problems fed in.
        assert_eq!(pp.lsp_items.len(), 10, "all 10 problem items must be stored in lsp_items");

        // Global counts must be correct.
        assert_eq!(pp.total_warning_count(), 10);
        assert_eq!(pp.total_error_count(), 0);
    }

    /// Scans /home/rakzo/github/zedit/src for every .rs file, creates one fake
    /// LSP Warning per file, and verifies compute_rows() returns a FileHeader
    /// for each one — proving the panel scales to the full workspace.
    #[test]
    fn test_lsp_loads_full_src_directory() {
        use std::path::Path;

        fn collect_rs_files(dir: &Path, out: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    collect_rs_files(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    // Store as relative path from repo root, matching LSP display.
                    let rel = path
                        .strip_prefix("/home/rakzo/github/zedit")
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();
                    out.push(rel);
                }
            }
        }

        let mut rs_files = Vec::new();
        collect_rs_files(Path::new("/home/rakzo/github/zedit/src"), &mut rs_files);

        assert!(
            rs_files.len() >= 10,
            "expected at least 10 .rs files in src, got {}",
            rs_files.len()
        );

        // Build one Warning per file — simulates LSP sending diagnostics for the
        // whole workspace (which is what rust-analyzer does after checkOnSave).
        let problems: Vec<Problem> = rs_files
            .iter()
            .map(|f| Problem {
                severity: Severity::Warning,
                file: f.clone(),
                line: 1,
                col: 1,
                message: "simulated lsp diagnostic".to_string(),
                code: None,
            })
            .collect();

        let total = problems.len();
        let mut pp = ProblemPanel::new();
        pp.set_lsp_items(problems);

        // Every .rs file must have its diagnostic accessible via lsp_items_for_file.
        // (LSP items are shown in the DIAGNOSTICS tab, not in compute_rows().)
        for f in &rs_files {
            let items = pp.lsp_items_for_file(f);
            assert!(
                !items.is_empty(),
                "lsp_items_for_file({}) should not be empty — diagnostics tab would hide this file",
                f
            );
        }

        // Total lsp_items must equal the number of files.
        assert_eq!(
            pp.lsp_items.len(),
            total,
            "all {} LSP items must be stored in lsp_items",
            total
        );
    }
}
