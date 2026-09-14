//! Remote hosts: the workbench on a machine reached over SSH, the way an
//! editor opens a folder on one.
//!
//! Connected to a host, serialX is that host's serial workspace. The device
//! list is the host's, a session opens the host's port and its bytes come
//! down the SSH channel, the saved sessions and commands are read from and
//! written to the host's own workspace file, recordings are written on the
//! host and a file transfer sends the host's files and lands files on it.
//! Nothing is installed there: the connection starts the platform's own
//! `ssh` with a small Python agent in its command line, and the two ends
//! talk frames over the channel. `agent.py` is the other end; the frame
//! kinds below are the ones it answers to.
//!
//! The system `ssh` is used rather than an SSH library so that everything a
//! user has already set up — `~/.ssh/config` host aliases, agents, jump
//! hosts, hardware keys — keeps working without serialX knowing about any of
//! it. A password, when one is typed, reaches `ssh` the way it asks for one:
//! through `SSH_ASKPASS`, pointed at this executable in its helper mode.

pub(crate) mod browser;
pub(crate) mod dialog;

use std::{
    collections::HashMap,
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Sender},
    },
    thread,
};

use serde::{Deserialize, Serialize};

use crate::modem::Tap;
use crate::presets::store_folder;
use crate::serial::{
    DATA_BITS, FLOW_CONTROLS, PARITIES, PortItem, PortKind, STOP_BITS, SerialCommand,
    SerialConfiguration, SerialEvent,
};

/// The agent, carried in the connection's command line.
const AGENT_SOURCE: &str = include_str!("agent.py");
/// Where the frames begin on the channel: whatever the login shell printed
/// before the agent came up lies before it and is not the agent's.
const AGENT_MARK: &[u8] = b"\x00serialX-agent\x00";
/// How much noise a login shell may print ahead of the mark before the
/// connection is given up as one with no agent at the other end.
const MAX_NOISE: usize = 64 * 1024;
/// The protocol both ends speak; the agent says its own in its hello.
const PROTOCOL: u32 = 1;
/// The helper mode's own variable: set, this executable prints the file's
/// contents and exits, and is `ssh`'s askpass program for the moment.
const ASKPASS_FILE_VARIABLE: &str = "SERIALX_ASKPASS_FILE";
/// The `ssh` to run, when the platform's own is not the one wanted.
const SSH_VARIABLE: &str = "SERIALX_SSH";

// Frames to the agent.
const HELLO: u8 = b'H';
const LIST_PORTS: u8 = b'L';
const OPEN_PORT: u8 = b'O';
const WRITE_PORT: u8 = b'W';
const CLOSE_PORT: u8 = b'C';
const READ_FILE: u8 = b'R';
const WRITE_FILE: u8 = b'S';
const LIST_DIR: u8 = b'D';
#[cfg_attr(not(test), allow(dead_code))]
const MAKE_DIR: u8 = b'M';
// Frames from it.
const OPENED: u8 = b'o';
const PORT_DATA: u8 = b'd';
const PORT_CLOSED: u8 = b'c';
const ERROR: u8 = b'e';

/// A host the workbench can connect to, as kept in the host book: the
/// address and how to reach it. The password is never written down.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteHost {
    pub(crate) id: u64,
    /// The name it is listed under; the address stands in when empty.
    #[serde(default)]
    pub(crate) name: String,
    pub(crate) host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    /// A private key file to offer, when the account's usual ones are not it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) identity: Option<String>,
}

impl RemoteHost {
    /// Reads `user@host:port` — the user and the port optional, an IPv6
    /// address in brackets — into its parts.
    pub(crate) fn parse_address(
        text: &str,
    ) -> Result<(Option<String>, String, Option<u16>), &'static str> {
        let text = text.trim();
        let text = text.strip_prefix("ssh://").unwrap_or(text);
        if text.is_empty() {
            return Err("Type the host to connect to.");
        }
        let (user, rest) = match text.split_once('@') {
            Some((user, rest)) => {
                if user.is_empty() {
                    return Err("The user before the @ is empty.");
                }
                (Some(user.to_string()), rest)
            }
            None => (None, text),
        };
        let (host, port) = if let Some(rest) = rest.strip_prefix('[') {
            let Some((host, after)) = rest.split_once(']') else {
                return Err("The bracketed address is not closed.");
            };
            let port = match after.strip_prefix(':') {
                Some(port) => Some(port),
                None if after.is_empty() => None,
                None => return Err("Only a :port may follow the address."),
            };
            (host.to_string(), port)
        } else {
            match rest.rsplit_once(':') {
                Some((host, port)) if !host.contains(':') => (host.to_string(), Some(port)),
                _ => (rest.to_string(), None),
            }
        };
        if host.is_empty() || host.chars().any(char::is_whitespace) {
            return Err("The host name has a space in it, or is missing.");
        }
        let port = match port {
            Some(port) => match port.parse::<u16>() {
                Ok(port) if port > 0 => Some(port),
                _ => return Err("The port is not a number between 1 and 65535."),
            },
            None => None,
        };
        Ok((user, host, port))
    }

    /// `user@host:port`, with what is set.
    pub(crate) fn address(&self) -> String {
        let mut address = String::new();
        if let Some(user) = &self.user {
            address.push_str(user);
            address.push('@');
        }
        if self.host.contains(':') {
            address.push('[');
            address.push_str(&self.host);
            address.push(']');
        } else {
            address.push_str(&self.host);
        }
        if let Some(port) = self.port {
            address.push(':');
            address.push_str(&port.to_string());
        }
        address
    }

    /// What the host is called in the chrome: its name, else its address.
    pub(crate) fn title(&self) -> String {
        if self.name.trim().is_empty() {
            self.address()
        } else {
            self.name.trim().to_string()
        }
    }

    /// The destination `ssh` is given; the port goes in its own option.
    fn destination(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        }
    }
}

