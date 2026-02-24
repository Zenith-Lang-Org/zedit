//! Session persistence module.
//!
//! Saves/restores editor session metadata (open files, cursor positions, etc.)
//! to JSON files in `~/.local/state/zedit/sessions/`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::syntax::json_parser::JsonValue;

/// Metadata for a single buffer in the session.
pub struct BufferSession {
    pub file_path: Option<String>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub scroll_row: usize,
    pub has_swap: bool,
    pub untitled_index: Option<usize>,
}

/// Full session state.
pub struct Session {
    pub version: u32,
    pub working_dir: PathBuf,
    pub buffers: Vec<BufferSession>,
    pub active_buffer: usize,
}

/// Get the session file path for a working directory.
/// Uses FNV-1a hash of canonical path as filename.
pub fn session_path(working_dir: &Path) -> PathBuf {
    let canonical = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());
    let hash = fnv1a(canonical.to_string_lossy().as_bytes());
    state_dir()
        .join("sessions")
        .join(format!("{:016x}.json", hash))
}

/// Save a session to disk.
pub fn save_session(session: &Session) -> Result<(), String> {
    let path = session_path(&session.working_dir);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let json = session_to_json(session);
    fs::write(&path, json.as_bytes()).map_err(|e| format!("session save failed: {}", e))
}

/// Load a session from disk.
pub fn load_session(working_dir: &Path) -> Option<Session> {
    let path = session_path(working_dir);
    let data = fs::read_to_string(&path).ok()?;
    let val = JsonValue::parse(&data).ok()?;
    parse_session(&val, working_dir)
}

/// Delete the session file for a working directory.
#[allow(dead_code)]
pub fn delete_session(working_dir: &Path) {
    let path = session_path(working_dir);
    let _ = fs::remove_file(&path);
}

fn session_to_json(session: &Session) -> String {
    let mut buffers = Vec::new();
    for bs in &session.buffers {
        let mut pairs: Vec<(String, JsonValue)> = Vec::new();

        match &bs.file_path {
            Some(p) => pairs.push(("file_path".into(), JsonValue::String(p.clone()))),
            None => pairs.push(("file_path".into(), JsonValue::Null)),
        }
        pairs.push((
            "cursor_line".into(),
            JsonValue::Number(bs.cursor_line as f64),
        ));
        pairs.push(("cursor_col".into(), JsonValue::Number(bs.cursor_col as f64)));
        pairs.push(("scroll_row".into(), JsonValue::Number(bs.scroll_row as f64)));
        pairs.push(("has_swap".into(), JsonValue::Bool(bs.has_swap)));
        match bs.untitled_index {
            Some(idx) => pairs.push(("untitled_index".into(), JsonValue::Number(idx as f64))),
            None => pairs.push(("untitled_index".into(), JsonValue::Null)),
        }

        buffers.push(JsonValue::Object(pairs));
    }

    let root = JsonValue::Object(vec![
        ("version".into(), JsonValue::Number(session.version as f64)),
        (
            "working_dir".into(),
            JsonValue::String(session.working_dir.to_string_lossy().into_owned()),
        ),
        ("buffers".into(), JsonValue::Array(buffers)),
        (
            "active_buffer".into(),
            JsonValue::Number(session.active_buffer as f64),
        ),
    ]);

    root.to_json_pretty(2)
}

fn parse_session(val: &JsonValue, working_dir: &Path) -> Option<Session> {
    let version = val.get("version")?.as_f64()? as u32;
    if version != 1 {
        return None;
    }

    let active_buffer = val.get("active_buffer")?.as_f64()? as usize;
    let buffers_val = val.get("buffers")?.as_array()?;

    let mut buffers = Vec::new();
    for bv in buffers_val {
        let file_path = bv
            .get("file_path")
            .and_then(|v| v.as_str().map(String::from));
        let cursor_line = bv
            .get("cursor_line")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as usize;
        let cursor_col = bv.get("cursor_col").and_then(|v| v.as_f64()).unwrap_or(0.0) as usize;
        let scroll_row = bv.get("scroll_row").and_then(|v| v.as_f64()).unwrap_or(0.0) as usize;
        let has_swap = bv
            .get("has_swap")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let untitled_index = bv
            .get("untitled_index")
            .and_then(|v| v.as_f64())
            .map(|n| n as usize);

        buffers.push(BufferSession {
            file_path,
            cursor_line,
            cursor_col,
            scroll_row,
            has_swap,
            untitled_index,
        });
    }

    Some(Session {
        version,
        working_dir: working_dir.to_path_buf(),
        buffers,
        active_buffer,
    })
}

/// XDG state directory: $XDG_STATE_HOME/zedit or ~/.local/state/zedit
fn state_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("zedit")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("zedit")
    } else {
        PathBuf::from("/tmp/zedit-state")
    }
}

/// FNV-1a 64-bit hash.
fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fnv1a() {
        let h1 = fnv1a(b"/home/user/project");
        let h2 = fnv1a(b"/home/user/other");
        assert_ne!(h1, h2);
        // Same input gives same hash
        assert_eq!(h1, fnv1a(b"/home/user/project"));
    }

    #[test]
    fn test_session_roundtrip() {
        let dir = std::env::temp_dir().join("zedit_session_test");
        let _ = fs::create_dir_all(&dir);

        let session = Session {
            version: 1,
            working_dir: dir.clone(),
            buffers: vec![
                BufferSession {
                    file_path: Some("/tmp/test.rs".to_string()),
                    cursor_line: 42,
                    cursor_col: 10,
                    scroll_row: 30,
                    has_swap: true,
                    untitled_index: None,
                },
                BufferSession {
                    file_path: None,
                    cursor_line: 0,
                    cursor_col: 0,
                    scroll_row: 0,
                    has_swap: false,
                    untitled_index: Some(0),
                },
            ],
            active_buffer: 0,
        };

        save_session(&session).unwrap();

        let loaded = load_session(&dir).unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.buffers.len(), 2);
        assert_eq!(loaded.buffers[0].file_path.as_deref(), Some("/tmp/test.rs"));
        assert_eq!(loaded.buffers[0].cursor_line, 42);
        assert_eq!(loaded.buffers[0].cursor_col, 10);
        assert_eq!(loaded.buffers[0].scroll_row, 30);
        assert!(loaded.buffers[0].has_swap);
        assert!(loaded.buffers[0].untitled_index.is_none());
        assert!(loaded.buffers[1].file_path.is_none());
        assert_eq!(loaded.buffers[1].untitled_index, Some(0));
        assert_eq!(loaded.active_buffer, 0);

        // Cleanup
        delete_session(&dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_session_path_deterministic() {
        let dir = Path::new("/tmp/zedit_test_proj");
        let p1 = session_path(dir);
        let p2 = session_path(dir);
        assert_eq!(p1, p2);
    }
}
