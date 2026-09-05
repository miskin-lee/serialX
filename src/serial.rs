use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use gpui_kit::component::input::InputState;
use gpui_kit::{Entity, Subscription};
use serde::{Deserialize, Serialize};

use crate::filter::OutputFilter;
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    Rx,
    Tx,
    System,
}

#[derive(Clone)]
pub(crate) struct TerminalLine {
    pub(crate) time: String,
    pub(crate) kind: LineKind,
    pub(crate) payload: Vec<u8>,
    pub(crate) note: Option<String>,
}

impl TerminalLine {
    /// The text the line prints as: its note, or the payload in the tab's
    /// mode. The filter matches against this, so what it reads is what you see.
    pub(crate) fn display_text(&self, hex_mode: bool) -> String {
        if let Some(note) = &self.note {
            return note.clone();
        }
        if hex_mode {
            self.payload
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            String::from_utf8_lossy(&self.payload)
                .trim_end_matches(['\r', '\n'])
                .to_string()
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
    Closed,
}

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
            Self::Empty => "Enter a rate, or pick one from the list",
            Self::NotANumber => "A rate is a whole number of bits per second",
            Self::Zero => "A rate has to be greater than zero",
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
    pub(crate) connected: bool,
    pub(crate) connecting: bool,
    pub(crate) paused: bool,
    pub(crate) hex_mode: bool,
    pub(crate) timestamps: bool,
    pub(crate) auto_scroll: bool,
    pub(crate) terminal_lines: Vec<TerminalLine>,
    clock_tick: usize,
    pub(crate) send_input: Entity<InputState>,
    /// The title bar filter box and what it currently holds back.
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) filter: OutputFilter,
    pub(crate) command_tx: Option<Sender<SerialCommand>>,
    pub(crate) event_tx: Sender<SerialEvent>,
    pub(crate) event_rx: Receiver<SerialEvent>,
    _input_subscription: Subscription,
    _filter_subscription: Subscription,
}

impl SerialTabState {
    pub(crate) fn new(
        id: usize,
        send_input: Entity<InputState>,
        input_subscription: Subscription,
        filter_input: Entity<InputState>,
        filter_subscription: Subscription,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            id,
            ports: discover_ports(),
            selected_port: 0,
            configuration: SerialConfiguration::default(),
            color: TagColor::default(),
            alias: None,
            connected: false,
            connecting: false,
            paused: false,
            hex_mode: false,
            timestamps: true,
            auto_scroll: true,
            terminal_lines: vec![TerminalLine {
                time: "14:32:40.018".into(),
                kind: LineKind::System,
                payload: Vec::new(),
                note: Some("Configure the serial port, then connect.".into()),
            }],
            clock_tick: 0,
            send_input,
            filter_input,
            filter: OutputFilter::default(),
            command_tx: None,
            event_tx,
            event_rx,
            _input_subscription: input_subscription,
            _filter_subscription: filter_subscription,
        }
    }

    pub(crate) fn selected_port(&self) -> &PortItem {
        &self.ports[self.selected_port.min(self.ports.len().saturating_sub(1))]
    }

    /// What the tab is called: the alias it was given, else the port's path.
    pub(crate) fn title(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.selected_port().name)
    }

    fn now(&mut self) -> String {
        self.clock_tick = self.clock_tick.wrapping_add(1);
        let seconds = 40 + (self.clock_tick % 19);
        format!("14:32:{seconds:02}.{:03}", (self.clock_tick * 73) % 1000)
    }

    pub(crate) fn push_line(&mut self, kind: LineKind, payload: Vec<u8>, note: Option<String>) {
        let time = self.now();
        self.terminal_lines.push(TerminalLine {
            time,
            kind,
            payload,
            note,
        });
        if self.terminal_lines.len() > 400 {
            self.terminal_lines.drain(..80);
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

impl Drop for SerialTabState {
    fn drop(&mut self) {
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(SerialCommand::Stop);
        }
    }
}

/// What the render pass reads of a tab: the log and the switches over it.
/// The port and its tag are drawn from the tab itself, in the strip.
#[derive(Clone)]
pub(crate) struct SerialTabSnapshot {
    pub(crate) id: usize,
    pub(crate) connected: bool,
    pub(crate) connecting: bool,
    pub(crate) paused: bool,
    pub(crate) hex_mode: bool,
    pub(crate) timestamps: bool,
    pub(crate) auto_scroll: bool,
    pub(crate) terminal_lines: Vec<TerminalLine>,
    pub(crate) send_input: Entity<InputState>,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) filter: OutputFilter,
}

