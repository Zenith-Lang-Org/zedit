# Zedit Phase 3 — Execution Studio

From IDE to execution environment. Native extension system, task runner, problem panel,
and deep integration with Zenith and Zymbol.

---

## Architectural Decision: Extension Format

### The Question

Should `zedit --import rust` use VS Code `.vsix` files, or create a native zedit format?

### Analysis

| Criterion | VS Code .vsix (pure) | Native zedit format |
|-----------|---------------------|---------------------|
| Ecosystem | 50,000+ extensions immediately | Zero on day one |
| Complexity | ZIP parser + huge package.json | Simple JSON manifest |
| Dependency | Tied to VS Code lifecycle | Full control |
| File size | 5–200MB (includes Node.js code) | 50–500KB (only what we use) |
| Features used | <5% of what's in a .vsix | 100% |
| Zedit-specific | Cannot add run/build/repl config | First-class citizen |

### Decision: Hybrid Architecture

**Native zedit extension format** is the canonical standard.
**VS Code `.vsix` import** is a compatibility conversion tool — it reads .vsix and converts
to native format automatically.

```
zedit --import rust          # downloads .vsix from Marketplace → converts to native
zedit --import ./my.vsix     # convert local .vsix file to native
zedit --import https://...   # convert from URL
zedit --ext install ./ruby/  # install a native extension directory
zedit --ext list             # show all installed
zedit --ext remove ruby      # uninstall
```

This means:
- Users can tap the entire VS Code grammar/theme ecosystem via import
- But the installed result is always a clean, minimal, zedit-native extension
- The zedit format is open and can be published independently of VS Code Marketplace
- Zedit-specific features (tasks, REPL config, custom keybindings) live in the manifest

---

## Native Extension Format

An extension is a **directory** under `~/.config/zedit/extensions/<id>/`:

```
~/.config/zedit/extensions/
  rust/
    manifest.json
    grammar.tmLanguage.json         ← optional, overrides built-in
    theme-dark.json                 ← optional
  zenith/
    manifest.json
    grammar.tmLanguage.json
  ruby/
    manifest.json
    ruby.tmLanguage.json
```

### manifest.json schema

```json
{
  "id":          "rust",
  "name":        "Rust Language Support",
  "version":     "1.0.0",
  "description": "Full Rust support: syntax, LSP, tasks",

  "languages": [
    {
      "id":         "rust",
      "extensions": [".rs"],
      "aliases":    ["Rust", "rs"],
      "comment":    "//"
    }
  ],

  "grammars": [
    {
      "language":  "rust",
      "scopeName": "source.rust",
      "path":      "grammar.tmLanguage.json"
    }
  ],

  "themes": [
    {
      "id":    "rust-dark",
      "label": "Rust Dark",
      "path":  "theme-dark.json"
    }
  ],

  "lsp": {
    "command": "rust-analyzer",
    "args":    [],
    "env":     {}
  },

  "tasks": {
    "run":   { "cmd": "cargo run",   "cwd": "{workspace}" },
    "build": { "cmd": "cargo build", "cwd": "{workspace}" },
    "test":  { "cmd": "cargo test",  "cwd": "{workspace}" },
    "check": { "cmd": "cargo check", "cwd": "{workspace}" }
  }
}
```

Available template variables: `{file}`, `{dir}`, `{workspace}`, `{stem}` (filename without extension).

---

## Implementation Roadmap

| # | Phase | Feature | ~Lines | Dependencies | Status |
|---|-------|---------|--------|--------------|--------|
| 1 | 7δ-A | Runtime Grammar Loading | 350 | — | DONE |
| 2 | 7δ-B | Native Extension System | 600 | 7δ-A | DONE |
| 3 | 7δ-C | VS Code .vsix Importer | 450 | 7δ-B | DONE |
| 4 | 21 | Task Runner (F5) | 500 | 7δ-B | DONE |
| 5 | 22 | Problem Panel | 650 | Phase 21 | TODO |
| 6 | 23 | Zenith + Zymbol Integration | 400 | 7δ-B | TODO |

Total: ~2,950 new lines across 6 phases.

---

## Phase 7δ-A: Runtime Grammar Loading

