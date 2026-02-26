//! Virtual memory region: reserve a large address space, commit pages on demand.
//!
//! Used by the gap buffer (Phase 36) to avoid `Vec::resize()` reallocation on
//! large files.  On creation, `mmap(PROT_NONE)` reserves virtual address space
//! without consuming any RAM.  Bytes become accessible only after `grow(n)` which
//! calls `mprotect(PROT_READ|WRITE)` on the next required chunk.
//!
//! SAFETY: Unix only (Linux / macOS). Uses mmap(2) + mprotect(2).

use std::io;
use std::ptr::NonNull;

// ── POSIX constants ───────────────────────────────────────────────

const PROT_NONE: i32 = 0x0;
const PROT_READ: i32 = 0x1;
const PROT_WRITE: i32 = 0x2;
const MAP_PRIVATE: i32 = 0x02;

// MAP_ANONYMOUS differs between Linux (0x20) and macOS (0x1000).
#[cfg(target_os = "linux")]
const MAP_ANONYMOUS: i32 = 0x20;
#[cfg(target_os = "macos")]
const MAP_ANONYMOUS: i32 = 0x1000;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const MAP_ANONYMOUS: i32 = 0x20; // best-effort fallback for other Unix

// MAP_FAILED is (void *)-1, which equals usize::MAX on all platforms.
const MAP_FAILED: usize = usize::MAX;

const EINVAL: i32 = 22;
const ENOMEM: i32 = 12;

// ── Inline FFI declarations ───────────────────────────────────────

unsafe extern "C" {
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut u8;
    fn munmap(addr: *mut u8, len: usize) -> i32;
    fn mprotect(addr: *mut u8, len: usize, prot: i32) -> i32;
}

// ── Constants ─────────────────────────────────────────────────────

/// Granularity of each `mprotect` commit call.
///
/// Linux: 64 KB — matches page granularity and amortises syscall overhead.
/// macOS: 2 MB — `mprotect` on macOS has higher per-call overhead so larger
///        chunks reduce the syscall rate at the cost of slightly more eager commit.
#[cfg(target_os = "macos")]
pub const COMMIT_CHUNK: usize = 2 * 1024 * 1024;
#[cfg(not(target_os = "macos"))]
pub const COMMIT_CHUNK: usize = 64 * 1024;

/// Files at or above this size use `VirtualRegion` instead of `Vec<u8>`.
/// Below this threshold the heap allocation overhead is negligible.
pub const VMEM_THRESHOLD: usize = 128 * 1024;

/// Total virtual address space reserved per gap buffer.
/// This is virtual, not physical — no RAM is used until `grow()` is called.
#[cfg(target_pointer_width = "64")]
pub const VMEM_RESERVE: usize = 2 * 1024 * 1024 * 1024; // 2 GB
#[cfg(target_pointer_width = "32")]
pub const VMEM_RESERVE: usize = 128 * 1024 * 1024; // 128 MB

// ── VirtualRegion ─────────────────────────────────────────────────

/// A reserved virtual memory region.  Bytes are committed lazily via `grow()`.
///
/// The region never moves in memory — gap operations inside the buffer are
/// in-place `ptr::copy` calls within the same reserved range.
pub struct VirtualRegion {
    base: NonNull<u8>,
    reserved: usize,
    committed: usize,
}

impl VirtualRegion {
    /// Reserve `size` bytes of virtual address space.  No RAM is allocated.
    pub fn reserve(size: usize) -> io::Result<Self> {
        if size == 0 {
            return Err(io::Error::from_raw_os_error(EINVAL));
        }
        let ptr = unsafe {
            mmap(
                std::ptr::null_mut(),
                size,
                PROT_NONE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if ptr as usize == MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            base: unsafe { NonNull::new_unchecked(ptr) },
            reserved: size,
            committed: 0,
        })
    }

    /// Commit at least `needed` bytes (from offset 0) with read/write access.
    /// Pages are committed in `COMMIT_CHUNK` multiples to amortise syscalls.
    pub fn grow(&mut self, needed: usize) -> io::Result<()> {
        if needed <= self.committed {
            return Ok(());
        }
        if needed > self.reserved {
            return Err(io::Error::from_raw_os_error(ENOMEM));
        }
        // Round up to next COMMIT_CHUNK boundary.
        let chunks = (needed + COMMIT_CHUNK - 1) / COMMIT_CHUNK;
        let new_committed = (chunks * COMMIT_CHUNK).min(self.reserved);
        let ptr = unsafe { self.base.as_ptr().add(self.committed) };
        let len = new_committed - self.committed;
        let rc = unsafe { mprotect(ptr, len, PROT_READ | PROT_WRITE) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        self.committed = new_committed;
        Ok(())
    }

    /// Raw pointer to byte 0 of the region.
    #[inline]
    pub fn as_ptr(&self) -> *mut u8 {
        self.base.as_ptr()
    }

    /// Number of bytes currently committed (readable and writable).
    #[allow(dead_code)]
    #[inline]
    pub fn committed(&self) -> usize {
        self.committed
    }
}

impl Drop for VirtualRegion {
    fn drop(&mut self) {
        unsafe {
            munmap(self.base.as_ptr(), self.reserved);
        }
    }
}

// SAFETY: MAP_PRIVATE — the region is not shared with other processes.
// All access is gated through `&mut VirtualRegion`.
unsafe impl Send for VirtualRegion {}
unsafe impl Sync for VirtualRegion {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_region_reserve_and_grow() {
        let mut region = VirtualRegion::reserve(16 * 1024 * 1024).unwrap();
        assert_eq!(region.committed(), 0);

        region.grow(1).unwrap();
        assert!(region.committed() >= COMMIT_CHUNK);

        // Write and read back through the committed page.
        unsafe {
            *region.as_ptr() = 42;
            assert_eq!(*region.as_ptr(), 42);
        }
    }

    #[test]
    fn virtual_region_grow_multiple_times() {
        let mut region = VirtualRegion::reserve(4 * 1024 * 1024).unwrap();
        region.grow(1).unwrap();
        let c1 = region.committed();
        region.grow(c1 + 1).unwrap();
        assert!(region.committed() > c1);
        // Idempotent: growing to already-committed amount is a no-op.
        let c2 = region.committed();
        region.grow(c2).unwrap();
        assert_eq!(region.committed(), c2);
    }

    #[test]
    fn virtual_region_exceeds_reserve_fails() {
        let mut region = VirtualRegion::reserve(COMMIT_CHUNK).unwrap();
        assert!(region.grow(COMMIT_CHUNK + 1).is_err());
    }

    #[test]
    fn virtual_region_write_read_large() {
        let size = 3 * COMMIT_CHUNK;
        let mut region = VirtualRegion::reserve(size).unwrap();
        region.grow(size).unwrap();
        assert_eq!(region.committed(), size);
        // Write to first and last byte.
        unsafe {
            *region.as_ptr() = 0xAA;
            *region.as_ptr().add(size - 1) = 0xBB;
            assert_eq!(*region.as_ptr(), 0xAA);
            assert_eq!(*region.as_ptr().add(size - 1), 0xBB);
        }
    }
}
