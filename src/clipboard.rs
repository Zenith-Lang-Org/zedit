//! System clipboard integration for zedit.
//!
//! Provides a single, unified API used by both the text editor (documents) and
//! the built-in terminal emulator:
//!
//! - [`set`]     — write text to the OS clipboard (arboard + OSC 52)
//! - [`get`]     — read text from the OS clipboard (arboard)
//! - [`set_osc52`] — emit an OSC 52 escape sequence only (for cases where the
//!                   terminal-native path must be preferred over arboard, e.g.
//!                   when operating inside a nested terminal session)
//!
//! # Exception to the zero-deps rule
//! The `arboard` crate (X11 via pure-Rust `x11rb`, Wayland via
//! `smithay-clipboard`) is used here with `default-features = false` (no image
//! support) to avoid requiring `xclip`/`xsel`/`wl-clipboard` CLI tools to be
//! installed on the user's system.  This exception was explicitly approved by
//! the project owner.

// ---------------------------------------------------------------------------
// Base64 (used by OSC 52 to encode clipboard data)
// ---------------------------------------------------------------------------

const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(BASE64_TABLE[((triple >> 18) & 0x3F) as usize] as char);
        result.push(BASE64_TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(BASE64_TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(BASE64_TABLE[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Write `text` to the system clipboard via OSC 52 (terminal-native path).
///
/// This is best-effort: the terminal must support OSC 52 (kitty, alacritty,
/// wezterm, tmux ≥3.3).  The native X11/Wayland path is handled by the Editor
/// via a persistent `arboard::Clipboard` instance — see `Editor::sys_clip_set`.
pub fn set(text: &str) {
    set_osc52(text);
}

/// Emit an OSC 52 escape sequence to write `text` to the terminal's clipboard.
///
/// This is a best-effort write: the terminal must support OSC 52.  Tmux DCS
/// passthrough is applied automatically when `$TMUX` is set.  X11 PRIMARY is
/// written in addition to the CLIPBOARD selection so middle-click paste works.
pub fn set_osc52(text: &str) {
    let encoded = base64_encode(text.as_bytes());
    let seq = if std::env::var("TMUX").is_ok() {
        // Tmux DCS passthrough: each ESC inside must be doubled.
        format!(
            "\x1bPtmux;\x1b\x1b]52;c;{enc}\x07\x1b\\\x1bPtmux;\x1b\x1b]52;p;{enc}\x07\x1b\\",
            enc = encoded
        )
    } else {
        // Standard OSC 52 with ST terminator.
        format!("\x1b]52;c;{enc}\x1b\\\x1b]52;p;{enc}\x1b\\", enc = encoded)
    };
    crate::terminal::write_all(seq.as_bytes());
    crate::terminal::flush();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode_rfc4648() {
        // RFC 4648 test vectors
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn test_base64_encode_utf8() {
        assert_eq!(base64_encode("café".as_bytes()), "Y2Fmw6k=");
    }
}
