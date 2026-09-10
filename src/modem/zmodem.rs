//! ZMODEM: headers and packets, streamed, with a way back.
//!
//! Everything on the line is either a header or a data packet. A header
//! is a type and four bytes — flags, or a file position — after a `*`,
//! ZDLE and a letter saying how it is written: `B` for hex, which any
//! line can carry, `A` for binary with a 16-bit check, `C` for binary
//! with a 32-bit one. A data packet follows a ZFILE or ZDATA header: the
//! bytes, ZDLE and a letter saying whether more follow and whether an
//! answer is wanted, then the check. Bytes that a line might eat — XON,
//! XOFF, ZDLE itself, and every control character when the receiver asks
//! for that — go out as ZDLE and the byte with bit 6 flipped.
//!
//! The sender opens with ZRQINIT and the receiver answers ZRINIT, saying
//! what it can take; each file is offered with ZFILE and its name, the
//! receiver answers ZRPOS with where to start — zero, or further on when
//! it has part of the file — and the data streams from there in ZCRCG
//! packets, no answer expected, until ZEOF. When a packet's check fails
//! the receiver sends ZRPOS again with where it got to, and the sender,
//! which looks between packets for anything the receiver said, seeks
//! there and goes on. ZFIN both ways ends the run, with `OO` after it.
//!
//! What is here is what `sz` and `rz` do with each other over a serial
//! line; the parts of the protocol for command execution, encryption and
//! compression were never used and are not.

use std::{
    fs::{self, File},
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use super::{
    Failure, Link, Reporter,
    crc::{CRC32_INIT, crc16, crc16_update, crc32, crc32_finish, crc32_update},
    file_name_of, modified, parse_file_info, unique_path,
};

const ZPAD: u8 = b'*';
pub(super) const ZDLE: u8 = 0x18;
const ZBIN: u8 = b'A';
const ZHEX: u8 = b'B';
const ZBIN32: u8 = b'C';
const XON: u8 = 0x11;
const XOFF: u8 = 0x13;

const ZRQINIT: u8 = 0;
const ZRINIT: u8 = 1;
const ZSINIT: u8 = 2;
const ZACK: u8 = 3;
const ZFILE: u8 = 4;
const ZSKIP: u8 = 5;
const ZNAK: u8 = 6;
const ZABORT: u8 = 7;
const ZFIN: u8 = 8;
const ZRPOS: u8 = 9;
const ZDATA: u8 = 10;
const ZEOF: u8 = 11;
const ZFERR: u8 = 12;
const ZCRC: u8 = 13;
const ZCHALLENGE: u8 = 14;
const ZCOMPL: u8 = 15;
const ZCAN: u8 = 16;
const ZFREECNT: u8 = 17;
const ZCOMMAND: u8 = 18;

/// How a data packet ends: the last of the file, one of a stream, one
/// wanting an answer with more to follow, one wanting an answer before
/// anything follows.
const ZCRCE: u8 = b'h';
const ZCRCG: u8 = b'i';
const ZCRCQ: u8 = b'j';
const ZCRCW: u8 = b'k';
/// DEL and 0xFF, escaped, when everything is.
const ZRUB0: u8 = b'l';
const ZRUB1: u8 = b'm';

/// What a receiver says it can do, in ZRINIT.
const CANFDX: u8 = 0x01;
const CANOVIO: u8 = 0x02;
const CANFC32: u8 = 0x20;
const ESCCTL: u8 = 0x40;
/// What a sender says of a file, in ZFILE: bytes, not text.
const ZCBIN: u8 = 1;

/// The data a packet carries, when the receiver has no smaller limit.
const PACKET: usize = 1024;
/// The most a packet may carry before it is not one.
const LONGEST_PACKET: usize = 8192;
/// How many CANs in a row mean the other side has stopped.
const CANS_TO_STOP: u8 = 5;

/// A header: its type and its four bytes, and whether the frame it came
/// in — and so the packet that follows it — uses the 32-bit check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Header {
    kind: u8,
    data: [u8; 4],
    wide: bool,
}

impl Header {
    /// A header carrying flags; ZF0 is the last byte.
    fn flags(kind: u8, f0: u8) -> Self {
        Self {
            kind,
            data: [0, 0, 0, f0],
            wide: false,
        }
    }

