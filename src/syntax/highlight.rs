/// Bridges grammar, tokenizer, and theme into a per-buffer highlighter.
/// Owns cached line states for incremental re-tokenization.
use std::path::Path;

use crate::config::LanguageDef;
use crate::render::Color;
use crate::syntax::grammar::Grammar;
use crate::syntax::json_parser;
use crate::syntax::theme::Theme;
use crate::syntax::tokenizer::{LineState, Tokenizer};

// ── Types ────────────────────────────────────────────────────

pub struct StyledSpan {
    pub start: usize, // byte offset in line
    pub end: usize,
    pub fg: Color,
    pub bold: bool,
}

pub struct Highlighter {
    pub grammar: Grammar,
    pub theme: Theme,
    lang: Option<String>,
    line_states: Vec<LineState>, // cached state *after* each line
    valid_until: usize,          // lines valid up to (exclusive)
}

// ── Highlighter ──────────────────────────────────────────────

impl Highlighter {
    pub fn new(grammar: Grammar, theme: Theme) -> Self {
        Highlighter {
            grammar,
            theme,
            lang: None,
            line_states: Vec::new(),
            valid_until: 0,
        }
    }

    pub fn with_lang(mut self, lang: &str) -> Self {
        self.lang = Some(lang.to_string());
        self
    }

    pub fn language(&self) -> Option<&str> {
        self.lang.as_deref()
    }

    /// Invalidate cached states from the given line onward.
    pub fn invalidate_from(&mut self, line: usize) {
        if line < self.valid_until {
            self.valid_until = line;
        }
        self.line_states.truncate(line);
    }

    /// Tokenize and style a single line. Builds up cached line states
    /// incrementally if needed by requesting line text via the callback.
    pub fn style_line<F>(&mut self, line: usize, text: &str, mut get_line: F) -> Vec<StyledSpan>
    where
        F: FnMut(usize) -> Option<String>,
    {
        // Ensure all lines up to `line` are tokenized
        let tokenizer = Tokenizer::new(&self.grammar);

        while self.valid_until < line {
            let state = if self.valid_until == 0 {
                LineState::initial()
            } else {
                self.line_states[self.valid_until - 1].clone()
            };

            if let Some(prev_text) = get_line(self.valid_until) {
                let (_, new_state) = tokenizer.tokenize_line(&prev_text, &state);
                if self.valid_until < self.line_states.len() {
                    self.line_states[self.valid_until] = new_state;
                } else {
                    self.line_states.push(new_state);
                }
            } else {
                // Line doesn't exist; push initial state
                if self.valid_until < self.line_states.len() {
                    self.line_states[self.valid_until] = LineState::initial();
                } else {
                    self.line_states.push(LineState::initial());
                }
            }
            self.valid_until += 1;
        }

        // Get the state for the start of this line
        let state = if line == 0 {
            LineState::initial()
        } else if line - 1 < self.line_states.len() {
            self.line_states[line - 1].clone()
        } else {
            LineState::initial()
        };

        // Tokenize the current line
        let (tokens, new_state) = tokenizer.tokenize_line(text, &state);

        // Cache state after this line
        if line < self.line_states.len() {
            self.line_states[line] = new_state;
        } else {
            // Extend to fill gaps
            while self.line_states.len() < line {
                self.line_states.push(LineState::initial());
            }
            self.line_states.push(new_state);
        }
        if self.valid_until <= line {
            self.valid_until = line + 1;
        }

        // Map tokens to styled spans via theme
        tokens
            .iter()
            .map(|tok| {
                let style = self.theme.resolve(&tok.scopes);
                StyledSpan {
                    start: tok.start,
                    end: tok.end,
                    fg: style.fg,
                    bold: style.bold,
                }
            })
            .collect()
    }
}

// ── Language detection ───────────────────────────────────────

pub fn detect_language(path: &Path, languages: &[LanguageDef]) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    for lang in languages {
        if lang.extensions.iter().any(|e| e == ext) {
            return Some(lang.name.clone());
        }
    }
    None
}

