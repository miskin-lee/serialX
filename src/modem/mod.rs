//! File transfer over the port: XMODEM, YMODEM and ZMODEM.
//!
//! A serial console is where firmware goes in and dumps come out, and the
//! protocols the bootloaders and the shells speak for that are three, all
//! from the modem days. XMODEM sends a file in 128-byte blocks, each
//! acknowledged before the next; it carries no name and no size, so the
//! receiver names the file and the last block is padded out. XMODEM-1K is
//! the same with 1024-byte blocks. YMODEM sends the name and the size
//! ahead of the data, in a block of its own, and can send several files in
//! one run. ZMODEM streams — the sender does not wait for an answer to
//! every block — with a 32-bit check on every packet and a way for the
//! receiver to say where to start again after an error; it also announces
//! itself, so a terminal can start receiving when the device runs `sz`
//! and offer a file when it runs `rz`.
//!
//! A transfer runs on a thread of its own, the way `rz` and `sz` are
//! programs of their own. While one is running the port's reader hands
//! what it reads to the transfer instead of to the terminal, and the
//! transfer writes through the port's writer: [`Link`] is that pair — a
//! channel in, a closure out — with the timeouts and the cancel flag, and
//! [`start`] runs a [`TransferJob`] over one, reporting in
//! [`TransferEvent`]s. Everything above the link is protocol and nothing
//! else, so the tests run a sender and a receiver against each other over
//! a pair of channels, with a byte flipped now and then to see the
//! recovery work.

mod crc;
mod xymodem;
mod zmodem;

use std::{
    collections::VecDeque,
    fmt, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// The protocols on offer, in the order the dialog lists them: oldest and
/// most widely spoken first. Kept with the settings as the one chosen
/// last, so it is named for the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Protocol {
    XModem,
    XModem1k,
    YModem,
    ZModem,
}

impl Protocol {
    pub(crate) const ALL: [Self; 4] = [Self::XModem, Self::XModem1k, Self::YModem, Self::ZModem];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::XModem => "XMODEM",
            Self::XModem1k => "XMODEM-1K",
            Self::YModem => "YMODEM",
            Self::ZModem => "ZMODEM",
        }
    }

    /// Whether the file's name and size travel with it. XMODEM's do not:
    /// the receiver names the file, and the last block is padded.
    pub(crate) fn carries_name(self) -> bool {
        matches!(self, Self::YModem | Self::ZModem)
    }

    /// Whether several files can go in one run.
    pub(crate) fn batches(self) -> bool {
        self.carries_name()
    }

    /// A line for the dialog, saying what the choice means.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::XModem => {
                "128-byte blocks, each answered before the next: the oldest, and what nearly every bootloader speaks. The name and size do not go along, so the receiver names the file and the last block is padded out."
            }
            Self::XModem1k => {
                "XMODEM with 1024-byte blocks: eight times fewer answers to wait for, on a link that can take them."
            }
            Self::YModem => {
                "1K blocks with the name and size sent ahead, so the file arrives whole and named; several files can go in one run."
            }
            Self::ZModem => {
                "Streams without waiting for an answer to every block, with a 32-bit check on every packet and a restart from where an error was. Starts by itself when the device runs sz or rz."
            }
        }
    }
}

/// Which way the file goes, seen from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Send,
    Receive,
}

/// What a transfer is asked to do.
#[derive(Clone, Debug)]
pub(crate) enum TransferJob {
    /// Send these files; a protocol that does not batch sends the first.
    Send {
        protocol: Protocol,
        paths: Vec<PathBuf>,
    },
    /// Receive into this folder, under `name` when the protocol carries
    /// none of its own.
    Receive {
        protocol: Protocol,
        folder: PathBuf,
        name: Option<String>,
    },
}

impl TransferJob {
    pub(crate) fn protocol(&self) -> Protocol {
        match self {
            Self::Send { protocol, .. } | Self::Receive { protocol, .. } => *protocol,
        }
    }

    pub(crate) fn direction(&self) -> Direction {
        match self {
            Self::Send { .. } => Direction::Send,
            Self::Receive { .. } => Direction::Receive,
        }
    }
}

/// What the link carried, both ways: the protocol's overhead included,
/// since that is what the port saw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Wire {
    pub(crate) read: u64,
    pub(crate) written: u64,
}

