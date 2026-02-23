/// VS Code .vsix importer.
///
/// Converts a VS Code extension (.vsix file) to the native zedit extension
/// format installed under ~/.config/zedit/extensions/<id>/.
///
/// .vsix files are ZIP archives. Extraction is done by shelling out to
/// `unzip`; downloads use `curl`. No Rust ZIP libraries needed.
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::extension;
use crate::syntax::json_parser::JsonValue;

// ── Public entry points ──────────────────────────────────────

/// Dispatch import from a CLI argument: local path, URL, or marketplace name.
pub fn import_from_arg(arg: &str) -> Result<String, String> {
    if arg.starts_with("http://") || arg.starts_with("https://") {
        import_from_url(arg)
    } else if arg.ends_with(".vsix") || std::path::Path::new(arg).exists() {
        import_vsix(Path::new(arg))
    } else {
        import_by_name(arg)
    }
}

/// Download and install a VS Code extension by publisher.name or plain name.
/// Queries Open VSX Registry (open-source, no ToS issues).
pub fn import_by_name(name: &str) -> Result<String, String> {
    println!("Searching Open VSX Registry for '{}'...", name);

    let (publisher, ext_name) = if let Some(dot) = name.find('.') {
        // publisher.name format (e.g. rust-lang.rust)
        (&name[..dot], &name[dot + 1..])
    } else {
        // Search by name and get first result
        let (pub_, nm) = search_open_vsx(name)?;
        // Return owned strings
        return import_by_name(&format!("{}.{}", pub_, nm));
    };

    let url = open_vsx_download_url(publisher, ext_name)?;
    println!("Downloading from: {}", url);

    let tmp_vsix = std::env::temp_dir().join(format!("zedit-import-{}.vsix", ext_name));
    download_file(&url, &tmp_vsix)?;

    let result = import_vsix(&tmp_vsix);
    let _ = std::fs::remove_file(&tmp_vsix);
    result
}

/// Download a .vsix from a URL and install it.
pub fn import_from_url(url: &str) -> Result<String, String> {
    println!("Downloading from: {}", url);
    let tmp_vsix = std::env::temp_dir().join("zedit-import-download.vsix");
    download_file(url, &tmp_vsix)?;
    let result = import_vsix(&tmp_vsix);
    let _ = std::fs::remove_file(&tmp_vsix);
    result
}

/// Convert a local .vsix file to a native zedit extension.
/// Returns the installed extension id.
pub fn import_vsix(vsix_path: &Path) -> Result<String, String> {
    if !vsix_path.exists() {
        return Err(format!("file not found: {}", vsix_path.display()));
    }

    println!("Extracting {}...", vsix_path.display());

    let tmp_dir = std::env::temp_dir().join("zedit-vsix-extract");
    // Clean up any leftover from previous run.
    let _ = std::fs::remove_dir_all(&tmp_dir);
    extract_vsix(vsix_path, &tmp_dir)?;

    let ext_root = tmp_dir.join("extension");
    let pkg_path = ext_root.join("package.json");
    if !pkg_path.exists() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err("invalid .vsix: missing extension/package.json".into());
    }

    let pkg_text = std::fs::read_to_string(&pkg_path)
        .map_err(|e| format!("cannot read package.json: {}", e))?;
    let pkg = parse_package_json(&pkg_text)?;

    let ext_id = format!("{}.{}", pkg.publisher, pkg.name);
    println!("Converting '{}' v{}...", pkg.display_name, pkg.version);

    let base = extension::extension_base_dir().ok_or("cannot determine extensions directory")?;
    std::fs::create_dir_all(&base)
        .map_err(|e| format!("cannot create extensions directory: {}", e))?;

    let dest = base.join(&ext_id);
    if dest.exists() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(format!(
            "extension '{}' is already installed. Remove it first with: zedit --ext remove {}",
            ext_id, ext_id
        ));
    }
    std::fs::create_dir_all(&dest)
        .map_err(|e| format!("cannot create extension directory: {}", e))?;

    // Copy grammar files to dest (using just the basename).
    let mut installed_grammars: Vec<InstalledGrammar> = Vec::new();
    for g in &pkg.grammars {
        let src_path = resolve_pkg_path(&ext_root, &g.path);
        if !src_path.exists() {
            eprintln!("  warning: grammar file not found: {}", g.path);
            continue;
        }
        let basename = src_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("grammar.tmLanguage.json")
            .to_string();
        let dst_path = dest.join(&basename);
        std::fs::copy(&src_path, &dst_path)
            .map_err(|e| format!("cannot copy grammar '{}': {}", g.path, e))?;
        installed_grammars.push(InstalledGrammar {
            language: g.language.clone(),
            scope_name: g.scope_name.clone(),
            installed_basename: basename,
        });
    }

    // Copy theme files to dest.
    let mut installed_themes: Vec<InstalledTheme> = Vec::new();
    for t in &pkg.themes {
        let src_path = resolve_pkg_path(&ext_root, &t.path);
        if !src_path.exists() {
            eprintln!("  warning: theme file not found: {}", t.path);
            continue;
        }
        let basename = src_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("theme.json")
            .to_string();
        let dst_path = dest.join(&basename);
        std::fs::copy(&src_path, &dst_path)
            .map_err(|e| format!("cannot copy theme '{}': {}", t.path, e))?;
        installed_themes.push(InstalledTheme {
            id: t.id.clone(),
            label: t.label.clone(),
            installed_basename: basename,
        });
    }

    // Try to detect line comment prefix per language from language-configuration.json files.
    let comment_map = collect_comment_prefixes(&ext_root, &pkg.languages);

    // Build and write manifest.json.
    let manifest_json = build_manifest_json(
        &ext_id,
        &pkg,
        &installed_grammars,
        &installed_themes,
        &comment_map,
    );
    std::fs::write(dest.join("manifest.json"), &manifest_json)
        .map_err(|e| format!("cannot write manifest.json: {}", e))?;

    let _ = std::fs::remove_dir_all(&tmp_dir);

    let grammar_count = installed_grammars.len();
    let theme_count = installed_themes.len();
    println!(
        "Installed '{}': {} grammar(s), {} theme(s)",
        ext_id, grammar_count, theme_count
    );
    Ok(ext_id)
}

