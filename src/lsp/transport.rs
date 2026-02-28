// ---------------------------------------------------------------------------
// LSP Transport — pipe + fork + exec for language server child processes
// ---------------------------------------------------------------------------
//
// Uses plain stdin/stdout pipes (not PTY) to communicate with language servers
// via the JSON-RPC protocol with Content-Length framing.

use crate::syntax::json_parser::JsonValue;

// ---------------------------------------------------------------------------
// FFI constants and declarations
// ---------------------------------------------------------------------------

const O_NONBLOCK: i32 = 0o4000;
const F_SETFL: i32 = 4;
const WNOHANG: i32 = 1;

const STDIN_FILENO: i32 = 0;
const STDOUT_FILENO: i32 = 1;
const STDERR_FILENO: i32 = 2;

unsafe extern "C" {
    fn pipe(pipefd: *mut i32) -> i32;
    fn fork() -> i32;
    fn close(fd: i32) -> i32;
    fn dup2(oldfd: i32, newfd: i32) -> i32;
    fn execvp(file: *const u8, argv: *const *const u8) -> i32;
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
    fn write(fd: i32, buf: *const u8, count: usize) -> isize;
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn open(path: *const u8, flags: i32, ...) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
    fn _exit(status: i32) -> !;
}

const O_WRONLY: i32 = 1;
const SIGTERM: i32 = 15;

// ---------------------------------------------------------------------------
// LspTransport
// ---------------------------------------------------------------------------

pub struct LspTransport {
    child_pid: i32,
    stdin_fd: i32,  // parent writes to server's stdin
    stdout_fd: i32, // parent reads from server's stdout
    read_buf: Vec<u8>,
    /// Buffered outgoing bytes that could not be written yet because the pipe
    /// was full.  Flushed at the start of every frame via `flush_pending_writes`.
    write_buf: Vec<u8>,
    dead: bool,
    /// Exit code of the child process, set when reap() detects it has exited.
    last_exit_code: Option<i32>,
}

impl LspTransport {
    /// Spawn a language server process with stdin/stdout pipes.
    pub fn spawn(command: &str, args: &[String]) -> Result<Self, String> {
        // Create two pipe pairs: one for server stdin, one for server stdout
        let mut stdin_pipe = [0i32; 2]; // [read, write]
        let mut stdout_pipe = [0i32; 2]; // [read, write]

        if unsafe { pipe(stdin_pipe.as_mut_ptr()) } != 0 {
            return Err("pipe() failed for stdin".into());
        }
        if unsafe { pipe(stdout_pipe.as_mut_ptr()) } != 0 {
            unsafe {
                close(stdin_pipe[0]);
                close(stdin_pipe[1]);
            }
            return Err("pipe() failed for stdout".into());
        }

        let pid = unsafe { fork() };
        if pid < 0 {
            unsafe {
                close(stdin_pipe[0]);
                close(stdin_pipe[1]);
                close(stdout_pipe[0]);
                close(stdout_pipe[1]);
            }
            return Err("fork() failed".into());
        }

        if pid == 0 {
            // ---- Child process ----
            unsafe {
                // Close parent ends
                close(stdin_pipe[1]); // parent writes to this
                close(stdout_pipe[0]); // parent reads from this

                // Redirect stdin to read end of stdin_pipe
                dup2(stdin_pipe[0], STDIN_FILENO);
                if stdin_pipe[0] != STDIN_FILENO {
                    close(stdin_pipe[0]);
                }

                // Redirect stdout to write end of stdout_pipe
                dup2(stdout_pipe[1], STDOUT_FILENO);
                if stdout_pipe[1] != STDOUT_FILENO {
                    close(stdout_pipe[1]);
                }

                // Redirect stderr to /dev/null
                let devnull = open(b"/dev/null\0".as_ptr(), O_WRONLY);
                if devnull >= 0 {
                    dup2(devnull, STDERR_FILENO);
                    if devnull != STDERR_FILENO {
                        close(devnull);
                    }
                }

                // Build argv for execvp
                let mut cmd_cstr: Vec<u8> = command.as_bytes().to_vec();
                cmd_cstr.push(0);

                let mut arg_cstrs: Vec<Vec<u8>> = Vec::new();
                for arg in args {
                    let mut a: Vec<u8> = arg.as_bytes().to_vec();
                    a.push(0);
                    arg_cstrs.push(a);
                }

                let mut argv_ptrs: Vec<*const u8> = Vec::new();
                argv_ptrs.push(cmd_cstr.as_ptr());
                for a in &arg_cstrs {
                    argv_ptrs.push(a.as_ptr());
                }
                argv_ptrs.push(std::ptr::null());

                execvp(cmd_cstr.as_ptr(), argv_ptrs.as_ptr());
                _exit(127);
            }
        }

        // ---- Parent process ----
        // Close child ends
        unsafe {
            close(stdin_pipe[0]); // child reads from this
            close(stdout_pipe[1]); // child writes to this
        }

        let stdin_fd = stdin_pipe[1]; // parent writes to server stdin
        let stdout_fd = stdout_pipe[0]; // parent reads from server stdout

        // Set both ends to non-blocking.
        // stdout_fd: prevents try_recv from blocking when no data available.
        // stdin_fd:  prevents send() from blocking when the 64KB pipe is full
        //            (which would freeze the main thread during bulk renotify).
        unsafe {
            fcntl(stdout_fd, F_SETFL, O_NONBLOCK);
            fcntl(stdin_fd, F_SETFL, O_NONBLOCK);
        }

        Ok(LspTransport {
            child_pid: pid,
            stdin_fd,
            stdout_fd,
            read_buf: Vec::with_capacity(4096),
            write_buf: Vec::new(),
            dead: false,
            last_exit_code: None,
        })
    }