/// What the transfer's thread reports, in order.
#[derive(Clone, Debug)]
pub(crate) enum TransferEvent {
    /// A file is beginning: its name, and its size when the protocol
    /// says it.
    File { name: String, size: Option<u64> },
    /// How much of the current file is done.
    Progress { done: u64, wire: Wire },
    /// The run is over: the files that went, with their sizes.
    Finished {
        files: Vec<(String, u64)>,
        elapsed: Duration,
        wire: Wire,
    },
    /// The run did not complete.
    Failed {
        reason: String,
        cancelled: bool,
        wire: Wire,
    },
}

/// Why a transfer stopped short.
#[derive(Debug)]
pub(crate) enum Failure {
    /// Cancelled from this end.
    Cancelled,
    /// Called off by the other end.
    Aborted,
    /// The other end stopped answering; says what was waited for.
    Timeout(&'static str),
    /// The other end said something the protocol has no place for.
    Protocol(String),
    /// A file or the port could not be read or written.
    Io(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::Aborted => write!(f, "called off by the other side"),
            Self::Timeout(what) => write!(f, "gave up waiting for {what}"),
            Self::Protocol(reason) | Self::Io(reason) => write!(f, "{reason}"),
        }
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// How long the protocols wait, and how often they try again. The
/// defaults are the ones the protocols were written with; the tests use
/// far shorter ones.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    /// The longest wait for the other side's next word, once it is
    /// talking, before one of the retries is spent.
    pub(crate) reply: Duration,
    /// How many times an unanswered word is repeated before giving up.
    pub(crate) retries: u32,
    /// How often the first word is repeated while the other side has
    /// not begun — the time it takes to type `sz` at the device.
    pub(crate) greet: Duration,
    /// How many greetings go unanswered before giving up.
    pub(crate) greetings: u32,
    /// How long the line has to be quiet after an error before it is
    /// answered, so the answer does not land in the middle of the block
    /// that went wrong.
    pub(crate) settle: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            reply: Duration::from_secs(10),
            retries: 10,
            greet: Duration::from_secs(3),
            greetings: 20,
            settle: Duration::from_secs(1),
        }
    }
}

#[cfg(test)]
impl Timing {
    fn fast() -> Self {
        Self {
            reply: Duration::from_millis(300),
            retries: 3,
            greet: Duration::from_millis(100),
            greetings: 5,
            settle: Duration::from_millis(20),
        }
    }
}

/// Where the port's reader is told to send what it reads while a transfer
/// runs: a channel's sending end, or nothing while there is no transfer.
pub(crate) type Tap = Arc<Mutex<Option<Sender<Vec<u8>>>>>;

type Outgoing = Box<dyn FnMut(&[u8]) -> io::Result<()> + Send>;
type Leftover = Box<dyn FnOnce(Vec<u8>) + Send>;

/// How long a wait is cut into, so a cancel is seen within that.
const CANCEL_POLL: Duration = Duration::from_millis(100);
/// What is sent to call the other side off: CANs, and backspaces to wipe
/// them from a shell that echoed them.
const CALL_OFF: &[u8] = b"\x18\x18\x18\x18\x18\x18\x18\x18\x08\x08\x08\x08\x08\x08\x08\x08";
/// The least time between two progress reports of one file.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// The transfer's end of the port.
pub(crate) struct Link {
    incoming: Receiver<Vec<u8>>,
    outgoing: Outgoing,
    /// What has come in and not been read yet.
    pending: VecDeque<u8>,
    cancel: Arc<AtomicBool>,
    pub(crate) timing: Timing,
    pub(crate) wire: Wire,
    /// The port's tap, to be cleared when the transfer is over.
    tap: Option<Tap>,
    /// Where what came in after the transfer goes: the device's words
    /// once it was back to being a console.
    leftover: Option<Leftover>,
}

