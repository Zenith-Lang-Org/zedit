# zedit
### A modern shell editor for code and notes.

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
- **Multi-buffer** — open and switch between multiple files
- **Search & replace** — incremental case-insensitive search with highlight
- **Undo/redo** — operation-based with smart grouping
- **Mouse support** — click, drag to select, scroll
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
zedit                  # new empty buffer
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
| `Ctrl+D` | Duplicate line |
| `Ctrl+Shift+K` | Delete line |
| `Tab` | Insert 4 spaces / indent selection |
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
| `Ctrl+F` | Find |
| `Ctrl+H` | Find and replace |
| `F3` / `Shift+F3` | Next / previous match |

### Selection

| Key | Action |
|-----|--------|
| `Shift+Arrows` | Extend selection |
| `Ctrl+Shift+Left/Right` | Select word |
| `Ctrl+A` | Select all |
| `Ctrl+L` | Select line |

### Mouse

| Action | Effect |
|--------|--------|
| Click | Position cursor |
| Drag | Select text |
| Scroll | Scroll viewport |

### Help

| Key | Action |
|-----|--------|
| `F1` | Toggle help overlay |

## Syntax Highlighting

Zedit uses TextMate `.tmLanguage.json` grammars and VS Code-compatible themes. Currently supported languages:

- Rust, C, C++, Go, Java
- JavaScript, TypeScript
- Python, PHP, Julia, R
- JSON, TOML, YAML
- Markdown
- Shell (Bash)
- HTML, CSS, XML
- Zenith, Zymbol, Minilux

Grammars are embedded at compile time. User-provided grammars go in `~/.config/zedit/grammars/`.

## Architecture

- **Gap buffer** for text storage — simple and fast for sequential edits, handles files up to ~50MB
- **Custom regex engine** — NFA/bytecode subset of Oniguruma patterns, zero dependencies
- **Custom JSON parser** — ~300 lines, parses TextMate grammars
- **Stateful tokenizer** — carries `LineState` between lines for multi-line constructs
- **Diff-based renderer** — only emits ANSI escape sequences for changed screen cells
- **OSC 52 clipboard** — terminal clipboard via escape sequences

See `docs/editor-plan.md` for the full architecture document.

## Performance Targets

| Metric | Target |
|--------|--------|
| Startup | < 10ms |
| Keypress to screen | < 5ms |
| Open 1MB file | < 50ms |
| Binary size | < 1MB |

## License

This project is licensed under the GNU General Public License v3.0. See [LICENSE](LICENSE) for details.
