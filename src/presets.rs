use std::{env, fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::SerialConfiguration;
use crate::theme::TagColor;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredSession {
    pub(crate) id: u64,
    pub(crate) label: String,
    pub(crate) port_name: String,
    pub(crate) configuration: SerialConfiguration,
    /// Absent in files written before sessions could be tagged.
    #[serde(default)]
    pub(crate) color: TagColor,
    /// The name the session was given, shown in place of the port's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) alias: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredCommand {
    pub(crate) id: u64,
    pub(crate) label: String,
    pub(crate) command: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct PresetStore {
    #[serde(default)]
    pub(crate) sessions: Vec<StoredSession>,
    #[serde(default = "default_commands")]
    pub(crate) commands: Vec<StoredCommand>,
    #[serde(default = "default_next_id")]
    next_id: u64,
}

impl Default for PresetStore {
    fn default() -> Self {
        Self {
            sessions: Vec::new(),
            commands: default_commands(),
            next_id: default_next_id(),
        }
    }
}

impl PresetStore {
    pub(crate) fn load() -> Self {
        let Some(path) = store_path() else {
            return Self::default();
        };
        let Ok(content) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    pub(crate) fn add_session(
        &mut self,
        label: String,
        port_name: String,
        configuration: SerialConfiguration,
        color: TagColor,
        alias: Option<String>,
    ) {
        if let Some(saved) = self.sessions.iter_mut().find(|saved| saved.label == label) {
            saved.port_name = port_name;
            saved.configuration = configuration;
            saved.color = color;
            saved.alias = alias;
        } else {
            let id = self.take_id();
            self.sessions.push(StoredSession {
                id,
                label,
                port_name,
                configuration,
                color,
                alias,
            });
        }
        self.persist();
    }

    pub(crate) fn remove_session(&mut self, id: u64) {
        self.sessions.retain(|session| session.id != id);
        self.persist();
    }

    pub(crate) fn update_session(
        &mut self,
        id: u64,
        port_name: String,
        configuration: SerialConfiguration,
        color: TagColor,
        alias: Option<String>,
    ) {
        if let Some(saved) = self.sessions.iter_mut().find(|saved| saved.id == id) {
            saved.label = format!("{} · {}", port_name, configuration.summary());
            saved.port_name = port_name;
            saved.configuration = configuration;
            saved.color = color;
            saved.alias = alias;
            self.persist();
        }
    }

    pub(crate) fn add_command(&mut self, command: String) {
        if command.is_empty() || self.commands.iter().any(|saved| saved.command == command) {
            return;
        }
        let id = self.take_id();
        self.commands.push(StoredCommand {
            id,
            label: command.clone(),
            command,
        });
        self.persist();
    }

    pub(crate) fn remove_command(&mut self, id: u64) {
        self.commands.retain(|command| command.id != id);
        self.persist();
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn persist(&self) {
        if let Err(error) = self.try_persist() {
            eprintln!("failed to save serialX presets: {error}");
        }
    }

    fn try_persist(&self) -> io::Result<()> {
        let Some(path) = store_path() else {
            return Ok(());
        };
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent)?;
        let content = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        fs::write(path, content)
    }
}

fn default_next_id() -> u64 {
    100
}

fn default_commands() -> Vec<StoredCommand> {
    [
        (1, "Status", "AT+STATUS?"),
        (2, "Version", "AT+VERSION?"),
        (3, "Reset", "AT+RST"),
    ]
    .into_iter()
    .map(|(id, label, command)| StoredCommand {
        id,
        label: label.into(),
        command: command.into(),
    })
    .collect()
}

/// Where the workspace file lives, per platform. None under `cargo test`, so
/// a test that saves a preset exercises the store without touching the file
/// the developer's own workbench reads.
fn store_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }

    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/serialX/workspace.json"))
    }

    #[cfg(target_os = "windows")]
    {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("serialX/workspace.json"))
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(root).join("serialX/workspace.json"));
        }
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".config/serialX/workspace.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::{PresetStore, TagColor};

    #[test]
    fn default_presets_round_trip() {
        let json = serde_json::to_string(&PresetStore::default()).unwrap();
        let restored: PresetStore = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.commands.len(), 3);
        assert_eq!(restored.commands[0].command, "AT+STATUS?");
    }

    /// A session saved before tags existed has no `color` field and has to
    /// load as the neutral grey rather than fail the whole store.
    #[test]
    fn sessions_saved_without_a_tag_load_untagged() {
        let json = r#"{"sessions":[{"id":7,"label":"/dev/tty.usb · 115200 8N1",
            "port_name":"/dev/tty.usb","configuration":{"baud_rate":115200,
            "data_bits_index":3,"stop_bits_index":0,"parity_index":0,
            "flow_control_index":0}}],"commands":[],"next_id":8}"#;
        let store: PresetStore = serde_json::from_str(json).unwrap();
        assert_eq!(store.sessions[0].color, TagColor::Gray);
        assert_eq!(store.sessions[0].alias, None);

        let mut store = store;
        store.update_session(
            7,
            "/dev/tty.usb".into(),
            store.sessions[0].configuration,
            TagColor::Teal,
            Some("Motor board".into()),
        );
        let json = serde_json::to_string(&store).unwrap();
        assert!(json.contains(r#""color":"teal""#));
        assert!(json.contains(r#""alias":"Motor board""#));

        // A session without a name does not write an empty field.
        store.update_session(
            7,
            "/dev/tty.usb".into(),
            store.sessions[0].configuration,
            TagColor::Teal,
            None,
        );
        assert!(!serde_json::to_string(&store).unwrap().contains("alias"));
    }
}