impl Link {
    /// A link that takes over a port's reader through its tap, and hands
    /// the reader back when it is released — with whatever arrived in
    /// between given to `leftover`.
    pub(crate) fn tapping(
        tap: &Tap,
        outgoing: impl FnMut(&[u8]) -> io::Result<()> + Send + 'static,
        cancel: Arc<AtomicBool>,
        leftover: impl FnOnce(Vec<u8>) + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        *tap.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tx);
        Self {
            incoming: rx,
            outgoing: Box::new(outgoing),
            pending: VecDeque::new(),
            cancel,
            timing: Timing::default(),
            wire: Wire::default(),
            tap: Some(tap.clone()),
            leftover: Some(Box::new(leftover)),
        }
    }

    #[cfg(test)]
    fn over(
        incoming: Receiver<Vec<u8>>,
        outgoing: impl FnMut(&[u8]) -> io::Result<()> + Send + 'static,
        cancel: Arc<AtomicBool>,
        timing: Timing,
    ) -> Self {
        Self {
            incoming,
            outgoing: Box::new(outgoing),
            pending: VecDeque::new(),
            cancel,
            timing,
            wire: Wire::default(),
            tap: None,
            leftover: None,
        }
    }

    fn check(&self) -> Result<(), Failure> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(Failure::Cancelled)
        } else {
            Ok(())
        }
    }

    /// The next byte, or none within the timeout.
    fn read_byte(&mut self, timeout: Duration) -> Result<Option<u8>, Failure> {
        let deadline = Instant::now() + timeout;
        loop {
            self.check()?;
            if self.pending.is_empty()
                && let Ok(bytes) = self.incoming.try_recv()
            {
                self.took(bytes);
            }
            if let Some(byte) = self.pending.pop_front() {
                return Ok(Some(byte));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            match self.incoming.recv_timeout((deadline - now).min(CANCEL_POLL)) {
                Ok(bytes) => self.took(bytes),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Failure::Io("the port closed".into()));
                }
            }
        }
    }

    /// Puts a byte back, to be read next: for a look at what follows a
    /// header that turned out to belong to something else.
    fn unread(&mut self, byte: u8) {
        self.pending.push_front(byte);
    }

    /// Fills the buffer, or says it could not within the timeout.
    fn read_exact(&mut self, buffer: &mut [u8], timeout: Duration) -> Result<bool, Failure> {
        let deadline = Instant::now() + timeout;
        for slot in buffer.iter_mut() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.read_byte(remaining)? {
                Some(byte) => *slot = byte,
                None => return Ok(false),
            }
        }
        Ok(true)
    }

    /// Whether anything has arrived that has not been read.
    fn has_pending(&mut self) -> bool {
        if !self.pending.is_empty() {
            return true;
        }
        match self.incoming.try_recv() {
            Ok(bytes) => {
                self.took(bytes);
                true
            }
            Err(_) => false,
        }
    }

    fn took(&mut self, bytes: Vec<u8>) {
        self.wire.read = self.wire.read.saturating_add(bytes.len() as u64);
        self.pending.extend(bytes);
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        self.check()?;
        (self.outgoing)(bytes).map_err(|error| Failure::Io(format!("send failed: {error}")))?;
        self.wire.written = self.wire.written.saturating_add(bytes.len() as u64);
        Ok(())
    }

    /// Throws away what has arrived and waits for the line to go quiet,
    /// so an answer to a bad block is not sent into the rest of it.
    fn settle(&mut self) -> Result<(), Failure> {
        let quiet = self.timing.settle;
        let deadline = Instant::now() + quiet * 10;
        while Instant::now() < deadline {
            if self.read_byte(quiet)?.is_none() {
                break;
            }
        }
        Ok(())
    }

    /// Tells the other side to stop, whatever state this side is in.
    fn call_off(&mut self) {
        let _ = (self.outgoing)(CALL_OFF);
    }

    /// Hands the port back to its terminal, with what arrived meanwhile.
    fn release(&mut self) {
        if let Some(tap) = self.tap.take() {
            *tap.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
        let mut leftover: Vec<u8> = self.pending.drain(..).collect();
        while let Ok(bytes) = self.incoming.try_recv() {
            leftover.extend(bytes);
        }
        if let Some(handoff) = self.leftover.take() {
            handoff(leftover);
        }
    }
}

/// Where the engines report to, with the progress held to a rate the
/// screen can use.
pub(crate) struct Reporter {
    send: Box<dyn FnMut(TransferEvent) + Send>,
    last_progress: Option<Instant>,
}

impl Reporter {
    fn new(send: Box<dyn FnMut(TransferEvent) + Send>) -> Self {
        Self {
            send,
            last_progress: None,
        }
    }

