// ---------------------------------------------------------------------------
// Debug logger — writes to /tmp/zedit_debug.log when ZEDIT_DEBUG=1
// ---------------------------------------------------------------------------
//
// Usage:
//   ZEDIT_DEBUG=1 ./target/debug/zedit file.rs
//   tail -f /tmp/zedit_debug.log

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

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