/// The hosts the workbench knows, kept in `remotes.json` beside the
/// workspace file — on this machine always, whichever host is connected.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct HostBook {
    #[serde(default)]
    pub(crate) hosts: Vec<RemoteHost>,
    #[serde(default = "first_host_id")]
    next_id: u64,
}

fn first_host_id() -> u64 {
    1
}

fn host_book_path() -> Option<PathBuf> {
    store_folder().map(|folder| folder.join("remotes.json"))
}

impl HostBook {
    pub(crate) fn load() -> Self {
        let Some(path) = host_book_path() else {
            return Self::default();
        };
        let Ok(content) = fs::read_to_string(path) else {
            return Self::default();
        };
        let mut book: Self = serde_json::from_str(&content).unwrap_or_default();
        if book.next_id == 0 {
            book.next_id = 1;
        }
        book
    }

    /// Keeps a host: in place of the one with its id, or as a new one at
    /// the end of the list. Returns the id it is kept under.
    pub(crate) fn keep(&mut self, mut host: RemoteHost) -> u64 {
        if let Some(existing) = self
            .hosts
            .iter_mut()
            .find(|existing| existing.id == host.id)
        {
            *existing = host.clone();
        } else {
            host.id = self.next_id;
            self.next_id += 1;
            self.hosts.push(host.clone());
        }
        self.persist();
        host.id
    }

    pub(crate) fn remove(&mut self, id: u64) {
        let before = self.hosts.len();
        self.hosts.retain(|host| host.id != id);
        if self.hosts.len() != before {
            self.persist();
        }
    }

    fn persist(&self) {
        let Some(path) = host_book_path() else {
            return;
        };
        let write = || -> io::Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let content = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
            fs::write(&path, content)
        };
        if let Err(error) = write() {
            eprintln!("failed to save serialX remote hosts: {error}");
        }
    }
}

/// What the agent says about its host in its hello, and the workspace
/// file it found there.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RemoteInfo {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) hostname: String,
    #[serde(default)]
    pub(crate) user: String,
    pub(crate) home: String,
    /// `linux`, `darwin`, … as Python names it.
    #[serde(default)]
    pub(crate) platform: String,
    pub(crate) workspace_path: String,
    /// The host's workspace file, when it has one.
    #[serde(default)]
    pub(crate) workspace: Option<String>,
    pub(crate) documents: String,
    pub(crate) downloads: String,
    #[serde(default, deserialize_with = "deserialize_ports")]
    pub(crate) ports: Vec<PortItem>,
}

/// What a dialog needs of the host the workbench is on: the link to ask it
/// things, the account's home there to start a listing from, and the name
/// to say.
#[derive(Clone)]
pub(crate) struct RemoteContext {
    pub(crate) link: RemoteLink,
    pub(crate) home: String,
    pub(crate) title: String,
}

/// A device as the agent lists it.
#[derive(Deserialize)]
struct RemotePort {
    name: String,
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    kind: String,
}

impl From<RemotePort> for PortItem {
    fn from(port: RemotePort) -> Self {
        PortItem {
            name: port.name,
            subtitle: if port.subtitle.is_empty() {
                "Serial Device".into()
            } else {
                port.subtitle
            },
            kind: match port.kind.as_str() {
                "usb" => PortKind::Usb,
                "bluetooth" => PortKind::Bluetooth,
                "pci" => PortKind::Pci,
                _ => PortKind::Unknown,
            },
        }
    }
}

fn deserialize_ports<'de, D>(deserializer: D) -> Result<Vec<PortItem>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let ports: Vec<RemotePort> = Vec::deserialize(deserializer)?;
    Ok(ports.into_iter().map(PortItem::from).collect())
}

fn parse_ports(payload: &[u8]) -> Result<Vec<PortItem>, String> {
    let ports: Vec<RemotePort> =
        serde_json::from_slice(payload).map_err(|error| format!("bad device list: {error}"))?;
    Ok(ports.into_iter().map(PortItem::from).collect())
}

/// A folder on the host, as the agent lists it.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Listing {
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) parent: Option<String>,
    #[serde(default)]
    pub(crate) entries: Vec<Entry>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Entry {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) dir: bool,
    #[serde(default)]
    pub(crate) size: u64,
    #[serde(default)]
    pub(crate) hidden: bool,
}

/// What the agent says of a file it wrote: where it went, which may differ
/// from where it was asked to go when a name was taken.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Written {
    pub(crate) path: String,
    #[serde(default)]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) size: u64,
}