    /// A header carrying a file position, low byte first.
    fn at(kind: u8, position: u64) -> Self {
        Self {
            kind,
            data: (position as u32).to_le_bytes(),
            wide: false,
        }
    }

    fn position(&self) -> u64 {
        u64::from(u32::from_le_bytes(self.data))
    }

    fn f0(&self) -> u8 {
        self.data[3]
    }

    fn body(&self) -> [u8; 5] {
        [self.kind, self.data[0], self.data[1], self.data[2], self.data[3]]
    }
}

/// What the receiver said in ZRINIT: how it wants the packets.
#[derive(Clone, Copy, Debug)]
struct Terms {
    wide: bool,
    escape_control: bool,
    /// Whether packets can stream, or each must be answered.
    streaming: bool,
    packet: usize,
}

impl Terms {
    fn from(header: &Header) -> Self {
        let f0 = header.f0();
        let buffer = usize::from(u16::from_le_bytes([header.data[0], header.data[1]]));
        Self {
            wide: f0 & CANFC32 != 0,
            escape_control: f0 & ESCCTL != 0,
            streaming: f0 & CANFDX != 0 && f0 & CANOVIO != 0,
            packet: if buffer == 0 {
                PACKET
            } else {
                buffer.min(PACKET)
            },
        }
    }
}

/// Writes bytes the way the line can carry them.
struct Escaper {
    escape_control: bool,
    last: u8,
}

impl Escaper {
    fn new(escape_control: bool) -> Self {
        Self {
            escape_control,
            last: 0,
        }
    }

    fn push(&mut self, out: &mut Vec<u8>, byte: u8) {
        let escaped = match byte {
            ZDLE | 0x10 | 0x11 | 0x13 | 0x90 | 0x91 | 0x93 => true,
            // A carriage return after `@` is a telnet escape to some.
            0x0D | 0x8D => self.last == b'@',
            0x7F if self.escape_control => {
                out.extend_from_slice(&[ZDLE, ZRUB0]);
                self.last = byte;
                return;
            }
            0xFF if self.escape_control => {
                out.extend_from_slice(&[ZDLE, ZRUB1]);
                self.last = byte;
                return;
            }
            _ => self.escape_control && byte & 0x60 == 0,
        };
        self.last = byte;
        if escaped {
            out.extend_from_slice(&[ZDLE, byte ^ 0x40]);
        } else {
            out.push(byte);
        }
    }

    fn extend(&mut self, out: &mut Vec<u8>, bytes: &[u8]) {
        for &byte in bytes {
            self.push(out, byte);
        }
    }
}

/// A hex header: readable by anything, and what both sides open with.
fn hex_header(header: &Header) -> Vec<u8> {
    let body = header.body();
    let mut out = vec![ZPAD, ZPAD, ZDLE, ZHEX];
    for byte in body.iter().chain(crc16(&body).to_be_bytes().iter()) {
        out.extend_from_slice(format!("{byte:02x}").as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    if !matches!(header.kind, ZFIN | ZACK) {
        out.push(XON);
    }
    out
}

/// A binary header, closed with the check the receiver asked for.
fn binary_header(header: &Header, wide: bool, escaper: &mut Escaper) -> Vec<u8> {
    let body = header.body();
    let mut out = vec![ZPAD, ZDLE, if wide { ZBIN32 } else { ZBIN }];
    escaper.extend(&mut out, &body);
    if wide {
        escaper.extend(&mut out, &crc32(&body).to_le_bytes());
    } else {
        escaper.extend(&mut out, &crc16(&body).to_be_bytes());
    }
    out
}

/// A data packet: the bytes, how it ends, and the check over both.
fn packet(data: &[u8], end: u8, wide: bool, escaper: &mut Escaper) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2 + 8);
    escaper.extend(&mut out, data);
    out.extend_from_slice(&[ZDLE, end]);
    escaper.last = end;
    if wide {
        let crc = crc32_finish(crc32_update(crc32_update(CRC32_INIT, data), &[end]));
        escaper.extend(&mut out, &crc.to_le_bytes());
    } else {
        let crc = crc16_update(crc16(data), &[end]);
        escaper.extend(&mut out, &crc.to_be_bytes());
    }
    out
}

/// One step of the incoming line, decoded.
enum Zin {
    Byte(u8),
    /// A packet's end: which kind.
    End(u8),
    Timeout,
    Garbled,
}

