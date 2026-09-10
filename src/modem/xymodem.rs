//! XMODEM and YMODEM: a file in numbered blocks, each answered.
//!
//! The receiver starts it, by asking: `C` for blocks closed with a CRC,
//! NAK for the older plain sum. Every block is a start byte — SOH for 128
//! bytes of data, STX for 1024 — the block number and its complement, the
//! data, and the check; the receiver answers ACK to move on or NAK to
//! have it again, and the sender ends with EOT, answered with ACK too.
//! Either side can call it off with two CANs.
//!
//! YMODEM puts a block 0 in front of each file, with the name and size
//! after it, and one more block 0 with nothing in it after the last file;
//! the receiver asks for each with `C`, the way it asks for the data. The
//! size means the padding can be cut from the last block, which XMODEM
//! cannot do: an XMODEM file arrives a whole number of blocks long, the
//! rest filled with `0x1A`.

use std::{
    fs::{self, File},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use super::{
    Failure, Link, Protocol, Reporter,
    crc::{checksum, crc16},
    file_name_of, modified, parse_file_info, safe_name, unique_path,
};

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const CRC_REQUEST: u8 = b'C';
/// What fills the last block past the end of the data.
const PAD: u8 = 0x1A;
const SHORT_BLOCK: usize = 128;
const LONG_BLOCK: usize = 1024;
/// How many CANs in a row mean the other side has stopped.
const CANS_TO_STOP: u8 = 2;

/// How a block is closed, as the receiver asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    Sum,
    Crc,
}

impl Check {
    fn request(self) -> u8 {
        match self {
            Self::Sum => NAK,
            Self::Crc => CRC_REQUEST,
        }
    }

    fn len(self) -> usize {
        match self {
            Self::Sum => 1,
            Self::Crc => 2,
        }
    }

    fn of(self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Sum => vec![checksum(data)],
            Self::Crc => crc16(data).to_be_bytes().to_vec(),
        }
    }

    fn matches(self, data: &[u8], tail: &[u8]) -> bool {
        self.of(data) == tail
    }
}

/// What the receiver answers a block with.
enum Reply {
    Ack,
    Nak,
    Timeout,
}

pub(super) fn send(
    link: &mut Link,
    protocol: Protocol,
    paths: &[PathBuf],
    reporter: &mut Reporter,
) -> Result<Vec<(String, u64)>, Failure> {
    let batch = protocol.batches();
    let long = protocol != Protocol::XModem;
    let paths = if batch {
        paths
    } else {
        &paths[..paths.len().min(1)]
    };
    let mut sent = Vec::new();
    for path in paths {
        let mut file = File::open(path)
            .map_err(|error| Failure::Io(format!("{}: {error}", path.display())))?;
        let size = file.metadata()?.len();
        let name = file_name_of(path);
        reporter.file(&name, Some(size));
        let mut check = await_request(link)?;
        if batch {
            send_block(link, 0, &name_block(&name, size, modified(&file)), check)?;
            check = await_request(link)?;
        }
        send_data(link, &mut file, size, long, check, reporter)?;
        send_end(link)?;
        sent.push((name, size));
    }
    if batch {
        let check = await_request(link)?;
        send_block(link, 0, &[0; SHORT_BLOCK], check)?;
    }
    Ok(sent)
}

/// YMODEM's block 0: the name, then the size, the time and the mode, the
/// way `sb` writes them; NUL-padded to a block.
fn name_block(name: &str, size: u64, modified: u64) -> Vec<u8> {
    let mut block = Vec::with_capacity(SHORT_BLOCK);
    block.extend_from_slice(name.as_bytes());
    block.push(0);
    block.extend_from_slice(format!("{size} {modified:o} 100644").as_bytes());
    block.push(0);
    let len = if block.len() <= SHORT_BLOCK {
        SHORT_BLOCK
    } else {
        LONG_BLOCK
    };
    block.resize(len, 0);
    block
}

/// Waits for the receiver to ask for the first block, and says how it
/// wants the blocks closed.
fn await_request(link: &mut Link) -> Result<Check, Failure> {
    let timing = link.timing;
    let deadline = Instant::now() + timing.greet * timing.greetings;
    let mut cans = 0;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match link.read_byte(remaining.min(timing.greet))? {
            Some(CRC_REQUEST) => return Ok(Check::Crc),
            Some(NAK) => return Ok(Check::Sum),
            Some(CAN) => {
                cans += 1;
                if cans >= CANS_TO_STOP {
                    return Err(Failure::Aborted);
                }
            }
            Some(_) | None => {}
        }
    }
    Err(Failure::Timeout("the receiver's request"))
}

