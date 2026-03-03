// ---------------------------------------------------------------------------
// Debug logger — writes to /tmp/zedit_debug.log when ZEDIT_DEBUG=1
// ---------------------------------------------------------------------------
//
// Usage:
//   ZEDIT_DEBUG=1 ./target/debug/zedit file.rs
//   tail -f /tmp/zedit_debug.log

// ---------------------------------------------------------------------------
// Performance logger — activated by the `--log` CLI flag.
// Writes a structured timing report to ~/.local/state/zedit/perf.log.
// ---------------------------------------------------------------------------
//
// Usage:
//   zedit --log [file]
//   cat ~/.local/state/zedit/perf.log

use std::cell::{Cell, RefCell};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

// ── Debug log ────────────────────────────────────────────────

static LOG: Mutex<Option<File>> = Mutex::new(None);

/// Initialize the debug logger. Call once from main().
/// Only opens the log file if the environment variable ZEDIT_DEBUG=1.
pub fn init() {
    if std::env::var("ZEDIT_DEBUG").as_deref() != Ok("1") {
        return;
    }
    match OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/zedit_debug.log")
    {
        Ok(f) => {
            *LOG.lock().unwrap() = Some(f);
            log("=== zedit debug log opened ===");
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "zedit: cannot open debug log: {}", e);
        }
    }
}

/// Write a line to the debug log (no-op if not initialised).
pub fn log(msg: &str) {
    if let Ok(mut guard) = LOG.lock() {
        if let Some(ref mut f) = *guard {
            let _ = writeln!(f, "{}", msg);
            let _ = f.flush();
        }
    }
}

/// Convenience macro: `dlog!("key={} val={}", k, v)`
#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        $crate::debug_log::log(&format!($($arg)*))
    };
}

// ── Performance / startup log ─────────────────────────────────

struct PerfState {
    enabled: bool,
    start: Option<Instant>,
    file: Option<File>,
    events: Vec<(u128, String)>, // (elapsed_us, label)
}

static PERF: Mutex<PerfState> = Mutex::new(PerfState {
    enabled: false,
    start: None,
    file: None,
    events: Vec::new(),
});

/// Enable the performance log.  Call before any `perf_record()` calls.
/// Creates / truncates `~/.local/state/zedit/perf.log`.
pub fn perf_enable() {
    let path = perf_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .ok();

    if let Ok(mut g) = PERF.lock() {
        g.enabled = true;
        g.start = Some(Instant::now());
        g.file = file;
        g.events.clear();
    }
}

/// Returns true when the performance log is active.
pub fn perf_enabled() -> bool {
    PERF.lock().map(|g| g.enabled).unwrap_or(false)
}

/// Record a timestamped event label.  No-op when the log is not enabled.
pub fn perf_record(label: &str) {
    if let Ok(mut g) = PERF.lock() {
        if !g.enabled {
            return;
        }
        let elapsed_us = g
            .start
            .map(|s| s.elapsed().as_micros())
            .unwrap_or(0);
        let line = format!("[{:>10.3}ms] {}", elapsed_us as f64 / 1000.0, label);
        if let Some(ref mut f) = g.file {
            let _ = writeln!(f, "{}", line);
            let _ = f.flush();
        }
        g.events.push((elapsed_us, label.to_string()));
    }
}

/// Convenience macro: `perf!("grammar loaded: {}", lang)`
#[macro_export]
macro_rules! perf {
    ($($arg:tt)*) => {
        $crate::debug_log::perf_record(&format!($($arg)*))
    };
}

/// Write the final summary (memory + CPU) to the log file and return the log path.
/// Call this just before the editor exits when --log is active.
pub fn perf_finish() {
    if let Ok(mut g) = PERF.lock() {
        if !g.enabled {
            return;
        }
        let total_us = g.start.map(|s| s.elapsed().as_micros()).unwrap_or(0);
        let summary = build_summary(total_us, &g.events);
        if let Some(ref mut f) = g.file {
            let _ = writeln!(f, "{}", summary);
            let _ = f.flush();
        }
    }
    // Also print to stderr so the user can see it immediately after zedit exits.
    if let Ok(g) = PERF.lock() {
        if g.enabled {
            let total_us = g.start.map(|s| s.elapsed().as_micros()).unwrap_or(0);
            let summary = build_summary(total_us, &g.events);
            let _ = writeln!(std::io::stderr(), "\n{}", summary);
            let path = perf_log_path();
            let _ = writeln!(
                std::io::stderr(),
                "  Full log: {}",
                path.display()
            );
        }
    }
}