    /// Send a JSON-RPC message with Content-Length framing.
    ///
    /// Appends the framed message to `write_buf` then flushes what it can
    /// non-blockingly.  Any bytes that couldn't be written yet (pipe full)
    /// stay in `write_buf` and are drained by `flush_pending_writes()` on
    /// the next frame — preventing main-thread blockage.
    pub fn send(&mut self, msg: &JsonValue) -> Result<(), String> {
        if self.dead {
            return Err("transport is dead".into());
        }
        let body = msg.to_json();
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.write_buf.extend_from_slice(header.as_bytes());
        self.write_buf.extend_from_slice(body.as_bytes());
        self.flush_write_buf();
        Ok(())
    }

    /// Flush as many buffered outgoing bytes as the pipe will accept right now.
    /// Stops without error when the pipe is full (`EAGAIN`/`EWOULDBLOCK`).
    /// Marks the transport dead on any real I/O error.
    pub fn flush_pending_writes(&mut self) {
        self.flush_write_buf();
    }

    /// Try to receive a complete JSON-RPC message (non-blocking).
    /// Returns None if no complete message is available yet.
    pub fn try_recv(&mut self) -> Result<Option<JsonValue>, String> {
        if self.dead {
            return Ok(None);
        }

        // Read available bytes into buffer
        let mut tmp = [0u8; 8192];
        loop {
            let n = unsafe { read(self.stdout_fd, tmp.as_mut_ptr(), tmp.len()) };
            if n > 0 {
                self.read_buf.extend_from_slice(&tmp[..n as usize]);
            } else {
                break;
            }
        }

        // Try to parse a complete message from the buffer
        self.try_parse_message()
    }

    /// Get the stdout fd for poll integration.
    pub fn stdout_fd(&self) -> i32 {
        self.stdout_fd
    }

    /// Check if child process has exited (non-blocking).
    pub fn reap(&mut self) -> bool {
        if self.dead {
            return true;
        }
        let mut status: i32 = 0;
        let result = unsafe { waitpid(self.child_pid, &mut status, WNOHANG) };
        if result > 0 {
            self.dead = true;
            // WEXITSTATUS: exit code = (status >> 8) & 0xff
            let code = (status >> 8) & 0xff;
            self.last_exit_code = Some(code);
            crate::dlog!(
                "[lsp_transport] child pid={} exited code={} — \
                 check if the LSP server binary is installed correctly",
                self.child_pid,
                code
            );
            true
        } else {
            false
        }
    }

    /// Return the exit code of the child process, if it has been reaped.
    pub fn last_exit_code(&self) -> Option<i32> {
        self.last_exit_code
    }

    /// Shut down the transport: send SIGTERM, close fds, wait.
    pub fn shutdown(&mut self) {
        if !self.dead {
            unsafe {
                close(self.stdin_fd);
                // Only kill valid PIDs — never kill(-1) or kill(0)
                if self.child_pid > 0 {
                    kill(self.child_pid, SIGTERM);
                    let mut status: i32 = 0;
                    waitpid(self.child_pid, &mut status, 0);
                }
                close(self.stdout_fd);
            }
            self.dead = true;
        }
    }

    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Create a permanently-dead transport with empty buffers, for unit tests
    /// that drive the client state machine via handle_message rather than real I/O.
    #[cfg(test)]
    pub(crate) fn new_dead() -> Self {
        LspTransport {
            child_pid: -1,
            stdin_fd: -1,
            stdout_fd: -1,
            read_buf: Vec::new(),
            write_buf: Vec::new(),
            dead: true,
            last_exit_code: None,
        }
    }

    /// Create a transport that is logically "alive" but backed by pre-populated
    /// incoming data (simulates server bytes already in the OS pipe buffer).
    /// read(-1) fails immediately so try_recv falls through to try_parse_message,
    /// which consumes the pre-loaded `incoming` bytes.
    #[cfg(test)]
    pub(crate) fn new_with_incoming(incoming: Vec<u8>) -> Self {
        LspTransport {
            child_pid: -1,
            stdin_fd: -1,
            stdout_fd: -1,
            read_buf: incoming,
            write_buf: Vec::new(),
            dead: false,
            last_exit_code: None,
        }
    }

    // -- Internal helpers --