/// How a file is written on the host.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct WriteOptions {
    /// Add to the end rather than start the file over.
    pub(crate) append: bool,
    /// Make the folders on the way.
    pub(crate) mkdirs: bool,
    /// A taken name gets a number after it, and the reply says which.
    pub(crate) unique: bool,
}

/// What the workbench hears of a connection as a whole.
#[derive(Debug)]
pub(crate) enum RemoteEvent {
    /// The agent answered: the host is ours.
    Ready(RemoteInfo),
    /// The connection could not be made; the message says what `ssh` said.
    Failed(String),
    /// A connection that was ready is gone.
    Closed(String),
}

/// A frame on the channel, either way.
struct Frame {
    kind: u8,
    channel: u32,
    payload: Vec<u8>,
}

/// A reply to a request: the frame's payload.
pub(crate) struct Reply {
    pub(crate) payload: Vec<u8>,
}

type ReplySender = smol::channel::Sender<Result<Reply, String>>;

/// A port open on the host: where its bytes go.
struct PortChannel {
    events: smol::channel::Sender<SerialEvent>,
    tap: Tap,
}

struct LinkState {
    next_id: AtomicU32,
    pending: Mutex<HashMap<u32, ReplySender>>,
    ports: Mutex<HashMap<u32, PortChannel>>,
    /// Whether the agent has answered its hello.
    ready: AtomicBool,
    /// Whether the channel has ended, either way.
    dead: AtomicBool,
}

/// The connection's sending end, cloned wherever something on the host is
/// asked for: a tab's port, the store, a recorder, a dialog's file list.
#[derive(Clone)]
pub(crate) struct RemoteLink {
    frames: Sender<Frame>,
    state: Arc<LinkState>,
}

/// A reply on its way.
pub(crate) struct Pending(smol::channel::Receiver<Result<Reply, String>>);

impl Pending {
    pub(crate) async fn wait(self) -> Result<Reply, String> {
        self.0
            .recv()
            .await
            .unwrap_or_else(|_| Err(CONNECTION_CLOSED.to_string()))
    }

    fn wait_blocking(self) -> Result<Reply, String> {
        self.0
            .recv_blocking()
            .unwrap_or_else(|_| Err(CONNECTION_CLOSED.to_string()))
    }
}

const CONNECTION_CLOSED: &str = "The connection to the host closed.";

