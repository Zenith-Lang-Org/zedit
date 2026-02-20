# Zedit Phase 2 — Shell Studio Code

From text editor to terminal IDE. Phases 7δ–20.

---

## Implementation Roadmap

Phases reordered by dependency graph and complexity (quick wins first, heavy phases deferred until their dependencies are ready).

| #  | Phase | Feature              | ~Lines | Dependencies      | Status |
|----|-------|----------------------|--------|-------------------|--------|
| 1  | 8     | Layout & Pane System | 600    | —                 | DONE   |
| 2  | 10    | Tab Bar              | 250    | —                 | DONE   |
| 3  | 12    | Multi-Cursor Editing | 900    | —                 | DONE   |
| 4  | 13    | Git Gutter           | 630    | —                 | DONE   |
| 5  | 15    | Command Palette      | 530    | —                 | DONE   |
| 6  | 16    | Soft Word Wrap       | 600    | —                 | DONE   |
| 7  | 11    | File Tree Sidebar    | 750    | Phase 8           | DONE   |
| 8  | 9     | Integrated Terminal  | 1,450  | Phase 8           | DONE   |
| 9  | 14    | Session + Swap Files | 650    | Phase 8           | DONE   |
| 10 | 17    | LSP Client           | 1,650  | Phase 9           |        |
| 11 | 19    | Diff / Merge View    | 650    | Phase 8 + 13      |        |
| 12 | 20    | Minimap              | 330    | Phase 8           |        |
| 13 | 18    | Plugin System        | 700    | Phase 15          |        |

Phase 7δ (Runtime Grammar System) is independent and can be done at any point.

Total: ~9,690 new lines across 13 phases.

---

## Phase 7δ (Delta MVP): External Grammar System + VS Code Extension Import

**Goal**: Remove all grammar files from the compiled binary. Language support becomes fully runtime-configurable. Users can import syntax highlighting, LSP configuration, and terminal tasks directly from VS Code extensions — the largest ecosystem of language tooling in existence.

### Problem

Currently, `build.rs` uses `include_str!` to embed every `.tmLanguage.json` file into the binary at compile time. This causes:

1. **Binary bloat** — 22 grammars = 1.9MB of JSON baked into a 2.5MB binary (~76% of total size)
2. **Recompilation required** — adding a language forces `cargo build`
3. **False promise** — the system claims to be "configurable" but the core grammars are hardcoded into the binary
4. **Manual process** — adding a language means hunting for grammar files on GitHub, guessing extensions, and hand-editing JSON

### Vision

```sh
# One command to add full Ruby support:
zedit --import ruby

# What happens behind the scenes:
# 1. Queries VS Code Marketplace for "ruby" language extensions
# 2. Downloads the .vsix (it's a ZIP file)
# 3. Extracts from package.json:
#    - contributes.grammars  → .tmLanguage.json → ~/.config/zedit/grammars/
#    - contributes.languages → extensions, aliases → languages.json
#    - comment token          → languages.json comment field
# 4. Optionally extracts LSP config for Phase 17
# 5. Done. Open a .rb file.
zedit myapp.rb
```

---

### Part A: Runtime Grammar Loading (Infrastructure)

#### Current Architecture

```text
build.rs
  └── reads grammars/*.tmLanguage.json
  └── generates embedded_grammars.rs with include_str!() for each file
  └── embeds grammars/languages.json as EMBEDDED_LANGUAGES_JSON

config.rs
  └── builtin_languages() → parses EMBEDDED_LANGUAGES_JSON (compiled-in)
  └── Config::load() → merges user overrides on top of builtins

highlight.rs
  └── load_grammar() → tries ~/.config/zedit/grammars/ first
  └── falls back to builtin_grammar_str() (compiled-in)
```

#### Target Architecture

```text
Runtime:
  config.rs
    └── load_languages() → reads languages.json from search path
    └── fallback: minimal hardcoded set (plain text only)

  highlight.rs
    └── load_grammar() → reads .tmLanguage.json from search path:
        1. ~/.config/zedit/grammars/          (user, highest priority)
        2. /usr/share/zedit/grammars/         (system-wide, package manager)
        3. /usr/local/share/zedit/grammars/   (manual system install)
        4. ./grammars/                        (dev mode, project-local)
```

#### Step A1: Grammar search path

```rust
pub fn grammar_search_paths() -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        paths.push(format!("{}/.config/zedit", home));
    }
    paths.push("/usr/share/zedit".to_string());
    paths.push("/usr/local/share/zedit".to_string());
    paths.push("grammars".to_string());
    paths
}
```

#### Step A2: Runtime language loading (replaces compile-time)

```rust
pub fn load_languages() -> Vec<LanguageDef> {
    for dir in grammar_search_paths() {
        let lang_file = format!("{}/languages.json", dir);
        if let Ok(content) = std::fs::read_to_string(&lang_file) {
            if let Some(langs) = parse_languages_array(&content) {
                if !langs.is_empty() { return langs; }
            }
        }
    }
    // Fallback: plain text only
    vec![LanguageDef { name: "text".into(), extensions: vec!["txt".into()],
                       grammar_file: String::new(), comment: None }]
}
```

#### Step A3: Runtime grammar loading (replaces `builtin_grammar_str()`)

```rust
pub fn load_grammar(lang: &str, languages: &[LanguageDef]) -> Option<Grammar> {
    let lang_def = languages.iter().find(|l| l.name == lang)?;
    for dir in grammar_search_paths() {
        let path = format!("{}/grammars/{}", dir, lang_def.grammar_file);
        if let Ok(json_str) = std::fs::read_to_string(&path) {
            if let Ok(val) = json_parser::JsonValue::parse(&json_str) {
                if let Ok(g) = Grammar::from_json(&val) { return Some(g); }
            }
        }
    }
    None
}
```

#### Step A4: `zedit --install` — bootstrap built-in grammars

Copies the bundled `grammars/` directory to `~/.config/zedit/`:

```text
~/.config/zedit/
  languages.json
  grammars/
    rust.tmLanguage.json
    python.tmLanguage.json
    ...
```

#### Step A5: Remove `build.rs` grammar embedding

- Delete `build.rs` (or reduce to version stamp only)
- Remove all `include_str!`, `EMBEDDED_LANGUAGES_JSON`, `builtin_grammar_str()`
- Binary drops from 2.5MB → ~500KB

#### Step A6: Dev mode transparency

`cargo run` works without `--install` because `./grammars/` is in the search path. Contributors just drop files and edit `languages.json`. Zero Rust code changes.

---

### Part B: VS Code Extension Import System

The killer feature. Import language support directly from the VS Code Marketplace — the world's largest collection of language tooling (40,000+ extensions).

#### VS Code Extension Anatomy

A `.vsix` file is a **ZIP archive** containing:

```text
extension/
  package.json                 ← manifest with all contributions
  syntaxes/
    lang.tmLanguage.json       ← TextMate grammars
  language-configuration.json  ← comment tokens, brackets, etc.
  ...
```

The `package.json` → `contributes` section declares everything:

```json
{
  "contributes": {
    "languages": [{
      "id": "ruby",
      "aliases": ["Ruby", "rb"],
      "extensions": [".rb", ".rake", ".gemspec", ".ru"],
      "configuration": "./language-configuration.json"
    }],
    "grammars": [{
      "language": "ruby",
      "scopeName": "source.ruby",
      "path": "./syntaxes/ruby.tmLanguage.json"
    }]
  }
}
```

The `language-configuration.json` contains comment tokens:

```json
{
  "comments": {
    "lineComment": "#",
    "blockComment": ["=begin", "=end"]
  },
  "brackets": [["(", ")"], ["[", "]"], ["{", "}"]],
  "autoClosingPairs": [{"open": "\"", "close": "\""}]
}
```

#### VS Code Marketplace REST API

**Search for extensions:**

```text
POST https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery
Headers:
  Content-Type: application/json
  Accept: application/json; charset=utf-8; api-version=7.2-preview.1

Body:
{
  "filters": [{
    "criteria": [{ "filterType": 7, "value": "ruby" }],
    "pageNumber": 1, "pageSize": 5,
    "sortBy": 0, "sortOrder": 0
  }],
  "flags": 16863
}
```

Filter types: 4 = extension ID, 7 = name search, 10 = full-text search.

**Download VSIX:**

```text
GET https://marketplace.visualstudio.com/_apis/public/gallery/publishers/{publisher}/vsextensions/{extension}/{version}/vspackage
```

Example:
```text
GET https://marketplace.visualstudio.com/_apis/public/gallery/publishers/rebornix/vsextensions/ruby/0.28.1/vspackage
```

#### New File: `src/import.rs` (~600 lines)

```rust
/// VS Code extension importer — downloads .vsix from the Marketplace,
/// extracts grammars, language definitions, and configuration.

pub struct VsixImporter {
    config_dir: PathBuf,  // ~/.config/zedit
}

/// Extracted language data from a VS Code extension.
pub struct ImportedLanguage {
    pub name: String,
    pub aliases: Vec<String>,
    pub extensions: Vec<String>,
    pub grammar_file: String,         // filename written to grammars/
    pub grammar_content: String,      // the .tmLanguage.json content
    pub comment: Option<String>,      // line comment token
    pub block_comment: Option<(String, String)>, // open/close
    pub scope_name: String,
    // Future (Phase 17): LSP configuration
    pub lsp_command: Option<String>,
    pub lsp_args: Option<Vec<String>>,
}

/// Marketplace search result.
pub struct ExtensionInfo {
    pub publisher: String,
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub download_url: String,
}

impl VsixImporter {
    /// Search the VS Code Marketplace for extensions matching a query.
    pub fn search(&self, query: &str) -> Result<Vec<ExtensionInfo>, String> {
        // 1. HTTP POST to marketplace API (using custom minimal HTTP client)
        // 2. Parse JSON response
        // 3. Extract publisher, name, version, download URL
        // 4. Return list of matches
    }

    /// Download and extract a VS Code extension.
    pub fn import(&self, ext: &ExtensionInfo) -> Result<Vec<ImportedLanguage>, String> {
        // 1. Download .vsix to temp file
        // 2. Unzip (custom ZIP reader — zero deps)
        // 3. Parse extension/package.json
        // 4. For each contributes.grammars entry:
        //    a. Read the .tmLanguage.json from the ZIP
        //    b. Extract language config (comments, brackets)
        //    c. Build ImportedLanguage
        // 5. Return all extracted languages
    }

    /// Install extracted languages into ~/.config/zedit/.
    pub fn install(&self, langs: &[ImportedLanguage]) -> Result<(), String> {
        // 1. Write each grammar to ~/.config/zedit/grammars/
        // 2. Read existing languages.json
        // 3. Merge new entries (replace existing by name)
        // 4. Write updated languages.json
        // 5. Print summary
    }
}
```

#### New File: `src/http.rs` (~250 lines)

Minimal HTTP/1.1 client over raw TCP sockets. Zero external deps.

```rust
/// Minimal HTTP client — only supports GET and POST over TLS.
/// Uses the system's TLS library via a simple approach:
/// spawn `curl` as a subprocess (available on virtually all systems).
///
/// Why subprocess instead of raw TLS:
/// - Implementing TLS from scratch = ~5,000+ lines (crypto, certificates)
/// - curl is pre-installed on Linux, macOS, WSL, Git Bash
/// - We only need 2 operations: GET (download) and POST (search)
/// - Zero-dep constraint applies to COMPILE-time deps, not runtime tools

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub fn http_get(url: &str, output_path: &Path) -> Result<(), String> {
    // spawn: curl -fsSL -o {output_path} {url}
}

pub fn http_post(url: &str, headers: &[(&str, &str)], body: &str) -> Result<HttpResponse, String> {
    // spawn: curl -s -X POST -H {headers} -d {body} {url}
    // capture stdout
}
```