fn send_data(
    link: &mut Link,
    file: &mut File,
    size: u64,
    long: bool,
    check: Check,
    reporter: &mut Reporter,
) -> Result<(), Failure> {
    let mut sequence: u8 = 1;
    let mut done: u64 = 0;
    let mut block = vec![0_u8; LONG_BLOCK];
    while done < size {
        let remaining = size - done;
        let len = if long && remaining > SHORT_BLOCK as u64 {
            LONG_BLOCK
        } else {
            SHORT_BLOCK
        };
        let want = remaining.min(len as u64) as usize;
        file.read_exact(&mut block[..want])?;
        block[want..len].fill(PAD);
        send_block(link, sequence, &block[..len], check)?;
        done += want as u64;
        sequence = sequence.wrapping_add(1);
        reporter.progress(done, link.wire);
    }
    Ok(())
}

/// Sends one block until it is acknowledged.
fn send_block(link: &mut Link, sequence: u8, data: &[u8], check: Check) -> Result<(), Failure> {
    let mut frame = Vec::with_capacity(data.len() + 5);
    frame.push(if data.len() == LONG_BLOCK { STX } else { SOH });
    frame.push(sequence);
    frame.push(!sequence);
    frame.extend_from_slice(data);
    frame.extend(check.of(data));
    for _ in 0..=link.timing.retries {
        link.write(&frame)?;
        if let Reply::Ack = await_reply(link)? {
            return Ok(());
        }
    }
    Err(Failure::Timeout("an acknowledgement"))
}

/// The receiver's answer to a block, within the reply time. A request for
/// the first block counts as a NAK: the receiver did not see the block.
fn await_reply(link: &mut Link) -> Result<Reply, Failure> {
    let deadline = Instant::now() + link.timing.reply;
    let mut cans = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(Reply::Timeout);
        }
        match link.read_byte(remaining)? {
            Some(ACK) => return Ok(Reply::Ack),
            Some(NAK | CRC_REQUEST) => return Ok(Reply::Nak),
            Some(CAN) => {
                cans += 1;
                if cans >= CANS_TO_STOP {
                    return Err(Failure::Aborted);
                }
            }
            Some(_) => {}
            None => return Ok(Reply::Timeout),
        }
    }
}

fn send_end(link: &mut Link) -> Result<(), Failure> {
    for _ in 0..=link.timing.retries {
        link.write(&[EOT])?;
        if let Reply::Ack = await_reply(link)? {
            return Ok(());
        }
    }
    Err(Failure::Timeout("the acknowledgement of the end"))
}

/// A block as read, or the end, or something that was not a block.
enum Frame {
    Block { sequence: u8, data: Vec<u8> },
    End,
    Bad,
}

pub(super) fn receive(
    link: &mut Link,
    protocol: Protocol,
    folder: &Path,
    name: Option<&str>,
    reporter: &mut Reporter,
) -> Result<Vec<(String, u64)>, Failure> {
    let batch = protocol.batches();
    let mut received = Vec::new();
    loop {
        let (mut first, mut check) = request(link, !batch)?;
        let (name, limit) = if batch {
            let block = loop {
                match read_frame(link, first, check)? {
                    Frame::Block { sequence: 0, data } => break data,
                    Frame::Block { sequence, .. } => {
                        return Err(Failure::Protocol(format!(
                            "block {sequence} arrived where the name block was expected"
                        )));
                    }
                    Frame::End => {
                        link.write(&[ACK])?;
                        return Ok(received);
                    }
                    Frame::Bad => {
                        link.settle()?;
                        link.write(&[NAK])?;
                        first = await_start(link)?.ok_or(Failure::Timeout("the name block"))?;
                    }
                }
            };
            link.write(&[ACK])?;
            let Some((name, size)) = parse_file_info(&block) else {
                // The empty name block: the batch is over.
                return Ok(received);
            };
            (name, size)
        } else {
            (
                name.map(safe_name)
                    .unwrap_or_else(|| "received.bin".to_string()),
                None,
            )
        };

        let path = unique_path(folder, &name);
        let mut out = BufWriter::new(
            File::create(&path)
                .map_err(|error| Failure::Io(format!("{}: {error}", path.display())))?,
        );
        reporter.file(&name, limit);
        if batch {
            (first, check) = request(link, false)?;
        }
        let done = match receive_data(link, first, check, &mut out, limit, reporter)
            .and_then(|done| out.flush().map(|()| done).map_err(Failure::from))
        {
            Ok(done) => done,
            Err(failure) => {
                drop(out);
                let _ = fs::remove_file(&path);
                return Err(failure);
            }
        };
        received.push((name, done));
        if !batch {
            return Ok(received);
        }
    }
}

