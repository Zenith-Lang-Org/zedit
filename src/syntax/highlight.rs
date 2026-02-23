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
    /// True when this span is inside a `string.*` or `comment.*` TextMate scope.
    /// Used to prevent LSP semantic tokens from overriding the correct contextual color.
    pub is_string_or_comment: bool,
}

/// A pre-resolved semantic token span for a single buffer line.
/// Colors are resolved at store time; spans are sorted by (line, start_char).
pub struct SemanticSpan {
    pub line: u32,
    pub start_char: u32, // inclusive (character index, 0-based)
    pub end_char: u32,   // exclusive
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
                let is_string_or_comment = tok
                    .scopes
                    .iter()
                    .any(|s| s.starts_with("string") || s.starts_with("comment"));
                StyledSpan {
                    start: tok.start,
                    end: tok.end,
                    fg: style.fg,
                    bold: style.bold,
                    is_string_or_comment,
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

    for dir in grammar_search_dirs() {
        let path = dir.join(grammar_file);
        if let Ok(json_str) = std::fs::read_to_string(&path) {
            if let Some(grammar) = json_parser::JsonValue::parse(&json_str)
                .ok()
                .and_then(|val| Grammar::from_json(&val).ok())
            {
                return Some(grammar);
            }
        }
    }
    None // no grammar found → plain text mode, no crash
}

/// Ordered directories to search for .tmLanguage.json grammar files.
/// Priority: extensions → user config → system-wide → dev mode (CWD/grammars).
fn grammar_search_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        // 1. Installed extension directories (highest priority).
        let ext_base = std::path::PathBuf::from(format!("{}/.config/zedit/extensions", home));
        if let Ok(entries) = std::fs::read_dir(&ext_base) {
            let mut ext_dirs: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path())
                .collect();
            ext_dirs.sort(); // deterministic order
            dirs.extend(ext_dirs);
        }
        // 2. Legacy user grammars directory.
        dirs.push(std::path::PathBuf::from(format!(
            "{}/.config/zedit/grammars",
            home
        )));
    }
    dirs.push(std::path::PathBuf::from("/usr/share/zedit/grammars"));
    dirs.push(std::path::PathBuf::from("/usr/local/share/zedit/grammars"));
    dirs.push(std::path::PathBuf::from("grammars")); // dev mode / source tree
    dirs
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

        // Derive the language name from the filename stem (e.g. "zenith.tmLanguage.json"
        // → "zenith"). This is correct because the TextMate "name" field is a human-readable
        // display name (e.g. "Zenith-Lang"), not a stable language identifier. The filename
        // always matches what languages.json and the task runner expect.
        let name = file_name
            .strip_suffix(".tmLanguage.json")
            .unwrap_or(&file_name)
            .to_lowercase();

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

/// Returns true if the byte offset falls inside a string or comment span.
/// Used to prevent LSP semantic tokens from overriding TextMate's context-aware coloring
/// (e.g., identifiers inside a string literal must stay green, not turn keyword purple).
pub fn is_in_string_or_comment(spans: &[StyledSpan], byte_offset: usize) -> bool {
    for span in spans {
        if byte_offset >= span.start && byte_offset < span.end {
            return span.is_string_or_comment;
        }
    }
    false
}

/// Find the color for (file_line, char_col) in pre-resolved semantic spans.
/// Spans must be sorted by (line, start_char) — guaranteed by LSP delta order.
pub fn lookup_semantic_span(
    spans: &[SemanticSpan],
    line: u32,
    char_col: u32,
) -> Option<(Color, bool)> {
    let start = spans.partition_point(|s| s.line < line);
    for s in &spans[start..] {
        if s.line > line {
            break;
        }
        if char_col >= s.start_char && char_col < s.end_char && s.fg != Color::Default {
            return Some((s.fg, s.bold));
        }
    }
    None
}