/// A packet as read.
enum Packet {
    Data(Vec<u8>, u8),
    Timeout,
    Garbled,
}

/// The link, read the way ZMODEM reads it.
struct Line<'a> {
    link: &'a mut Link,
    cans: u8,
}

impl Line<'_> {
    /// The next byte as it came, counting CANs on the way.
    fn raw(&mut self, timeout: Duration) -> Result<Option<u8>, Failure> {
        let byte = self.link.read_byte(timeout)?;
        match byte {
            Some(ZDLE) => {
                self.cans += 1;
                if self.cans >= CANS_TO_STOP {
                    return Err(Failure::Aborted);
                }
            }
            Some(_) => self.cans = 0,
            None => {}
        }
        Ok(byte)
    }

    /// The next byte of a hex header, past any flow control.
    fn raw7(&mut self, timeout: Duration) -> Result<Option<u8>, Failure> {
        loop {
            match self.raw(timeout)? {
                Some(XON | XOFF | 0x91 | 0x93) => {}
                other => return Ok(other),
            }
        }
    }

    /// The next byte with its escaping undone, or a packet's end.
    fn zdle(&mut self, timeout: Duration) -> Result<Zin, Failure> {
        loop {
            let Some(byte) = self.raw(timeout)? else {
                return Ok(Zin::Timeout);
            };
            match byte {
                XON | XOFF | 0x91 | 0x93 => {}
                ZDLE => {
                    let Some(next) = self.raw(timeout)? else {
                        return Ok(Zin::Timeout);
                    };
                    return Ok(match next {
                        ZDLE => continue,
                        ZCRCE | ZCRCG | ZCRCQ | ZCRCW => Zin::End(next),
                        ZRUB0 => Zin::Byte(0x7F),
                        ZRUB1 => Zin::Byte(0xFF),
                        next if next & 0x60 == 0x40 => Zin::Byte(next ^ 0x40),
                        _ => Zin::Garbled,
                    });
                }
                _ => return Ok(Zin::Byte(byte)),
            }
        }
    }

    /// Fills the buffer with decoded bytes, or says a packet's end or
    /// the timeout came first.
    fn zdle_into(&mut self, buffer: &mut [u8], timeout: Duration) -> Result<bool, Failure> {
        for slot in buffer.iter_mut() {
            match self.zdle(timeout)? {
                Zin::Byte(byte) => *slot = byte,
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    fn hex_byte(&mut self, timeout: Duration) -> Result<Option<u8>, Failure> {
        let high = self.raw7(timeout)?.and_then(hex_digit);
        let low = self.raw7(timeout)?.and_then(hex_digit);
        Ok(match (high, low) {
            (Some(high), Some(low)) => Some(high << 4 | low),
            _ => None,
        })
    }

    /// The next header within the timeout: anything that is not one is
    /// passed over, a header whose check fails included.
    fn header(&mut self, timeout: Duration) -> Result<Option<Header>, Failure> {
        let deadline = Instant::now() + timeout;
        'hunt: loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            loop {
                match self.raw(remaining)? {
                    Some(ZPAD) => break,
                    Some(_) => {}
                    None => return Ok(None),
                }
            }
            loop {
                match self.raw(remaining)? {
                    Some(ZPAD) => {}
                    Some(ZDLE) => break,
                    Some(_) => continue 'hunt,
                    None => return Ok(None),
                }
            }
            let Some(kind) = self.raw(remaining)? else {
                return Ok(None);
            };
            let mut body = [0_u8; 5];
            let wide = kind == ZBIN32;
            let sound = match kind {
                ZHEX => {
                    for slot in body.iter_mut() {
                        match self.hex_byte(remaining)? {
                            Some(byte) => *slot = byte,
                            None => continue 'hunt,
                        }
                    }
                    let mut crc = [0_u8; 2];
                    for slot in crc.iter_mut() {
                        match self.hex_byte(remaining)? {
                            Some(byte) => *slot = byte,
                            None => continue 'hunt,
                        }
                    }
                    // The line end and the XON after a hex header are
                    // not part of anything: taken now, so what follows
                    // a header reads as what it is.
                    self.skim()?;
                    crc16(&body) == u16::from_be_bytes(crc)
                }
                ZBIN => {
                    let mut crc = [0_u8; 2];
                    self.zdle_into(&mut body, remaining)?
                        && self.zdle_into(&mut crc, remaining)?
                        && crc16(&body) == u16::from_be_bytes(crc)
                }
                ZBIN32 => {
                    let mut crc = [0_u8; 4];
                    self.zdle_into(&mut body, remaining)?
                        && self.zdle_into(&mut crc, remaining)?
                        && crc32(&body) == u32::from_le_bytes(crc)
                }
                _ => continue 'hunt,
            };
            if !sound {
                continue 'hunt;
            }
            return Ok(Some(Header {
                kind: body[0],
                data: [body[1], body[2], body[3], body[4]],
                wide,
            }));
        }
    }

    /// The packet after a header, checked the way the header was.
    fn packet(&mut self, wide: bool, timeout: Duration) -> Result<Packet, Failure> {
        let mut data = Vec::with_capacity(PACKET);
        loop {
            match self.zdle(timeout)? {
                Zin::Byte(byte) => {
                    data.push(byte);
                    if data.len() > LONGEST_PACKET {
                        return Ok(Packet::Garbled);
                    }
                }
                Zin::End(end) => {
                    let sound = if wide {
                        let mut crc = [0_u8; 4];
                        self.zdle_into(&mut crc, timeout)?
                            && crc32_finish(crc32_update(crc32_update(CRC32_INIT, &data), &[end]))
                                == u32::from_le_bytes(crc)
                    } else {
                        let mut crc = [0_u8; 2];
                        self.zdle_into(&mut crc, timeout)?
                            && crc16_update(crc16(&data), &[end]) == u16::from_be_bytes(crc)
                    };
                    return Ok(if sound {
                        Packet::Data(data, end)
                    } else {
                        Packet::Garbled
                    });
                }
                Zin::Timeout => return Ok(Packet::Timeout),
                Zin::Garbled => return Ok(Packet::Garbled),
            }
        }
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        self.link.write(bytes)
    }

    /// Drops what has arrived that means nothing — line ends and flow
    /// control — leaving whatever comes after them to be read.
    fn skim(&mut self) -> Result<(), Failure> {
        loop {
            match self.link.read_byte(Duration::ZERO)? {
                Some(b'\r' | b'\n' | 0x8A | XON | XOFF | 0x91 | 0x93) => {}
                Some(other) => {
                    self.link.unread(other);
                    return Ok(());
                }
                None => return Ok(()),
            }
        }
    }

    /// Whether the receiver has sent something worth reading.
    fn has_word(&mut self) -> Result<bool, Failure> {
        self.skim()?;
        Ok(self.link.has_pending())
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|digit| digit as u8)
}

