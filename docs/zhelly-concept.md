# Zhelly — Concept Document

> **Status**: Concept / Pre-fork notes
> **Date**: 2026-02-23
> **Author**: Zedit team
> **Origin**: Analysis of zedit's embedded terminal implementation

---

## What is Zhelly?

**Zhelly** is a potential standalone terminal emulator (and eventually a shell host)
forked from zedit's PTY + VTerm infrastructure. The name follows the Z-ecosystem
convention: Zenith, Zymbol, Zedit → **Zhelly**.

The idea arose from the observation that zedit already contains a production-grade,
zero-dependency terminal emulator. Extracting it as a standalone application would
produce a minimal, fast, extensible terminal emulator written in pure Rust + libc FFI.

---

## Zedit's Terminal: What Already Exists

### Three-layer architecture

```
src/pty.rs        ← Real PTY allocation (posix_openpt + fork + execvp)
src/vterm.rs      ← VT100/xterm state machine (escape sequence interpreter)
src/editor/mod.rs ← Terminal panel integration inside the editor layout
```

### pty.rs — The pseudo-terminal layer

- `posix_openpt()` + `grantpt()` + `unlockpt()` + `ptsname_r()` (no libutil dep)
- Real `fork()` → child does `setsid()` + `execvp($SHELL)`
- Master fd set to `O_NONBLOCK` for event-driven I/O
- `TIOCSWINSZ` ioctl to sync window size to PTY
- Dead child detection via `waitpid(WNOHANG)`
- Sets `TERM=xterm-256color` in child environment before exec

### vterm.rs — The terminal emulator state machine

Full VT100/xterm emulation:

| Category   | Supported |
|------------|-----------|
| Cursor     | CUP, CUU, CUD, CUF, CUB, CNL, CPL, CHA, VPA |
| Erase      | ED, EL, DCH, ICH, ECH |
| Scroll     | SU, SD, DECSTBM (scroll region) |
| Graphics   | SGR — bold, italic, underline, inverse, ANSI, 256-color, 24-bit RGB |
| Modes      | DECSET/DECRST — autowrap (7), cursor visibility (25), alt screen (1049) |
| Title      | OSC 0/2 |
| Reports    | DSR 6n (cursor position) |
| UTF-8      | Incremental state-machine decoder (no allocations per byte) |
| Scrollback | 1000-line buffer (in-memory `Vec<Vec<VTermCell>>`) |
| Selection  | Mouse drag + Shift+Arrow with visual inversion |

### Shell prompt clarification

The prompt (`user@host:~/path$`) is **entirely controlled by the shell** (PS1/PROMPT
variables in `.bashrc` / `.zshrc`). VTerm only interprets and renders the escape
sequences the shell emits. The terminal emulator has no knowledge of prompt structure.

To let shell configs detect they are running inside Zhelly, the PTY should inject:

```rust
// In pty.rs, child side, before execvp:
libc_setenv(c"TERM_PROGRAM".as_ptr().cast(), c"zhelly".as_ptr().cast());
```

Shell configs can then branch on `$TERM_PROGRAM`:

```bash
# ~/.bashrc
if [ "$TERM_PROGRAM" = "zhelly" ]; then
    PS1='[zhelly] \u@\h:\w\$ '
fi
```

This is the same pattern used by VS Code (`TERM_PROGRAM=vscode`) and
WezTerm (`TERM_PROGRAM=WezTerm`).

---

## Why fastfetch Shows "zedit" as the Terminal Name

fastfetch detects the terminal by reading the **parent process name** from
`/proc/$PPID/cmdline`. Since bash is a child of zedit's PTY fork, zedit
appears as the terminal. This is expected and correct behavior — Zhelly
would appear the same way.

---

## Fork Scope: What to Extract

A Zhelly fork would extract and keep:

| Module | Keep as-is | Notes |
|--------|-----------|-------|
| `src/pty.rs` | Yes | Core PTY logic, minimal changes needed |
| `src/vterm.rs` | Yes | Complete state machine, production-ready |
| `src/terminal.rs` | Partial | Keep raw mode, SIGWINCH, resize; drop editor-specific parts |
| `src/input.rs` | Partial | Keep escape sequence decoding, mouse, bracketed paste |
| `src/editor/view.rs` | Extract renderer | Diff-based cell renderer, adapt for full-screen |

What to leave behind (editor-specific):
- `src/editor/buffer_state.rs`, `src/editor/editing.rs`, `src/editor/selection.rs`
- `src/syntax/` (highlighting, grammar, tokenizer)
- `src/lsp/` (language server protocol)
- `src/filetree.rs`, `src/vsix_import.rs`, `src/config.rs` (editor config)

---

## Zhelly Architecture (Proposed)