/// Map a semantic token type name (from LSP legend) to a TextMate scope prefix.
/// Used to resolve colors via the active theme without hardcoding hex values.
pub fn semantic_token_scope(token_type: &str) -> &'static str {
    match token_type {
        "keyword" => "keyword.control",
        "type" => "entity.name.type",
        "variable" => "variable.other",
        "function" => "entity.name.function",
        "struct" => "entity.name.type.struct",
        "property" => "variable.other.property",
        "number" => "constant.numeric",
        "string" => "string.quoted",
        "comment" => "comment",
        "operator" => "keyword.operator",
        "directive" => "entity.name.tag",   // .:ES:., .:EN:.
        "constraint" => "storage.modifier", // PRIMARY-KEY, UNIQUE
        "boolean" => "constant.language",
        // standard VSCode extras (forward-compat)
        "namespace" => "entity.name.namespace",
        "class" => "entity.name.type",
        "enum" => "entity.name.type",
        "interface" => "entity.name.type",
        "parameter" => "variable.parameter",
        "enumMember" => "variable.other.enummember",
        "method" => "entity.name.function",
        "macro" => "entity.name.function.macro",
        "decorator" => "entity.name.function",
        _ => "",
    }
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
                is_string_or_comment: false,
            },
            StyledSpan {
                start: 3,
                end: 7,
                fg: Color::Default,
                bold: false,
                is_string_or_comment: false,
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
    fn test_is_in_string_or_comment() {
        // Simulates: 'HELLO' where the whole span (including delimiters) has string scope.
        // LSP sends a "variable" token for HELLO (chars 1-5 inside the string).
        // The render loop should NOT apply the semantic token there.
        let spans = vec![
            StyledSpan {
                start: 0,
                end: 7, // 'HELLO'  (7 bytes)
                fg: Color::Rgb(166, 227, 161), // green string color
                bold: false,
                is_string_or_comment: true, // ← the whole region is protected
            },
        ];
        // All bytes in the string are protected
        assert!(is_in_string_or_comment(&spans, 0)); // opening quote
        assert!(is_in_string_or_comment(&spans, 1)); // H
        assert!(is_in_string_or_comment(&spans, 5)); // O
        assert!(is_in_string_or_comment(&spans, 6)); // closing quote

        // Outside the string — not protected
        assert!(!is_in_string_or_comment(&spans, 7));
        assert!(!is_in_string_or_comment(&[], 0));

        // Comment region is also protected
        let comment_spans = vec![StyledSpan {
            start: 0,
            end: 20,
            fg: Color::Color256(240),
            bold: false,
            is_string_or_comment: true,
        }];
        assert!(is_in_string_or_comment(&comment_spans, 10));

        // Non-string span (keyword) is NOT protected
        let keyword_spans = vec![StyledSpan {
            start: 0,
            end: 2,
            fg: Color::Rgb(203, 166, 247), // purple keyword
            bold: true,
            is_string_or_comment: false,
        }];
        assert!(!is_in_string_or_comment(&keyword_spans, 1));
    }

    // ── lookup_semantic_span ─────────────────────────────────

    fn make_semantic_spans() -> Vec<SemanticSpan> {
        vec![
            // Line 0: ".:ES:." directive (chars 0-5)
            SemanticSpan {
                line: 0,
                start_char: 0,
                end_char: 6,
                fg: Color::Rgb(100, 200, 255),
                bold: false,
            },
            // Line 0: "SI" keyword (chars 7-8)
            SemanticSpan {
                line: 0,
                start_char: 7,
                end_char: 9,
                fg: Color::Rgb(255, 200, 0),
                bold: true,
            },
            // Line 1: "miVar" variable (chars 0-4)
            SemanticSpan {
                line: 1,
                start_char: 0,
                end_char: 5,
                fg: Color::Rgb(200, 200, 200),
                bold: false,
            },
            // Line 3: "FIN-SI" keyword (chars 0-5)
            SemanticSpan {
                line: 3,
                start_char: 0,
                end_char: 6,
                fg: Color::Rgb(255, 200, 0),
                bold: true,
            },
        ]
    }

    #[test]
    fn test_lookup_semantic_span_hit_directive() {
        let spans = make_semantic_spans();
        // char 0 on line 0 → inside ".:ES:." directive (chars 0-5)
        let result = lookup_semantic_span(&spans, 0, 0);
        assert!(result.is_some());
        let (fg, bold) = result.unwrap();
        assert_eq!(fg, Color::Rgb(100, 200, 255));
        assert!(!bold);
    }

    #[test]
    fn test_lookup_semantic_span_hit_keyword() {
        let spans = make_semantic_spans();
        // char 7 on line 0 → "SI" keyword
        let result = lookup_semantic_span(&spans, 0, 7);
        assert!(result.is_some());
        let (fg, bold) = result.unwrap();
        assert_eq!(fg, Color::Rgb(255, 200, 0));
        assert!(bold);
    }

    #[test]
    fn test_lookup_semantic_span_gap_returns_none() {
        let spans = make_semantic_spans();
        // char 6 on line 0 → the space between ".:ES:." and "SI" — no span
        let result = lookup_semantic_span(&spans, 0, 6);
        assert!(result.is_none(), "gap between tokens should return None");
    }

    #[test]
    fn test_lookup_semantic_span_end_exclusive() {
        let spans = make_semantic_spans();
        // end_char=6, so char 6 is exclusive (not in the span)
        let result = lookup_semantic_span(&spans, 0, 6);
        assert!(result.is_none());
        // char 5 is the last included char (0-5 inclusive, end=6 exclusive)
        let result = lookup_semantic_span(&spans, 0, 5);
        assert!(result.is_some());
    }

    #[test]
    fn test_lookup_semantic_span_different_line() {
        let spans = make_semantic_spans();
        // line 1, char 2 → inside "miVar" (chars 0-4)
        let result = lookup_semantic_span(&spans, 1, 2);
        assert!(result.is_some());
        let (fg, _) = result.unwrap();
        assert_eq!(fg, Color::Rgb(200, 200, 200));
    }

    #[test]
    fn test_lookup_semantic_span_skips_to_correct_line() {
        let spans = make_semantic_spans();
        // line 2 has no spans at all
        let result = lookup_semantic_span(&spans, 2, 0);
        assert!(result.is_none(), "line 2 has no semantic spans");

        // line 3 has "FIN-SI"
        let result = lookup_semantic_span(&spans, 3, 0);
        assert!(result.is_some());
    }

    #[test]
    fn test_lookup_semantic_span_empty_list() {
        let result = lookup_semantic_span(&[], 0, 0);
        assert!(result.is_none());
    }

    #[test]
    fn test_lookup_semantic_span_default_color_invisible() {
        // Spans with Color::Default fg should NOT be returned (they are "no color")
        let spans = vec![SemanticSpan {
            line: 0,
            start_char: 0,
            end_char: 5,
            fg: Color::Default,
            bold: false,
        }];
        let result = lookup_semantic_span(&spans, 0, 2);
        assert!(
            result.is_none(),
            "Default fg spans should be skipped like invisible tokens"
        );
    }

    // ── semantic_token_scope ─────────────────────────────────

    #[test]
    fn test_semantic_token_scope_zenith_types() {
        assert_eq!(semantic_token_scope("keyword"), "keyword.control");
        assert_eq!(semantic_token_scope("type"), "entity.name.type");
        assert_eq!(semantic_token_scope("variable"), "variable.other");
        assert_eq!(semantic_token_scope("function"), "entity.name.function");
        assert_eq!(semantic_token_scope("directive"), "entity.name.tag");
        assert_eq!(semantic_token_scope("constraint"), "storage.modifier");
        assert_eq!(semantic_token_scope("boolean"), "constant.language");
        assert_eq!(semantic_token_scope("number"), "constant.numeric");
        assert_eq!(semantic_token_scope("string"), "string.quoted");
        assert_eq!(semantic_token_scope("comment"), "comment");
        assert_eq!(semantic_token_scope("operator"), "keyword.operator");
    }

    #[test]
    fn test_semantic_token_scope_standard_extras() {
        assert_eq!(semantic_token_scope("namespace"), "entity.name.namespace");
        assert_eq!(semantic_token_scope("parameter"), "variable.parameter");
        assert_eq!(semantic_token_scope("method"), "entity.name.function");
        assert_eq!(semantic_token_scope("macro"), "entity.name.function.macro");
    }

    #[test]
    fn test_semantic_token_scope_unknown_returns_empty() {
        assert_eq!(semantic_token_scope("nonexistent_type"), "");
        assert_eq!(semantic_token_scope(""), "");
    }

    #[test]
    fn test_semantic_token_scope_resolves_via_theme() {
        // Verify that the scope strings actually produce colors in the built-in theme
        let theme = load_theme("zedit-dark");
        let keyword_scope = semantic_token_scope("keyword");
        let style = theme.resolve(&[keyword_scope.to_string()]);
        assert_ne!(
            style.fg,
            Color::Default,
            "keyword scope should resolve to a color in zedit-dark"
        );

        // Directive scope (entity.name.tag) now has a teal color rule
        let directive_scope = semantic_token_scope("directive");
        let style = theme.resolve(&[directive_scope.to_string()]);
        assert_ne!(
            style.fg,
            Color::Default,
            "entity.name.tag (directive) should resolve to a color in zedit-dark"
        );

        // Operator scope now has a distinct color
        let op_scope = semantic_token_scope("operator");
        let style = theme.resolve(&[op_scope.to_string()]);
        assert_ne!(
            style.fg,
            Color::Default,
            "keyword.operator should resolve to a color in zedit-dark"
        );
    }

    #[test]
    fn test_zenith_string_is_protected() {
        // Verifies that a Zenith single-quoted string produces spans with
        // is_string_or_comment=true, which prevents LSP semantic tokens from
        // overriding the green string color with keyword/variable colors.
        let langs = builtin_languages();
        let grammar = match load_grammar("zenith", &langs) {
            Some(g) => g,
            None => return, // grammar file not available in this test environment
        };
        let theme = load_theme("zedit-dark");
        let mut hl = Highlighter::new(grammar, theme);

        let line = "'HELLO MUNDO'";
        let spans = hl.style_line(0, line, |_| None);

        // Every byte inside the string literal (including delimiters) must be protected
        for (i, _) in line.char_indices() {
            assert!(
                is_in_string_or_comment(&spans, i),
                "byte {i} inside Zenith string literal should be protected from LSP override"
            );
        }
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