/// Whether a header means the other side has stopped.
fn calls_it_off(kind: u8) -> bool {
    matches!(kind, ZCAN | ZABORT | ZFERR | ZFIN)
}

pub(super) fn send(
    link: &mut Link,
    paths: &[PathBuf],
    reporter: &mut Reporter,
) -> Result<Vec<(String, u64)>, Failure> {
    let timing = link.timing;
    let mut line = Line { link, cans: 0 };
    let terms = greet_receiver(&mut line)?;
    let mut escaper = Escaper::new(terms.escape_control);
    let mut sent = Vec::new();

    for (index, path) in paths.iter().enumerate() {
        let mut file = File::open(path)
            .map_err(|error| Failure::Io(format!("{}: {error}", path.display())))?;
        let size = file.metadata()?.len();
        let name = file_name_of(path);
        reporter.file(&name, Some(size));
        let files_left = paths.len() - index;
        let bytes_left: u64 = paths[index..]
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum();
        let info = format!(
            "{name}\0{size} {:o} 100644 0 {files_left} {bytes_left}\0",
            modified(&file)
        );
        let Some(mut position) = offer_file(&mut line, &terms, &mut escaper, &info, &mut file)?
        else {
            continue;
        };

        let mut chunk = vec![0_u8; terms.packet];
        let mut repositions = 0;
        let mut skipped = false;
        'file: loop {
            file.seek(SeekFrom::Start(position))?;
            line.send(&binary_header(
                &Header::at(ZDATA, position),
                terms.wide,
                &mut escaper,
            ))?;
            loop {
                let want = (size - position).min(terms.packet as u64) as usize;
                file.read_exact(&mut chunk[..want])?;
                let last = position + want as u64 == size;
                let end = if last {
                    ZCRCE
                } else if terms.streaming {
                    ZCRCG
                } else {
                    ZCRCW
                };
                line.send(&packet(&chunk[..want], end, terms.wide, &mut escaper))?;
                position += want as u64;
                reporter.progress(position, line.link.wire);

                // What the receiver has to say: an answer when one was
                // asked for, and between streamed packets whatever it
                // sent without being asked.
                let answer = if end == ZCRCW {
                    line.header(timing.reply)?
                } else if line.has_word()? {
                    line.header(timing.settle)?
                } else {
                    None
                };
                match answer {
                    Some(header) if header.kind == ZRPOS => {
                        position = reposition(&header, size, &mut repositions, timing.retries)?;
                        continue 'file;
                    }
                    Some(header) if header.kind == ZSKIP => {
                        skipped = true;
                        break 'file;
                    }
                    Some(header) if calls_it_off(header.kind) => return Err(Failure::Aborted),
                    Some(_) => {}
                    None if end == ZCRCW => {
                        // The answer never came: from the packet again.
                        position -= want as u64;
                        repositions += 1;
                        if repositions > timing.retries {
                            return Err(Failure::Timeout("the receiver's acknowledgement"));
                        }
                        continue 'file;
                    }
                    None => {}
                }
                if last {
                    break;
                }
            }

            let mut attempts = 0;
            loop {
                line.send(&binary_header(
                    &Header::at(ZEOF, position),
                    terms.wide,
                    &mut escaper,
                ))?;
                match line.header(timing.reply)? {
                    Some(header) if header.kind == ZRINIT => break 'file,
                    Some(header) if header.kind == ZRPOS => {
                        position = reposition(&header, size, &mut repositions, timing.retries)?;
                        continue 'file;
                    }
                    Some(header) if calls_it_off(header.kind) => return Err(Failure::Aborted),
                    _ => {
                        attempts += 1;
                        if attempts > timing.retries {
                            return Err(Failure::Timeout("the receiver's word that the file is whole"));
                        }
                    }
                }
            }
        }
        if !skipped {
            sent.push((name, size));
        }
    }

    let mut attempts = 0;
    loop {
        line.send(&hex_header(&Header::flags(ZFIN, 0)))?;
        match line.header(timing.reply)? {
            Some(header) if header.kind == ZFIN => {
                line.send(b"OO")?;
                break;
            }
            Some(header) if matches!(header.kind, ZCAN | ZABORT | ZFERR) => {
                return Err(Failure::Aborted);
            }
            _ => {
                // The files went; a receiver that will not say goodbye
                // is left to it.
                attempts += 1;
                if attempts > timing.retries {
                    break;
                }
            }
        }
    }
    Ok(sent)
}