#### New File: `src/zip.rs` (~350 lines)

Minimal ZIP reader for `.vsix` extraction. Zero external deps.

```rust
/// Minimal ZIP file reader.
/// Only supports: Store (no compression) and Deflate.
/// VSIX files use standard ZIP format.

pub struct ZipArchive {
    entries: Vec<ZipEntry>,
    data: Vec<u8>,
}

pub struct ZipEntry {
    pub name: String,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub compression: Compression,
    pub offset: usize,  // offset to local file header
}

pub enum Compression {
    Store,    // no compression (method 0)
    Deflate,  // standard deflate (method 8)
}

impl ZipArchive {
    /// Parse a ZIP archive from raw bytes.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self, String> {
        // 1. Find End of Central Directory record (scan backwards for signature)
        // 2. Read Central Directory entries
        // 3. Build entry list
    }

    /// Read a file entry by name.
    pub fn read_entry(&self, name: &str) -> Result<Vec<u8>, String> {
        // 1. Find entry
        // 2. Seek to local file header
        // 3. If Store: return raw bytes
        // 4. If Deflate: decompress (custom inflate implementation)
    }
}
```

#### Custom DEFLATE Decompressor (~200 lines within `zip.rs`)

DEFLATE is a well-specified algorithm (RFC 1951). We need a decompressor only (not compressor). Core implementation:

```rust
/// Inflate (decompress) DEFLATE data.
/// Implements RFC 1951: fixed Huffman + dynamic Huffman + stored blocks.
fn inflate(compressed: &[u8]) -> Result<Vec<u8>, String> {
    // Bit reader
    // For each block:
    //   - Block type 0 (stored): copy literal bytes
    //   - Block type 1 (fixed Huffman): use predefined code tables
    //   - Block type 2 (dynamic Huffman): read code tables, then decode
    // Length/distance pairs reference previous output (sliding window)
}
```

This is ~200 lines of straightforward bit manipulation. The algorithm is fully specified and well-documented. Many clean-room implementations exist for reference.

#### CLI Interface

```text
zedit --import <query>           Search marketplace and import
zedit --import-vsix <path>       Import from local .vsix file
zedit --import-list              List installed imported languages
zedit --import-remove <name>     Remove an imported language
```

**Interactive flow for `zedit --import ruby`:**

```text
$ zedit --import ruby

Searching VS Code Marketplace for "ruby"...

  1. rebornix.ruby (Ruby) v0.28.1
     Ruby language support and debugging for VS Code
  2. wingrunr21.vscode-ruby (VSCode Ruby) v0.28.0
     Syntax highlighting, snippet, and language configuration
  3. shopify.ruby-lsp (Ruby LSP) v0.7.4
     VS Code extension for Ruby LSP

Select extension [1-3]: 1

Downloading rebornix.ruby v0.28.1... OK (2.3MB)
Extracting...
  ✓ Grammar: ruby.tmLanguage.json (source.ruby)
  ✓ Extensions: .rb, .rake, .gemspec, .ru, .erb
  ✓ Comment: #
  ✓ LSP: saved to config (solargraph)

Installed "ruby" to ~/.config/zedit/grammars/
```

#### Config File Structure After Import

```text
~/.config/zedit/
  languages.json                    ← auto-updated by importer
  grammars/
    ruby.tmLanguage.json            ← extracted from .vsix
    kotlin.tmLanguage.json
    ...
  config.json                       ← LSP config merged here
```

The `languages.json` gains entries automatically:

```json
{ "name": "ruby", "extensions": ["rb", "rake", "gemspec", "ru"],
  "grammar": "ruby.tmLanguage.json", "comment": "#",
  "source": "vscode:rebornix.ruby@0.28.1" }
```

The `source` field tracks where the grammar came from (for updates).

The `config.json` gains LSP entries (used by Phase 17):

```json
{
  "lsp": {
    "ruby": { "command": "solargraph", "args": ["stdio"] },
    "kotlin": { "command": "kotlin-language-server" }
  }
}
```

---

### Part C: Preparing for Phase 17 (LSP) and Phase 9 (Terminal)

The import system extracts more than just grammars. VS Code extensions often declare:

#### LSP Configuration (Phase 17 preparation)

From `package.json` → `contributes.configuration` and activation events:

```json
{
  "activationEvents": ["onLanguage:ruby"],
  "contributes": {
    "configuration": {
      "properties": {
        "ruby.lsp.command": { "type": "string", "default": "solargraph" }
      }
    }
  }
}
```

The importer extracts this and writes it to `config.json → lsp` section. When Phase 17 lands, the LSP client reads this config and knows which server to spawn for each language. **Zero additional user configuration needed.**

#### Task/Terminal Configuration (Phase 9 preparation)

From `package.json` → `contributes.taskDefinitions` and terminal profiles:

```json
{
  "contributes": {
    "taskDefinitions": [{
      "type": "ruby",
      "properties": {
        "task": { "type": "string" }
      }
    }]
  }
}
```

The importer stores task templates in `config.json → tasks` for Phase 9's integrated terminal:

```json
{
  "tasks": {
    "ruby": {
      "run": "ruby ${file}",
      "test": "ruby -e 'require \"minitest/autorun\"' ${file}",
      "repl": "irb"
    }
  }
}
```

This means when Phase 9 (terminal) is implemented, `Ctrl+F5` can run the current file using the correct interpreter — because the import already configured it.

---

### File Changes Summary

| File | Action |
|------|--------|
| `build.rs` | **Delete** |
| `src/config.rs` | Remove embedded refs. Add `load_languages()`, `grammar_search_paths()` |
| `src/syntax/highlight.rs` | Update `load_grammar()` to use search path |
| `src/import.rs` | **New** — VS Code extension importer |
| `src/http.rs` | **New** — Minimal HTTP client (curl subprocess) |
| `src/zip.rs` | **New** — ZIP reader + DEFLATE decompressor |
| `src/main.rs` | Add `--install`, `--import`, `--import-vsix` subcommands |

### Complexity

| Component | Lines |
|-----------|-------|
| **Part A: Runtime loading** | |
| `grammar_search_paths()` + `load_languages()` | ~50 |
| `load_grammar()` refactor | ~30 |
| `--install` subcommand | ~60 |
| Remove `build.rs` + embedded refs | -65 (deletion) |
| Test updates | ~60 |
| **Part B: VS Code import** | |
| `import.rs` — Marketplace API + VSIX parser | ~600 |
| `http.rs` — HTTP via curl subprocess | ~250 |
| `zip.rs` — ZIP reader + DEFLATE inflate | ~350 |
| CLI interface + interactive selection | ~120 |
| **Part C: Config extraction** | |
| LSP config extraction + merge | ~80 |
| Task/terminal config extraction | ~60 |
| **Total** | **~1,595 lines** |

### Result

| Metric | Before | After |
|--------|--------|-------|
| Binary size (stripped) | 2.5MB | ~500KB |
| Add new language | Manual: find grammar, download, configure | `zedit --import <name>` |
| Available languages | 22 built-in | **40,000+ from VS Code Marketplace** |
| Recompilation needed | Yes | **Never** |
| LSP auto-configured | No | **Yes** (extracted from extension) |
| Terminal tasks auto-configured | No | **Yes** (extracted from extension) |

### User Workflows

**Import from Marketplace (primary):**
```sh
zedit --import kotlin        # search, select, download, install — done
zedit main.kt                # syntax highlighting works immediately
```

**Import from local .vsix file:**
```sh
zedit --import-vsix ~/Downloads/custom-lang-0.1.0.vsix
```

**Manual drop-in (still works):**
```sh
cp my-grammar.tmLanguage.json ~/.config/zedit/grammars/
# edit ~/.config/zedit/languages.json
```

**Dev mode (contributors):**
```sh
# Just edit grammars/languages.json + drop .tmLanguage.json
cargo run -- myfile.xyz       # works via ./grammars/ search path
```

---

## Current State (Post-MVP)

Zedit's MVP is complete: **~10,250 lines of pure Rust**, zero external dependencies. The editor ships as a single static binary (~500KB stripped, ~2.2MB with embedded grammars) and supports:

- **Gap buffer** text storage with line offset cache (`buffer.rs`)
- **Diff-based rendering** — only changed cells emit ANSI sequences (`render.rs`)
- **Multi-buffer** editing with per-buffer undo stacks (`editor/buffer_state.rs`)
- **TextMate syntax highlighting** — custom JSON parser, regex engine, tokenizer (`syntax/`)
- **VS Code-compatible themes** with 24-bit/256/16 color fallback (`syntax/theme.rs`)
- **Modern keybindings** — Ctrl+C/V/X/S/Z, no modes (`editor/mod.rs`)
- **Incremental search & replace** with regex support (`editor/search.rs`)
- **Operation-based undo/redo** with 500ms transaction grouping (`undo.rs`)
- **OSC 52 clipboard**, SGR mouse events, bracketed paste (`terminal.rs`, `input.rs`)

### Architecture Snapshot

```text
src/
  main.rs                  Entry point, CLI args, config loading
  terminal.rs              Raw mode via libc FFI, SIGWINCH, alternate screen
  input.rs                 Escape sequence decoding, mouse events, bracketed paste
  buffer.rs                Gap buffer, line cache, UTF-8 aware
  cursor.rs                Position + desired_col for vertical movement
  render.rs                Screen/Cell/Color, diff-based rendering
  undo.rs                  Operation/Group/UndoStack with GroupContext
  config.rs                Runtime configuration, language definitions
  unicode.rs               Unicode width utilities
  editor/
    mod.rs                 Editor struct, main loop, event dispatch
    buffer_state.rs        BufferState (Buffer + Cursor + UndoStack + Highlighter)
    editing.rs             Insert, delete, indent, comment toggle
    selection.rs           Selection (anchor/head byte offsets), clipboard ops
    search.rs              SearchState, incremental find/replace
    view.rs                Viewport calculation, gutter, syntax-colored rendering
    prompt.rs              Mini-prompts (Open, SaveAs, Find, Replace, GoToLine)
    helpers.rs             Utility functions
    tests.rs               Integration tests
  syntax/
    mod.rs                 Public API
    json_parser.rs         Custom JSON parser (~300 lines)
    regex.rs               Custom regex engine (~500 lines)
    grammar.rs             TextMate grammar data structures
    tokenizer.rs           Stateful line tokenizer with LineState
    theme.rs               VS Code theme loader, scope hierarchy matching
    highlight.rs           Highlighter combining grammar + theme
```

### Key Types (for reference in later phases)

```rust
// editor/mod.rs
pub struct Editor {
    buffers: Vec<BufferState>,
    active_buffer: usize,
    terminal: Terminal,
    screen: Screen,
    color_mode: ColorMode,
    config: Config,
    status_height: usize,
    message: Option<String>,
    message_type: MessageType,
    quit_confirm: bool,
    clipboard: String,
    prompt: Option<Prompt>,
    mouse_dragging: bool,
    help_visible: bool,
    running: bool,
}

// editor/buffer_state.rs
pub(super) struct BufferState {
    pub(super) buffer: Buffer,
    pub(super) cursor: Cursor,
    pub(super) scroll_row: usize,
    pub(super) scroll_col: usize,
    pub(super) selection: Option<Selection>,
    pub(super) undo_stack: UndoStack,
    pub(super) search: Option<SearchState>,
    pub(super) highlighter: Option<Highlighter>,
    pub(super) gutter_width: usize,
}

// render.rs
pub struct Screen { width, height, cells: Vec<Vec<Cell>>, prev_cells: Vec<Vec<Cell>> }
pub struct Cell { ch: char, fg: Color, bg: Color, bold: bool, wide_cont: bool }
pub enum Color { Default, Ansi(u8), Color256(u8), Rgb(u8, u8, u8) }
```

---

## Design Constraints (Carried Forward)

