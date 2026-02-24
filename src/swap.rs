//! Swap file module for crash recovery.
//!
//! Binary swap files are stored alongside original files (`.filename.ext.swp`).
//! Format:
//!   0..4     Magic: b"ZSWP"
//!   4..8     Version: u32 LE (1)
//!   8..12    PID: u32 LE
//!   12..20   Timestamp: u64 LE (Unix epoch secs)
//!   20..24   Path length: u32 LE
//!   24..24+N Original path: UTF-8
//!   24+N     Modified flag: 0x00/0x01
//!   25+N..   Buffer content: raw UTF-8

use std::fs;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"ZSWP";
const VERSION: u32 = 1;

#[derive(Debug)]
pub struct SwapHeader {
    pub pid: u32,
    #[allow(dead_code)]
    pub timestamp: u64,
    #[allow(dead_code)]
    pub original_path: String,
    #[allow(dead_code)]
    pub modified: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SwapStatus {
    None,
    OwnedByUs,
    Orphaned,
    Corrupt,
}

/// Compute the swap file path for a given file.
/// e.g. `/home/user/project/foo.rs` → `/home/user/project/.foo.rs.swp`
pub fn swap_path(file_path: &Path) -> PathBuf {
    let parent = file_path.parent().unwrap_or(Path::new("."));
    let name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled".to_string());
    parent.join(format!(".{}.swp", name))
}

/// Compute the swap file path for an untitled buffer.
/// Stored in `~/.local/state/zedit/swap/NewBufferNN.swp`.
pub fn swap_path_untitled(id: usize) -> PathBuf {
    let dir = state_dir().join("swap");
    dir.join(format!("NewBuffer{:02}.swp", id))
}

/// Write a swap file atomically (write tmp → fsync → rename).
pub fn write_swap(file_path: &Path, content: &[u8], modified: bool) -> Result<(), String> {
    let swp = swap_path(file_path);
    write_swap_to(&swp, file_path, content, modified)
}

/// Write a swap file for an untitled buffer.
pub fn write_swap_untitled(id: usize, content: &[u8], modified: bool) -> Result<(), String> {
    let swp = swap_path_untitled(id);
    let fake_path = format!("NewBuffer{:02}", id);
    write_swap_to(&swp, Path::new(&fake_path), content, modified)
}

fn write_swap_to(
    swp: &Path,
    original_path: &Path,
    content: &[u8],
    modified: bool,
) -> Result<(), String> {
    // Ensure parent directory exists
    if let Some(parent) = swp.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let tmp = swp.with_extension("swp.tmp");
    let path_str = original_path.to_string_lossy();
    let path_bytes = path_str.as_bytes();

    let pid = getpid();
    let timestamp = unix_timestamp();

    let mut data = Vec::with_capacity(25 + path_bytes.len() + content.len());
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&VERSION.to_le_bytes());
    data.extend_from_slice(&pid.to_le_bytes());
    data.extend_from_slice(&timestamp.to_le_bytes());
    data.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
    data.extend_from_slice(path_bytes);
    data.push(if modified { 0x01 } else { 0x00 });
    data.extend_from_slice(content);

    fs::write(&tmp, &data).map_err(|e| format!("swap write failed: {}", e))?;

    // fsync via opening and syncing
    if let Ok(f) = fs::File::open(&tmp) {
        let _ = f.sync_all();
    }

    fs::rename(&tmp, swp).map_err(|e| format!("swap rename failed: {}", e))?;
    Ok(())
}

/// Read a swap file, returning the header and buffer content.
pub fn read_swap(swap: &Path) -> Result<(SwapHeader, String), String> {
    let data = fs::read(swap).map_err(|e| format!("failed to read swap: {}", e))?;

    if data.len() < 25 {
        return Err("swap file too short".to_string());
    }
    if &data[0..4] != MAGIC {
        return Err("invalid swap magic".to_string());
    }
    let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    if version != VERSION {
        return Err(format!("unsupported swap version: {}", version));
    }
    let pid = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let timestamp = u64::from_le_bytes([
        data[12], data[13], data[14], data[15], data[16], data[17], data[18], data[19],
    ]);
    let path_len = u32::from_le_bytes([data[20], data[21], data[22], data[23]]) as usize;

    if data.len() < 25 + path_len {
        return Err("swap file truncated".to_string());
    }

    let original_path =
        std::str::from_utf8(&data[24..24 + path_len]).map_err(|_| "invalid path in swap")?;

    let modified = data[24 + path_len] != 0;
    let content_bytes = &data[25 + path_len..];
    let content = String::from_utf8_lossy(content_bytes).into_owned();

    Ok((
        SwapHeader {
            pid,
            timestamp,
            original_path: original_path.to_string(),
            modified,
        },
        content,
    ))
}

/// Remove the swap file for a given file path.
pub fn remove_swap(file_path: &Path) {
    let swp = swap_path(file_path);
    let _ = fs::remove_file(&swp);
}

/// Remove the swap file for an untitled buffer.
pub fn remove_swap_untitled(id: usize) {
    let swp = swap_path_untitled(id);
    let _ = fs::remove_file(&swp);
}