```
zhelly/
├── src/
│   ├── main.rs          ← Entry point: raw mode, main loop, event dispatch
│   ├── pty.rs           ← PTY allocation + fork/exec (from zedit)
│   ├── vterm.rs         ← VT100/xterm state machine (from zedit)
│   ├── terminal.rs      ← Raw mode, SIGWINCH, mouse setup (adapted)
│   ├── input.rs         ← Escape sequence decoder (from zedit)
│   ├── render.rs        ← Diff-based cell renderer (extracted from view.rs)
│   └── config.rs        ← Zhelly-specific config (keybindings, scrollback, etc.)
├── themes/              ← Color schemes (VTerm cell colors)
├── Cargo.toml
└── README.md
```

### Main loop (conceptual)

```
poll(pty_master_fd, stdin_fd, timeout=50ms)
  ├── stdin readable  → decode key/mouse → send bytes to PTY master
  ├── pty readable    → read bytes → VTerm::process_byte() → mark dirty cells
  └── timeout         → if dirty → diff render dirty cells to stdout
```

---

## Color Pipeline: Bytes → Cell → Screen

This is one of the most important areas for Zhelly customization. The entire pipeline
is implemented in pure Rust with no external dependencies, giving full control over
how colors are stored, transformed, and emitted.

### The Color type (`render.rs`)

```rust
pub enum Color {
    Default,          // delegate to terminal host (emits \x1b[39m / \x1b[49m)
    Ansi(u8),         // 0–15: standard + bright ANSI palette
    Color256(u8),     // 0–255: 256-color palette
    Rgb(u8, u8, u8),  // 24-bit true color
}
```

### Stage 1 — SGR parsing into VTermCell (`vterm.rs:execute_sgr`)

When the shell emits `\x1b[38;2;255;128;0m` (RGB foreground), `execute_sgr()`
captures it into the active SGR state. The next `put_char()` call stamps that
state onto every cell written:

```rust
// vterm.rs:put_char()
self.cells[idx] = VTermCell {
    ch,
    fg: self.cur_fg,   // e.g. Color::Rgb(255, 128, 0)
    bg: self.cur_bg,   // e.g. Color::Default
    bold: self.cur_bold,
    italic: self.cur_italic,
    underline: self.cur_underline,
    inverse: self.cur_inverse,
};
```

Supported SGR codes:
- `0` → reset all
- `1/22` → bold on/off
- `3/23` → italic on/off
- `4/24` → underline on/off
- `7/27` → inverse on/off
- `30–37` / `90–97` → ANSI foreground (normal + bright)
- `40–47` / `100–107` → ANSI background (normal + bright)
- `38;5;N` / `48;5;N` → 256-color fg/bg
- `38;2;R;G;B` / `48;2;R;G;B` → RGB true-color fg/bg
- `39` / `49` → reset fg/bg to default

**Intervention point**: color remapping before storage.
A palette override table applied here would remap ANSI colors to theme-specific
RGB values before they reach the cell grid:

```rust
// Hypothetical palette remapping in execute_sgr()
fn remap_color(c: Color) -> Color {
    match c {
        Color::Ansi(1) => Color::Rgb(220, 50,  47),  // red   → Solarized red
        Color::Ansi(2) => Color::Rgb(133, 153,  0),  // green → Solarized green
        Color::Ansi(4) => Color::Rgb( 38, 139, 210), // blue  → Solarized blue
        other          => other,
    }
}
```

### Stage 2 — VTermCell → CellStyle (`view.rs:render_terminal_pane`)

The render function reads each `VTermCell` and maps it to a `CellStyle` for the
diff-based `Screen`. Selection inversion is applied here:

```rust
// view.rs:1823
let style = CellStyle {
    fg: if selected { cell.bg } else { cell.fg },
    bg: if selected { cell.fg } else { cell.bg },
    bold: cell.bold,
    underline: cell.underline,
    inverse: cell.inverse != selected,
    italic: cell.italic,
};
self.screen.put_cell_styled(screen_row, screen_col, cell.ch, style);
```

**Intervention point**: resolving `Color::Default` against the active theme.
Currently `Color::Default` passes through to the terminal host, which resolves it
as whatever the outer terminal's default colors are. In Zhelly, this is the cleanest
place to substitute the theme's actual background and foreground:

```rust
// Hypothetical: resolve Default against Zhelly's active theme
let theme_fg = theme.foreground; // e.g. Color::Rgb(200, 200, 185)
let theme_bg = theme.background; // e.g. Color::Rgb( 28,  28,  28)

let fg = match cell.fg {
    Color::Default => theme_fg,
    other          => other,
};
let bg = match cell.bg {
    Color::Default => theme_bg,
    other          => other,
};
```

### Stage 3 — Color emission to stdout (`render.rs:write_fg/bg_color`)

The final stage converts `Color` variants to ANSI escape sequences written into
the output buffer:

```rust
fn write_fg_color(buf: &mut Vec<u8>, color: Color, mode: &ColorMode) {
    match effective_color(color, mode) {
        Color::Default      => buf.extend_from_slice(b"\x1b[39m"),
        Color::Ansi(n)      => /* \x1b[3Xm or \x1b[9Xm */
        Color::Color256(n)  => /* \x1b[38;5;Nm */
        Color::Rgb(r, g, b) => /* \x1b[38;2;R;G;Bm */
    }
}
```

