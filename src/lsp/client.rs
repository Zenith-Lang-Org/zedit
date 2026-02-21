// ---------------------------------------------------------------------------
// LSP Client — request/response state machine and document sync
// ---------------------------------------------------------------------------

use crate::syntax::json_parser::JsonValue;

use super::protocol::{self, CompletionItem, Diagnostic, Location};
use super::transport::LspTransport;

// ---------------------------------------------------------------------------
// Pending request tracking
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum PendingKind {
    Initialize,
    Shutdown,
    Completion,
    Hover,
    Definition,
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
        if !self.initialized {
            return;
        }
        let id = self.next_id();
        let req = protocol::completion_request(id, uri, line, character);
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

    /// Drain all pending messages from the server.
    pub fn drain_messages(&mut self) {
        loop {
            match self.transport.try_recv() {
                Ok(Some(msg)) => self.handle_message(msg),
                Ok(None) => break,
                Err(_) => break,
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
                    let _caps = protocol::parse_initialize_result(result);
                    self.initialized = true;
                    // Send initialized notification
                    let notif = protocol::initialized_notification();
                    let _ = self.transport.send(&notif);
                }
            }
            PendingKind::Shutdown => {
                // Server acknowledged shutdown — we already sent exit
            }
            PendingKind::Completion => {
                if let Some(result) = msg.get("result") {
                    self.completion_result = Some(protocol::parse_completion_result(result));
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
