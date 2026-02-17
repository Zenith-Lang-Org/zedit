/// VS Code-compatible theme loader with scope hierarchy matching.
/// Parses theme JSON files and resolves scope selectors to styled colors.
use crate::render::Color;
use crate::syntax::json_parser::JsonValue;

// ── Types ────────────────────────────────────────────────────

#[allow(dead_code)]
pub struct Theme {
    pub name: String,
    pub token_rules: Vec<TokenRule>,
}

pub struct TokenRule {
    pub scopes: Vec<String>,
    pub foreground: Option<Color>,
    pub bold: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStyle {
    pub fg: Color,
    pub bold: bool,
}

// ── Hex parsing ──────────────────────────────────────────────

fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

// ── Theme ────────────────────────────────────────────────────

impl Theme {
    /// Parse a VS Code-compatible theme from JSON.
    pub fn from_json(json: &JsonValue) -> Result<Theme, String> {
        let name = json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled")
            .to_string();

        let mut token_rules = Vec::new();

        if let Some(token_colors) = json.get("tokenColors").and_then(|v| v.as_array()) {
            for entry in token_colors {
                let scopes = parse_scope_field(entry);
                if scopes.is_empty() {
                    continue;
                }

                let settings = match entry.get("settings") {
                    Some(s) => s,
                    None => continue,
                };

                let foreground = settings
                    .get("foreground")
                    .and_then(|v| v.as_str())
                    .and_then(parse_hex_color);

                let bold = settings
                    .get("fontStyle")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s.contains("bold"));

                token_rules.push(TokenRule {
                    scopes,
                    foreground,
                    bold,
                });
            }
        }

        Ok(Theme { name, token_rules })
    }

    /// Resolve scopes to a style using the best-matching rule.
    ///
    /// A selector matches a scope if the scope equals the selector or starts
    /// with the selector followed by `.`. More specific selectors (longer) win.
    pub fn resolve(&self, scopes: &[String]) -> ResolvedStyle {
        let mut best_specificity: usize = 0;
        let mut best_fg = Color::Default;
        let mut best_bold = false;

        for rule in &self.token_rules {
            for selector in &rule.scopes {
                for scope in scopes {
                    if scope_matches(scope, selector) {
                        let specificity = selector.len();
                        if specificity > best_specificity {
                            best_specificity = specificity;
                            best_fg = rule.foreground.unwrap_or(Color::Default);
                            best_bold = rule.bold;
                        }
                    }
                }
            }
        }

        ResolvedStyle {
            fg: best_fg,
            bold: best_bold,
        }
    }

    /// Embedded default theme (hardcoded fallback).
    pub fn default_theme() -> Theme {
        Theme {
            name: "Default".to_string(),
            token_rules: vec![
                TokenRule {
                    scopes: vec!["comment".to_string()],
                    foreground: Some(Color::Color256(242)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["string".to_string()],
                    foreground: Some(Color::Color256(113)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["keyword".to_string()],
                    foreground: Some(Color::Color256(176)),
                    bold: true,
                },
                TokenRule {
                    scopes: vec!["constant.numeric".to_string()],
                    foreground: Some(Color::Color256(215)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["storage.type".to_string()],
                    foreground: Some(Color::Color256(75)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["entity.name.function".to_string()],
                    foreground: Some(Color::Color256(75)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["entity.name.type".to_string()],
                    foreground: Some(Color::Color256(222)),
                    bold: false,
                },
            ],
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────

/// Check if a scope matches a selector.
/// `keyword.control` matches selectors `keyword` and `keyword.control`,
/// but not `keyword.other`.
fn scope_matches(scope: &str, selector: &str) -> bool {
    if scope == selector {
        return true;
    }
    scope.starts_with(selector) && scope.as_bytes().get(selector.len()) == Some(&b'.')
}

/// Parse the "scope" field which can be a string or an array of strings.
fn parse_scope_field(entry: &JsonValue) -> Vec<String> {
    match entry.get("scope") {
        Some(val) => match val {
            JsonValue::String(s) => s
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            JsonValue::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#ff0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_hex_color("#00ff00"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_hex_color("#1e1e2e"), Some(Color::Rgb(30, 30, 46)));
        assert_eq!(parse_hex_color("invalid"), None);
        assert_eq!(parse_hex_color("#fff"), None); // too short
    }

    #[test]
    fn test_default_theme_loads() {
        let theme = Theme::default_theme();
        assert_eq!(theme.name, "Default");
        assert!(!theme.token_rules.is_empty());
    }

    #[test]
    fn test_scope_exact_match() {
        let theme = Theme {
            name: "test".to_string(),
            token_rules: vec![TokenRule {
                scopes: vec!["keyword".to_string()],
                foreground: Some(Color::Rgb(200, 100, 50)),
                bold: true,
            }],
        };
        let style = theme.resolve(&["keyword".to_string()]);
        assert_eq!(style.fg, Color::Rgb(200, 100, 50));
        assert!(style.bold);
    }

    #[test]
    fn test_scope_prefix_match() {
        let theme = Theme {
            name: "test".to_string(),
            token_rules: vec![TokenRule {
                scopes: vec!["keyword".to_string()],
                foreground: Some(Color::Rgb(200, 100, 50)),
                bold: false,
            }],
        };
        let style = theme.resolve(&["keyword.control".to_string()]);
        assert_eq!(style.fg, Color::Rgb(200, 100, 50));
    }

    #[test]
    fn test_specificity_longer_wins() {
        let theme = Theme {
            name: "test".to_string(),
            token_rules: vec![
                TokenRule {
                    scopes: vec!["keyword".to_string()],
                    foreground: Some(Color::Rgb(100, 100, 100)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["keyword.control".to_string()],
                    foreground: Some(Color::Rgb(200, 200, 200)),
                    bold: true,
                },
            ],
        };
        let style = theme.resolve(&["keyword.control".to_string()]);
        assert_eq!(style.fg, Color::Rgb(200, 200, 200));
        assert!(style.bold);
    }

    #[test]
    fn test_no_match_returns_default() {
        let theme = Theme {
            name: "test".to_string(),
            token_rules: vec![TokenRule {
                scopes: vec!["keyword".to_string()],
                foreground: Some(Color::Rgb(200, 100, 50)),
                bold: true,
            }],
        };
        let style = theme.resolve(&["string".to_string()]);
        assert_eq!(style.fg, Color::Default);
        assert!(!style.bold);
    }

    #[test]
    fn test_from_json() {
        use crate::syntax::json_parser;
        let json_str = r##"{
            "name": "Test Theme",
            "tokenColors": [
                {
                    "scope": "comment",
                    "settings": { "foreground": "#aabbcc", "fontStyle": "italic" }
                },
                {
                    "scope": "keyword",
                    "settings": { "foreground": "#112233", "fontStyle": "bold" }
                }
            ]
        }"##;
        let val = json_parser::JsonValue::parse(json_str).unwrap();
        let theme = Theme::from_json(&val).unwrap();
        assert_eq!(theme.name, "Test Theme");
        assert_eq!(theme.token_rules.len(), 2);
        assert_eq!(
            theme.token_rules[0].foreground,
            Some(Color::Rgb(0xaa, 0xbb, 0xcc))
        );
        assert!(!theme.token_rules[0].bold); // italic, not bold
        assert!(theme.token_rules[1].bold);
    }
}
