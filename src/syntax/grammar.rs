/// TextMate grammar loader.
/// Parses `.tmLanguage.json` files (via our custom JSON parser) into
/// compiled `Grammar` structs with `Pattern` trees.
use super::json_parser::JsonValue;
use super::regex::Regex;
use std::sync::atomic::{AtomicUsize, Ordering};

static REGION_ID_COUNTER: AtomicUsize = AtomicUsize::new(0);

// ── Public types ──────────────────────────────────────────────

#[derive(Debug)]
#[allow(dead_code)]
pub struct Grammar {
    pub name: String,
    pub scope_name: String,
    pub file_types: Vec<String>,
    pub patterns: Vec<Pattern>,
    pub repository: Vec<(String, Vec<Pattern>)>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum Pattern {
    Match {
        name: Option<String>,
        regex: Regex,
        captures: Vec<(usize, String)>,
    },
    Region {
        id: usize,
        name: Option<String>,
        content_name: Option<String>,
        begin: Regex,
        end_pattern: String,
        begin_captures: Vec<(usize, String)>,
        end_captures: Vec<(usize, String)>,
        patterns: Vec<Pattern>,
    },
    Include(IncludeTarget),
}

#[derive(Debug)]
pub enum IncludeTarget {
    Repository(String),
    SelfRef,
}

// ── Grammar loading ───────────────────────────────────────────

impl Grammar {
    pub fn from_json(json: &JsonValue) -> Result<Grammar, String> {
        let scope_name = json
            .get("scopeName")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Grammar missing required 'scopeName' field".to_string())?
            .to_string();

        let name = json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&scope_name)
            .to_string();

        let file_types = json
            .get("fileTypes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // Load repository first (patterns may reference it)
        let repository = match json.get("repository").and_then(|v| v.as_object()) {
            Some(pairs) => pairs
                .iter()
                .map(|(key, val)| {
                    let patterns = parse_repository_entry(val);
                    (key.clone(), patterns)
                })
                .collect(),
            None => Vec::new(),
        };

        let patterns = json
            .get("patterns")
            .and_then(|v| v.as_array())
            .map(parse_pattern_array)
            .unwrap_or_default();

        Ok(Grammar {
            name,
            scope_name,
            file_types,
            patterns,
            repository,
        })
    }

    pub fn find_repository(&self, key: &str) -> Option<&[Pattern]> {
        for (k, patterns) in &self.repository {
            if k == key {
                return Some(patterns);
            }
        }
        None
    }

