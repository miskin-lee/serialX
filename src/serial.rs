use std::{
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender},
    },
    thread,
    time::Duration,
};

use gpui_kit::component::input::InputState;
use gpui_kit::{Entity, Subscription};
use serde::{Deserialize, Serialize};

use crate::filter::OutputFilter;
use crate::find::{FindState, FindView};
use crate::terminal::Terminal;
use crate::theme::TagColor;

/// The rates the session dialog lists. Any other rate can be typed in; these
/// are the ones a device is most likely to want.
pub(crate) const BAUD_RATES: &[u32] = &[
    1_200, 2_400, 4_800, 9_600, 19_200, 38_400, 57_600, 115_200, 230_400, 460_800, 921_600,
    1_500_000, 2_000_000,
];
/// The rate a new session opens at: what nearly every module and board
/// speaks out of the box.
pub(crate) const DEFAULT_BAUD_RATE: u32 = 115_200;
/// The list a saved session's `baud_index` pointed into before rates were
/// stored as numbers. Kept so an older workspace file still opens its
/// sessions at the rates they saved.
const LEGACY_BAUD_RATES: &[u32] = &[9_600, 19_200, 38_400, 57_600, 115_200, 230_400];
pub(crate) const DATA_BITS: &[&str] = &["5", "6", "7", "8"];
pub(crate) const STOP_BITS: &[&str] = &["1", "2"];
pub(crate) const PARITIES: &[&str] = &["None", "Odd", "Even"];
pub(crate) const FLOW_CONTROLS: &[&str] = &["None", "Software", "Hardware"];

/// What kind of device sits behind a port. Picks the glyph a port is drawn
/// with, the way the subtitle picks its words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PortKind {
    Usb,
    Bluetooth,
    Pci,
    Unknown,
    /// A port a saved or configured session names that is not attached now.
    Unavailable,
}

#[derive(Clone)]
pub(crate) struct PortItem {
    pub(crate) name: String,
    pub(crate) subtitle: String,
    pub(crate) kind: PortKind,
}

impl PortItem {
    /// A port named by a saved session or a configured tab that is not attached now.
    pub(crate) fn unavailable(name: String, subtitle: &str) -> Self {
        Self {
            name,
            subtitle: subtitle.into(),
            kind: PortKind::Unavailable,
        }
    }
}

pub(crate) enum SerialCommand {
    Write(Vec<u8>),
    Stop,
}

pub(crate) enum SerialEvent {
    Connected,
    Data(Vec<u8>),
    Error(String),
}

/// How long a read waits for bytes before checking whether it should stop.
const READ_TIMEOUT: Duration = Duration::from_millis(20);

/// The parameters a port is opened with. The baud rate is the number itself,
/// since it can be any the device asks for; the frame is stored as indices
/// into the fixed lists above.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(from = "StoredConfiguration")]
pub(crate) struct SerialConfiguration {
    pub(crate) baud_rate: u32,
    pub(crate) data_bits_index: usize,
    pub(crate) stop_bits_index: usize,
    pub(crate) parity_index: usize,
    pub(crate) flow_control_index: usize,
}

impl Default for SerialConfiguration {
    fn default() -> Self {
        Self {
            baud_rate: DEFAULT_BAUD_RATE,
            data_bits_index: 3,
            stop_bits_index: 0,
            parity_index: 0,
            flow_control_index: 0,
        }
    }
}

/// A configuration as the workspace file holds it, one release of history
/// deep: a file written before custom rates carries `baud_index` instead of
/// `baud_rate`, and is read back through the list that index pointed into.
#[derive(Deserialize)]
struct StoredConfiguration {
    baud_rate: Option<u32>,
    baud_index: Option<usize>,
    data_bits_index: usize,
    stop_bits_index: usize,
    parity_index: usize,
    flow_control_index: usize,
}