impl SerialTabSnapshot {
    /// The lines the title bar filter lets through, with their positions in
    /// the log. An idle filter skips the text formatting altogether.
    pub(crate) fn visible_lines(&self) -> impl Iterator<Item = (usize, &TerminalLine)> {
        let active = self.filter.is_active();
        self.terminal_lines
            .iter()
            .enumerate()
            .filter(move |(_, line)| {
                !active || self.filter.matches(&line.display_text(self.hex_mode))
            })
    }
}

impl From<&SerialTabState> for SerialTabSnapshot {
    fn from(tab: &SerialTabState) -> Self {
        Self {
            id: tab.id,
            connected: tab.connected,
            connecting: tab.connecting,
            paused: tab.paused,
            hex_mode: tab.hex_mode,
            timestamps: tab.timestamps,
            auto_scroll: tab.auto_scroll,
            terminal_lines: tab.terminal_lines.clone(),
            send_input: tab.send_input.clone(),
            filter_input: tab.filter_input.clone(),
            filter: tab.filter.clone(),
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

pub(crate) fn spawn_serial_worker(
    port_name: String,
    configuration: SerialConfiguration,
    commands: Receiver<SerialCommand>,
    events: Sender<SerialEvent>,
) {
    thread::spawn(move || {
        let mut port = match serialport::new(&port_name, configuration.baud_rate())
            .data_bits(configuration.data_bits())
            .stop_bits(configuration.stop_bits())
            .parity(configuration.parity())
            .flow_control(configuration.flow_control())
            .timeout(Duration::from_millis(24))
            .open()
        {
            Ok(port) => port,
            Err(error) => {
                let _ = events.send(SerialEvent::Error(format!(
                    "Unable to open {port_name}: {error}"
                )));
                return;
            }
        };

        let _ = events.send(SerialEvent::Connected);
        let mut buffer = [0_u8; 2048];
        loop {
            while let Ok(command) = commands.try_recv() {
                match command {
                    SerialCommand::Write(bytes) => {
                        if let Err(error) = port.write_all(&bytes) {
                            let _ =
                                events.send(SerialEvent::Error(format!("Send failed: {error}")));
                            return;
                        }
                    }
                    SerialCommand::Stop => {
                        let _ = events.send(SerialEvent::Closed);
                        return;
                    }
                }
            }

            match port.read(&mut buffer) {
                Ok(count) if count > 0 => {
                    let _ = events.send(SerialEvent::Data(buffer[..count].to_vec()));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    let _ = events.send(SerialEvent::Error(format!("Read failed: {error}")));
                    return;
                }
            }
        }
    });
}

pub(crate) fn parse_hex(value: &str) -> Option<Vec<u8>> {
    let compact: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.is_empty() || !compact.len().is_multiple_of(2) {
        return None;
    }
    (0..compact.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&compact[index..index + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        BAUD_RATES, BaudRateError, DEFAULT_BAUD_RATE, SerialConfiguration, is_listed_baud_rate,
        parse_baud_rate, parse_hex,
    };

    #[test]
    fn parses_hex_with_or_without_spaces() {
        assert_eq!(parse_hex("41 54 0D 0A"), Some(b"AT\r\n".to_vec()));
        assert_eq!(parse_hex("41540d0a"), Some(b"AT\r\n".to_vec()));
        assert_eq!(parse_hex("123"), None);
        assert_eq!(parse_hex("GG"), None);
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
