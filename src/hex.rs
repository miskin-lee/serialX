//! The hex view: what the port reads set out the way a hex editor sets a
//! file, rather than the way a terminal sets text.
//!
//! A session picks its view when it is made. In the text view the bytes go
//! through the emulation and come out as the device drew them; here they
//! are not read as text at all. Sixteen bytes make a row: the sixteen in
//! hex, and — down the right-hand side, where every hex editor puts it —
//! the same sixteen as characters, with a full stop standing for each one
//! that is not text. Where a row stands is the gutter's business, as it is
//! for text: the log numbers its lines and stamps them either way.
//!
//! The rows are laid out here and handed to the terminal as ordinary
//! printable text, so everything the log can do it can still do over a
//! dump: the timestamps and line numbers in the gutter, the scrollback,
//! the filter, the find, the selection and the copy all work on the rows
//! as they read.
//!
//! Bytes arrive in reads, not in sixteens. A read fills whole rows and its
//! tail waits for the read that follows — or, when nothing follows within
//! the hold the workbench allows a line, goes out as a short row of its
//! own. So a device streaming comes out in an even grid, and a device that
//! says three bytes and stops shows them at once.
//!
//! Colour says what a byte is, since hex digits cannot: the zeroes, the
//! bytes that end a line, the rest of the control bytes and everything
//! above ASCII each have an ink, in both panes at once, so the shape of a
//! frame can be read before a single pair of digits is. The inks are the
//! terminal's own ANSI colours, so a dump reads in either theme; naming
//! them also keeps the semantic highlighter — which would otherwise find
//! addresses and times all over a dump — off the rows.

use serde::{Deserialize, Serialize};

/// How a session's log reads what the port sends: as the text the device
/// drew, or as the bytes it sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogView {
    #[default]
    Text,
    Hex,
}

impl LogView {
    pub(crate) fn is_hex(self) -> bool {
        matches!(self, Self::Hex)
    }
}

/// How many bytes a row holds, and where its two halves part: sixteen, in
/// two eights, as `hexdump -C` sets them so a byte can be counted to.
const ROW_BYTES: usize = 16;
const HALF: usize = ROW_BYTES / 2;

/// The inks, as the ANSI colours the terminal palette answers for. Bright
/// black for what frames the row, white for text — the palette's seventh
/// colour is its foreground — and one colour each for the three kinds of
/// byte that are not text.
const FRAME: &str = "\x1b[90m";
const TEXT: &str = "\x1b[37m";
const BREAK: &str = "\x1b[32m";
const CONTROL: &str = "\x1b[35m";
const HIGH: &str = "\x1b[33m";

/// What a byte is, as far as the eye is concerned.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ByteKind {
    /// Nothing: the padding a frame is full of, and the end of a string.
    Zero,
    /// A printable character, space included.
    Text,
    /// What ends a line, and the tab: the bytes a text log is made of.
    Break,
    /// The rest of the control bytes.
    Control,
    /// Above ASCII: half a character of UTF-8, or not text at all.
    High,
}

impl ByteKind {
    fn of(byte: u8) -> Self {
        match byte {
            0 => Self::Zero,
            b'\t' | b'\n' | b'\r' => Self::Break,
            0x20..=0x7e => Self::Text,
            0x80.. => Self::High,
            _ => Self::Control,
        }
    }

    fn ink(self) -> &'static str {
        match self {
            Self::Zero => FRAME,
            Self::Text => TEXT,
            Self::Break => BREAK,
            Self::Control => CONTROL,
            Self::High => HIGH,
        }
    }

    /// The byte in the character pane: itself when it is text, a full stop
    /// when there is nothing to show.
    fn glyph(self, byte: u8) -> u8 {
        if self == Self::Text { byte } else { b'.' }
    }
}

/// A dump as the bytes come in: the bytes of the row that is still
/// filling, and nothing else — a row goes out the moment it is whole.
#[derive(Default)]
pub(crate) struct HexDump {
    row: Vec<u8>,
}

impl HexDump {
    /// Takes what the port read and answers with the rows it fills, as the
    /// terminal is to print them. What is left over waits for the next
    /// read.
    pub(crate) fn take(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for &byte in bytes {
            self.row.push(byte);
            if self.row.len() == ROW_BYTES {
                self.end_row(&mut out);
            }
        }
        out
    }

    /// Lets out the row that is filling, short as it is: nothing followed
    /// it within the hold, or a line of the workbench's own is about to be
    /// printed under it.
    pub(crate) fn flush(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.row.is_empty() {
            self.end_row(&mut out);
        }
        out
    }

