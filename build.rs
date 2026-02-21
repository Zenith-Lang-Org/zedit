use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("embedded_grammars.rs");

    // Only embed languages.json (2.3KB metadata).
    // Grammar files (.tmLanguage.json) are loaded from disk at runtime.
    let code = "/// Embedded languages.json content.\n\
        pub const EMBEDDED_LANGUAGES_JSON: &str = \
        include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/grammars/languages.json\"));\n";

    fs::write(&dest_path, code).unwrap();

    println!("cargo:rerun-if-changed=grammars/");
}