    /// Find the child patterns of a Region by its unique ID.
    /// Searches top-level patterns and all repository entries.
    pub fn find_region_children(&self, id: usize) -> Option<&[Pattern]> {
        fn search_patterns(patterns: &[Pattern], id: usize) -> Option<&[Pattern]> {
            for pat in patterns {
                if let Pattern::Region {
                    id: rid, patterns, ..
                } = pat
                {
                    if *rid == id {
                        return Some(patterns);
                    }
                    // Search nested regions
                    if let Some(found) = search_patterns(patterns, id) {
                        return Some(found);
                    }
                }
            }
            None
        }

        if let Some(found) = search_patterns(&self.patterns, id) {
            return Some(found);
        }
        for (_key, patterns) in &self.repository {
            if let Some(found) = search_patterns(patterns, id) {
                return Some(found);
            }
        }
        None
    }
}

// ── Parsing helpers ───────────────────────────────────────────

fn parse_repository_entry(json: &JsonValue) -> Vec<Pattern> {
    // A repository entry is either:
    // 1. A Region (has "begin") — even if it also has "patterns" (those are child patterns).
    //    Must be checked FIRST; otherwise an entry like "string" that has both "begin"
    //    and "patterns" would be misclassified as a pattern group, causing the Region to
    //    never be created and the begin/end delimiters to be silently ignored.
    // 2. A pattern group (has "patterns" but NO "begin") → expand inline.
    // 3. A single pattern object (has "match" or "include").
    if json.get("begin").is_some() {
        return if let Some(pat) = parse_pattern(json) {
            vec![pat]
        } else {
            Vec::new()
        };
    }
    if let Some(arr) = json.get("patterns").and_then(|v| v.as_array()) {
        parse_pattern_array(arr)
    } else if let Some(pat) = parse_pattern(json) {
        vec![pat]
    } else {
        Vec::new()
    }
}

fn parse_pattern_array(arr: &[JsonValue]) -> Vec<Pattern> {
    arr.iter().filter_map(parse_pattern).collect()
}

fn parse_pattern(json: &JsonValue) -> Option<Pattern> {
    // Check include first
    if let Some(include) = json.get("include").and_then(|v| v.as_str()) {
        return Some(parse_include(include));
    }

    // Check for region (has "begin")
    if let Some(begin_str) = json.get("begin").and_then(|v| v.as_str()) {
        return parse_region(json, begin_str);
    }

    // Check for match
    if let Some(match_str) = json.get("match").and_then(|v| v.as_str()) {
        return parse_match(json, match_str);
    }

    // Check for pattern group (just "patterns" array, no match/begin)
    // These are sometimes used in top-level patterns. We flatten them.
    // Actually, we skip these at this level — they are handled elsewhere.
    None
}

fn parse_include(target: &str) -> Pattern {
    if target == "$self" {
        Pattern::Include(IncludeTarget::SelfRef)
    } else if let Some(key) = target.strip_prefix('#') {
        Pattern::Include(IncludeTarget::Repository(key.to_string()))
    } else {
        // External grammar references — not supported yet, treat as self
        Pattern::Include(IncludeTarget::SelfRef)
    }
}

fn parse_match(json: &JsonValue, match_str: &str) -> Option<Pattern> {
    let regex = Regex::new(match_str).ok()?; // silently skip broken regex
    let name = json
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let captures = parse_captures(json.get("captures")).unwrap_or_default();
    Some(Pattern::Match {
        name,
        regex,
        captures,
    })
}

fn parse_region(json: &JsonValue, begin_str: &str) -> Option<Pattern> {
    let begin = Regex::new(begin_str).ok()?; // silently skip broken regex
    let end_pattern = json
        .get("end")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let name = json
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let content_name = json
        .get("contentName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let begin_captures =
        parse_captures(json.get("beginCaptures")).or_else(|| parse_captures(json.get("captures")));
    let end_captures =
        parse_captures(json.get("endCaptures")).or_else(|| parse_captures(json.get("captures")));

    let patterns = json
        .get("patterns")
        .and_then(|v| v.as_array())
        .map(parse_pattern_array)
        .unwrap_or_default();

    let id = REGION_ID_COUNTER.fetch_add(1, Ordering::Relaxed);

    Some(Pattern::Region {
        id,
        name,
        content_name,
        begin,
        end_pattern,
        begin_captures: begin_captures.unwrap_or_default(),
        end_captures: end_captures.unwrap_or_default(),
        patterns,
    })
}

/// Parse a captures object like `{"0": {"name": "..."}, "1": {"name": "..."}}`.
/// Returns `None` if the input is `None`, and `Some(vec)` otherwise (possibly empty).
fn parse_captures(json: Option<&JsonValue>) -> Option<Vec<(usize, String)>> {
    let obj = json?.as_object()?;
    let mut result = Vec::new();
    for (key, val) in obj {
        if let Ok(idx) = key.parse::<usize>()
            && let Some(name) = val.get("name").and_then(|v| v.as_str())
        {
            result.push((idx, name.to_string()));
        }
    }
    result.sort_by_key(|(idx, _)| *idx);
    Some(result)
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_grammar(json_str: &str) -> Result<Grammar, String> {
        let json = JsonValue::parse(json_str).map_err(|e| e.to_string())?;
        Grammar::from_json(&json)
    }

    #[test]
    fn test_minimal_grammar() {
        let g = parse_grammar(r#"{"scopeName": "source.test", "patterns": []}"#).unwrap();
        assert_eq!(g.scope_name, "source.test");
        assert_eq!(g.name, "source.test"); // defaults to scopeName
        assert!(g.patterns.is_empty());
        assert!(g.file_types.is_empty());
    }

    #[test]
    fn test_match_pattern_with_name() {
        let g = parse_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "\\b(if|else)\\b", "name": "keyword.control"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(g.patterns.len(), 1);
        match &g.patterns[0] {
            Pattern::Match { name, .. } => {
                assert_eq!(name.as_deref(), Some("keyword.control"));
            }
            _ => panic!("expected Match pattern"),
        }
    }

    #[test]
    fn test_region_pattern_with_children() {
        let g = parse_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {
                        "begin": "\"",
                        "end": "\"",
                        "name": "string.quoted.double",
                        "patterns": [
                            {"match": "\\\\.", "name": "constant.character.escape"}
                        ]
                    }
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(g.patterns.len(), 1);
        match &g.patterns[0] {
            Pattern::Region {
                name,
                end_pattern,
                patterns,
                ..
            } => {
                assert_eq!(name.as_deref(), Some("string.quoted.double"));
                assert_eq!(end_pattern, "\"");
                assert_eq!(patterns.len(), 1);
            }
            _ => panic!("expected Region pattern"),
        }
    }

    #[test]
    fn test_include_and_repository() {
        let g = parse_grammar(
            r##"{
                "scopeName": "source.test",
                "patterns": [
                    {"include": "#comments"}
                ],
                "repository": {
                    "comments": {
                        "match": "//.*$",
                        "name": "comment.line"
                    }
                }
            }"##,
        )
        .unwrap();
        assert_eq!(g.patterns.len(), 1);
        match &g.patterns[0] {
            Pattern::Include(IncludeTarget::Repository(key)) => {
                assert_eq!(key, "comments");
            }
            _ => panic!("expected Include pattern"),
        }
        let repo = g.find_repository("comments").unwrap();
        assert_eq!(repo.len(), 1);
        assert!(g.find_repository("nonexistent").is_none());
    }

    #[test]
    fn test_captures_parsing() {
        let g = parse_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {
                        "match": "(\\w+)\\.(\\w+)",
                        "captures": {
                            "1": {"name": "entity.name"},
                            "2": {"name": "support.function"}
                        }
                    }
                ]
            }"#,
        )
        .unwrap();
        match &g.patterns[0] {
            Pattern::Match { captures, .. } => {
                assert_eq!(captures.len(), 2);
                assert_eq!(captures[0], (1, "entity.name".to_string()));
                assert_eq!(captures[1], (2, "support.function".to_string()));
            }
            _ => panic!("expected Match pattern"),
        }
    }

    #[test]
    fn test_invalid_regex_silently_skipped() {
        let g = parse_grammar(
            r#"{
                "scopeName": "source.test",
                "patterns": [
                    {"match": "[invalid", "name": "broken"},
                    {"match": "valid", "name": "keyword"}
                ]
            }"#,
        )
        .unwrap();
        // The broken regex is skipped, only the valid one remains
        assert_eq!(g.patterns.len(), 1);
        match &g.patterns[0] {
            Pattern::Match { name, .. } => {
                assert_eq!(name.as_deref(), Some("keyword"));
            }
            _ => panic!("expected Match pattern"),
        }
    }

    #[test]
    fn test_missing_scope_name_error() {
        let result = parse_grammar(r#"{"patterns": []}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("scopeName"));
    }

    #[test]
    fn test_file_types_extraction() {
        let g = parse_grammar(
            r#"{
                "scopeName": "source.rust",
                "name": "Rust",
                "fileTypes": ["rs"],
                "patterns": []
            }"#,
        )
        .unwrap();
        assert_eq!(g.name, "Rust");
        assert_eq!(g.file_types, vec!["rs".to_string()]);
    }

    /// Regression: repository entry that has both "begin" and "patterns" must be
    /// parsed as a Region, NOT as a pattern group.
    ///
    /// Before the fix, `parse_repository_entry` checked for "patterns" first.
    /// A JSON "string" entry (which has begin:`"`, end:`"`, patterns:[stringcontent])
    /// was returned as `[Include(stringcontent)]` instead of `[Region(begin:")]`.
    /// This caused string/object/array/objectkey regions to never activate.
    #[test]
    fn test_repository_region_with_patterns_is_parsed_as_region() {
        let g = parse_grammar(
            r##"{
                "scopeName": "source.test",
                "patterns": [{"include": "#string"}],
                "repository": {
                    "string": {
                        "begin": "\"",
                        "end": "\"",
                        "name": "string.quoted.double",
                        "patterns": [
                            {"match": "\\\\.", "name": "constant.character.escape"}
                        ]
                    }
                }
            }"##,
        )
        .unwrap();

        let repo = g.find_repository("string").unwrap();
        assert_eq!(repo.len(), 1, "string entry must produce exactly one Region pattern");
        match &repo[0] {
            Pattern::Region { name, end_pattern, patterns, .. } => {
                assert_eq!(name.as_deref(), Some("string.quoted.double"));
                assert_eq!(end_pattern, "\"");
                assert_eq!(patterns.len(), 1, "Region must contain child patterns");
            }
            other => panic!("expected Region, got {:?}", other),
        }
    }

    /// End-to-end: a JSON-like grammar with repository string regions must
    /// highlight string content (not leave it uncolored).
    #[test]
    fn test_string_region_from_repository_tokenizes_content() {
        use crate::syntax::tokenizer::{LineState, Tokenizer};

        let g = parse_grammar(
            r##"{
                "scopeName": "source.test",
                "patterns": [{"include": "#value"}],
                "repository": {
                    "value": {
                        "patterns": [
                            {"include": "#string"},
                            {"include": "#number"}
                        ]
                    },
                    "string": {
                        "begin": "\"",
                        "end": "\"",
                        "name": "string.quoted.double"
                    },
                    "number": {
                        "match": "-?[0-9]+",
                        "name": "constant.numeric"
                    }
                }
            }"##,
        )
        .unwrap();

        let t = Tokenizer::new(&g);

        // String content should carry "string.quoted.double" scope
        let (tokens, state) = t.tokenize_line(r#""hello""#, &LineState::initial());
        assert!(state.stack.is_empty(), "string region should close");
        let str_tok = tokens
            .iter()
            .find(|tok| tok.scopes.iter().any(|s| s.starts_with("string")));
        assert!(str_tok.is_some(), "string content must have string.* scope");

        // Number should be colored too
        let (tokens2, _) = t.tokenize_line("42", &LineState::initial());
        let num_tok = tokens2
            .iter()
            .find(|tok| tok.scopes.contains(&"constant.numeric".to_string()));
        assert!(num_tok.is_some(), "42 must match constant.numeric");
    }
}