/// Load a grammar for the given language key.
pub fn load_grammar(lang: &str, languages: &[LanguageDef]) -> Option<Grammar> {
    let lang_def = languages.iter().find(|l| l.name == lang)?;
    let grammar_file = &lang_def.grammar_file;

    // Try user config directory first
    if let Ok(home) = std::env::var("HOME") {
        let path = format!("{}/.config/zedit/grammars/{}", home, grammar_file);
        if let Ok(json_str) = std::fs::read_to_string(&path)
            && let Some(grammar) = json_parser::JsonValue::parse(&json_str)
                .ok()
                .and_then(|val| Grammar::from_json(&val).ok())
        {
            return Some(grammar);
        }
    }

    // Fall back to built-in embedded grammars (generated by build.rs)
    let json_str = crate::config::builtin_grammar_str(grammar_file)?;
    let val = json_parser::JsonValue::parse(json_str).ok()?;
    Grammar::from_json(&val).ok()
}

/// Discover user grammars from ~/.config/zedit/grammars/.
/// For each .tmLanguage.json file, parse it to extract name and fileTypes,
/// then auto-register as a language definition.
pub fn discover_user_grammars(home: &str) -> Vec<LanguageDef> {
    let grammars_dir = format!("{}/.config/zedit/grammars", home);
    let entries = match std::fs::read_dir(&grammars_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut discovered = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.ends_with(".tmLanguage.json") {
            continue;
        }

        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let val = match json_parser::JsonValue::parse(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Extract name from grammar
        let name = match val.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_lowercase(),
            None => continue,
        };

        // Extract fileTypes for extensions
        let extensions: Vec<String> = val
            .get("fileTypes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        if extensions.is_empty() {
            continue;
        }

        discovered.push(LanguageDef {
            name,
            extensions,
            grammar_file: file_name,
            comment: None,
        });
    }

    discovered
}

/// Load a theme by name.
/// Searches: ~/.config/zedit/themes/{name}.json → built-in embedded → default.
pub fn load_theme(theme_name: &str) -> Theme {
    // Try user config directory first
    if let Ok(home) = std::env::var("HOME") {
        let path = format!("{}/.config/zedit/themes/{}.json", home, theme_name);
        if let Ok(json_str) = std::fs::read_to_string(&path)
            && let Some(theme) = json_parser::JsonValue::parse(&json_str)
                .ok()
                .and_then(|val| Theme::from_json(&val).ok())
        {
            return theme;
        }
    }
    // Fall back to built-in embedded themes
    let json_str = match theme_name {
        "zedit-dark" => Some(include_str!("../../themes/zedit-dark.json")),
        "zedit-light" => Some(include_str!("../../themes/zedit-light.json")),
        _ => None,
    };
    if let Some(json_str) = json_str
        && let Some(theme) = json_parser::JsonValue::parse(json_str)
            .ok()
            .and_then(|val| Theme::from_json(&val).ok())
    {
        return theme;
    }
    Theme::default_theme()
}

// ── Comment prefix lookup ────────────────────────────────────

pub fn comment_prefix(lang: &str, languages: &[LanguageDef]) -> Option<String> {
    languages
        .iter()
        .find(|l| l.name == lang)
        .and_then(|l| l.comment.clone())
}

// ── Span lookup helper ──────────────────────────────────────

