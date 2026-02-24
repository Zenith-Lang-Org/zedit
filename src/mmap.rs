/// Memory-mapped file support for read-only large-file access.
///
/// Uses POSIX `mmap(2)` directly (no external crates), following the same
/// inline FFI pattern as `src/terminal.rs`.
///
/// Files opened via `Mmap` are mapped read-only (`PROT_READ | MAP_PRIVATE`).
/// The OS handles paging: only pages that are actually accessed consume RAM.
/// This lets zedit open 500MB log files in < 10ms without pre-loading content.
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;

// ── Linux / POSIX constants ───────────────────────────────────────

const PROT_READ: i32 = 0x1;
const MAP_PRIVATE: i32 = 0x02;
const MADV_SEQUENTIAL: i32 = 2;

// MAP_FAILED is (void *)-1, which on 64-bit equals usize::MAX as *mut u8.
const MAP_FAILED: usize = usize::MAX;

// ── Inline FFI declarations ───────────────────────────────────────

unsafe extern "C" {
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, offset: i64)
        -> *mut u8;
    fn munmap(addr: *mut u8, len: usize) -> i32;
    fn madvise(addr: *mut u8, len: usize, advice: i32) -> i32;
}

// ── Mmap struct ───────────────────────────────────────────────────

/// A read-only memory-mapped view of a file on disk.
///
/// `Send + Sync` because the mapping is `PROT_READ` only; no mutation
/// is possible through the pointer and the backing file is not moved.
pub struct Mmap {
    ptr: *const u8,
    len: usize,
}

impl Mmap {
    /// Open `path` read-only via `mmap(2)`.
    ///
    /// Returns an `io::Error` if the file cannot be opened, the `fstat` fails,
    /// or the kernel `mmap` call returns `MAP_FAILED`.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len() as usize;

        if len == 0 {
            // Zero-length file: dangling-but-non-null pointer; `as_bytes`
            // returns an empty slice whenever len == 0.
            return Ok(Mmap {
                ptr: std::ptr::NonNull::<u8>::dangling().as_ptr(),
                len: 0,
            });
        }

        // SAFETY: `file` is open (fd valid), `len` > 0, MAP_PRIVATE|PROT_READ
        //         creates a private read-only page mapping.
        let raw = unsafe {
            mmap(
                std::ptr::null_mut(),
                len,
                PROT_READ,
                MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };

        if raw as usize == MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        // Advisory hint: tell the kernel we will access pages sequentially.
        // Errors are intentionally ignored — this is advisory only.
        unsafe { madvise(raw, len, MADV_SEQUENTIAL) };

        Ok(Mmap {
            ptr: raw as *const u8,
            len,
        })
    }

    /// Immutable byte-slice view of the mapped region.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: `ptr` is valid for `len` bytes for the entire lifetime of
        // `self`; the mapping is `PROT_READ` so no aliased mutation is possible.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Length of the mapped region in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        if self.len > 0 {
            // SAFETY: `ptr` + `len` come from a successful `mmap` call.
            unsafe { munmap(self.ptr as *mut u8, self.len) };
        }
    }
}

// SAFETY: The mapping is `PROT_READ`; no mutation through the pointer is
// possible, so sharing across threads is safe.
unsafe impl Send for Mmap {}
unsafe impl Sync for Mmap {}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Write `content` to a uniquely-named temp file and return its path.
    fn write_tmp(content: &[u8]) -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!("zedit_mmap_{}_{}.bin", std::process::id(), n));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    #[test]
    fn test_mmap_basic_read() {
        let path = write_tmp(b"hello mmap");
        let map = Mmap::open(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(map.len(), 10);
        assert_eq!(map.as_bytes(), b"hello mmap");
    }

    #[test]
    fn test_mmap_empty_file() {
        let path = write_tmp(b"");
        let map = Mmap::open(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(map.len(), 0);
        assert!(map.as_bytes().is_empty());
    }

    #[test]
    fn test_mmap_large_file() {
        let content = vec![b'L'; 4 * 1024 * 1024]; // 4 MB
        let path = write_tmp(&content);
        let map = Mmap::open(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(map.len(), 4 * 1024 * 1024);
        assert_eq!(map.as_bytes()[0], b'L');
        assert_eq!(map.as_bytes()[map.len() - 1], b'L');
    }

    #[test]
    fn test_mmap_nonexistent_file_errors() {
        let result = Mmap::open(std::path::Path::new("/nonexistent/zedit_test_file.bin"));
        assert!(result.is_err());
    }
}