impl From<StoredConfiguration> for SerialConfiguration {
    fn from(stored: StoredConfiguration) -> Self {
        let baud_rate = stored
            .baud_rate
            .or_else(|| {
                stored
                    .baud_index
                    .and_then(|index| LEGACY_BAUD_RATES.get(index).copied())
            })
            .unwrap_or(DEFAULT_BAUD_RATE);
        Self {
            baud_rate,
            data_bits_index: stored.data_bits_index,
            stop_bits_index: stored.stop_bits_index,
            parity_index: stored.parity_index,
            flow_control_index: stored.flow_control_index,
        }
    }
}

/// Why a typed baud rate cannot be used, worded for the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BaudRateError {
    Empty,
    NotANumber,
    Zero,
    TooLarge,
}

impl BaudRateError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Empty => "Enter a rate, or pick one",
            Self::NotANumber => "Whole number of bits per second",
            Self::Zero => "Has to be greater than zero",
            Self::TooLarge => "Too large for a serial port",
        }
    }
}

/// Reads a baud rate as typed: digits only, greater than zero, and no more
/// than a port can be asked for. Surrounding whitespace is forgiven.
pub(crate) fn parse_baud_rate(text: &str) -> Result<u32, BaudRateError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(BaudRateError::Empty);
    }
    if !text.chars().all(|character| character.is_ascii_digit()) {
        return Err(BaudRateError::NotANumber);
    }
    match text.parse::<u32>() {
        Ok(0) => Err(BaudRateError::Zero),
        Ok(rate) => Ok(rate),
        Err(_) => Err(BaudRateError::TooLarge),
    }
}

/// Whether a rate is one of the standard ones the dialog lists.
pub(crate) fn is_listed_baud_rate(rate: u32) -> bool {
    BAUD_RATES.contains(&rate)
}

impl SerialConfiguration {
    pub(crate) fn sanitized(mut self) -> Self {
        if self.baud_rate == 0 {
            self.baud_rate = DEFAULT_BAUD_RATE;
        }
        self.data_bits_index = self.data_bits_index.min(DATA_BITS.len() - 1);
        self.stop_bits_index = self.stop_bits_index.min(STOP_BITS.len() - 1);
        self.parity_index = self.parity_index.min(PARITIES.len() - 1);
        self.flow_control_index = self.flow_control_index.min(FLOW_CONTROLS.len() - 1);
        self
    }

    pub(crate) fn baud_rate(self) -> u32 {
        self.baud_rate
    }

    fn data_bits(self) -> serialport::DataBits {
        match self.data_bits_index {
            0 => serialport::DataBits::Five,
            1 => serialport::DataBits::Six,
            2 => serialport::DataBits::Seven,
            _ => serialport::DataBits::Eight,
        }
    }

    fn stop_bits(self) -> serialport::StopBits {
        match self.stop_bits_index {
            1 => serialport::StopBits::Two,
            _ => serialport::StopBits::One,
        }
    }

    fn parity(self) -> serialport::Parity {
        match self.parity_index {
            1 => serialport::Parity::Odd,
            2 => serialport::Parity::Even,
            _ => serialport::Parity::None,
        }
    }

    fn flow_control(self) -> serialport::FlowControl {
        match self.flow_control_index {
            1 => serialport::FlowControl::Software,
            2 => serialport::FlowControl::Hardware,
            _ => serialport::FlowControl::None,
        }
    }

    pub(crate) fn summary(self) -> String {
        let parity = match self.parity_index {
            1 => 'O',
            2 => 'E',
            _ => 'N',
        };
        format!(
            "{} {}{}{}",
            self.baud_rate(),
            DATA_BITS[self.data_bits_index],
            parity,
            STOP_BITS[self.stop_bits_index]
        )
    }
}