/// Opens with ZRQINIT until the receiver answers with its terms.
fn greet_receiver(line: &mut Line) -> Result<Terms, Failure> {
    let timing = line.link.timing;
    line.send(b"rz\r")?;
    for _ in 0..timing.greetings {
        line.send(&hex_header(&Header::flags(ZRQINIT, 0)))?;
        let deadline = Instant::now() + timing.greet;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match line.header(remaining)? {
                Some(header) if header.kind == ZRINIT => return Ok(Terms::from(&header)),
                Some(header) if header.kind == ZCHALLENGE => {
                    line.send(&hex_header(&Header {
                        kind: ZACK,
                        data: header.data,
                        wide: false,
                    }))?;
                }
                Some(header) if calls_it_off(header.kind) => return Err(Failure::Aborted),
                Some(_) => {}
                None => break,
            }
        }
    }
    Err(Failure::Timeout("the receiver's ZRINIT"))
}

/// Offers a file until the receiver says where to start it from, or that
/// it does not want it.
fn offer_file(
    line: &mut Line,
    terms: &Terms,
    escaper: &mut Escaper,
    info: &str,
    file: &mut File,
) -> Result<Option<u64>, Failure> {
    let timing = line.link.timing;
    let mut attempts = 0;
    loop {
        line.send(&binary_header(&Header::flags(ZFILE, ZCBIN), terms.wide, escaper))?;
        line.send(&packet(info.as_bytes(), ZCRCW, terms.wide, escaper))?;
        let mut wait = timing.reply;
        loop {
            match line.header(wait)? {
                Some(header) if header.kind == ZRPOS => return Ok(Some(header.position())),
                Some(header) if header.kind == ZSKIP => return Ok(None),
                Some(header) if header.kind == ZCRC => {
                    // The receiver has a file of that name and wants to
                    // know whether this is the same one.
                    let crc = whole_file_crc(file)?;
                    line.send(&binary_header(
                        &Header {
                            kind: ZCRC,
                            data: crc.to_le_bytes(),
                            wide: false,
                        },
                        terms.wide,
                        escaper,
                    ))?;
                }
                Some(header) if calls_it_off(header.kind) => return Err(Failure::Aborted),
                // ZRINIT again, or ZNAK: a greeting the receiver
                // repeated as the offer went out, as often as not, with
                // its answer to the offer right behind. The offer goes
                // again only when nothing follows.
                Some(_) => wait = timing.settle,
                None => break,
            }
        }
        attempts += 1;
        if attempts > timing.retries {
            return Err(Failure::Timeout("the receiver's answer to the file"));
        }
    }
}