    /// Whether a row is waiting for the bytes that would fill it.
    pub(crate) fn holds(&self) -> bool {
        !self.row.is_empty()
    }

    /// The log was cleared: the row that was filling goes with it, having
    /// never been shown.
    pub(crate) fn reset(&mut self) {
        self.row.clear();
    }

    fn end_row(&mut self, out: &mut Vec<u8>) {
        render_row(out, &self.row);
        self.row.clear();
    }
}

/// One row, printed: the bytes in hex in their two halves, and the bytes
/// again as characters between two bars. A row short of sixteen still
/// takes the whole width, so the bars stand in their column however the
/// reads fell.
fn render_row(out: &mut Vec<u8>, row: &[u8]) {
    let mut ink = Ink::default();
    for index in 0..ROW_BYTES {
        if index == HALF {
            out.push(b' ');
        }
        match row.get(index) {
            Some(&byte) => {
                ink.set(out, ByteKind::of(byte).ink());
                out.extend_from_slice(format!("{byte:02X} ").as_bytes());
            }
            None => out.extend_from_slice(b"   "),
        }
    }
    ink.set(out, FRAME);
    out.extend_from_slice(b" |");
    for index in 0..ROW_BYTES {
        match row.get(index) {
            Some(&byte) => {
                let kind = ByteKind::of(byte);
                ink.set(out, kind.ink());
                out.push(kind.glyph(byte));
            }
            None => out.push(b' '),
        }
    }
    ink.set(out, FRAME);
    out.extend_from_slice(b"|\x1b[0m\r\n");
}

/// The ink the row is being written in, so a run of bytes of one kind
/// costs one escape rather than one apiece.
#[derive(Default)]
struct Ink(&'static str);

impl Ink {
    fn set(&mut self, out: &mut Vec<u8>, ink: &'static str) {
        if self.0 != ink {
            out.extend_from_slice(ink.as_bytes());
            self.0 = ink;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HexDump, LogView};

    /// The rows as they read, without the colour: what a reader sees.
    fn plain(bytes: &[u8]) -> Vec<String> {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let mut out = Vec::new();
        for line in text.split("\r\n").filter(|line| !line.is_empty()) {
            let mut row = String::new();
            let mut chars = line.chars();
            while let Some(character) = chars.next() {
                if character != '\x1b' {
                    row.push(character);
                    continue;
                }
                // An escape runs to the `m` that ends it.
                for escaped in chars.by_ref() {
                    if escaped == 'm' {
                        break;
                    }
                }
            }
            out.push(row);
        }
        out
    }

    #[test]
    fn a_full_row_reads_as_a_hex_editor_sets_it() {
        let mut dump = HexDump::default();
        let rows = plain(&dump.take(b"temp=25.0'C\r\nOK\r\n"));
        assert_eq!(
            rows,
            vec![
                "74 65 6D 70 3D 32 35 2E  30 27 43 0D 0A 4F 4B 0D  |temp=25.0'C..OK.|"
                    .to_string(),
            ]
        );
        assert!(dump.holds());
        // Every row is the same width, so the panes line up down the log
        // however the reads fell.
        let tail = plain(&dump.flush());
        assert_eq!(tail[0].len(), rows[0].len());
        assert!(tail[0].starts_with("0A    "));
        assert!(tail[0].ends_with("|.               |"));
    }

    #[test]
    fn a_read_short_of_a_row_waits_for_the_next() {
        let mut dump = HexDump::default();
        assert!(dump.take(b"AT").is_empty());
        assert!(dump.holds());
        let rows = plain(&dump.take(b"\r\nOK\r\nAT\r\nOK\r\n123"));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].starts_with("41 54 0D 0A 4F 4B 0D 0A  41 54 0D 0A 4F 4B 0D 0A"));
        assert!(dump.holds());
    }

    #[test]
    fn a_clear_takes_the_row_that_was_filling() {
        let mut dump = HexDump::default();
        dump.take(b"abc");
        assert!(dump.holds());
        dump.reset();
        assert!(!dump.holds());
        assert!(dump.flush().is_empty());
        // What comes after the clear starts a row of its own.
        dump.take(b"de");
        assert!(plain(&dump.flush())[0].starts_with("64 65 "));
    }

    #[test]
    fn a_log_is_text_unless_it_says_otherwise() {
        assert_eq!(LogView::default(), LogView::Text);
        assert!(!LogView::default().is_hex());
        assert_eq!(
            serde_json::from_str::<LogView>("\"hex\"").unwrap(),
            LogView::Hex
        );
        assert_eq!(serde_json::to_string(&LogView::Hex).unwrap(), "\"hex\"");
    }
}
