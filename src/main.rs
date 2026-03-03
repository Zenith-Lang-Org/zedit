mod buffer;
mod clipboard;
mod config;
mod cursor;
pub mod debug_log;
mod diff_view;
mod editor;
mod extension;
mod filetree;
mod git;
mod glob;
mod input;
mod keybindings;
mod layout;
mod lsp;
mod mmap;
mod oklab;
mod plugin;
mod problem_panel;
mod pty;
mod render;
mod session;
mod simd;
mod swap;
mod syntax;
mod terminal;
mod undo;
pub mod unicode;
mod vmem;
mod vsix_import;
mod vterm;

use std::env;
use std::path::{Path, PathBuf};

const VERSION: &str = "0.1.0";

fn print_help() {
    println!("zedit {} - modern console text editor", VERSION);
    println!();
    println!("Usage: zedit [options] [file]");
    println!();
    println!("Options:");
    println!("  -h, --help               Print this help message and exit");
    println!("  -V, --version            Print version and exit");
    println!("  --log                    Startup timing → ~/.local/state/zedit/perf.log");
    println!("  --log-key                Keypress latency → ~/.local/state/zedit/key.log");
    println!("  --log-render             Frame render breakdown → ~/.local/state/zedit/render.log");
    println!("  --log-syntax             Tokenizer timing → ~/.local/state/zedit/syntax.log");
    println!("  --ext list               List installed extensions");
    println!("  --ext install <path>     Install extension from directory");
    println!("  --ext remove <id>        Remove installed extension");
    println!("  --ext info <id>          Show extension details");
    println!("  --install-grammars       Copy bundled grammars to ~/.config/zedit/grammars/");
    println!("  --import <arg>           Import a VS Code .vsix extension");
    println!("                           <arg>: local .vsix path, URL, or publisher.name");
    println!();
    println!("Keybindings:");
    println!("  Ctrl+S       Save          Ctrl+Z      Undo");
    println!("  Ctrl+Shift+S Save As       Ctrl+Y      Redo");
    println!("  Ctrl+O       Open file     Ctrl+C      Copy");
    println!("  Ctrl+Q       Quit          Ctrl+X      Cut");
    println!("  Ctrl+N       New buffer    Ctrl+V      Paste");
    println!("  Ctrl+W       Close buffer  Ctrl+F      Find");
    println!("  Ctrl+PgDn    Next buffer   Ctrl+H      Replace");
    println!("  Ctrl+PgUp    Prev buffer   Ctrl+G      Go to line");
    println!("  F1           Help overlay  Ctrl+/      Comment");
    println!();
    println!("Configuration: ~/.config/zedit/config.json");
    println!("Grammars:      ~/.config/zedit/grammars/");
    println!("Extensions:    ~/.config/zedit/extensions/");
    println!();
    println!("See zedit(1) man page for full documentation.");
}

fn install_grammars() {
    use crate::syntax::highlight::grammar_search_dirs;
    use std::path::PathBuf;

    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => {
            eprintln!("Error: HOME environment variable not set");
            std::process::exit(1);
        }
    };

    let dest_dir = PathBuf::from(format!("{}/.config/zedit/grammars", home));
    let dest_canonical =
        std::fs::canonicalize(&dest_dir).unwrap_or_else(|_| dest_dir.clone());

    println!("Searching for bundled grammars...");

    // Find the first source directory (other than the destination) with grammars.
    let mut source_dir: Option<PathBuf> = None;
    for dir in grammar_search_dirs() {
        let dir_canonical =
            std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if dir_canonical == dest_canonical {
            continue; // skip the destination itself
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let has_grammars = entries.flatten().any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .ends_with(".tmLanguage.json")
            });
            if has_grammars {
                source_dir = Some(dir);
                break;
            }
        }
    }

    let source_dir = match source_dir {
        Some(d) => d,
        None => {
            eprintln!(
                "No bundled grammars found.\n\
                 Place a 'grammars/' folder next to the zedit binary, or build from source."
            );
            std::process::exit(1);
        }
    };

    println!("Found grammars at {}", source_dir.display());

    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        eprintln!("Error creating {}: {}", dest_dir.display(), e);
        std::process::exit(1);
    }

    println!("Installing to {}", dest_dir.display());

    let grammar_entries: Vec<_> = match std::fs::read_dir(&source_dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .ends_with(".tmLanguage.json")
            })
            .collect(),
        Err(e) => {
            eprintln!("Error reading {}: {}", source_dir.display(), e);
            std::process::exit(1);
        }
    };

    let mut count = 0usize;
    for entry in &grammar_entries {
        let file_name = entry.file_name();
        let dest_file = dest_dir.join(&file_name);

        // Skip if the user's copy is newer than the source.
        let should_copy = match (entry.metadata(), std::fs::metadata(&dest_file)) {
            (Ok(src_m), Ok(dst_m)) => match (src_m.modified(), dst_m.modified()) {
                (Ok(src_t), Ok(dst_t)) => src_t > dst_t,
                _ => true,
            },
            _ => true, // destination does not exist yet
        };

        if should_copy {
            match std::fs::copy(entry.path(), &dest_file) {
                Ok(_) => {
                    println!("  + {}", file_name.to_string_lossy());
                    count += 1;
                }
                Err(e) => {
                    eprintln!("  ! {}: {}", file_name.to_string_lossy(), e);
                }
            }
        }
    }

    println!("Done. {} grammar(s) installed.", count);
}