impl RemoteLink {
    fn next_id(&self) -> u32 {
        self.state.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn is_alive(&self) -> bool {
        !self.state.dead.load(Ordering::Relaxed)
    }

    /// Puts a frame on the channel. False once the channel is gone.
    fn send(&self, kind: u8, channel: u32, payload: Vec<u8>) -> bool {
        if self.state.dead.load(Ordering::Relaxed) {
            return false;
        }
        self.frames
            .send(Frame {
                kind,
                channel,
                payload,
            })
            .is_ok()
    }

    /// Asks the agent something, and gets the reply's handle.
    pub(crate) fn request(&self, kind: u8, payload: Vec<u8>) -> Pending {
        let id = self.next_id();
        let (tx, rx) = smol::channel::bounded(1);
        if self.state.dead.load(Ordering::Relaxed) {
            let _ = tx.send_blocking(Err(CONNECTION_CLOSED.to_string()));
            return Pending(rx);
        }
        self.state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, tx.clone());
        if !self.send(kind, id, payload) {
            self.state
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&id);
            let _ = tx.send_blocking(Err(CONNECTION_CLOSED.to_string()));
        }
        Pending(rx)
    }

    pub(crate) async fn list_ports(&self) -> Result<Vec<PortItem>, String> {
        let reply = self.request(LIST_PORTS, Vec::new()).wait().await?;
        parse_ports(&reply.payload)
    }

    pub(crate) async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        let request = serde_json::json!({ "path": path }).to_string().into_bytes();
        Ok(self.request(READ_FILE, request).wait().await?.payload)
    }

    fn write_file_frame(path: &str, bytes: &[u8], options: WriteOptions) -> Vec<u8> {
        let header = serde_json::json!({
            "path": path,
            "append": options.append,
            "mkdirs": options.mkdirs,
            "unique": options.unique,
        })
        .to_string()
        .into_bytes();
        let mut payload = Vec::with_capacity(2 + header.len() + bytes.len());
        payload.extend_from_slice(&(header.len() as u16).to_be_bytes());
        payload.extend_from_slice(&header);
        payload.extend_from_slice(bytes);
        payload
    }

    pub(crate) async fn write_file(
        &self,
        path: &str,
        bytes: &[u8],
        options: WriteOptions,
    ) -> Result<Written, String> {
        let reply = self
            .request(WRITE_FILE, Self::write_file_frame(path, bytes, options))
            .wait()
            .await?;
        serde_json::from_slice(&reply.payload).map_err(|error| format!("bad reply: {error}"))
    }

    /// Writes a file without waiting to hear how it went: for what is
    /// written often and has no one to tell — the store, a recording. A
    /// failure is printed to the console by the reader.
    pub(crate) fn write_file_detached(&self, path: &str, bytes: &[u8], options: WriteOptions) {
        drop(self.request(WRITE_FILE, Self::write_file_frame(path, bytes, options)));
    }

    pub(crate) async fn list_dir(&self, path: &str) -> Result<Listing, String> {
        let request = serde_json::json!({ "path": path }).to_string().into_bytes();
        let reply = self.request(LIST_DIR, request).wait().await?;
        serde_json::from_slice(&reply.payload).map_err(|error| format!("bad listing: {error}"))
    }

    /// Makes a folder on the host, and the ones on the way to it. Kept
    /// for a picker that offers a new folder; the agent test exercises it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn make_dir(&self, path: &str) -> Result<String, String> {
        let request = serde_json::json!({ "path": path }).to_string().into_bytes();
        let reply = self.request(MAKE_DIR, request).wait().await?;
        let written: Written = serde_json::from_slice(&reply.payload)
            .map_err(|error| format!("bad reply: {error}"))?;
        Ok(written.path)
    }

    /// Opens a port on the host and runs it the way the local worker runs
    /// one: what arrives goes to `events` (or to the tap while a transfer
    /// has the port), and what is sent on the returned channel goes out.
    /// `Connected` comes when the agent has the port, an `Error` when it
    /// could not open it or it went away.
    pub(crate) fn open_port(
        &self,
        port_name: &str,
        configuration: &SerialConfiguration,
        events: smol::channel::Sender<SerialEvent>,
        tap: Tap,
    ) -> mpsc::Sender<SerialCommand> {
        let channel = self.next_id();
        self.state
            .ports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                channel,
                PortChannel {
                    events: events.clone(),
                    tap,
                },
            );
        let settings = serde_json::json!({
            "path": port_name,
            "baud": configuration.baud_rate(),
            "data_bits": DATA_BITS[configuration.data_bits_index].parse::<u8>().unwrap_or(8),
            "parity": PARITIES[configuration.parity_index].to_lowercase(),
            "stop_bits": STOP_BITS[configuration.stop_bits_index].parse::<u8>().unwrap_or(1),
            "flow": FLOW_CONTROLS[configuration.flow_control_index].to_lowercase(),
        })
        .to_string()
        .into_bytes();
        if !self.send(OPEN_PORT, channel, settings) {
            self.drop_port(channel);
            let _ = events.send_blocking(SerialEvent::Error(format!(
                "Unable to open {port_name}: {CONNECTION_CLOSED}"
            )));
        }

        let (tx, rx) = mpsc::channel();
        let link = self.clone();
        thread::spawn(move || {
            for command in rx {
                match command {
                    SerialCommand::Write(bytes) => {
                        if !link.send(WRITE_PORT, channel, bytes) {
                            break;
                        }
                    }
                    SerialCommand::Stop => break,
                }
            }
            // The tab has let go: the agent closes the device and answers
            // with a close, which finds no channel to report to.
            link.drop_port(channel);
            link.send(CLOSE_PORT, channel, Vec::new());
        });
        tx
    }

    fn drop_port(&self, channel: u32) -> Option<PortChannel> {
        self.state
            .ports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&channel)
    }

    /// Hands a frame from the agent to whoever asked for it.
    fn dispatch(&self, frame: Frame) {
        match frame.kind {
            OPENED => {
                let ports = self
                    .state
                    .ports
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(port) = ports.get(&frame.channel) {
                    let _ = port.events.send_blocking(SerialEvent::Connected);
                }
            }
            PORT_DATA => {
                let ports = self
                    .state
                    .ports
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let Some(port) = ports.get(&frame.channel) else {
                    return;
                };
                let mut bytes = Some(frame.payload);
                {
                    let mut tap = port
                        .tap
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Some(transfer) = tap.as_ref()
                        && let Err(returned) = transfer.send(bytes.take().expect("just taken"))
                    {
                        *tap = None;
                        bytes = Some(returned.0);
                    }
                }
                if let Some(bytes) = bytes {
                    let _ = port.events.send_blocking(SerialEvent::Data(bytes));
                }
            }
            PORT_CLOSED => {
                // A close the tab asked for finds its channel gone already;
                // one the device forced still has someone to tell.
                if let Some(port) = self.drop_port(frame.channel) {
                    let reason = String::from_utf8_lossy(&frame.payload).into_owned();
                    let reason = if reason.is_empty() {
                        "The port closed on the host.".to_string()
                    } else {
                        format!("Port closed: {reason}")
                    };
                    let _ = port.events.send_blocking(SerialEvent::Error(reason));
                }
            }
            ERROR => {
                let message = String::from_utf8_lossy(&frame.payload).into_owned();
                let waiting = self
                    .state
                    .pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&frame.channel);
                match waiting {
                    Some(reply) => {
                        if reply.send_blocking(Err(message.clone())).is_err() {
                            eprintln!("serialX remote host: {message}");
                        }
                    }
                    None => {
                        if let Some(port) = self.drop_port(frame.channel) {
                            let _ = port.events.send_blocking(SerialEvent::Error(format!(
                                "Unable to open the port on the host: {message}"
                            )));
                        } else {
                            eprintln!("serialX remote host: {message}");
                        }
                    }
                }
            }
            _ => {
                let waiting = self
                    .state
                    .pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&frame.channel);
                if let Some(reply) = waiting {
                    let _ = reply.send_blocking(Ok(Reply {
                        payload: frame.payload,
                    }));
                }
            }
        }
    }

    /// The channel has ended: everyone waiting on it hears so.
    fn expire(&self, reason: &str) {
        self.state.dead.store(true, Ordering::Relaxed);
        let pending: Vec<_> = self
            .state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain()
            .map(|(_, reply)| reply)
            .collect();
        for reply in pending {
            let _ = reply.send_blocking(Err(reason.to_string()));
        }
        let ports: Vec<_> = self
            .state
            .ports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain()
            .map(|(_, port)| port)
            .collect();
        for port in ports {
            let _ = port
                .events
                .send_blocking(SerialEvent::Error(reason.to_string()));
        }
    }
}