pub(crate) struct SerialTabState {
    pub(crate) id: usize,
    pub(crate) ports: Vec<PortItem>,
    pub(crate) selected_port: usize,
    pub(crate) configuration: SerialConfiguration,
    /// The colour the tab is tagged with, chosen in the session dialog.
    pub(crate) color: TagColor,
    /// The name the tab was given, if any; the port's path stands in for it.
    pub(crate) alias: Option<String>,
    /// The saved group the session was filed under when it was made or
    /// opened, so saving it again keeps it there.
    pub(crate) group: Option<u64>,
    pub(crate) connected: bool,
    pub(crate) connecting: bool,
    pub(crate) paused: bool,
    /// Whether the composer's line goes out as the hex bytes it spells,
    /// rather than as text.
    pub(crate) hex_mode: bool,
    /// What follows the line, kept once per encoding: a switch to HEX and
    /// back does not forget which ending the device wanted for its text.
    pub(crate) text_line_ending: LineEnding,
    pub(crate) hex_line_ending: LineEnding,
    pub(crate) timestamps: bool,
    /// Whether the log's lines are numbered in the gutter.
    pub(crate) line_numbers: bool,
    /// Whether plain output is coloured by what it says — levels, times,
    /// addresses — on top of any colour the device sent itself.
    pub(crate) highlight: bool,
    pub(crate) auto_scroll: bool,
    pub(crate) terminal: Terminal,
    /// The title bar filter box and what it currently holds back.
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) filter: OutputFilter,
    /// The find bar over the terminal, and its box.
    pub(crate) find: FindState,
    pub(crate) find_input: Entity<InputState>,
    pub(crate) command_tx: Option<Sender<SerialCommand>>,
    /// Where the port's threads report to. The receiving end is taken by
    /// the task that feeds the terminal, the moment the tab is built.
    pub(crate) event_tx: smol::channel::Sender<SerialEvent>,
    event_rx: Option<smol::channel::Receiver<SerialEvent>>,
    _filter_subscription: Subscription,
    _find_subscription: Subscription,
}

impl SerialTabState {
    /// A tab with its boxes wired, keeping `scrollback` lines of log.
    pub(crate) fn new(
        id: usize,
        filter_input: Entity<InputState>,
        filter_subscription: Subscription,
        find_input: Entity<InputState>,
        find_subscription: Subscription,
        scrollback: usize,
    ) -> Self {
        let (event_tx, event_rx) = smol::channel::unbounded();
        Self {
            id,
            ports: discover_ports(),
            selected_port: 0,
            configuration: SerialConfiguration::default(),
            color: TagColor::default(),
            alias: None,
            group: None,
            connected: false,
            connecting: false,
            paused: false,
            hex_mode: false,
            text_line_ending: LineEnding::default_for(false),
            hex_line_ending: LineEnding::default_for(true),
            timestamps: true,
            line_numbers: true,
            highlight: true,
            auto_scroll: true,
            terminal: {
                let mut terminal = Terminal::new(scrollback);
                terminal.note("Configure the serial port, then connect.", &now());
                terminal
            },
            filter_input,
            filter: OutputFilter::default(),
            find: FindState::default(),
            find_input,
            command_tx: None,
            event_tx,
            event_rx: Some(event_rx),
            _filter_subscription: filter_subscription,
            _find_subscription: find_subscription,
        }
    }

    pub(crate) fn selected_port(&self) -> &PortItem {
        &self.ports[self.selected_port.min(self.ports.len().saturating_sub(1))]
    }

    /// What follows a line sent in the encoding the composer is in.
    pub(crate) fn line_ending(&self) -> LineEnding {
        if self.hex_mode {
            self.hex_line_ending
        } else {
            self.text_line_ending
        }
    }

    /// Sets the ending for the encoding the composer is in; the other
    /// encoding keeps its own.
    pub(crate) fn set_line_ending(&mut self, ending: LineEnding) {
        if self.hex_mode {
            self.hex_line_ending = ending;
        } else {
            self.text_line_ending = ending;
        }
    }

    /// What the tab is called: the alias it was given, else the port's path.
    pub(crate) fn title(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.selected_port().name)
    }

    pub(crate) fn take_events(&mut self) -> Option<smol::channel::Receiver<SerialEvent>> {
        self.event_rx.take()
    }

    /// Prints a line of the workbench's own in the terminal.
    pub(crate) fn note(&mut self, text: impl AsRef<str>) {
        self.terminal.note(text.as_ref(), &now());
    }

    /// Hands what the port read to the terminal, and sends back whatever
    /// the terminal answers with.
    pub(crate) fn receive(&mut self, bytes: &[u8]) {
        let answer = self.terminal.receive(bytes, &now());
        if !answer.is_empty() {
            self.write(answer);
        }
    }

    /// Hands bytes to the port. Nothing happens when the tab is not open.
    pub(crate) fn write(&self, bytes: Vec<u8>) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(SerialCommand::Write(bytes));
        }
    }

    pub(crate) fn disconnect(&mut self) {
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(SerialCommand::Stop);
        }
        self.connected = false;
        self.connecting = false;
    }
}