The `effective_color()` function handles automatic color downgrade based on the
detected terminal capability (`ColorMode`):

```
TrueColor  → Rgb passes through unchanged
Color256   → Rgb is mapped to nearest Color256 via rgb_to_ansi256()
Color16    → Rgb → Color256 → Ansi(0–15) via ansi256_to_ansi16()
```

**Intervention point**: force-upgrade or force-downgrade colors globally,
or inject custom emission formats (e.g., Kitty color stack).

### Full pipeline diagram

```
Shell bytes
    │
    ▼
VTerm::execute_sgr()          ← [A] remap ANSI palette to theme RGB here
    │  stores Color into cur_fg/cur_bg
    ▼
VTerm::put_char()
    │  stamps VTermCell { ch, fg, bg, bold, italic, underline, inverse }
    ▼
VTermCell grid (in-memory)
    │
    ▼
render_terminal_pane()        ← [B] resolve Color::Default to theme colors here
    │  builds CellStyle, applies selection inversion
    ▼
Screen::put_cell_styled()
    │  diff against prev frame (only changed cells emitted)
    ▼
write_fg/bg_color()           ← [C] downgrade RGB→256→16, emit ANSI sequences
    │
    ▼
stdout (terminal host)
```

### What Zhelly can fully control

| What | How | Where |
|------|-----|-------|
| ANSI palette remapping | Override `Color::Ansi(n)` → `Color::Rgb(r,g,b)` before storage | `vterm.rs:execute_sgr()` |
| Default color resolution | Replace `Color::Default` with theme fg/bg | `view.rs:render_terminal_pane()` |
| Color depth downgrade | Customize `effective_color()` thresholds | `render.rs:effective_color()` |
| Emission format | Change ANSI sequence style (SGR, Kitty, etc.) | `render.rs:write_fg/bg_color()` |
| Selection highlight | Change inversion logic or use RGB highlight | `view.rs:render_terminal_pane()` |

Since everything is pure Rust with no external renderer, Zhelly has **total control**
over every pixel of color that reaches the screen, without any protocol negotiation
or terminal capability queries for color handling.

---

## Extension Points

The VTerm state machine is designed to be extended. Key hooks:

| Feature | Where to add |
|---------|-------------|
| New CSI sequences | `VTerm::execute_csi()` |
| New DECSET modes  | `VTerm::decset()` |
| OSC hyperlinks (OSC 8) | `VTerm::execute_osc()` |
| Sixel / iTerm2 graphics | Cell type extension in `vterm.rs` |
| Kitty keyboard protocol | `input.rs` + `pty.rs` env injection |
| Tabs / multiplexer | Multiple `Pty` + `VTerm` pairs, tab-strip renderer |
| Shell integration | `TERM_PROGRAM` + OSC 133 prompt markers |

---

## Current Limitations (Known Technical Debt)

1. **No Sixel / iTerm2 graphics** — no image rendering
2. **Scrollback hardcoded** — 1000 lines, not configurable
3. **No reflow on resize** — existing content does not reflow when PTY size changes
4. **No multiplexing** — no tmux-like tab/split built into the terminal layer
5. **Basic mouse** — SGR format supported, but no complex button sequences
6. **No OSC 8 hyperlinks** — clickable URLs not implemented
7. **No Kitty keyboard protocol** — would improve key disambiguation (Shift+Enter, etc.)

---

## Performance Baseline (from zedit measurements)

| Operation | Target | Notes |
|-----------|--------|-------|
| PTY spawn | ~1–2ms | `posix_openpt` + `fork` |
| Keypress → screen | < 5ms | poll → write → render cycle |
| Startup | < 10ms | inherited from zedit constraint |

Zero external dependencies means no dependency graph overhead.
Binary size target: < 500KB stripped.

---

## Z-Ecosystem Fit

```
Zenith   ← programming language
Zymbol   ← package manager / toolchain
Zedit    ← code editor (embeds terminal)
Zhelly   ← standalone terminal emulator (forked from Zedit's vterm+pty)
```

Zhelly would set `TERM_PROGRAM=zhelly` in child processes, allowing Zenith
tooling (build output, REPL, debugger) to detect it and emit Zhelly-specific
features (e.g., clickable error locations, inline diffs, structured output).

---

## Next Steps (when picking this up)

1. [ ] Create `Zenith-Lang-Org/zhelly` repository
2. [ ] Extract `pty.rs`, `vterm.rs`, `terminal.rs`, `input.rs` from zedit
3. [ ] Write minimal `main.rs` (raw mode → poll loop → render)
4. [ ] Strip editor-specific code from extracted modules
5. [ ] Add `TERM_PROGRAM=zhelly` injection in `pty.rs`
6. [ ] Implement configurable scrollback size
7. [ ] Add resize reflow (optional, complex)
8. [ ] Add tab bar (multiple PTY instances)
9. [ ] Add OSC 8 hyperlink support
10. [ ] Evaluate Kitty keyboard protocol

---

*This document is a starting point. The core infrastructure already exists in zedit —
the main work is extraction, cleanup, and standalone packaging.*