    /// Non-blocking drain of `write_buf` into the LSP server's stdin pipe.
    ///
    /// - Returns immediately when the pipe is full (`EAGAIN = 11`).
    /// - Marks transport dead on any other write error.
    fn flush_write_buf(&mut self) {
        if self.write_buf.is_empty() || self.dead || self.stdin_fd < 0 {
            return;
        }
        let mut offset = 0;
        while offset < self.write_buf.len() {
            let n = unsafe {
                write(
                    self.stdin_fd,
                    self.write_buf[offset..].as_ptr(),
                    self.write_buf.len() - offset,
                )
            };
            if n > 0 {
                offset += n as usize;
            } else {
                // n == -1  →  check errno
                // n == 0   →  unexpected, treat as error
                let errno = std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or(0);
                if errno == 11 {
                    // EAGAIN / EWOULDBLOCK: pipe buffer full, retry next frame
                    break;
                }
                // Real error (EPIPE, EBADF, …): server died
                crate::dlog!(
                    "[lsp_transport] write error errno={} — marking transport dead",
                    errno
                );
                self.dead = true;
                break;
            }
        }
        if offset > 0 {
            self.write_buf.drain(..offset);
        }
    }

    fn try_parse_message(&mut self) -> Result<Option<JsonValue>, String> {
        // Look for Content-Length header followed by \r\n\r\n
        let header_end = match find_header_end(&self.read_buf) {
            Some(pos) => pos,
            None => return Ok(None),
        };

        let header_bytes = &self.read_buf[..header_end];
        let content_length = match parse_content_length(header_bytes) {
            Some(len) => len,
            None => {
                // Malformed header — skip past it
                self.read_buf.drain(..header_end + 4);
                return Err("malformed Content-Length header".into());
            }
        };

        let body_start = header_end + 4; // skip \r\n\r\n
        let total_needed = body_start + content_length;

        if self.read_buf.len() < total_needed {
            return Ok(None); // body not fully received yet
        }

        // Extract body
        let body_bytes = self.read_buf[body_start..total_needed].to_vec();

        // ALWAYS consume the message bytes first — this prevents a bad message
        // from permanently blocking the buffer and dropping all subsequent ones.
        self.read_buf.drain(..total_needed);

        let body_str = std::str::from_utf8(&body_bytes)
            .map_err(|_| "invalid UTF-8 in LSP message body".to_string())?;
        let value = JsonValue::parse(body_str)
            .map_err(|e| format!("JSON parse error in LSP message: {}", e))?;

        Ok(Some(value))
    }
}

impl Drop for LspTransport {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Header parsing helpers
// ---------------------------------------------------------------------------

/// Find the position of \r\n\r\n in the buffer (returns position of first \r).
fn find_header_end(buf: &[u8]) -> Option<usize> {
    if buf.len() < 4 {
        return None;
    }
    for i in 0..buf.len() - 3 {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some(i);
        }
    }
    None
}

/// Parse Content-Length value from header bytes.
fn parse_content_length(header: &[u8]) -> Option<usize> {
    let s = std::str::from_utf8(header).ok()?;
    for line in s.split("\r\n") {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("Content-Length:") {
            return val.trim().parse().ok();
        }
        // Case-insensitive check
        let lower = line.to_ascii_lowercase();
        if let Some(val) = lower.strip_prefix("content-length:") {
            return val.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_header_end() {
        let data = b"Content-Length: 10\r\n\r\n{\"test\":1}";
        assert_eq!(find_header_end(data), Some(18));
    }

    #[test]
    fn test_find_header_end_not_found() {
        let data = b"Content-Length: 10\r\n";
        assert_eq!(find_header_end(data), None);
    }

    #[test]
    fn test_parse_content_length() {
        let header = b"Content-Length: 42";
        assert_eq!(parse_content_length(header), Some(42));
    }

    #[test]
    fn test_parse_content_length_case_insensitive() {
        let header = b"content-length: 100";
        assert_eq!(parse_content_length(header), Some(100));
    }

    #[test]
    fn test_parse_content_length_missing() {
        let header = b"Content-Type: application/json";
        assert_eq!(parse_content_length(header), None);
    }

    #[test]
    fn test_try_parse_complete_message() {
        let mut transport = LspTransport {
            child_pid: -1,
            stdin_fd: -1,
            stdout_fd: -1,
            read_buf: Vec::new(),
            write_buf: Vec::new(),
            dead: true,
            last_exit_code: None,
        };
        let body = r#"{"jsonrpc":"2.0","method":"test"}"#;
        let framed = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        transport.read_buf = framed.into_bytes();

        // Can't use try_recv (it reads from fd), so test parse directly
        let result = transport.try_parse_message().unwrap();
        assert!(result.is_some());
        let msg = result.unwrap();
        assert_eq!(msg.get("method").unwrap().as_str(), Some("test"));
        assert!(transport.read_buf.is_empty());
    }

    #[test]
    fn test_try_parse_incomplete_message() {
        let mut transport = LspTransport {
            child_pid: -1,
            stdin_fd: -1,
            stdout_fd: -1,
            read_buf: b"Content-Length: 100\r\n\r\n{\"partial".to_vec(),
            write_buf: Vec::new(),
            dead: true,
            last_exit_code: None,
        };
        let result = transport.try_parse_message().unwrap();
        assert!(result.is_none()); // body not complete
    }
}
