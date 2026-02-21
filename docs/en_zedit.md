# zedit User Manual

**Version 0.1.0** | Part of the Z ecosystem (Zenith, Zymbol)

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Installation and Building](#2-installation-and-building)
3. [Quick Start](#3-quick-start)
4. [The Interface](#4-the-interface)
5. [Complete Keybinding Reference](#5-complete-keybinding-reference)
6. [Search and Replace](#6-search-and-replace)
7. [Multi-Cursor Editing](#7-multi-cursor-editing)
8. [Pane Management](#8-pane-management)
9. [File Tree Sidebar](#9-file-tree-sidebar)
10. [Integrated Terminal](#10-integrated-terminal)
11. [LSP Integration](#11-lsp-integration)
12. [Git Gutter](#12-git-gutter)
13. [Diff View](#13-diff-view)
14. [Minimap](#14-minimap)
15. [Session and Crash Recovery](#15-session-and-crash-recovery)
16. [Configuration Reference](#16-configuration-reference)
17. [Custom Keybindings](#17-custom-keybindings)
18. [Syntax Highlighting and Themes](#18-syntax-highlighting-and-themes)
19. [Plugin System](#19-plugin-system)
20. [Extension System](#20-extension-system)
21. [Task Runner](#21-task-runner)
22. [Problem Panel](#22-problem-panel)
23. [REPL Integration](#23-repl-integration)
24. [Troubleshooting](#24-troubleshooting)
25. [License](#25-license)

---

## 1. Introduction

zedit is a modern console text editor written entirely in pure Rust with zero external dependencies. It uses only the Rust standard library and direct libc FFI calls, producing a single static binary under 500 KB (stripped).

zedit is part of the Z ecosystem alongside Zenith and Zymbol, and was born from the Minilux REPL project. Its design philosophy is that a terminal editor should feel like a desktop editor: modern keybindings (Ctrl+C, Ctrl+V, Ctrl+Z), smooth rendering, syntax highlighting, and rich features out of the box, without modes, without a learning curve, and without a startup delay.

**Key properties:**

- **Zero external dependencies** — only `std` + libc FFI. No crates, no build-time C compilation.
- **Modern keybindings** — Ctrl+C/V/X/S/Z work exactly as they do in GUI editors.
- **UTF-8 native** — all text handling assumes full UTF-8 from day one.
- **Standard formats** — TextMate `.tmLanguage.json` grammars and VS Code-compatible themes. No proprietary formats.
- **Sub-10ms startup** — a single `exec()` call, no interpreter, no JIT.
- **Diff-based rendering** — only changed terminal cells are redrawn each frame, keeping the display smooth and flicker-free.

**Performance targets:**

| Metric | Target |
|--------|--------|
| Startup | < 10 ms |
| Keypress to screen | < 5 ms |
| Open 1 MB file | < 50 ms |
| Binary size (stripped) | < 500 KB |

---

## 2. Installation and Building

### Prerequisites

- Rust toolchain (edition 2024). Install from [https://rustup.rs](https://rustup.rs).
- No other dependencies are required — not even libc headers; the FFI declarations are included in the source.

### Building from source

```sh
git clone https://github.com/Zenith-Lang-Org/zedit.git
cd zedit
cargo build --release
```

The resulting binary is at `target/release/zedit`.

### Stripping the binary (recommended)

```sh
strip target/release/zedit
```

This reduces the binary to approximately 500 KB by removing debug symbols.

### Installing the binary

Copy the binary to any directory on your `$PATH`:

```sh
cp target/release/zedit ~/.local/bin/zedit
```

### Other build commands

```sh
cargo build          # Debug build (larger, includes debug info)
cargo test           # Run all tests
cargo test <name>    # Run a single test by name
cargo clippy         # Lint
cargo fmt            # Format code
cargo fmt -- --check # Check formatting without modifying files
```

### Man page

A man page is included at `zedit.1` in the repository root. Install it with:

```sh
install -m 644 zedit.1 /usr/local/share/man/man1/zedit.1
```

---

## 3. Quick Start

### Opening zedit

```sh
zedit                  # Start with a new empty buffer (or restore last session)
zedit file.rs          # Open a specific file
zedit --help           # Print keybinding summary and exit
zedit --version        # Print version and exit
```

### Your first editing session

1. Open a file: `zedit myfile.txt` or press `Ctrl+O` from inside zedit to be prompted for a path.
2. Type to insert text. Arrow keys move the cursor. `Home`/`End` jump to the start or end of a line.
3. Press `Ctrl+S` to save. If the buffer is untitled, zedit will prompt you for a file name.
4. Press `Ctrl+Z` to undo and `Ctrl+Y` to redo.
5. Press `Ctrl+F` to open the find bar, type your search term, and use `F3`/`Shift+F3` to jump between matches.
6. Press `Ctrl+Q` to quit. If there are unsaved changes, press `Ctrl+Q` a second time to confirm.

### Working with multiple files

- `Ctrl+N` opens a new empty buffer.
- `Ctrl+O` prompts for a file path to open.
- `Ctrl+PgDn` / `Ctrl+PgUp` cycles through open buffers.
- `Ctrl+W` closes the current buffer.

The tab bar at the top shows all open buffers. The active one is highlighted.

---

## 4. The Interface

zedit uses the full terminal window. The screen is divided into several regions:

```
┌──────────────────────────────────────────────────────────────┐
│ Tab Bar                                                       │
├──────────────────────────────────────────────────────────────┤
│ File Tree │  Gutter │  Editor Area          │ Minimap        │
│ Sidebar   │         │                       │                │
│ (optional)│         │                       │ (optional)     │
│           │         │                       │                │
├───────────┴─────────┴───────────────────────┴────────────────┤
│ Terminal Panel (optional)                                     │
├──────────────────────────────────────────────────────────────┤
│ Status Bar                                                    │
└──────────────────────────────────────────────────────────────┘
```

### Tab Bar

The tab bar runs across the top of the window and shows one tab per open buffer. Each tab displays:

- The file name (or `[New Buffer N]` for untitled buffers).
- A modification indicator (`*`) when the buffer has unsaved changes.

Use `Ctrl+PgDn` / `Ctrl+PgUp` to cycle between tabs, or click a tab with the mouse.

### Editor Area

The main editing area is where text is displayed and edited. It supports:

- Syntax highlighting using TextMate grammars.
- Selection highlighting.
- Search match highlighting.
- Multi-cursor indicators.
- Soft word wrap (toggleable with `Alt+Z`).

### Gutter

The gutter is the narrow column to the left of the editor area. It displays:

- **Line numbers** (configurable; default on).
- **Git change indicators** when the file is inside a Git repository:
  - `+` in green: line added since HEAD.
  - `~` in yellow: line modified since HEAD.
  - `-` in red: line(s) deleted (indicator shown on the adjacent surviving line).
- **LSP diagnostic markers**: errors and warnings from the language server.

### Status Bar

The status bar at the bottom shows context-sensitive information:

- File path or buffer name.
- Modified indicator.
- Cursor position (line:column).
- Language / file type.
- Encoding and line ending style.
- Active mode (search, command palette, etc.).
- LSP diagnostic summary (error and warning counts).

### File Tree Sidebar

The file tree sidebar shows the directory tree rooted at the working directory. Toggle it with `Ctrl+B`. See [Section 9](#9-file-tree-sidebar) for details.

### Minimap

The minimap is a condensed overview of the entire file rendered in a thin column on the right edge of the editor. Toggle it with `Ctrl+Shift+M`. See [Section 14](#14-minimap) for details.

### Panes (Splits)

The editor area can be split into multiple independent panes, each displaying a different buffer (or the same buffer at a different scroll position). See [Section 8](#8-pane-management) for details.

### Terminal Panel

An integrated terminal panel can be toggled at the bottom of the screen with `Ctrl+T`. See [Section 10](#10-integrated-terminal) for details.

### Help Overlay

Press `F1` to toggle a keyboard shortcut reference overlay that lists all keybindings.

---

## 5. Complete Keybinding Reference

All default keybindings are listed below. Every binding can be remapped in the configuration file (see [Section 17](#17-custom-keybindings)).

### File Operations

| Key | Action |
|-----|--------|
| `Ctrl+S` | Save the current buffer. Prompts for a file name if untitled. |
| `Ctrl+Shift+S` | Save As — prompt for a new file name regardless. |
| `Ctrl+O` | Open a file — prompts for a path. |
| `Ctrl+Q` | Quit. If there are unsaved changes, press a second time to confirm. |
| `Ctrl+N` | New empty buffer. |
| `Ctrl+W` | Close the current buffer. |
| `Ctrl+PgDn` | Switch to the next buffer (tab). |
| `Ctrl+PgUp` | Switch to the previous buffer (tab). |

### Editing

| Key | Action |
|-----|--------|
| `Ctrl+Z` | Undo. |
| `Ctrl+Y` | Redo. |
| `Ctrl+C` | Copy selection to clipboard. If no selection, copies the entire current line. |
| `Ctrl+X` | Cut selection. If no selection, cuts the entire current line. |
| `Ctrl+V` | Paste from clipboard. |
| `Ctrl+Shift+D` | Duplicate the current line (inserts a copy immediately below). |
| `Ctrl+Shift+K` | Delete the current line entirely. |
| `Tab` | Indent the selection (or insert spaces at the cursor if no selection). |
| `Shift+Tab` | Unindent the selection. |
| `Ctrl+/` | Toggle a line comment on the current line or selection (language-aware). |
| `Enter` | Insert a newline with automatic indentation matching the current line. |

### Navigation

| Key | Action |
|-----|--------|
| `Arrow keys` | Move the cursor one character or line. |
| `Home` | Move to the beginning of the current line. |
| `End` | Move to the end of the current line. |
| `Ctrl+Home` | Move to the very beginning of the file. |
| `Ctrl+End` | Move to the very end of the file. |
| `Page Up` | Scroll up one full page. |
| `Page Down` | Scroll down one full page. |
| `Ctrl+G` | Go to a specific line number (a prompt appears in the status bar). |

### Search

| Key | Action |
|-----|--------|
| `Ctrl+F` | Open the find bar (incremental search). |
| `Ctrl+H` | Open the find and replace bar. |
| `F3` | Jump to the next match. |
| `Shift+F3` | Jump to the previous match. |
| `Ctrl+R` | (In find mode) Toggle regex mode on/off. |

### Selection

| Key | Action |
|-----|--------|
| `Shift+Arrow keys` | Extend the selection one character or line at a time. |
| `Ctrl+A` | Select all text in the buffer. |
| `Ctrl+L` | Select the current entire line. |
| `Ctrl+D` | Select the next occurrence of the current word or selection (adds a cursor for multi-cursor editing). |
| `Ctrl+Shift+L` | Select all occurrences of the current word or selection at once. |
| `Alt+Click` | Add a cursor at the clicked position (multi-cursor). |
| `Escape` | Collapse all cursors back to a single primary cursor. |

### Panes and Splits

| Key | Action |
|-----|--------|
| `Ctrl+\` | Split the active pane horizontally (side by side). |
| `Ctrl+Shift+\` | Split the active pane vertically (top and bottom). |
| `Ctrl+Shift+W` | Close the active pane. |
| `Alt+Left` | Move focus to the pane to the left. |
| `Alt+Right` | Move focus to the pane to the right. |
| `Alt+Up` | Move focus to the pane above. |
| `Alt+Down` | Move focus to the pane below. |
| `Alt+Shift+Left` | Resize the active pane: move the split left. |
| `Alt+Shift+Right` | Resize the active pane: move the split right. |
| `Alt+Shift+Up` | Resize the active pane: move the split up. |
| `Alt+Shift+Down` | Resize the active pane: move the split down. |

### View

| Key | Action |
|-----|--------|
| `F1` | Toggle the help overlay. |
| `Alt+Z` | Toggle soft word wrap. |
| `Ctrl+B` | Toggle the file tree sidebar. |
| `Ctrl+P` | Open the command palette (fuzzy search for all commands). |
| `Ctrl+T` | Toggle the integrated terminal panel. |
| `Ctrl+Shift+T` | Open a new terminal tab in the terminal panel. |
| `Alt+M` | Toggle the minimap. |

### Task Runner

| Key | Action |
|-----|--------|
| `F5` | Run the default task for the current language. |
| `Ctrl+F5` | Build the project. |
| `Shift+F5` | Run tests. |
| `Alt+F5` | Stop the currently running task. |

### Problem Panel

| Key | Action |
|-----|--------|
| `F6` | Toggle the problem panel (build errors and warnings overlay). |
| `Up` / `Down` | Navigate between problems (when panel is focused). |
| `Enter` | Jump to the file and line of the selected problem. |
| `Escape` | Close the problem panel. |

### REPL Integration

| Key | Action |
|-----|--------|
| `Alt+Enter` | Send the current selection (or current line) to the language REPL in the terminal. |

### LSP (Language Server Protocol)

| Key | Action |
|-----|--------|
| `Ctrl+Space` | Show the completion menu. Use `Tab` or `Enter` to insert, `Escape` to dismiss. |
| `Alt+K` | Show the hover documentation popup. Press any key to dismiss. |
| `F12` | Go to the definition of the symbol under the cursor. Jumps within the file or opens the target file. |

### Diff View

| Key | Action |
|-----|--------|
| `F7` | Open the diff view: compare the current buffer against the Git HEAD version. |
| `F8` | Jump to the next changed hunk. |
| `Shift+F8` | Jump to the previous changed hunk. |
| `Up` / `Down` / `Page Up` / `Page Down` | Scroll the diff view. |
| `Escape` | Close the diff view and return to the editor. |

### Mouse

| Action | Effect |
|--------|--------|
| Click | Position the cursor at the clicked location. |
| Drag | Select text from the drag start to the current position. |
| Double-click | Select the word under the cursor. |
| Scroll wheel | Scroll the viewport up or down. |
| `Alt+Click` | Add an additional cursor at the clicked position. |

### Terminal Panel Keys

| Key | Action |
|-----|--------|
| `Ctrl+T` | Toggle the terminal panel (shows or hides it; the shell session persists). |
| `Ctrl+Shift+T` | Open a new terminal tab inside the panel. |
| `Shift+Page Up` / `Shift+Page Down` | Scroll through terminal history. |
| `Ctrl+Q` | Exit terminal focus and return keyboard input to the editor. |

---

## 6. Search and Replace

### Incremental Find

Press `Ctrl+F` to open the find bar in the status area. As you type your search term, zedit highlights all matches in the current buffer and jumps to the nearest one. The match count is displayed in the status bar.

- `F3` jumps to the next match.
- `Shift+F3` jumps to the previous match.
- `Escape` closes the find bar and clears highlights.

### Regex Mode

While the find bar is open, press `Ctrl+R` to toggle regular expression mode. The status bar shows an indicator when regex mode is active. zedit's regex engine supports:

- Literals and escape sequences (`\n`, `\t`, `\\`, `\.`)
- Character classes: `[a-z]`, `[^0-9]`, `\w`, `\d`, `\s`, `\b`
- Quantifiers: `*`, `+`, `?`, `{n}`, `{n,m}`
- Alternation: `a|b`
- Groups: `(...)`, `(?:...)`
- Anchors: `^`, `$`, `\b`
- Lookahead: `(?=...)`, `(?!...)`
- Backreferences: `\1`

### Find and Replace

Press `Ctrl+H` to open the find and replace interface. Two input fields appear:

1. **Find** — enter the search term (regex is supported if toggled).
2. **Replace** — enter the replacement text.

Navigation within the replace interface mirrors the find bar: `F3` and `Shift+F3` cycle through matches. Confirm a replacement with `Enter`. A "Replace All" option is also available from this interface.

---

## 7. Multi-Cursor Editing

zedit supports multiple simultaneous cursors. All cursors accept the same keystrokes at the same time, allowing you to make identical edits in multiple locations in a single pass.

### Adding cursors

| Method | Description |
|--------|-------------|
| `Ctrl+D` | Select the next occurrence of the text under the cursor (or current selection). Each press adds one more cursor. |
| `Ctrl+Shift+L` | Instantly select all occurrences of the current word or selection and place a cursor at each one. |
| `Alt+Click` | Click anywhere in the document to add a cursor at that position. |

### Collapsing cursors

Press `Escape` to discard all secondary cursors and return to a single cursor at the primary position.

### What you can do with multiple cursors

With multiple cursors active, every standard editing operation applies to all of them simultaneously:

- Type to insert text at all cursor positions.
- `Backspace` / `Delete` to delete characters at all cursor positions.
- `Ctrl+C` to copy the selection at each cursor independently.
- Arrow keys to move all cursors.
- `Home` / `End` to jump to the start/end of each cursor's respective line.

Multi-cursor editing is particularly effective for:

- Renaming a variable that appears multiple times on screen.
- Adding or removing a common prefix or suffix across many lines.
- Editing CSV or table data where each row has the same structure.

---

## 8. Pane Management

zedit supports splitting the editor area into multiple independent panes, each with its own buffer, cursor position, and scroll state.

### Creating splits

| Key | Action |
|-----|--------|
| `Ctrl+\` | Split the current pane horizontally: the active pane becomes two side-by-side panes. |
| `Ctrl+Shift+\` | Split the current pane vertically: the active pane becomes two panes stacked top and bottom. |

When you split a pane, the new pane initially displays the same buffer. You can then switch buffers independently in each pane using `Ctrl+PgDn` / `Ctrl+PgUp` or `Ctrl+O`.

### Navigating between panes

| Key | Action |
|-----|--------|
| `Alt+Left` | Move focus to the pane to the left. |
| `Alt+Right` | Move focus to the pane to the right. |
| `Alt+Up` | Move focus to the pane above. |
| `Alt+Down` | Move focus to the pane below. |

### Resizing panes

| Key | Action |
|-----|--------|
| `Alt+Shift+Left` | Shrink the active pane horizontally (move the vertical divider left). |
| `Alt+Shift+Right` | Expand the active pane horizontally (move the vertical divider right). |
| `Alt+Shift+Up` | Shrink the active pane vertically (move the horizontal divider up). |
| `Alt+Shift+Down` | Expand the active pane vertically (move the horizontal divider down). |

### Closing a pane

Press `Ctrl+Shift+W` to close the currently focused pane. The adjacent pane expands to fill the vacated space. Closing a pane does not close the buffer it was displaying.

---

## 9. File Tree Sidebar

The file tree sidebar provides a directory navigator rooted at zedit's working directory (the directory from which zedit was launched).

### Toggling the sidebar

Press `Ctrl+B` to show or hide the sidebar. The editor area resizes automatically.

### Navigating the tree

- **Arrow keys** move the selection up and down.
- **Enter** opens the selected file in the active editor pane, or expands/collapses a directory.
- **Mouse click** selects an entry; a second click on a file opens it.

### Configuration

The sidebar width and ignored entries can be configured in `config.json`:

```json
{
  "filetree_width": 30,
  "filetree_ignored": [".git", "target", "node_modules"]
}
```

- `filetree_width`: width in columns (15–60, default 30).
- `filetree_ignored`: list of directory or file names to hide.

---

## 10. Integrated Terminal

zedit includes a built-in terminal emulator running a real PTY (pseudo-terminal) with a full VT100/VT220 emulator. The shell session inside the terminal panel is persistent: hiding the panel does not kill the shell.

### Opening and closing the terminal

| Key | Action |
|-----|--------|
| `Ctrl+T` | Toggle the terminal panel. |
| `Ctrl+Shift+T` | Open a new terminal tab inside the panel. |

### Scrolling terminal history

| Key | Action |
|-----|--------|
| `Shift+Page Up` | Scroll up through terminal output history. |
| `Shift+Page Down` | Scroll down through terminal output history. |

The scrollback buffer holds up to 1,000 lines by default. This is configurable:

```json
{
  "terminal_scrollback": 5000
}
```

### Returning to the editor

Press `Ctrl+Q` while the terminal panel has focus to hand keyboard input back to the editor. The shell continues running in the background.

### Shell selection

By default, zedit launches the shell defined by the `$SHELL` environment variable. You can override this:

```json
{
  "terminal_shell": "/bin/zsh"
}
```

---

## 11. LSP Integration

zedit includes a built-in Language Server Protocol (LSP) client. When an LSP server is configured for the current file's language, zedit automatically connects to it, sends document synchronization updates, and surfaces completions, hover documentation, diagnostics, and go-to-definition.

### Setting up LSP servers

Add an `lsp` object to `~/.config/zedit/config.json`. Map each language identifier to the server command:

```json
{
  "lsp": {
    "rust": { "command": "rust-analyzer" },
    "python": { "command": "pylsp" },
    "typescript": { "command": "typescript-language-server", "args": ["--stdio"] }
  }
}
```

The `command` must be on your `$PATH`. The `args` array is optional.

Common LSP servers:

| Language | Server | Install |
|----------|--------|---------|
| Rust | `rust-analyzer` | `rustup component add rust-analyzer` |
| Python | `pylsp` | `pip install python-lsp-server` |
| TypeScript/JavaScript | `typescript-language-server` | `npm install -g typescript-language-server typescript` |
| Go | `gopls` | `go install golang.org/x/tools/gopls@latest` |
| C/C++ | `clangd` | Package manager or LLVM release |

### Code Completion

Press `Ctrl+Space` to trigger the completion menu at the cursor position. The menu shows a list of candidates from the LSP server (symbols, keywords, snippets).

- `Tab` or `Enter` inserts the selected completion.
- `Escape` dismisses the menu without inserting.
- Arrow keys navigate the list.

### Hover Documentation

Press `Alt+K` to request hover documentation for the symbol under the cursor. A popup appears with the type signature and documentation string from the LSP server. Press any key to dismiss it.

### Go to Definition

Press `F12` to jump to the definition of the symbol under the cursor. If the definition is in the current file, the cursor moves there. If it is in a different file, that file is opened in the current pane.

### Diagnostics

zedit continuously receives diagnostic notifications (errors, warnings, hints) from the LSP server and displays them in two places:

1. **Gutter**: a colored marker next to the affected line.
2. **Status bar**: a summary count of errors and warnings in the current buffer.

Diagnostics update automatically as you edit without requiring a manual save.

---

## 12. Git Gutter

When a file is inside a Git repository, zedit automatically computes the diff between the working-tree version of the buffer and the HEAD version, and displays change indicators in the gutter column.

### Indicators

| Indicator | Color | Meaning |
|-----------|-------|---------|
| `+` | Green | This line was added (does not exist in HEAD). |
| `~` | Yellow | This line was modified (exists in HEAD with different content). |
| `-` | Red | One or more lines were deleted here (shown on the adjacent line that survived). |

The gutter updates automatically as you edit. No configuration is required; it activates whenever a Git repository is detected.

---

## 13. Diff View

The diff view shows a side-by-side comparison of the current buffer against the version stored in Git HEAD. It is useful for reviewing your changes before committing.

### Opening the diff view

Press `F7` to open the diff view for the current buffer. The editor area is replaced by a two-column display:

- **Left column**: the HEAD (committed) version of the file.
- **Right column**: the current working-tree version (as it exists in the buffer).

Changed lines are color-coded:

- **Green**: lines added in the working version.
- **Red**: lines removed from the HEAD version.
- **Yellow**: lines that exist in both versions but have been modified (with character-level inline highlighting showing the exact characters that changed).

### Navigating hunks

A "hunk" is a group of consecutive changed lines.

| Key | Action |
|-----|--------|
| `F8` | Jump to the next changed hunk. |
| `Shift+F8` | Jump to the previous changed hunk. |
| `Up` / `Down` | Scroll the diff view one line at a time. |
| `Page Up` / `Page Down` | Scroll the diff view one page at a time. |

### Closing the diff view

Press `Escape` to close the diff view and return to normal editing.

---

## 14. Minimap

The minimap is a scaled-down rendering of the entire file contents displayed in a thin column on the right edge of the editor area. It provides a visual overview of the file's structure and lets you estimate your position within a long file at a glance.

### Toggling the minimap

Press `Alt+M` to show or hide the minimap. The editor area resizes to accommodate it.

The minimap highlights the region currently visible in the editor viewport, making it easy to see what fraction of the file you are viewing.

---

## 15. Session and Crash Recovery

### Session persistence

zedit automatically saves a session file when you exit normally. A session records:

- The list of open buffers (file paths).
- The cursor position in each buffer.
- The scroll offset in each buffer.
- Which buffer was active.

Session files are stored in `~/.local/state/zedit/sessions/` (or `$XDG_STATE_HOME/zedit/sessions/` if `XDG_STATE_HOME` is set). Each session is keyed to its working directory.

**Restoring a session**: when you run `zedit` with no arguments, it looks for a session file for the current working directory and restores it automatically. If no session is found, zedit opens a new empty buffer.

**Opening a specific file**: when you run `zedit <file>`, the session is not restored; zedit opens only the named file.

### Swap files and crash recovery

zedit writes swap files every 2 seconds while you are actively editing. Swap files are binary files with a `.swp` extension:

- For named files: stored alongside the original as `.filename.ext.swp` (e.g., `.foo.rs.swp` next to `foo.rs`).
- For untitled buffers: stored in `~/.local/state/zedit/swap/NewBuffer00.swp`.

**On crash recovery**: when zedit starts and detects an orphaned swap file (a swap file whose owning process is no longer running), it offers to recover the unsaved content:

- Press `R` to recover the contents from the swap file.
- Press `D` to delete the swap file and start fresh.

Swap files are removed automatically when a buffer is saved or closed normally.

---

## 16. Configuration Reference

The configuration file is located at `~/.config/zedit/config.json`. All settings are optional; zedit works with zero configuration.

### Full example

```json
{
  "tab_size": 4,
  "use_spaces": true,
  "theme": "zedit-dark",
  "line_numbers": true,
  "auto_indent": true,
  "word_wrap": false,
  "filetree_width": 30,
  "filetree_ignored": [".git", "target", "node_modules"],
  "terminal_shell": "",
  "terminal_scrollback": 1000,
  "lsp": {
    "rust": { "command": "rust-analyzer" },
    "python": { "command": "pylsp" },
    "typescript": { "command": "typescript-language-server", "args": ["--stdio"] }
  },
  "languages": [
    {
      "name": "ruby",
      "extensions": ["rb", "rake", "gemspec"],
      "grammar": "ruby.tmLanguage.json",
      "comment": "#"
    }
  ],
  "keybindings": {
    "save": "Ctrl+S",
    "toggle_terminal": "Ctrl+T"
  }
}
```

### Settings reference

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `tab_size` | integer | `4` | Number of spaces per indentation level. Valid range: 1–16. |
| `use_spaces` | boolean | `true` | `true` to insert spaces for indentation; `false` to insert a real tab character. |
| `theme` | string | `"zedit-dark"` | Color theme name. Built-in options: `zedit-dark`, `zedit-light`. Custom themes go in `~/.config/zedit/themes/`. |
| `line_numbers` | boolean | `true` | Show line numbers in the gutter. |
| `auto_indent` | boolean | `true` | When pressing `Enter`, automatically match the indentation of the current line. |
| `word_wrap` | boolean | `false` | Soft-wrap lines that exceed the window width. Does not insert actual newlines. |
| `filetree_width` | integer | `30` | Width of the file tree sidebar in columns. Valid range: 15–60. |
| `filetree_ignored` | array of strings | `[]` | Directory and file names to hide in the file tree. |
| `terminal_shell` | string | `""` | Shell executable for the integrated terminal. Empty string uses `$SHELL`. |
| `terminal_scrollback` | integer | `1000` | Number of lines to keep in the terminal scrollback buffer. Valid range: 100–100,000. |
| `lsp` | object | `{}` | Map of language identifier to LSP server configuration. See [Section 11](#11-lsp-integration). |
| `languages` | array | built-ins | Language definitions. Entries here override built-in definitions matched by `name`. |
| `keybindings` | object | defaults | Custom keybinding overrides. See [Section 17](#17-custom-keybindings). |

### Language definition format

Each entry in the `languages` array can define or override a language:

```json
{
  "name": "ruby",
  "extensions": ["rb", "rake", "gemspec"],
  "grammar": "ruby.tmLanguage.json",
  "comment": "#"
}
```

| Field | Description |
|-------|-------------|
| `name` | Language identifier (lowercase). If it matches a built-in, overrides it. |
| `extensions` | File extensions (without the leading dot) that trigger this language. |
| `grammar` | Grammar file name to use. Must exist in `~/.config/zedit/grammars/` or be a built-in. |
| `comment` | Comment prefix string for `Ctrl+/` toggle comment. Omit for languages with no line comment. |

---

## 17. Custom Keybindings

Any default keybinding can be overridden in the `keybindings` object inside `config.json`. Setting a new key for an action also removes the old binding for that action.

### Format

```json
{
  "keybindings": {
    "action_name": "Key+String"
  }
}
```

Key strings follow the format `[Ctrl+][Alt+][Shift+]<key>`. The modifiers are case-insensitive. The key name can be:

- A single letter or digit (`S`, `7`, `/`)
- `Enter`, `Tab`, `Backspace`, `Delete`, `Escape`
- `Up`, `Down`, `Left`, `Right`
- `Home`, `End`, `PgUp`, `PgDn` (also `PageUp`, `PageDown`)
- `F1` through `F12`
- `Space`, `` ` `` (backtick), `\` (backslash), `/` (slash)

### Example

```json
{
  "keybindings": {
    "save": "Ctrl+S",
    "diff_open_vs_head": "F7",
    "lsp_complete": "Ctrl+Space",
    "toggle_minimap": "Alt+M",
    "toggle_problem_panel": "F6",
    "task_run": "F5",
    "toggle_terminal": "F10"
  }
}
```

### Available action names

| Action name | Default key | Description |
|-------------|-------------|-------------|
| `save` | `Ctrl+S` | Save current buffer |
| `save_as` | `Ctrl+Shift+S` | Save As |
| `open_file` | `Ctrl+O` | Open file |
| `quit` | `Ctrl+Q` | Quit editor |
| `new_buffer` | `Ctrl+N` | New empty buffer |
| `close_buffer` | `Ctrl+W` | Close current buffer |
| `undo` | `Ctrl+Z` | Undo |
| `redo` | `Ctrl+Y` | Redo |
| `duplicate_line` | `Ctrl+Shift+D` | Duplicate current line |
| `delete_line` | `Ctrl+Shift+K` | Delete current line |
| `toggle_comment` | `Ctrl+/` | Toggle line comment |
| `unindent` | `Shift+Tab` | Unindent |
| `copy` | `Ctrl+C` | Copy |
| `cut` | `Ctrl+X` | Cut |
| `paste` | `Ctrl+V` | Paste |
| `select_all` | `Ctrl+A` | Select all |
| `select_line` | `Ctrl+L` | Select current line |
| `select_next_occurrence` | `Ctrl+D` | Select next occurrence |
| `select_all_occurrences` | `Ctrl+Shift+L` | Select all occurrences |
| `find` | `Ctrl+F` | Find |
| `replace` | `Ctrl+H` | Find and Replace |
| `find_next` | `F3` | Next match |
| `find_prev` | `Shift+F3` | Previous match |
| `go_to_line` | `Ctrl+G` | Go to line number |
| `next_buffer` | `Ctrl+PgDn` | Next buffer/tab |
| `prev_buffer` | `Ctrl+PgUp` | Previous buffer/tab |
| `split_horizontal` | `Ctrl+\` | Split pane horizontally |
| `split_vertical` | `Ctrl+Shift+\` | Split pane vertically |
| `close_pane` | `Ctrl+Shift+W` | Close active pane |
| `focus_left` | `Alt+Left` | Focus pane to the left |
| `focus_right` | `Alt+Right` | Focus pane to the right |
| `focus_up` | `Alt+Up` | Focus pane above |
| `focus_down` | `Alt+Down` | Focus pane below |
| `resize_pane_left` | `Alt+Shift+Left` | Resize pane left |
| `resize_pane_right` | `Alt+Shift+Right` | Resize pane right |
| `resize_pane_up` | `Alt+Shift+Up` | Resize pane up |
| `resize_pane_down` | `Alt+Shift+Down` | Resize pane down |
| `toggle_help` | `F1` | Toggle help overlay |
| `toggle_wrap` | `Alt+Z` | Toggle soft word wrap |
| `toggle_file_tree` | `Ctrl+B` | Toggle file tree sidebar |
| `command_palette` | `Ctrl+P` | Open command palette |
| `toggle_terminal` | `Ctrl+T` | Toggle terminal panel |
| `new_terminal` | `Ctrl+Shift+T` | Open new terminal tab |
| `lsp_complete` | `Ctrl+Space` | Show LSP completion menu |
| `lsp_hover` | `Alt+K` | Show LSP hover documentation |
| `lsp_go_to_def` | `F12` | Go to definition |
| `diff_open_vs_head` | `F7` | Open diff view vs Git HEAD |
| `diff_next_hunk` | `F8` | Next diff hunk |
| `diff_prev_hunk` | `Shift+F8` | Previous diff hunk |
| `toggle_minimap` | `Alt+M` | Toggle minimap |
| `task_run` | `F5` | Run default task |
| `task_build` | `Ctrl+F5` | Build project |
| `task_test` | `Shift+F5` | Run tests |
| `task_stop` | `Alt+F5` | Stop running task |
| `toggle_problem_panel` | `F6` | Toggle problem panel |
| `send_to_repl` | `Alt+Enter` | Send selection/line to REPL |

---

## 18. Syntax Highlighting and Themes

### How highlighting works

zedit uses TextMate `.tmLanguage.json` grammars — the same format used by VS Code, Sublime Text, and Atom. When you open a file, zedit:

1. Detects the language from the file extension.
2. Loads the matching grammar.
3. Tokenizes only the visible lines (lazy; off-screen lines are tokenized on demand).
4. Maps scope names (e.g., `keyword.control.rust`) to colors defined by the active theme.
5. On each edit, re-tokenizes from the modified line downward until the tokenizer state matches the previously cached state (typically stops after 1–3 lines).

All grammar processing uses a custom NFA/bytecode regex engine and a custom JSON parser, both implemented in pure Rust without external dependencies.

### Built-in language support

The following languages are supported out of the box with embedded grammars:

| Language | File extensions |
|----------|----------------|
| Rust | `.rs` |
| C | `.c`, `.h` |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp` |
| Go | `.go` |
| Java | `.java` |
| JavaScript | `.js`, `.mjs` |
| TypeScript | `.ts`, `.tsx` |
| Python | `.py` |
| PHP | `.php` |
| Julia | `.jl` |
| R | `.r`, `.R` |
| JSON | `.json` |
| TOML | `.toml` |
| YAML | `.yml`, `.yaml` |
| Markdown | `.md`, `.markdown` |
| Shell/Bash | `.sh`, `.bash` |
| HTML | `.html`, `.htm` |
| CSS | `.css` |
| XML | `.xml` |
| Zenith | `.zl` |
| Zymbol | `.zy` |
| Minilux | `.mi` |

### Adding custom language grammars

Drop any VS Code-compatible `.tmLanguage.json` file into `~/.config/zedit/grammars/`. zedit detects it automatically on next launch. No configuration is required if the grammar's `fileTypes` array matches the file extensions you want.

To explicitly associate a grammar with specific extensions (or to override a built-in), add an entry to the `languages` array in `config.json`:

```json
{
  "languages": [
    {
      "name": "ruby",
      "extensions": ["rb", "rake", "gemspec"],
      "grammar": "ruby.tmLanguage.json",
      "comment": "#"
    }
  ]
}
```

### Grammar search priority

Grammars are loaded from disk at runtime in the following order:

1. `~/.config/zedit/grammars/` — user-installed grammars (highest priority).
2. `/usr/share/zedit/grammars/` and `/usr/local/share/zedit/grammars/` — system-wide grammars.
3. `grammars/` in the current working directory — development / source tree.

### Themes

Themes use the VS Code-compatible JSON format with `tokenColors` scope mappings. Two themes are built in:

- **`zedit-dark`** (default): a dark theme based on the Catppuccin Mocha color palette.
- **`zedit-light`**: a light theme for bright environments.

To use a custom theme, place a `.json` theme file in `~/.config/zedit/themes/` and set the `theme` key in `config.json` to the base name of the file (without the `.json` extension).

### Color mode detection

zedit automatically detects the terminal's color capabilities:

| Condition | Mode |
|-----------|------|
| `COLORTERM=truecolor` or `COLORTERM=24bit` | 24-bit true color |
| `xterm-256color` (or similar) in `$TERM` | 256-color palette |
| Fallback | 16 ANSI colors |

In 256-color and 16-color modes, theme hex colors are mapped to the nearest available palette entry.

### Creating or porting a VS Code theme

A zedit theme is a standard VS Code theme JSON file. The minimal structure is:

```json
{
  "name": "My Theme",
  "type": "dark",
  "colors": {
    "editor.background": "#1e1e2e",
    "editor.foreground": "#cdd6f4",
    "editorLineNumber.foreground": "#6c7086",
    "editor.selectionBackground": "#45475a"
  },
  "tokenColors": [
    { "scope": "comment", "settings": { "foreground": "#6c7086" } },
    { "scope": "string",  "settings": { "foreground": "#a6e3a1" } },
    { "scope": "keyword", "settings": { "foreground": "#cba6f7", "fontStyle": "bold" } }
  ]
}
```

Scope names follow TextMate hierarchy rules: `keyword` matches `keyword.control.rust`, `keyword.operator`, and so on. More specific selectors take priority.

---

## 19. Plugin System

zedit supports external plugins that communicate with the editor over newline-delimited JSON IPC through stdin and stdout. The plugin process can be written in any language; the Minilux scripting runtime is the primary supported option.

### Plugin directory structure

Plugins are installed by placing a directory inside `~/.config/zedit/plugins/`:

```
~/.config/zedit/plugins/
  myplugin/
    manifest.json
    main.mlx
```

### Manifest format

Each plugin directory must contain a `manifest.json`:

```json
{
  "name": "myplugin",
  "version": "1.0.0",
  "description": "My plugin",
  "main": "main.mlx"
}
```

| Field | Description |
|-------|-------------|
| `name` | Unique plugin identifier. Used in IPC and the command palette. |
| `version` | Version string (displayed in the palette). |
| `description` | Short human-readable description. |
| `main` | Entry point file relative to the plugin directory. |

### Plugin launch

On startup, zedit scans `~/.config/zedit/plugins/` for valid manifests and launches each plugin's `main` file using the `minilux` runtime (which must be on your `$PATH`):

```sh
minilux ~/.config/zedit/plugins/myplugin/main.mlx
```

The plugin communicates via its stdin (receives messages from zedit) and stdout (sends messages to zedit).

### IPC protocol

All messages are single-line JSON objects terminated by a newline. Messages follow a JSON-RPC-inspired format.

**From the plugin to zedit** (requests):

#### RegisterCommand

Register a command that appears in the command palette.

```json
{ "method": "RegisterCommand", "params": { "id": "myplugin.hello", "label": "My Plugin: Hello", "keybinding": "Ctrl+Shift+H" } }
```

#### SubscribeEvent

Subscribe to an editor event. The `event` field can be one of:
`buffer_open`, `buffer_save`, `buffer_close`, `cursor_move`, `text_change`.

```json
{ "method": "SubscribeEvent", "params": { "event": "buffer_save" } }
```

#### GetBufferText

Request the full text of the current buffer. Zedit will send a response containing the content.

```json
{ "id": 1, "method": "GetBufferText", "params": {} }
```

#### GetFilePath

Request the file path of the current buffer.

```json
{ "id": 2, "method": "GetFilePath", "params": {} }
```

#### InsertText

Insert text at the current cursor position.

```json
{ "method": "InsertText", "params": { "text": "Hello, world!" } }
```

#### ShowMessage

Display a message in the status bar.

```json
{ "method": "ShowMessage", "params": { "text": "Plugin ready.", "kind": "info" } }
```

**From zedit to the plugin** (notifications and responses):

- **Event notification**: sent when a subscribed event fires. Contains the event name and relevant data (file path, cursor position, etc.).
- **Command notification**: sent when the user invokes a command registered by the plugin.
- **Response**: sent in reply to a `GetBufferText` or `GetFilePath` request, with the `id` field matching the original request.

### Plugin lifecycle

1. zedit launches all discovered plugins on startup.
2. Plugins send their `RegisterCommand` and `SubscribeEvent` messages during initialization.
3. zedit polls all plugin stdout file descriptors in its main event loop alongside the terminal input.
4. Dead plugin processes are reaped automatically.
5. On editor exit, zedit sends a shutdown signal and closes all plugin processes.

---

## 20. Extension System

zedit includes a native extension system that lets you install, manage, and import language extensions without recompiling.

### Installing extensions

```sh
zedit --ext list              # list all installed extensions
zedit --ext install <name>    # install an extension
zedit --ext remove  <name>    # uninstall an extension
zedit --ext info    <name>    # show extension metadata
```

Extensions are stored in `~/.config/zedit/extensions/`. Each extension is a subdirectory containing at least a `manifest.json`, and optionally grammar files and theme files.

### Importing VS Code extensions

```sh
zedit --import my-extension.vsix
```

zedit extracts the grammar (`.tmLanguage.json`) and theme (`.json`) assets from the `.vsix` archive and installs them into the user configuration directory. JavaScript code is ignored; only data files are imported.

### Extension directory structure

```
~/.config/zedit/extensions/
  my-language/
    manifest.json
    my-language.tmLanguage.json
```

The `manifest.json` mirrors the plugin manifest format (see Section 19) but extensions are pure data — they do not run code.

---

## 21. Task Runner

zedit has a built-in task runner that can launch language-specific build, run, and test commands directly from the editor. Output is shown in the integrated terminal and errors are parsed into the Problem Panel.

### Keybindings

| Key | Action |
|-----|--------|
| `F5` | Run the default task for the current file's language. |
| `Ctrl+F5` | Build the project. |
| `Shift+F5` | Run tests. |
| `Alt+F5` | Stop the currently running task. |

### Built-in task presets

| Language | Run (`F5`) | Build (`Ctrl+F5`) | Test (`Shift+F5`) |
|----------|-----------|-------------------|-------------------|
| Rust | `cargo run` | `cargo build` | `cargo test` |
| Zenith | `zenith run` | `zenith build` | `zenith test` |
| Zymbol | `zymbol run` | `zymbol build` | `zymbol test` |
| Python | `python3 <file>` | — | `pytest` |
| Go | `go run .` | `go build .` | `go test ./...` |
| JavaScript | `node <file>` | — | `npm test` |

Task output streams into the integrated terminal. When a build or test task finishes, zedit also feeds the output to the Problem Panel to highlight errors.

### Custom tasks

You can override the default tasks in `config.json` (custom task configuration will be documented in a future release). For now, all task presets are language-driven and require no configuration.

---

## 22. Problem Panel

The Problem Panel is a collapsible overlay at the bottom of the editor that aggregates errors and warnings produced by the task runner.

### Opening and navigating

| Key | Action |
|-----|--------|
| `F6` | Toggle the problem panel (show / hide). |
| `Up` / `Down` | Move the selection through the problem list. |
| `Enter` | Jump to the file and line of the selected problem. |
| `Escape` | Close the problem panel. |

### Parsed formats

The panel automatically parses error output from:

- **Rust/Cargo** — `error[E…]: message` / `  --> file:line:col`
- **GCC / Clang** — `file:line:col: error: message`
- **Python** — `File "file", line N` tracebacks
- **Generic** — any line matching `file:line: …` or `file:line:col: …`

### Problem indicators

The status bar shows a combined count of LSP diagnostics and build errors:

```
● src/main.rs  E:2 W:1  Ln 42  Col 8
```

---

## 23. REPL Integration

zedit can send code directly to a live REPL session running in the integrated terminal, enabling an interactive development workflow for supported languages.

### Sending code

| Key | Action |
|-----|--------|
| `Alt+Enter` | Send the current selection to the REPL. If nothing is selected, sends the current line. |

### Supported languages

| Language | REPL command |
|----------|-------------|
| Zenith | `zenith --repl` |
| Zymbol | `zymbol --repl` |

When you press `Alt+Enter` in a Zenith or Zymbol file, zedit:

1. Opens the integrated terminal if it is not already visible.
2. Starts the appropriate REPL if one is not already running.
3. Sends the selected text (or current line) followed by a newline.

The REPL session persists for the lifetime of the editor session — subsequent `Alt+Enter` presses send to the same REPL process.

---

## 24. Troubleshooting

### zedit displays garbled characters or boxes

Your terminal may not support UTF-8. Ensure your locale is set to a UTF-8 locale:

```sh
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8
```

### Colors look wrong or are missing

zedit auto-detects color support. If colors are not showing correctly:

- For 24-bit true color, set: `export COLORTERM=truecolor`
- For 256-color support, ensure `$TERM` contains `256color`, e.g., `export TERM=xterm-256color`

### Ctrl+C does not copy; it interrupts the editor

This can happen if your terminal is configured to pass `SIGINT` on Ctrl+C. zedit intercepts Ctrl+C internally in raw mode, so this should not occur under normal circumstances. If it does, check that no terminal multiplexer (tmux, screen) is intercepting the key before zedit receives it.

### The integrated terminal does not start

Ensure a shell is available. Check that `$SHELL` is set and points to an executable shell, or set `terminal_shell` explicitly in `config.json`. Also verify that `/dev/ptmx` is accessible (it must be readable and writable by your user).

### LSP features are not working

1. Confirm the LSP server binary is installed and on your `$PATH`. Test by running it manually in a terminal.
2. Verify your `config.json` `lsp` section uses the correct language identifier (use lowercase, e.g., `"rust"`, `"python"`).
3. Check that the file you are editing has a recognized extension that matches the configured language.

### A swap file recovery prompt appears on every startup

This means a previous session ended without cleaning up the swap file. If the content in the swap file is no longer needed, choose `D` to delete it. If you want to recover the contents, choose `R`.

### The binary is larger than 500 KB

Running `strip target/release/zedit` removes debug symbols and reduces the binary to approximately 500 KB. Grammar files are loaded from disk at runtime and are not embedded in the binary.

### Config changes are not taking effect

zedit reads `config.json` at startup only. Restart zedit after editing the configuration file.

### Terminal resize does not work correctly

zedit handles `SIGWINCH` for resize notifications. If resize does not work inside a multiplexer (tmux, screen), ensure the multiplexer is correctly propagating `SIGWINCH` to the child process group.

---

## 25. License

zedit is released under the **GNU General Public License v3.0**.

```
zedit — modern console text editor
Copyright (C) the zedit contributors

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
```

The full license text is in the `LICENSE` file in the repository root.

### Third-party components

zedit embeds TextMate grammar files (`.tmLanguage.json`) sourced from open-source repositories. Each embedded grammar retains its original license (MIT or Apache 2.0). The grammar files themselves are data files; they do not affect the license of the zedit binary under GPLv3.

VS Code-compatible color themes shipped with zedit are original works released under the same GPLv3 license as the editor.

---

*zedit is part of the Z ecosystem — Zenith, Zymbol, Minilux.*