// ── Internal helpers ─────────────────────────────────────────

/// Returns (and creates) the zedit state directory: `~/.local/state/zedit/`.
fn log_state_dir() -> std::path::PathBuf {
    let base = if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        std::path::PathBuf::from(xdg).join("zedit")
    } else if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("zedit")
    } else {
        std::path::PathBuf::from("/tmp/zedit-state")
    };
    let _ = std::fs::create_dir_all(&base);
    base
}

fn perf_log_path() -> std::path::PathBuf {
    log_state_dir().join("perf.log")
}

fn build_summary(total_us: u128, events: &[(u128, String)]) -> String {
    let mut out = String::new();
    out.push_str("─────────────────────────────────────────────\n");
    out.push_str("zedit startup performance summary\n");
    out.push_str("─────────────────────────────────────────────\n");

    // Events
    let mut prev_us: u128 = 0;
    for (us, label) in events {
        let delta = us.saturating_sub(prev_us);
        out.push_str(&format!(
            "  [{:>10.3}ms] (+{:.3}ms)  {}\n",
            *us as f64 / 1000.0,
            delta as f64 / 1000.0,
            label
        ));
        prev_us = *us;
    }
    out.push_str(&format!(
        "  [{:>10.3}ms] total session time\n",
        total_us as f64 / 1000.0
    ));
    out.push_str("─────────────────────────────────────────────\n");

    // Memory from /proc/self/status (Linux)
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        let mut rss_kb: u64 = 0;
        let mut vm_kb: u64 = 0;
        let mut peak_kb: u64 = 0;
        for line in status.lines() {
            let parse_kb = |s: &str| -> u64 {
                s.split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
            };
            if line.starts_with("VmRSS:") {
                rss_kb = parse_kb(line);
            } else if line.starts_with("VmSize:") {
                vm_kb = parse_kb(line);
            } else if line.starts_with("VmPeak:") {
                peak_kb = parse_kb(line);
            }
        }
        out.push_str(&format!(
            "  memory:  RSS {:.1} MB   virtual {:.1} MB   peak {:.1} MB\n",
            rss_kb as f64 / 1024.0,
            vm_kb as f64 / 1024.0,
            peak_kb as f64 / 1024.0,
        ));
    }

    // CPU time via getrusage (manual FFI — no external crates)
    if let Some((user_ms, sys_ms)) = cpu_times_ms() {
        out.push_str(&format!(
            "  cpu:     user {:.1}ms   sys {:.1}ms   total {:.1}ms\n",
            user_ms,
            sys_ms,
            user_ms + sys_ms,
        ));
    }

    out.push_str("─────────────────────────────────────────────");
    out
}

/// Query process CPU times using `getrusage(RUSAGE_SELF)`.
/// Returns `(user_ms, sys_ms)` or `None` on unsupported platforms.
fn cpu_times_ms() -> Option<(f64, f64)> {
    // Manual FFI — zero external crates, matching the project convention.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        // Minimal representations of `struct timeval` and `struct rusage`.
        // On both Linux x86-64 and macOS, tv_sec/tv_usec are both i64 (or
        // equivalent), and the two timeval fields are the first 32 bytes of rusage.
        #[repr(C)]
        struct Timeval {
            tv_sec: i64,
            tv_usec: i64,
        }
        // rusage has many fields; we only read the first two (utime, stime).
        // 144 bytes covers the full struct on both Linux and macOS x86-64.
        #[repr(C)]
        struct Rusage {
            ru_utime: Timeval,
            ru_stime: Timeval,
            _rest: [u8; 112],
        }

        unsafe extern "C" {
            fn getrusage(who: i32, usage: *mut Rusage) -> i32;
        }

        const RUSAGE_SELF: i32 = 0;
        let mut usage = Rusage {
            ru_utime: Timeval { tv_sec: 0, tv_usec: 0 },
            ru_stime: Timeval { tv_sec: 0, tv_usec: 0 },
            _rest: [0u8; 112],
        };
        let ret = unsafe { getrusage(RUSAGE_SELF, &mut usage) };
        if ret != 0 {
            return None;
        }
        let user_ms = usage.ru_utime.tv_sec as f64 * 1000.0
            + usage.ru_utime.tv_usec as f64 / 1000.0;
        let sys_ms = usage.ru_stime.tv_sec as f64 * 1000.0
            + usage.ru_stime.tv_usec as f64 / 1000.0;
        return Some((user_ms, sys_ms));
    }

    #[allow(unreachable_code)]
    None
}

