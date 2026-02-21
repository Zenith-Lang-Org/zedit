// ---------------------------------------------------------------------------
// Plugin system — discovery, lifecycle, IPC, command palette integration
// ---------------------------------------------------------------------------

mod api;
mod bridge;

use api::parse_plugin_message;
pub use api::{
    EventKind, PluginCommand, PluginManifest, build_command_notification, build_event_notification,
    build_response,
};
use bridge::PluginTransport;

use crate::syntax::json_parser::JsonValue;

// ---------------------------------------------------------------------------
// Plugin — a running plugin process
// ---------------------------------------------------------------------------

pub struct Plugin {
    pub name: String,
    pub version: String,
    pub description: String,
    transport: PluginTransport,
    /// Commands this plugin has registered via IPC.
    pub commands: Vec<PluginCommand>,
    /// Events this plugin subscribes to.
    pub subscriptions: Vec<EventKind>,
}

impl Plugin {
    fn new(manifest: &PluginManifest, transport: PluginTransport) -> Self {
        Plugin {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone(),
            transport,
            commands: Vec::new(),
            subscriptions: Vec::new(),
        }
    }

    pub fn is_alive(&self) -> bool {
        !self.transport.is_dead()
    }

    pub fn stdout_fd(&self) -> i32 {
        self.transport.stdout_fd()
    }

    /// Send a JSON message to this plugin.
    pub fn send(&mut self, msg: &JsonValue) {
        let _ = self.transport.send_line(msg);
    }

    /// Send an event notification if this plugin is subscribed.
    pub fn send_event(&mut self, kind: &EventKind, data: &JsonValue) {
        if self.subscriptions.contains(kind) {
            let msg = build_event_notification(kind, data);
            let _ = self.transport.send_line(&msg);
        }
    }

    /// Send a command-invoked notification.
    pub fn send_command_invoked(&mut self, command_id: &str) {
        let msg = build_command_notification(command_id);
        let _ = self.transport.send_line(&msg);
    }

    /// Drain all pending messages from this plugin. Returns the raw JSON values.
    pub fn drain(&mut self) -> Vec<JsonValue> {
        let mut msgs = Vec::new();
        while let Some(msg) = self.transport.try_recv_line() {
            msgs.push(msg);
        }
        msgs
    }

    /// Check if child process has exited (non-blocking).
    pub fn reap(&mut self) -> bool {
        self.transport.reap()
    }

    pub fn shutdown(&mut self) {
        self.transport.shutdown();
    }
}

// ---------------------------------------------------------------------------
// PluginRequest — incoming request from a plugin, ready for the editor
// ---------------------------------------------------------------------------

pub enum PluginRequest {
    /// Plugin registers a new palette command.
    RegisterCommand {
        plugin_name: String,
        id: String,
        label: String,
        keybinding: Option<String>,
    },
    /// Plugin subscribes to an event kind. (Already handled internally.)
    SubscribeEvent {
        plugin_name: String,
        kind: EventKind,
    },
    /// Plugin requests the current buffer's text.
    /// The `request_id` is used to send a response back.
    GetBufferText {
        plugin_name: String,
        request_id: JsonValue,
    },
    /// Plugin inserts text at the current cursor position.
    InsertText { plugin_name: String, text: String },
    /// Plugin shows a message in the editor status bar.
    ShowMessage {
        plugin_name: String,
        text: String,
        kind: String,
    },
    /// Plugin requests the current file path.
    GetFilePath {
        plugin_name: String,
        request_id: JsonValue,
    },
}

// ---------------------------------------------------------------------------
// PluginManager
// ---------------------------------------------------------------------------

pub struct PluginManager {
    /// Running plugin processes.
    pub plugins: Vec<Plugin>,
    /// Discovered manifests + plugin directories (not yet launched).
    pub discovered: Vec<(PluginManifest, std::path::PathBuf)>,
}