/// Asks for the first block, again and again until one begins, and says
/// how it was asked for. The CRC is asked for first; when that goes
/// unanswered for half the greetings, the plain sum is tried, in case the
/// sender is old enough not to know the CRC.
fn request(link: &mut Link, allow_sum: bool) -> Result<(u8, Check), Failure> {
    let timing = link.timing;
    let mut check = Check::Crc;
    let mut cans = 0;
    for attempt in 0..timing.greetings {
        if allow_sum && attempt >= timing.greetings / 2 {
            check = Check::Sum;
        }
        link.write(&[check.request()])?;
        let deadline = Instant::now() + timing.greet;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match link.read_byte(remaining)? {
                Some(start @ (SOH | STX | EOT)) => return Ok((start, check)),
                Some(CAN) => {
                    cans += 1;
                    if cans >= CANS_TO_STOP {
                        return Err(Failure::Aborted);
                    }
                }
                Some(_) => {}
                None => break,
            }
        }
    }
    Err(Failure::Timeout("the sender"))
}

/// The start of the next block within the reply time, or none.
fn await_start(link: &mut Link) -> Result<Option<u8>, Failure> {
    let deadline = Instant::now() + link.timing.reply;
    let mut cans = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        match link.read_byte(remaining)? {
            Some(start @ (SOH | STX | EOT)) => return Ok(Some(start)),
            Some(CAN) => {
                cans += 1;
                if cans >= CANS_TO_STOP {
                    return Err(Failure::Aborted);
                }
            }
            Some(_) => {}
            None => return Ok(None),
        }
    }
}

/// Reads the rest of a block whose first byte was `start`.
fn read_frame(link: &mut Link, start: u8, check: Check) -> Result<Frame, Failure> {
    let len = match start {
        SOH => SHORT_BLOCK,
        STX => LONG_BLOCK,
        EOT => return Ok(Frame::End),
        _ => return Ok(Frame::Bad),
    };
    let reply = link.timing.reply;
    let mut head = [0_u8; 2];
    if !link.read_exact(&mut head, reply)? {
        return Ok(Frame::Bad);
    }
    let mut data = vec![0_u8; len];
    if !link.read_exact(&mut data, reply)? {
        return Ok(Frame::Bad);
    }
    let mut tail = vec![0_u8; check.len()];
    if !link.read_exact(&mut tail, reply)? {
        return Ok(Frame::Bad);
    }
    if head[0] != !head[1] || !check.matches(&data, &tail) {
        return Ok(Frame::Bad);
    }
    Ok(Frame::Block {
        sequence: head[0],
        data,
    })
}

/// Takes the data blocks of one file, and says how many bytes were kept:
/// up to `limit` when the size is known, every block whole when not.
fn receive_data(
    link: &mut Link,
    first: u8,
    check: Check,
    out: &mut impl Write,
    limit: Option<u64>,
    reporter: &mut Reporter,
) -> Result<u64, Failure> {
    let timing = link.timing;
    let mut expected: u8 = 1;
    let mut done: u64 = 0;
    let mut errors = 0;
    let mut first = Some(first);
    loop {
        let start = match first.take() {
            Some(start) => start,
            None => match await_start(link)? {
                Some(start) => start,
                None => {
                    errors += 1;
                    if errors > timing.retries {
                        return Err(Failure::Timeout("the next block"));
                    }
                    link.write(&[NAK])?;
                    continue;
                }
            },
        };
        match read_frame(link, start, check)? {
            Frame::End => {
                link.write(&[ACK])?;
                return Ok(done);
            }
            Frame::Bad => {
                errors += 1;
                if errors > timing.retries {
                    return Err(Failure::Protocol("too many bad blocks".into()));
                }
                link.settle()?;
                link.write(&[NAK])?;
            }
            Frame::Block { sequence, data } => {
                if sequence == expected {
                    let keep = limit.map_or(data.len() as u64, |limit| {
                        (limit - done).min(data.len() as u64)
                    }) as usize;
                    out.write_all(&data[..keep])?;
                    done += keep as u64;
                    expected = expected.wrapping_add(1);
                    errors = 0;
                    link.write(&[ACK])?;
                    reporter.progress(done, link.wire);
                } else if sequence == expected.wrapping_sub(1) {
                    // Sent again because the ACK was lost: agree again.
                    link.write(&[ACK])?;
                } else {
                    return Err(Failure::Protocol(format!(
                        "block {sequence} arrived where {expected} was expected"
                    )));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LONG_BLOCK, SHORT_BLOCK, name_block};

    #[test]
    fn the_name_block_says_the_name_and_the_size() {
        let block = name_block("boot.bin", 4096, 0o17654321);
        assert_eq!(block.len(), SHORT_BLOCK);
        assert!(block.starts_with(b"boot.bin\x004096 17654321 100644\x00"));
        let long = name_block(&"n".repeat(200), 1, 0);
        assert_eq!(long.len(), LONG_BLOCK);
    }
}