/// A connection to a host: the `ssh` process with the agent at its far end,
/// and the link the workbench talks to it through. Dropping it ends the
/// process, and with it every port open on the host.
pub(crate) struct RemoteConnection {
    link: RemoteLink,
    child: Arc<Mutex<Child>>,
    /// The file the password waits in for `ssh` to ask, while it might.
    secret: Option<PathBuf>,
}

impl RemoteConnection {
    /// Starts connecting. What happens next arrives on the returned
    /// channel: `Ready` once the agent has answered, `Failed` if it never
    /// does, `Closed` when a ready connection ends.
    pub(crate) fn connect(
        host: RemoteHost,
        password: Option<String>,
    ) -> io::Result<(Self, smol::channel::Receiver<RemoteEvent>)> {
        let mut command = Command::new(ssh_program());
        command
            .arg("-T")
            .arg("-o")
            .arg("ServerAliveInterval=15")
            .arg("-o")
            .arg("ServerAliveCountMax=3")
            .arg("-o")
            .arg("ConnectTimeout=20")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg("NumberOfPasswordPrompts=1");
        if let Some(port) = host.port {
            command.arg("-p").arg(port.to_string());
        }
        if let Some(identity) = host
            .identity
            .as_deref()
            .filter(|path| !path.trim().is_empty())
        {
            command
                .arg("-i")
                .arg(identity.trim())
                .arg("-o")
                .arg("IdentitiesOnly=yes");
        }
        let secret = match password.filter(|password| !password.is_empty()) {
            Some(password) => {
                let path = write_secret(&password)?;
                command
                    .env("SSH_ASKPASS", env::current_exe()?)
                    .env("SSH_ASKPASS_REQUIRE", "force")
                    .env(ASKPASS_FILE_VARIABLE, &path);
                // Older `ssh` runs its askpass program only for a display.
                if env::var_os("DISPLAY").is_none() {
                    command.env("DISPLAY", ":0");
                }
                Some(path)
            }
            None => {
                // Nothing to answer a prompt with: better to fail at once
                // than to hang on a question nobody sees.
                command.arg("-o").arg("BatchMode=yes");
                None
            }
        };
        command
            .arg("--")
            .arg(host.destination())
            .arg(bootstrap_command());
        let (connection, events) = Self::spawn(command, host, secret)?;
        Ok((connection, events))
    }

    /// Runs `command` — `ssh`, or under test anything that reaches an
    /// agent — and wires the link to it.
    fn spawn(
        mut command: Command,
        host: RemoteHost,
        secret: Option<PathBuf>,
    ) -> io::Result<(Self, smol::channel::Receiver<RemoteEvent>)> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        let (frames_tx, frames_rx) = mpsc::channel::<Frame>();
        let state = Arc::new(LinkState {
            next_id: AtomicU32::new(1),
            pending: Mutex::new(HashMap::new()),
            ports: Mutex::new(HashMap::new()),
            ready: AtomicBool::new(false),
            dead: AtomicBool::new(false),
        });
        let link = RemoteLink {
            frames: frames_tx,
            state,
        };
        let (events_tx, events_rx) = smol::channel::unbounded();

        // The writer: frames go out as they are handed over.
        thread::spawn(move || {
            let mut stdin = stdin;
            for frame in frames_rx {
                let mut header = [0_u8; 9];
                header[0] = frame.kind;
                header[1..5].copy_from_slice(&frame.channel.to_be_bytes());
                header[5..9].copy_from_slice(&(frame.payload.len() as u32).to_be_bytes());
                if stdin.write_all(&header).is_err()
                    || stdin.write_all(&frame.payload).is_err()
                    || stdin.flush().is_err()
                {
                    break;
                }
            }
        });

