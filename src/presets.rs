use std::{env, fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::SerialConfiguration;
use crate::theme::TagColor;

/// Which of the side panel's two libraries a thing belongs to: the saved
/// sessions, or the commands kept for Quick send. Groups are of one or the
/// other, so a folder of sessions never turns up among the commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Library {
    #[default]
    Sessions,
    Commands,
}

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

/// A folder in one of the lists. It is only a name: which sessions or
/// commands it holds is said by those themselves, so one moves by changing
/// one field, and a group can go without taking anything with it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredGroup {
    pub(crate) id: u64,
    pub(crate) name: String,
    /// Which list the folder is in. Absent in files written when only the
    /// sessions had groups.
    #[serde(default)]
    pub(crate) library: Library,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredCommand {
    pub(crate) id: u64,
    /// What the card is headed: the name the command was given, or the
    /// command itself when it was given none.
    pub(crate) label: String,
    pub(crate) command: String,
    /// The group the command is filed under, by id; none leaves it at the
    /// top of the list. Absent in files written before commands had groups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) group: Option<u64>,
}

impl StoredCommand {
    /// The name the command was given, or none when it goes by its text.
    pub(crate) fn alias(&self) -> Option<&str> {
        (self.label != self.command).then_some(self.label.as_str())
    }
}

/// Lines a terminal keeps above its screen to scroll back through, out of
/// the box: a long bench session without losing the morning's boot log.
pub(crate) const DEFAULT_SCROLLBACK_LINES: usize = 50_000;
/// The least and the most the setting takes. Below a hundred a terminal
/// would forget what it just showed; above a million the memory is the
/// machine's to lose.
pub(crate) const MIN_SCROLLBACK_LINES: usize = 100;
pub(crate) const MAX_SCROLLBACK_LINES: usize = 1_000_000;

/// What is the workbench's to set rather than a session's, kept in the
/// same file as the presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Settings {
    /// Lines kept above the screen to scroll back through, per terminal.
    #[serde(default = "default_scrollback_lines")]
    pub(crate) scrollback_lines: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
        }
    }
}

fn default_scrollback_lines() -> usize {
    DEFAULT_SCROLLBACK_LINES
}

#[derive(Serialize, Deserialize)]
pub(crate) struct PresetStore {
    #[serde(default)]
    pub(crate) sessions: Vec<StoredSession>,
    #[serde(default)]
    pub(crate) groups: Vec<StoredGroup>,
    #[serde(default)]
    pub(crate) commands: Vec<StoredCommand>,
    #[serde(default = "default_next_id")]
    next_id: u64,
    /// Absent in files written before there was anything to set.
    #[serde(default)]
    pub(crate) settings: Settings,
}