impl PluginManager {
    pub fn new() -> Self {
        PluginManager {
            plugins: Vec::new(),
            discovered: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Scan `~/.config/zedit/plugins/` for plugin manifests.
    /// Populates `self.discovered` but does not launch anything.
    pub fn discover(&mut self) {
        self.discovered.clear();

        let home = match std::env::var("HOME") {
            Ok(h) => h,
            Err(_) => return,
        };
        let plugins_dir = std::path::PathBuf::from(home)
            .join(".config")
            .join("zedit")
            .join("plugins");

        let dir = match std::fs::read_dir(&plugins_dir) {
            Ok(d) => d,
            Err(_) => return,
        };

        for entry in dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("manifest.json");
            if !manifest_path.exists() {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&manifest_path)
                && let Ok(json) = JsonValue::parse(&content)
                && let Some(manifest) = PluginManifest::from_json(&json)
            {
                self.discovered.push((manifest, path));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Launch all discovered plugins.
    pub fn launch_all(&mut self) {
        for i in 0..self.discovered.len() {
            let (manifest, dir) = &self.discovered[i];
            let main_path = dir.join(&manifest.main);
            let main_str = main_path.to_string_lossy().to_string();
            // Minilux runtime must be on PATH
            if let Ok(transport) = PluginTransport::spawn("minilux", &[main_str]) {
                let plugin = Plugin::new(manifest, transport);
                self.plugins.push(plugin);
            }
        }
    }

    /// Reap dead plugin processes.
    pub fn reap_dead(&mut self) {
        for p in &mut self.plugins {
            p.reap();
        }
        self.plugins.retain(|p| p.is_alive());
    }

    /// Shutdown all plugins.
    pub fn shutdown_all(&mut self) {
        for p in &mut self.plugins {
            p.shutdown();
        }
        self.plugins.clear();
    }

    // -----------------------------------------------------------------------
    // IPC drain and request handling
    // -----------------------------------------------------------------------

    /// Drain all plugins and collect requests for the editor to handle.
    /// `RegisterCommand` and `SubscribeEvent` are also handled internally
    /// (updating plugin.commands / plugin.subscriptions) before being surfaced.
    pub fn drain_and_collect(&mut self) -> Vec<PluginRequest> {
        let mut requests = Vec::new();

        for plugin in &mut self.plugins {
            let msgs = plugin.drain();
            let plugin_name = plugin.name.clone();

            for msg in msgs {
                if let Some((id, method, params)) = parse_plugin_message(&msg) {
                    match method {
                        "RegisterCommand" => {
                            let cmd_id = params
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let label = params
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&cmd_id)
                                .to_string();
                            let keybinding = params
                                .get("keybinding")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            if !cmd_id.is_empty() {
                                // Register command on the plugin itself
                                if !plugin.commands.iter().any(|c| c.id == cmd_id) {
                                    plugin.commands.push(PluginCommand {
                                        id: cmd_id.clone(),
                                        label: label.clone(),
                                        keybinding: keybinding.clone(),
                                    });
                                }
                                requests.push(PluginRequest::RegisterCommand {
                                    plugin_name: plugin_name.clone(),
                                    id: cmd_id,
                                    label,
                                    keybinding,
                                });
                            }
                        }
                        "SubscribeEvent" => {
                            if let Some(event_str) = params.get("event").and_then(|v| v.as_str())
                                && let Some(kind) = EventKind::from_str(event_str)
                            {
                                if !plugin.subscriptions.contains(&kind) {
                                    plugin.subscriptions.push(kind.clone());
                                }
                                requests.push(PluginRequest::SubscribeEvent {
                                    plugin_name: plugin_name.clone(),
                                    kind,
                                });
                            }
                        }
                        "GetBufferText" => {
                            let request_id = id.unwrap_or(JsonValue::Null);
                            requests.push(PluginRequest::GetBufferText {
                                plugin_name: plugin_name.clone(),
                                request_id,
                            });
                        }
                        "InsertText" => {
                            let text = params
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            requests.push(PluginRequest::InsertText {
                                plugin_name: plugin_name.clone(),
                                text,
                            });
                        }
                        "ShowMessage" => {
                            let text = params
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let kind = params
                                .get("kind")
                                .and_then(|v| v.as_str())
                                .unwrap_or("info")
                                .to_string();
                            requests.push(PluginRequest::ShowMessage {
                                plugin_name: plugin_name.clone(),
                                text,
                                kind,
                            });
                        }
                        "GetFilePath" => {
                            let request_id = id.unwrap_or(JsonValue::Null);
                            requests.push(PluginRequest::GetFilePath {
                                plugin_name: plugin_name.clone(),
                                request_id,
                            });
                        }
                        _ => {
                            // Unknown method — ignore
                        }
                    }
                }
            }
        }

        requests
    }

    // -----------------------------------------------------------------------
    // Event dispatch
    // -----------------------------------------------------------------------

    /// Dispatch an event to all subscribed plugins.
    pub fn dispatch_event(&mut self, kind: &EventKind, data: &JsonValue) {
        for plugin in &mut self.plugins {
            plugin.send_event(kind, data);
        }
    }

    /// Invoke a command on its owning plugin.
    pub fn invoke_command(&mut self, command_id: &str) {
        if let Some(plugin) = self
            .plugins
            .iter_mut()
            .find(|p| p.commands.iter().any(|c| c.id == command_id))
        {
            plugin.send_command_invoked(command_id);
        }
    }

    /// Send a response to a specific plugin (by name).
    pub fn send_to(&mut self, plugin_name: &str, msg: &JsonValue) {
        if let Some(plugin) = self.plugins.iter_mut().find(|p| p.name == plugin_name) {
            plugin.send(msg);
        }
    }

    // -----------------------------------------------------------------------
    // Poll integration
    // -----------------------------------------------------------------------

    /// Collect all alive plugin stdout fds for poll() integration.
    pub fn stdout_fds(&self) -> Vec<i32> {
        self.plugins
            .iter()
            .filter(|p| p.is_alive())
            .map(|p| p.stdout_fd())
            .collect()
    }

    // -----------------------------------------------------------------------
    // Palette helpers
    // -----------------------------------------------------------------------

    /// All registered commands from all plugins (plugin_name, command).
    pub fn all_commands(&self) -> Vec<(String, &PluginCommand)> {
        let mut cmds = Vec::new();
        for plugin in &self.plugins {
            for cmd in &plugin.commands {
                cmds.push((plugin.name.clone(), cmd));
            }
        }
        cmds
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_manager_new() {
        let mgr = PluginManager::new();
        assert!(mgr.plugins.is_empty());
        assert!(mgr.discovered.is_empty());
    }

    #[test]
    fn test_all_commands_empty() {
        let mgr = PluginManager::new();
        assert!(mgr.all_commands().is_empty());
    }

    #[test]
    fn test_discover_nonexistent_dir() {
        // Should not panic when plugins dir doesn't exist
        let mut mgr = PluginManager::new();
        mgr.discover();
        assert!(mgr.discovered.is_empty());
    }

    #[test]
    fn test_drain_and_collect_empty() {
        let mut mgr = PluginManager::new();
        let requests = mgr.drain_and_collect();
        assert!(requests.is_empty());
    }
}