**Goal**: Remove all `include_str!()` grammars from the compiled binary.
Binary goes from ~2.5MB → ~600KB. Language support becomes fully runtime-configurable.

### Problem

`build.rs` currently embeds 22 grammars + languages.json directly into the binary:
```rust
// build.rs generates this at compile time:
pub const EMBEDDED_RUST_GRAMMAR: &str = include_str!("../../grammars/rust.tmLanguage.json");
pub const EMBEDDED_LANGUAGES_JSON: &str = include_str!("../../grammars/languages.json");
```

Result: 1.9MB of JSON baked into binary. Adding a language requires `cargo build`.

### Target Architecture

Grammar search path (highest priority first):

```
1. ~/.config/zedit/extensions/<id>/grammar.tmLanguage.json  (extensions)
2. ~/.config/zedit/grammars/<file>                          (legacy user override)
3. /usr/share/zedit/grammars/<file>                        (system install)
4. /usr/local/share/zedit/grammars/<file>                  (manual system install)
5. ./grammars/<file>                                        (dev mode, cwd-relative)
```

Language definitions (`languages.json`) search path:
```
1. ~/.config/zedit/extensions/*/manifest.json   (aggregate from all extensions)
2. ~/.config/zedit/languages.json               (user override)
3. /usr/share/zedit/languages.json
4. ./grammars/languages.json                    (dev mode fallback)
```

### Files Modified

**`build.rs`**:
- Remove all `include_str!()` calls for grammars
- Keep only a minimal fallback: plain text (no grammar) embedded as constant
- Remove generated `embedded_grammars.rs`

**`src/config.rs`**:
```rust
/// Load language definitions from all sources, merged by priority.
pub fn load_languages() -> Vec<LanguageDef> {
    let mut result = Vec::new();

    // 1. Extensions manifests
    result.extend(load_extension_languages());

    // 2. User languages.json
    if let Some(user) = load_user_languages_json() {
        result.extend(user);
    }

    // 3. System-wide
    result.extend(load_system_languages());

    // 4. Dev mode fallback (./grammars/languages.json)
    if result.is_empty() {
        result.extend(load_dev_languages());
    }

    deduplicate_languages(result)
}
```

**`src/syntax/highlight.rs`** (or wherever `load_grammar` lives):
```rust
pub fn load_grammar(grammar_file: &str) -> Option<Grammar> {
    // Search path: extensions → user → system → dev
    for dir in grammar_search_dirs() {
        let path = dir.join(grammar_file);
        if path.exists() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                return Grammar::from_json(&text).ok();
            }
        }
    }
    None  // plain text mode
}

fn grammar_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // extensions
    if let Some(ext_dir) = extension_base_dir() {
        for entry in std::fs::read_dir(&ext_dir).into_iter().flatten().flatten() {
            dirs.push(entry.path());
        }
    }
    // ~/.config/zedit/grammars/
    if let Some(home) = home_dir() {
        dirs.push(home.join(".config/zedit/grammars"));
    }
    // system paths
    dirs.push(PathBuf::from("/usr/share/zedit/grammars"));
    dirs.push(PathBuf::from("/usr/local/share/zedit/grammars"));
    // dev mode
    dirs.push(PathBuf::from("grammars"));
    dirs
}
```

**`src/main.rs`**:
- Remove reference to `EMBEDDED_LANGUAGES_JSON`
- Call `config::load_languages()` instead of `config::builtin_languages()`

**Migration**: Ship the built `grammars/` directory alongside the binary in releases.
In dev mode, `./grammars/` is on the search path so it just works.

### Unit Tests

- Search path priority: extension grammar overrides system grammar
- Missing grammar → returns None (plain text mode, no crash)
- languages.json merge: user entry overrides builtin by language id

---

## Phase 7δ-B: Native Extension System

**Goal**: `zedit --ext` CLI for managing extensions. Extension directory loading at startup.

### New File: `src/extension.rs`