    fn file(&mut self, name: &str, size: Option<u64>) {
        self.last_progress = None;
        (self.send)(TransferEvent::File {
            name: name.to_string(),
            size,
        });
    }

    fn progress(&mut self, done: u64, wire: Wire) {
        if self
            .last_progress
            .is_some_and(|last| last.elapsed() < PROGRESS_INTERVAL)
        {
            return;
        }
        self.last_progress = Some(Instant::now());
        (self.send)(TransferEvent::Progress { done, wire });
    }

    fn emit(&mut self, event: TransferEvent) {
        (self.send)(event);
    }
}

/// Runs the job on a thread of its own, reporting as it goes. The link is
/// released when the job is over, however it ended.
pub(crate) fn start(
    job: TransferJob,
    link: Link,
    report: impl FnMut(TransferEvent) + Send + 'static,
) {
    thread::spawn(move || {
        let _ = run(job, link, Reporter::new(Box::new(report)));
    });
}

fn run(job: TransferJob, mut link: Link, mut reporter: Reporter) -> Result<Vec<(String, u64)>, Failure> {
    let started = Instant::now();
    let outcome = match &job {
        TransferJob::Send {
            protocol: Protocol::ZModem,
            paths,
        } => zmodem::send(&mut link, paths, &mut reporter),
        TransferJob::Send { protocol, paths } => {
            xymodem::send(&mut link, *protocol, paths, &mut reporter)
        }
        TransferJob::Receive {
            protocol: Protocol::ZModem,
            folder,
            ..
        } => zmodem::receive(&mut link, folder, &mut reporter),
        TransferJob::Receive {
            protocol,
            folder,
            name,
        } => xymodem::receive(&mut link, *protocol, folder, name.as_deref(), &mut reporter),
    };
    if matches!(outcome, Err(Failure::Cancelled)) {
        link.call_off();
    }
    let wire = link.wire;
    link.release();
    match &outcome {
        Ok(files) => reporter.emit(TransferEvent::Finished {
            files: files.clone(),
            elapsed: started.elapsed(),
            wire,
        }),
        Err(failure) => reporter.emit(TransferEvent::Failed {
            reason: failure.to_string(),
            cancelled: matches!(failure, Failure::Cancelled),
            wire,
        }),
    }
    outcome
}

/// Listens on what the device says for ZMODEM announcing itself. `sz`
/// opens with a ZRQINIT header — `**\x18B00…` — which means the device
/// has a file to send and this end should receive; `rz` opens with
/// ZRINIT — `**\x18B01…` — which means the device is waiting for a file.
/// The header may arrive split across reads, so the last few bytes of
/// each are kept for the next.
#[derive(Default)]
pub(crate) struct Sniffer {
    tail: Vec<u8>,
}

impl Sniffer {
    const SIGNATURE: [u8; 4] = [b'*', zmodem::ZDLE, b'B', b'0'];

    pub(crate) fn hear(&mut self, bytes: &[u8]) -> Option<Direction> {
        let mut window = std::mem::take(&mut self.tail);
        window.extend_from_slice(bytes);
        let heard = window.windows(5).find_map(|run| {
            if run[..4] != Self::SIGNATURE {
                return None;
            }
            match run[4] {
                b'0' => Some(Direction::Receive),
                b'1' => Some(Direction::Send),
                _ => None,
            }
        });
        let keep = window.len().saturating_sub(4);
        self.tail = window[keep..].to_vec();
        if heard.is_some() {
            self.tail.clear();
        }
        heard
    }
}

/// The name a file goes out under: the last part of its path.
fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "file".into())
}

/// The name a received file is written under: what the sender said, less
/// any directory it named — a name from the wire must not climb out of
/// the folder — and something when it said nothing usable.
fn safe_name(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if name.is_empty() || name == "." || name == ".." {
        "received".into()
    } else {
        name.to_string()
    }
}

/// Where a received file goes: under its name, or with a number after it
/// when that name is taken, the way a browser files a second download.
fn unique_path(folder: &Path, name: &str) -> PathBuf {
    let first = folder.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    (2..)
        .map(|n| folder.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !candidate.exists())
        .expect("an unbounded range")
}

