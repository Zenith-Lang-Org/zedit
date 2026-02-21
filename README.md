# zedit

```
 ███████╗███████╗██████╗ ██╗████████╗
 ╚══███╔╝██╔════╝██╔══██╗██║╚══██╔══╝
   ███╔╝ █████╗  ██║  ██║██║   ██║
  ███╔╝  ██╔══╝  ██║  ██║██║   ██║
 ███████╗███████╗██████╔╝██║   ██║
 ╚══════╝╚══════╝╚═════╝ ╚═╝   ╚═╝
```

**Modern console text editor** — pure Rust, zero dependencies.

Part of the Z ecosystem (Zenith, Zymbol). Born from the Minilux REPL project.

## Features

- **Zero external dependencies** — only `std` + libc FFI, no crates
- **Modern keybindings** — Ctrl+C/V/X/S/Z like desktop editors
- **UTF-8 native** — full Unicode text handling
- **Syntax highlighting** — TextMate grammars with VS Code-compatible themes
- **Diff-based rendering** — only redraws changed cells for smooth performance
- **Multi-buffer + tab bar** — open and switch between multiple files
- **Multi-cursor editing** — add cursors, select all occurrences
- **Split panes** — horizontal and vertical splits, resizable
- **Integrated terminal** — persistent PTY with VT100 emulation, multiple tabs
- **LSP client** — completions, hover docs, go-to-definition, inline diagnostics
- **Git gutter** — live added/modified/deleted markers in the line gutter
- **Diff view** — side-by-side comparison of buffer vs HEAD (F7)
- **Minimap** — braille-encoded code overview on the right edge (Alt+M)
- **File tree sidebar** — directory browser with Ctrl+B
- **Command palette** — fuzzy-searchable list of all actions (Ctrl+P)
- **Plugin system** — external processes via newline-delimited JSON (Minilux)
- **Session + swap** — auto-saves open buffers on exit, crash recovery
- **Search & replace** — incremental search, regex mode, highlight all matches
- **Undo/redo** — operation-based with smart grouping
- **Mouse support** — click, drag, scroll, Alt+Click for multi-cursor
- **Auto-indent** — preserves indentation on Enter
- **Line comments** — language-aware toggle with Ctrl+/
- **Adaptive color** — auto-detects TrueColor, 256-color, or 16-color terminals
- **Small binary** — under 500KB stripped

## Building

Requires Rust (edition 2024). No external dependencies to install.

```sh
cargo build --release
strip target/release/zedit    # optional, ~500KB result
```

## Usage

```sh
zedit                  # restore previous session or new empty buffer
zedit file.rs          # open a file
zedit --help           # show usage and keybindings
zedit --version        # print version
```

## Keybindings

### File

| Key | Action |
|-----|--------|
| `Ctrl+S` | Save |
| `Ctrl+Shift+S` | Save As |
| `Ctrl+O` | Open file |
| `Ctrl+Q` | Quit (press twice if unsaved) |
| `Ctrl+N` | New buffer |
| `Ctrl+W` | Close buffer |
| `Ctrl+PgDn` | Next buffer |
| `Ctrl+PgUp` | Previous buffer |

### Edit

| Key | Action |
|-----|--------|
| `Ctrl+Z` | Undo |
| `Ctrl+Y` | Redo |
| `Ctrl+C` | Copy (selection or current line) |
| `Ctrl+X` | Cut (selection or current line) |
| `Ctrl+V` | Paste |
| `Ctrl+Shift+D` | Duplicate line |
| `Ctrl+Shift+K` | Delete line |
| `Tab` | Indent (insert spaces / indent selection) |
| `Shift+Tab` | Unindent |
| `Ctrl+/` | Toggle line comment |
| `Enter` | Newline with auto-indent |

### Navigation

| Key | Action |
|-----|--------|
| `Arrow keys` | Move cursor |
| `Home` / `End` | Line start / end |
| `Ctrl+Home` / `Ctrl+End` | File start / end |
| `Page Up` / `Page Down` | Scroll page |
| `Ctrl+G` | Go to line |
| `Ctrl+F` | Find (incremental, Ctrl+R inside to toggle regex) |
| `Ctrl+H` | Find and replace |
| `F3` / `Shift+F3` | Next / previous match |

### Selection

| Key | Action |
|-----|--------|
| `Shift+Arrows` | Extend selection |
| `Ctrl+Shift+Left/Right` | Extend selection word by word |
| `Ctrl+A` | Select all |
| `Ctrl+L` | Select current line |

### Multi-Cursor

| Key | Action |
|-----|--------|
| `Ctrl+D` | Add cursor at next occurrence of selection (or select word) |
| `Ctrl+Shift+L` | Select all occurrences at once |
| `Alt+Click` | Add cursor at clicked position |
| `Escape` | Collapse to single cursor |

### Panes

