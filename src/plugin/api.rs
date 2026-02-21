// ---------------------------------------------------------------------------
// Plugin API — types for editor ↔ plugin IPC messages
// ---------------------------------------------------------------------------

use crate::syntax::json_parser::JsonValue;

// ---------------------------------------------------------------------------
// EventKind
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventKind {
    BufferOpen,
    BufferSave,
    BufferClose,
    CursorMove,
    TextChange,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::BufferOpen => "buffer_open",
            EventKind::BufferSave => "buffer_save",
            EventKind::BufferClose => "buffer_close",
            EventKind::CursorMove => "cursor_move",
            EventKind::TextChange => "text_change",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "buffer_open" => Some(EventKind::BufferOpen),
            "buffer_save" => Some(EventKind::BufferSave),
            "buffer_close" => Some(EventKind::BufferClose),
            "cursor_move" => Some(EventKind::CursorMove),
            "text_change" => Some(EventKind::TextChange),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// PluginCommand
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PluginCommand {
    pub id: String,
    pub label: String,
    pub keybinding: Option<String>,
}

// ---------------------------------------------------------------------------
// PluginManifest
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    /// Entrypoint script, relative to the plugin directory.
    pub main: String,
}

impl PluginManifest {
    pub fn from_json(json: &JsonValue) -> Option<Self> {
        let name = json.get("name")?.as_str()?.to_string();
        let version = json
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.1.0")
            .to_string();
        let description = json
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let main = json.get("main")?.as_str()?.to_string();
        Some(PluginManifest {
            name,
            version,
            description,
            main,
        })
    }
}

// ---------------------------------------------------------------------------
// Message builders — editor → plugin
// ---------------------------------------------------------------------------

/// Build an event notification to send to a plugin.
pub fn build_event_notification(kind: &EventKind, data: &JsonValue) -> JsonValue {
    JsonValue::Object(vec![
        ("method".to_string(), JsonValue::String("event".to_string())),
        (
            "params".to_string(),
            JsonValue::Object(vec![
                (
                    "kind".to_string(),
                    JsonValue::String(kind.as_str().to_string()),
                ),
                ("data".to_string(), data.clone()),
            ]),
        ),
    ])
}

/// Build a command-invoked notification to send to a plugin.
pub fn build_command_notification(command_id: &str) -> JsonValue {
    JsonValue::Object(vec![
        (
            "method".to_string(),
            JsonValue::String("command_invoked".to_string()),
        ),
        (
            "params".to_string(),
            JsonValue::Object(vec![(
                "command_id".to_string(),
                JsonValue::String(command_id.to_string()),
            )]),
        ),
    ])
}

/// Build a response to a plugin request (e.g., GetBufferText).
pub fn build_response(id: &JsonValue, result: JsonValue) -> JsonValue {
    JsonValue::Object(vec![
        ("id".to_string(), id.clone()),
        ("result".to_string(), result),
    ])
}

// ---------------------------------------------------------------------------
// Message parsers — plugin → editor
// ---------------------------------------------------------------------------

/// Parse a plugin request. Returns (id, method, params).
/// `id` is None for notifications (no response expected).
pub fn parse_plugin_message(msg: &JsonValue) -> Option<(Option<JsonValue>, &str, &JsonValue)> {
    let method = msg.get("method")?.as_str()?;
    static EMPTY: JsonValue = JsonValue::Null;
    let params = msg.get("params").unwrap_or(&EMPTY);
    let id = msg.get("id").cloned();
    Some((id, method, params))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_kind_round_trip() {
        for kind in &[
            EventKind::BufferOpen,
            EventKind::BufferSave,
            EventKind::BufferClose,
            EventKind::CursorMove,
            EventKind::TextChange,
        ] {
            let s = kind.as_str();
            assert_eq!(EventKind::from_str(s).as_ref(), Some(kind));
        }
    }

    #[test]
    fn test_event_kind_unknown() {
        assert!(EventKind::from_str("unknown_event").is_none());
    }

    #[test]
    fn test_manifest_from_json() {
        let json_str =
            r#"{"name":"myplugin","version":"1.0.0","description":"A plugin","main":"main.mlx"}"#;
        let json = JsonValue::parse(json_str).unwrap();
        let manifest = PluginManifest::from_json(&json).unwrap();
        assert_eq!(manifest.name, "myplugin");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.description, "A plugin");
        assert_eq!(manifest.main, "main.mlx");
    }

    #[test]
    fn test_manifest_from_json_minimal() {
        // version and description are optional
        let json_str = r#"{"name":"tiny","main":"run.mlx"}"#;
        let json = JsonValue::parse(json_str).unwrap();
        let manifest = PluginManifest::from_json(&json).unwrap();
        assert_eq!(manifest.name, "tiny");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.description, "");
    }

    #[test]
    fn test_manifest_from_json_missing_name() {
        let json_str = r#"{"main":"run.mlx"}"#;
        let json = JsonValue::parse(json_str).unwrap();
        assert!(PluginManifest::from_json(&json).is_none());
    }

    #[test]
    fn test_build_event_notification() {
        let kind = EventKind::BufferSave;
        let data = JsonValue::Object(vec![(
            "path".to_string(),
            JsonValue::String("/a/b.rs".to_string()),
        )]);
        let msg = build_event_notification(&kind, &data);
        assert_eq!(msg.get("method").and_then(|v| v.as_str()), Some("event"));
        let params = msg.get("params").unwrap();
        assert_eq!(
            params.get("kind").and_then(|v| v.as_str()),
            Some("buffer_save")
        );
    }

    #[test]
    fn test_build_command_notification() {
        let msg = build_command_notification("myplugin.format");
        assert_eq!(
            msg.get("method").and_then(|v| v.as_str()),
            Some("command_invoked")
        );
        let params = msg.get("params").unwrap();
        assert_eq!(
            params.get("command_id").and_then(|v| v.as_str()),
            Some("myplugin.format")
        );
    }

    #[test]
    fn test_parse_plugin_message_with_id() {
        let json_str =
            r#"{"id":1,"method":"RegisterCommand","params":{"id":"p.cmd","label":"My Cmd"}}"#;
        let json = JsonValue::parse(json_str).unwrap();
        let (id, method, params) = parse_plugin_message(&json).unwrap();
        assert!(id.is_some());
        assert_eq!(method, "RegisterCommand");
        assert_eq!(params.get("id").and_then(|v| v.as_str()), Some("p.cmd"));
    }

    #[test]
    fn test_parse_plugin_message_notification() {
        let json_str = r#"{"method":"SubscribeEvent","params":{"event":"buffer_save"}}"#;
        let json = JsonValue::parse(json_str).unwrap();
        let (id, method, _params) = parse_plugin_message(&json).unwrap();
        assert!(id.is_none());
        assert_eq!(method, "SubscribeEvent");
    }
}