```rust
pub struct Extension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub languages: Vec<LanguageDef>,
    pub lsp: Option<ExtLspConfig>,
    pub tasks: HashMap<String, TaskDef>,  // "run", "build", "test"…
}

pub struct ExtLspConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub struct TaskDef {
    pub cmd: String,
    pub cwd: String,  // template: {workspace}, {dir}, {file}
}

/// Load all installed extensions from ~/.config/zedit/extensions/
pub fn load_extensions() -> Vec<Extension> { ... }

/// Load a single extension from a directory.
pub fn load_extension_dir(path: &Path) -> Result<Extension, String> {
    let manifest_path = path.join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("cannot read manifest.json: {}", e))?;
    parse_manifest(&text)
}

fn parse_manifest(json: &str) -> Result<Extension, String> { ... }

/// Extension base directory: ~/.config/zedit/extensions/
pub fn extension_base_dir() -> Option<PathBuf> { ... }

/// Install an extension directory into the extensions base dir.
/// Returns the installed extension id.
pub fn install_extension(src: &Path) -> Result<String, String> { ... }

/// Uninstall an extension by id.
pub fn uninstall_extension(id: &str) -> Result<(), String> { ... }

/// List installed extensions: (id, name, version).
pub fn list_extensions() -> Vec<(String, String, String)> { ... }
```

### CLI Commands (added to `src/main.rs`)

```
zedit --ext list
zedit --ext install <path>
zedit --ext remove <id>
zedit --ext info <id>
```

Parse args before entering editor mode. If `--ext` arg present, execute and exit.

### Integration with Config + LSP

At startup, `Config::load()` also calls `extension::load_extensions()` and:
1. Merges extension language definitions into the language table
2. Merges extension LSP configs into the `lsp` config section
3. Makes extension tasks available to the Task Runner (Phase 21)

### `config.json` manual override still works

User's `~/.config/zedit/config.json` can override any extension setting, because
config loading order is: extensions → config.json (config.json wins).

---

## Phase 7δ-C: VS Code .vsix Importer

**Goal**: `zedit --import <name>` converts any VS Code extension to native zedit format.

### What a .vsix file is

A `.vsix` file is a renamed ZIP. Inside:
```
extension/
  package.json          ← manifest (contributes.grammars, contributes.languages, etc.)
  syntaxes/
    *.tmLanguage.json   ← grammar files (what we want)
  themes/
    *.json              ← color themes (optional)
  LICENSE
[Content_Types].xml
```

### Import Sources

```sh
zedit --import rust                    # search VS Code Marketplace by name (uses curl)
zedit --import ./downloaded.vsix       # convert local file (uses unzip)
zedit --import https://example.com/x.vsix  # download and convert (uses curl + unzip)
```

The importer shells out to `curl` and `unzip` (both universally available on Linux/macOS).
No Rust ZIP library needed — zero new dependencies.

### New File: `src/vsix_import.rs`

```rust
/// Download and install a VS Code extension by name (queries Open VSX or VS Code Marketplace).
pub fn import_by_name(name: &str) -> Result<String, String> {
    let url = marketplace_download_url(name)?;
    let vsix_path = download_vsix(&url)?;
    let ext_id = import_vsix(&vsix_path)?;
    let _ = std::fs::remove_file(&vsix_path);
    Ok(ext_id)
}

/// Convert a local .vsix file to a native zedit extension.
/// Returns the installed extension id.
pub fn import_vsix(vsix_path: &Path) -> Result<String, String> {
    // 1. Extract to temp dir via: unzip <vsix_path> -d <tmpdir>
    let tmp = extract_vsix(vsix_path)?;

    // 2. Parse extension/package.json
    let pkg = parse_package_json(&tmp.join("extension/package.json"))?;

    // 3. Build zedit manifest from contributes.grammars + contributes.languages
    let manifest = convert_package_json(&pkg)?;

    // 4. Copy grammar files
    let ext_id = manifest.id.clone();
    let dest = extension_base_dir()?.join(&ext_id);
    std::fs::create_dir_all(&dest)?;
    install_grammars(&tmp, &pkg, &dest)?;
    install_themes(&tmp, &pkg, &dest)?;

    // 5. Write manifest.json
    write_manifest(&dest.join("manifest.json"), &manifest)?;

    // 6. Clean up temp dir
    let _ = std::fs::remove_dir_all(&tmp);

    Ok(ext_id)
}

/// Query VS Code Marketplace for download URL.
fn marketplace_download_url(name: &str) -> Result<String, String> {
    // Uses Open VSX Registry (open-source, no ToS issues):
    // https://open-vsx.org/api/<publisher>/<name>/latest
    // Falls back to VS Code Marketplace:
    // https://marketplace.visualstudio.com/_apis/public/gallery/publishers/...
    ...
}

fn extract_vsix(vsix_path: &Path) -> Result<PathBuf, String> {
    let tmp = std::env::temp_dir().join("zedit-vsix-import");
    std::fs::create_dir_all(&tmp)?;
    // shell out: unzip -o <vsix> -d <tmp>
    let status = std::process::Command::new("unzip")
        .args(["-o", vsix_path.to_str().unwrap(), "-d", tmp.to_str().unwrap()])
        .status()
        .map_err(|e| format!("unzip failed: {}", e))?;
    if !status.success() {
        return Err("unzip exited with error".into());
    }
    Ok(tmp)
}

struct PackageJson {
    publisher: String,
    name: String,
    display_name: String,
    version: String,
    grammars: Vec<PkgGrammar>,
    languages: Vec<PkgLanguage>,
    themes: Vec<PkgTheme>,
}

struct PkgGrammar { language: String, scope_name: String, path: String }
struct PkgLanguage { id: String, extensions: Vec<String>, aliases: Vec<String> }
struct PkgTheme { id: String, label: String, path: String, ui_theme: String }
```