1. **Zero external dependencies** — only `std` + libc FFI. Implement PTY management, VT100 emulation, diff algorithms, fuzzy matching, LSP JSON-RPC, and plugin hosting in-house.
2. **Performance budgets** — startup < 10ms, keypress-to-screen < 5ms, terminal command round-trip < 16ms (60fps).
3. **Single-threaded** — the main loop is a synchronous read-dispatch-render cycle. Async work (LSP, long file I/O) uses non-blocking `poll()` integrated into the main loop, not OS threads.
4. **Binary size** — target < 1.5MB stripped (grammars add ~1.7MB on top).
5. **UTF-8 native** — all text operations assume valid UTF-8.
6. **Graceful degradation** — every feature must work (or silently degrade) on 16-color terminals without mouse support.

---

## Phase 8: Layout & Pane System

**Foundation for everything.** Splits, sidebars, and terminal panes all need a recursive layout engine.

### New File: `src/layout.rs` (~600 lines)

```rust
/// Unique identifier for a pane within the layout tree.
pub type PaneId = u32;

/// Direction of a split.
#[derive(Clone, Copy, PartialEq)]
pub enum SplitDir {
    Horizontal, // left | right
    Vertical,   // top / bottom
}

/// A node in the recursive layout tree.
pub enum LayoutNode {
    /// A leaf pane with content.
    Leaf {
        id: PaneId,
        content: PaneContent,
    },
    /// A split containing two or more children.
    Split {
        dir: SplitDir,
        children: Vec<LayoutNode>,
        /// Proportional sizes (sum = 1.0). One entry per child.
        ratios: Vec<f32>,
    },
}

/// What a pane displays.
pub enum PaneContent {
    Editor(usize),       // index into Editor.buffers
    Terminal(usize),     // index into Editor.terminals (Phase 9)
    FileTree,            // sidebar (Phase 11)
    Minimap,             // code overview (Phase 20)
}

/// Resolved screen rectangle for a pane after layout calculation.
#[derive(Clone, Copy)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// Maps PaneId → Rect after layout resolution.
pub struct LayoutState {
    root: LayoutNode,
    next_id: PaneId,
    rects: Vec<(PaneId, Rect)>, // cached after resolve()
}
```

### Layout Resolution

```rust
impl LayoutState {
    /// Resolve the layout tree into concrete screen rectangles.
    /// Called on resize and after split/close operations.
    pub fn resolve(&mut self, total: Rect) {
        self.rects.clear();
        self.resolve_node(&self.root, total);
    }

    fn resolve_node(&mut self, node: &LayoutNode, area: Rect) {
        match node {
            LayoutNode::Leaf { id, .. } => {
                self.rects.push((*id, area));
            }
            LayoutNode::Split { dir, children, ratios } => {
                let mut offset = 0u16;
                for (i, child) in children.iter().enumerate() {
                    let size = match dir {
                        SplitDir::Horizontal => {
                            let w = (area.width as f32 * ratios[i]).round() as u16;
                            Rect { x: area.x + offset, y: area.y, width: w, height: area.height }
                        }
                        SplitDir::Vertical => {
                            let h = (area.height as f32 * ratios[i]).round() as u16;
                            Rect { x: area.x, y: area.y + offset, width: area.width, height: h }
                        }
                    };
                    self.resolve_node(child, size);
                    offset += match dir {
                        SplitDir::Horizontal => size.width,
                        SplitDir::Vertical => size.height,
                    };
                }
            }
        }
    }
}
```

### Editor Integration

The `Editor` struct gains a `layout: LayoutState` field. The existing single-pane rendering path (`editor/view.rs`) is wrapped behind `Rect` — each pane renders into its assigned rectangle instead of the full screen.

```rust
// In Editor
layout: LayoutState,
active_pane: PaneId,
```

The main loop changes from rendering one buffer to iterating over `layout.rects` and rendering each pane's content into its `Rect`. The `Screen` struct already supports arbitrary positioning via cell coordinates, so no changes to `render.rs` are needed.

### Keybindings

| Key                  | Action                     |
| -------------------- | -------------------------- |
| `Ctrl+\`             | Split horizontal           |
| `Ctrl+Shift+\`       | Split vertical             |
| `Ctrl+Shift+W`       | Close pane                 |
| `Alt+Arrow`          | Focus adjacent pane        |
| `Alt+Shift+Arrow`    | Resize pane (±2 cols/rows) |

### Rendering

Each pane draws a 1-character border on its right/bottom edge using box-drawing characters (`│`, `─`, `┼`). The active pane's border is highlighted with the theme's accent color. Borders consume 1 column/row from the pane's usable area.

### Complexity: ~600 lines

| Component             | Lines |
| --------------------- | ----- |
| `layout.rs` structs   | ~80   |
| Tree resolution       | ~120  |
| Split/close/resize    | ~150  |
| Focus navigation      | ~100  |
| Border rendering      | ~80   |
| Editor integration    | ~70   |

---

## Phase 9: Integrated Terminal

Run a shell inside a Zedit pane. The editor becomes the outer container.

### New Files

| File                  | Lines | Purpose                              |
| --------------------- | ----- | ------------------------------------ |
| `src/pty.rs`          | ~350  | PTY allocation + child process mgmt  |
| `src/vterm.rs`        | ~1,100| VT100/xterm escape sequence emulator |

### `src/pty.rs` — PTY Management (~350 lines)

```rust
/// Represents an open pseudo-terminal with a child process.
pub struct Pty {
    master_fd: i32,          // PTY master file descriptor
    child_pid: i32,          // Child process PID
    cols: u16,
    rows: u16,
}
```

FFI calls (all standard POSIX, available on Linux and macOS):

```rust
// libc FFI declarations
extern "C" {
    fn openpty(
        master: *mut i32, slave: *mut i32,
        name: *mut u8, termp: *const Termios, winp: *const Winsize,
    ) -> i32;
    fn fork() -> i32;
    fn setsid() -> i32;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn execvp(file: *const u8, argv: *const *const u8) -> i32;
    fn close(fd: i32) -> i32;
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
    fn write(fd: i32, buf: *const u8, count: usize) -> isize;
    fn kill(pid: i32, sig: i32) -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
}
```

PTY lifecycle:

1. `openpty()` — allocate master/slave pair
2. `fork()` — child process
3. In child: `setsid()`, `ioctl(TIOCSCTTY)`, `dup2()` slave to stdin/stdout/stderr, `execvp()` shell
4. In parent: close slave fd, set master to non-blocking, store `Pty`
5. On resize: `ioctl(master_fd, TIOCSWINSZ, &winsize)` to notify child

### `src/vterm.rs` — VT100 Emulator (~1,100 lines)

A minimal terminal emulator that interprets the escape sequences a shell sends back and renders them into a `Cell` grid.

```rust
/// Virtual terminal state — interprets PTY output into a cell grid.
pub struct VTerm {
    cols: u16,
    rows: u16,
    cells: Vec<Vec<Cell>>,        // The terminal's screen buffer
    cursor_row: u16,
    cursor_col: u16,
    /// Scroll region (top, bottom) — for programs that set scroll margins.
    scroll_top: u16,
    scroll_bottom: u16,
    /// Current text attributes applied to new characters.
    attr: CellAttr,
    /// Scrollback buffer (lines that scrolled off the top).
    scrollback: Vec<Vec<Cell>>,
    scrollback_limit: usize,      // max lines to keep (default: 1000)
    /// Parser state machine for escape sequences.
    parse_state: ParseState,
    /// Buffer for incomplete escape sequences between read() calls.
    partial: Vec<u8>,
    /// Alternate screen buffer (used by programs like vim, less).
    alt_cells: Option<Vec<Vec<Cell>>>,
    alt_cursor: Option<(u16, u16)>,
}

struct CellAttr {
    fg: Color,
    bg: Color,
    bold: bool,
    underline: bool,
    inverse: bool,
}

enum ParseState {
    Ground,
    Escape,           // saw \x1b
    CsiEntry,         // saw \x1b[
    CsiParam,         // collecting numeric params
    OscString,        // saw \x1b]
}
```

Supported escape sequences (covers 95%+ of real shell/TUI usage):

| Category          | Sequences                                                |
| ----------------- | -------------------------------------------------------- |
| Cursor movement   | CUU/CUD/CUF/CUB, CUP (absolute), save/restore          |
| Erase             | ED (erase display), EL (erase line), ECH (erase chars)   |
| Scroll            | SU/SD (scroll up/down), DECSTBM (set scroll region)     |
| Text attributes   | SGR (bold, underline, inverse, fg/bg colors, 256, RGB)   |
| Screen modes      | DECSET/DECRST 1049 (alternate screen), 25 (cursor vis)  |
| Tabs              | HT, CHT, TBC                                            |
| Misc              | BEL (ignored), OSC title (captured for tab label)        |

### Main Loop Integration

The main loop gains a `poll()` call that checks both stdin (user input) and all PTY master fds simultaneously:

```rust
// Pseudocode for the enhanced main loop
loop {
    // 1. Build poll set: stdin + all pty master fds
    let mut fds = vec![PollFd { fd: STDIN, events: POLLIN }];
    for term in &self.terminals {
        fds.push(PollFd { fd: term.pty.master_fd, events: POLLIN });
    }

    // 2. poll() with timeout for cursor blink / message expiry
    poll(&mut fds, timeout_ms);

    // 3. Handle stdin if ready
    if fds[0].revents & POLLIN != 0 {
        let event = input::read_event(&self.terminal);
        self.handle_event(event);
    }

    // 4. Handle PTY output if ready
    for (i, term) in self.terminals.iter_mut().enumerate() {
        if fds[i + 1].revents & POLLIN != 0 {
            let mut buf = [0u8; 4096];
            let n = read(term.pty.master_fd, &mut buf);
            if n > 0 {
                term.vterm.process(&buf[..n as usize]);
            }
        }
    }

    // 5. Check resize
    // 6. Render all panes
}
```

When a terminal pane is focused, all keyboard input (except `Ctrl+Shift+T` to toggle back) is forwarded to the PTY via `write(master_fd, ...)`.

### Keybindings

| Key                | Action                              |
| ------------------ | ----------------------------------- |
| `` Ctrl+` ``       | Toggle terminal panel (bottom pane) |
| `Ctrl+Shift+T`     | New terminal instance               |

The terminal pane captures almost all keys when focused. The escape hatch is `Alt+Arrow` (from Phase 8) to move focus back to an editor pane.

### Complexity: ~1,450 lines

| Component             | Lines  |
| --------------------- | ------ |
| `pty.rs` FFI + mgmt   | ~350   |
| `vterm.rs` emulator    | ~850   |
| `vterm.rs` SGR parser  | ~150   |
| Editor integration     | ~100   |

---

## Phase 10: Tab Bar

Visual buffer switching at the top of the screen.

### New File: `src/editor/tabs.rs` (~250 lines)

```rust
/// Represents the tab bar state.
pub struct TabBar {
    /// Visible scroll offset when tabs overflow the screen width.
    scroll_offset: usize,
}

/// Information needed to render a single tab.
pub struct TabInfo {
    pub label: String,         // filename or "untitled-N"
    pub modified: bool,        // show dot indicator
    pub is_active: bool,       // highlight
}
```

### Rendering

The tab bar occupies line 0 of the screen (the status bar moves from line `height-2` to line `height-2`, unchanged). Each tab shows:

```text
 filename.rs ● │ main.rs │ untitled-1 │
```

- Active tab: theme `editor.selectionBackground` + bold text
- Modified indicator: `●` after filename
- Overflow: `◀` / `▶` arrows when tabs exceed screen width
- Mouse: click tab to switch, middle-click to close

### Keybindings

Existing buffer-switch keys gain visual feedback:

