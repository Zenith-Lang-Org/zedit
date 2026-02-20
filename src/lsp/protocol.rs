// ---------------------------------------------------------------------------
// LSP Protocol types and JSON-RPC message builders
// ---------------------------------------------------------------------------
//
// Minimal subset of the Language Server Protocol needed for Phase 17A:
// - Initialize handshake
// - Document sync (didOpen, didChange, didSave, didClose)
// - publishDiagnostics notification (server → client)
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
}