        // What `ssh` says on its own behalf, for the report when it ends.
        let complaints = Arc::new(Mutex::new(String::new()));
        let stderr_log = complaints.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let mut log = stderr_log
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if !log.is_empty() {
                    log.push('\n');
                }
                log.push_str(&line);
                if log.len() > 8 * 1024 {
                    let cut = log.len() - 4 * 1024;
                    let cut = log
                        .char_indices()
                        .map(|(index, _)| index)
                        .find(|&index| index >= cut)
                        .unwrap_or(log.len());
                    log.drain(..cut);
                }
            }
        });

        // The reader: frames come in and are dispatched; the end of the
        // stream is the end of the connection.
        let child = Arc::new(Mutex::new(child));
        let reader_link = link.clone();
        let reader_child = child.clone();
        let reader_events = events_tx.clone();
        let reader_host = host.title();
        thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            let outcome = match skip_to_mark(&mut stdout) {
                Ok(()) => read_frames(&mut stdout, &reader_link),
                Err(error) => Err(error),
            };
            let status = reader_child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .wait()
                .ok();
            let said = complaints
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let reason = describe_end(&reader_host, outcome.err(), status, &said);
            reader_link.expire(&reason);
            let event = if reader_link.state.ready.load(Ordering::Relaxed) {
                RemoteEvent::Closed(reason)
            } else {
                RemoteEvent::Failed(reason)
            };
            let _ = reader_events.send_blocking(event);
        });

        // The hello: the agent's answer is the moment the host is ours.
        let hello = link.request(HELLO, b"{}".to_vec());
        let hello_link = link.clone();
        let hello_child = child.clone();
        let hello_secret = secret.clone();
        thread::spawn(move || {
            let outcome = hello.wait_blocking().and_then(|reply| {
                serde_json::from_slice::<RemoteInfo>(&reply.payload)
                    .map_err(|error| format!("The agent's hello could not be read: {error}"))
            });
            // Authenticated by now, either way: the password has done its
            // work and need not wait on disk any longer.
            if let Some(path) = hello_secret {
                let _ = fs::remove_file(path);
            }
            match outcome {
                Ok(info) if info.version == PROTOCOL => {
                    hello_link.state.ready.store(true, Ordering::Relaxed);
                    let _ = events_tx.send_blocking(RemoteEvent::Ready(info));
                }
                Ok(info) => {
                    let _ = events_tx.send_blocking(RemoteEvent::Failed(format!(
                        "The agent on the host speaks protocol {} and this serialX speaks {PROTOCOL}.",
                        info.version
                    )));
                    let _ = hello_child
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .kill();
                }
                Err(message) => {
                    // A channel that died reports itself; a hello the
                    // agent refused does not.
                    if hello_link.is_alive() {
                        let _ = events_tx.send_blocking(RemoteEvent::Failed(message));
                        let _ = hello_child
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .kill();
                    }
                }
            }
        });

        Ok((
            Self {
                link,
                child,
                secret,
            },
            events_rx,
        ))
    }

    pub(crate) fn link(&self) -> &RemoteLink {
        &self.link
    }
}

impl Drop for RemoteConnection {
    fn drop(&mut self) {
        self.link.state.dead.store(true, Ordering::Relaxed);
        let _ = self
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .kill();
        if let Some(path) = self.secret.take() {
            let _ = fs::remove_file(path);
        }
    }
}

/// Reads past whatever the login shell printed, up to the agent's mark.
fn skip_to_mark(reader: &mut impl BufRead) -> Result<(), String> {
    let mut matched = 0;
    let mut noise = Vec::new();
    loop {
        let buffer = reader
            .fill_buf()
            .map_err(|error| format!("The channel could not be read: {error}"))?;
        if buffer.is_empty() {
            return Err(if noise.is_empty() {
                "The host closed the connection before the agent started.".to_string()
            } else {
                format!(
                    "The agent did not start. The host said:\n{}",
                    String::from_utf8_lossy(&noise).trim()
                )
            });
        }
        let byte = buffer[0];
        reader.consume(1);
        if byte == AGENT_MARK[matched] {
            matched += 1;
            if matched == AGENT_MARK.len() {
                if !noise.is_empty() {
                    eprintln!(
                        "serialX remote host: the login shell printed {} bytes before the agent",
                        noise.len()
                    );
                }
                return Ok(());
            }
        } else {
            noise.extend_from_slice(&AGENT_MARK[..matched]);
            noise.push(byte);
            matched = usize::from(byte == AGENT_MARK[0]);
            if noise.len() > MAX_NOISE {
                return Err(
                    "The agent did not start: the host keeps printing something else.".to_string(),
                );
            }
        }
    }
}

/// Frames until the stream ends.
fn read_frames(reader: &mut impl Read, link: &RemoteLink) -> Result<(), String> {
    let mut header = [0_u8; 9];
    loop {
        match reader.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(format!("The channel could not be read: {error}")),
        }
        let kind = header[0];
        let channel = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
        let length = u32::from_be_bytes([header[5], header[6], header[7], header[8]]) as usize;
        let mut payload = vec![0_u8; length];
        reader
            .read_exact(&mut payload)
            .map_err(|error| format!("The channel ended mid-frame: {error}"))?;
        link.dispatch(Frame {
            kind,
            channel,
            payload,
        });
    }
}