// ── Runtime diagnostic logging ────────────────────────────────────────────────
//
// Three independent subsystems for diagnosing runtime hotspots:
//   --log-key     → ~/.local/state/zedit/key.log     (keypress latency)
//   --log-render  → ~/.local/state/zedit/render.log  (frame breakdown)
//   --log-syntax  → ~/.local/state/zedit/syntax.log  (tokenizer timing)
//
// Overhead when disabled: one AtomicBool::load(Relaxed) ≈ 1ns per call site.
// No allocations, no mutex, no I/O on the hot path.

// ── Global enable flags (checked in hot path) ────────────────────────────────

static KEY_LOG_ENABLED:    AtomicBool = AtomicBool::new(false);
static RENDER_LOG_ENABLED: AtomicBool = AtomicBool::new(false);
static SYNTAX_LOG_ENABLED: AtomicBool = AtomicBool::new(false);

// ── Log file state (Mutex only at open/write time) ───────────────────────────

struct RuntimeLog {
    file:  Option<File>,
    start: Option<Instant>,
}

static KEY_LOG:    Mutex<RuntimeLog> = Mutex::new(RuntimeLog { file: None, start: None });
static RENDER_LOG: Mutex<RuntimeLog> = Mutex::new(RuntimeLog { file: None, start: None });
static SYNTAX_LOG: Mutex<RuntimeLog> = Mutex::new(RuntimeLog { file: None, start: None });

// ── Per-thread timing state (zero contention in hot path) ────────────────────

thread_local! {
    // --log-key
    static KEY_T0:    Cell<Option<Instant>> = const { Cell::new(None) };
    static KEY_T1:    Cell<Option<Instant>> = const { Cell::new(None) };
    static KEY_LABEL: RefCell<String>       = RefCell::new(String::new());

    // --log-render
    static RENDER_T0:    Cell<Option<Instant>> = const { Cell::new(None) };
    static RENDER_PREV:  Cell<Option<Instant>> = const { Cell::new(None) };
    static RENDER_FRAME: Cell<u64>             = const { Cell::new(0) };
    static RENDER_BUF:   RefCell<String>       = RefCell::new(String::new());
}

// ── --log-key API ─────────────────────────────────────────────────────────────

/// Enable keypress latency logging.  Opens `~/.local/state/zedit/key.log`.
pub fn key_log_enable() {
    let path = log_state_dir().join("key.log");
    let file = OpenOptions::new()
        .create(true).write(true).truncate(true)
        .open(&path).ok();
    if let Ok(mut g) = KEY_LOG.lock() {
        g.file  = file;
        g.start = Some(Instant::now());
    }
    KEY_LOG_ENABLED.store(true, Ordering::Relaxed);
}

/// Returns `true` when keypress logging is active.
pub fn key_log_enabled() -> bool {
    KEY_LOG_ENABLED.load(Ordering::Relaxed)
}

/// Record the start of a key event (T0).  Call before `handle_event`.
pub fn key_event_start(label: &str) {
    if !KEY_LOG_ENABLED.load(Ordering::Relaxed) { return; }
    KEY_T0.with(|t| t.set(Some(Instant::now())));
    KEY_T1.with(|t| t.set(None));
    KEY_LABEL.with(|l| { let mut b = l.borrow_mut(); b.clear(); b.push_str(label); });
}