### Mapping: package.json → manifest.json

| package.json | manifest.json |
|---|---|
| `publisher.name` | `id` |
| `displayName` | `name` |
| `version` | `version` |
| `contributes.languages[].id` | `languages[].id` |
| `contributes.languages[].extensions` | `languages[].extensions` |
| `contributes.languages[].aliases[0]` | `languages[].aliases` |
| `contributes.grammars[].language` | `grammars[].language` |
| `contributes.grammars[].path` | `grammars[].path` (copied) |
| `contributes.themes[].path` | `themes[].path` (copied) |

LSP and tasks are **not** in package.json (VS Code uses activation events + Node.js).
After import, the user can manually add `lsp` and `tasks` to `manifest.json`.

---

## Phase 21: Task Runner

**Goal**: Press F5 to run the current file with the right command. No manual terminal typing.

### Keybindings

| Key | Action | Example |
|-----|--------|---------|
| `F5` | Run | `cargo run` / `python3 {file}` / `zenith {file}` |
| `Ctrl+F5` | Build (no run) | `cargo build` / `python3 -c "compile()"` |
| `Shift+F5` | Test | `cargo test` / `pytest {file}` |
| `Alt+F5` | Stop running task | kills the process |

### Task Resolution

When F5 is pressed on `src/main.rs` (language=rust):

1. Check `config.json → tasks.rust.run` (user override)
2. Check installed extensions for language=rust → `manifest.json → tasks.run`
3. Fall back to built-in defaults table

Built-in defaults:

```rust
fn builtin_task(lang: &str, kind: TaskKind) -> Option<&'static str> {
    match (lang, kind) {
        ("rust",       Run)   => Some("cargo run"),
        ("rust",       Build) => Some("cargo build"),
        ("rust",       Test)  => Some("cargo test"),
        ("python",     Run)   => Some("python3 {file}"),
        ("javascript", Run)   => Some("node {file}"),
        ("typescript", Run)   => Some("ts-node {file}"),
        ("zenith",     Run)   => Some("zenith {file}"),
        ("zymbol",     Run)   => Some("zymbol {file}"),
        ("shell",      Run)   => Some("bash {file}"),
        ("go",         Run)   => Some("go run {file}"),
        ("java",       Build) => Some("javac {file}"),
        ("java",       Run)   => Some("java {stem}"),
        _ => None,
    }
}
```

### Template Variable Expansion

```rust
fn expand_task_cmd(cmd: &str, file_path: &str, workspace: &str) -> String {
    let path = Path::new(file_path);
    cmd.replace("{file}",      file_path)
       .replace("{dir}",       path.parent().map(|p| p.to_str().unwrap_or("")).unwrap_or(""))
       .replace("{stem}",      path.file_stem().and_then(|s| s.to_str()).unwrap_or(""))
       .replace("{workspace}", workspace)
}
```

### Execution: Sends Command to Integrated Terminal

