/// Native zedit extension system.
///
/// An extension lives at ~/.config/zedit/extensions/<id>/ and contains
/// a manifest.json describing languages, grammars, LSP config, and tasks.
use std::path::{Path, PathBuf};

use crate::config::LanguageDef;
use crate::syntax::json_parser::JsonValue;

// ── Extension types ──────────────────────────────────────────

pub struct ExtLspConfig {
    pub command: String,
    pub args: Vec<String>,
}

pub struct TaskDef {
    pub cmd: String,
    pub cwd: String, // template: {workspace}, {dir}, {file}, {stem}
}

pub struct Extension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub dir: PathBuf, // installed directory path
    pub languages: Vec<LanguageDef>,
    pub lsp: Option<ExtLspConfig>,
    pub tasks: Vec<(String, TaskDef)>, // e.g. [("run", ...), ("build", ...)]
}

// Manual Debug impl because LanguageDef doesn't derive Debug.
impl std::fmt::Debug for Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Extension {{ id: {:?}, version: {:?} }}",
            self.id, self.version
        )
    }
}

// ── Directory helpers ─────────────────────────────────────────

/// Base directory for installed extensions: ~/.config/zedit/extensions/
pub fn extension_base_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(format!("{}/.config/zedit/extensions", home)))
}

// ── Load ─────────────────────────────────────────────────────

/// Load all installed extensions from ~/.config/zedit/extensions/.
pub fn load_extensions() -> Vec<Extension> {
    let base = match extension_base_dir() {
        Some(b) => b,
        None => return Vec::new(),
    };
    let entries = match std::fs::read_dir(&base) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut extensions = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            match load_extension_dir(&entry.path()) {
                Ok(ext) => extensions.push(ext),
                Err(e) => {
                    crate::dlog!("[extension] skipping {:?}: {}", entry.path(), e);
                }
            }
        }
    }
    extensions
}

/// Load a single extension from a directory.
pub fn load_extension_dir(path: &Path) -> Result<Extension, String> {
    let manifest_path = path.join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("cannot read manifest.json: {}", e))?;
    parse_manifest(&text, path)
}

fn parse_manifest(json: &str, dir: &Path) -> Result<Extension, String> {
    let val = JsonValue::parse(json).map_err(|e| format!("JSON parse error: {:?}", e))?;

    let id = val
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("manifest missing 'id' field")?
        .to_string();

    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();

    let version = val
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();

    // Build language-id → grammar-filename map from "grammars" array.
    let mut grammar_map: Vec<(String, String)> = Vec::new();
    if let Some(grammars) = val.get("grammars").and_then(|v| v.as_array()) {
        for g in grammars {
            if let (Some(lang), Some(path)) = (
                g.get("language").and_then(|v| v.as_str()),
                g.get("path").and_then(|v| v.as_str()),
            ) {
                grammar_map.push((lang.to_string(), path.to_string()));
            }
        }
    }

    // Parse language definitions.
    let mut languages = Vec::new();
    if let Some(langs) = val.get("languages").and_then(|v| v.as_array()) {
        for l in langs {
            let lang_id = match l.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };

            let extensions: Vec<String> = l
                .get("extensions")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.trim_start_matches('.').to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            if extensions.is_empty() {
                continue;
            }

            // Find the grammar filename declared for this language.
            let grammar_file = grammar_map
                .iter()
                .find(|(lid, _)| lid == &lang_id)
                .map(|(_, p)| p.clone())
                .unwrap_or_default();

            let comment = l
                .get("comment")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            languages.push(LanguageDef {
                name: lang_id,
                extensions,
                grammar_file,
                comment,
            });
        }
    }

    // LSP config.
    let lsp = val.get("lsp").and_then(|lsp_val| {
        let command = lsp_val.get("command")?.as_str()?.to_string();
        let args = lsp_val
            .get("args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Some(ExtLspConfig { command, args })
    });

    // Tasks.
    let mut tasks: Vec<(String, TaskDef)> = Vec::new();
    if let Some(tasks_pairs) = val.get("tasks").and_then(|v| v.as_object()) {
        for (task_name, task_val) in tasks_pairs {
            if let Some(cmd) = task_val.get("cmd").and_then(|v| v.as_str()) {
                let cwd = task_val
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{workspace}")
                    .to_string();
                tasks.push((
                    task_name.clone(),
                    TaskDef {
                        cmd: cmd.to_string(),
                        cwd,
                    },
                ));
            }
        }
    }

    Ok(Extension {
        id,
        name,
        version,
        dir: dir.to_path_buf(),
        languages,
        lsp,
        tasks,
    })
}

// ── Install / uninstall ──────────────────────────────────────

/// Install an extension directory into ~/.config/zedit/extensions/<id>/.
/// Returns the installed extension id.
pub fn install_extension(src: &Path) -> Result<String, String> {
    let ext = load_extension_dir(src)?;
    let base = extension_base_dir().ok_or("cannot determine extensions directory")?;

    std::fs::create_dir_all(&base)
        .map_err(|e| format!("cannot create extensions directory: {}", e))?;

    let dest = base.join(&ext.id);
    if dest.exists() {
        return Err(format!(
            "extension '{}' is already installed. Remove it first with: zedit --ext remove {}",
            ext.id, ext.id
        ));
    }

    std::fs::create_dir_all(&dest)
        .map_err(|e| format!("cannot create extension directory: {}", e))?;

    copy_dir_all(src, &dest)?;
    Ok(ext.id)
}