/// Find the style for a byte offset within a list of styled spans.
pub fn lookup_style(spans: &[StyledSpan], byte_offset: usize) -> (Color, Color, bool) {
    for span in spans {
        if byte_offset >= span.start && byte_offset < span.end && span.fg != Color::Default {
            return (span.fg, Color::Default, span.bold);
        }
    }
    (Color::Default, Color::Default, false)
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::builtin_languages;

    #[test]
    fn test_detect_language() {
        let langs = builtin_languages();
        assert_eq!(
            detect_language(Path::new("main.rs"), &langs).as_deref(),
            Some("rust")
        );
        assert_eq!(
            detect_language(Path::new("app.js"), &langs).as_deref(),
            Some("javascript")
        );
        assert_eq!(detect_language(Path::new("file.txt"), &langs), None);
        assert_eq!(detect_language(Path::new("noext"), &langs), None);
    }

    #[test]
    fn test_load_rust_grammar() {
        let langs = builtin_languages();
        let g = load_grammar("rust", &langs);
        assert!(g.is_some());
        let g = g.unwrap();
        assert_eq!(g.scope_name, "source.rust");
    }

    #[test]
    fn test_load_theme() {
        let t = load_theme("zedit-dark");
        assert_eq!(t.name, "Zedit Dark");
        assert!(!t.token_rules.is_empty());
    }

    #[test]
    fn test_style_line_keyword() {
        let langs = builtin_languages();
        let grammar = load_grammar("rust", &langs).unwrap();
        let theme = load_theme("zedit-dark");
        let mut hl = Highlighter::new(grammar, theme);
        let spans = hl.style_line(0, "fn main() {", |_| None);
        // "fn" should be highlighted as keyword
        assert!(!spans.is_empty());
        // Find the span covering "fn" (bytes 0..2)
        let fn_span = spans.iter().find(|s| s.start == 0 && s.end <= 3);
        assert!(fn_span.is_some(), "Should have a span for 'fn'");
        let fn_span = fn_span.unwrap();
        assert_ne!(fn_span.fg, Color::Default, "'fn' should be colored");
    }

    #[test]
    fn test_invalidate_from() {
        let langs = builtin_languages();
        let grammar = load_grammar("rust", &langs).unwrap();
        let theme = load_theme("zedit-dark");
        let mut hl = Highlighter::new(grammar, theme);
        // Tokenize a few lines
        hl.style_line(0, "fn main() {", |_| None);
        hl.style_line(1, "    let x = 5;", |l| {
            if l == 0 {
                Some("fn main() {".to_string())
            } else {
                None
            }
        });
        assert!(hl.valid_until >= 2);
        hl.invalidate_from(1);
        assert!(hl.valid_until <= 1);
        assert!(hl.line_states.len() <= 1);
    }

    #[test]
    fn test_block_comment_multiline() {
        let langs = builtin_languages();
        let grammar = load_grammar("rust", &langs).unwrap();
        let theme = load_theme("zedit-dark");
        let mut hl = Highlighter::new(grammar, theme);

        let lines = vec!["/* this is", "   a block comment */", "fn test() {}"];

        // Style line 0
        let spans0 = hl.style_line(0, lines[0], |_| None);
        assert!(!spans0.is_empty());

        // Style line 1 (continuation of block comment)
        let spans1 = hl.style_line(1, lines[1], |l| {
            if l == 0 {
                Some(lines[0].to_string())
            } else {
                None
            }
        });
        assert!(!spans1.is_empty());

        // Style line 2 (after block comment ends)
        let spans2 = hl.style_line(2, lines[2], |l| Some(lines[l].to_string()));
        // "fn" should be keyword-colored, not comment-colored
        let fn_span = spans2.iter().find(|s| s.start == 0);
        assert!(fn_span.is_some());
    }

    #[test]
    fn test_lookup_style() {
        let spans = vec![
            StyledSpan {
                start: 0,
                end: 2,
                fg: Color::Rgb(200, 100, 50),
                bold: true,
            },
            StyledSpan {
                start: 3,
                end: 7,
                fg: Color::Default,
                bold: false,
            },
        ];
        let (fg, _, bold) = lookup_style(&spans, 0);
        assert_eq!(fg, Color::Rgb(200, 100, 50));
        assert!(bold);

        // Default span should return default
        let (fg, _, _) = lookup_style(&spans, 4);
        assert_eq!(fg, Color::Default);

        // Out of range
        let (fg, _, _) = lookup_style(&spans, 10);
        assert_eq!(fg, Color::Default);
    }

    #[test]
    fn test_discover_user_grammars_empty_dir() {
        // Non-existent directory should return empty vec
        let result = discover_user_grammars("/tmp/nonexistent_zedit_test_dir");
        assert!(result.is_empty());
    }

    #[test]
    fn test_discover_user_grammars_with_file() {
        let tmp = std::env::temp_dir().join("zedit_test_discover");
        let grammars_dir = tmp.join(".config/zedit/grammars");
        std::fs::create_dir_all(&grammars_dir).unwrap();

        // Write a minimal grammar
        let grammar = r#"{
            "name": "TestLang",
            "scopeName": "source.testlang",
            "fileTypes": ["tl", "tlx"],
            "patterns": []
        }"#;
        std::fs::write(grammars_dir.join("testlang.tmLanguage.json"), grammar).unwrap();

        let result = discover_user_grammars(tmp.to_str().unwrap());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "testlang");
        assert_eq!(result[0].extensions, vec!["tl", "tlx"]);
        assert_eq!(result[0].grammar_file, "testlang.tmLanguage.json");

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