fn handle_ext_command(args: &[String]) {
    let subcmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match subcmd {
        "list" => {
            let exts = extension::list_extensions();
            if exts.is_empty() {
                println!("(no extensions installed)");
                println!();
                println!("Install with: zedit --ext install <path-to-extension-directory>");
            } else {
                println!("{:<20} {:<30} {}", "ID", "NAME", "VERSION");
                println!("{}", "-".repeat(55));
                for (id, name, version) in exts {
                    println!("{:<20} {:<30} {}", id, name, version);
                }
            }
        }
        "install" => {
            let path_str = match args.get(1) {
                Some(p) => p,
                _none => {
                    eprintln!("Usage: zedit --ext install <path>");
                    std::process::exit(1);
                }
            };
            let path = Path::new(path_str);
            match extension::install_extension(path) {
                Ok(id) => println!("Extension '{}' installed successfully.", id),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        "remove" => {
            let id = match args.get(1) {
                Some(id) => id,
                _none => {
                    eprintln!("Usage: zedit --ext remove <id>");
                    std::process::exit(1);
                }
            };
            match extension::uninstall_extension(id) {
                Ok(()) => println!("Extension '{}' removed.", id),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        "info" => {
            let id = match args.get(1) {
                Some(id) => id,
                _none => {
                    eprintln!("Usage: zedit --ext info <id>");
                    std::process::exit(1);
                }
            };
            let base = match extension::extension_base_dir() {
                Some(b) => b,
                _none => {
                    eprintln!("Error: cannot determine extensions directory");
                    std::process::exit(1);
                }
            };
            let ext_dir = base.join(id);
            match extension::load_extension_dir(&ext_dir) {
                Ok(ext) => {
                    println!("ID:       {}", ext.id);
                    println!("Name:     {}", ext.name);
                    println!("Version:  {}", ext.version);
                    println!("Dir:      {}", ext.dir.display());
                    if ext.languages.is_empty() {
                        println!("Languages: (_none)");
                    } else {
                        let names: Vec<&str> =
                            ext.languages.iter().map(|l| l.name.as_str()).collect();
                        println!("Languages: {}", names.join(", "));
                    }
                    if let Some(lsp) = &ext.lsp {
                        println!("LSP:      {} {:?}", lsp.command, lsp.args);
                    }
                    if !ext.tasks.is_empty() {
                        let task_names: Vec<&str> =
                            ext.tasks.iter().map(|(n, _)| n.as_str()).collect();
                        println!("Tasks:    {}", task_names.join(", "));
                    }
                }
                Err(e) => {
                    eprintln!("Error: extension '{}' not found or invalid: {}", id, e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Unknown subcommand: '{}'", subcmd);
            eprintln!();
            eprintln!("Available subcommands:");
            eprintln!("  list               List installed extensions");
            eprintln!("  install <path>     Install extension from directory");
            eprintln!("  remove <id>        Remove installed extension");
            eprintln!("  info <id>          Show extension details");
            std::process::exit(1);
        }
    }
}

/// Return the shell's logical working directory.
///
/// Uses `$PWD` (set by the shell, preserves symlinks) rather than `getcwd()`
/// (which resolves symlinks to their physical target).  This ensures that two
/// symlinked paths that point to the same inode are still treated as
/// independent session locations.
fn logical_cwd() -> PathBuf {
    std::env::var("PWD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn main() {
    debug_log::init();
    let args: Vec<String> = env::args().collect();

    // Strip diagnostic flags from the argument list before processing other flags.
    // These may appear in any position before the file argument.
    let log_mode        = args.iter().any(|a| a == "--log");
    let key_log_mode    = args.iter().any(|a| a == "--log-key");
    let render_log_mode = args.iter().any(|a| a == "--log-render");
    let syntax_log_mode = args.iter().any(|a| a == "--log-syntax");
    let args: Vec<String> = args.into_iter()
        .filter(|a| !matches!(a.as_str(),
            "--log" | "--log-key" | "--log-render" | "--log-syntax"))
        .collect();

    if log_mode        { debug_log::perf_enable();       perf!("zedit {} startup", VERSION); }
    if key_log_mode    { debug_log::key_log_enable();    }
    if render_log_mode { debug_log::render_log_enable(); }
    if syntax_log_mode { debug_log::syntax_log_enable(); }

    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_help();
                return;
            }
            "--version" | "-V" => {
                println!("zedit {}", VERSION);
                return;
            }
            "--ext" => {
                handle_ext_command(&args[2..]);
                return;
            }
            "--install-grammars" => {
                install_grammars();
                return;
            }
            "--import" => {
                let arg = match args.get(2) {
                    Some(a) => a,
                    _none => {
                        eprintln!("Usage: zedit --import <path|url|publisher.name>");
                        std::process::exit(1);
                    }
                };
                match vsix_import::import_from_arg(arg) {
                    Ok(id) => {
                        println!(
                            "Extension '{}' installed. Use 'zedit --ext info {}' to inspect.",
                            id, id
                        );
                    }
                    Err(e) => {
                        eprintln!("Import failed: {}", e);
                        std::process::exit(1);
                    }
                }
                return;
            }
            _ => {}
        }
    }

    let config = config::Config::load();
    perf!("config loaded (theme={}, {} languages)", config.theme, config.languages.len());

    let has_file_arg = args.len() > 1;

    let mut editor = if has_file_arg {
        let path = Path::new(&args[1]);
        perf!("opening file: {}", path.display());
        let mut ed = editor::Editor::open(path, config).unwrap_or_else(|e| {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        });
        perf!("file opened: {}", path.display());
        // Check for orphaned swap on the opened file
        ed.check_swap_on_open(path);
        ed
    } else {
        // Try to restore session
        let cwd = logical_cwd();
        perf!("loading session for: {}", cwd.display());
        match session::load_session(&cwd) {
            Some(sess) => {
                let n = sess.buffers.len();
                perf!("session found: {} buffer(s)", n);
                let ed = editor::Editor::restore_session(sess, config).unwrap_or_else(|e| {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                });
                perf!("session restored: {} buffer(s) loaded", n);
                ed
            }
            _none => {
                perf!("no session — starting fresh");
                // No session, but check for orphaned untitled swap files
                let orphans = swap::scan_orphaned_untitled();
                if !orphans.is_empty() {
                    // Build a minimal session to trigger recovery
                    let mut buf_sessions = Vec::new();
                    for (id, _) in &orphans {
                        buf_sessions.push(session::BufferSession {
                            file_path: None,
                            cursor_line: 0,
                            cursor_col: 0,
                            scroll_row: 0,
                            has_swap: true,
                            untitled_index: Some(*id),
                        });
                    }
                    let sess = session::Session {
                        version: 1,
                        working_dir: cwd,
                        buffers: buf_sessions,
                        active_buffer: 0,
                        filetree_open: false,
                        filetree_expanded_dirs: Vec::new(),
                        minimap_visible: false,
                        bottom_panel_open: false,
                        bottom_tab: "terminal".to_string(),
                        word_wrap: false,
                    };
                    editor::Editor::restore_session(sess, config).unwrap_or_else(|e| {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    })
                } else {
                    editor::Editor::new(config).unwrap_or_else(|e| {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    })
                }
            }
        }
    };

    perf!("editor ready — entering main loop");

    if let Err(e) = editor.run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    debug_log::perf_finish();
}
