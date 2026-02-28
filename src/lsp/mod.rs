// ---------------------------------------------------------------------------
// LSP module — Language Server Protocol client support
// ---------------------------------------------------------------------------

pub mod client;
pub mod protocol;
pub mod transport;

use client::LspClient;
pub use protocol::{CompletionItem, Location};
use transport::LspTransport;

// ---------------------------------------------------------------------------
// LspServerConfig
// ---------------------------------------------------------------------------

pub struct LspServerConfig {
    pub command: String,
    pub args: Vec<String>,
}

// ---------------------------------------------------------------------------
// LspManager — owns all active LSP client instances
// ---------------------------------------------------------------------------

pub struct LspManager {
    /// language_id → client
    clients: Vec<(String, LspClient)>,
    /// language_id → server config
    config: Vec<(String, LspServerConfig)>,
    root_uri: String,
}

impl LspManager {
    pub fn new(config: Vec<(String, LspServerConfig)>, root_dir: &str) -> Self {
        let root_uri = protocol::path_to_uri(root_dir);
        LspManager {
            clients: Vec::new(),
            config,
            root_uri,
        }
    }

    /// Get or lazily spawn a client for the given language.
    /// Returns None if no server is configured for this language.
    pub fn ensure_client(&mut self, language_id: &str) -> Option<&mut LspClient> {
        // Check if client already exists
        let exists = self.clients.iter().any(|(lang, _)| lang == language_id);
        if exists {
            return self
                .clients
                .iter_mut()
                .find(|(lang, _)| lang == language_id)
                .map(|(_, c)| c);
        }

        // Find config for this language
        let config_idx = self
            .config
            .iter()
            .position(|(lang, _)| lang == language_id)?;
        let (_, cfg) = &self.config[config_idx];

        // Spawn transport
        crate::dlog!(
            "[lsp_mgr] spawning '{}' for lang={}",
            cfg.command,
            language_id
        );
        let transport = match LspTransport::spawn(&cfg.command, &cfg.args) {
            Ok(t) => {
                crate::dlog!("[lsp_mgr] spawn OK");
                t
            }
            Err(e) => {
                crate::dlog!("[lsp_mgr] spawn FAILED: {}", e);
                return None;
            }
        };

        // Create client and send initialize
        crate::dlog!("[lsp_mgr] sending initialize, rootUri={}", self.root_uri);
        let mut client = LspClient::new(transport, &self.root_uri, language_id);
        if client.initialize().is_err() {
            crate::dlog!("[lsp_mgr] initialize() send FAILED");
            return None;
        }
        crate::dlog!("[lsp_mgr] initialize request sent OK");

        self.clients.push((language_id.to_string(), client));
        self.clients.last_mut().map(|(_, c)| c)
    }

    /// Get a mutable reference to the client for a language (if running).
    pub fn client_mut(&mut self, language_id: &str) -> Option<&mut LspClient> {
        self.clients
            .iter_mut()
            .find(|(lang, _)| lang == language_id)
            .map(|(_, c)| c)
    }

    /// Sum of all clients' diagnostics_gen counters.
    /// Increments whenever any file's diagnostics change.
    /// Use this to skip rebuilding the problem panel on frames with no change.
    pub fn diagnostics_generation(&self) -> u64 {
        self.clients.iter().map(|(_, c)| c.diagnostics_gen).sum()
    }

    /// Snapshot every diagnostic stored across ALL clients and ALL files as
    /// owned data. Returns `(uri, diagnostics)` pairs for every file the LSP
    /// server has ever sent a `publishDiagnostics` notification for — including
    /// files that are NOT currently open in the editor.
    pub fn all_diagnostics_owned(&self) -> Vec<(String, Vec<protocol::Diagnostic>)> {
        let mut result = Vec::new();
        for (_, c) in &self.clients {
            for (uri, diags) in &c.diagnostics {
                if !diags.is_empty() {
                    result.push((uri.clone(), diags.clone()));
                }
            }
        }
        result
    }

    /// Returns true if at least one LSP client process is currently alive.
    pub fn has_alive_client(&self) -> bool {
        self.clients.iter().any(|(_, c)| c.is_alive())
    }

    /// Collect all stdout fds from alive clients for poll integration.
    pub fn stdout_fds(&self) -> Vec<i32> {
        self.clients
            .iter()
            .filter(|(_, c)| c.is_alive())
            .map(|(_, c)| c.stdout_fd())
            .collect()
    }