/// The name and size YMODEM and ZMODEM send ahead of a file:
/// `name NUL size mtime mode … NUL`. None when there is no name, which is
/// how YMODEM ends a batch.
fn parse_file_info(info: &[u8]) -> Option<(String, Option<u64>)> {
    let mut parts = info.splitn(3, |&byte| byte == 0);
    let name = parts.next().filter(|name| !name.is_empty())?;
    let name = safe_name(&String::from_utf8_lossy(name));
    let size = parts
        .next()
        .map(|rest| String::from_utf8_lossy(rest).into_owned())
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok());
    Some((name, size))
}

/// The seconds since the epoch a file was last written, for the info
/// the name goes out with; zero when the file system will not say.
fn modified(file: &std::fs::File) -> u64 {
    file.metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{self, ErrorKind},
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc,
        },
        thread,
    };

    use super::{
        Direction, Failure, Link, Protocol, Reporter, Sniffer, Timing, TransferEvent, TransferJob,
        parse_file_info, run, safe_name, unique_path,
    };

    static SCRATCH: AtomicUsize = AtomicUsize::new(0);

    /// A folder of this test's own under the system's temporary one.
    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "serialx-modem-{}-{name}-{}",
            std::process::id(),
            SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&folder).expect("a scratch folder");
        folder
    }

    /// What one side of a pair sends through, given a chance to tamper.
    type Tamper = Arc<Mutex<Box<dyn FnMut(usize, &mut Vec<u8>) + Send>>>;

    fn no_tamper() -> Tamper {
        Arc::new(Mutex::new(Box::new(|_, _| {})))
    }

    /// Two links wired to each other, each writing into the other's
    /// channel; `tamper` gets every write of the first, numbered.
    fn pair(timing: Timing, tamper: Tamper) -> (Link, Arc<AtomicBool>, Link, Arc<AtomicBool>) {
        let (a_tx, a_rx) = mpsc::channel::<Vec<u8>>();
        let (b_tx, b_rx) = mpsc::channel::<Vec<u8>>();
        let a_cancel = Arc::new(AtomicBool::new(false));
        let b_cancel = Arc::new(AtomicBool::new(false));
        let mut writes = 0;
        let a = Link::over(
            a_rx,
            move |bytes| {
                let mut bytes = bytes.to_vec();
                (tamper.lock().unwrap())(writes, &mut bytes);
                writes += 1;
                b_tx.send(bytes)
                    .map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "closed"))
            },
            a_cancel.clone(),
            timing,
        );
        let b = Link::over(
            b_rx,
            move |bytes| {
                a_tx.send(bytes.to_vec())
                    .map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "closed"))
            },
            b_cancel.clone(),
            timing,
        );
        (a, a_cancel, b, b_cancel)
    }

    fn quiet() -> Reporter {
        Reporter::new(Box::new(|_| {}))
    }

    /// Bytes that run through every value, so every escape gets a turn.
    fn every_byte(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 % 256) as u8).collect()
    }

    /// Sends `files` one way and receives them the other, over a pair of
    /// links, and says what each side made of it and where the received
    /// files went.
    #[allow(clippy::type_complexity)]
    fn exchange(
        protocol: Protocol,
        files: &[(&str, Vec<u8>)],
        name: Option<&str>,
        tamper: Tamper,
        sender_cancel: Option<Arc<AtomicBool>>,
    ) -> (
        Result<Vec<(String, u64)>, Failure>,
        Result<Vec<(String, u64)>, Failure>,
        PathBuf,
    ) {
        let outbox = scratch("out");
        let inbox = scratch("in");
        let mut paths = Vec::new();
        for (file_name, bytes) in files {
            let path = outbox.join(file_name);
            fs::write(&path, bytes).expect("a file to send");
            paths.push(path);
        }
        let (sender, cancel, receiver, _) = pair(Timing::fast(), tamper);
        let receive = TransferJob::Receive {
            protocol,
            folder: inbox.clone(),
            name: name.map(str::to_string),
        };
        let receiving = thread::spawn(move || run(receive, receiver, quiet()));
        let reporter = match sender_cancel {
            Some(flag) => {
                let _ = flag;
                Reporter::new(Box::new(move |event| {
                    if matches!(event, TransferEvent::Progress { .. }) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }))
            }
            None => quiet(),
        };
        let sent = run(TransferJob::Send { protocol, paths }, sender, reporter);
        let received = receiving.join().expect("the receiver's thread");
        if sent.is_err() || received.is_err() {
            eprintln!("sender: {sent:?}\nreceiver: {received:?}");
        }
        (sent, received, inbox)
    }

    #[test]
    fn xmodem_moves_a_file_padded_out_to_its_block() {
        let payload = every_byte(300);
        let (sent, received, inbox) = exchange(
            Protocol::XModem,
            &[("blob.bin", payload.clone())],
            Some("landed.bin"),
            no_tamper(),
            None,
        );
        assert_eq!(sent.unwrap(), vec![("blob.bin".to_string(), 300)]);
        assert_eq!(received.unwrap(), vec![("landed.bin".to_string(), 384)]);
        let landed = fs::read(inbox.join("landed.bin")).unwrap();
        assert_eq!(landed.len(), 384);
        assert_eq!(&landed[..300], &payload[..]);
        assert!(landed[300..].iter().all(|&byte| byte == 0x1A));
    }

    #[test]
    fn xmodem_1k_sends_long_blocks_and_a_short_tail() {
        // 2 × 1024 + 100: the tail is short enough for a 128-byte block.
        let payload = every_byte(2148);
        let (sent, received, inbox) = exchange(
            Protocol::XModem1k,
            &[("image.bin", payload.clone())],
            Some("image.bin"),
            no_tamper(),
            None,
        );
        assert!(sent.is_ok());
        assert_eq!(received.unwrap()[0].1, 2176);
        let landed = fs::read(inbox.join("image.bin")).unwrap();
        assert_eq!(&landed[..2148], &payload[..]);
    }

    #[test]
    fn ymodem_carries_names_and_sizes_for_a_batch() {
        let first = every_byte(1500);
        let second = every_byte(77);
        let (sent, received, inbox) = exchange(
            Protocol::YModem,
            &[("first.bin", first.clone()), ("second.txt", second.clone())],
            None,
            no_tamper(),
            None,
        );
        assert_eq!(
            sent.unwrap(),
            vec![("first.bin".to_string(), 1500), ("second.txt".to_string(), 77)]
        );
        assert_eq!(
            received.unwrap(),
            vec![("first.bin".to_string(), 1500), ("second.txt".to_string(), 77)]
        );
        assert_eq!(fs::read(inbox.join("first.bin")).unwrap(), first);
        assert_eq!(fs::read(inbox.join("second.txt")).unwrap(), second);
    }

    #[test]
    fn zmodem_moves_every_byte_value_and_a_batch() {
        let first = every_byte(5000);
        let second = every_byte(3);
        let (sent, received, inbox) = exchange(
            Protocol::ZModem,
            &[("firmware.bin", first.clone()), ("note.txt", second.clone())],
            None,
            no_tamper(),
            None,
        );
        assert_eq!(
            sent.unwrap(),
            vec![("firmware.bin".to_string(), 5000), ("note.txt".to_string(), 3)]
        );
        assert_eq!(received.unwrap().len(), 2);
        assert_eq!(fs::read(inbox.join("firmware.bin")).unwrap(), first);
        assert_eq!(fs::read(inbox.join("note.txt")).unwrap(), second);
    }

    #[test]
    fn zmodem_sends_an_empty_file() {
        let (sent, received, inbox) =
            exchange(Protocol::ZModem, &[("empty", Vec::new())], None, no_tamper(), None);
        assert_eq!(sent.unwrap(), vec![("empty".to_string(), 0)]);
        assert_eq!(received.unwrap(), vec![("empty".to_string(), 0)]);
        assert_eq!(fs::read(inbox.join("empty")).unwrap(), Vec::<u8>::new());
    }

    /// A byte flipped in a data packet is caught by its check; the
    /// receiver asks for that position again and the sender goes back.
    #[test]
    fn zmodem_starts_again_where_a_packet_went_bad() {
        let payload = every_byte(4000);
        let flipped = Arc::new(AtomicBool::new(false));
        let seen = flipped.clone();
        let tamper: Tamper = Arc::new(Mutex::new(Box::new(move |_, bytes: &mut Vec<u8>| {
            // The first write long enough to be a data packet.
            if !seen.load(Ordering::Relaxed) && bytes.len() > 900 {
                bytes[500] ^= 0x55;
                seen.store(true, Ordering::Relaxed);
            }
        })));
        let (sent, received, inbox) =
            exchange(Protocol::ZModem, &[("blob", payload.clone())], None, tamper, None);
        assert!(flipped.load(Ordering::Relaxed), "the tamper never ran");
        assert!(sent.is_ok(), "{sent:?}");
        assert!(received.is_ok(), "{received:?}");
        assert_eq!(fs::read(inbox.join("blob")).unwrap(), payload);
    }

    /// The same for XMODEM: a bad block is refused and sent again.
    #[test]
    fn xmodem_sends_a_bad_block_again() {
        let payload = every_byte(700);
        let tamper: Tamper = Arc::new(Mutex::new(Box::new(|write, bytes: &mut Vec<u8>| {
            if write == 1 && bytes.len() > 100 {
                bytes[50] ^= 0xFF;
            }
        })));
        let (sent, received, inbox) = exchange(
            Protocol::XModem,
            &[("blob", payload.clone())],
            Some("blob"),
            tamper,
            None,
        );
        assert!(sent.is_ok(), "{sent:?}");
        assert!(received.is_ok(), "{received:?}");
        assert_eq!(&fs::read(inbox.join("blob")).unwrap()[..700], &payload[..]);
    }

    #[test]
    fn a_cancel_stops_both_sides() {
        let payload = every_byte(200_000);
        let (sent, received, _) = exchange(
            Protocol::ZModem,
            &[("big", payload)],
            None,
            no_tamper(),
            Some(Arc::new(AtomicBool::new(false))),
        );
        assert!(matches!(sent, Err(Failure::Cancelled)), "{sent:?}");
        assert!(matches!(received, Err(Failure::Aborted)), "{received:?}");
    }

    #[test]
    fn a_receiver_with_nobody_there_gives_up() {
        // The other link stays open and says nothing, like a device
        // that never ran `sz`.
        let (_silent, _, receiver, _) = pair(Timing::fast(), no_tamper());
        let inbox = scratch("in");
        let outcome = run(
            TransferJob::Receive {
                protocol: Protocol::ZModem,
                folder: inbox,
                name: None,
            },
            receiver,
            quiet(),
        );
        assert!(matches!(outcome, Err(Failure::Timeout(_))), "{outcome:?}");
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let folder = scratch("names");
        fs::write(folder.join("dump.bin"), b"one").unwrap();
        assert_eq!(unique_path(&folder, "dump.bin"), folder.join("dump (2).bin"));
        fs::write(folder.join("dump (2).bin"), b"two").unwrap();
        assert_eq!(unique_path(&folder, "dump.bin"), folder.join("dump (3).bin"));
        assert_eq!(unique_path(&folder, "fresh"), folder.join("fresh"));
    }

    #[test]
    fn a_name_from_the_wire_is_only_a_name() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\boot\\image.bin"), "image.bin");
        assert_eq!(safe_name("  "), "received");
        assert_eq!(safe_name(".."), "received");
        assert_eq!(
            parse_file_info(b"firmware.bin\0123456 17654321 100644 0 1 123456\0"),
            Some(("firmware.bin".to_string(), Some(123_456)))
        );
        assert_eq!(parse_file_info(b"\0"), None);
        assert_eq!(parse_file_info(b"bare"), Some(("bare".to_string(), None)));
    }

    #[test]
    fn zmodem_announces_itself() {
        let mut sniffer = Sniffer::default();
        assert_eq!(sniffer.hear(b"$ sz firmware.bin\r\n"), None);
        assert_eq!(
            sniffer.hear(b"rz\r**\x18B00000000000000\r\x8a\x11"),
            Some(Direction::Receive)
        );
        assert_eq!(sniffer.hear(b"**\x18B0100000023be50\r\x8a"), Some(Direction::Send));
        // Split across two reads.
        assert_eq!(sniffer.hear(b"**\x18"), None);
        assert_eq!(sniffer.hear(b"B00000000000000\r\x8a\x11"), Some(Direction::Receive));
        assert_eq!(sniffer.hear(b"plain text with a * in it"), None);
    }
}
