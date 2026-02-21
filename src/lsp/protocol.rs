// ---------------------------------------------------------------------------
// LSP Protocol types and JSON-RPC message builders
// ---------------------------------------------------------------------------
//
// Minimal subset of the Language Server Protocol needed for Phase 17A/17B:
// - Initialize handshake
// - Document sync (didOpen, didChange, didSave, didClose)
// - publishDiagnostics notification (server → client)
// - textDocument/completion, hover, definition (Phase 17B)
//
// Uses the existing JsonValue type from src/syntax/json_parser.rs.

use crate::syntax::json_parser::JsonValue;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Debug)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Info = 3,
    Hint = 4,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<String>,
}

// ---------------------------------------------------------------------------
// Phase 17B types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct CompletionItem {
    pub label: String,
    /// LSP kind: 1=Text, 3=Function, 6=Variable, 7=Class, 9=Module…
    pub kind: Option<u32>,
    /// Short type hint shown right-aligned in the menu.
    pub detail: Option<String>,
    /// Text to insert; falls back to `label` when absent.
    pub insert_text: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

// ---------------------------------------------------------------------------
// Server capabilities (minimal subset)
// ---------------------------------------------------------------------------

pub struct ServerCapabilities {
    pub text_document_sync: i32, // 0=None, 1=Full, 2=Incremental
}