/// One sentence on how a connection ended, from what the reader saw, how
/// `ssh` exited and what it printed.
fn describe_end(
    host: &str,
    error: Option<String>,
    status: Option<std::process::ExitStatus>,
    said: &str,
) -> String {
    let said: Vec<&str> = said
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("Warning: Permanently added"))
        .collect();
    let said = said.join("\n");
    if let Some(error) = error {
        return if said.is_empty() {
            error
        } else {
            format!("{error}\n{said}")
        };
    }
    if !said.is_empty() {
        return said;
    }
    match status {
        Some(status) if status.success() => format!("The connection to {host} closed."),
        Some(status) => match status.code() {
            Some(code) => format!("ssh exited with status {code}."),
            None => "ssh was stopped.".to_string(),
        },
        None => format!("The connection to {host} closed."),
    }
}

/// The `ssh` to run: the platform's own, unless another is named.
fn ssh_program() -> String {
    env::var(SSH_VARIABLE)
        .ok()
        .filter(|program| !program.trim().is_empty())
        .unwrap_or_else(|| "ssh".to_string())
}

/// The command line that starts the agent on the host: Python, handed the
/// agent's source in base64 so that no shell on the way has anything to
/// quote. `python3` is on every host a serial device is likely to hang
/// off; one without it says so in the connection's error.
pub(crate) fn bootstrap_command() -> String {
    format!(
        "exec python3 -c 'import sys,base64;exec(compile(base64.b64decode(sys.argv[1]),\"serialx-agent\",\"exec\"))' {}",
        base64(AGENT_SOURCE.as_bytes())
    )
}

/// Standard base64 with padding; enough for one string, so no crate.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut triple = [0_u8; 3];
        triple[..chunk.len()].copy_from_slice(chunk);
        let n = u32::from_be_bytes([0, triple[0], triple[1], triple[2]]);
        let symbols = [
            TABLE[(n >> 18) as usize & 63],
            TABLE[(n >> 12) as usize & 63],
            TABLE[(n >> 6) as usize & 63],
            TABLE[n as usize & 63],
        ];
        out.push(symbols[0] as char);
        out.push(symbols[1] as char);
        out.push(if chunk.len() > 1 {
            symbols[2] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            symbols[3] as char
        } else {
            '='
        });
    }
    out
}

/// Writes the password where the helper mode will find it: a file only
/// this account can read, in a folder of the same, gone once `ssh` has
/// asked.
fn write_secret(password: &str) -> io::Result<PathBuf> {
    let folder = env::temp_dir().join(format!("serialX-{}", std::process::id()));
    fs::create_dir_all(&folder)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))?;
    }
    let path = folder.join(format!(
        "askpass-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or(0)
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(password.as_bytes())?;
    Ok(path)
}

/// Whether this process was started by `ssh` as its askpass program.
pub(crate) fn is_askpass_mode() -> bool {
    env::var_os(ASKPASS_FILE_VARIABLE).is_some()
}

/// The helper mode: prints the password for a password prompt and says
/// nothing to any other question, so a host-key prompt that slipped past
/// the options gets no answer rather than the password. The exit code is
/// the process's.
pub(crate) fn run_askpass() -> i32 {
    let prompt = env::args().nth(1).unwrap_or_default().to_lowercase();
    if !(prompt.is_empty() || prompt.contains("password") || prompt.contains("passphrase")) {
        return 1;
    }
    let Some(path) = env::var_os(ASKPASS_FILE_VARIABLE) else {
        return 1;
    };
    match fs::read_to_string(path) {
        Ok(password) => {
            let mut stdout = io::stdout().lock();
            let _ = stdout.write_all(password.trim_end_matches(['\r', '\n']).as_bytes());
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
            0
        }
        Err(_) => 1,
    }
}

/// `folder/name` on a POSIX host, whatever this machine's separator is.
pub(crate) fn remote_join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        return name.to_string();
    }
    if folder.ends_with('/') {
        format!("{folder}{name}")
    } else {
        format!("{folder}/{name}")
    }
}

