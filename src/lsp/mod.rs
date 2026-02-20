// ---------------------------------------------------------------------------
// LSP module — Language Server Protocol client support
// ---------------------------------------------------------------------------

pub mod client;
pub mod protocol;
pub mod transport;

use client::LspClient;
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
        let transport = match LspTransport::spawn(&cfg.command, &cfg.args) {
            Ok(t) => t,
            Err(_) => return None,
        };

        // Create client and send initialize
        let mut client = LspClient::new(transport, &self.root_uri, language_id);
        if client.initialize().is_err() {
            return None;
        }

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

    /// Collect all stdout fds from alive clients for poll integration.
    pub fn stdout_fds(&self) -> Vec<i32> {
        self.clients
            .iter()
            .filter(|(_, c)| c.is_alive())
            .map(|(_, c)| c.stdout_fd())
            .collect()
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

    /// Reap dead clients.
    pub fn reap_dead(&mut self) {
        self.clients.retain(|(_, c)| c.is_alive());
    }

    /// Check if any LSP server is configured.
    pub fn has_config(&self) -> bool {
        !self.config.is_empty()
    }
}