// ── Internal data structures ──────────────────────────────────

struct PackageJson {
    publisher: String,
    name: String,
    display_name: String,
    version: String,
    grammars: Vec<PkgGrammar>,
    languages: Vec<PkgLanguage>,
    themes: Vec<PkgTheme>,
}

impl std::fmt::Debug for PackageJson {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PackageJson {{ publisher: {:?}, name: {:?} }}",
            self.publisher, self.name
        )
    }
}

struct PkgGrammar {
    language: String,
    scope_name: String,
    path: String,
}

struct PkgLanguage {
    id: String,
    extensions: Vec<String>,
    aliases: Vec<String>,
    configuration: Option<String>, // path to language-configuration.json
}

struct PkgTheme {
    id: String,
    label: String,
    path: String,
}

struct InstalledGrammar {
    language: String,
    scope_name: String,
    installed_basename: String,
}

struct InstalledTheme {
    id: String,
    label: String,
    installed_basename: String,
}

// ── package.json parser ───────────────────────────────────────

fn parse_package_json(text: &str) -> Result<PackageJson, String> {
    let val = JsonValue::parse(text).map_err(|e| format!("package.json parse error: {:?}", e))?;

    let publisher = val
        .get("publisher")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("package.json missing 'name' field")?
        .to_string();

    let display_name = val
        .get("displayName")
        .and_then(|v| v.as_str())
        .unwrap_or(&name)
        .to_string();

    let version = val
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();

    let contributes = val.get("contributes");

    // Parse contributes.grammars
    let mut grammars = Vec::new();
    if let Some(arr) = contributes
        .and_then(|c| c.get("grammars"))
        .and_then(|v| v.as_array())
    {
        for g in arr {
            let path = match g.get("path").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None => continue,
            };
            // Only import TextMate grammars (not JSON Language grammars)
            if !path.ends_with(".tmLanguage.json")
                && !path.ends_with(".tmLanguage")
                && !path.ends_with(".plist")
            {
                continue;
            }
            let language = g
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let scope_name = g
                .get("scopeName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            grammars.push(PkgGrammar {
                language,
                scope_name,
                path,
            });
        }
    }

    // Parse contributes.languages
    let mut languages = Vec::new();
    if let Some(arr) = contributes
        .and_then(|c| c.get("languages"))
        .and_then(|v| v.as_array())
    {
        for l in arr {
            let id = match l.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            let extensions: Vec<String> = l
                .get("extensions")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let aliases: Vec<String> = l
                .get("aliases")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let configuration = l
                .get("configuration")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            languages.push(PkgLanguage {
                id,
                extensions,
                aliases,
                configuration,
            });
        }
    }

    // Parse contributes.themes
    let mut themes = Vec::new();
    if let Some(arr) = contributes
        .and_then(|c| c.get("themes"))
        .and_then(|v| v.as_array())
    {
        for t in arr {
            let path = match t.get("path").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None => continue,
            };
            let id = t
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("theme")
                .to_string();
            let label = t
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string();
            themes.push(PkgTheme { id, label, path });
        }
    }

    Ok(PackageJson {
        publisher,
        name,
        display_name,
        version,
        grammars,
        languages,
        themes,
    })
}