/// Record the moment handle_event returned (T1).
pub fn key_handle_done() {
    if !KEY_LOG_ENABLED.load(Ordering::Relaxed) { return; }
    KEY_T1.with(|t| t.set(Some(Instant::now())));
}

/// Record the moment the frame finished rendering (T2) and write one log line.
///
/// `rendered`: `true` when a frame was actually drawn, `false` when skipped.
pub fn key_render_done(rendered: bool) {
    if !KEY_LOG_ENABLED.load(Ordering::Relaxed) { return; }
    let t2  = Instant::now();
    let t0  = KEY_T0.with(|t| t.get());
    let t1  = KEY_T1.with(|t| t.get());
    let lbl = KEY_LABEL.with(|l| l.borrow().clone());

    let t0 = match t0 { Some(t) => t, None => return };

    let handle_us = t1.map(|t1| t1.duration_since(t0).as_micros()).unwrap_or(0);
    let render_us = t1.map(|t1| t2.duration_since(t1).as_micros()).unwrap_or(0);
    let total_us  = t2.duration_since(t0).as_micros();
    let slow      = if total_us > 5_000 { "  *** SLOW" } else { "" };

    if let Ok(mut g) = KEY_LOG.lock() {
        let elapsed_ms = g.start
            .map(|s| s.elapsed().as_micros() as f64 / 1_000.0)
            .unwrap_or(0.0);
        let line = if rendered {
            format!(
                "[{:>10.3}ms] {:<20} → handle={:.1}ms render={:.1}ms total={:.1}ms{}",
                elapsed_ms, lbl,
                handle_us as f64 / 1_000.0,
                render_us as f64 / 1_000.0,
                total_us  as f64 / 1_000.0,
                slow,
            )
        } else {
            format!(
                "[{:>10.3}ms] {:<20} → handle={:.1}ms (no-render) total={:.1}ms{}",
                elapsed_ms, lbl,
                handle_us as f64 / 1_000.0,
                total_us  as f64 / 1_000.0,
                slow,
            )
        };
        if let Some(ref mut f) = g.file {
            let _ = writeln!(f, "{}", line);
            let _ = f.flush();
        }
    }

    // Reset T0 so a stale T0 is not reused on the next render if no key was pressed.
    KEY_T0.with(|t| t.set(None));
}

// ── --log-render API ──────────────────────────────────────────────────────────

/// Enable frame render breakdown logging.  Opens `~/.local/state/zedit/render.log`.
pub fn render_log_enable() {
    let path = log_state_dir().join("render.log");
    let file = OpenOptions::new()
        .create(true).write(true).truncate(true)
        .open(&path).ok();
    if let Ok(mut g) = RENDER_LOG.lock() {
        g.file  = file;
        g.start = Some(Instant::now());
    }
    RENDER_LOG_ENABLED.store(true, Ordering::Relaxed);
}

/// Returns `true` when render breakdown logging is active.
pub fn render_log_enabled() -> bool {
    RENDER_LOG_ENABLED.load(Ordering::Relaxed)
}

/// Record the start of a render frame (T0).  Call at the top of `render()`.
pub fn render_frame_start() {
    if !RENDER_LOG_ENABLED.load(Ordering::Relaxed) { return; }
    let now = Instant::now();
    RENDER_T0.with(|t|   t.set(Some(now)));
    RENDER_PREV.with(|t| t.set(Some(now)));
    RENDER_FRAME.with(|f| f.set(f.get() + 1));
    RENDER_BUF.with(|b| b.borrow_mut().clear());
}

/// Record a named checkpoint within `render()`.
/// Appends `name=Xms ` to the internal per-frame buffer.
pub fn render_checkpoint(name: &str) {
    if !RENDER_LOG_ENABLED.load(Ordering::Relaxed) { return; }
    let now = Instant::now();
    let delta_ms = RENDER_PREV.with(|p| {
        let prev = p.get().unwrap_or(now);
        let delta = now.duration_since(prev).as_micros() as f64 / 1_000.0;
        p.set(Some(now));
        delta
    });
    RENDER_BUF.with(|b| {
        use std::fmt::Write as FmtWrite;
        let _ = write!(b.borrow_mut(), "{}={:.1} ", name, delta_ms);
    });
}