impl Default for PresetStore {
    fn default() -> Self {
        Self {
            sessions: Vec::new(),
            groups: Vec::new(),
            commands: Vec::new(),
            next_id: default_next_id(),
            settings: Settings::default(),
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
        let mut store: Self = serde_json::from_str(&content).unwrap_or_default();
        if store.shed_seed_commands() {
            store.persist();
        }
        store
    }

    /// Drops the commands a store used to be seeded with, so a workbench
    /// that still carries them comes up as clean as a new one does now. A
    /// seed is known by its id — the first hundred were kept for them, and
    /// the user's own start past that — and by still saying what it was
    /// made to say, at the top of the list: one renamed, rewritten or
    /// filed under a group is the user's now, and stays. Says whether
    /// anything went.
    fn shed_seed_commands(&mut self) -> bool {
        let before = self.commands.len();
        self.commands.retain(|command| {
            !SEED_COMMANDS.iter().any(|(id, label, text)| {
                command.id == *id
                    && command.label == *label
                    && command.command == *text
                    && command.group.is_none()
            })
        });
        self.commands.len() != before
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
        let group = self.resolve_group(Library::Sessions, group);
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
        let group = self.resolve_group(Library::Sessions, group);
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

    /// Makes a group in a library, and says which it is. A name that is
    /// already a group's there names that group rather than a second one
    /// like it; a blank name makes nothing.
    pub(crate) fn add_group(&mut self, library: Library, name: &str) -> Option<u64> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(group) = self
            .groups_in(library)
            .find(|group| group.name == name)
        {
            return Some(group.id);
        }
        let id = self.take_id();
        self.groups.push(StoredGroup {
            id,
            name: name.to_string(),
            library,
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

    /// Removes a group. What was in it is kept, and moves to the top of its
    /// list.
    pub(crate) fn remove_group(&mut self, id: u64) {
        self.groups.retain(|group| group.id != id);
        for session in &mut self.sessions {
            if session.group == Some(id) {
                session.group = None;
            }
        }
        for command in &mut self.commands {
            if command.group == Some(id) {
                command.group = None;
            }
        }
        self.persist();
    }

    pub(crate) fn group(&self, id: u64) -> Option<&StoredGroup> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// The groups of one library, in the order they were made.
    pub(crate) fn groups_in(&self, library: Library) -> impl Iterator<Item = &StoredGroup> {
        self.groups
            .iter()
            .filter(move |group| group.library == library)
    }

    /// A group reference that points at a group there is, in the library
    /// it is meant for: one that does not — a group removed under an open
    /// tab, a hand-edited file — counts as no group at all.
    pub(crate) fn resolve_group(&self, library: Library, group: Option<u64>) -> Option<u64> {
        group.filter(|id| self.group(*id).is_some_and(|group| group.library == library))
    }

    /// The sessions filed under a group, or, for none, the ones at the top
    /// of the list.
    pub(crate) fn sessions_in(&self, group: Option<u64>) -> impl Iterator<Item = &StoredSession> {
        self.sessions
            .iter()
            .filter(move |session| self.resolve_group(Library::Sessions, session.group) == group)
    }

    /// The commands filed under a group, or, for none, the ones at the top
    /// of the list.
    pub(crate) fn commands_in(&self, group: Option<u64>) -> impl Iterator<Item = &StoredCommand> {
        self.commands
            .iter()
            .filter(move |command| self.resolve_group(Library::Commands, command.group) == group)
    }

    pub(crate) fn command(&self, id: u64) -> Option<&StoredCommand> {
        self.commands.iter().find(|command| command.id == id)
    }

    /// Keeps a command, under a name if it was given one, and says which
    /// it is. The same command saved again into the same group is the one
    /// card, renamed, rather than a second card that sends the same thing;
    /// a blank command is nothing to keep.
    pub(crate) fn add_command(
        &mut self,
        alias: Option<String>,
        command: String,
        group: Option<u64>,
    ) -> Option<u64> {
        let command = command.trim().to_string();
        if command.is_empty() {
            return None;
        }
        let group = self.resolve_group(Library::Commands, group);
        let label = Self::command_label(alias, &command);
        if let Some(saved) = self
            .commands
            .iter_mut()
            .find(|saved| saved.command == command && saved.group == group)
        {
            saved.label = label;
            let id = saved.id;
            self.persist();
            return Some(id);
        }
        let id = self.take_id();
        self.commands.push(StoredCommand {
            id,
            label,
            command,
            group,
        });
        self.persist();
        Some(id)
    }

    /// Changes a saved command: its name, its text, or where it is filed.
    /// A blank command leaves the card as it was.
    pub(crate) fn update_command(
        &mut self,
        id: u64,
        alias: Option<String>,
        command: String,
        group: Option<u64>,
    ) {
        let command = command.trim().to_string();
        if command.is_empty() {
            return;
        }
        let group = self.resolve_group(Library::Commands, group);
        if let Some(saved) = self.commands.iter_mut().find(|saved| saved.id == id) {
            saved.label = Self::command_label(alias, &command);
            saved.command = command;
            saved.group = group;
            self.persist();
        }
    }

    /// A card's heading: the name, or the command when the name is blank.
    fn command_label(alias: Option<String>, command: &str) -> String {
        match alias.map(|alias| alias.trim().to_string()) {
            Some(alias) if !alias.is_empty() => alias,
            _ => command.to_string(),
        }
    }

    pub(crate) fn remove_command(&mut self, id: u64) {
        self.commands.retain(|command| command.id != id);
        self.persist();
    }

    /// Replaces the settings and writes them down.
    pub(crate) fn set_settings(&mut self, settings: Settings) {
        if self.settings != settings {
            self.settings = settings;
            self.persist();
        }
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

/// Ids the user's own things start from. The ones below were the seed
/// commands', and are still left to them so a store that carries them can
/// tell them apart; see `shed_seed_commands`.
fn default_next_id() -> u64 {
    100
}

/// What every store began with through 0.1.6: three AT lines, under ids
/// below the hundred the user's own start from. Nothing starts with them
/// now; they are kept only so a store that still holds them can shed them.
const SEED_COMMANDS: [(u64, &str, &str); 3] = [
    (1, "Status", "AT+STATUS?"),
    (2, "Version", "AT+VERSION?"),
    (3, "Reset", "AT+RST"),
];

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
    use super::{DEFAULT_SCROLLBACK_LINES, Library, PresetStore, Settings, TagColor};
    use crate::SerialConfiguration;

    /// A file from before there were settings comes up with the defaults,
    /// and a setting written down comes back.
    #[test]
    fn settings_default_and_round_trip() {
        let old = r#"{"sessions":[],"groups":[],"commands":[],"next_id":100}"#;
        let store: PresetStore = serde_json::from_str(old).unwrap();
        assert_eq!(store.settings.scrollback_lines, DEFAULT_SCROLLBACK_LINES);

        let mut store = PresetStore::default();
        store.set_settings(Settings {
            scrollback_lines: 1_234,
        });
        let json = serde_json::to_string(&store).unwrap();
        let restored: PresetStore = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.settings.scrollback_lines, 1_234);
    }

    #[test]
    fn default_presets_round_trip() {
        let json = serde_json::to_string(&PresetStore::default()).unwrap();
        let restored: PresetStore = serde_json::from_str(&json).unwrap();
        assert!(restored.sessions.is_empty());
        assert!(restored.groups.is_empty());
        assert!(restored.commands.is_empty(), "a new store starts empty");
        assert_eq!(restored.next_id, 100);
    }

    /// A store written while Quick send was seeded still holds the three AT
    /// lines. Loading sheds the ones left as they were; one the user renamed,
    /// rewrote or filed away is theirs, and so is anything they made.
    #[test]
    fn seed_commands_are_shed_and_the_users_kept() {
        let json = r#"{"sessions":[],"groups":[{"id":50,"name":"Bench","library":"commands"}],
            "commands":[
                {"id":1,"label":"Status","command":"AT+STATUS?"},
                {"id":2,"label":"Firmware","command":"AT+VERSION?"},
                {"id":3,"label":"Reset","command":"AT+RST","group":50},
                {"id":101,"label":"ls -la","command":"ls -la"}],
            "next_id":103}"#;
        let mut store: PresetStore = serde_json::from_str(json).unwrap();
        assert!(store.shed_seed_commands());
        let kept = store
            .commands
            .iter()
            .map(|command| command.id)
            .collect::<Vec<_>>();
        assert_eq!(kept, [2, 3, 101]);
        assert!(!store.shed_seed_commands(), "a second pass finds nothing");
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
        let group = store.add_group(Library::Sessions, "  Motor boards ").expect("a group");
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
        let first = store.add_group(Library::Sessions, "Sensors").unwrap();
        assert_eq!(store.add_group(Library::Sessions, "Sensors"), Some(first));
        assert_eq!(store.add_group(Library::Sessions, "   "), None);
        assert_eq!(store.groups.len(), 1);
        // The same name in the other library is another group.
        let commands = store.add_group(Library::Commands, "Sensors").unwrap();
        assert_ne!(commands, first);
        assert_eq!(store.groups_in(Library::Sessions).count(), 1);
        assert_eq!(store.groups_in(Library::Commands).count(), 1);

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
        let group = store.add_group(Library::Sessions, "Bench").unwrap();
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
        assert_eq!(store.resolve_group(Library::Sessions, Some(999)), None);
        // Nor is a pointer at a group of the other library.
        let commands = store.add_group(Library::Commands, "Bench").unwrap();
        assert_eq!(store.resolve_group(Library::Sessions, Some(commands)), None);
    }

    /// A command is kept under the name it was given, or under its own
    /// text; saved again into the same group it is renamed, not doubled.
    #[test]
    fn commands_are_named_and_not_doubled() {
        let mut store = PresetStore::default();
        let id = store
            .add_command(Some(" Factory reset ".into()), " AT+RESTORE ".into(), None)
            .expect("a command");
        let saved = store.command(id).unwrap();
        assert_eq!(
            (saved.label.as_str(), saved.command.as_str()),
            ("Factory reset", "AT+RESTORE")
        );
        assert_eq!(saved.alias(), Some("Factory reset"));

        let again = store.add_command(None, "AT+RESTORE".into(), None);
        assert_eq!(again, Some(id));
        assert_eq!(store.command(id).unwrap().label, "AT+RESTORE");
        assert_eq!(store.command(id).unwrap().alias(), None);
        assert_eq!(store.add_command(Some("Blank".into()), "  ".into(), None), None);
        assert_eq!(store.commands.len(), 1);
        // A card saved again under a name is that card, renamed.
        let reset = store.add_command(None, "AT+RST".into(), None).unwrap();
        assert_eq!(
            store.add_command(Some("Reboot".into()), "AT+RST".into(), None),
            Some(reset)
        );
        assert_eq!(store.commands.len(), 2);
        assert_eq!(store.command(reset).unwrap().label, "Reboot");
        assert!(!serde_json::to_string(&store).unwrap().contains(r#""group""#));
    }

    /// Commands file under groups of their own, and come back there; a
    /// group removed leaves them at the top of the list.
    #[test]
    fn commands_file_under_their_own_groups() {
        let mut store = PresetStore::default();
        let bench = store.add_group(Library::Commands, "Bench").unwrap();
        let sessions = store.add_group(Library::Sessions, "Bench").unwrap();
        let loose = store
            .add_command(Some("Status".into()), "AT+STATUS?".into(), None)
            .unwrap();
        let id = store
            .add_command(Some("Status".into()), "AT+STATUS?".into(), Some(bench))
            .unwrap();
        // The same command in another group is another card.
        assert_ne!(loose, id);
        let stray = store
            .add_command(None, "AT+GMR".into(), Some(sessions))
            .unwrap();
        assert_eq!(store.command(stray).unwrap().group, None);

        let json = serde_json::to_string(&store).unwrap();
        assert!(json.contains(r#""library":"commands""#));
        let restored: PresetStore = serde_json::from_str(&json).unwrap();
        let in_bench: Vec<_> = restored
            .commands_in(Some(bench))
            .map(|command| command.label.as_str())
            .collect();
        assert_eq!(in_bench, ["Status"]);
        assert_eq!(restored.commands_in(None).count(), 2);

        let mut store = restored;
        store.update_command(id, None, "AT+STATUS?".into(), None);
        assert_eq!(store.command(id).unwrap().label, "AT+STATUS?");
        store.update_command(id, Some("Status".into()), "AT+STATUS?".into(), Some(bench));
        store.remove_group(bench);
        assert_eq!(store.command(id).unwrap().group, None);
        assert_eq!(store.commands_in(None).count(), 3);
    }
}
