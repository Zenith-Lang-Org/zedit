mod buffer;
mod clipboard;
mod config;
mod cursor;
pub mod debug_log;
mod diff_view;
mod editor;
mod extension;
mod filetree;
mod glob;
mod git;
mod input;
mod keybindings;
mod layout;
mod lsp;
mod plugin;
mod problem_panel;
mod pty;
mod render;
mod session;
mod swap;
mod syntax;
mod terminal;
mod undo;
pub mod unicode;
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
    println!("  --ext list               List installed extensions");
    println!("  --ext install <path>     Install extension from directory");
    println!("  --ext remove <id>        Remove installed extension");
    println!("  --ext info <id>          Show extension details");
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

fn main() {
    debug_log::init();
    let args: Vec<String> = env::args().collect();

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

    let has_file_arg = args.len() > 1;

    let mut editor = if has_file_arg {
        let path = Path::new(&args[1]);
        let mut ed = editor::Editor::open(path, config).unwrap_or_else(|e| {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        });
        // Check for orphaned swap on the opened file
        ed.check_swap_on_open(path);
        ed
    } else {
        // Try to restore session
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        match session::load_session(&cwd) {
            Some(sess) => editor::Editor::restore_session(sess, config).unwrap_or_else(|e| {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }),
            _none => {
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

    if let Err(e) = editor.run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
