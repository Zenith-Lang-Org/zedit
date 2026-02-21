/// Problem Panel — captures build/run output, parses errors, and lets the user
/// navigate to source locations with Enter.
///
/// Parsers supported:
///   - Rust / Cargo  (`error[E0425]: msg\n  --> file:line:col`)
///   - GCC / Clang   (`file:line:col: error: msg`)
///   - Python        (`File "file", line N\n  SyntaxError: msg`)
///   - Generic       (`file:line:col: msg`)

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
    pub code: Option<String>, // "E0425", "W0001", etc.
}

// ── ProblemPanel ─────────────────────────────────────────────

pub struct ProblemPanel {
    pub visible: bool,
    pub focused: bool,
    pub items: Vec<Problem>,
    pub selected: usize,
    pub scroll: usize,
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
            selected: 0,
            scroll: 0,
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

    pub fn selected_problem(&self) -> Option<&Problem> {
        self.items.get(self.selected)
    }

    pub fn error_count(&self) -> usize {
        self.items
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.items
            .iter()
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
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

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
        let mut pp = ProblemPanel::new();
        pp.feed_line("src/a.c:1:1: error: err1");
        pp.feed_line("src/b.c:2:1: error: err2");
        pp.feed_line("src/c.c:3:1: error: err3");
        assert_eq!(pp.items.len(), 3);
        assert_eq!(pp.selected, 0);
        pp.move_down();
        assert_eq!(pp.selected, 1);
        pp.move_down();
        assert_eq!(pp.selected, 2);
        pp.move_down(); // no-op at end
        assert_eq!(pp.selected, 2);
        pp.move_up();
        assert_eq!(pp.selected, 1);
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
}
