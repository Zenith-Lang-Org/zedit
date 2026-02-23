// ---------------------------------------------------------------------------
// LSP Client — request/response state machine and document sync
// ---------------------------------------------------------------------------

use crate::syntax::json_parser::JsonValue;

use super::protocol::{self, CompletionItem, Diagnostic, Location};
use super::transport::LspTransport;

// ---------------------------------------------------------------------------
// Pending request tracking
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum PendingKind {
    Initialize,
    Shutdown,
    Completion,
    Hover,
    Definition,
    /// Carries the document URI so multiple concurrent requests don't conflict.
    SemanticTokens(String),
}

struct PendingRequest {
    kind: PendingKind,
}

// ---------------------------------------------------------------------------
// LspClient
// ---------------------------------------------------------------------------

pub struct LspClient {
    transport: LspTransport,
    next_id: i64,
    initialized: bool,
    root_uri: String,
    language_id: String,
    /// uri → document version
    doc_versions: Vec<(String, i32)>,
    /// uri → diagnostics
    pub diagnostics: Vec<(String, Vec<Diagnostic>)>,
    /// request id → pending info
    pending: Vec<(i64, PendingRequest)>,
    /// True after shutdown request sent
    shutting_down: bool,
    /// Pending completion result (set by handle_response, consumed by drain_lsp_messages).
    pub completion_result: Option<Vec<CompletionItem>>,
    /// Pending hover result.
    pub hover_result: Option<String>,
    /// Pending definition result.
    pub definition_result: Option<Vec<Location>>,
    /// Token type names received in initialize response (e.g. ["keyword","type",...]).
    pub semantic_legend: Vec<String>,
    /// Queue of decoded semantic token results: (uri, tokens).
    /// Multiple results can accumulate between drain_lsp_messages calls.
    pub semantic_tokens_result: Vec<(String, Vec<protocol::SemanticTokenEntry>)>,
    /// Set to true when the initialize response arrives; cleared by take_newly_initialized.
    /// Allows drain_lsp_messages to re-send did_open + request_semantic_tokens for all
    /// already-open buffers that were attempted before initialization completed.
    newly_initialized: bool,
}

impl LspClient {
    pub fn new(transport: LspTransport, root_uri: &str, language_id: &str) -> Self {
        LspClient {
            transport,
            next_id: 1,
            initialized: false,
            root_uri: root_uri.to_string(),
            language_id: language_id.to_string(),
            doc_versions: Vec::new(),
            diagnostics: Vec::new(),
            pending: Vec::new(),
            shutting_down: false,
            completion_result: None,
            hover_result: None,
            definition_result: None,
            semantic_legend: Vec::new(),
            semantic_tokens_result: Vec::new(),
            newly_initialized: false,
        }
    }

    /// Send the initialize request to the server.
    pub fn initialize(&mut self) -> Result<(), String> {
        let id = self.next_id();
        let req = protocol::initialize_request(id, &self.root_uri);
        self.transport.send(&req)?;
        self.pending.push((
            id,
            PendingRequest {
                kind: PendingKind::Initialize,
            },
        ));
        Ok(())
    }

    /// Notify the server that a document was opened.
    pub fn did_open(&mut self, uri: &str, text: &str) {
        if !self.initialized {
            return;
        }
        let version = 1;
        self.set_version(uri, version);
        let msg = protocol::did_open_notification(uri, &self.language_id, version, text);
        let _ = self.transport.send(&msg);
    }

    /// Notify the server that a document changed (full text sync).
    pub fn did_change(&mut self, uri: &str, text: &str) {
        if !self.initialized {
            return;
        }
        let version = self.bump_version(uri);
        let msg = protocol::did_change_notification(uri, version, text);
        let _ = self.transport.send(&msg);
    }

    /// Notify the server that a document was saved.
    pub fn did_save(&mut self, uri: &str) {
        if !self.initialized {
            return;
        }
        let msg = protocol::did_save_notification(uri);
        let _ = self.transport.send(&msg);
    }