/// Uninstall an extension by id.
pub fn uninstall_extension(id: &str) -> Result<(), String> {
    let base = extension_base_dir().ok_or("cannot determine extensions directory")?;
    let ext_dir = base.join(id);
    if !ext_dir.exists() {
        return Err(format!("extension '{}' is not installed", id));
    }
    std::fs::remove_dir_all(&ext_dir).map_err(|e| format!("cannot remove extension: {}", e))
}

/// List installed extensions: (id, name, version).
pub fn list_extensions() -> Vec<(String, String, String)> {
    load_extensions()
        .into_iter()
        .map(|e| (e.id, e.name, e.version))
        .collect()
}

// ── Directory copy helper ────────────────────────────────────

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(src).map_err(|e| format!("cannot read directory: {}", e))?;
    for entry in entries.flatten() {
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            std::fs::create_dir_all(&dst_path).map_err(|e| e.to_string())?;
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path).map_err(|e| format!("copy failed: {}", e))?;
        }
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_manifest(id: &str) -> String {
        format!(
            r#"{{
  "id": "{}",
  "name": "Test Extension",
  "version": "1.0.0",
  "languages": [
    {{ "id": "{}", "extensions": [".tl", ".tlx"], "comment": "//" }}
  ],
  "grammars": [
    {{ "language": "{}", "scopeName": "source.test", "path": "test.tmLanguage.json" }}
  ]
}}"#,
            id, id, id
        )
    }

    #[test]
    fn test_parse_manifest_basic() {
        let json = minimal_manifest("testlang");
        let ext = parse_manifest(&json, Path::new("/fake/path")).unwrap();
        assert_eq!(ext.id, "testlang");
        assert_eq!(ext.name, "Test Extension");
        assert_eq!(ext.version, "1.0.0");
        assert_eq!(ext.languages.len(), 1);
        let lang = &ext.languages[0];
        assert_eq!(lang.name, "testlang");
        assert_eq!(lang.extensions, vec!["tl", "tlx"]); // dots stripped
        assert_eq!(lang.grammar_file, "test.tmLanguage.json");
        assert_eq!(lang.comment.as_deref(), Some("//"));
        assert!(ext.lsp.is_none());
        assert!(ext.tasks.is_empty());
    }

    #[test]
    fn test_parse_manifest_with_lsp_and_tasks() {
        let json = r#"{
  "id": "rust",
  "name": "Rust Support",
  "version": "2.0.0",
  "languages": [{ "id": "rust", "extensions": [".rs"] }],
  "grammars": [{ "language": "rust", "scopeName": "source.rust", "path": "grammar.tmLanguage.json" }],
  "lsp": { "command": "rust-analyzer", "args": [] },
  "tasks": {
    "run":   { "cmd": "cargo run",   "cwd": "{workspace}" },
    "build": { "cmd": "cargo build", "cwd": "{workspace}" }
  }
}"#;
        let ext = parse_manifest(json, Path::new("/fake")).unwrap();
        assert_eq!(ext.id, "rust");
        let lsp = ext.lsp.unwrap();
        assert_eq!(lsp.command, "rust-analyzer");
        assert!(lsp.args.is_empty());
        assert_eq!(ext.tasks.len(), 2);
        let run_task = ext.tasks.iter().find(|(n, _)| n == "run").unwrap();
        assert_eq!(run_task.1.cmd, "cargo run");
    }

    #[test]
    fn test_parse_manifest_missing_id() {
        let json = r#"{ "name": "No ID Extension", "version": "1.0.0", "languages": [] }"#;
        let result = parse_manifest(json, Path::new("/fake"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing 'id'"));
    }

    #[test]
    fn test_parse_manifest_dot_stripped_extensions() {
        let json = r#"{ "id": "ex", "languages": [{ "id": "ex", "extensions": [".ex", "exs"] }], "grammars": [] }"#;
        let ext = parse_manifest(json, Path::new("/fake")).unwrap();
        assert_eq!(ext.languages[0].extensions, vec!["ex", "exs"]);
    }

    #[test]
    fn test_list_extensions_nonexistent_dir() {
        // list_extensions() on missing dir returns empty — no crash
        let result = load_extensions();
        // Just ensure it doesn't panic; we can't assert count in a real env
        let _ = result;
    }

    #[test]
    fn test_install_uninstall_roundtrip() {
        let tmp = std::env::temp_dir().join("zedit_ext_test_roundtrip");
        let src = tmp.join("mysrc");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("manifest.json"), minimal_manifest("roundtrip-ext")).unwrap();

        // Install into a temp extensions dir by temporarily pointing HOME
        let fake_home = tmp.join("home");
        let ext_base = fake_home.join(".config/zedit/extensions");
        std::fs::create_dir_all(&ext_base).unwrap();

        // We can't easily override HOME in tests, so test copy_dir_all directly.
        let dest = ext_base.join("roundtrip-ext");
        std::fs::create_dir_all(&dest).unwrap();
        copy_dir_all(&src, &dest).unwrap();

        let loaded = load_extension_dir(&dest).unwrap();
        assert_eq!(loaded.id, "roundtrip-ext");

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_uninstall_missing_extension() {
        // Calling uninstall on a non-existent extension returns an error.
        // We call it with an id that definitely doesn't exist.
        // This would use the real HOME — just verify it returns Err, not panics.
        let result = uninstall_extension("__zedit_test_nonexistent_extension_xyz__");
        assert!(result.is_err());
    }
}
