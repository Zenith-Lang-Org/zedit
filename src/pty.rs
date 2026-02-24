// ---------------------------------------------------------------------------
// PTY — pseudo-terminal allocation and child process management
// ---------------------------------------------------------------------------
//
// Uses posix_openpt + grantpt + unlockpt + ptsname_r to avoid linking -lutil.
// The child runs in its own session with the slave fd as stdin/stdout/stderr.

use crate::terminal::Winsize;

// ---------------------------------------------------------------------------
// FFI constants
// ---------------------------------------------------------------------------

const O_RDWR: i32 = 2;
const O_NOCTTY: i32 = 0o400;
const O_NONBLOCK: i32 = 0o4000;

const F_SETFL: i32 = 4;

const TIOCSWINSZ: u64 = 0x5414;
const TIOCSCTTY: u64 = 0x540E;

const WNOHANG: i32 = 1;

pub const POLLIN: i16 = 0x001;
#[allow(dead_code)]
pub const POLLHUP: i16 = 0x010;

const STDIN_FILENO: i32 = 0;
const STDOUT_FILENO: i32 = 1;
const STDERR_FILENO: i32 = 2;

// ---------------------------------------------------------------------------
// FFI declarations
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn posix_openpt(flags: i32) -> i32;
    fn grantpt(fd: i32) -> i32;
    fn unlockpt(fd: i32) -> i32;
    fn ptsname_r(fd: i32, buf: *mut u8, buflen: usize) -> i32;
    fn open(path: *const u8, flags: i32, ...) -> i32;
    fn close(fd: i32) -> i32;
    fn fork() -> i32;
    fn setsid() -> i32;
    fn dup2(oldfd: i32, newfd: i32) -> i32;
    fn execvp(file: *const u8, argv: *const *const u8) -> i32;
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
    fn write(fd: i32, buf: *const u8, count: usize) -> isize;
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn _exit(status: i32) -> !;
}

// ---------------------------------------------------------------------------
// PollFd
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct PollFd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

/// Wrapper around libc poll().
pub fn poll_fds(fds: &mut [PollFd], timeout_ms: i32) -> i32 {
    unsafe { poll(fds.as_mut_ptr(), fds.len() as u64, timeout_ms) }
}

// ---------------------------------------------------------------------------
// Pty
// ---------------------------------------------------------------------------

pub struct Pty {
    master_fd: i32,
    child_pid: i32,
    dead: bool,
}

impl Pty {
    /// Spawn a new PTY with the given shell command.
    /// The child process runs `shell` in a new session.
    pub fn spawn(cols: u16, rows: u16, shell: &str) -> Result<Self, String> {
        // Open master PTY
        let master_fd = unsafe { posix_openpt(O_RDWR | O_NOCTTY) };
        if master_fd < 0 {
            return Err("posix_openpt failed".into());
        }

        if unsafe { grantpt(master_fd) } != 0 {
            unsafe { close(master_fd) };
            return Err("grantpt failed".into());
        }

        if unsafe { unlockpt(master_fd) } != 0 {
            unsafe { close(master_fd) };
            return Err("unlockpt failed".into());
        }

        // Get slave name
        let mut name_buf = [0u8; 256];
        if unsafe { ptsname_r(master_fd, name_buf.as_mut_ptr(), name_buf.len()) } != 0 {
            unsafe { close(master_fd) };
            return Err("ptsname_r failed".into());
        }

        // Set initial window size
        let ws = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            ioctl(master_fd, TIOCSWINSZ, &ws);
        }

        let pid = unsafe { fork() };
        if pid < 0 {
            unsafe { close(master_fd) };
            return Err("fork failed".into());
        }

        if pid == 0 {
            // ---- Child process ----
            unsafe {
                close(master_fd);
                setsid();

                // Open slave PTY
                let slave_fd = open(name_buf.as_ptr(), O_RDWR, 0);
                if slave_fd < 0 {
                    _exit(1);
                }

                // Set controlling terminal
                ioctl(slave_fd, TIOCSCTTY, 0i32);

                // Set window size on slave
                ioctl(slave_fd, TIOCSWINSZ, &ws);

                // Redirect stdin/stdout/stderr
                dup2(slave_fd, STDIN_FILENO);
                dup2(slave_fd, STDOUT_FILENO);
                dup2(slave_fd, STDERR_FILENO);
                if slave_fd > STDERR_FILENO {
                    close(slave_fd);
                }

                // Set TERM environment variable
                libc_setenv(c"TERM".as_ptr().cast(), c"xterm-256color".as_ptr().cast());

                // Build null-terminated shell string
                let mut shell_cstr: Vec<u8> = shell.as_bytes().to_vec();
                shell_cstr.push(0);

                let argv: [*const u8; 2] = [shell_cstr.as_ptr(), std::ptr::null()];
                execvp(shell_cstr.as_ptr(), argv.as_ptr());

                // If execvp returns, it failed — try /bin/sh
                let fallback = b"/bin/sh\0";
                let argv2: [*const u8; 2] = [fallback.as_ptr(), std::ptr::null()];
                execvp(fallback.as_ptr(), argv2.as_ptr());

                _exit(127);
            }
        }

        // ---- Parent process ----
        let mut pty = Pty {
            master_fd,
            child_pid: pid,
            dead: false,
        };
        pty.set_nonblocking();
        Ok(pty)
    }

    /// Set the master fd to non-blocking mode.
    fn set_nonblocking(&mut self) {
        unsafe {
            fcntl(self.master_fd, F_SETFL, O_NONBLOCK);
        }
    }

    /// Resize the PTY.
    pub fn resize(&self, cols: u16, rows: u16) {
        let ws = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            ioctl(self.master_fd, TIOCSWINSZ, &ws);
        }
    }

    /// Write bytes to the PTY master (sends to the child's stdin).
    pub fn write_bytes(&self, data: &[u8]) {
        let mut offset = 0;
        while offset < data.len() {
            let n = unsafe { write(self.master_fd, data[offset..].as_ptr(), data.len() - offset) };
            if n <= 0 {
                break;
            }
            offset += n as usize;
        }
    }

    /// Read available bytes from the PTY master (non-blocking).
    /// Returns the number of bytes read.
    pub fn read_nonblocking(&self, buf: &mut [u8]) -> usize {
        let n = unsafe { read(self.master_fd, buf.as_mut_ptr(), buf.len()) };
        if n > 0 { n as usize } else { 0 }
    }

    /// Check if the child process has exited (non-blocking).
    /// Returns true if the child is dead.
    pub fn reap(&mut self) -> bool {
        if self.dead {
            return true;
        }
        let mut status: i32 = 0;
        let result = unsafe { waitpid(self.child_pid, &mut status, WNOHANG) };
        if result > 0 {
            self.dead = true;
            true
        } else {
            false
        }
    }

    pub fn is_dead(&self) -> bool {
        self.dead
    }

    pub fn master_fd(&self) -> i32 {
        self.master_fd
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        unsafe {
            close(self.master_fd);
        }
        // Child gets SIGHUP when master is closed
    }
}

// ---------------------------------------------------------------------------
// Helper: setenv without libc crate
// ---------------------------------------------------------------------------

unsafe fn libc_setenv(name: *const u8, value: *const u8) {
    unsafe extern "C" {
        fn setenv(name: *const u8, value: *const u8, overwrite: i32) -> i32;
    }
    unsafe {
        setenv(name, value, 1);
    }
}