/// The wall clock, to the millisecond, as a line's timestamp reads.
fn now() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

impl Drop for SerialTabState {
    fn drop(&mut self) {
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(SerialCommand::Stop);
        }
    }
}

/// What the render pass reads of a tab: its connection, and the switches
/// that shape the terminal and the composer in the side panel. The port and
/// its tag are drawn from the tab itself, in the strip.
#[derive(Clone)]
pub(crate) struct SerialTabSnapshot {
    pub(crate) id: usize,
    pub(crate) connected: bool,
    pub(crate) connecting: bool,
    pub(crate) hex_mode: bool,
    pub(crate) line_ending: LineEnding,
    pub(crate) timestamps: bool,
    pub(crate) line_numbers: bool,
    /// How many rows on screen the title bar filter matches, out of how
    /// many there are, while a filter is set.
    pub(crate) filter_counts: Option<(usize, usize)>,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) filter: OutputFilter,
    pub(crate) find: FindView,
}

impl From<&SerialTabState> for SerialTabSnapshot {
    fn from(tab: &SerialTabState) -> Self {
        Self {
            id: tab.id,
            connected: tab.connected,
            connecting: tab.connecting,
            hex_mode: tab.hex_mode,
            line_ending: tab.line_ending(),
            timestamps: tab.timestamps,
            line_numbers: tab.line_numbers,
            filter_counts: tab.filter.is_active().then(|| {
                let texts = tab.terminal.visible_texts();
                let matching = texts.iter().filter(|text| tab.filter.matches(text)).count();
                (matching, texts.len())
            }),
            filter_input: tab.filter_input.clone(),
            filter: tab.filter.clone(),
            find: FindView {
                open: tab.find.open,
                matcher: tab.find.matcher.clone(),
                current: tab.find.current_span(),
                status: tab.find.status(),
                input: tab.find_input.clone(),
            },
        }
    }
}

/// The serial devices attached right now, in the order the system lists
/// them. Empty when there are none: the dialog says so, and a tab keeps the
/// port it was given.
pub(crate) fn discover_ports() -> Vec<PortItem> {
    let mut ports = Vec::new();

    if let Ok(detected) = serialport::available_ports() {
        ports.extend(detected.into_iter().map(|port| {
            let (subtitle, kind) = match port.port_type {
                serialport::SerialPortType::UsbPort(info) => (
                    info.product.unwrap_or_else(|| "USB Serial".into()),
                    PortKind::Usb,
                ),
                serialport::SerialPortType::BluetoothPort => {
                    ("Bluetooth Serial".into(), PortKind::Bluetooth)
                }
                serialport::SerialPortType::PciPort => ("PCI Serial".into(), PortKind::Pci),
                serialport::SerialPortType::Unknown => ("Serial Device".into(), PortKind::Unknown),
            };
            PortItem {
                name: port.port_name,
                subtitle,
                kind,
            }
        }));
    }
    ports
}