impl Default for ServerCapabilities {
    fn default() -> Self {
        Self {
            text_document_sync: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC message builders
// ---------------------------------------------------------------------------

/// Build an initialize request.
pub fn initialize_request(id: i64, root_uri: &str) -> JsonValue {
    json_rpc_request(
        id,
        "initialize",
        JsonValue::Object(vec![
            (
                "processId".into(),
                JsonValue::Number(std::process::id() as f64),
            ),
            ("rootUri".into(), JsonValue::String(root_uri.into())),
            (
                "capabilities".into(),
                JsonValue::Object(vec![(
                    "textDocument".into(),
                    JsonValue::Object(vec![
                        (
                            "synchronization".into(),
                            JsonValue::Object(vec![
                                ("didSave".into(), JsonValue::Bool(true)),
                                ("dynamicRegistration".into(), JsonValue::Bool(false)),
                            ]),
                        ),
                        (
                            "publishDiagnostics".into(),
                            JsonValue::Object(vec![(
                                "relatedInformation".into(),
                                JsonValue::Bool(false),
                            )]),
                        ),
                    ]),
                )]),
            ),
        ]),
    )
}

/// Build an initialized notification (sent after initialize response).
pub fn initialized_notification() -> JsonValue {
    json_rpc_notification("initialized", JsonValue::Object(vec![]))
}

/// Build a textDocument/didOpen notification.
pub fn did_open_notification(uri: &str, language_id: &str, version: i32, text: &str) -> JsonValue {
    json_rpc_notification(
        "textDocument/didOpen",
        JsonValue::Object(vec![(
            "textDocument".into(),
            JsonValue::Object(vec![
                ("uri".into(), JsonValue::String(uri.into())),
                ("languageId".into(), JsonValue::String(language_id.into())),
                ("version".into(), JsonValue::Number(version as f64)),
                ("text".into(), JsonValue::String(text.into())),
            ]),
        )]),
    )
}

/// Build a textDocument/didChange notification (full text sync).
pub fn did_change_notification(uri: &str, version: i32, text: &str) -> JsonValue {
    json_rpc_notification(
        "textDocument/didChange",
        JsonValue::Object(vec![
            (
                "textDocument".into(),
                JsonValue::Object(vec![
                    ("uri".into(), JsonValue::String(uri.into())),
                    ("version".into(), JsonValue::Number(version as f64)),
                ]),
            ),
            (
                "contentChanges".into(),
                JsonValue::Array(vec![JsonValue::Object(vec![(
                    "text".into(),
                    JsonValue::String(text.into()),
                )])]),
            ),
        ]),
    )
}

/// Build a textDocument/didSave notification.
pub fn did_save_notification(uri: &str) -> JsonValue {
    json_rpc_notification(
        "textDocument/didSave",
        JsonValue::Object(vec![(
            "textDocument".into(),
            JsonValue::Object(vec![("uri".into(), JsonValue::String(uri.into()))]),
        )]),
    )
}

/// Build a textDocument/didClose notification.
pub fn did_close_notification(uri: &str) -> JsonValue {
    json_rpc_notification(
        "textDocument/didClose",
        JsonValue::Object(vec![(
            "textDocument".into(),
            JsonValue::Object(vec![("uri".into(), JsonValue::String(uri.into()))]),
        )]),
    )
}

/// Build a textDocument/completion request.
pub fn completion_request(id: i64, uri: &str, line: u32, character: u32) -> JsonValue {
    text_document_position_request(id, "textDocument/completion", uri, line, character)
}

/// Build a textDocument/hover request.
pub fn hover_request(id: i64, uri: &str, line: u32, character: u32) -> JsonValue {
    text_document_position_request(id, "textDocument/hover", uri, line, character)
}

/// Build a textDocument/definition request.
pub fn definition_request(id: i64, uri: &str, line: u32, character: u32) -> JsonValue {
    text_document_position_request(id, "textDocument/definition", uri, line, character)
}

fn text_document_position_request(
    id: i64,
    method: &str,
    uri: &str,
    line: u32,
    character: u32,
) -> JsonValue {
    json_rpc_request(
        id,
        method,
        JsonValue::Object(vec![
            (
                "textDocument".into(),
                JsonValue::Object(vec![("uri".into(), JsonValue::String(uri.into()))]),
            ),
            (
                "position".into(),
                JsonValue::Object(vec![
                    ("line".into(), JsonValue::Number(line as f64)),
                    ("character".into(), JsonValue::Number(character as f64)),
                ]),
            ),
        ]),
    )
}

/// Build a shutdown request.
pub fn shutdown_request(id: i64) -> JsonValue {
    json_rpc_request(id, "shutdown", JsonValue::Null)
}

/// Build an exit notification.
pub fn exit_notification() -> JsonValue {
    json_rpc_notification("exit", JsonValue::Null)
}

// ---------------------------------------------------------------------------
// JSON-RPC message parsers
// ---------------------------------------------------------------------------

/// Parse a publishDiagnostics notification params.
/// Returns (uri, diagnostics).
pub fn parse_diagnostics(params: &JsonValue) -> Option<(String, Vec<Diagnostic>)> {
    let uri = params.get("uri")?.as_str()?.to_string();
    let diag_array = params.get("diagnostics")?.as_array()?;
    let mut diagnostics = Vec::new();
    for d in diag_array {
        if let Some(diag) = parse_one_diagnostic(d) {
            diagnostics.push(diag);
        }
    }
    Some((uri, diagnostics))
}

fn parse_one_diagnostic(val: &JsonValue) -> Option<Diagnostic> {
    let range = parse_range(val.get("range")?)?;
    let severity_num = val.get("severity").and_then(|v| v.as_f64()).unwrap_or(1.0) as i32;
    let severity = match severity_num {
        1 => DiagnosticSeverity::Error,
        2 => DiagnosticSeverity::Warning,
        3 => DiagnosticSeverity::Info,
        4 => DiagnosticSeverity::Hint,
        _ => DiagnosticSeverity::Error,
    };
    let message = val.get("message")?.as_str()?.to_string();
    let source = val
        .get("source")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(Diagnostic {
        range,
        severity,
        message,
        source,
    })
}

fn parse_range(val: &JsonValue) -> Option<Range> {
    let start = parse_position(val.get("start")?)?;
    let end = parse_position(val.get("end")?)?;
    Some(Range { start, end })
}

fn parse_position(val: &JsonValue) -> Option<Position> {
    let line = val.get("line")?.as_f64()? as u32;
    let character = val.get("character")?.as_f64()? as u32;
    Some(Position { line, character })
}

/// Parse initialize result to extract server capabilities.
pub fn parse_initialize_result(result: &JsonValue) -> ServerCapabilities {
    let mut caps = ServerCapabilities::default();
    if let Some(cap) = result.get("capabilities") {
        if let Some(sync) = cap.get("textDocumentSync") {
            // Can be a number or an object with { openClose, change, save }
            if let Some(n) = sync.as_f64() {
                caps.text_document_sync = n as i32;
            } else if let Some(change) = sync.get("change").and_then(|v| v.as_f64()) {
                caps.text_document_sync = change as i32;
            }
        }
    }
    caps
}

/// Parse a textDocument/completion result.
/// Handles both `CompletionList { items }` and a plain array.
pub fn parse_completion_result(result: &JsonValue) -> Vec<CompletionItem> {
    let items_val = if let Some(items) = result.get("items") {
        items
    } else {
        result
    };
    match items_val.as_array() {
        Some(arr) => arr.iter().filter_map(parse_one_completion_item).collect(),
        None => Vec::new(),
    }
}

fn parse_one_completion_item(val: &JsonValue) -> Option<CompletionItem> {
    let label = val.get("label")?.as_str()?.to_string();
    let kind = val.get("kind").and_then(|v| v.as_f64()).map(|n| n as u32);
    let detail = val
        .get("detail")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let insert_text = val
        .get("insertText")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(CompletionItem {
        label,
        kind,
        detail,
        insert_text,
    })
}

/// Parse a textDocument/hover result.
/// Flattens MarkupContent / MarkedString / array → plain String.
pub fn parse_hover_result(result: &JsonValue) -> Option<String> {
    let contents = result.get("contents")?;
    flatten_hover_contents(contents)
}

fn flatten_hover_contents(val: &JsonValue) -> Option<String> {
    match val {
        JsonValue::String(s) => {
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        JsonValue::Object(_) => {
            // MarkupContent: { kind, value } or MarkedString: { language, value }
            if let Some(value) = val.get("value").and_then(|v| v.as_str()) {
                if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            } else {
                None
            }
        }
        JsonValue::Array(arr) => {
            let parts: Vec<String> = arr.iter().filter_map(flatten_hover_contents).collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

/// Parse a textDocument/definition result.
/// Handles `Location`, `Location[]`, and `LocationLink[]`.
pub fn parse_definition_result(result: &JsonValue) -> Vec<Location> {
    match result {
        JsonValue::Array(arr) => arr.iter().filter_map(parse_one_location_or_link).collect(),
        JsonValue::Null => Vec::new(),
        _ => parse_one_location_or_link(result).into_iter().collect(),
    }
}

fn parse_one_location_or_link(val: &JsonValue) -> Option<Location> {
    // LocationLink has targetUri
    if let Some(target_uri) = val.get("targetUri") {
        let uri = target_uri.as_str()?.to_string();
        let range = val
            .get("targetSelectionRange")
            .or_else(|| val.get("targetRange"))
            .and_then(parse_range)?;
        return Some(Location { uri, range });
    }
    // Plain Location { uri, range }
    let uri = val.get("uri")?.as_str()?.to_string();
    let range = parse_range(val.get("range")?)?;
    Some(Location { uri, range })
}

// ---------------------------------------------------------------------------
// JSON-RPC helpers
// ---------------------------------------------------------------------------

fn json_rpc_request(id: i64, method: &str, params: JsonValue) -> JsonValue {
    JsonValue::Object(vec![
        ("jsonrpc".into(), JsonValue::String("2.0".into())),
        ("id".into(), JsonValue::Number(id as f64)),
        ("method".into(), JsonValue::String(method.into())),
        ("params".into(), params),
    ])
}

fn json_rpc_notification(method: &str, params: JsonValue) -> JsonValue {
    JsonValue::Object(vec![
        ("jsonrpc".into(), JsonValue::String("2.0".into())),
        ("method".into(), JsonValue::String(method.into())),
        ("params".into(), params),
    ])
}

// ---------------------------------------------------------------------------
// URI helpers
// ---------------------------------------------------------------------------

/// Convert a file path to a file:// URI.
pub fn path_to_uri(path: &str) -> String {
    format!("file://{}", path)
}

/// Convert a file:// URI to a file path.
pub fn uri_to_path(uri: &str) -> Option<String> {
    uri.strip_prefix("file://").map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_request_structure() {
        let req = initialize_request(1, "file:///project");
        assert_eq!(req.get("jsonrpc").unwrap().as_str(), Some("2.0"));
        assert_eq!(req.get("id").unwrap().as_f64(), Some(1.0));
        assert_eq!(req.get("method").unwrap().as_str(), Some("initialize"));
        let params = req.get("params").unwrap();
        assert_eq!(
            params.get("rootUri").unwrap().as_str(),
            Some("file:///project")
        );
    }

    #[test]
    fn test_did_open_notification() {
        let msg = did_open_notification("file:///test.rs", "rust", 1, "fn main() {}");
        assert_eq!(
            msg.get("method").unwrap().as_str(),
            Some("textDocument/didOpen")
        );
        let params = msg.get("params").unwrap();
        let td = params.get("textDocument").unwrap();
        assert_eq!(td.get("uri").unwrap().as_str(), Some("file:///test.rs"));
        assert_eq!(td.get("languageId").unwrap().as_str(), Some("rust"));
        assert_eq!(td.get("version").unwrap().as_f64(), Some(1.0));
        assert_eq!(td.get("text").unwrap().as_str(), Some("fn main() {}"));
    }

    #[test]
    fn test_did_change_notification() {
        let msg = did_change_notification("file:///test.rs", 2, "fn main() { println!(); }");
        let params = msg.get("params").unwrap();
        let changes = params.get("contentChanges").unwrap().as_array().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].get("text").unwrap().as_str(),
            Some("fn main() { println!(); }")
        );
    }

    #[test]
    fn test_parse_diagnostics() {
        let json = r#"{
            "uri": "file:///test.rs",
            "diagnostics": [
                {
                    "range": {
                        "start": {"line": 5, "character": 10},
                        "end": {"line": 5, "character": 15}
                    },
                    "severity": 1,
                    "message": "expected type `i32`",
                    "source": "rust-analyzer"
                },
                {
                    "range": {
                        "start": {"line": 10, "character": 0},
                        "end": {"line": 10, "character": 5}
                    },
                    "severity": 2,
                    "message": "unused variable"
                }
            ]
        }"#;
        let val = JsonValue::parse(json).unwrap();
        let (uri, diags) = parse_diagnostics(&val).unwrap();
        assert_eq!(uri, "file:///test.rs");
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert_eq!(diags[0].message, "expected type `i32`");
        assert_eq!(diags[0].range.start.line, 5);
        assert_eq!(diags[0].range.start.character, 10);
        assert_eq!(diags[0].source.as_deref(), Some("rust-analyzer"));
        assert_eq!(diags[1].severity, DiagnosticSeverity::Warning);
        assert_eq!(diags[1].source, None);
    }

    #[test]
    fn test_parse_initialize_result() {
        let json = r#"{"capabilities": {"textDocumentSync": 1}}"#;
        let val = JsonValue::parse(json).unwrap();
        let caps = parse_initialize_result(&val);
        assert_eq!(caps.text_document_sync, 1);
    }

    #[test]
    fn test_parse_initialize_result_object_sync() {
        let json = r#"{"capabilities": {"textDocumentSync": {"openClose": true, "change": 2}}}"#;
        let val = JsonValue::parse(json).unwrap();
        let caps = parse_initialize_result(&val);
        assert_eq!(caps.text_document_sync, 2);
    }

    #[test]
    fn test_path_to_uri() {
        assert_eq!(
            path_to_uri("/home/user/test.rs"),
            "file:///home/user/test.rs"
        );
    }

    #[test]
    fn test_uri_to_path() {
        assert_eq!(
            uri_to_path("file:///home/user/test.rs"),
            Some("/home/user/test.rs".to_string())
        );
        assert_eq!(uri_to_path("https://example.com"), None);
    }

    #[test]
    fn test_shutdown_and_exit() {
        let shutdown = shutdown_request(42);
        assert_eq!(shutdown.get("id").unwrap().as_f64(), Some(42.0));
        assert_eq!(shutdown.get("method").unwrap().as_str(), Some("shutdown"));

        let exit = exit_notification();
        assert_eq!(exit.get("method").unwrap().as_str(), Some("exit"));
    }

    #[test]
    fn test_completion_request_structure() {
        let req = completion_request(5, "file:///test.rs", 10, 4);
        assert_eq!(
            req.get("method").unwrap().as_str(),
            Some("textDocument/completion")
        );
        assert_eq!(req.get("id").unwrap().as_f64(), Some(5.0));
        let params = req.get("params").unwrap();
        assert_eq!(
            params
                .get("textDocument")
                .unwrap()
                .get("uri")
                .unwrap()
                .as_str(),
            Some("file:///test.rs")
        );
        assert_eq!(
            params
                .get("position")
                .unwrap()
                .get("line")
                .unwrap()
                .as_f64(),
            Some(10.0)
        );
        assert_eq!(
            params
                .get("position")
                .unwrap()
                .get("character")
                .unwrap()
                .as_f64(),
            Some(4.0)
        );
    }

    #[test]
    fn test_parse_completion_result_array() {
        let json = r#"[
            {"label": "println", "kind": 3, "insertText": "println!(${1:})"},
            {"label": "print", "kind": 3}
        ]"#;
        let val = JsonValue::parse(json).unwrap();
        let items = parse_completion_result(&val);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "println");
        assert_eq!(items[0].kind, Some(3));
        assert_eq!(items[0].insert_text.as_deref(), Some("println!(${1:})"));
        assert_eq!(items[1].label, "print");
        assert_eq!(items[1].insert_text, None);
    }

    #[test]
    fn test_parse_completion_result_list() {
        let json = r#"{"isIncomplete": false, "items": [{"label": "foo", "kind": 6}]}"#;
        let val = JsonValue::parse(json).unwrap();
        let items = parse_completion_result(&val);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "foo");
    }

    #[test]
    fn test_parse_hover_markup_content() {
        let json = r#"{"contents": {"kind": "markdown", "value": "type `HashMap<K, V>`"}}"#;
        let val = JsonValue::parse(json).unwrap();
        let text = parse_hover_result(&val);
        assert_eq!(text.as_deref(), Some("type `HashMap<K, V>`"));
    }

    #[test]
    fn test_parse_hover_string() {
        let json = r#"{"contents": "fn foo() -> i32"}"#;
        let val = JsonValue::parse(json).unwrap();
        let text = parse_hover_result(&val);
        assert_eq!(text.as_deref(), Some("fn foo() -> i32"));
    }

    #[test]
    fn test_parse_definition_location() {
        let json = r#"{"uri": "file:///src/lib.rs", "range": {"start": {"line": 5, "character": 0}, "end": {"line": 5, "character": 10}}}"#;
        let val = JsonValue::parse(json).unwrap();
        let locs = parse_definition_result(&val);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].uri, "file:///src/lib.rs");
        assert_eq!(locs[0].range.start.line, 5);
    }

    #[test]
    fn test_parse_definition_array() {
        let json = r#"[
            {"uri": "file:///a.rs", "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 5}}},
            {"uri": "file:///b.rs", "range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 5}}}
        ]"#;
        let val = JsonValue::parse(json).unwrap();
        let locs = parse_definition_result(&val);
        assert_eq!(locs.len(), 2);
        assert_eq!(locs[0].uri, "file:///a.rs");
        assert_eq!(locs[1].uri, "file:///b.rs");
    }

    #[test]
    fn test_parse_definition_location_link() {
        let json = r#"[{"targetUri": "file:///main.rs", "targetRange": {"start": {"line": 3, "character": 0}, "end": {"line": 3, "character": 4}}, "targetSelectionRange": {"start": {"line": 3, "character": 0}, "end": {"line": 3, "character": 4}}}]"#;
        let val = JsonValue::parse(json).unwrap();
        let locs = parse_definition_result(&val);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].uri, "file:///main.rs");
        assert_eq!(locs[0].range.start.line, 3);
    }
}