fn whole_file_crc(file: &mut File) -> Result<u32, Failure> {
    file.seek(SeekFrom::Start(0))?;
    let mut crc = CRC32_INIT;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        crc = crc32_update(crc, &buffer[..read]);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(crc32_finish(crc))
}

/// Where the receiver asked to go on from, counting how often it has
/// asked: a line that loses every packet is not going to carry the file.
fn reposition(
    header: &Header,
    size: u64,
    repositions: &mut u32,
    retries: u32,
) -> Result<u64, Failure> {
    *repositions += 1;
    if *repositions > retries * 2 {
        return Err(Failure::Protocol(
            "the receiver kept asking for the same data again".into(),
        ));
    }
    Ok(header.position().min(size))
}

pub(super) fn receive(
    link: &mut Link,
    folder: &Path,
    reporter: &mut Reporter,
) -> Result<Vec<(String, u64)>, Failure> {
    let timing = link.timing;
    let mut line = Line { link, cans: 0 };
    let ready = hex_header(&Header::flags(ZRINIT, CANFDX | CANOVIO | CANFC32));
    let mut received = Vec::new();
    let mut silences = 0;
    line.send(&ready)?;
    loop {
        let Some(header) = line.header(timing.greet)? else {
            silences += 1;
            if silences > timing.greetings {
                return Err(Failure::Timeout("the sender"));
            }
            line.send(&ready)?;
            continue;
        };
        silences = 0;
        match header.kind {
            ZRQINIT => line.send(&ready)?,
            ZSINIT => {
                let _ = line.packet(header.wide, timing.reply)?;
                line.send(&hex_header(&Header::flags(ZACK, 0)))?;
            }
            ZFILE => {
                let Packet::Data(info, _) = line.packet(header.wide, timing.reply)? else {
                    line.send(&hex_header(&Header::flags(ZNAK, 0)))?;
                    continue;
                };
                let Some((name, size)) = parse_file_info(&info) else {
                    line.send(&hex_header(&Header::flags(ZSKIP, 0)))?;
                    continue;
                };
                let path = unique_path(folder, &name);
                let mut out = BufWriter::new(
                    File::create(&path)
                        .map_err(|error| Failure::Io(format!("{}: {error}", path.display())))?,
                );
                reporter.file(&name, size);
                match receive_file(&mut line, &mut out, reporter)
                    .and_then(|done| out.flush().map(|()| done).map_err(Failure::from))
                {
                    Ok(done) => {
                        drop(out);
                        received.push((name, done));
                        line.send(&ready)?;
                    }
                    Err(failure) => {
                        drop(out);
                        let _ = fs::remove_file(&path);
                        return Err(failure);
                    }
                }
            }
            ZFIN => {
                line.send(&hex_header(&Header::flags(ZFIN, 0)))?;
                // The sender signs off with `OO`; taken, so it does not
                // land in the log.
                for _ in 0..2 {
                    if line.link.read_byte(timing.settle)? != Some(b'O') {
                        break;
                    }
                }
                return Ok(received);
            }
            ZCAN | ZABORT | ZFERR => return Err(Failure::Aborted),
            ZFREECNT => line.send(&hex_header(&Header {
                kind: ZACK,
                data: u32::MAX.to_le_bytes(),
                wide: false,
            }))?,
            ZCOMMAND => {
                let _ = line.packet(header.wide, timing.reply)?;
                line.send(&hex_header(&Header::flags(ZCOMPL, 0)))?;
            }
            _ => {}
        }
    }
}