/// Opens the port and runs it on two threads: one reading, one writing.
///
/// A read blocks for up to its timeout, so a write that had to wait its
/// turn behind one would go out late by that much — a keystroke visibly
/// behind the finger. With a thread each, what you type goes out the moment
/// it is typed, and what arrives is passed on the moment it is read. The
/// writer stops when it is told to or when its channel closes with the tab;
/// the reader notices on its next timeout and follows.
pub(crate) fn spawn_serial_worker(
    port_name: String,
    configuration: SerialConfiguration,
    commands: Receiver<SerialCommand>,
    events: smol::channel::Sender<SerialEvent>,
) {
    thread::spawn(move || {
        let opened = serialport::new(&port_name, configuration.baud_rate())
            .data_bits(configuration.data_bits())
            .stop_bits(configuration.stop_bits())
            .parity(configuration.parity())
            .flow_control(configuration.flow_control())
            .timeout(READ_TIMEOUT)
            .open()
            .and_then(|reader| reader.try_clone().map(|writer| (reader, writer)));
        let (mut reader, mut writer) = match opened {
            Ok(ports) => ports,
            Err(error) => {
                let _ = events.send_blocking(SerialEvent::Error(format!(
                    "Unable to open {port_name}: {error}"
                )));
                return;
            }
        };
        let _ = events.send_blocking(SerialEvent::Connected);

        let stopped = Arc::new(AtomicBool::new(false));
        let writer_events = events.clone();
        let writer_stopped = stopped.clone();
        thread::spawn(move || {
            for command in commands {
                match command {
                    SerialCommand::Write(bytes) => {
                        if let Err(error) = writer.write_all(&bytes) {
                            let _ = writer_events
                                .send_blocking(SerialEvent::Error(format!("Send failed: {error}")));
                            break;
                        }
                    }
                    SerialCommand::Stop => break,
                }
            }
            writer_stopped.store(true, Ordering::Relaxed);
        });

        let mut buffer = [0_u8; 4096];
        while !stopped.load(Ordering::Relaxed) {
            match reader.read(&mut buffer) {
                Ok(count) if count > 0 => {
                    if events
                        .send_blocking(SerialEvent::Data(buffer[..count].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    let _ = events.send_blocking(SerialEvent::Error(format!("Read failed: {error}")));
                    break;
                }
            }
        }
    });
}

/// What follows a line the composer sends. Text goes out as a line, so it
/// ends the way the device expects one to: `\r\n` is what nearly every
/// serial console and AT modem wants, `\n` what a Unix shell on a UART
/// reads. Hex goes out as the frame it spells, with nothing added unless
/// asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LineEnding {
    #[default]
    CrLf,
    Lf,
    None,
}

impl LineEnding {
    /// Every ending, in the order the composer lists them.
    pub(crate) const ALL: [Self; 3] = [Self::CrLf, Self::Lf, Self::None];

    /// What a tab starts with in each encoding: a line after text, nothing
    /// after hex bytes.
    pub(crate) const fn default_for(hex: bool) -> Self {
        if hex { Self::None } else { Self::CrLf }
    }

    pub(crate) const fn bytes(self) -> &'static [u8] {
        match self {
            Self::CrLf => b"\r\n",
            Self::Lf => b"\n",
            Self::None => b"",
        }
    }

    /// Its name on the composer's switch.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::CrLf => "CRLF",
            Self::Lf => "LF",
            Self::None => "None",
        }
    }

    /// The bytes as code spells them, for the list behind the switch.
    pub(crate) const fn spelled(self) -> &'static str {
        match self {
            Self::CrLf => "\\r\\n",
            Self::Lf => "\\n",
            Self::None => "",
        }
    }
}

/// Why a line is not hex.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HexError {
    /// Something in it is neither a hex digit nor a space.
    NotHex,
    /// A digit short of whole bytes.
    OddDigit,
}

impl HexError {
    /// What to tell whoever typed it.
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::NotHex => "Not sent: hex takes only 0-9 and A-F, with spaces between bytes.",
            Self::OddDigit => "Not sent: hex bytes take two digits each.",
        }
    }
}