// ── Comment prefix detection ──────────────────────────────────

/// Try to read line comment prefix from language-configuration.json files.
/// Returns a map of language-id → line-comment-prefix.
fn collect_comment_prefixes(ext_root: &Path, languages: &[PkgLanguage]) -> Vec<(String, String)> {
    let mut map = Vec::new();
    for lang in languages {
        let cfg_path = match &lang.configuration {
            Some(p) => resolve_pkg_path(ext_root, p),
            None => continue,
        };
        if !cfg_path.exists() {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&cfg_path) {
            if let Ok(val) = JsonValue::parse(&text) {
                if let Some(line) = val
                    .get("comments")
                    .and_then(|c| c.get("lineComment"))
                    .and_then(|v| v.as_str())
                {
                    map.push((lang.id.clone(), line.to_string()));
                }
            }
        }
    }
    map
}

// ── Manifest builder ──────────────────────────────────────────

fn build_manifest_json(
    ext_id: &str,
    pkg: &PackageJson,
    grammars: &[InstalledGrammar],
    themes: &[InstalledTheme],
    comment_map: &[(String, String)],
) -> String {
    // languages array
    let lang_items: Vec<JsonValue> = pkg
        .languages
        .iter()
        .map(|l| {
            let mut pairs: Vec<(String, JsonValue)> = vec![
                ("id".to_string(), JsonValue::String(l.id.clone())),
                (
                    "extensions".to_string(),
                    JsonValue::Array(
                        l.extensions
                            .iter()
                            .map(|e| JsonValue::String(e.clone()))
                            .collect(),
                    ),
                ),
            ];
            if !l.aliases.is_empty() {
                pairs.push((
                    "aliases".to_string(),
                    JsonValue::Array(
                        l.aliases
                            .iter()
                            .map(|a| JsonValue::String(a.clone()))
                            .collect(),
                    ),
                ));
            }
            if let Some(comment) = comment_map.iter().find(|(id, _)| id == &l.id) {
                pairs.push(("comment".to_string(), JsonValue::String(comment.1.clone())));
            }
            JsonValue::Object(pairs)
        })
        .collect();

    // grammars array
    let grammar_items: Vec<JsonValue> = grammars
        .iter()
        .map(|g| {
            JsonValue::Object(vec![
                (
                    "language".to_string(),
                    JsonValue::String(g.language.clone()),
                ),
                (
                    "scopeName".to_string(),
                    JsonValue::String(g.scope_name.clone()),
                ),
                (
                    "path".to_string(),
                    JsonValue::String(g.installed_basename.clone()),
                ),
            ])
        })
        .collect();

    // themes array
    let theme_items: Vec<JsonValue> = themes
        .iter()
        .map(|t| {
            JsonValue::Object(vec![
                ("id".to_string(), JsonValue::String(t.id.clone())),
                ("label".to_string(), JsonValue::String(t.label.clone())),
                (
                    "path".to_string(),
                    JsonValue::String(t.installed_basename.clone()),
                ),
            ])
        })
        .collect();

    let manifest = JsonValue::Object(vec![
        ("id".to_string(), JsonValue::String(ext_id.to_string())),
        (
            "name".to_string(),
            JsonValue::String(pkg.display_name.clone()),
        ),
        (
            "version".to_string(),
            JsonValue::String(pkg.version.clone()),
        ),
        ("languages".to_string(), JsonValue::Array(lang_items)),
        ("grammars".to_string(), JsonValue::Array(grammar_items)),
        ("themes".to_string(), JsonValue::Array(theme_items)),
    ]);

    manifest.to_json_pretty(2)
}

// ── Shell helpers ─────────────────────────────────────────────

fn extract_vsix(vsix_path: &Path, tmp_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(tmp_dir).map_err(|e| format!("cannot create temp dir: {}", e))?;

    let status = Command::new("unzip")
        .args([
            "-o",
            "-q",
            vsix_path.to_str().unwrap_or(""),
            "-d",
            tmp_dir.to_str().unwrap_or(""),
        ])
        .status()
        .map_err(|e| format!("cannot run 'unzip' (is it installed?): {}", e))?;

    if !status.success() {
        return Err(format!(
            "unzip failed with exit code {:?} — is '{}' a valid .vsix file?",
            status.code(),
            vsix_path.display()
        ));
    }
    Ok(())
}

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let status = Command::new("curl")
        .args(["-s", "-S", "-L", "-o", dest.to_str().unwrap_or(""), url])
        .status()
        .map_err(|e| format!("cannot run 'curl' (is it installed?): {}", e))?;

    if !status.success() {
        return Err(format!("curl failed with exit code {:?}", status.code()));
    }
    if !dest.exists() {
        return Err(format!("curl produced no output for URL: {}", url));
    }
    Ok(())
}