    /// Notify the server that a document was closed.
    pub fn did_close(&mut self, uri: &str) {
        if !self.initialized {
            return;
        }
        let msg = protocol::did_close_notification(uri);
        let _ = self.transport.send(&msg);
        // Remove version tracking
        self.doc_versions.retain(|(u, _)| u != uri);
    }

    /// Request completion at the given document position.
    pub fn request_completion(&mut self, uri: &str, line: u32, character: u32) {
        crate::dlog!(
            "[lsp_client] request_completion: initialized={} uri={}",
            self.initialized,
            uri
        );
        if !self.initialized {
            crate::dlog!("[lsp_client] skipping: not initialized yet");
            return;
        }
        let id = self.next_id();
        let req = protocol::completion_request(id, uri, line, character);
        crate::dlog!("[lsp_client] sending completion request id={}", id);
        let _ = self.transport.send(&req);
        self.pending.push((
            id,
            PendingRequest {
                kind: PendingKind::Completion,
            },
        ));
    }

    /// Request hover information at the given document position.
    pub fn request_hover(&mut self, uri: &str, line: u32, character: u32) {
        if !self.initialized {
            return;
        }
        let id = self.next_id();
        let req = protocol::hover_request(id, uri, line, character);
        let _ = self.transport.send(&req);
        self.pending.push((
            id,
            PendingRequest {
                kind: PendingKind::Hover,
            },
        ));
    }

    /// Request go-to-definition at the given document position.
    pub fn request_definition(&mut self, uri: &str, line: u32, character: u32) {
        if !self.initialized {
            return;
        }
        let id = self.next_id();
        let req = protocol::definition_request(id, uri, line, character);
        let _ = self.transport.send(&req);
        self.pending.push((
            id,
            PendingRequest {
                kind: PendingKind::Definition,
            },
        ));
    }

    /// Request full semantic tokens for a document.
    pub fn request_semantic_tokens(&mut self, uri: &str) {
        // Only request if server advertised a legend (supports semantic tokens)
        if !self.initialized || self.semantic_legend.is_empty() {
            return;
        }
        let id = self.next_id();
        let req = protocol::semantic_tokens_request(id, uri);
        let _ = self.transport.send(&req);
        self.pending.push((
            id,
            PendingRequest {
                kind: PendingKind::SemanticTokens(uri.to_string()),
            },
        ));
    }

    /// Drain all pending messages from the server.
    pub fn drain_messages(&mut self) {
        let mut count = 0usize;
        loop {
            match self.transport.try_recv() {
                Ok(Some(msg)) => {
                    count += 1;
                    crate::dlog!("[lsp_client] drain_messages: got message #{}", count);
                    self.handle_message(msg);
                }
                Ok(None) => break,
                Err(e) => {
                    crate::dlog!("[lsp_client] drain_messages: try_recv error: {}", e);
                    break;
                }
            }
        }
    }

    /// Send shutdown request + exit notification.
    pub fn shutdown(&mut self) {
        if self.shutting_down || !self.initialized {
            self.transport.shutdown();
            return;
        }
        self.shutting_down = true;
        let id = self.next_id();
        let req = protocol::shutdown_request(id);
        let _ = self.transport.send(&req);
        self.pending.push((
            id,
            PendingRequest {
                kind: PendingKind::Shutdown,
            },
        ));
        // Give the server a brief moment to respond, then exit
        // In practice, we send exit immediately — the transport Drop will clean up
        let exit = protocol::exit_notification();
        let _ = self.transport.send(&exit);
        self.transport.shutdown();
    }

    /// Check if the transport is alive.
    pub fn is_alive(&self) -> bool {
        !self.transport.is_dead()
    }

    /// Reap the child process (non-blocking). Returns true if it died.
    pub fn reap_transport(&mut self) -> bool {
        self.transport.reap()
    }