| Key              | Action                            |
| ---------------- | --------------------------------- |
| `Ctrl+PageDown`  | Next tab (existing, now visual)   |
| `Ctrl+PageUp`    | Previous tab (existing)           |
| `Ctrl+W`         | Close tab (existing)              |
| `Ctrl+N`         | New tab (existing)                |

No new keybindings needed — this phase adds visual representation to existing behavior.

### Complexity: ~250 lines

| Component            | Lines |
| -------------------- | ----- |
| Tab bar rendering    | ~100  |
| Overflow/scroll      | ~50   |
| Mouse click handling | ~60   |
| Editor integration   | ~40   |

### Implementation Notes (DONE)

Tab bar implemented inline in `src/editor/view.rs` (`render_tab_bar()`) and `src/editor/editing.rs` (mouse click handling) rather than in a separate `src/editor/tabs.rs` file.

**Deviations from plan:**
- No separate `TabBar` or `TabInfo` structs — tab state is computed on-the-fly from `buffers` during rendering. `tab_bar_height` and `tab_regions` fields on `Editor` track the tab bar layout.
- Modified indicator uses ` [+] ` suffix instead of `●` dot.
- No middle-click to close (not implemented).
- Overflow scroll uses `" < "` / `" > "` arrow labels (3 cols each) instead of `◀` / `▶`.
- `tab_scroll_offset: usize` field on `Editor` tracks the first visible tab index (default 0).
- Auto-scroll: before rendering, the active buffer is guaranteed visible by adjusting `tab_scroll_offset`.
- Arrow click regions use sentinel values in `tab_regions`: `usize::MAX` for left arrow, `usize::MAX - 1` for right arrow.
- Separator between tabs uses `" │ "` (3 cols) with dim color.

**Files changed:**
- `src/editor/mod.rs` — `tab_bar_height`, `tab_regions`, `tab_scroll_offset` fields in `Editor` struct, initialized in all 3 constructors.
- `src/editor/view.rs` — `render_tab_bar()` with scroll support, pre-computes labels/widths, renders arrows when tabs overflow.
- `src/editor/editing.rs` — Tab bar mouse click handler with sentinel detection for scroll arrows.

---

## Phase 11: File Tree Sidebar

Project-level navigation panel on the left side.

### New File: `src/filetree.rs` (~750 lines)

```rust
/// A node in the file tree.
pub struct TreeNode {
    pub name: String,
    pub path: PathBuf,
    pub kind: NodeKind,
    pub children: Vec<TreeNode>,  // sorted: dirs first, then alpha
    pub expanded: bool,           // only meaningful for directories
    pub depth: u16,               // nesting level (for indentation)
}

pub enum NodeKind {
    File,
    Directory,
    Symlink,
}

/// File tree panel state.
pub struct FileTree {
    root: TreeNode,
    /// Flattened visible nodes (expanded dirs + their children).
    visible: Vec<FlatNode>,
    /// Currently highlighted row in the visible list.
    cursor: usize,
    /// Scroll offset for long file lists.
    scroll: usize,
    /// Width of the sidebar in columns.
    width: u16,
    /// Filter pattern for file search within tree.
    filter: Option<String>,
}

struct FlatNode {
    index: usize,       // index into some node storage
    depth: u16,
    kind: NodeKind,
    expanded: bool,
    name_range: (usize, usize), // for highlight during filter
}
```

### Directory Scanning

```rust
impl FileTree {
    /// Scan a directory non-recursively. Only expand children on demand.
    pub fn scan_dir(path: &Path) -> Result<Vec<TreeNode>, String> {
        // Uses std::fs::read_dir()
        // Sorts: directories first, then alphabetical (case-insensitive)
        // Skips hidden files (configurable), .git, node_modules, target/
    }
}
```

Directories are scanned lazily — only when the user expands them. This keeps startup fast even for large projects.

### Rendering

The file tree renders into its `Rect` (from the layout system, Phase 8):

```text
▼ src/
  ▼ editor/
      buffer_state.rs
      editing.rs
    ► syntax/
    buffer.rs
    main.rs
  ▶ grammars/
  Cargo.toml
```

- `▶`/`▼` for collapsed/expanded directories
- File icons via Unicode if terminal supports it (optional, detected via `$TERM`)
- Active file highlighted
- Current cursor line has reverse video

### Keybindings (when file tree is focused)

| Key              | Action                            |
| ---------------- | --------------------------------- |
| `Ctrl+B`         | Toggle file tree sidebar          |
| `Up/Down`        | Move cursor in tree               |
| `Enter`          | Open file / toggle directory      |
| `Right`          | Expand directory                  |
| `Left`           | Collapse directory / go to parent |
| `a`              | New file (prompt for name)        |
| `A`              | New directory                     |
| `d`              | Delete (confirm prompt)           |
| `r`              | Rename (inline edit)              |
| `/`              | Filter files by name              |
| `Escape`         | Clear filter / return to editor   |

### Ignored Paths (default)

```rust
const DEFAULT_IGNORED: &[&str] = &[
    ".git", "node_modules", "target", "__pycache__",
    ".DS_Store", "thumbs.db", ".idea", ".vscode",
];
```

Configurable via `config.json` → `"filetree.ignored": [...]`.

### Complexity: ~750 lines

| Component             | Lines |
| --------------------- | ----- |
| Tree data structures  | ~80   |
| Directory scanning    | ~120  |
| Flatten/expand logic  | ~130  |
| Rendering             | ~150  |
| Keyboard navigation   | ~100  |
| File operations       | ~100  |
| Editor integration    | ~70   |

---

## Phase 12: Multi-Cursor Editing

Select the next occurrence with `Ctrl+D`, type to edit all at once.

### Refactor: `cursor.rs` + `editor/buffer_state.rs` + `editor/editing.rs` (~900 lines changed)

This phase modifies existing code rather than adding a new module. The key change: `BufferState` holds a `Vec<CursorSelection>` instead of a single `Cursor` + `Option<Selection>`.

```rust
/// A cursor with an optional selection range. Multi-cursor support
/// means BufferState holds a Vec of these.
pub struct CursorSelection {
    pub cursor: Cursor,
    pub selection: Option<Selection>,
}

// In BufferState (replaces cursor + selection fields):
pub(super) cursors: Vec<CursorSelection>,
pub(super) primary: usize,  // index of the primary cursor
```

### Invariants

1. Cursors are always sorted by position (ascending byte offset).
2. No two cursor selections may overlap — if they would, they merge.
3. The primary cursor is the one that determines viewport scrolling.
4. All edit operations (insert char, delete, paste) apply to every cursor independently, processing from **last to first** to preserve byte offsets.

### Edit Dispatch (Last-to-First)

```rust
fn insert_at_all_cursors(&mut self, text: &str) {
    // Process cursors in reverse order so byte offsets stay valid
    let indices: Vec<usize> = (0..self.cursors.len()).rev().collect();
    for i in indices {
        let pos = self.cursors[i].cursor.byte_offset(&self.buffer);
        self.buffer.insert(pos, text);
        // Adjust cursor position
        self.cursors[i].cursor.advance(text.len());
    }
    // Adjust all cursor offsets for earlier insertions
    self.recalculate_cursor_offsets();
}
```

### Adding Cursors

| Key                   | Action                                         |
| --------------------- | ---------------------------------------------- |
| `Ctrl+D`              | Select next occurrence of current selection/word|
| `Ctrl+Shift+D`        | Skip current, select next occurrence            |
| `Alt+Click`           | Add cursor at mouse position                   |
| `Ctrl+Shift+L`        | Select all occurrences of current selection     |
| `Escape`              | Collapse to single cursor (primary)            |

**`Ctrl+D` behavior** (matches VS Code):
1. If no selection: select the word under the primary cursor.
2. If selection exists: search forward for the next occurrence of the selected text, add a new cursor+selection there.
3. If the next occurrence wraps past end-of-file, search from the beginning.
4. If all occurrences are already selected, do nothing (flash status message).

### Rendering

Each cursor renders as a blinking block. Each selection renders with `editor.selectionBackground`. The primary cursor uses the normal cursor color; secondary cursors use a dimmed variant (50% opacity approximation via theme color blending).

### Complexity: ~900 lines (refactor)

| Component                   | Lines |
| --------------------------- | ----- |
| CursorSelection refactor    | ~150  |
| Multi-cursor edit dispatch  | ~250  |
| Ctrl+D / occurrence search  | ~150  |
| Overlap merge logic         | ~100  |
| Rendering adjustments       | ~120  |
| Undo integration            | ~130  |

Note: This phase has the highest refactor cost because it changes the fundamental editing model. Every function in `editing.rs` that touches `self.cursor` must become cursor-list-aware.

---

## Phase 13: Git Gutter

Show line-level change indicators in the gutter and implement Myers diff.

### New File: `src/git.rs` (~630 lines)

```rust
/// Line-level change status for git gutter display.
#[derive(Clone, Copy, PartialEq)]
pub enum LineStatus {
    Unchanged,
    Added,
    Modified,
    Deleted,   // rendered as a small triangle on the line *after* deletion
}

/// Git integration for a single buffer.
pub struct GitInfo {
    /// The original file content from HEAD (read via `git show HEAD:<path>`).
    head_content: Option<String>,
    /// Per-line status computed by diffing head_content vs current buffer.
    line_status: Vec<LineStatus>,
    /// Path relative to repo root.
    repo_relative: Option<String>,
}
```

### Git HEAD Content

To get the committed version of a file:

```rust
impl GitInfo {
    /// Read the HEAD version of a file by shelling out to git.
    /// Returns None if not in a git repo or file is untracked.
    pub fn load_head(file_path: &Path) -> Option<String> {
        // 1. Find repo root: walk up from file_path looking for .git/
        // 2. Compute relative path
        // 3. Read .git/HEAD to find current branch ref
        // 4. Resolve ref to commit hash
        // 5. Parse git tree objects to find blob hash for the file
        // 6. Decompress and return blob content
        //
        // Alternatively (simpler first pass): read the file from the
        // working tree's last-committed state via git object parsing.
        // No shelling out to `git` — we parse .git/ directly.
    }
}
```

**Implementation choice**: Parse `.git/` objects directly (HEAD → ref → commit → tree → blob) rather than shelling out to `git`. This keeps the zero-dependency constraint and avoids the subprocess overhead. The `.git/objects/` format is well-documented: zlib-compressed content with a simple header. We use `std::io::Read` with a manual inflate implementation (~150 lines for raw DEFLATE) or read from `.git/objects/pack/` packfiles for packed repos.

Simplified approach for v1: Read loose objects only. If the blob is in a packfile (common after `git gc`), fall back to showing no gutter. Packfile parsing can be added incrementally.

### Myers Diff Algorithm

```rust
/// Compute the shortest edit script between two sequences of lines.
/// Returns a list of edit operations (Insert, Delete, Equal).
pub fn myers_diff(old: &[&str], new: &[&str]) -> Vec<DiffOp> {
    // Standard Myers algorithm (O((N+M)D) time, O(N+M) space)
    // Where N, M are line counts, D is edit distance
}

pub enum DiffOp {
    Equal(usize),      // number of equal lines
    Insert(usize),     // number of inserted lines
    Delete(usize),     // number of deleted lines
}
```

### Gutter Rendering

The gutter already exists (line numbers). Git status adds a single-character column to the left of line numbers:

```text
│+ 14│  fn new_feature() {
│~ 15│      let x = modified_line();
│  16│      unchanged();
│▸ 17│                              ← line after deleted block
```

- `+` green — added line
- `~` yellow — modified line
- `▸` red — line(s) deleted above this position
- Colored using theme keys `gitDecoration.addedResourceForeground`, `modifiedResourceForeground`, `deletedResourceForeground` (VS Code-compatible)

### Refresh Strategy