Tasks run in the **existing integrated terminal** (Phase 9). F5 does:

1. If no terminal pane is open → open one (same as `Ctrl+T`)
2. Focus the terminal pane
3. Send: `<expanded_command>\n` to the PTY

This means the task output appears in the terminal with full color, interactive input,
and the user can Ctrl+C to stop. No separate subprocess management needed.

### New `src/editor/tasks.rs`

```rust
pub enum TaskKind { Run, Build, Test }

pub struct TaskRunner;

impl TaskRunner {
    /// Resolve the command to run for a given language + task kind.
    pub fn resolve(lang: &str, kind: TaskKind, extensions: &[Extension],
                   config: &Config) -> Option<String> { ... }

    /// Expand {file}, {dir}, {stem}, {workspace} in a command template.
    pub fn expand(cmd: &str, file_path: &str, workspace: &str) -> String { ... }
}
```

### New Fields on `Editor`

```rust
last_task: Option<String>,      // last command sent, for "re-run" (F5 again)
task_language: Option<String>,  // language of last task
```

### Editor Actions

```rust
EditorAction::TaskRun,     // F5
EditorAction::TaskBuild,   // Ctrl+F5
EditorAction::TaskTest,    // Shift+F5
EditorAction::TaskStop,    // Alt+F5
```

### `handle_action()` additions

```rust
EditorAction::TaskRun => self.run_task(TaskKind::Run),

fn run_task(&mut self, kind: TaskKind) {
    let buf = &self.buffers[self.active_buffer_index()];
    let file_path = match &buf.file_path { Some(p) => p.clone(), None => return };
    let lang = self.detect_lsp_language(&file_path).unwrap_or_default();
    let workspace = std::env::current_dir()
        .ok().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();

    let cmd = match TaskRunner::resolve(&lang, kind, &self.extensions, &self.config) {
        Some(c) => TaskRunner::expand(&c, &file_path, &workspace),
        None => {
            self.set_status_message(&format!("No task configured for '{}'", lang));
            return;
        }
    };
    self.last_task = Some(cmd.clone());
    self.ensure_terminal_pane();
    self.send_to_active_terminal(&format!("{}\n", cmd));
}
```

---

## Phase 22: Problem Panel

**Goal**: Capture build output, parse errors, jump to source location with Enter.

### UI: Bottom Panel

```
╔═ Problems ══════════════════════════════════════════════════════╗
║ ● 3 errors   △ 1 warning   [cargo build — 2.3s]                ║
╠═════════════════════════════════════════════════════════════════╣
║ E src/main.rs:74:5   cannot find value `x` in this scope [E0425]║
║ E src/lib.rs:12:1    expected `;`, found `}`           [E0002]  ║
║ E src/main.rs:80:9   mismatched types: expected i32, got str    ║
║ W src/main.rs:22:5   unused variable: `result`          [W0001] ║
╚═════════════════════════════════════════════════════════════════╝
```

Keybindings:
- `Ctrl+Shift+P` — toggle Problem Panel open/close  ← (distinct from Command Palette)
- `Up`/`Down` — navigate items when panel focused
- `Enter` — jump to file:line:col
- `Escape` — close panel

### Error Parsers

Each parser takes a line of output and returns `Option<Problem>`:

```rust
pub struct Problem {
    pub severity: Severity,          // Error | Warning | Info | Note
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub message: String,
    pub code: Option<String>,        // "E0425", "W0001"
}

pub enum Severity { Error, Warning, Info, Note }

pub trait OutputParser {
    fn parse_line(&self, line: &str) -> Option<Problem>;
}
```

**Rust/Cargo parser** (handles `rustc` output format):
```
error[E0425]: cannot find value `x` in this scope
  --> src/main.rs:74:5
```

**Python parser**:
```
File "script.py", line 12
  SyntaxError: invalid syntax
```

**GCC/Clang parser**:
```
src/main.c:42:10: error: use of undeclared identifier 'foo'
```

**Generic parser** (fallback, matches `file:line:col: message`):
```
any/path.ext:N:M: error: message
```

**Zenith/Zymbol parser**: to be determined based on their error output format.

### New `src/problem_panel.rs`