| Key | Action |
|-----|--------|
| `Ctrl+\` | Split pane horizontally (side-by-side) |
| `Ctrl+Shift+\` | Split pane vertically (top/bottom) |
| `Ctrl+Shift+W` | Close active pane |
| `Alt+Left/Right/Up/Down` | Move focus to adjacent pane |
| `Alt+Shift+Left/Right` | Resize pane horizontally |
| `Alt+Shift+Up/Down` | Resize pane vertically |

### View

| Key | Action |
|-----|--------|
| `F1` | Toggle help overlay |
| `Alt+Z` | Toggle soft word wrap |
| `Ctrl+B` | Toggle file tree sidebar |
| `Ctrl+P` | Open command palette |
| `Alt+M` | Toggle minimap |

### Terminal

| Key | Action |
|-----|--------|
| `Ctrl+T` | Toggle integrated terminal panel |
| `Ctrl+Shift+T` | Open new terminal tab |
| `Shift+Page Up/Down` | Scroll terminal history |

### LSP

| Key | Action |
|-----|--------|
| `Ctrl+Space` | Request completions |
| `Alt+K` | Show hover documentation |
| `F12` | Go to definition |

### Diff View

| Key | Action |
|-----|--------|
| `F7` | Open side-by-side diff vs HEAD |
| `F8` | Jump to next changed hunk |
| `Shift+F8` | Jump to previous changed hunk |
| `Escape` | Close diff view |

### Mouse

| Action | Effect |
|--------|--------|
| Click | Position cursor |
| Double-click | Select word under cursor |
| Drag | Select text |
| Scroll | Scroll viewport |
| Alt+Click | Add multi-cursor at position |
| Click tab bar | Switch to that buffer |

## Configuration

Zedit reads `~/.config/zedit/config.json`:

```json
{
    "tab_size": 4,
    "use_spaces": true,
    "theme": "zedit-dark",
    "line_numbers": true,
    "auto_indent": true,
    "word_wrap": false,
    "lsp": {
        "rust":   { "command": "rust-analyzer" },
        "python": { "command": "pylsp" }
    },
    "keybindings": {
        "toggle_minimap": "Alt+M"
    }
}
```

## LSP

Language servers are started on demand when a matching file is opened.
Configure them in `~/.config/zedit/config.json` under the `"lsp"` key:

```json
"lsp": {
    "rust":       { "command": "rust-analyzer" },
    "python":     { "command": "pylsp" },
    "typescript": { "command": "typescript-language-server", "args": ["--stdio"] },
    "go":         { "command": "gopls" },
    "c":          { "command": "clangd" }
}
```

Diagnostics appear as colored underlines in the text and are counted in the
status bar (`E:2 W:1`).

## Syntax Highlighting

Zedit uses TextMate `.tmLanguage.json` grammars and VS Code-compatible themes.
Built-in languages:

Rust · C · C++ · Go · Java · JavaScript · TypeScript · Python · PHP · Julia · R
· JSON · TOML · YAML · Markdown · Shell/Bash · HTML · CSS · XML · Zenith · Zymbol · Minilux

User-provided grammars go in `~/.config/zedit/grammars/`.
Custom themes go in `~/.config/zedit/themes/`.

## Git Gutter

When a file is inside a git repository, change markers appear in the left
gutter:

| Marker | Meaning |
|--------|---------|
| `+` (green) | Line added since HEAD |
| `~` (yellow) | Line modified since HEAD |
| `-` (red) | Lines deleted (shown on adjacent line) |

## Session & Swap

- **Session**: open files, cursor positions, and scroll offsets are saved
  automatically on exit. Running `zedit` without arguments restores the previous
  session for the current directory.
- **Swap files**: written every 2 seconds while editing. If zedit crashes, the
  next launch detects orphaned swap files and offers to recover them.

## Plugins

Plugins are external processes communicating over newline-delimited JSON on
stdin/stdout. Place each plugin in `~/.config/zedit/plugins/<name>/` with a
`manifest.json`:

```json
{
    "name": "myplugin",
    "version": "1.0.0",
    "description": "Does something useful",
    "main": "main.mlx"
}
```

Plugins can register palette commands, subscribe to editor events
(`buffer_open`, `buffer_save`, `buffer_close`, `cursor_move`, `text_change`),
read and insert buffer text, and show status bar messages.

## Architecture

- **Gap buffer** for text storage — simple and fast for sequential edits, handles files up to ~50MB
- **Custom regex engine** — NFA/bytecode subset of Oniguruma patterns, zero dependencies
- **Custom JSON parser** — ~300 lines, parses TextMate grammars
- **Stateful tokenizer** — carries `LineState` between lines for multi-line constructs
- **Diff-based renderer** — only emits ANSI escape sequences for changed screen cells
- **OSC 52 clipboard** — terminal clipboard via escape sequences

See `docs/editor-plan.md` for the full architecture document.
Full user manuals: [`docs/en_zedit.md`](docs/en_zedit.md) (English) · [`docs/es_zedit.md`](docs/es_zedit.md) (Español)

## Performance Targets

| Metric | Target |
|--------|--------|
| Startup | < 10ms |
| Keypress to screen | < 5ms |
| Open 1MB file | < 50ms |
| Binary size | < 1MB |

## License

GNU General Public License v3.0 — see [LICENSE](LICENSE) for details.