    /// Reap any dead child processes (non-blocking).
    /// Returns `(language_id, exit_code)` for each client that just died.
    pub fn reap_dead_clients(&mut self) -> Vec<(String, i32)> {
        let mut newly_dead = Vec::new();
        for (lang, client) in &mut self.clients {
            if client.is_alive() {
                let died = client.reap_transport();
                if died {
                    let code = client.last_exit_code().unwrap_or(-1);
                    crate::dlog!(
                        "[lsp_mgr] client for lang='{}' exited code={}",
                        lang,
                        code
                    );
                    newly_dead.push((lang.clone(), code));
                }
            }
        }
        newly_dead
    }

    /// Flush buffered outgoing bytes for all alive clients.
    /// Called once per frame before (or after) `drain_all` to ensure that
    /// messages queued while the pipe was full are eventually delivered.
    pub fn flush_all_writes(&mut self) {
        for (_, client) in &mut self.clients {
            if client.is_alive() {
                client.flush_pending_writes();
            }
        }
    }

    /// Drain messages from all clients.
    pub fn drain_all(&mut self) {
        for (_, client) in &mut self.clients {
            if client.is_alive() {
                client.drain_messages();
            }
        }
    }

    /// Shutdown all clients.
    pub fn shutdown_all(&mut self) {
        for (_, client) in &mut self.clients {
            client.shutdown();
        }
        self.clients.clear();
    }

    /// Shutdown and remove any client that has zero open documents.
    /// Called after every didClose so that a server like rust-analyzer is
    /// killed as soon as the user closes the last file of that language,
    /// freeing its memory (850 MB for rust-analyzer) immediately.
    pub fn cull_idle_clients(&mut self) {
        let mut i = 0;
        while i < self.clients.len() {
            if self.clients[i].1.open_document_count() == 0 {
                self.clients[i].1.shutdown();
                self.clients.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Send a completion request for the given language.
    pub fn request_completion(&mut self, lang: &str, uri: &str, line: u32, character: u32) {
        if let Some(client) = self.client_mut(lang) {
            client.request_completion(uri, line, character);
        }
    }

    /// Send a hover request for the given language.
    pub fn request_hover(&mut self, lang: &str, uri: &str, line: u32, character: u32) {
        if let Some(client) = self.client_mut(lang) {
            client.request_hover(uri, line, character);
        }
    }

    /// Send a definition request for the given language.
    pub fn request_definition(&mut self, lang: &str, uri: &str, line: u32, character: u32) {
        if let Some(client) = self.client_mut(lang) {
            client.request_definition(uri, line, character);
        }
    }

    /// Take any pending completion result from any client.
    pub fn take_completion_result(&mut self) -> Option<Vec<CompletionItem>> {
        for (_, client) in &mut self.clients {
            if let Some(r) = client.completion_result.take() {
                return Some(r);
            }
        }
        None
    }

    /// Take any pending hover result from any client.
    pub fn take_hover_result(&mut self) -> Option<String> {
        for (_, client) in &mut self.clients {
            if let Some(r) = client.hover_result.take() {
                return Some(r);
            }
        }
        None
    }

    /// Take any pending definition result from any client.
    pub fn take_definition_result(&mut self) -> Option<Vec<Location>> {
        for (_, client) in &mut self.clients {
            if let Some(r) = client.definition_result.take() {
                return Some(r);
            }
        }
        None
    }

    /// Return the language IDs of clients whose initialization just completed.
    /// Each ID is returned exactly once (flag is cleared on read).
    /// Used by drain_lsp_messages to re-send did_open for already-open buffers
    /// that were attempted before the server finished initializing.
    pub fn take_newly_initialized_langs(&mut self) -> Vec<String> {
        self.clients
            .iter_mut()
            .filter_map(|(lang, client)| {
                if client.take_newly_initialized() {
                    Some(lang.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Take any pending semantic tokens result: (uri, tokens, legend).
    pub fn take_semantic_tokens_result(
        &mut self,
    ) -> Option<(String, Vec<protocol::SemanticTokenEntry>, Vec<String>)> {
        for (_, client) in &mut self.clients {
            if !client.semantic_tokens_result.is_empty() {
                let (uri, tokens) = client.semantic_tokens_result.remove(0);
                let legend = client.semantic_legend.clone();
                return Some((uri, tokens, legend));
            }
        }
        None
    }
}