- On file open: compute diff once
- On save: recompute diff (HEAD content doesn't change until next commit, but the buffer changed)
- On buffer edit: mark diff as stale, recompute on next render if stale (debounced — only recompute if >200ms since last edit)

### Complexity: ~630 lines

| Component              | Lines |
| ---------------------- | ----- |
| Git object parsing     | ~200  |
| Myers diff             | ~130  |
| DiffOp → LineStatus    | ~60   |
| Gutter rendering       | ~80   |
| Refresh/debounce logic | ~80   |
| Editor integration     | ~80   |

---

## Phase 14: Session Restore + Swap Files

Remember open files, cursor positions, scroll state, and layout across restarts. Preserve unsaved changes via swap files so nothing is ever lost — quit without saving and resume exactly where you left off (Notepad++ model).

### Design: Two Complementary Systems

1. **Session file** — lightweight JSON with metadata (which files, cursor positions, layout). Stored centrally per project.
2. **Swap files** — full buffer content written alongside the original file. The session only records whether a swap exists; the swap itself lives next to the source file for permission/security reasons.

### New Files

| File                    | Lines | Purpose                              |
| ----------------------- | ----- | ------------------------------------ |
| `src/session.rs`        | ~350  | Session metadata save/load           |
| `src/swap.rs`           | ~300  | Swap file write/read/cleanup         |

### Swap File Convention

For every modified buffer, a hidden swap file is written in the **same directory** as the original:

```text
Original:   /home/user/project/src/main.rs
Swap:       /home/user/project/src/.main.rs.swp

Original:   /etc/nginx/nginx.conf
Swap:       /etc/nginx/.nginx.conf.swp
```

**Why same directory?**
- Inherits the same filesystem permissions — if the user can write the file, they can write the swap
- Sensitive data stays on the same volume (no leaking `/tmp` or `~/.local/state`)
- Works on network mounts, encrypted drives, and permission-restricted directories
- If the user can't write the swap (read-only dir), degrade gracefully — warn once, continue without swap

**For untitled buffers** (never saved to disk), the swap goes to a fallback location:

```text
~/.local/state/zedit/untitled/
    untitled-0.swp
    untitled-1.swp
```

### `src/swap.rs` — Swap File Management (~300 lines)

```rust
/// Swap file path for a given source file.
/// Returns `.filename.ext.swp` in the same directory.
pub fn swap_path(file_path: &Path) -> PathBuf {
    let dir = file_path.parent().unwrap_or(Path::new("."));
    let name = file_path.file_name().unwrap_or_default().to_string_lossy();
    dir.join(format!(".{}.swp", name))
}

/// Swap file path for an untitled buffer.
pub fn swap_path_untitled(index: usize) -> PathBuf {
    let dir = state_dir().join("zedit/untitled");
    dir.join(format!("untitled-{}.swp", index))
}

/// Header written at the start of every swap file for identification.
pub struct SwapHeader {
    pub magic: [u8; 4],          // b"ZSWP"
    pub version: u32,            // swap format version
    pub pid: u32,                // PID of the editor that created this swap
    pub timestamp: u64,          // Unix timestamp of last write
    pub original_path: String,   // absolute path to the original file
    pub modified: bool,          // was the buffer modified when swap was written?
}

/// Write the full buffer content to a swap file.
/// Uses write-to-temp + rename for atomicity.
pub fn write_swap(file_path: &Path, buffer: &Buffer, modified: bool) -> Result<(), String> {
    let swap = swap_path(file_path);
    let tmp = swap.with_extension("swp.tmp");

    // 1. Write header + content to temp file
    // 2. fsync() the temp file
    // 3. rename() temp → swap (atomic on POSIX)
    //
    // If rename fails (cross-device, permissions), fall back to direct write.
    // If even that fails, warn the user once and disable swap for this buffer.
}

/// Read a swap file, return the recovered buffer content.
pub fn read_swap(swap_path: &Path) -> Result<(SwapHeader, String), String> {
    // 1. Read and validate header (magic bytes, version)
    // 2. Read content after header
    // 3. Return header + content
}

/// Delete a swap file (called after explicit Ctrl+S save or Ctrl+W close).
pub fn remove_swap(file_path: &Path) {
    let swap = swap_path(file_path);
    let _ = std::fs::remove_file(swap); // ignore errors (may not exist)
}

/// Check if a swap file exists and whether it belongs to a running editor.
pub fn check_swap(file_path: &Path) -> SwapStatus {
    let swap = swap_path(file_path);
    if !swap.exists() {
        return SwapStatus::None;
    }
    match read_swap(&swap) {
        Ok((header, _)) => {
            // Check if the PID is still alive
            if process_alive(header.pid) {
                SwapStatus::OwnedByPid(header.pid)
            } else {
                SwapStatus::Orphaned(header)
            }
        }
        Err(_) => SwapStatus::Corrupt,
    }
}

pub enum SwapStatus {
    /// No swap file exists.
    None,
    /// Swap file belongs to a running editor instance.
    OwnedByPid(u32),
    /// Swap file from a crashed/killed editor — can be recovered.
    Orphaned(SwapHeader),
    /// Swap file exists but is unreadable.
    Corrupt,
}

/// Check if a PID is alive (kill(pid, 0) on POSIX).
fn process_alive(pid: u32) -> bool {
    unsafe { libc_kill(pid as i32, 0) == 0 }
}
```

### `src/session.rs` — Session Metadata (~350 lines)

```rust
/// Serializable session state (metadata only — no buffer content).
pub struct Session {
    pub version: u32,
    pub working_dir: PathBuf,
    pub buffers: Vec<BufferSession>,
    pub active_buffer: usize,
    pub layout: LayoutSession,
    pub terminals: Vec<TerminalSession>,
}

pub struct BufferSession {
    pub file_path: Option<PathBuf>,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub scroll_row: usize,
    pub scroll_col: usize,
    /// Whether a swap file existed for this buffer at session save time.
    /// On restore, the editor checks for the swap and offers recovery.
    pub has_swap: bool,
    /// True if this was an untitled buffer (content only in swap, no file_path on disk).
    pub untitled_index: Option<usize>,
}

pub struct LayoutSession {
    pub root: LayoutNodeSession,
}

pub enum LayoutNodeSession {
    Leaf { content: String },
    Split { dir: String, ratios: Vec<f32>, children: Vec<LayoutNodeSession> },
}

pub struct TerminalSession {
    pub shell: String,
    pub cwd: PathBuf,
}
```

### Storage

Session file: `~/.local/state/zedit/sessions/<hash>.json`

The `<hash>` is derived from the canonicalized working directory path, so each project gets its own session.

```rust
impl Session {
    pub fn path_for(working_dir: &Path) -> PathBuf {
        let hash = fnv1a(working_dir.to_str().unwrap().as_bytes());
        let dir = state_dir().join("zedit/sessions");
        dir.join(format!("{:016x}.json", hash))
    }

    pub fn save(&self) -> Result<(), String>;
    pub fn load(working_dir: &Path) -> Option<Session>;
}
```

### JSON Serializer

The existing `json_parser.rs` is read-only. Add a minimal `to_json()` method to `JsonValue`:

```rust
impl JsonValue {
    pub fn to_json(&self) -> String {
        match self {
            JsonValue::Null => "null".to_string(),
            JsonValue::Bool(b) => b.to_string(),
            JsonValue::Number(n) => format!("{}", n),
            JsonValue::String(s) => format!("\"{}\"", escape_json_string(s)),
            JsonValue::Array(items) => {
                let inner: Vec<String> = items.iter().map(|v| v.to_json()).collect();
                format!("[{}]", inner.join(","))
            }
            JsonValue::Object(pairs) => {
                let inner: Vec<String> = pairs.iter()
                    .map(|(k, v)| format!("\"{}\":{}", escape_json_string(k), v.to_json()))
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
        }
    }
}
```

### Lifecycle

#### Swap File Lifecycle

| Event                        | Action                                                    |
| ---------------------------- | --------------------------------------------------------- |
| First edit on buffer         | Create swap file (write header + content)                 |
| Every N edits or 2s idle     | Update swap file (debounced, atomic write-rename)         |
| `Ctrl+S` save                | Delete swap file (buffer matches disk)                    |
| `Ctrl+W` close (confirmed)  | Delete swap file                                          |
| `Ctrl+Q` quit               | Leave swap files in place (they ARE the recovery data)    |

#### Session Lifecycle

| Event                        | Action                                                    |
| ---------------------------- | --------------------------------------------------------- |
| Editor startup               | Load session → for each buffer, check swap status         |
| `Ctrl+Q` quit                | Write session JSON (records `has_swap` per buffer)        |
| `Ctrl+S` save                | Update session (debounced, 5s)                            |
| Close last buffer            | Delete session file + all associated untitled swaps       |

#### Startup Recovery Flow

```text
1. Load session.json
2. For each buffer in session:
   a. If file_path exists on disk:
      - Check for swap file (.filename.swp)
      - If swap exists AND is orphaned (PID dead):
           → Ask: "Recover unsaved changes? (Y)es / (N)o / (D)iff"
           → Y: load content from swap, mark buffer as modified
           → N: load from disk, delete swap
           → D: show diff view (Phase 19) between disk and swap, then ask
      - If swap exists AND owned by running PID:
           → Warn: "File is being edited by PID XXXX"
           → Open as read-only or skip
      - If no swap:
           → Load from disk normally
   b. If untitled (no file_path):
      - Check untitled swap in ~/.local/state/zedit/untitled/
      - If exists: load content, mark as modified
      - If missing: create empty buffer
3. Restore cursor positions, scroll, layout
4. Resume editing — no data lost
```

#### Quit Behavior (Notepad++ Model)

`Ctrl+Q` does NOT ask "unsaved changes!" anymore. Instead:

1. For every modified buffer: ensure swap file is up to date (final write)
2. Write session.json with `has_swap: true` for modified buffers
3. Exit immediately

The user's mental model: "closing the editor is like hibernating — everything comes back."

If the user explicitly wants to discard changes and close a buffer, they use `Ctrl+W` which still confirms on unsaved changes and deletes the swap on close.

### Swap File Format

Binary format for fast write (no JSON overhead for potentially large files):

```text
Offset  Size    Field
0       4       Magic: b"ZSWP"
4       4       Version: u32 (little-endian), currently 1
8       4       PID: u32 (little-endian)
12      8       Timestamp: u64 (little-endian, Unix epoch seconds)
20      4       Path length: u32 (little-endian)
24      N       Original path: UTF-8 bytes (N = path length)
24+N    1       Modified flag: 0x00 or 0x01
25+N    ...     Buffer content: raw UTF-8 text (rest of file)
```

Reading and writing is straightforward with `std::io::Read/Write` — no external deps needed.

### Edge Cases

| Scenario                          | Behavior                                          |
| --------------------------------- | ------------------------------------------------- |
| Read-only directory               | Warn once, disable swap for that buffer            |
| Disk full during swap write       | Warn, keep old swap if it exists                   |
| File deleted while editor open    | Swap still exists, user can re-save                |
| Two editors open same file        | Second editor sees swap owned by live PID → warn   |
| `git checkout` changes file       | Swap preserves user's version, recovery dialog     |
| Swap file without session         | `zedit file.txt` detects orphaned swap on open     |
| Crash during swap write           | Temp file (.swp.tmp) left behind, ignored on next open |

### Complexity: ~650 lines

| Component                | Lines |
| ------------------------ | ----- |
| `swap.rs` SwapHeader     | ~40   |
| `swap.rs` write (atomic) | ~80   |
| `swap.rs` read + validate| ~60   |
| `swap.rs` check/cleanup  | ~60   |
| `swap.rs` PID check      | ~20   |
| `swap.rs` untitled paths | ~40   |
| `session.rs` structs     | ~60   |
| `session.rs` JSON serial.| ~80   |
| `session.rs` save/load   | ~80   |
| `session.rs` path hashing| ~30   |
| Recovery dialog UI       | ~50   |
| Editor integration       | ~50   |

### Implementation Notes (DONE)

Implemented in `src/swap.rs` (357 lines) and `src/session.rs` (246 lines), totalling ~603 lines. Integration spread across `src/editor/mod.rs` (3 constructors, `run()` loop, `save_all_swaps()`, `cleanup_swap()`, `save_session()`, `check_swap_on_open()`).

**Deviations from plan:**
- `SwapStatus` uses `OwnedByUs` instead of `OwnedByPid(u32)` and `Orphaned` without carrying the full header. Simpler enum since the recovery flow doesn't need the PID externally.
- `Session` struct omits `LayoutSession` and `TerminalSession` — layout/terminal state is not persisted in this iteration (only buffer metadata: file paths, cursor positions, scroll offsets, untitled IDs).
- `BufferSession.file_path` is `Option<String>` rather than `Option<PathBuf>` to simplify JSON serialization.
- `BufferSession.scroll_col` was dropped — only `scroll_row` is persisted.
- Recovery is automatic on startup (no interactive Y/N/D dialog). Orphaned swaps are silently recovered and a message is shown (e.g. "Recovered: /path/to/file"). Corrupt swaps are deleted.
- Quit behavior still confirms on unsaved changes (`Ctrl+Q` twice). The plan's Notepad++ "always hibernates" model was relaxed — swap files are written on quit as recovery data, but the user is still warned.
- Untitled buffer swaps use `NewBufferNN.swp` naming (matching the editor's `NewBuffer01` display convention) instead of `untitled-N.swp`.
- Swap file timer in the main loop writes every 2 seconds (`swap_interval_ms: 2000`) rather than tracking edit counts.
- `json_parser.rs` gained `to_json_pretty()` for human-readable session JSON output.
- `scan_orphaned_untitled()` added to discover orphaned untitled swaps not tracked by the session file (crash recovery).
- libc bindings (`getpid`, `kill`, `time`) are declared in a local `mod libc` inside `swap.rs` — consistent with the zero-deps constraint.

**Files changed:**
- `src/swap.rs` (new) — `SwapHeader`, `SwapStatus`, `swap_path()`, `swap_path_untitled()`, `write_swap()`, `read_swap()`, `remove_swap()`, `check_swap()`, `scan_orphaned_untitled()`, PID/timestamp via libc FFI, 5 unit tests.
- `src/session.rs` (new) — `Session`, `BufferSession`, `session_path()` (FNV-1a hash), `save_session()`, `load_session()`, `delete_session()`, JSON serialization via `JsonValue`, XDG state directory, 3 unit tests.
- `src/syntax/json_parser.rs` — Added `to_json()` and `to_json_pretty()` methods to `JsonValue`.
- `src/editor/mod.rs` — `swap_timer`, `swap_interval_ms` fields. `restore_session()` constructor with swap recovery. `save_all_swaps()`, `cleanup_swap()`, `save_session()`, `check_swap_on_open()`. Periodic swap writes in `run()` loop.
- `src/main.rs` — Session restore on startup, `--restore` flag integration.

---

## Phase 15: Command Palette

Fuzzy-search any editor command. The gateway to discoverability.

### New File: `src/editor/palette.rs` (~530 lines)

```rust
/// A command that can be invoked from the palette.
pub struct PaletteCommand {
    pub id: &'static str,           // "file.save", "edit.undo"
    pub label: &'static str,        // "Save File"
    pub keybinding: Option<&'static str>,  // "Ctrl+S"
    pub category: &'static str,     // "File", "Edit", "View"
    pub action: fn(&mut Editor),    // function pointer to execute
}

/// Palette UI state.
pub struct Palette {
    pub query: String,
    pub filtered: Vec<usize>,       // indices into the global command list
    pub selected: usize,            // cursor in filtered list
    pub scroll: usize,              // scroll offset in the dropdown
}
```

### Command Registry

All editor commands are registered in a static array:

```rust
pub fn all_commands() -> &'static [PaletteCommand] {
    &[
        PaletteCommand {
            id: "file.save", label: "Save File",
            keybinding: Some("Ctrl+S"), category: "File",
            action: |ed| ed.save_current(),
        },
        PaletteCommand {
            id: "file.open", label: "Open File",
            keybinding: Some("Ctrl+O"), category: "File",
            action: |ed| ed.start_prompt(PromptKind::Open),
        },
        // ... all commands
    ]
}
```

### Fuzzy Matching

```rust
/// Score a candidate string against a query using subsequence matching.
/// Returns None if no match, Some(score) where higher is better.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    // Algorithm:
    // 1. Check if query chars appear in order in candidate (case-insensitive)
    // 2. Score bonuses:
    //    - Consecutive character matches: +5 per consecutive char
    //    - Match at word boundary (after space, _, -, uppercase): +10
    //    - Match at start of candidate: +15
    //    - Exact prefix match: +20
    // 3. Score penalties:
    //    - Gap between matches: -1 per skipped char
    //    - Total candidate length: -0.5 per char (prefer shorter matches)
}
```

### Rendering

The palette renders as a centered overlay (not a pane), similar to VS Code's `Ctrl+Shift+P`:

```text
┌─────────────────────────────────────┐
│ > save                              │
├─────────────────────────────────────┤
│   Save File                 Ctrl+S  │
│   Save As...          Ctrl+Shift+S  │
│   Session: Save                     │
└─────────────────────────────────────┘
```

- Width: 60% of screen width (min 40, max 80 columns)
- Max visible items: 10
- Matched characters highlighted in the label
- Keybinding right-aligned

### Keybindings

| Key                | Action                |
| ------------------ | --------------------- |
| `Ctrl+Shift+P`     | Open command palette  |
| Typing             | Filter commands       |
| `Up/Down`          | Navigate results      |
| `Enter`            | Execute selected      |
| `Escape`           | Close palette         |

### Complexity: ~530 lines

| Component             | Lines |
| --------------------- | ----- |
| Command registry      | ~120  |
| Fuzzy matching        | ~100  |
| Palette UI rendering  | ~130  |
| Input handling        | ~80   |
| Match highlighting    | ~50   |
| Editor integration    | ~50   |

### Implementation Notes (DONE)

Implemented in `src/editor/palette.rs` with integration into `mod.rs`, `editing.rs`, and `view.rs`.

**Deviations from plan:**
- Used an enum `PaletteAction` instead of function pointers (`fn(&mut Editor)`) for the command registry. This avoids lifetime complexity and integrates naturally with the existing `match`-based dispatch pattern used throughout the editor (prompts, keybindings).
- The `PaletteEntry` struct uses `label` + `shortcut` + `action` (enum) instead of `id` + `label` + `keybinding` + `category` + `action` (fn ptr). Categories are encoded as label prefixes (e.g. `"File: Save"`) which simplifies the data structure and works well with fuzzy matching.
- Fuzzy scoring uses a simpler greedy-forward algorithm: +1 per match, +5 consecutive bonus, +10 word boundary bonus, distance-from-start penalty. No gap penalty or length penalty — the simpler scoring produces good results for the ~35 command set.

**Files changed:**
- `src/editor/palette.rs` (new) — `PaletteAction` enum (35 commands), `PaletteEntry`, `Palette` struct, `fuzzy_score()`, `handle_palette_key()`, `execute_palette_action()`, 11 unit tests.
- `src/editor/mod.rs` — Module declaration, `palette: Option<Palette>` field, event interception, `Ctrl+Shift+P` keybinding.
- `src/editor/editing.rs` — Extracted `do_undo()` / `do_redo()` methods (previously inline in `handle_key`).
- `src/editor/view.rs` — `render_palette()` overlay (box-drawing borders, yellow highlighted matches, blue selected row, right-aligned shortcuts), palette cursor positioning.

---

## Phase 16: Soft Word Wrap

Wrap long lines visually without modifying the buffer. Essential for prose editing (Markdown, text files).

### Changes: `editor/view.rs` + `editor/buffer_state.rs` (~600 lines)

```rust
/// Wrap mode for a buffer.
#[derive(Clone, Copy, PartialEq)]
pub enum WrapMode {
    /// No wrapping — horizontal scroll for long lines (current behavior).
    None,
    /// Wrap at the pane's right edge.
    Edge,
    /// Wrap at a specific column (e.g., 80).
    Column(u16),
}

/// Mapping from visual (screen) rows to buffer positions.
/// One buffer line may produce multiple visual rows.
pub struct WrapMap {
    /// For each buffer line, the list of wrap break points (byte offsets).
    /// An unwrapped line has an empty vec.
    breaks: Vec<Vec<usize>>,
    /// Cached total visual row count.
    total_visual_rows: usize,
}
```

### Wrap Calculation

```rust
impl WrapMap {
    /// Recompute wrap breaks for a range of buffer lines.
    pub fn recompute(&mut self, buffer: &Buffer, range: Range<usize>, wrap_col: u16) {
        for line_idx in range {
            let line = buffer.line_content(line_idx);
            self.breaks[line_idx] = Self::find_breaks(line, wrap_col);
        }
        self.total_visual_rows = self.breaks.iter()
            .map(|b| b.len().max(1))
            .sum();
    }

    /// Find word-boundary break points for a single line.
    fn find_breaks(line: &str, wrap_col: u16) -> Vec<usize> {
        // Walk through the line tracking visual column width.
        // When width exceeds wrap_col, backtrack to the last
        // word boundary (space, punctuation). If no boundary
        // found (single long word), break at wrap_col exactly.
    }
}
```

### Coordinate Translation

The wrap map introduces a distinction between **buffer coordinates** (line, byte_col) and **visual coordinates** (visual_row, visual_col). Every component that converts between screen position and buffer position must go through the wrap map:

```rust
impl WrapMap {
    /// Buffer position → visual position.
    pub fn buffer_to_visual(&self, line: usize, col: usize) -> (usize, usize);

    /// Visual position → buffer position.
    pub fn visual_to_buffer(&self, visual_row: usize, visual_col: usize) -> (usize, usize);

    /// How many visual rows does buffer line `line` occupy?
    pub fn visual_rows_for(&self, line: usize) -> usize;
}
```

### Rendering Changes

Wrapped continuation lines are indented by 2 spaces and prefixed with `↪` (or a configurable indicator) in place of the line number. Only the first visual row of a buffer line shows the line number.

```text
 14│ This is a long paragraph that
   │ ↪ wraps at the edge of the
   │ ↪ pane.
 15│ Next line.
```

### Keybindings

| Key              | Action                       |
| ---------------- | ---------------------------- |
| `Alt+Z`          | Toggle word wrap             |

Wrap mode is per-buffer and can be set in config with defaults per file type (e.g., wrap by default for `.md`, `.txt`).

### Complexity: ~600 lines

| Component              | Lines |
| ---------------------- | ----- |
| WrapMap struct + logic | ~180  |
| Break point finding    | ~100  |
| Coordinate translation | ~120  |
| View rendering changes | ~100  |
| Cursor movement adj.   | ~60   |
| Editor integration     | ~40   |

---

## Phase 17: LSP Client

Language Server Protocol support for completions, diagnostics, go-to-definition, hover, and formatting.

### New Directory: `src/lsp/` (~1,650 lines)

| File                   | Lines | Purpose                          |
| ---------------------- | ----- | -------------------------------- |
| `src/lsp/mod.rs`       | ~100  | Public API, server lifecycle     |
| `src/lsp/transport.rs` | ~250  | JSON-RPC over stdio transport    |
| `src/lsp/protocol.rs`  | ~500  | LSP message types and (de)serial |
| `src/lsp/client.rs`    | ~500  | Request/response management      |
| `src/lsp/ui.rs`        | ~300  | Completion menu, diagnostics UI  |

### Transport (`lsp/transport.rs`)

LSP uses JSON-RPC 2.0 over stdin/stdout of a child process:

```rust
/// Manages communication with a language server process.
pub struct LspTransport {
    child_pid: i32,
    stdin_fd: i32,        // write requests here
    stdout_fd: i32,       // read responses here
    /// Incomplete message buffer (HTTP-style headers + JSON body).
    read_buf: Vec<u8>,
}

impl LspTransport {
    /// Spawn a language server process and set up stdio pipes.
    pub fn spawn(command: &str, args: &[&str]) -> Result<Self, String> {
        // pipe() for stdin, pipe() for stdout
        // fork() + execvp()
        // Parent: close child ends, store fds
    }

    /// Write a JSON-RPC message with Content-Length header.
    pub fn send(&mut self, msg: &JsonValue) -> Result<(), String> {
        let body = msg.to_json();
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        write_all(self.stdin_fd, header.as_bytes())?;
        write_all(self.stdin_fd, body.as_bytes())
    }

    /// Try to read a complete message (non-blocking).
    /// Returns None if no complete message is available yet.
    pub fn try_recv(&mut self) -> Result<Option<JsonValue>, String> {
        // Non-blocking read into read_buf
        // Parse "Content-Length: N\r\n\r\n" header
        // If we have N bytes of body, parse JSON and return
    }
}
```

### Protocol Types (`lsp/protocol.rs`)

Defines the subset of LSP messages we support, using our `JsonValue`:

```rust
pub struct InitializeParams { pub root_uri: String, pub capabilities: ClientCapabilities }
pub struct TextDocumentItem { pub uri: String, pub language_id: String, pub version: i32, pub text: String }
pub struct Position { pub line: u32, pub character: u32 }
pub struct Range { pub start: Position, pub end: Position }
pub struct Diagnostic { pub range: Range, pub severity: u8, pub message: String, pub source: Option<String> }
pub struct CompletionItem { pub label: String, pub kind: u8, pub detail: Option<String>, pub insert_text: Option<String> }
pub struct Location { pub uri: String, pub range: Range }

/// Build JSON-RPC request/response/notification from these types.
impl InitializeParams {
    pub fn to_json(&self) -> JsonValue;
}
// ... etc for all types
```

### Client Logic (`lsp/client.rs`)

```rust
pub struct LspClient {
    transport: LspTransport,
    next_id: i64,
    /// Pending requests awaiting responses.
    pending: Vec<(i64, PendingRequest)>,
    /// Server capabilities (from initialize response).
    capabilities: ServerCapabilities,
    /// Current diagnostics per file URI.
    diagnostics: HashMap<String, Vec<Diagnostic>>,
    /// Document version counters for synchronization.
    versions: HashMap<String, i32>,
}

enum PendingRequest {
    Completion { buffer_idx: usize },
    Definition { buffer_idx: usize },
    Hover { buffer_idx: usize },
    Format { buffer_idx: usize },
}
```

The client integrates into the main loop via `poll()` — the LSP stdout fd is added to the poll set alongside stdin and PTY fds.

### Server Configuration

Configured in `config.json`:

```json
{
    "lsp": {
        "rust": { "command": "rust-analyzer" },
        "python": { "command": "pylsp" },
        "typescript": { "command": "typescript-language-server", "args": ["--stdio"] },
        "go": { "command": "gopls" }
    }
}
```

Server is launched lazily on first file open for a supported language.

### UI Rendering (`lsp/ui.rs`)

**Completion menu**: floating box below cursor.

```text
    let x = foo.ba│
               ┌──────────────┐
               │ bar()    fn  │
               │ baz      var │
               │ batch()  fn  │
               └──────────────┘
```

**Diagnostics**: inline underline + gutter icon + status bar count.

```text
│E 14│  let x: i32 = "hello";
│                     ~~~~~~~~  ← red underline
│  Status: ✖ 2 errors, ⚠ 1 warning
```

**Hover**: tooltip overlay at cursor position.

### Keybindings

| Key              | Action                        |
| ---------------- | ----------------------------- |
| `Ctrl+Space`     | Trigger completion            |
| `F2`             | Rename symbol                 |
| `F12`            | Go to definition              |
| `Shift+F12`      | Find all references           |
| `Ctrl+Shift+F`   | Format document               |
| `Alt+.`          | Quick fix / code action       |
| `Ctrl+Shift+M`   | Show diagnostics panel        |

### Document Sync

Use `textDocument/didOpen`, `textDocument/didChange` (incremental), `textDocument/didSave`, `textDocument/didClose`. Send changes on every edit (debounced by 100ms to batch rapid keystrokes).

### Complexity: ~1,650 lines

| Component              | Lines |
| ---------------------- | ----- |
| Transport (JSON-RPC)   | ~250  |
| Protocol types         | ~500  |
| Client lifecycle       | ~500  |
| Completion UI          | ~150  |
| Diagnostics UI         | ~100  |
| Hover/definition UI    | ~50   |
| Editor integration     | ~100  |

---

## Phase 18: Plugin System (Minilux Scripting)

Extend the editor with Minilux scripts. This connects Zedit to the broader Z ecosystem.

### New Directory: `src/plugin/` (~700+ lines)

| File                    | Lines | Purpose                           |
| ----------------------- | ----- | --------------------------------- |
| `src/plugin/mod.rs`     | ~100  | Plugin discovery and lifecycle    |
| `src/plugin/api.rs`     | ~300  | Editor API exposed to plugins     |
| `src/plugin/bridge.rs`  | ~300  | Minilux ↔ Zedit IPC protocol     |

### Architecture

Plugins run as **separate Minilux processes** communicating with Zedit over stdin/stdout using a simple JSON-based IPC protocol (same pattern as LSP). This keeps the editor stable even if a plugin crashes.

```rust
/// A loaded plugin.
pub struct Plugin {
    pub name: String,
    pub version: String,
    pub description: String,
    /// The running Minilux process.
    child_pid: i32,
    stdin_fd: i32,
    stdout_fd: i32,
    /// Commands this plugin registered.
    pub commands: Vec<PluginCommand>,
    /// Events this plugin subscribes to.
    pub subscriptions: Vec<EventKind>,
}

pub struct PluginCommand {
    pub id: String,            // "myplugin.format_table"
    pub label: String,         // "Format Markdown Table"
    pub keybinding: Option<String>,
}

pub enum EventKind {
    BufferOpen,
    BufferSave,
    BufferClose,
    CursorMove,
    TextChange,
}
```

### Editor API (`plugin/api.rs`)

The API exposed to plugins via IPC:

```rust
/// Messages the editor can receive from a plugin.
pub enum PluginRequest {
    // Buffer operations
    GetBufferText { buffer_id: usize },
    GetSelection,
    InsertText { pos: Position, text: String },
    ReplaceRange { range: Range, text: String },
    SetSelection { anchor: Position, head: Position },

    // UI operations
    ShowMessage { text: String, kind: String },
    ShowInputPrompt { prompt: String, callback_id: u64 },

    // Editor state
    GetConfig { key: String },
    GetFilePath,
    GetCursorPosition,

    // Registration
    RegisterCommand { id: String, label: String, keybinding: Option<String> },
    SubscribeEvent { event: EventKind },
}

/// Messages the editor sends to a plugin.
pub enum PluginNotification {
    Event { kind: EventKind, data: JsonValue },
    CommandInvoked { command_id: String },
    InputResult { callback_id: u64, value: Option<String> },
}
```

### Plugin Discovery

Plugins live in `~/.config/zedit/plugins/`:

```text
~/.config/zedit/plugins/
  table-formatter/
    manifest.json        # name, version, description, main entrypoint
    main.mlx             # Minilux script
  git-blame/
    manifest.json
    main.mlx
```

On startup, scan the plugins directory, parse each `manifest.json`, but don't launch processes until the plugin is activated (lazy loading).

### Complexity: ~700+ lines

| Component              | Lines |
| ---------------------- | ----- |
| Plugin discovery       | ~80   |
| Plugin lifecycle       | ~120  |
| API message types      | ~120  |
| IPC bridge             | ~180  |
| Command palette integ. | ~80   |
| Event dispatch         | ~80   |
| Editor integration     | ~40   |

Note: The actual Minilux runtime is external — this is only the bridge/host side.

---

## Phase 19: Diff / Merge View

Side-by-side file comparison with optional three-way merge support for conflict resolution.

### New File: `src/diff_view.rs` (~650 lines)

```rust
/// Side-by-side diff view state.
pub struct DiffView {
    pub left: DiffBuffer,
    pub right: DiffBuffer,
    /// Aligned line pairs (Some(n), Some(m)) = matched, None = gap.
    pub alignment: Vec<(Option<usize>, Option<usize>)>,
    /// Which hunk the cursor is currently in.
    pub current_hunk: usize,
    pub hunks: Vec<Hunk>,
    /// Synchronized scroll offset.
    pub scroll: usize,
}

pub struct DiffBuffer {
    pub content: Vec<String>,     // lines
    pub path: PathBuf,
    pub label: String,            // e.g., "HEAD", "working tree"
}

pub struct Hunk {
    pub left_start: usize,
    pub left_count: usize,
    pub right_start: usize,
    pub right_count: usize,
    pub kind: HunkKind,
}

pub enum HunkKind {
    Added,      // only in right
    Deleted,    // only in left
    Modified,   // different in both
}
```

### Rendering

The diff view uses the layout system (Phase 8) to create a locked two-pane split:

```text
 HEAD                          │ Working Tree
─────────────────────────────────────────────────────
  1│ fn main() {               │  1│ fn main() {
  2│     println!("hello");    │  2│     println!("hello");
   │                           │  3│+    println!("world");
  3│ }                         │  4│ }
```

- Added lines: green background on right side, blank on left
- Deleted lines: red background on left side, blank on right
- Modified lines: yellow background on both sides with inline char-level diff highlighting
- Scroll is synchronized between both panes

### Inline Character Diff

For modified lines, highlight the specific changed characters:

```rust
/// Find character-level differences within two lines.
/// Returns ranges of changed characters in each line.
pub fn inline_diff(left: &str, right: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    // Use longest common subsequence on characters
    // Mark non-matching ranges
}
```

### Navigation

| Key              | Action                         |
| ---------------- | ------------------------------ |
| `F7`             | Open diff view (current file vs HEAD) |
| `]c` / `[c`     | Next / previous hunk           |
| `Ctrl+Shift+D`  | Open diff between two files (prompted) |
| `Escape`         | Close diff view                |

### Complexity: ~650 lines

| Component              | Lines |
| ---------------------- | ----- |
| DiffView struct + mgmt | ~80   |
| Line alignment         | ~120  |
| Hunk detection         | ~80   |
| Inline char diff       | ~100  |
| Rendering (both panes) | ~150  |
| Navigation + scroll    | ~70   |
| Editor integration     | ~50   |

Note: Reuses the Myers diff algorithm from Phase 13 (`git.rs`).

---

## Phase 20: Minimap

Code overview sidebar showing a zoomed-out view of the file.

### New File: `src/editor/minimap.rs` (~330 lines)

```rust
/// Minimap state for a buffer.
pub struct Minimap {
    /// Rendered minimap content (each "pixel" = one character in the source).
    /// Using braille characters (⠁⠂⠃...) to pack 2x4 pixels per cell.
    pixels: Vec<Vec<MinimapCell>>,
    /// Width of the minimap in terminal columns.
    width: u16,
    /// Visible viewport indicator (start_row, end_row in minimap coords).
    viewport_start: usize,
    viewport_end: usize,
    /// Whether the minimap is enabled.
    pub visible: bool,
}

struct MinimapCell {
    braille: char,       // Unicode braille pattern
    fg: Color,           // dominant syntax color for this region
    bg: Color,           // background
}
```

### Rendering Strategy

Each minimap column represents ~2 source characters. Each minimap row represents ~4 source lines (using Unicode Braille patterns U+2800–U+28FF which encode a 2x4 dot matrix in a single character).

```text
Source code                    │ Minimap
───────────────────────────────│─────
fn main() {                    │ ⣿⣷⡇
    let x = 42;               │ ⠀⣿⣿  ← viewport highlighted
    if x > 0 {                │ ⠀⣷⣿
        println!("yes");      │ ⠀⠀⣿
    }                          │ ⠀⣷⡇
}                              │ ⣿⡇⠀
```

The minimap occupies 8-12 columns on the right edge of the editor pane. The current viewport is shown as a semi-transparent overlay (using a lighter background color).

### Braille Encoding

```rust
/// Encode a 2x4 pixel grid into a single braille character.
/// Each pixel is a bool (true = dot, false = blank).
fn encode_braille(dots: [[bool; 2]; 4]) -> char {
    // Unicode braille: U+2800 + bit pattern
    // Bit layout:
    //   0 3
    //   1 4
    //   2 5
    //   6 7
    let mut code: u32 = 0x2800;
    if dots[0][0] { code |= 0x01; }
    if dots[1][0] { code |= 0x02; }
    if dots[2][0] { code |= 0x04; }
    if dots[0][1] { code |= 0x08; }
    if dots[1][1] { code |= 0x10; }
    if dots[2][1] { code |= 0x20; }
    if dots[3][0] { code |= 0x40; }
    if dots[3][1] { code |= 0x80; }
    char::from_u32(code).unwrap()
}
```

### Color Mapping

Each minimap cell's foreground color is the dominant syntax color for the source characters it represents. "Dominant" = the color that covers the most characters in that 2x4 block. This reuses the existing syntax highlighting data.

### Interaction

| Key              | Action                |
| ---------------- | --------------------- |
| `Ctrl+Shift+M`   | Toggle minimap        |
| Mouse click      | Jump to that position |
| Mouse drag       | Scroll by dragging    |

### Complexity: ~330 lines

| Component              | Lines |
| ---------------------- | ----- |
| Minimap struct         | ~40   |
| Braille encoding       | ~50   |
| Source → pixel mapping | ~80   |
| Color aggregation      | ~50   |
| Rendering              | ~60   |
| Mouse interaction      | ~30   |
| Editor integration     | ~20   |

---

## Summary Table

| Phase | Feature              | New Files                          | Lines  | Depends On     |
| ----- | -------------------- | ---------------------------------- | ------ | -------------- |
| 8     | Layout & Pane System | `layout.rs`                        | ~600   | —              |
| 9     | Integrated Terminal  | `pty.rs`, `vterm.rs`               | ~1,450 | Phase 8        |
| 10    | Tab Bar              | `editor/tabs.rs`                   | ~250   | —              |
| 11    | File Tree Sidebar    | `filetree.rs`                      | ~750   | Phase 8        |
| 12    | Multi-Cursor Editing | *(refactor existing)*              | ~900   | —              |
| 13    | Git Gutter           | `git.rs`                           | ~630   | —              |
| 14    | Session + Swap Files | `session.rs`, `swap.rs`            | ~650   | Phase 8        |
| 15    | Command Palette      | `editor/palette.rs`                | ~530   | —              |
| 16    | Soft Word Wrap       | *(modify view.rs, buffer_state)*   | ~600   | —              |
| 17    | LSP Client           | `lsp/mod.rs`, `transport.rs`, etc. | ~1,650 | Phase 9 (poll) |
| 18    | Plugin System        | `plugin/mod.rs`, `api.rs`, etc.    | ~700   | Phase 15       |
| 19    | Diff/Merge View      | `diff_view.rs`                     | ~650   | Phase 8, 13    |
| 20    | Minimap              | `editor/minimap.rs`                | ~330   | Phase 8        |
|       | **TOTAL**            |                                    |**~9,690**|              |

---

## Implementation Order

Dependency graph showing which phases can be parallelized and which must be sequential:

```text
Phase 8 (Layout) ──┬── Phase 9 (Terminal) ──── Phase 17 (LSP)*
                    ├── Phase 11 (File Tree)
                    ├── Phase 14 (Session)
                    ├── Phase 19 (Diff View)**
                    └── Phase 20 (Minimap)

Phase 10 (Tabs) ──── standalone

Phase 12 (Multi-Cursor) ──── standalone

Phase 13 (Git Gutter) ──── Phase 19 (Diff View)**

Phase 15 (Command Palette) ──── Phase 18 (Plugins)

Phase 16 (Soft Wrap) ──── standalone

*  Phase 17 reuses the poll() integration from Phase 9
** Phase 19 reuses Myers diff from Phase 13
```

### Recommended Order

1. **Phase 8** — Layout (foundation for 5 other phases)
2. **Phase 10** — Tabs (quick win, no dependencies)
3. **Phase 12** — Multi-Cursor (high user value, independent)
4. **Phase 13** — Git Gutter (independent, enables Phase 19)
5. **Phase 15** — Command Palette (independent, enables Phase 18)
6. **Phase 16** — Soft Word Wrap (independent, high prose value)
7. **Phase 11** — File Tree (needs Phase 8)
8. **Phase 9** — Integrated Terminal (needs Phase 8, enables Phase 17)
9. **Phase 14** — Session + Swap Files (needs Phase 8)
10. **Phase 17** — LSP Client (needs poll from Phase 9)
11. **Phase 19** — Diff View (needs Phase 8 + 13)
12. **Phase 20** — Minimap (needs Phase 8, lower priority)
13. **Phase 18** — Plugin System (needs Phase 15, depends on external Minilux)

---

## Timeline Estimate

| Phase    | Feature              | Estimate   |
| -------- | -------------------- | ---------- |
| Phase 8  | Layout & Pane System | ~2 weeks   |
| Phase 9  | Integrated Terminal  | ~4 weeks   |
| Phase 10 | Tab Bar              | ~1 week    |
| Phase 11 | File Tree Sidebar    | ~2 weeks   |
| Phase 12 | Multi-Cursor Editing | ~3 weeks   |
| Phase 13 | Git Gutter           | ~2 weeks   |
| Phase 14 | Session + Swap Files | ~2 weeks   |
| Phase 15 | Command Palette      | ~2 weeks   |
| Phase 16 | Soft Word Wrap       | ~2 weeks   |
| Phase 17 | LSP Client           | ~4 weeks   |
| Phase 18 | Plugin System        | ~2 weeks   |
| Phase 19 | Diff/Merge View      | ~2 weeks   |
| Phase 20 | Minimap              | ~1 week    |
| **Total**|                      | **~29 weeks** |

Some phases can overlap (see dependency graph above). With parallel work on independent phases, the critical path is approximately:

```text
Phase 8 (2w) → Phase 9 (4w) → Phase 17 (4w) = 10 weeks critical path
```

Total wall-clock time with parallelization: **~16-20 weeks**.

---

## Keybinding Summary (New in Phase 2)

All new keybindings checked against existing MVP bindings for conflicts:

| Key                  | Phase | Action                     |
| -------------------- | ----- | -------------------------- |
| `Ctrl+\`             | 8     | Split horizontal           |
| `Ctrl+Shift+\`       | 8     | Split vertical             |
| `Ctrl+Shift+W`       | 8     | Close pane                 |
| `Alt+Arrow`          | 8     | Focus adjacent pane        |
| `Alt+Shift+Arrow`    | 8     | Resize pane                |
| `` Ctrl+` ``         | 9     | Toggle terminal panel      |
| `Ctrl+Shift+T`       | 9     | New terminal instance      |
| `Ctrl+B`             | 11    | Toggle file tree           |
| `Ctrl+D`             | 12    | Select next occurrence (*) |
| `Ctrl+Shift+D`       | 12    | Skip, select next          |
| `Alt+Click`          | 12    | Add cursor at click        |
| `Ctrl+Shift+L`       | 12    | Select all occurrences     |
| `Ctrl+Shift+P`       | 15    | Command palette            |
| `Alt+Z`              | 16    | Toggle word wrap           |
| `Ctrl+Space`         | 17    | Trigger completion         |
| `F2`                 | 17    | Rename symbol              |
| `F12`                | 17    | Go to definition           |
| `Shift+F12`          | 17    | Find all references        |
| `Ctrl+Shift+F`       | 17    | Format document            |
| `Alt+.`              | 17    | Quick fix / code action    |
| `Ctrl+Shift+M`       | 17/20 | Diagnostics / toggle minimap |
| `F7`                 | 19    | Diff view (file vs HEAD)   |
| `]c` / `[c`          | 19    | Next / prev diff hunk      |
| `Ctrl+Shift+D`       | 19    | Diff two files (prompted)  |

(*) `Ctrl+D` is currently "duplicate line" in the MVP. Phase 12 reassigns it to "select next occurrence" (matching VS Code). Duplicate line moves to `Ctrl+Shift+D` in non-multi-cursor context, or is accessible via the command palette.

**Conflict resolution for `Ctrl+Shift+M`**: Used by both Phase 17 (diagnostics panel) and Phase 20 (minimap toggle). Resolution: `Ctrl+Shift+M` opens diagnostics. Minimap toggle is available via command palette or can be assigned a custom keybinding in config.

**Conflict resolution for `Ctrl+Shift+D`**: Used by both Phase 12 (skip occurrence) and Phase 19 (diff two files). Resolution: `Ctrl+Shift+D` behavior is context-dependent — in multi-cursor mode it skips occurrence, otherwise it opens the diff file prompt. Both are accessible via command palette regardless of context.

---

## Architecture After Phase 20

```text
src/
  main.rs                  Entry point, CLI args
  terminal.rs              Raw mode FFI, SIGWINCH
  input.rs                 Key/mouse/paste decoding
  buffer.rs                Gap buffer text storage
  cursor.rs                Cursor + multi-cursor
  render.rs                Diff-based screen rendering
  undo.rs                  Undo/redo with grouping
  config.rs                Runtime configuration
  unicode.rs               Unicode width utilities
  layout.rs                Recursive pane layout          [Phase 8]
  pty.rs                   PTY allocation + child mgmt    [Phase 9]
  vterm.rs                 VT100 terminal emulator        [Phase 9]
  filetree.rs              File tree sidebar               [Phase 11]
  git.rs                   Git gutter + Myers diff         [Phase 13]
  session.rs               Session metadata save/restore   [Phase 14]
  swap.rs                  Swap file write/read/cleanup    [Phase 14]
  diff_view.rs             Side-by-side diff               [Phase 19]
  editor/
    mod.rs                 Editor struct, main loop
    buffer_state.rs        Per-buffer state
    editing.rs             Insert, delete, indent
    selection.rs           Selection, clipboard
    search.rs              Find/replace
    view.rs                Viewport, rendering, wrap       [Phase 16]
    prompt.rs              Mini-prompts
    helpers.rs             Utilities
    tabs.rs                Tab bar                         [Phase 10]
    palette.rs             Command palette                 [Phase 15]
    minimap.rs             Code overview                   [Phase 20]
    tests.rs               Tests
  syntax/
    mod.rs                 Syntax highlighting API
    json_parser.rs         JSON parser (+ serializer)      [Phase 14]
    regex.rs               Regex engine
    grammar.rs             TextMate grammar types
    tokenizer.rs           Line tokenizer
    theme.rs               Theme loader
    highlight.rs           Highlighter
  lsp/
    mod.rs                 LSP public API                  [Phase 17]
    transport.rs           JSON-RPC over stdio             [Phase 17]
    protocol.rs            LSP message types               [Phase 17]
    client.rs              Request/response mgmt           [Phase 17]
    ui.rs                  Completion/diagnostics UI       [Phase 17]
  plugin/
    mod.rs                 Plugin discovery                [Phase 18]
    api.rs                 Editor API for plugins          [Phase 18]
    bridge.rs              Minilux IPC bridge              [Phase 18]
```

**Estimated total**: ~19,940 lines (10,250 MVP + 9,690 Phase 2).

Still zero external dependencies. Still a single static binary. Still starts in under 10ms.