    /// Whether initialization handshake is complete.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get the stdout fd for poll integration.
    pub fn stdout_fd(&self) -> i32 {
        self.transport.stdout_fd()
    }

    /// Get diagnostics for a specific URI.
    pub fn diagnostics_for(&self, uri: &str) -> &[Diagnostic] {
        for (u, diags) in &self.diagnostics {
            if u == uri {
                return diags;
            }
        }
        &[]
    }

    /// Returns and clears the newly-initialized flag.
    /// Called by LspManager::drain_lsp_messages to detect when a server just became
    /// ready, so all open buffers can be re-notified with did_open + semantic tokens.
    pub fn take_newly_initialized(&mut self) -> bool {
        let v = self.newly_initialized;
        self.newly_initialized = false;
        v
    }

    // -- Internal --

    fn next_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn set_version(&mut self, uri: &str, version: i32) {
        for (u, v) in &mut self.doc_versions {
            if u == uri {
                *v = version;
                return;
            }
        }
        self.doc_versions.push((uri.to_string(), version));
    }

    fn bump_version(&mut self, uri: &str) -> i32 {
        for (u, v) in &mut self.doc_versions {
            if u == uri {
                *v += 1;
                return *v;
            }
        }
        // Not tracked yet — start at 1
        self.doc_versions.push((uri.to_string(), 1));
        1
    }

