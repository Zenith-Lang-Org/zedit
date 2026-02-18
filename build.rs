// Build script that auto-embeds all grammar files from grammars/ directory.
// Generates `embedded_grammars.rs` in $OUT_DIR with:
// - A function returning a static map of grammar filename → content
// - The embedded languages.json content
//
// Adding a new built-in language requires only:
// 1. Place the .tmLanguage.json in grammars/
// 2. Add an entry to grammars/languages.json
// 3. cargo build — zero Rust code changes needed.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("embedded_grammars.rs");

    let grammars_dir = Path::new("grammars");

    // Collect all .tmLanguage.json files
    let mut grammar_files: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(grammars_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".tmLanguage.json") {
                grammar_files.push(name);
            }
        }
    }
    grammar_files.sort();

    // Generate the embedded grammars source
    let mut code = String::new();

    // Embedded languages.json
    code.push_str("/// Embedded languages.json content.\n");
    code.push_str(
        "pub const EMBEDDED_LANGUAGES_JSON: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/grammars/languages.json\"));\n\n",
    );

    // Grammar map function
    code.push_str("/// Map grammar filename to built-in embedded content.\n");
    code.push_str("pub fn builtin_grammar_str(grammar_file: &str) -> Option<&'static str> {\n");
    code.push_str("    match grammar_file {\n");

    for file in &grammar_files {
        code.push_str(&format!(
            "        \"{}\" => Some(include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/grammars/{}\"))),\n",
            file, file
        ));
    }

    code.push_str("        _ => None,\n");
    code.push_str("    }\n");
    code.push_str("}\n");

    fs::write(&dest_path, code).unwrap();

    // Tell Cargo to re-run if grammars/ changes
    println!("cargo:rerun-if-changed=grammars/");
}
