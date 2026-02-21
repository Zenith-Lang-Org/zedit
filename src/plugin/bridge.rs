// ---------------------------------------------------------------------------
// Plugin IPC bridge — newline-delimited JSON over stdin/stdout pipes
// ---------------------------------------------------------------------------
//
// Same fork/exec/pipe pattern as src/lsp/transport.rs, but uses newline-
// delimited JSON instead of Content-Length-framed JSON-RPC.

use crate::syntax::json_parser::JsonValue;

// ---------------------------------------------------------------------------
// FFI
// ---------------------------------------------------------------------------

const O_NONBLOCK: i32 = 0o4000;
const F_SETFL: i32 = 4;
const WNOHANG: i32 = 1;
const STDIN_FILENO: i32 = 0;
const STDOUT_FILENO: i32 = 1;
const STDERR_FILENO: i32 = 2;
const O_WRONLY: i32 = 1;
const SIGTERM: i32 = 15;

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

// ---------------------------------------------------------------------------
// PluginTransport
// ---------------------------------------------------------------------------

pub struct PluginTransport {
    child_pid: i32,
    stdin_fd: i32,  // parent writes to plugin's stdin
    stdout_fd: i32, // parent reads from plugin's stdout
    read_buf: Vec<u8>,
    dead: bool,
}

impl PluginTransport {
    /// Spawn a plugin process with stdin/stdout pipes.
    pub fn spawn(command: &str, args: &[String]) -> Result<Self, String> {
        let mut stdin_pipe = [0i32; 2];
        let mut stdout_pipe = [0i32; 2];

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
                close(stdin_pipe[1]);
                close(stdout_pipe[0]);

                dup2(stdin_pipe[0], STDIN_FILENO);
                if stdin_pipe[0] != STDIN_FILENO {
                    close(stdin_pipe[0]);
                }

                dup2(stdout_pipe[1], STDOUT_FILENO);
                if stdout_pipe[1] != STDOUT_FILENO {
                    close(stdout_pipe[1]);
                }

                let devnull = open(b"/dev/null\0".as_ptr(), O_WRONLY);
                if devnull >= 0 {
                    dup2(devnull, STDERR_FILENO);
                    if devnull != STDERR_FILENO {
                        close(devnull);
                    }
                }

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
        unsafe {
            close(stdin_pipe[0]);
            close(stdout_pipe[1]);
        }

        let stdin_fd = stdin_pipe[1];
        let stdout_fd = stdout_pipe[0];

        unsafe {
            fcntl(stdout_fd, F_SETFL, O_NONBLOCK);
        }

        Ok(PluginTransport {
            child_pid: pid,
            stdin_fd,
            stdout_fd,
            read_buf: Vec::with_capacity(4096),
            dead: false,
        })
    }

    /// Send a JSON value as a single newline-terminated line.
    pub fn send_line(&mut self, msg: &JsonValue) -> Result<(), String> {
        if self.dead {
            return Err("transport is dead".into());
        }
        let mut line = msg.to_json();
        line.push('\n');
        self.write_all(line.as_bytes())
    }

    /// Try to receive one complete line of JSON (non-blocking).
    /// Returns None if no complete line is available yet.
    pub fn try_recv_line(&mut self) -> Option<JsonValue> {
        if self.dead {
            return None;
        }

        // Read available bytes into buffer
        let mut tmp = [0u8; 4096];
        loop {
            let n = unsafe { read(self.stdout_fd, tmp.as_mut_ptr(), tmp.len()) };
            if n > 0 {
                self.read_buf.extend_from_slice(&tmp[..n as usize]);
            } else {
                break;
            }
        }

        // Look for a newline
        if let Some(pos) = self.read_buf.iter().position(|&b| b == b'\n') {
            let line_bytes = self.read_buf[..pos].to_vec();
            self.read_buf.drain(..pos + 1);
            if let Ok(s) = std::str::from_utf8(&line_bytes) {
                JsonValue::parse(s.trim()).ok()
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn stdout_fd(&self) -> i32 {
        self.stdout_fd
    }

    pub fn is_dead(&self) -> bool {
        self.dead
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
            true
        } else {
            false
        }
    }

    pub fn shutdown(&mut self) {
        if !self.dead {
            unsafe {
                close(self.stdin_fd);
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

    fn write_all(&self, data: &[u8]) -> Result<(), String> {
        let mut offset = 0;
        while offset < data.len() {
            let n = unsafe { write(self.stdin_fd, data[offset..].as_ptr(), data.len() - offset) };
            if n <= 0 {
                return Err("write to plugin stdin failed".into());
            }
            offset += n as usize;
        }
        Ok(())
    }
}

impl Drop for PluginTransport {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_recv_line_complete() {
        let mut t = PluginTransport {
            child_pid: -1,
            stdin_fd: -1,
            stdout_fd: -1,
            read_buf: b"{\"method\":\"ping\"}\n".to_vec(),
            dead: false,
        };
        let msg = t.try_recv_line();
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(msg.get("method").and_then(|v| v.as_str()), Some("ping"));
        assert!(t.read_buf.is_empty());
    }

    #[test]
    fn test_try_recv_line_incomplete() {
        let mut t = PluginTransport {
            child_pid: -1,
            stdin_fd: -1,
            stdout_fd: -1,
            read_buf: b"{\"method\":\"ping\"".to_vec(),
            dead: false,
        };
        // No newline yet — should return None
        assert!(t.try_recv_line().is_none());
        // Buffer should still have the partial data
        assert!(!t.read_buf.is_empty());
    }

    #[test]
    fn test_try_recv_line_dead() {
        let mut t = PluginTransport {
            child_pid: -1,
            stdin_fd: -1,
            stdout_fd: -1,
            read_buf: b"{\"method\":\"ping\"}\n".to_vec(),
            dead: true,
        };
        // Dead transport returns None immediately
        assert!(t.try_recv_line().is_none());
    }

    #[test]
    fn test_try_recv_multiple_lines() {
        let mut t = PluginTransport {
            child_pid: -1,
            stdin_fd: -1,
            stdout_fd: -1,
            read_buf: b"{\"a\":1}\n{\"b\":2}\n".to_vec(),
            dead: false,
        };
        let first = t.try_recv_line();
        assert!(first.is_some());
        let second = t.try_recv_line();
        assert!(second.is_some());
        assert!(t.try_recv_line().is_none());
    }
}
