/// VS Code-compatible theme loader with scope hierarchy matching.
/// Parses theme JSON files and resolves scope selectors to styled colors.
use crate::render::Color;
use crate::syntax::json_parser::JsonValue;

// ── Types ────────────────────────────────────────────────────

pub struct Theme {
    #[allow(dead_code)]
    pub name: String,
    pub token_rules: Vec<TokenRule>,
    /// Background color parsed from `colors["editor.background"]`.
    /// Used by `ensure_readable_contrast` to correct low-contrast token colors.
    pub background: (u8, u8, u8),
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

        let background = json
            .get("colors")
            .and_then(|c| c.get("editor.background"))
            .and_then(|v| v.as_str())
            .and_then(parse_hex_color)
            .and_then(|c| {
                if let Color::Rgb(r, g, b) = c {
                    Some((r, g, b))
                } else {
                    None
                }
            })
            .unwrap_or((30, 30, 46)); // #1e1e2e dark fallback

        Ok(Theme {
            name,
            token_rules,
            background,
        })
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
                TokenRule {
                    scopes: vec!["markup.heading".to_string()],
                    foreground: Some(Color::Color256(141)),
                    bold: true,
                },
                TokenRule {
                    scopes: vec!["entity.name.section".to_string()],
                    foreground: Some(Color::Color256(141)),
                    bold: true,
                },
                TokenRule {
                    scopes: vec!["markup.bold".to_string()],
                    foreground: Some(Color::Color256(215)),
                    bold: true,
                },
                TokenRule {
                    scopes: vec!["markup.italic".to_string()],
                    foreground: Some(Color::Color256(229)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["markup.underline.link".to_string()],
                    foreground: Some(Color::Color256(75)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["markup.raw".to_string()],
                    foreground: Some(Color::Color256(113)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["markup.quote".to_string()],
                    foreground: Some(Color::Color256(246)),
                    bold: false,
                },
                TokenRule {
                    scopes: vec!["markup.list".to_string()],
                    foreground: Some(Color::Color256(79)),
                    bold: false,
                },
            ],
            background: (30, 30, 46), // #1e1e2e dark default
        }
    }
}

// ── Contrast adjustment ──────────────────────────────────────

/// Ensure every `Color::Rgb` foreground in `theme` meets a minimum contrast
/// ratio of 3.0 against `bg` (measured via OKLab lightness).
///
/// Rules with insufficient contrast have their foreground blended toward white
/// (dark background) or black (light background) until the threshold is met.
///
/// Call this once after loading a theme, not on every render frame.
pub fn ensure_readable_contrast(theme: &mut Theme, bg: (u8, u8, u8)) {
    for rule in &mut theme.token_rules {
        if let Some(Color::Rgb(r, g, b)) = rule.foreground {
            let ratio = crate::oklab::contrast_ratio(r, g, b, bg.0, bg.1, bg.2);
            if ratio < 3.0 {
                rule.foreground = Some(adjust_toward_readable(r, g, b, bg));
            }
        }
    }
}

/// Blend `(r, g, b)` toward white or black until it achieves contrast ≥ 3.0
/// against `bg`.  Uses eight linear interpolation steps before falling back
/// to the target pole (pure white or pure black).
fn adjust_toward_readable(r: u8, g: u8, b: u8, bg: (u8, u8, u8)) -> Color {
    let (l_bg, _, _) = crate::oklab::srgb_to_oklab_u8(bg.0, bg.1, bg.2);
    // Dark background → blend toward white; light background → blend toward black.
    let (target_r, target_g, target_b): (u8, u8, u8) = if l_bg < 0.5 {
        (255, 255, 255)
    } else {
        (0, 0, 0)
    };

    for step in 1..=8u8 {
        let t = step as f32 / 8.0;
        let nr = lerp_u8(r, target_r, t);
        let ng = lerp_u8(g, target_g, t);
        let nb = lerp_u8(b, target_b, t);
        if crate::oklab::contrast_ratio(nr, ng, nb, bg.0, bg.1, bg.2) >= 3.0 {
            return Color::Rgb(nr, ng, nb);
        }
    }
    Color::Rgb(target_r, target_g, target_b)
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
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
            background: (30, 30, 46),
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
            background: (30, 30, 46),
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
            background: (30, 30, 46),
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
            background: (30, 30, 46),
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