```rust
pub struct ProblemPanel {
    pub visible: bool,
    pub items: Vec<Problem>,
    pub selected: usize,
    pub scroll: usize,
    pub source_cmd: Option<String>,   // "cargo build"
    pub elapsed_ms: u64,
    pub capture_buf: String,          // raw output being parsed in real-time
}

impl ProblemPanel {
    pub fn new() -> Self { ... }
    pub fn clear(&mut self) { ... }
    pub fn feed_line(&mut self, line: &str) { ... }  // called with each terminal output line
    pub fn selected_problem(&self) -> Option<&Problem> { ... }
    pub fn error_count(&self) -> usize { ... }
    pub fn warning_count(&self) -> usize { ... }
}
```

### Terminal Output Capture

When a Task (F5) is running, the terminal output is also fed to `ProblemPanel::feed_line()`.
This requires a small change to `vterm.rs` / `pty.rs`: emit an event when a new line is
written, which the editor captures to feed into the problem panel.

### Status Bar Integration

Status bar shows problem counts even when panel is hidden:
```
  ● E:3 W:1                              rust  UTF-8  LF  Ln 74  Col 5
```
Clicking on `E:3 W:1` opens the Problem Panel (mouse click on status bar).

---

## Phase 23: Zenith + Zymbol Deep Integration

**Goal**: First-class support for the two languages in the Z ecosystem.

### Current State

Both already have basic support:
- `zenith` → `.zl` files → `grammars/zenith.tmLanguage.json` ✓
- `zymbol` → `.zy` files → `grammars/zymbol.tmLanguage.json` ✓

Missing: LSP servers, task runner integration, grammar improvements.

### Part A: LSP Server Integration

Both projects have LSP servers ready to use:

```
~/github/ash-project/zenith-lang/crates/zenith-lsp/  → binary: zenith-lsp
~/github/mini-project/crates/zymbol-lsp/             → binary: zymbol-lsp
```

Build them once:
```sh
cd ~/github/ash-project/zenith-lang && cargo build --release -p zenith-lsp
cd ~/github/mini-project             && cargo build --release -p zymbol-lsp
```

Then add to `~/.config/zedit/config.json`:
```json
"lsp": {
  "rust":   { "command": "rust-analyzer", "args": [] },
  "zenith": { "command": "/full/path/to/zenith-lsp", "args": [] },
  "zymbol": { "command": "/full/path/to/zymbol-lsp", "args": [] }
}
```

For zero-config startup, the extension system (Phase 7δ-B) can auto-detect these
if the binaries are on `$PATH` (e.g., via `cargo install` or symlink in `~/.cargo/bin`).

### Part B: Task Defaults