    fn handle_message(&mut self, msg: JsonValue) {
        crate::dlog!(
            "[lsp_client] handle_message: id={:?} method={:?}",
            msg.get("id").and_then(|v| v.as_f64()),
            msg.get("method").and_then(|v| v.as_str())
        );
        // Check if it's a response (has "id" and "result" or "error")
        if let Some(id_val) = msg.get("id") {
            if let Some(id) = id_val.as_f64() {
                self.handle_response(id as i64, &msg);
                return;
            }
        }

        // Otherwise it's a notification (has "method")
        if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
            self.handle_notification(method, &msg);
        }
    }

    fn handle_response(&mut self, id: i64, msg: &JsonValue) {
        let kind = match self.pending.iter().position(|(pid, _)| *pid == id) {
            Some(idx) => {
                let (_, req) = self.pending.remove(idx);
                req.kind
            }
            None => return,
        };

        match kind {
            PendingKind::Initialize => {
                if let Some(result) = msg.get("result") {
                    let (_caps, legend) = protocol::parse_initialize_result(result);
                    self.semantic_legend = legend;
                    self.initialized = true;
                    self.newly_initialized = true; // signal editor to re-notify open docs
                    // Send initialized notification
                    let notif = protocol::initialized_notification();
                    let _ = self.transport.send(&notif);
                }
            }
            PendingKind::Shutdown => {
                // Server acknowledged shutdown — we already sent exit
            }
            PendingKind::Completion => {
                crate::dlog!("[lsp_client] got completion response");
                if let Some(result) = msg.get("result") {
                    let items = protocol::parse_completion_result(result);
                    crate::dlog!("[lsp_client] parsed {} completion items", items.len());
                    self.completion_result = Some(items);
                } else {
                    crate::dlog!("[lsp_client] completion response has no 'result' field");
                }
            }
            PendingKind::Hover => {
                if let Some(result) = msg.get("result") {
                    self.hover_result = protocol::parse_hover_result(result);
                }
            }
            PendingKind::Definition => {
                if let Some(result) = msg.get("result") {
                    self.definition_result = Some(protocol::parse_definition_result(result));
                }
            }
            PendingKind::SemanticTokens(uri) => {
                if let Some(result) = msg.get("result") {
                    if let Some(data) = result.get("data") {
                        let tokens = protocol::parse_semantic_tokens(data);
                        self.semantic_tokens_result.push((uri, tokens));
                    }
                }
            }
        }
    }

    fn handle_notification(&mut self, method: &str, msg: &JsonValue) {
        match method {
            "textDocument/publishDiagnostics" => {
                if let Some(params) = msg.get("params") {
                    if let Some((uri, diags)) = protocol::parse_diagnostics(params) {
                        // Update diagnostics for this URI
                        let mut found = false;
                        for (u, d) in &mut self.diagnostics {
                            if *u == uri {
                                *d = diags.clone();
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            self.diagnostics.push((uri, diags));
                        }
                    }
                }
            }
            _ => {
                // Ignore unknown notifications
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// Strategy: construct LspClient directly with a dead transport (no real process),
// then inject JSON-RPC messages via handle_message to verify the state machine.
// This tests the full message-handling flow without needing a running LSP server.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::json_parser::JsonValue;

    /// Build a minimal LspClient backed by a dead, inert transport.
    /// The transport will never send or receive real bytes; we drive the
    /// client state machine by calling handle_message() directly.
    fn make_client() -> LspClient {
        LspClient {
            transport: LspTransport::new_dead(),
            next_id: 1,
            initialized: false,
            root_uri: "file:///project".into(),
            language_id: "zenith".into(),
            doc_versions: Vec::new(),
            diagnostics: Vec::new(),
            pending: Vec::new(),
            shutting_down: false,
            completion_result: None,
            hover_result: None,
            definition_result: None,
            semantic_legend: Vec::new(),
            semantic_tokens_result: Vec::new(),
            newly_initialized: false,
        }
    }

    /// Wrap a JSON string in LSP Content-Length framing.
    fn frame(json: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{}", json.len(), json).into_bytes()
    }

    // -----------------------------------------------------------------------
    // Initialize flow
    // -----------------------------------------------------------------------

    #[test]
    fn test_initialize_response_sets_initialized() {
        let mut client = make_client();
        // Register a pending Initialize request with id=1
        client.pending.push((
            1,
            PendingRequest {
                kind: PendingKind::Initialize,
            },
        ));

        let resp = JsonValue::parse(
            r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":1}}}"#,
        )
        .unwrap();
        client.handle_message(resp);

        assert!(client.initialized, "should be initialized after response");
        assert!(
            client.newly_initialized,
            "newly_initialized flag must be set"
        );
        assert!(
            client.semantic_legend.is_empty(),
            "no legend in this response"
        );
    }

    #[test]
    fn test_initialize_response_stores_legend() {
        let mut client = make_client();
        client.pending.push((
            1,
            PendingRequest {
                kind: PendingKind::Initialize,
            },
        ));

        let resp = JsonValue::parse(
            r#"{
                "jsonrpc":"2.0","id":1,
                "result":{
                    "capabilities":{
                        "textDocumentSync":1,
                        "semanticTokensProvider":{
                            "legend":{
                                "tokenTypes":["keyword","type","variable","function","directive"],
                                "tokenModifiers":[]
                            },
                            "full":true
                        }
                    }
                }
            }"#,
        )
        .unwrap();
        client.handle_message(resp);

        assert!(client.initialized);
        assert!(client.newly_initialized);
        assert_eq!(
            client.semantic_legend,
            vec!["keyword", "type", "variable", "function", "directive"]
        );
    }

    #[test]
    fn test_take_newly_initialized_clears_flag() {
        let mut client = make_client();
        client.newly_initialized = true;

        assert!(
            client.take_newly_initialized(),
            "should return true first call"
        );
        assert!(
            !client.take_newly_initialized(),
            "should return false on second call"
        );
    }

    // -----------------------------------------------------------------------
    // Semantic tokens response
    // -----------------------------------------------------------------------

    #[test]
    fn test_semantic_tokens_response_stored() {
        let mut client = make_client();
        // Pre-condition: client is initialized and has a legend
        client.initialized = true;
        client.semantic_legend = vec!["keyword".into(), "variable".into()];
        client.pending.push((
            2,
            PendingRequest {
                kind: PendingKind::SemanticTokens("file:///test.zl".into()),
            },
        ));

        // delta-encoded: line=0 char=0 len=2 type=0 (keyword), line=0 char=3 len=5 type=1 (variable)
        let resp = JsonValue::parse(
            r#"{"jsonrpc":"2.0","id":2,"result":{"data":[0,0,2,0,0, 0,3,5,1,0]}}"#,
        )
        .unwrap();
        client.handle_message(resp);

        assert!(
            !client.semantic_tokens_result.is_empty(),
            "semantic_tokens_result should be set"
        );
        let (uri, tokens) = client.semantic_tokens_result.remove(0);
        assert_eq!(uri, "file:///test.zl");
        assert_eq!(tokens.len(), 2);

        // First token: keyword at line=0, char=0, len=2
        assert_eq!(tokens[0].line, 0);
        assert_eq!(tokens[0].start_char, 0);
        assert_eq!(tokens[0].length, 2);
        assert_eq!(tokens[0].token_type_idx, 0);

        // Second token: variable at line=0, char=3, len=5
        assert_eq!(tokens[1].line, 0);
        assert_eq!(tokens[1].start_char, 3);
        assert_eq!(tokens[1].length, 5);
        assert_eq!(tokens[1].token_type_idx, 1);
    }

    #[test]
    fn test_semantic_tokens_response_multiline() {
        let mut client = make_client();
        client.initialized = true;
        client.semantic_legend = vec!["keyword".into()];
        client.pending.push((
            3,
            PendingRequest {
                kind: PendingKind::SemanticTokens("file:///multi.zl".into()),
            },
        ));

        // Three tokens spanning two lines:
        // Line 0, char 0, len 2, type 0
        // Line 0, char 3, len 5, type 0 (delta_line=0, delta_char=3)
        // Line 2, char 1, len 3, type 0 (delta_line=2, char=1 absolute)
        let resp = JsonValue::parse(
            r#"{"jsonrpc":"2.0","id":3,"result":{"data":[0,0,2,0,0, 0,3,5,0,0, 2,1,3,0,0]}}"#,
        )
        .unwrap();
        client.handle_message(resp);

        let (_, tokens) = client.semantic_tokens_result.remove(0);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].line, 0);
        assert_eq!(tokens[0].start_char, 0);
        assert_eq!(tokens[1].line, 0);
        assert_eq!(tokens[1].start_char, 3);
        assert_eq!(tokens[2].line, 2);
        assert_eq!(tokens[2].start_char, 1);
    }

    #[test]
    fn test_semantic_tokens_not_stored_when_not_pending() {
        let mut client = make_client();
        // No pending SemanticTokens request
        let resp =
            JsonValue::parse(r#"{"jsonrpc":"2.0","id":99,"result":{"data":[0,0,2,0,0]}}"#).unwrap();
        client.handle_message(resp);
        assert!(
            client.semantic_tokens_result.is_empty(),
            "should not store tokens for unknown id"
        );
    }

    // -----------------------------------------------------------------------
    // Request guards (no send when not initialized)
    // -----------------------------------------------------------------------

    #[test]
    fn test_request_semantic_tokens_skipped_when_not_initialized() {
        let mut client = make_client();
        // Not initialized → request is a no-op (no pending entry added)
        client.request_semantic_tokens("file:///test.zl");
        assert!(client.pending.is_empty());
    }

    #[test]
    fn test_request_semantic_tokens_skipped_when_legend_empty() {
        let mut client = make_client();
        client.initialized = true;
        // Legend empty → server doesn't support semantic tokens → skip
        client.request_semantic_tokens("file:///test.zl");
        assert!(client.pending.is_empty());
    }

    #[test]
    fn test_request_semantic_tokens_adds_pending_when_ready() {
        let mut client = make_client();
        client.initialized = true;
        client.semantic_legend = vec!["keyword".into()];
        // Transport is dead so send fails, but the pending entry is added before send
        client.request_semantic_tokens("file:///test.zl");
        // The pending entry should have been pushed before the (ignored) send error
        // Note: because transport is dead, send returns Err but it's ignored with `let _ =`
        // So pending IS added.
        assert_eq!(client.pending.len(), 1);
        if let PendingKind::SemanticTokens(ref uri) = client.pending[0].1.kind {
            assert_eq!(uri, "file:///test.zl");
        } else {
            panic!("expected PendingKind::SemanticTokens");
        }
    }

    // -----------------------------------------------------------------------
    // Diagnostics notification
    // -----------------------------------------------------------------------

    #[test]
    fn test_diagnostics_notification_stored() {
        let mut client = make_client();
        let notif = JsonValue::parse(
            r#"{
            "jsonrpc":"2.0",
            "method":"textDocument/publishDiagnostics",
            "params":{
                "uri":"file:///test.zl",
                "diagnostics":[{
                    "range":{"start":{"line":3,"character":0},"end":{"line":3,"character":5}},
                    "severity":1,
                    "message":"undeclared variable"
                }]
            }
        }"#,
        )
        .unwrap();
        client.handle_message(notif);

        let diags = client.diagnostics_for("file:///test.zl");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "undeclared variable");
        assert_eq!(diags[0].range.start.line, 3);
    }

    // -----------------------------------------------------------------------
    // Full flow: initialize → semantic tokens → result
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_flow_init_then_semantic_tokens() {
        let mut client = make_client();

        // Step 1: Register initialize pending and process response
        client.pending.push((
            1,
            PendingRequest {
                kind: PendingKind::Initialize,
            },
        ));
        let init_resp = JsonValue::parse(
            r#"{
            "jsonrpc":"2.0","id":1,
            "result":{
                "capabilities":{
                    "textDocumentSync":1,
                    "semanticTokensProvider":{
                        "legend":{
                            "tokenTypes":["keyword","type","variable","directive"],
                            "tokenModifiers":[]
                        },
                        "full":true
                    }
                }
            }
        }"#,
        )
        .unwrap();
        client.handle_message(init_resp);

        // Verify initialization
        assert!(client.initialized);
        assert!(client.newly_initialized);
        assert_eq!(client.semantic_legend.len(), 4);
        assert_eq!(client.semantic_legend[3], "directive");

        // Consume the flag (simulating what drain_lsp_messages does)
        assert!(client.take_newly_initialized());
        assert!(!client.newly_initialized);

        // Step 2: Register and process semantic tokens response
        client.pending.push((
            2,
            PendingRequest {
                kind: PendingKind::SemanticTokens("file:///hello.zl".into()),
            },
        ));
        // Tokens: .:ES:. is directive (idx=3), SI is keyword (idx=0)
        // Line 0: char=0 len=6 type=3(directive); char=7 len=2 type=0(keyword)
        let tok_resp = JsonValue::parse(
            r#"{"jsonrpc":"2.0","id":2,"result":{"data":[0,0,6,3,0, 0,7,2,0,0]}}"#,
        )
        .unwrap();
        client.handle_message(tok_resp);

        assert!(!client.semantic_tokens_result.is_empty());
        let (uri, tokens) = client.semantic_tokens_result.remove(0);
        assert_eq!(uri, "file:///hello.zl");
        assert_eq!(tokens.len(), 2);

        // .:ES:. → directive
        assert_eq!(tokens[0].start_char, 0);
        assert_eq!(tokens[0].length, 6);
        assert_eq!(tokens[0].token_type_idx, 3); // directive

        // SI → keyword
        assert_eq!(tokens[1].start_char, 7);
        assert_eq!(tokens[1].length, 2);
        assert_eq!(tokens[1].token_type_idx, 0); // keyword
    }

    // -----------------------------------------------------------------------
    // Transport read_buf integration via drain_messages()
    // -----------------------------------------------------------------------

    #[test]
    fn test_drain_messages_processes_framed_initialize_response() {
        // Build a client whose transport has a pre-loaded incoming message in its
        // read_buf. read(stdout_fd=-1) fails immediately, so try_recv falls through
        // to try_parse_message which consumes the pre-loaded bytes.
        let init_resp = r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":1,"semanticTokensProvider":{"legend":{"tokenTypes":["keyword","variable"],"tokenModifiers":[]},"full":true}}}}"#;

        let mut client = LspClient {
            transport: LspTransport::new_with_incoming(frame(init_resp)),
            next_id: 2,
            initialized: false,
            root_uri: "file:///project".into(),
            language_id: "zenith".into(),
            doc_versions: Vec::new(),
            diagnostics: Vec::new(),
            pending: vec![(
                1,
                PendingRequest {
                    kind: PendingKind::Initialize,
                },
            )],
            shutting_down: false,
            completion_result: None,
            hover_result: None,
            definition_result: None,
            semantic_legend: Vec::new(),
            semantic_tokens_result: Vec::new(),
            newly_initialized: false,
        };

        // drain_messages reads from the fake buffer, processes the initialize response
        client.drain_messages();

        assert!(client.initialized, "should be initialized after drain");
        assert!(client.newly_initialized, "should set newly_initialized");
        assert_eq!(
            client.semantic_legend,
            vec!["keyword", "variable"],
            "legend should be extracted"
        );
    }

    #[test]
    fn test_drain_messages_processes_semantic_tokens_response() {
        // Two framed messages in sequence: initialize response + semantic tokens response
        let init_resp = r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":1,"semanticTokensProvider":{"legend":{"tokenTypes":["keyword","variable","directive"],"tokenModifiers":[]},"full":true}}}}"#;
        let tok_resp = r#"{"jsonrpc":"2.0","id":2,"result":{"data":[0,0,3,0,0,0,4,5,1,0]}}"#;

        let mut combined = frame(init_resp);
        combined.extend(frame(tok_resp));

        let mut client = LspClient {
            transport: LspTransport::new_with_incoming(combined),
            next_id: 3,
            initialized: false,
            root_uri: "file:///project".into(),
            language_id: "zenith".into(),
            doc_versions: Vec::new(),
            diagnostics: Vec::new(),
            pending: vec![
                (
                    1,
                    PendingRequest {
                        kind: PendingKind::Initialize,
                    },
                ),
                (
                    2,
                    PendingRequest {
                        kind: PendingKind::SemanticTokens("file:///prog.zl".into()),
                    },
                ),
            ],
            shutting_down: false,
            completion_result: None,
            hover_result: None,
            definition_result: None,
            semantic_legend: Vec::new(),
            semantic_tokens_result: Vec::new(),
            newly_initialized: false,
        };

        client.drain_messages();

        // After draining both messages:
        assert!(client.initialized);
        assert!(client.newly_initialized);
        assert_eq!(client.semantic_legend.len(), 3);

        assert!(
            !client.semantic_tokens_result.is_empty(),
            "semantic tokens should be stored"
        );
        let (uri, tokens) = client.semantic_tokens_result.remove(0);
        assert_eq!(uri, "file:///prog.zl");
        assert_eq!(tokens.len(), 2);
        // First: line=0 char=0 len=3 type=0 (keyword)
        assert_eq!(tokens[0].line, 0);
        assert_eq!(tokens[0].start_char, 0);
        assert_eq!(tokens[0].length, 3);
        assert_eq!(tokens[0].token_type_idx, 0);
        // Second: line=0 char=4 len=5 type=1 (variable)
        assert_eq!(tokens[1].line, 0);
        assert_eq!(tokens[1].start_char, 4);
        assert_eq!(tokens[1].length, 5);
        assert_eq!(tokens[1].token_type_idx, 1);
    }
}