/// Scan the untitled swap directory for orphaned swap files.
/// Returns a list of (id, swap_path) for each orphaned NewBufferNN.swp found.
pub fn scan_orphaned_untitled() -> Vec<(usize, PathBuf)> {
    let dir = state_dir().join("swap");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Match "NewBufferNN.swp"
        if let Some(rest) = name_str.strip_prefix("NewBuffer")
            && let Some(num_str) = rest.strip_suffix(".swp")
            && let Ok(id) = num_str.parse::<usize>()
        {
            let swp_path = entry.path();
            let status = check_swap_at(&swp_path);
            if status == SwapStatus::Orphaned {
                result.push((id, swp_path));
            }
        }
    }
    result.sort_by_key(|(id, _)| *id);
    result
}

/// Check the status of a swap file for a given file path.
pub fn check_swap(file_path: &Path) -> SwapStatus {
    let swp = swap_path(file_path);
    check_swap_at(&swp)
}

/// Check the status of a swap file at a given path.
fn check_swap_at(swp: &Path) -> SwapStatus {
    if !swp.exists() {
        return SwapStatus::None;
    }

    match read_swap(swp) {
        Ok((header, _)) => {
            let our_pid = getpid();
            if header.pid == our_pid {
                SwapStatus::OwnedByUs
            } else if process_alive(header.pid) {
                // Another running zedit instance owns this swap
                SwapStatus::OwnedByUs // Treat as owned — don't interfere
            } else {
                SwapStatus::Orphaned
            }
        }
        Err(_) => SwapStatus::Corrupt,
    }
}

/// Check if a process is alive using kill(pid, 0).
fn process_alive(pid: u32) -> bool {
    // SAFETY: kill with signal 0 just checks if the process exists.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

fn getpid() -> u32 {
    // SAFETY: getpid() is always safe.
    unsafe { libc::getpid() as u32 }
}

fn unix_timestamp() -> u64 {
    // Use libc::time for zero-dep timestamp
    // SAFETY: time(NULL) is always safe.
    unsafe { libc::time(std::ptr::null_mut()) as u64 }
}

/// Get the XDG state directory: $XDG_STATE_HOME or ~/.local/state
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

// We need libc bindings for getpid, kill, time
mod libc {
    unsafe extern "C" {
        pub fn getpid() -> i32;
        pub fn kill(pid: i32, sig: i32) -> i32;
        pub fn time(tloc: *mut i64) -> i64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swap_path() {
        let p = Path::new("/home/user/project/foo.rs");
        let swp = swap_path(p);
        assert_eq!(swp, PathBuf::from("/home/user/project/.foo.rs.swp"));
    }

    #[test]
    fn test_swap_path_untitled() {
        let swp = swap_path_untitled(1);
        assert!(swp.to_string_lossy().contains("NewBuffer01.swp"));
        let swp2 = swap_path_untitled(12);
        assert!(swp2.to_string_lossy().contains("NewBuffer12.swp"));
    }

    #[test]
    fn test_swap_roundtrip() {
        let dir = std::env::temp_dir().join("zedit_swap_test");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("test.txt");
        fs::write(&file_path, "original").unwrap();

        let content = b"modified content here";
        write_swap(&file_path, content, true).unwrap();

        let swp = swap_path(&file_path);
        assert!(swp.exists());

        let (header, recovered) = read_swap(&swp).unwrap();
        assert_eq!(header.original_path, file_path.to_string_lossy());
        assert!(header.modified);
        assert_eq!(recovered, "modified content here");
        assert_eq!(header.pid, getpid());

        // Check status — should be owned by us
        let status = check_swap(&file_path);
        assert_eq!(status, SwapStatus::OwnedByUs);

        // Remove and verify
        remove_swap(&file_path);
        assert!(!swp.exists());
        assert_eq!(check_swap(&file_path), SwapStatus::None);

        // Cleanup
        let _ = fs::remove_file(&file_path);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_swap_corrupt() {
        let dir = std::env::temp_dir().join("zedit_swap_corrupt");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("bad.txt");
        let swp = swap_path(&file_path);

        // Write garbage
        fs::write(&swp, b"not a swap file").unwrap();
        assert_eq!(check_swap(&file_path), SwapStatus::Corrupt);

        let _ = fs::remove_file(&swp);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_orphaned_swap() {
        let dir = std::env::temp_dir().join("zedit_swap_orphan");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("orphan.txt");
        let swp = swap_path(&file_path);

        // Manually write a swap with a dead PID (99999999)
        let path_str = file_path.to_string_lossy();
        let path_bytes = path_str.as_bytes();
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC);
        data.extend_from_slice(&VERSION.to_le_bytes());
        data.extend_from_slice(&99999999u32.to_le_bytes()); // dead PID
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
        data.extend_from_slice(path_bytes);
        data.push(0x01);
        data.extend_from_slice(b"orphaned content");

        fs::write(&swp, &data).unwrap();
        assert_eq!(check_swap(&file_path), SwapStatus::Orphaned);

        let _ = fs::remove_file(&swp);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_process_alive() {
        // Our own PID should be alive
        assert!(process_alive(getpid()));
        // PID 99999999 should not be alive (almost certainly)
        assert!(!process_alive(99999999));
    }
}