/// Reads a line of hex — `41 54 0D 0A`, or run together — as the bytes it
/// spells. Spaces are for the eye and are skipped; anything else, or a
/// digit short of a byte, is refused. Nothing spells no bytes.
pub(crate) fn parse_hex(value: &str) -> Result<Vec<u8>, HexError> {
    let compact: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
    if !compact.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(HexError::NotHex);
    }
    if !compact.len().is_multiple_of(2) {
        return Err(HexError::OddDigit);
    }
    Ok((0..compact.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&compact[index..index + 2], 16).unwrap_or(0))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{
        BAUD_RATES, BaudRateError, DEFAULT_BAUD_RATE, HexError, LineEnding, SerialConfiguration,
        is_listed_baud_rate, parse_baud_rate, parse_hex,
    };

    #[test]
    fn parses_hex_with_or_without_spaces() {
        assert_eq!(parse_hex("41 54 0D 0A"), Ok(b"AT\r\n".to_vec()));
        assert_eq!(parse_hex("41540d0a"), Ok(b"AT\r\n".to_vec()));
        assert_eq!(parse_hex("  "), Ok(Vec::new()));
        assert_eq!(parse_hex("123"), Err(HexError::OddDigit));
        assert_eq!(parse_hex("GG"), Err(HexError::NotHex));
        assert_eq!(parse_hex("4G"), Err(HexError::NotHex));
    }

    /// Text starts as a line, hex as bare bytes, and each ending is the
    /// bytes its name says.
    #[test]
    fn line_endings_spell_their_bytes() {
        assert_eq!(LineEnding::default_for(false), LineEnding::CrLf);
        assert_eq!(LineEnding::default_for(true), LineEnding::None);
        assert_eq!(LineEnding::CrLf.bytes(), b"\r\n");
        assert_eq!(LineEnding::Lf.bytes(), b"\n");
        assert_eq!(LineEnding::None.bytes(), b"");
        assert_eq!(LineEnding::ALL.map(LineEnding::label), ["CRLF", "LF", "None"]);
    }

    #[test]
    fn a_typed_rate_is_a_positive_whole_number() {
        assert_eq!(parse_baud_rate("115200"), Ok(115_200));
        assert_eq!(parse_baud_rate("  250000 "), Ok(250_000));
        assert_eq!(parse_baud_rate(""), Err(BaudRateError::Empty));
        assert_eq!(parse_baud_rate("   "), Err(BaudRateError::Empty));
        assert_eq!(parse_baud_rate("115,200"), Err(BaudRateError::NotANumber));
        assert_eq!(parse_baud_rate("-9600"), Err(BaudRateError::NotANumber));
        assert_eq!(parse_baud_rate("0"), Err(BaudRateError::Zero));
        assert_eq!(parse_baud_rate("99999999999"), Err(BaudRateError::TooLarge));
    }

    #[test]
    fn the_list_holds_the_default_and_knows_its_own() {
        assert!(BAUD_RATES.contains(&DEFAULT_BAUD_RATE));
        assert!(is_listed_baud_rate(9_600));
        assert!(!is_listed_baud_rate(250_000));
        assert!(BAUD_RATES.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// A workspace written before custom rates stored the rate as an index
    /// into a six-entry list; it reads back as the rate that index named.
    #[test]
    fn an_older_workspace_keeps_the_rate_its_index_named() {
        let stored: SerialConfiguration = serde_json::from_str(
            r#"{"baud_index":5,"data_bits_index":3,"stop_bits_index":0,"parity_index":0,"flow_control_index":0}"#,
        )
        .unwrap();
        assert_eq!(stored.baud_rate(), 230_400);

        let out_of_range: SerialConfiguration = serde_json::from_str(
            r#"{"baud_index":9,"data_bits_index":3,"stop_bits_index":0,"parity_index":0,"flow_control_index":0}"#,
        )
        .unwrap();
        assert_eq!(out_of_range.baud_rate(), DEFAULT_BAUD_RATE);
    }

    #[test]
    fn a_custom_rate_round_trips_through_the_workspace_file() {
        let configuration = SerialConfiguration {
            baud_rate: 250_000,
            ..SerialConfiguration::default()
        };
        let json = serde_json::to_string(&configuration).unwrap();
        assert!(json.contains("\"baud_rate\":250000"));
        let restored: SerialConfiguration = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.baud_rate(), 250_000);
        assert_eq!(restored.summary(), "250000 8N1");
    }

    #[test]
    fn a_zero_rate_is_sanitized_to_the_default() {
        let configuration = SerialConfiguration {
            baud_rate: 0,
            ..SerialConfiguration::default()
        }
        .sanitized();
        assert_eq!(configuration.baud_rate(), DEFAULT_BAUD_RATE);
    }
}