/// Run curl and capture stdout.
fn curl_get(url: &str) -> Result<String, String> {
    let output = Command::new("curl")
        .args(["-s", "-S", "-L", url])
        .output()
        .map_err(|e| format!("cannot run 'curl' (is it installed?): {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "curl failed for {}: exit code {:?}",
            url,
            output.status.code()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("curl output not UTF-8: {}", e))
}

// ── Open VSX Registry helpers ─────────────────────────────────

/// Search Open VSX for an extension by name; returns (publisher, name).
fn search_open_vsx(name: &str) -> Result<(String, String), String> {
    let url = format!(
        "https://open-vsx.org/api/-/search?query={}&size=5",
        url_encode(name)
    );
    let body = curl_get(&url)?;
    let val = JsonValue::parse(&body)
        .map_err(|e| format!("Open VSX search response parse error: {:?}", e))?;

    let exts = val
        .get("extensions")
        .and_then(|v| v.as_array())
        .ok_or("Open VSX search returned no results")?;

    if exts.is_empty() {
        return Err(format!("no extensions found for '{}'", name));
    }

    let first = &exts[0];
    let publisher = first
        .get("namespace")
        .and_then(|v| v.as_str())
        .ok_or("Open VSX response missing 'namespace'")?
        .to_string();
    let ext_name = first
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Open VSX response missing 'name'")?
        .to_string();

    println!(
        "Found: {}.{} — {}",
        publisher,
        ext_name,
        first
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or(&ext_name)
    );
    Ok((publisher, ext_name))
}

/// Get the download URL for a specific extension from Open VSX.
fn open_vsx_download_url(publisher: &str, name: &str) -> Result<String, String> {
    let api_url = format!(
        "https://open-vsx.org/api/{}/{}/latest",
        url_encode(publisher),
        url_encode(name)
    );
    let body = curl_get(&api_url)?;
    let val = JsonValue::parse(&body)
        .map_err(|e| format!("Open VSX API response parse error: {:?}", e))?;

    // Check for API error
    if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
        return Err(format!("Open VSX: {}", err));
    }

    val.get("files")
        .and_then(|f| f.get("download"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "Open VSX: no download URL found for '{}.{}'",
                publisher, name
            )
        })
}

// ── Path / string helpers ─────────────────────────────────────

/// Resolve a package.json-relative path to an absolute path.
/// Package.json paths look like `"./syntaxes/rust.tmLanguage.json"`.
fn resolve_pkg_path(ext_root: &Path, pkg_path: &str) -> PathBuf {
    let normalized = pkg_path.trim_start_matches("./");
    ext_root.join(normalized)
}

