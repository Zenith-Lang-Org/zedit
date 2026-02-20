mod buffer;
mod config;
mod cursor;
mod editor;
mod filetree;
mod git;
mod input;
mod keybindings;
mod layout;
mod pty;
mod render;
mod session;
mod swap;
mod syntax;
mod terminal;
mod undo;
pub mod unicode;
mod vterm;

use std::env;
use std::path::{Path, PathBuf};

const VERSION: &str = "0.1.0";

fn print_help() {
    println!("zedit {} - modern console text editor", VERSION);
    println!();
    println!("Usage: zedit [file]");
    println!();
    println!("Options:");
    println!("  -h, --help       Print this help message and exit");
    println!("  -V, --version    Print version and exit");
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
    println!();
    println!("See zedit(1) man page for full documentation.");
}

fn main() {
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
            None => {
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