/// Takes one file's data from the ZDATA packets, asking again from where
/// it got to whenever one goes wrong, and says how long the file was
/// once ZEOF agrees.
fn receive_file(line: &mut Line, out: &mut impl Write, reporter: &mut Reporter) -> Result<u64, Failure> {
    let timing = line.link.timing;
    let mut position: u64 = 0;
    let mut errors = 0;
    let from = |position| hex_header(&Header::at(ZRPOS, position));
    line.send(&from(0))?;
    loop {
        let Some(header) = line.header(timing.reply)? else {
            errors += 1;
            if errors > timing.retries {
                return Err(Failure::Timeout("the file's data"));
            }
            line.send(&from(position))?;
            continue;
        };
        match header.kind {
            ZDATA => {
                if header.position() != position {
                    line.link.settle()?;
                    line.send(&from(position))?;
                    continue;
                }
                loop {
                    match line.packet(header.wide, timing.reply)? {
                        Packet::Data(bytes, end) => {
                            out.write_all(&bytes)?;
                            position += bytes.len() as u64;
                            errors = 0;
                            reporter.progress(position, line.link.wire);
                            match end {
                                ZCRCW => {
                                    line.send(&hex_header(&Header::at(ZACK, position)))?;
                                    break;
                                }
                                ZCRCQ => line.send(&hex_header(&Header::at(ZACK, position)))?,
                                ZCRCE => break,
                                _ => {}
                            }
                        }
                        Packet::Timeout | Packet::Garbled => {
                            errors += 1;
                            if errors > timing.retries {
                                return Err(Failure::Protocol("too many bad packets".into()));
                            }
                            line.link.settle()?;
                            line.send(&from(position))?;
                            break;
                        }
                    }
                }
            }
            ZEOF => {
                if header.position() == position {
                    return Ok(position);
                }
                line.send(&from(position))?;
            }
            ZFILE => {
                // Offered again: the ZRPOS did not arrive.
                let _ = line.packet(header.wide, timing.reply)?;
                line.send(&from(position))?;
            }
            ZNAK => line.send(&from(position))?,
            kind if calls_it_off(kind) => return Err(Failure::Aborted),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Escaper, Header, ZACK, ZDLE, ZFIN, ZRINIT, ZRQINIT, hex_header};

    /// The headers `sz` and `rz` open with, byte for byte.
    #[test]
    fn the_opening_headers_match_lrzsz() {
        assert_eq!(
            hex_header(&Header::flags(ZRQINIT, 0)),
            b"**\x18B00000000000000\r\n\x11"
        );
        assert_eq!(
            hex_header(&Header::flags(ZRINIT, 0x23)),
            b"**\x18B0100000023be50\r\n\x11"
        );
        // ZFIN and ZACK go out without the XON.
        assert!(!hex_header(&Header::flags(ZFIN, 0)).ends_with(b"\x11"));
        assert!(!hex_header(&Header::at(ZACK, 1024)).ends_with(b"\x11"));
    }

    #[test]
    fn positions_go_low_byte_first() {
        let header = Header::at(ZACK, 0x0102_0304);
        assert_eq!(header.data, [0x04, 0x03, 0x02, 0x01]);
        assert_eq!(header.position(), 0x0102_0304);
    }

    #[test]
    fn the_bytes_a_line_might_eat_are_escaped() {
        let mut escaper = Escaper::new(false);
        let mut out = Vec::new();
        escaper.extend(&mut out, &[b'a', ZDLE, 0x11, 0x13, 0x90, b'@', 0x0D, b'b', 0x0D, 0x01]);
        assert_eq!(
            out,
            [
                b'a', ZDLE, 0x58, ZDLE, 0x51, ZDLE, 0x53, ZDLE, 0xD0, b'@', ZDLE, 0x4D, b'b',
                0x0D, 0x01
            ]
        );
        let mut all = Escaper::new(true);
        let mut out = Vec::new();
        all.extend(&mut out, &[0x01, 0x7F, 0xFF, b'x']);
        assert_eq!(out, [ZDLE, 0x41, ZDLE, b'l', ZDLE, b'm', b'x']);
    }
}