Built-in task defaults (added to Phase 21's table):

```rust
("zenith", Run) => Some("zenith {file}"),
("zenith", Build) => None,  // interpreted, no separate build step
("zenith", Test) => None,   // TBD based on zenith's test framework

("zymbol", Run) => Some("zymbol {file}"),
("zymbol", Build) => None,
```

### Part C: Grammar Improvements

Review and improve `grammars/zenith.tmLanguage.json` and `grammars/zymbol.tmLanguage.json`:

1. **String interpolation** highlighting (if supported by the languages)
2. **Operator** tokens
3. **Keyword** completeness check against current language specs
4. **Error token** for syntax mistakes (helpful without LSP)
5. Verify `.zy` extension is correctly mapped for Zymbol (currently yes in languages.json)

Method: open a real Zenith/Zymbol file in zedit, compare highlighting vs VS Code extension,
identify missing patterns, update the grammar files.

### Part D: REPL Integration (optional / future)

If Zenith or Zymbol have a REPL mode (`zenith --repl`, `zymbol --repl`):

```
Ctrl+Enter on a Zenith block → sends selection to zenith --repl in terminal pane
```

This requires no new infrastructure — just a task variant that opens a persistent
terminal session (the PTY stays open) and sends text to stdin.

---

## Implementation Order

### 1. Phase 7δ-A: Runtime Grammar Loading (Week 1)

Files to modify:
- `build.rs` — remove `include_str!()` calls for grammars
- `src/config.rs` — `load_languages()` reads from disk
- `src/syntax/highlight.rs` — `load_grammar()` searches path
- `src/main.rs` — remove embedded grammar references

Verification:
```sh
cargo build
# open a .rs file — syntax highlighting still works (loads from ./grammars/)
# binary size should drop from ~2.5MB to ~600KB
strip target/debug/zedit && ls -lh target/debug/zedit
```

### 2. Phase 7δ-B: Native Extension System (Week 1–2)

New files:
- `src/extension.rs`

Modified:
- `src/config.rs` — integrate extension loading
- `src/main.rs` — `--ext` CLI arg parsing

Verification:
```sh
zedit --ext list     # prints "(no extensions installed)"
mkdir -p ~/.config/zedit/extensions/ruby
# create minimal manifest.json
zedit --ext list     # prints "ruby  Ruby Support  1.0.0"
```

### 3. Phase 7δ-C: VS Code Importer (Week 2)

New files:
- `src/vsix_import.rs`

Modified:
- `src/main.rs` — `--import` CLI arg parsing

Verification:
```sh
# Download a .vsix manually from VS Code Marketplace, then:
zedit --import ./ruby-1.0.0.vsix
zedit --ext list     # ruby extension now installed
# Open a .rb file — syntax highlighting works
```

### 4. Phase 21: Task Runner (Week 3)

New files:
- `src/editor/tasks.rs`

Modified:
- `src/keybindings.rs` — F5, Ctrl+F5, Shift+F5, Alt+F5
- `src/editor/mod.rs` — run_task(), ensure_terminal_pane()
- `src/config.rs` — load tasks from extensions + config.json

Verification:
```sh
# Open src/main.rs, press F5
# → terminal opens, "cargo run" executes
# Open script.py, press F5
# → terminal opens, "python3 script.py" executes
```

### 5. Phase 22: Problem Panel (Week 3–4)

New files:
- `src/problem_panel.rs`

Modified:
- `src/editor/mod.rs` — capture terminal output + feed to panel
- `src/editor/view.rs` — render_problem_panel()
- `src/vterm.rs` — line-output callback
- `src/keybindings.rs` — Ctrl+Shift+P (toggle panel)

Verification:
```sh
# Open src/main.rs with a deliberate error, press Ctrl+F5
# → Problem Panel appears with parsed error
# Press Enter on the error → cursor jumps to file:line:col
```

### 6. Phase 23: Zenith + Zymbol Integration (Week 4)

New files:
- `~/.config/zedit/extensions/zenith/manifest.json` (generated by docs or install script)
- `~/.config/zedit/extensions/zymbol/manifest.json`

Modified:
- `grammars/zenith.tmLanguage.json` — grammar improvements
- `grammars/zymbol.tmLanguage.json` — grammar improvements
- `src/editor/tasks.rs` — zenith/zymbol defaults

Verification:
```sh
# Build zenith-lsp and zymbol-lsp, add to config.json
# Open a .zl file → syntax highlighting + LSP diagnostics
# Press F5 → zenith app.zl runs in terminal
```

---

## Verification Suite

```sh
cargo build && cargo test && cargo clippy && cargo fmt -- --check
```

End-to-end manual test:

| Test | Expected |
|------|----------|
| Open `.rs` file | Syntax highlighting (loaded from `./grammars/`) |
| `zedit --ext list` | Lists installed extensions |
| `zedit --import ./ruby.vsix` | Ruby extension installed, `.rb` files highlighted |
| F5 on `.rs` file | `cargo run` sent to terminal |
| F5 on `.py` file | `python3 file.py` sent to terminal |
| F5 on `.zl` file | `zenith file.zl` sent to terminal |
| Build error → Ctrl+Shift+P | Problem Panel shows parsed errors |
| Enter on error in panel | Cursor jumps to file:line:col |
| Alt+K on Zenith symbol | Hover popup with type info (if zenith-lsp running) |
| Binary size | `strip`ped binary < 700KB |

---

## Design Constraints (Inherited)

- Zero external Rust dependencies — all new code uses only `std`
- Shell-out allowed for: `curl`, `unzip`, `git` (these are system utilities, not Rust crates)
- Startup time budget: < 10ms (grammar loading is lazy, only when a file is opened)
- Performance: grammar search path checked once at startup, cached
- All new user-facing text strings in English
