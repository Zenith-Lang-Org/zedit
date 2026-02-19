mod buffer;
mod config;
mod cursor;
mod editor;
mod git;
mod input;
mod layout;
mod render;
mod syntax;
mod terminal;
mod undo;
pub mod unicode;

use std::env;
use std::path::Path;

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

    let mut editor = if args.len() > 1 {
        editor::Editor::open(Path::new(&args[1]), config)
    } else {
        editor::Editor::new(config)
    }
    .unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    });

    if let Err(e) = editor.run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