/// Minimal percent-encoding for URL path components.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            ' ' => out.push('+'),
            c => {
                for byte in c.to_string().as_bytes() {
                    out.push('%');
                    out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
                    out.push(char::from_digit((byte & 0xf) as u32, 16).unwrap_or('0'));
                }
            }
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_package_json(publisher: &str, name: &str) -> String {
        format!(
            r#"{{
  "publisher": "{}",
  "name": "{}",
  "displayName": "Test Extension",
  "version": "1.2.3",
  "contributes": {{
    "languages": [
      {{ "id": "{}", "extensions": [".tl", ".tlx"], "aliases": ["TestLang", "tl"] }}
    ],
    "grammars": [
      {{ "language": "{}", "scopeName": "source.tl", "path": "./syntaxes/testlang.tmLanguage.json" }}
    ],
    "themes": []
  }}
}}"#,
            publisher, name, name, name
        )
    }

    #[test]
    fn test_parse_package_json_basic() {
        let json = minimal_package_json("test-pub", "testlang");
        let pkg = parse_package_json(&json).unwrap();
        assert_eq!(pkg.publisher, "test-pub");
        assert_eq!(pkg.name, "testlang");
        assert_eq!(pkg.display_name, "Test Extension");
        assert_eq!(pkg.version, "1.2.3");
        assert_eq!(pkg.languages.len(), 1);
        assert_eq!(pkg.languages[0].id, "testlang");
        assert_eq!(pkg.languages[0].extensions, vec![".tl", ".tlx"]);
        assert_eq!(pkg.languages[0].aliases, vec!["TestLang", "tl"]);
        assert_eq!(pkg.grammars.len(), 1);
        assert_eq!(pkg.grammars[0].language, "testlang");
        assert_eq!(pkg.grammars[0].scope_name, "source.tl");
        assert_eq!(pkg.grammars[0].path, "./syntaxes/testlang.tmLanguage.json");
        assert!(pkg.themes.is_empty());
    }

    #[test]
    fn test_parse_package_json_missing_name() {
        let json = r#"{ "publisher": "foo", "version": "1.0.0" }"#;
        let result = parse_package_json(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing 'name'"));
    }

    #[test]
    fn test_parse_package_json_no_contributes() {
        let json = r#"{ "publisher": "foo", "name": "bar", "version": "1.0.0" }"#;
        let pkg = parse_package_json(json).unwrap();
        assert!(pkg.grammars.is_empty());
        assert!(pkg.languages.is_empty());
        assert!(pkg.themes.is_empty());
    }

    #[test]
    fn test_build_manifest_json() {
        let json = minimal_package_json("rust-lang", "rust");
        let pkg = parse_package_json(&json).unwrap();
        let grammars = vec![InstalledGrammar {
            language: "rust".to_string(),
            scope_name: "source.rust".to_string(),
            installed_basename: "rust.tmLanguage.json".to_string(),
        }];
        let manifest = build_manifest_json("rust-lang.rust", &pkg, &grammars, &[], &[]);
        // Parse back and verify
        let val = JsonValue::parse(&manifest).unwrap();
        assert_eq!(
            val.get("id").and_then(|v| v.as_str()),
            Some("rust-lang.rust")
        );
        assert_eq!(val.get("version").and_then(|v| v.as_str()), Some("1.2.3"));
        let langs = val.get("languages").and_then(|v| v.as_array()).unwrap();
        assert_eq!(langs.len(), 1);
        let grammars_out = val.get("grammars").and_then(|v| v.as_array()).unwrap();
        assert_eq!(grammars_out.len(), 1);
        assert_eq!(
            grammars_out[0].get("path").and_then(|v| v.as_str()),
            Some("rust.tmLanguage.json")
        );
    }

    #[test]
    fn test_build_manifest_with_comment() {
        let json = minimal_package_json("lang", "mylang");
        let pkg = parse_package_json(&json).unwrap();
        let comment_map = vec![("mylang".to_string(), "//".to_string())];
        let manifest = build_manifest_json("lang.mylang", &pkg, &[], &[], &comment_map);
        let val = JsonValue::parse(&manifest).unwrap();
        let langs = val.get("languages").and_then(|v| v.as_array()).unwrap();
        let comment = langs[0].get("comment").and_then(|v| v.as_str());
        assert_eq!(comment, Some("//"));
    }

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("rust"), "rust");
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("rust-lang"), "rust-lang");
    }

    #[test]
    fn test_resolve_pkg_path() {
        let root = Path::new("/tmp/extension");
        let resolved = resolve_pkg_path(root, "./syntaxes/rust.tmLanguage.json");
        assert_eq!(
            resolved,
            Path::new("/tmp/extension/syntaxes/rust.tmLanguage.json")
        );
        let resolved2 = resolve_pkg_path(root, "themes/dark.json");
        assert_eq!(resolved2, Path::new("/tmp/extension/themes/dark.json"));
    }

    #[test]
    fn test_filters_non_textmate_grammars() {
        // JSON Language Grammar files (.json that aren't .tmLanguage.json) should be skipped.
        let json = r#"{
  "publisher": "ms",
  "name": "json",
  "version": "1.0.0",
  "contributes": {
    "languages": [],
    "grammars": [
      { "language": "json", "scopeName": "source.json", "path": "./syntaxes/JSON.tmLanguage" },
      { "language": "jsonc", "scopeName": "source.json.comments", "path": "./syntaxes/JSONC.tmLanguage.json" },
      { "language": "jsonl", "scopeName": "source.jsonl", "path": "./syntaxes/jsonl.lang.json" }
    ],
    "themes": []
  }
}"#;
        let pkg = parse_package_json(json).unwrap();
        // .tmLanguage and .tmLanguage.json pass; .lang.json does not
        assert_eq!(pkg.grammars.len(), 2);
        assert_eq!(pkg.grammars[0].scope_name, "source.json");
        assert_eq!(pkg.grammars[1].scope_name, "source.json.comments");
    }

    #[test]
    fn test_import_vsix_invalid_path() {
        let result = import_vsix(Path::new("/nonexistent/file.vsix"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("file not found"));
    }
}