/// Write one log line summarising the frame.  Call after `screen.flush()`.
///
/// `dirty_cells`: number of screen cells that changed (pass 0 if unavailable).
pub fn render_frame_done(dirty_cells: usize) {
    if !RENDER_LOG_ENABLED.load(Ordering::Relaxed) { return; }
    let t2    = Instant::now();
    let frame = RENDER_FRAME.with(|f| f.get());
    let t0    = RENDER_T0.with(|t| t.get());
    let buf   = RENDER_BUF.with(|b| b.borrow().clone());

    let total_us = t0.map(|t| t2.duration_since(t).as_micros()).unwrap_or(0);
    let total_ms = total_us as f64 / 1_000.0;
    let slow     = if total_us > 5_000 { "  *** SLOW" } else { "" };

    if let Ok(mut g) = RENDER_LOG.lock() {
        let elapsed_ms = g.start
            .map(|s| s.elapsed().as_micros() as f64 / 1_000.0)
            .unwrap_or(0.0);
        let line = format!(
            "[f={:06} {:>8.1}ms] {}total={:.1}ms{}  dirty={}",
            frame, elapsed_ms, buf, total_ms, slow, dirty_cells,
        );
        if let Some(ref mut f) = g.file {
            let _ = writeln!(f, "{}", line);
            let _ = f.flush();
        }
    }
}

/// Convenience macro: `render_cp!("section_name")` inside `render()`.
#[macro_export]
macro_rules! render_cp {
    ($name:expr) => {
        $crate::debug_log::render_checkpoint($name)
    };
}

// ── --log-syntax API ──────────────────────────────────────────────────────────

/// Enable tokenizer timing logging.  Opens `~/.local/state/zedit/syntax.log`.
pub fn syntax_log_enable() {
    let path = log_state_dir().join("syntax.log");
    let file = OpenOptions::new()
        .create(true).write(true).truncate(true)
        .open(&path).ok();
    if let Ok(mut g) = SYNTAX_LOG.lock() {
        g.file  = file;
        g.start = Some(Instant::now());
    }
    SYNTAX_LOG_ENABLED.store(true, Ordering::Relaxed);
}

/// Returns `true` when tokenizer timing logging is active.
pub fn syntax_log_enabled() -> bool {
    SYNTAX_LOG_ENABLED.load(Ordering::Relaxed)
}

/// Write one tokenizer timing entry.  Filters noise: only records cache hits
/// slower than 0.5ms, cold starts (warmup > 0), or misses slower than 0.5ms.
///
/// * `line_num` — 0-based line index
/// * `cached`   — `true` when the result came from the span cache (HIT)
/// * `warmup`   — number of preceding lines tokenized to reach `line_num` (COLD > 0)
/// * `total_us` — elapsed microseconds for the whole `style_line` call
/// * `spans`    — number of `StyledSpan`s returned
pub fn syntax_write(
    line_num: usize,
    cached:   bool,
    warmup:   usize,
    total_us: u128,
    spans:    usize,
) {
    // Noise filter: skip fast cache hits and fast simple misses.
    if cached && total_us <= 500 { return; }
    if !cached && warmup == 0 && total_us <= 500 { return; }

    let line_str = if cached {
        format!(
            "[HIT ] L{:04}  {:.3}ms  spans={}",
            line_num, total_us as f64 / 1_000.0, spans,
        )
    } else if warmup > 0 {
        let slow = if total_us > 1_000 { "  *** SLOW" } else { "" };
        format!(
            "[COLD] L{:04}  warm={:<4} total={:.1}ms  spans={}{}",
            line_num, warmup, total_us as f64 / 1_000.0, spans, slow,
        )
    } else {
        format!(
            "[MISS] L{:04}  warm=0   total={:.3}ms  spans={}",
            line_num, total_us as f64 / 1_000.0, spans,
        )
    };

    if let Ok(mut g) = SYNTAX_LOG.lock() {
        if let Some(ref mut f) = g.file {
            let _ = writeln!(f, "{}", line_str);
            let _ = f.flush();
        }
    }
}