/// The last segment of a host path.
pub(crate) fn remote_file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xee, 0xdd]), "/+7d");
    }

    #[test]
    fn an_address_is_read_into_its_parts() {
        assert_eq!(
            RemoteHost::parse_address("pi@lab.local:2222"),
            Ok((Some("pi".into()), "lab.local".into(), Some(2222)))
        );
        assert_eq!(
            RemoteHost::parse_address("  lab  "),
            Ok((None, "lab".into(), None))
        );
        assert_eq!(
            RemoteHost::parse_address("ssh://root@[fe80::1]:22"),
            Ok((Some("root".into()), "fe80::1".into(), Some(22)))
        );
        assert_eq!(
            RemoteHost::parse_address("me@fe80::1"),
            Ok((Some("me".into()), "fe80::1".into(), None))
        );
        assert!(RemoteHost::parse_address("").is_err());
        assert!(RemoteHost::parse_address("@host").is_err());
        assert!(RemoteHost::parse_address("host:70000").is_err());
        assert!(RemoteHost::parse_address("a host").is_err());
    }

    #[test]
    fn a_host_prints_its_address_back() {
        let host = RemoteHost {
            id: 1,
            name: String::new(),
            host: "fe80::1".into(),
            user: Some("me".into()),
            port: Some(2200),
            identity: None,
        };
        assert_eq!(host.address(), "me@[fe80::1]:2200");
        assert_eq!(host.title(), "me@[fe80::1]:2200");
        assert_eq!(host.destination(), "me@fe80::1");
        let named = RemoteHost {
            name: "Lab".into(),
            ..host
        };
        assert_eq!(named.title(), "Lab");
    }

    #[test]
    fn host_paths_join_with_a_slash() {
        assert_eq!(remote_join("/home/pi", "serialX"), "/home/pi/serialX");
        assert_eq!(remote_join("/", "etc"), "/etc");
        assert_eq!(remote_file_name("/home/pi/a.bin"), "a.bin");
        assert_eq!(remote_file_name("a.bin"), "a.bin");
    }

    #[test]
    fn the_mark_is_found_past_a_shells_noise() {
        let mut stream = io::Cursor::new(
            [
                b"stty: not a tty\n\x00serial".as_slice(),
                AGENT_MARK,
                b"rest",
            ]
            .concat(),
        );
        skip_to_mark(&mut stream).unwrap();
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"rest");

        let mut silent = io::Cursor::new(b"bash: python3: command not found\n".to_vec());
        let error = skip_to_mark(&mut silent).unwrap_err();
        assert!(error.contains("python3: command not found"));
    }

    #[test]
    fn the_end_of_a_connection_is_said_from_what_ssh_left() {
        assert_eq!(
            describe_end(
                "lab",
                None,
                None,
                "Warning: Permanently added 'lab' to the list of known hosts.\nPermission denied (publickey).\n"
            ),
            "Permission denied (publickey)."
        );
        assert_eq!(
            describe_end("lab", Some("The channel ended mid-frame".into()), None, ""),
            "The channel ended mid-frame"
        );
        assert_eq!(
            describe_end("lab", None, None, ""),
            "The connection to lab closed."
        );
    }

    /// Whether the machine running the tests can run the agent.
    fn python3() -> bool {
        Command::new("python3")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// The agent, run on this machine through the same command line the
    /// connection gives `ssh`, so the quoting and the framing are the ones
    /// a host sees.
    fn local_agent() -> Option<(RemoteConnection, smol::channel::Receiver<RemoteEvent>)> {
        if !python3() {
            eprintln!("python3 is not on this machine; the agent test is skipped");
            return None;
        }
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!(
            "echo 'noise from a login shell'; {}",
            bootstrap_command()
        ));
        let host = RemoteHost {
            id: 0,
            name: "here".into(),
            host: "localhost".into(),
            user: None,
            port: None,
            identity: None,
        };
        Some(RemoteConnection::spawn(command, host, None).expect("sh runs"))
    }

    #[test]
    fn the_agent_answers_over_the_channel() {
        let Some((connection, events)) = local_agent() else {
            return;
        };
        let info = match events.recv_blocking().expect("an event") {
            RemoteEvent::Ready(info) => info,
            other => panic!("expected the agent's hello, got {other:?}"),
        };
        assert_eq!(info.version, PROTOCOL);
        assert!(!info.home.is_empty());
        assert!(info.workspace_path.ends_with("serialX/workspace.json"));

        let link = connection.link().clone();
        let scratch = env::temp_dir().join(format!("serialX-agent-test-{}", std::process::id()));
        let scratch_str = scratch.to_string_lossy().into_owned();
        smol::block_on(async {
            let made = link.make_dir(&scratch_str).await.unwrap();
            assert_eq!(made, scratch_str);

            let file = remote_join(&scratch_str, "hello.bin");
            let written = link
                .write_file(&file, b"hello", WriteOptions::default())
                .await
                .unwrap();
            assert_eq!(written.path, file);
            assert_eq!(written.size, 5);
            link.write_file(
                &file,
                b" world",
                WriteOptions {
                    append: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(link.read_file(&file).await.unwrap(), b"hello world");

            // A taken name gets a number, the way a received file's does.
            let second = link
                .write_file(
                    &file,
                    b"again",
                    WriteOptions {
                        unique: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(second.path, remote_join(&scratch_str, "hello (2).bin"));

            let listing = link.list_dir(&scratch_str).await.unwrap();
            assert_eq!(listing.path, scratch_str);
            let names: Vec<_> = listing
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect();
            assert_eq!(names, ["hello (2).bin", "hello.bin"]);
            assert!(link.list_dir(&file).await.is_err());
            assert!(
                link.read_file(&remote_join(&scratch_str, "missing"))
                    .await
                    .is_err()
            );

            // The device list is whatever this machine has; the call
            // itself has to come back.
            let _ports = link.list_ports().await.unwrap();
        });
        let _ = fs::remove_dir_all(scratch);

        // A port that does not exist is refused, on the tab's own channel.
        let (events_tx, events_rx) = smol::channel::unbounded();
        let commands = link.open_port(
            "/dev/serialX-no-such-device",
            &SerialConfiguration::default(),
            events_tx,
            Tap::default(),
        );
        match events_rx.recv_blocking().expect("a port event") {
            SerialEvent::Error(message) => assert!(message.contains("Unable to open"), "{message}"),
            _ => panic!("expected the open to fail"),
        }
        drop(commands);

        drop(connection);
        match events.recv_blocking().expect("the end") {
            RemoteEvent::Closed(_) => {}
            other => panic!("expected the connection to close, got {other:?}"),
        }
    }
}
