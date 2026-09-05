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
    /// The group the session is filed under, by id; none leaves it at the
    /// top of the list. Absent in files written before there were groups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) group: Option<u64>,
}

/// A folder in the saved sessions list. It is only a name: which sessions
/// it holds is said by the sessions themselves, so a session moves by
/// changing one field, and a group can go without taking anything with it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredGroup {
    pub(crate) id: u64,
    pub(crate) name: String,
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
    #[serde(default)]
    pub(crate) groups: Vec<StoredGroup>,
    #[serde(default = "default_commands")]
    pub(crate) commands: Vec<StoredCommand>,
    #[serde(default = "default_next_id")]
    next_id: u64,
}

impl Default for PresetStore {
    fn default() -> Self {
        Self {
            sessions: Vec::new(),
            groups: Vec::new(),
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
        group: Option<u64>,
    ) {
        let group = self.resolve_group(group);
        if let Some(saved) = self.sessions.iter_mut().find(|saved| saved.label == label) {
            saved.port_name = port_name;
            saved.configuration = configuration;
            saved.color = color;
            saved.alias = alias;
            saved.group = group;
        } else {
            let id = self.take_id();
            self.sessions.push(StoredSession {
                id,
                label,
                port_name,
                configuration,
                color,
                alias,
                group,
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
        group: Option<u64>,
    ) {
        let group = self.resolve_group(group);
        if let Some(saved) = self.sessions.iter_mut().find(|saved| saved.id == id) {
            saved.label = format!("{} · {}", port_name, configuration.summary());
            saved.port_name = port_name;
            saved.configuration = configuration;
            saved.color = color;
            saved.alias = alias;
            saved.group = group;
            self.persist();
        }
    }

    /// Makes a group, and says which it is. A name that is already a
    /// group's names that group rather than a second one like it; a blank
    /// name makes nothing.
    pub(crate) fn add_group(&mut self, name: &str) -> Option<u64> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(group) = self.groups.iter().find(|group| group.name == name) {
            return Some(group.id);
        }
        let id = self.take_id();
        self.groups.push(StoredGroup {
            id,
            name: name.to_string(),
        });
        self.persist();
        Some(id)
    }

    pub(crate) fn rename_group(&mut self, id: u64, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(group) = self.groups.iter_mut().find(|group| group.id == id) {
            group.name = name.to_string();
            self.persist();
        }
    }

    /// Removes a group. The sessions in it are kept, and move to the top of
    /// the list.
    pub(crate) fn remove_group(&mut self, id: u64) {
        self.groups.retain(|group| group.id != id);
        for session in &mut self.sessions {
            if session.group == Some(id) {
                session.group = None;
            }
        }
        self.persist();
    }

    pub(crate) fn group(&self, id: u64) -> Option<&StoredGroup> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// A group reference that points at a group there is: one that does not
    /// — a group removed under an open tab, a hand-edited file — counts as
    /// no group at all.
    pub(crate) fn resolve_group(&self, group: Option<u64>) -> Option<u64> {
        group.filter(|id| self.group(*id).is_some())
    }

    /// The sessions filed under a group, or, for none, the ones at the top
    /// of the list.
    pub(crate) fn sessions_in(&self, group: Option<u64>) -> impl Iterator<Item = &StoredSession> {
        self.sessions
            .iter()
            .filter(move |session| self.resolve_group(session.group) == group)
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
    use crate::SerialConfiguration;

    #[test]
    fn default_presets_round_trip() {
        let json = serde_json::to_string(&PresetStore::default()).unwrap();
        let restored: PresetStore = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.commands.len(), 3);
        assert_eq!(restored.commands[0].command, "AT+STATUS?");
        assert!(restored.groups.is_empty());
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
        assert_eq!(store.sessions[0].group, None);
        assert!(store.groups.is_empty());

        let mut store = store;
        store.update_session(
            7,
            "/dev/tty.usb".into(),
            store.sessions[0].configuration,
            TagColor::Teal,
            Some("Motor board".into()),
            None,
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
            None,
        );
        assert!(!serde_json::to_string(&store).unwrap().contains("alias"));
    }

    /// A group is a name with an id; sessions point at it, and the pointer
    /// survives a trip through the file.
    #[test]
    fn sessions_file_under_a_group_and_come_back_there() {
        let mut store = PresetStore::default();
        let group = store.add_group("  Motor boards ").expect("a group");
        assert_eq!(store.group(group).unwrap().name, "Motor boards");

        store.add_session(
            "/dev/tty.a · 115200 8N1".into(),
            "/dev/tty.a".into(),
            SerialConfiguration::default(),
            TagColor::Red,
            None,
            Some(group),
        );
        store.add_session(
            "/dev/tty.b · 115200 8N1".into(),
            "/dev/tty.b".into(),
            SerialConfiguration::default(),
            TagColor::Teal,
            None,
            None,
        );

        let json = serde_json::to_string(&store).unwrap();
        assert!(json.contains(&format!(r#""group":{group}"#)));
        let restored: PresetStore = serde_json::from_str(&json).unwrap();
        let in_group: Vec<_> = restored
            .sessions_in(Some(group))
            .map(|session| session.port_name.as_str())
            .collect();
        assert_eq!(in_group, ["/dev/tty.a"]);
        let at_top: Vec<_> = restored
            .sessions_in(None)
            .map(|session| session.port_name.as_str())
            .collect();
        assert_eq!(at_top, ["/dev/tty.b"]);
    }

    /// The same name twice is one group; a blank name is none.
    #[test]
    fn group_names_are_unique_and_never_blank() {
        let mut store = PresetStore::default();
        let first = store.add_group("Sensors").unwrap();
        assert_eq!(store.add_group("Sensors"), Some(first));
        assert_eq!(store.add_group("   "), None);
        assert_eq!(store.groups.len(), 1);

        store.rename_group(first, "Field sensors");
        assert_eq!(store.group(first).unwrap().name, "Field sensors");
        store.rename_group(first, "");
        assert_eq!(store.group(first).unwrap().name, "Field sensors");
    }

    /// Removing a group keeps its sessions, at the top of the list; a
    /// session pointing at a group that is gone reads the same way.
    #[test]
    fn removing_a_group_keeps_its_sessions() {
        let mut store = PresetStore::default();
        let group = store.add_group("Bench").unwrap();
        store.add_session(
            "/dev/tty.a · 115200 8N1".into(),
            "/dev/tty.a".into(),
            SerialConfiguration::default(),
            TagColor::Red,
            None,
            Some(group),
        );
        store.remove_group(group);
        assert!(store.groups.is_empty());
        assert_eq!(store.sessions.len(), 1);
        assert_eq!(store.sessions[0].group, None);
        assert_eq!(store.sessions_in(None).count(), 1);

        // A pointer to a group that never existed is no group either.
        store.add_session(
            "/dev/tty.b · 115200 8N1".into(),
            "/dev/tty.b".into(),
            SerialConfiguration::default(),
            TagColor::Red,
            None,
            Some(999),
        );
        assert_eq!(store.sessions[1].group, None);
        assert_eq!(store.resolve_group(Some(999)), None);
    }
}
