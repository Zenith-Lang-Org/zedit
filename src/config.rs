use crate::syntax::json_parser::JsonValue;

// ── Language definition ─────────────────────────────────────

pub struct LanguageDef {
    pub name: String,
    pub extensions: Vec<String>,
    pub grammar_file: String,
    pub comment: Option<String>,
}

pub fn builtin_languages() -> Vec<LanguageDef> {
    vec![
        LanguageDef {
            name: "rust".into(),
            extensions: vec!["rs".into()],
            grammar_file: "rust.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
        LanguageDef {
            name: "javascript".into(),
            extensions: vec!["js".into(), "mjs".into(), "cjs".into()],
            grammar_file: "javascript.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
        LanguageDef {
            name: "typescript".into(),
            extensions: vec!["ts".into(), "tsx".into()],
            grammar_file: "typescript.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
        LanguageDef {
            name: "python".into(),
            extensions: vec!["py".into(), "pyw".into(), "pyi".into()],
            grammar_file: "python.tmLanguage.json".into(),
            comment: Some("#".into()),
        },
        LanguageDef {
            name: "c".into(),
            extensions: vec!["c".into(), "h".into()],
            grammar_file: "c.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
        LanguageDef {
            name: "cpp".into(),
            extensions: vec![
                "cpp".into(),
                "cc".into(),
                "cxx".into(),
                "hpp".into(),
                "hxx".into(),
                "hh".into(),
            ],
            grammar_file: "cpp.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
        LanguageDef {
            name: "php".into(),
            extensions: vec!["php".into(), "phtml".into()],
            grammar_file: "php.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
        LanguageDef {
            name: "json".into(),
            extensions: vec!["json".into(), "jsonc".into()],
            grammar_file: "json.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
        LanguageDef {
            name: "toml".into(),
            extensions: vec!["toml".into()],
            grammar_file: "toml.tmLanguage.json".into(),
            comment: Some("#".into()),
        },
        LanguageDef {
            name: "markdown".into(),
            extensions: vec!["md".into(), "markdown".into()],
            grammar_file: "markdown.tmLanguage.json".into(),
            comment: None,
        },
        LanguageDef {
            name: "shell".into(),
            extensions: vec!["sh".into(), "bash".into(), "zsh".into()],
            grammar_file: "shell.tmLanguage.json".into(),
            comment: Some("#".into()),
        },
        LanguageDef {
            name: "html".into(),
            extensions: vec!["html".into(), "htm".into()],
            grammar_file: "html.tmLanguage.json".into(),
            comment: Some("<!--".into()),
        },
        LanguageDef {
            name: "css".into(),
            extensions: vec!["css".into()],
            grammar_file: "css.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
        LanguageDef {
            name: "zenith".into(),
            extensions: vec!["zl".into()],
            grammar_file: "zenith.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
        LanguageDef {
            name: "zymbol".into(),
            extensions: vec!["zy".into()],
            grammar_file: "zymbol.tmLanguage.json".into(),
            comment: Some("//".into()),
        },
    ]
}

fn parse_languages(val: &JsonValue) -> Option<Vec<LanguageDef>> {
    let arr = val.get("languages")?.as_array()?;
    let mut langs = Vec::new();
    for item in arr {
        let name = item.get("name")?.as_str()?.to_string();
        let extensions: Vec<String> = item
            .get("extensions")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        let grammar_file = item.get("grammar")?.as_str()?.to_string();
        if name.is_empty() || extensions.is_empty() || grammar_file.is_empty() {
            continue;
        }
        let comment = item
            .get("comment")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        langs.push(LanguageDef {
            name,
            extensions,
            grammar_file,
            comment,
        });
    }
    Some(langs)
}

fn merge_languages(user: Vec<LanguageDef>, mut builtins: Vec<LanguageDef>) -> Vec<LanguageDef> {
    let mut result: Vec<LanguageDef> = Vec::new();
    // User entries override built-ins by name
    for entry in user {
        // Remove any matching built-in
        builtins.retain(|b| b.name != entry.name);
        result.push(entry);
    }
    // Append remaining built-ins
    result.extend(builtins);
    result
}

// ── Config ──────────────────────────────────────────────────

pub struct Config {
    pub tab_size: usize,
    pub use_spaces: bool,
    pub theme: String,
    pub line_numbers: bool,
    pub auto_indent: bool,
    pub languages: Vec<LanguageDef>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tab_size: 4,
            use_spaces: true,
            theme: "zedit-dark".to_string(),
            line_numbers: true,
            auto_indent: true,
            languages: builtin_languages(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let home = match std::env::var("HOME") {
            Ok(h) => h,
            Err(_) => return Config::default(),
        };
        let path = format!("{}/.config/zedit/config.json", home);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Config::default(),
        };
        let val = match JsonValue::parse(&content) {
            Ok(v) => v,
            Err(_) => return Config::default(),
        };
        let mut config = Config::default();
        if let Some(n) = val.get("tab_size").and_then(|v| v.as_f64()) {
            config.tab_size = (n as usize).clamp(1, 16);
        }
        if let Some(b) = val.get("use_spaces").and_then(|v| v.as_bool()) {
            config.use_spaces = b;
        }
        if let Some(s) = val.get("theme").and_then(|v| v.as_str()) {
            config.theme = s.to_string();
        }
        if let Some(b) = val.get("line_numbers").and_then(|v| v.as_bool()) {
            config.line_numbers = b;
        }
        if let Some(b) = val.get("auto_indent").and_then(|v| v.as_bool()) {
            config.auto_indent = b;
        }
        if let Some(user_langs) = parse_languages(&val) {
            config.languages = merge_languages(user_langs, builtin_languages());
        }
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let config = Config::default();
        assert_eq!(config.tab_size, 4);
        assert_eq!(config.theme, "zedit-dark");
        assert!(config.line_numbers);
        assert!(config.auto_indent);
    }

    #[test]
    fn test_default_languages_count() {
        let config = Config::default();
        assert_eq!(config.languages.len(), 15);
    }

    #[test]
    fn test_parse_partial() {
        let json = r#"{"tab_size": 2, "line_numbers": false}"#;
        let val = JsonValue::parse(json).unwrap();
        let mut config = Config::default();
        if let Some(n) = val.get("tab_size").and_then(|v| v.as_f64()) {
            config.tab_size = (n as usize).clamp(1, 16);
        }
        if let Some(b) = val.get("line_numbers").and_then(|v| v.as_bool()) {
            config.line_numbers = b;
        }
        assert_eq!(config.tab_size, 2);
        assert!(!config.line_numbers);
        assert_eq!(config.theme, "zedit-dark");
        assert!(config.auto_indent);
    }

    #[test]
    fn test_clamp_tab_size() {
        let json = r#"{"tab_size": 100}"#;
        let val = JsonValue::parse(json).unwrap();
        let n = val.get("tab_size").and_then(|v| v.as_f64()).unwrap();
        let clamped = (n as usize).clamp(1, 16);
        assert_eq!(clamped, 16);

        let json = r#"{"tab_size": 0}"#;
        let val = JsonValue::parse(json).unwrap();
        let n = val.get("tab_size").and_then(|v| v.as_f64()).unwrap();
        let clamped = (n as usize).clamp(1, 16);
        assert_eq!(clamped, 1);
    }

    #[test]
    fn test_invalid_json_returns_defaults() {
        let config = Config::default();
        assert_eq!(config.tab_size, 4);
        assert!(config.line_numbers);
    }

    #[test]
    fn test_user_override_replaces_builtin() {
        let json = r#"{"languages": [{"name": "rust", "extensions": ["rs", "rsx"], "grammar": "rust.tmLanguage.json", "comment": "//"}]}"#;
        let val = JsonValue::parse(json).unwrap();
        let user_langs = parse_languages(&val).unwrap();
        let merged = merge_languages(user_langs, builtin_languages());
        let rust = merged.iter().find(|l| l.name == "rust").unwrap();
        assert_eq!(rust.extensions, vec!["rs", "rsx"]);
        // Other built-ins still present
        assert!(merged.iter().any(|l| l.name == "python"));
        assert_eq!(merged.len(), 15); // replaced, not duplicated
    }

    #[test]
    fn test_user_adds_new_language() {
        let json = r##"{"languages": [{"name": "ruby", "extensions": ["rb", "rake"], "grammar": "ruby.tmLanguage.json", "comment": "#"}]}"##;
        let val = JsonValue::parse(json).unwrap();
        let user_langs = parse_languages(&val).unwrap();
        let merged = merge_languages(user_langs, builtin_languages());
        assert!(merged.iter().any(|l| l.name == "ruby"));
        assert_eq!(merged.len(), 16); // 15 built-in + 1 new
    }

    #[test]
    fn test_no_languages_key_uses_builtins() {
        let json = r#"{"tab_size": 2}"#;
        let val = JsonValue::parse(json).unwrap();
        assert!(parse_languages(&val).is_none());
    }
}
