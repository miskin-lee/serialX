//! The terminal behind a tab: an alacritty grid the port writes into, with
//! the time every line began kept alongside.
//!
//! The emulation is `alacritty_terminal`'s, the core Alacritty and Zed's
//! terminal run on. It owns the screen and its scrollback, and it is what
//! turns the bytes a device sends — colours, cursor motion, a progress bar
//! redrawing itself, a wide character split across two reads — into a grid
//! of cells. There is no pseudo-terminal behind it, only the serial port:
//! what the port reads goes in through [`Terminal::receive`], and what the
//! terminal answers on its own (a device asking what it is talking to)
//! comes back out of the same call, to be written to the port.
//!
//! What the port reads goes in as it comes, with one exception: a read
//! that leaves the cursor at the head of a line — the newline a shell
//! answers Enter with, its prompt a read behind — is kept back for the
//! next read, or for a moment ([`Terminal::flush`]), so the two go in
//! together and the cursor goes straight to the prompt.
//!
//! Timestamps are not something a terminal has, so they are kept here
//! beside it. Every line the device ends gets the time its first byte
//! arrived; when the grid is drawn, those times are laid against the rows
//! that begin a line, counting back from the cursor. A line the terminal
//! wrapped shows its time once, on the row where it starts. Output that
//! moves the cursor about instead of printing lines — a full-screen
//! program — makes the times approximate, which is the most any line-based
//! stamp can be for it.
//!
//! Colour is laid on at the same moment. Text the device left in the
//! default colour is read for what it says — a level, a time, an address —
//! and drawn in that role's colour (see [`crate::highlight`]) once its line
//! is finished; the line the cursor is still on is left plain, so nothing
//! shifts under a hand that is typing. Text the device coloured itself is
//! drawn as it asked. The grid is never touched, so the filter, the copy
//! and the scrollback all see the bytes as sent.
//!
//! A clear from the device clears. `clear` on the other end sends an
//! erase of the whole screen — `ESC [ 2 J`, or `ESC [ J` from the home
//! position on a vt100 — and here that wipes the log: the screen, the
//! scrollback and the stamps go, the same as the workbench's own Clear, so
//! a `clear` typed at the device's prompt leaves the terminal as empty as
//! it leaves any other. `ESC [ 3 J`, the request to discard the scrollback
//! alone, does that alone. The alternate screen is left to itself: a
//! full-screen program clearing its own screen is not clearing the log,
//! which is there again when the program ends.
//!
//! Lines are numbered, the way an editor numbers them, from the first line
//! since the log was last cleared. The number rides with the stamp: both
//! are kept per line begun, so a line the terminal wrapped shows its number
//! once, and a line the scrollback has let go still counts — the first
//! number on screen climbs past one, rather than every line renumbering
//! as the oldest falls off. A clear, from either end, starts again at one.
//!
//! The find bar asks the grid where a pattern occurs ([`Terminal::find`]).
//! It reads logical lines — a wrapped line joined back together, so a word
//! split by the wrap is still found — and answers in cells, so a match can
//! be painted over the rows it sits on and scrolled into view.
//!
//! The selection is alacritty's as well ([`Terminal::begin_selection`] and
//! its kin). It is anchored to cells of the grid, so it rides up with the
//! log as lines arrive and lets go when the columns change under it; it
//! reads back the way a terminal copies — a line the terminal wrapped
//! comes out whole, without the break the wrap put in it — and the screen
//! is drawn with the cells it covers marked, row by row.

use std::{cell::RefCell, collections::VecDeque, ops::Range, rc::Rc};

use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionRange, SelectionType},
    term::{
        Config, Term, TermMode,
        cell::{Flags, LineLength},
        color::Colors,
    },
    vte::ansi::{Color, CursorShape, CursorStyle, NamedColor, Processor, Rgb},
};
use gpui_kit::Keystroke;

use crate::{
    filter::OutputFilter, highlight::Highlighter, presets::DEFAULT_SCROLLBACK_LINES,
    theme::TerminalPalette,
};

/// Stamps kept past the scrollback, for the lines on screen: the screen is
/// rarely this tall, and a stamp is small.
const STAMP_SLACK: usize = INITIAL_LINES * 8;
/// How wide a stamp reads — `14:32:40.018` — and what stands between it and
/// the line, for the copy that takes the stamps along.
const STAMP_WIDTH: usize = 12;
const STAMP_GAP: &str = "  ";
/// The fewest digits the line-number gutter is sized for, so it does not
/// widen at every power of ten while a log is short.
const MIN_NUMBER_DIGITS: usize = 4;
/// The size a terminal starts at, before its first layout says otherwise.
const INITIAL_COLUMNS: usize = 80;
const INITIAL_LINES: usize = 24;

#[derive(Clone, Copy, PartialEq, Eq)]
struct GridSize {
    columns: usize,
    lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// How much a press picks up: the cell under it and whatever is dragged
/// over, the word there, or the whole line — one, two and three clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionKind {
    Cells,
    Words,
    Lines,
}

impl SelectionKind {
    /// What a press of `clicks` picks up.
    pub(crate) fn for_clicks(clicks: usize) -> Self {
        match clicks {
            0 | 1 => Self::Cells,
            2 => Self::Words,
            _ => Self::Lines,
        }
    }
}

/// A cell of the grid as the pointer names it: the row — zero the top of
/// the screen, negative into the scrollback — the column, and which half
/// of the cell the pointer is on, since a selection begun in the right
/// half begins after the character there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GridCell {
    pub(crate) line: i32,
    pub(crate) column: usize,
    pub(crate) right_half: bool,
}

/// What the terminal itself wants written to the port: the answer to a
/// query, such as a device asking for the cursor's position. It collects
/// here and is handed over after each write.
#[derive(Clone, Default)]
struct Outbox(Rc<RefCell<Vec<u8>>>);

impl EventListener for Outbox {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            self.0.borrow_mut().extend_from_slice(text.as_bytes());
        }
    }
}

pub(crate) struct Terminal {
    term: Term<Outbox>,
    parser: Processor,
    outbox: Outbox,
    size: GridSize,
    /// When each line began, oldest first: one entry for every line the
    /// stream has ended, and one for the line in progress once it holds a
    /// byte.
    stamps: VecDeque<String>,
    /// Whether the line the cursor is on has yet to receive its first byte.
    unstamped: bool,
    /// The start of an escape sequence a read ended in the middle of,
    /// kept until the rest arrives and says whether it is an erase.
    held: Vec<u8>,
    /// A read that left the cursor at the head of a line — its last byte
    /// a `\r` or a `\n` — kept back with the time it arrived, for the next
    /// read to go in with, or for [`Terminal::flush`] if none comes. A
    /// shell answers Enter with the newline first and its prompt a moment
    /// later, in a read of its own; shown as they come, the cursor lands
    /// at the head of the line and then jumps to the prompt, at every
    /// keystroke.
    pending: Option<(Vec<u8>, String)>,
    /// Lines kept above the screen to scroll back through.
    scrollback: usize,
    /// The number of the oldest line still stamped, counting from one at
    /// the last clear: what the scrollback let go is added here, so the
    /// lines that remain keep the numbers they had.
    first_number: usize,
    /// Goes up whenever the grid changes — bytes in, a resize, a clear —
    /// so a search over it can tell whether its answer is still current.
    revision: u64,
    /// Which log this is: goes up at every clear, since the numbering
    /// starts over then and a number that named one line names another.
    epoch: u64,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new(DEFAULT_SCROLLBACK_LINES)
    }
}

impl Terminal {
    /// A terminal that keeps `scrollback` lines above its screen.
    pub(crate) fn new(scrollback: usize) -> Self {
        Self::with_size(
            GridSize {
                columns: INITIAL_COLUMNS,
                lines: INITIAL_LINES,
            },
            scrollback,
        )
    }

    fn with_size(size: GridSize, scrollback: usize) -> Self {
        let outbox = Outbox::default();
        Self {
            term: Term::new(Self::config(scrollback), &size, outbox.clone()),
            parser: Processor::new(),
            outbox,
            size,
            stamps: VecDeque::new(),
            unstamped: true,
            held: Vec::new(),
            pending: None,
            scrollback,
            first_number: 1,
            revision: 0,
            epoch: 0,
        }
    }

    fn config(scrollback: usize) -> Config {
        Config {
            scrolling_history: scrollback,
            // A blinking block unless the program on the other end asks
            // for something else.
            default_cursor_style: CursorStyle {
                shape: CursorShape::Block,
                blinking: true,
            },
            ..Config::default()
        }
    }

    /// Changes how many lines are kept above the screen. Fewer, and the
    /// oldest go now; more, and the room is there for what comes.
    pub(crate) fn set_scrollback(&mut self, scrollback: usize) {
        if scrollback == self.scrollback {
            return;
        }
        self.scrollback = scrollback;
        self.term.set_options(Self::config(scrollback));
        self.trim_stamps();
        self.revision += 1;
    }

    #[cfg(test)]
    pub(crate) fn scrollback(&self) -> usize {
        self.scrollback
    }

    /// Which state of the grid this is: a search made at one revision is
    /// stale at any other.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Which log this is, counting clears: a line number kept from one
    /// epoch names nothing in the next.
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    /// How many rows the screen holds.
    pub(crate) fn screen_lines(&self) -> usize {
        self.size.lines
    }

    /// Lets go of the stamps of lines the scrollback no longer holds,
    /// carrying their count into the numbering.
    fn trim_stamps(&mut self) {
        let cap = self.scrollback + STAMP_SLACK;
        if self.stamps.len() > cap {
            let drop = self.stamps.len() - cap;
            self.stamps.drain(..drop);
            self.first_number += drop;
        }
    }

    /// Takes what the port read, stamped with the time it arrived, and
    /// returns what the terminal wants written back, usually nothing.
    ///
    /// A read that leaves the cursor at the head of a line is not fed yet
    /// but kept, to go in with the next read — or on its own, from
    /// [`Terminal::flush`], if none comes within the hold. What was kept
    /// goes in first, so nothing is reordered.
    pub(crate) fn receive(&mut self, bytes: &[u8], time: &str) -> Vec<u8> {
        self.feed_pending();
        if matches!(bytes.last(), Some(b'\r' | b'\n')) {
            self.pending = Some((bytes.to_vec(), time.to_owned()));
        } else {
            self.stamp_and_advance(bytes, time);
        }
        self.answers()
    }

    /// Feeds the read kept back, if there is one, and returns what the
    /// terminal wants written back for it.
    pub(crate) fn flush(&mut self) -> Vec<u8> {
        self.feed_pending();
        self.answers()
    }

    /// Whether a read is being kept back for the next.
    pub(crate) fn holds(&self) -> bool {
        self.pending.is_some()
    }

    /// Feeds bytes straight in, with no hold, after anything kept back:
    /// the workbench's own lines, and a test's device. Returns what the
    /// terminal wants written back.
    pub(crate) fn feed(&mut self, bytes: &[u8], time: &str) -> Vec<u8> {
        self.feed_pending();
        self.stamp_and_advance(bytes, time);
        self.answers()
    }

    fn feed_pending(&mut self) {
        if let Some((bytes, time)) = self.pending.take() {
            self.stamp_and_advance(&bytes, &time);
        }
    }

    fn answers(&mut self) -> Vec<u8> {
        std::mem::take(&mut *self.outbox.0.borrow_mut())
    }

    /// Runs bytes through the emulation, stamping the lines they begin
    /// with `time`. A view that was following the output goes on
    /// following it.
    fn stamp_and_advance(&mut self, bytes: &[u8], time: &str) {
        if bytes.is_empty() {
            return;
        }
        for &byte in bytes {
            if byte == b'\n' {
                if self.unstamped {
                    self.stamps.push_back(time.to_owned());
                }
                self.unstamped = true;
            } else if self.unstamped {
                self.stamps.push_back(time.to_owned());
                self.unstamped = false;
            }
        }
        self.trim_stamps();
        let following = self.is_at_bottom();
        self.advance(bytes);
        if following {
            self.scroll_to_bottom();
        }
        self.revision += 1;
    }

    /// Runs the bytes through the emulation, watching for the erases that
    /// take lines away, since the stamps have to lose the same lines: an
    /// erase of the whole screen — or of the screen below the cursor, from
    /// the home position — wipes the log, and an erase of the scrollback
    /// drops the stamps of the lines it held. An erase a read cut short is
    /// held back until the next read completes it.
    fn advance(&mut self, bytes: &[u8]) {
        let joined;
        let bytes = if self.held.is_empty() {
            bytes
        } else {
            self.held.extend_from_slice(bytes);
            joined = std::mem::take(&mut self.held);
            &joined
        };
        let mut start = 0;
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] != 0x1b {
                at += 1;
                continue;
            }
            match erase(&bytes[at..]) {
                Erase::None => at += 1,
                Erase::Unfinished => {
                    self.parser.advance(&mut self.term, &bytes[start..at]);
                    self.held = bytes[at..].to_vec();
                    return;
                }
                Erase::Screen { length, mode } => {
                    self.parser.advance(&mut self.term, &bytes[start..at]);
                    let sequence = &bytes[at..at + length];
                    let home = self.term.grid().cursor.point == Point::new(Line(0), Column(0));
                    let alternate = self.term.mode().contains(TermMode::ALT_SCREEN);
                    match mode {
                        _ if alternate => self.parser.advance(&mut self.term, sequence),
                        2 => self.wipe(),
                        0 if home => self.wipe(),
                        3 => {
                            let keep = self.stamped_on_screen();
                            self.parser.advance(&mut self.term, sequence);
                            self.keep_last_stamps(keep);
                        }
                        _ => self.parser.advance(&mut self.term, sequence),
                    }
                    at += length;
                    start = at;
                }
            }
        }
        self.parser.advance(&mut self.term, &bytes[start..]);
    }

    /// Wipes the screen and the scrollback, as `clear` asks. The line the
    /// cursor is on keeps its stamp, since what the device prints next —
    /// its prompt — lands there; every other line is gone with its time.
    fn wipe(&mut self) {
        let keep = usize::from(!self.unstamped);
        self.parser.advance(&mut self.term, b"\x1b[2J\x1b[3J");
        self.keep_last_stamps(keep);
        // The log starts over, and so does the count: the line kept, if
        // any, is the first of what comes next.
        self.first_number = 1;
        self.epoch += 1;
    }

    /// How many lines on screen, from the top row down to the cursor's,
    /// have a stamp: the rows that begin a line, less the cursor's while
    /// it is still empty.
    fn stamped_on_screen(&self) -> usize {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let cursor_line = grid.cursor.point.line.0.max(0);
        let hard = (0..=cursor_line)
            .filter(|&line| {
                let above = Line(line - 1);
                line - 1 < -history
                    || !grid[above]
                        .last()
                        .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE))
            })
            .count();
        hard.saturating_sub(usize::from(self.unstamped))
    }

    /// Drops every stamp but the newest `count`. The lines let go still
    /// count towards the numbering, as an erase of the scrollback is not a
    /// fresh start.
    fn keep_last_stamps(&mut self, count: usize) {
        let drop = self.stamps.len().saturating_sub(count);
        self.stamps.drain(..drop);
        self.first_number += drop;
    }

    /// Prints a line of the workbench's own — a port opening, a scan —
    /// greyed so it is not taken for the device's. It starts on a fresh
    /// line if the device left the cursor part way through one.
    pub(crate) fn note(&mut self, text: &str, time: &str) {
        self.feed_pending();
        let mut bytes = Vec::with_capacity(text.len() + 16);
        if self.term.grid().cursor.point.column.0 != 0 {
            bytes.extend_from_slice(b"\r\n");
        }
        bytes.extend_from_slice(b"\x1b[90m");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[0m\r\n");
        let _ = self.feed(&bytes, time);
    }

    /// Fits the grid to the cells the view has room for. Lines already on
    /// screen reflow, as they do in any terminal.
    pub(crate) fn resize(&mut self, columns: usize, lines: usize) {
        let size = GridSize {
            columns: columns.max(2),
            lines: lines.max(1),
        };
        if size != self.size {
            let following = self.is_at_bottom();
            self.size = size;
            self.term.resize(size);
            if following {
                self.scroll_to_bottom();
            }
            self.revision += 1;
        }
    }

    /// Wipes the screen, the scrollback and the stamps, and starts the
    /// numbering over.
    pub(crate) fn clear(&mut self) {
        let revision = self.revision + 1;
        let epoch = self.epoch + 1;
        *self = Self::with_size(self.size, self.scrollback);
        self.revision = revision;
        self.epoch = epoch;
    }

    /// Moves the view through the scrollback: positive is back in time.
    pub(crate) fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    /// Brings a line of the grid to the middle of the screen, as near as
    /// the ends of the scrollback allow.
    pub(crate) fn scroll_to_line(&mut self, line: i32) {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let current = grid.display_offset() as i32;
        let wanted = (self.size.lines as i32 / 2 - line).clamp(0, history);
        if wanted != current {
            self.term.scroll_display(Scroll::Delta(wanted - current));
        }
    }

    /// How far back the view is scrolled, in lines: the grid line drawn
    /// on the first row of the screen is minus this.
    pub(crate) fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// How many lines the scrollback holds above the screen.
    pub(crate) fn history_lines(&self) -> usize {
        self.term.grid().history_size()
    }

    /// Puts the view this many lines back from the newest, as far as the
    /// scrollback reaches: the scrollbar's way of scrolling, which names
    /// where to be rather than how far to go.
    pub(crate) fn scroll_to_offset(&mut self, offset: usize) {
        let wanted = offset.min(self.history_lines()) as i32;
        let current = self.display_offset() as i32;
        if wanted != current {
            self.term.scroll_display(Scroll::Delta(wanted - current));
        }
    }

    /// A cell clamped onto the grid, as the point alacritty anchors a
    /// selection to and the side of it the pointer was on.
    fn anchor(&self, cell: GridCell) -> (Point, Side) {
        let history = self.term.grid().history_size() as i32;
        let line = cell.line.clamp(-history, self.size.lines as i32 - 1);
        let column = cell.column.min(self.size.columns - 1);
        let side = if cell.right_half {
            Side::Right
        } else {
            Side::Left
        };
        (Point::new(Line(line), Column(column)), side)
    }

    /// Starts a selection at a cell — the cell, the word it is in or its
    /// whole line — to be grown from there by [`extend_selection`]. What
    /// was selected before is let go.
    ///
    /// [`extend_selection`]: Self::extend_selection
    pub(crate) fn begin_selection(&mut self, cell: GridCell, kind: SelectionKind) {
        let (point, side) = self.anchor(cell);
        let ty = match kind {
            SelectionKind::Cells => SelectionType::Simple,
            SelectionKind::Words => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
        };
        self.term.selection = Some(Selection::new(ty, point, side));
    }

    /// Moves the far end of the selection to a cell. With nothing to grow,
    /// a selection of cells starts there.
    pub(crate) fn extend_selection(&mut self, cell: GridCell) {
        let (point, side) = self.anchor(cell);
        match &mut self.term.selection {
            Some(selection) => selection.update(point, side),
            None => {
                self.term.selection = Some(Selection::new(SelectionType::Simple, point, side));
            }
        }
    }

    /// Selects the whole log, from the top of the scrollback to the last
    /// line on screen that holds anything: the blank rows under the log
    /// are not part of it, and would only copy as blank lines.
    pub(crate) fn select_all(&mut self) {
        let grid = self.term.grid();
        let last = (0..self.size.lines as i32)
            .rev()
            .map(Line)
            .find(|&line| grid[line].line_length().0 > 0)
            .unwrap_or(Line(0));
        let start = Point::new(self.term.topmost_line(), Column(0));
        let end = Point::new(last, self.term.last_column());
        let mut selection = Selection::new(SelectionType::Lines, start, Side::Left);
        selection.update(end, Side::Right);
        self.term.selection = Some(selection);
    }

    /// Lets go of the selection; says whether there was one to let go of.
    pub(crate) fn clear_selection(&mut self) -> bool {
        self.term.selection.take().is_some()
    }

    /// Whether any text is selected. A press that was dragged over nothing
    /// selects nothing, and a selection the scrollback has let go of is
    /// gone with it.
    pub(crate) fn has_selection(&self) -> bool {
        self.term.selection.as_ref().is_some_and(|selection| {
            !selection.is_empty() && selection.to_range(&self.term).is_some()
        })
    }

    /// The selected text, as a terminal copies it: rows the terminal
    /// wrapped joined back into their line, and the blank end of every
    /// line dropped.
    pub(crate) fn selection_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    /// Where the selection begins: the row of the grid and the column of
    /// its first cell — where a find seeded from it should land.
    pub(crate) fn selection_start(&self) -> Option<(i32, usize)> {
        let range = self.term.selection.as_ref()?.to_range(&self.term)?;
        Some((range.start.line.0, range.start.column.0))
    }

    /// The selected text of the lines `keep` says, by number, with the
    /// rest left out: what a copy takes while the mask hides lines the
    /// selection runs across. Each line reads as [`selection_text`] reads
    /// it, its wrapped rows joined.
    ///
    /// [`selection_text`]: Self::selection_text
    pub(crate) fn selection_text_within(&self, keep: impl Fn(i64) -> bool) -> Option<String> {
        let (lines, whole_lines) = self.selection_lines(keep)?;
        let mut text = lines
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            return None;
        }
        if whole_lines {
            text.push('\n');
        }
        Some(text)
    }

    /// The same selection with the time of each line before it, laid out
    /// as the gutter lays it: the stamp, then the line. A line the log has
    /// no stamp for — the blank rows a cleared grid starts with — keeps
    /// the column and says nothing in it, so the text stays in one column
    /// however the stamps run.
    pub(crate) fn selection_text_stamped(&self, keep: impl Fn(i64) -> bool) -> Option<String> {
        let (lines, whole_lines) = self.selection_lines(keep)?;
        if lines.iter().all(|(_, text)| text.is_empty()) {
            return None;
        }
        let mut text = lines
            .iter()
            .map(|(number, text)| {
                let stamp = self
                    .stamp_for(*number)
                    .map_or_else(String::new, |(stamp, _)| stamp);
                format!("{stamp:<STAMP_WIDTH$}{STAMP_GAP}{text}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        if whole_lines {
            text.push('\n');
        }
        Some(text)
    }

    /// The lines the selection covers, in order, each with its number and
    /// what it says — the rows the terminal wrapped joined back together,
    /// and the lines `keep` leaves out left out — and whether the
    /// selection is of whole lines, which a copy ends with a newline.
    fn selection_lines(&self, keep: impl Fn(i64) -> bool) -> Option<(Vec<(i64, String)>, bool)> {
        let selection = self.term.selection.as_ref()?;
        let range = selection.to_range(&self.term)?;
        let last_column = self.term.last_column();
        let mut parts = Vec::new();
        for line in self.logical_lines() {
            if line.end <= range.start.line.0 || line.start > range.end.line.0 {
                continue;
            }
            if !keep(line.number) {
                continue;
            }
            let start = if range.start.line.0 >= line.start {
                range.start
            } else {
                Point::new(Line(line.start), Column(0))
            };
            let end = if range.end.line.0 < line.end {
                range.end
            } else {
                Point::new(Line(line.end - 1), last_column)
            };
            parts.push((line.number, self.term.bounds_to_string(start, end)));
        }
        Some((parts, matches!(selection.ty, SelectionType::Lines)))
    }

    /// How wide the line numbers run: the digits of the highest number in
    /// the log, and never fewer than a short log is sized for.
    pub(crate) fn number_digits(&self) -> usize {
        let highest = (self.first_number + self.stamps.len()).saturating_sub(1).max(1);
        let digits = highest.checked_ilog10().map_or(1, |log| log as usize + 1);
        digits.max(MIN_NUMBER_DIGITS)
    }

    /// Whether a row continues the line above it: that row was wrapped by
    /// the terminal rather than ended by the device.
    fn is_continuation(&self, line: i32) -> bool {
        let grid = self.term.grid();
        line > -(grid.history_size() as i32)
            && grid[Line(line - 1)]
                .last()
                .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE))
    }

    /// The number of the first line in the grid — the top of the
    /// scrollback — from which every line below is numbered in turn. Found
    /// from the cursor's line, whose stamp is the newest: counting the
    /// lines from the top to it says how far the newest number is from the
    /// first. Below the first stamp, for a grid cleared to blank rows, the
    /// count runs into what would have been negative; those rows have no
    /// text, so the number never shows.
    fn first_line_number(&self) -> i64 {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let cursor_line = grid.cursor.point.line.0;
        let hard_at_cursor = (-history..=cursor_line)
            .filter(|&line| !self.is_continuation(line))
            .count()
            .saturating_sub(1);
        let anchor = hard_at_cursor as i64 - i64::from(self.unstamped);
        self.first_number as i64 + self.stamps.len() as i64 - 1 - anchor
    }

    /// The last row of the log: the cursor's, or the last row under it
    /// that holds anything, whichever is lower. The cursor's own row is
    /// left out while nothing has landed on it yet — it is where the next
    /// line will go, not a line.
    fn last_content_line(&self) -> i32 {
        let grid = self.term.grid();
        let cursor = grid.cursor.point.line.0;
        let last_with_text = (0..self.size.lines as i32)
            .rev()
            .find(|&line| grid[Line(line)].line_length().0 > 0);
        let last = last_with_text.map_or(cursor, |line| line.max(cursor));
        if last == cursor && self.unstamped && grid[Line(cursor)].line_length().0 == 0 {
            cursor - 1
        } else {
            last
        }
    }

    /// The lines of the log as the device ended them, oldest first: each
    /// with its number and the rows the terminal laid it over, from the
    /// top of the scrollback to the last row that holds anything.
    pub(crate) fn logical_lines(&self) -> impl Iterator<Item = LogicalLine> + '_ {
        let grid = self.term.grid();
        let last = self.last_content_line();
        let mut number = self.first_line_number();
        let mut line = -(grid.history_size() as i32);
        std::iter::from_fn(move || {
            if line > last {
                return None;
            }
            let start = line;
            loop {
                let wrapped = grid[Line(line)]
                    .last()
                    .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE));
                line += 1;
                if !wrapped || line > last {
                    break;
                }
            }
            let found = LogicalLine {
                number,
                start,
                end: line,
            };
            number += 1;
            Some(found)
        })
    }

    /// What a line says: the characters of its rows in turn, a wide
    /// character once, and the blank end dropped.
    pub(crate) fn line_text(&self, line: &LogicalLine) -> String {
        let grid = self.term.grid();
        let mut text = String::new();
        for at in line.rows() {
            text.extend(
                grid[Line(at)][..]
                    .iter()
                    .filter(|cell| !is_spacer(cell.flags))
                    .map(|cell| cell.c),
            );
        }
        text.truncate(text.trim_end().len());
        text
    }

    /// Where a pattern occurs in the log, oldest first, as cells, in the
    /// lines `keep` says by number — every line, or the ones the mask
    /// shows. Each logical line is read whole — the rows the terminal
    /// wrapped it over joined back — and a match that straddles the wrap
    /// comes back as one match of two spans. A match is named by its
    /// line's number and how far along the line it starts, which is the
    /// same match through a scroll, a reflow or new lines pushing the
    /// rows up.
    pub(crate) fn find(&self, matcher: &OutputFilter, keep: impl Fn(i64) -> bool) -> Vec<FindMatch> {
        let grid = self.term.grid();
        let mut matches = Vec::new();
        for line in self.logical_lines() {
            if !keep(line.number) {
                continue;
            }
            let text = self.line_text(&line);
            let ranges = matcher.find_ranges(&text);
            if ranges.is_empty() {
                continue;
            }
            // One cell per character of the text, in order, for the few
            // lines that hold a match.
            let cells: Vec<FindSpan> = line
                .rows()
                .flat_map(|at| {
                    grid[Line(at)][..]
                        .iter()
                        .enumerate()
                        .filter(|(_, cell)| !is_spacer(cell.flags))
                        .map(move |(column, cell)| FindSpan {
                            line: at,
                            column,
                            width: if cell.flags.contains(Flags::WIDE_CHAR) {
                                2
                            } else {
                                1
                            },
                        })
                })
                .collect();
            for range in ranges {
                let start = text[..range.start].chars().count();
                let end = start + text[range].chars().count();
                let mut spans: Vec<FindSpan> = Vec::new();
                for cell in &cells[start..end.min(cells.len())] {
                    match spans.last_mut() {
                        Some(span)
                            if span.line == cell.line
                                && span.column + span.width == cell.column =>
                        {
                            span.width += cell.width;
                        }
                        _ => spans.push(*cell),
                    }
                }
                if !spans.is_empty() {
                    matches.push(FindMatch {
                        line: line.number,
                        offset: start,
                        spans,
                    });
                }
            }
        }
        matches
    }

    pub(crate) fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub(crate) fn is_at_bottom(&self) -> bool {
        self.term.grid().display_offset() == 0
    }

    pub(crate) fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// Whether the cursor blinks: it does unless the program on the other
    /// end asked for a steady one.
    pub(crate) fn cursor_blinks(&self) -> bool {
        self.term.cursor_style().blinking
    }

    /// Where the cursor is on screen, as (line, column), while it is in
    /// view rather than scrolled away.
    pub(crate) fn cursor_position(&self) -> Option<(usize, usize)> {
        let grid = self.term.grid();
        let line = grid.cursor.point.line.0 + grid.display_offset() as i32;
        (0..self.size.lines as i32)
            .contains(&line)
            .then_some((line as usize, grid.cursor.point.column.0))
    }

    /// The rows of the line the cursor is on — the whole logical line,
    /// wrapped rows included — which is still being written and is drawn
    /// in plain ink; see [`render_rows`].
    ///
    /// [`render_rows`]: Self::render_rows
    fn unfinished_lines(&self) -> Range<i32> {
        let cursor_line = self.term.grid().cursor.point.line.0;
        let lines = self.size.lines as i32;
        let mut unfinished = cursor_line..cursor_line + 1;
        while self.is_continuation(unfinished.start) {
            unfinished.start -= 1;
        }
        while unfinished.end < lines && self.is_continuation(unfinished.end) {
            unfinished.end += 1;
        }
        unfinished
    }

    /// The stamp of the line with a number, and the number itself, while
    /// the log still holds the line.
    fn stamp_for(&self, number: i64) -> Option<(String, usize)> {
        let index = usize::try_from(number - self.first_number as i64).ok()?;
        self.stamps
            .get(index)
            .map(|stamp| (stamp.clone(), self.first_number + index))
    }

    /// The cells of a row the selection covers: from edge to edge on a row
    /// it runs through, from and to the cell it was dragged over on its
    /// first and last rows, and a wide character's second cell along with
    /// its first.
    fn selected_cells(&self, range: &SelectionRange, line: i32) -> Option<Range<usize>> {
        if line < range.start.line.0 || line > range.end.line.0 {
            return None;
        }
        let columns = self.size.columns;
        let start = if range.is_block || line == range.start.line.0 {
            range.start.column.0
        } else {
            0
        };
        let mut end = if range.is_block || line == range.end.line.0 {
            range.end.column.0 + 1
        } else {
            columns
        };
        if end < columns
            && self.term.grid()[Line(line)][Column(end - 1)]
                .flags
                .contains(Flags::WIDE_CHAR)
        {
            end += 1;
        }
        let end = end.min(columns);
        (start < end).then_some(start..end)
    }

    /// The screen as it is to be drawn, colours resolved against the theme.
    /// Text the device left in the default colour takes the colour of what
    /// it says once its line is finished; the line the cursor is still on
    /// stays plain, and what the device coloured itself is kept.
    pub(crate) fn render(&self, palette: &TerminalPalette) -> RenderContent {
        let grid = self.term.grid();
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        let lines = self.size.lines;
        let history = grid.history_size() as i32;

        // Rows that begin a line, numbered from the top of the scrollback:
        // one pass up to the cursor, noting the number as it passes the
        // first row on screen.
        let first_visible = -offset;
        let cursor_line = grid.cursor.point.line.0;
        let (mut hard_at_first, mut hard_at_cursor) = (0, 0);
        let mut count = 0usize;
        for at in -history..=cursor_line.max(first_visible) {
            if !self.is_continuation(at) {
                count += 1;
            }
            if at == first_visible {
                hard_at_first = count.saturating_sub(1);
            }
            if at == cursor_line {
                hard_at_cursor = count.saturating_sub(1);
            }
        }
        // The last stamped line is the cursor's, or the one above it while
        // the cursor's line is still empty. A hard row's number is found by
        // counting back from it; a row past the last stamp, or before the
        // first, gets a number the log has no stamp for, and shows none.
        let anchor = hard_at_cursor as i64 - i64::from(self.unstamped);
        let number_of = |hard: usize| -> i64 {
            self.first_number as i64 + self.stamps.len() as i64 - 1 - (anchor - hard as i64)
        };
        let mut hard = hard_at_first;
        let specs: Vec<(i32, Option<i64>)> = (0..lines)
            .map(|index| {
                let line = index as i32 - offset;
                let continuation = self.is_continuation(line);
                if index > 0 && !continuation {
                    hard += 1;
                }
                (line, (!continuation).then(|| number_of(hard)))
            })
            .collect();
        let rows = self.render_rows(&specs, palette);

        let cursor = (content.cursor.shape != CursorShape::Hidden)
            .then(|| {
                let line = content.cursor.point.line.0 + offset;
                (0..lines as i32).contains(&line).then(|| RenderCursor {
                    line: line as usize,
                    column: content.cursor.point.column.0,
                    wide: grid[content.cursor.point]
                        .flags
                        .contains(Flags::WIDE_CHAR),
                    shape: match content.cursor.shape {
                        CursorShape::Underline => CaretShape::Underline,
                        CursorShape::Beam => CaretShape::Beam,
                        CursorShape::HollowBlock => CaretShape::Hollow,
                        CursorShape::Block | CursorShape::Hidden => CaretShape::Block,
                    },
                })
            })
            .flatten();

        RenderContent { rows, cursor }
    }

    /// Rows of the grid, named by row, as they are to be drawn: each with
    /// its runs of styled text, its plain text, the cells the selection
    /// covers, and — on a row that begins a line, given the line's number
    /// — the line's stamp and number. The screen is its rows in order;
    /// the mask draws the rows of the lines it keeps the same way.
    pub(crate) fn render_rows(
        &self,
        specs: &[(i32, Option<i64>)],
        palette: &TerminalPalette,
    ) -> Vec<RenderRow> {
        let grid = self.term.grid();
        let colors = self.term.colors();
        let selection = self
            .term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(&self.term));
        let highlighter = Highlighter::shared();
        let plain = (
            Color::Named(NamedColor::Foreground),
            Color::Named(NamedColor::Background),
        );
        // The line the cursor is on is still being written — by the device,
        // or by whoever is typing at its prompt — and stays in plain ink
        // until it is done: colours shifting under the caret as each
        // character lands are a distraction, not a reading. The whole
        // logical line, wrapped rows included, waits together.
        let unfinished = self.unfinished_lines();

        struct Pending {
            column: usize,
            width: usize,
            text: String,
            flags: Flags,
            fg: Color,
            bg: Color,
        }

        specs
            .iter()
            .map(|&(line, begins)| {
                let (stamp, number) = match begins.and_then(|number| self.stamp_for(number)) {
                    Some((stamp, number)) => (Some(stamp), Some(number)),
                    None => (None, None),
                };
                // Every cell first, unstyled: a row's text has to be whole
                // before the parts of it worth a colour can be found.
                let mut text = String::new();
                let mut columns = Vec::new();
                let mut pending = Vec::new();
                for (column, cell) in grid[Line(line)][..].iter().enumerate() {
                    let flags = cell.flags;
                    if is_spacer(flags) {
                        continue;
                    }
                    let mut glyph = String::new();
                    if flags.contains(Flags::HIDDEN) {
                        glyph.push(' ');
                    } else {
                        glyph.push(cell.c);
                        if let Some(marks) = cell.zerowidth() {
                            glyph.extend(marks);
                        }
                    }
                    text.push(cell.c);
                    columns.push(column);
                    pending.push(Pending {
                        column,
                        width: if flags.contains(Flags::WIDE_CHAR) {
                            2
                        } else {
                            1
                        },
                        text: glyph,
                        flags,
                        fg: cell.fg,
                        bg: cell.bg,
                    });
                }

                // One character of the row's text per cell, so a role found
                // at a position in the text belongs to the cell at that
                // position.
                let roles = (!unfinished.contains(&line)).then(|| highlighter.roles(&text));
                let mut runs: Vec<RenderRun> = Vec::new();
                for (index, cell) in pending.into_iter().enumerate() {
                    let flags = cell.flags;
                    let (mut fg, mut bg) = (cell.fg, cell.bg);
                    if flags.contains(Flags::INVERSE) {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    let background = resolve(bg, colors, palette);
                    let mut style = RunStyle {
                        foreground: resolve(fg, colors, palette),
                        background: (background != palette.background).then_some(background),
                        bold: flags.contains(Flags::BOLD),
                        italic: flags.contains(Flags::ITALIC),
                        underline: flags.intersects(
                            Flags::UNDERLINE
                                | Flags::DOUBLE_UNDERLINE
                                | Flags::UNDERCURL
                                | Flags::DOTTED_UNDERLINE
                                | Flags::DASHED_UNDERLINE,
                        ),
                        strikeout: flags.contains(Flags::STRIKEOUT),
                    };
                    // Only ink the device left plain takes a role's colour:
                    // what it coloured itself, it meant.
                    let role = roles
                        .as_ref()
                        .and_then(|roles| roles.get(index).copied().flatten());
                    if let Some(role) = role
                        && (cell.fg, cell.bg) == plain
                        && !flags.contains(Flags::INVERSE)
                    {
                        let ink = role.style(palette.theme);
                        style.foreground = ink.color;
                        if let Some(ground) = ink.background {
                            style.background = Some(ground);
                        }
                        style.bold |= ink.bold;
                        style.italic |= ink.italic;
                        style.underline |= ink.underline;
                    }
                    if flags.contains(Flags::DIM) {
                        style.foreground = blend(style.foreground, palette.background, 0.5);
                    }
                    match runs.last_mut() {
                        Some(run) if run.column + run.width == cell.column && run.style == style => {
                            run.text.push_str(&cell.text);
                            run.width += cell.width;
                        }
                        _ => runs.push(RenderRun {
                            column: cell.column,
                            width: cell.width,
                            text: cell.text,
                            style,
                        }),
                    }
                }
                text.truncate(text.trim_end().len());
                columns.truncate(text.chars().count());
                // Blank stretches with nothing to show are not worth shaping.
                runs.retain(|run| {
                    run.style.background.is_some()
                        || run.style.underline
                        || run.style.strikeout
                        || !run.text.trim().is_empty()
                });
                let selected = selection
                    .as_ref()
                    .and_then(|range| self.selected_cells(range, line));
                RenderRow {
                    line,
                    stamp,
                    number,
                    runs,
                    text,
                    columns,
                    selected,
                }
            })
            .collect()
    }
}

/// Whether a cell is the second half of a wide character, or the blank a
/// wide character left at a row's end when it would not fit: no character
/// of its own either way.
fn is_spacer(flags: Flags) -> bool {
    flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
}

/// A line as the device ended it, over the rows the terminal laid it on:
/// its number in the log, and its rows — the first, and one past the last
/// — as alacritty numbers them, zero the top of the screen and negative
/// into the scrollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LogicalLine {
    pub(crate) number: i64,
    pub(crate) start: i32,
    pub(crate) end: i32,
}

impl LogicalLine {
    pub(crate) fn rows(&self) -> Range<i32> {
        self.start..self.end
    }

    /// Whether the line is wholly in the scrollback, where nothing is
    /// written again: what it says now is what it will say.
    pub(crate) fn settled(&self) -> bool {
        self.end <= 0
    }
}

/// What the bytes at an escape character are, as far as erasing goes.
#[derive(Debug, PartialEq, Eq)]
enum Erase {
    /// Some other sequence, or a lone escape.
    None,
    /// The start of an erase, or of a sequence that could still turn out
    /// to be one, with the rest yet to arrive.
    Unfinished,
    /// `ESC [ Ps J`, erase in display: `length` bytes, with `mode` the
    /// parameter — 0 below the cursor, 1 above, 2 the screen, 3 the
    /// scrollback.
    Screen { length: usize, mode: u8 },
}

fn erase(bytes: &[u8]) -> Erase {
    match bytes {
        [0x1b] | [0x1b, b'['] | [0x1b, b'[', b'0'..=b'3'] => Erase::Unfinished,
        [0x1b, b'[', b'J', ..] => Erase::Screen { length: 3, mode: 0 },
        [0x1b, b'[', mode @ b'0'..=b'3', b'J', ..] => Erase::Screen {
            length: 4,
            mode: mode - b'0',
        },
        _ => Erase::None,
    }
}

/// The screen, row by row, ready to paint.
pub(crate) struct RenderContent {
    pub(crate) rows: Vec<RenderRow>,
    pub(crate) cursor: Option<RenderCursor>,
}

pub(crate) struct RenderRow {
    /// The row of the grid this is: zero the top of the screen, negative
    /// into the scrollback.
    pub(crate) line: i32,
    /// When the line began, on the row that begins it.
    pub(crate) stamp: Option<String>,
    /// Which line of the log it is, on the row that begins it.
    pub(crate) number: Option<usize>,
    pub(crate) runs: Vec<RenderRun>,
    /// The row's text, for the filter and the find.
    pub(crate) text: String,
    /// The cell each character of the text sits in: a wide character
    /// takes two cells, so the two drift apart after one.
    pub(crate) columns: Vec<usize>,
    /// The cells the selection covers on this row, when it reaches it.
    pub(crate) selected: Option<Range<usize>>,
}

/// One occurrence of what the find bar looks for: a run of cells, or two
/// when the terminal wrapped the line under it. Named by the number of the
/// line it is on and how many characters along it starts — the name a
/// match keeps while the rows under it move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FindMatch {
    pub(crate) line: i64,
    pub(crate) offset: usize,
    pub(crate) spans: Vec<FindSpan>,
}

impl FindMatch {
    /// Where the match begins, on the grid as it was scanned.
    pub(crate) fn start(&self) -> FindSpan {
        self.spans[0]
    }

    /// The match's name: the same occurrence answers to it scan after scan.
    pub(crate) fn key(&self) -> (i64, usize) {
        (self.line, self.offset)
    }
}

/// Cells on one row of the grid: the row as alacritty numbers it — zero
/// the top of the screen, negative into the scrollback — and the cells
/// along it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FindSpan {
    pub(crate) line: i32,
    pub(crate) column: usize,
    pub(crate) width: usize,
}

/// Adjacent cells that share a style, painted as one piece of text.
pub(crate) struct RenderRun {
    pub(crate) column: usize,
    /// In cells: a wide character takes two.
    pub(crate) width: usize,
    pub(crate) text: String,
    pub(crate) style: RunStyle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunStyle {
    pub(crate) foreground: u32,
    /// `None` where the theme's own background shows through.
    pub(crate) background: Option<u32>,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikeout: bool,
}

pub(crate) struct RenderCursor {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) wide: bool,
    pub(crate) shape: CaretShape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaretShape {
    Block,
    Hollow,
    Underline,
    Beam,
}

fn pack(rgb: Rgb) -> u32 {
    (u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b)
}

/// A terminal colour as pixels: what the device set with an escape
/// sequence if it did, else the theme's reading of the name.
fn resolve(color: Color, overrides: &Colors, palette: &TerminalPalette) -> u32 {
    match color {
        Color::Spec(rgb) => pack(rgb),
        Color::Named(named) => overrides[named]
            .map(pack)
            .unwrap_or_else(|| named_color(named, palette)),
        Color::Indexed(index) => overrides[usize::from(index)]
            .map(pack)
            .unwrap_or_else(|| indexed_color(index, palette)),
    }
}

fn named_color(named: NamedColor, palette: &TerminalPalette) -> u32 {
    use NamedColor::*;
    match named {
        Foreground | BrightForeground => palette.foreground,
        Background => palette.background,
        Cursor => palette.cursor,
        DimForeground => blend(palette.foreground, palette.background, 0.5),
        DimBlack | DimRed | DimGreen | DimYellow | DimBlue | DimMagenta | DimCyan | DimWhite => {
            let index = named as usize - DimBlack as usize;
            blend(palette.ansi[index], palette.background, 0.5)
        }
        _ => palette.ansi[(named as usize) & 0xF],
    }
}

/// The 256-colour table: the sixteen ANSI colours, a 6×6×6 cube, and a
/// ramp of greys.
fn indexed_color(index: u8, palette: &TerminalPalette) -> u32 {
    match index {
        0..=15 => palette.ansi[usize::from(index)],
        16..=231 => {
            let index = index - 16;
            let level = |value: u8| -> u32 {
                if value == 0 {
                    0
                } else {
                    55 + 40 * u32::from(value)
                }
            };
            (level(index / 36) << 16) | (level(index / 6 % 6) << 8) | level(index % 6)
        }
        232..=255 => {
            let grey = 8 + 10 * u32::from(index - 232);
            (grey << 16) | (grey << 8) | grey
        }
    }
}

/// `a` moved `amount` of the way towards `b`, per channel.
fn blend(a: u32, b: u32, amount: f32) -> u32 {
    let channel = |shift: u32| -> u32 {
        let from = ((a >> shift) & 0xFF) as f32;
        let to = ((b >> shift) & 0xFF) as f32;
        (from + (to - from) * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

/// The bytes a paste sends. Every line ending goes as the Return key sends
/// one, since that is what a device reads at the end of a line and a CRLF
/// passed on whole would read as two. A program that asked for bracketed
/// paste — a shell with a line editor, most often — gets the text between
/// the two markers that tell it the text was pasted rather than typed, so
/// it does not run each line as it arrives.
pub(crate) fn paste_bytes(text: &str, mode: TermMode) -> Vec<u8> {
    let typed = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend_from_slice(typed.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        return bytes;
    }
    typed.into_bytes()
}

/// The bytes a key sends, the way a terminal emulator sends them: editing
/// and cursor keys as their escape sequences, control-letter as the control
/// character, with xterm's modifier parameter when a modifier is held.
/// Printable text does not come this way — it arrives through the input
/// handler, so an input method can compose it — and keys with the command
/// modifier belong to the menus.
pub(crate) fn key_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let modifiers = keystroke.modifiers;
    if modifiers.platform || modifiers.function {
        return None;
    }
    let key = keystroke.key.as_str();

    // xterm's modifier parameter: 1, plus 1 for shift, 2 for alt, 4 for control.
    let modifier =
        1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.control);
    let plain = modifier == 1;

    if modifiers.control && !modifiers.alt && !modifiers.shift {
        let mut letters = key.chars();
        if let (Some(letter @ 'a'..='z'), None) = (letters.next(), letters.next()) {
            return Some(vec![letter as u8 - b'a' + 1]);
        }
        let control = match key {
            "space" | "2" => Some(0x00),
            "[" | "3" => Some(0x1b),
            "\\" | "4" => Some(0x1c),
            "]" | "5" => Some(0x1d),
            "^" | "6" => Some(0x1e),
            "_" | "7" | "-" => Some(0x1f),
            "?" | "8" => Some(0x7f),
            _ => None,
        };
        if let Some(byte) = control {
            return Some(vec![byte]);
        }
    }

    // Cursor keys have an application form the program can ask for; every
    // other key of that kind keeps the CSI form.
    let cursor = |letter: char| -> Vec<u8> {
        if !plain {
            format!("\x1b[1;{modifier}{letter}")
        } else if mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{letter}")
        } else {
            format!("\x1b[{letter}")
        }
        .into_bytes()
    };
    let tilde = |code: u8| -> Vec<u8> {
        if plain {
            format!("\x1b[{code}~")
        } else {
            format!("\x1b[{code};{modifier}~")
        }
        .into_bytes()
    };
    let function = |letter: char| -> Vec<u8> {
        if plain {
            format!("\x1bO{letter}")
        } else {
            format!("\x1b[1;{modifier}{letter}")
        }
        .into_bytes()
    };

    let bytes = match key {
        "enter" if modifiers.alt => b"\x1b\r".to_vec(),
        "enter" => b"\r".to_vec(),
        "backspace" if modifiers.alt => b"\x1b\x7f".to_vec(),
        "backspace" if modifiers.control => b"\x08".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "tab" if modifiers.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "space" if modifiers.alt => b"\x1b ".to_vec(),
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => function('P'),
        "f2" => function('Q'),
        "f3" => function('R'),
        "f4" => function('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        _ => return None,
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::{
        CaretShape, DEFAULT_SCROLLBACK_LINES, FindMatch, FindSpan, GridCell, GridSize,
        MIN_NUMBER_DIGITS, RenderContent, RenderRun, SelectionKind, Terminal, key_bytes,
        paste_bytes,
    };
    use crate::{filter::OutputFilter, highlight::Role, theme::TerminalPalette};
    use alacritty_terminal::{grid::Dimensions, term::TermMode};
    use gpui_kit::Keystroke;

    fn terminal(columns: usize, lines: usize) -> Terminal {
        Terminal::with_size(GridSize { columns, lines }, DEFAULT_SCROLLBACK_LINES)
    }

    /// The line number on each row of the screen.
    fn numbers(terminal: &Terminal) -> Vec<Option<usize>> {
        terminal
            .render(&TerminalPalette::DARK)
            .rows
            .iter()
            .map(|row| row.number)
            .collect()
    }

    fn rows(terminal: &Terminal) -> Vec<(Option<String>, String)> {
        terminal
            .render(&TerminalPalette::DARK)
            .rows
            .into_iter()
            .map(|row| (row.stamp, row.text))
            .collect()
    }

    fn stamped(time: &str, text: &str) -> (Option<String>, String) {
        (Some(time.into()), text.into())
    }

    fn plain(text: &str) -> (Option<String>, String) {
        (None, text.into())
    }

    #[test]
    fn every_line_shows_the_time_it_began() {
        let mut terminal = terminal(40, 5);
        terminal.feed(b"temp=25\r\n", "10:00:00.000");
        terminal.feed(b"hum", "10:00:01.000");
        terminal.feed(b"=61\r\n", "10:00:02.000");
        terminal.feed(b"\r\n", "10:00:03.000");
        assert_eq!(
            rows(&terminal),
            vec![
                stamped("10:00:00.000", "temp=25"),
                stamped("10:00:01.000", "hum=61"),
                stamped("10:00:03.000", ""),
                plain(""),
                plain(""),
            ]
        );
        // The empty line the cursor sits on has no time until something lands on it.
        terminal.feed(b"OK", "10:00:04.000");
        assert_eq!(rows(&terminal)[3], stamped("10:00:04.000", "OK"));
    }

    /// A shell answers Enter with the newline first and its prompt a
    /// moment later, in a read of its own. The first read is kept back
    /// and goes in with the second, so the cursor goes straight to the
    /// prompt rather than to the head of the line and then on.
    #[test]
    fn a_read_that_ends_at_the_head_of_a_line_waits_for_the_next() {
        let mut terminal = terminal(30, 3);
        terminal.receive(b"root@ubuntu:/# ", "1");
        assert!(!terminal.holds());
        assert_eq!(terminal.cursor_position(), Some((0, 15)));
        terminal.receive(b"\r\n\x1b[?2004l\r", "2");
        assert!(terminal.holds());
        assert_eq!(terminal.cursor_position(), Some((0, 15)));
        terminal.receive(b"\x1b[?2004hroot@ubuntu:/# ", "3");
        assert!(!terminal.holds());
        assert_eq!(terminal.cursor_position(), Some((1, 15)));
        assert_eq!(
            rows(&terminal),
            vec![
                stamped("1", "root@ubuntu:/#"),
                stamped("2", "root@ubuntu:/#"),
                plain("")
            ]
        );
    }

    /// A read with nothing behind it — a device logging a line, a
    /// progress bar's return — goes in when the hold runs out, stamped
    /// with the time it arrived.
    #[test]
    fn a_read_on_its_own_goes_in_when_the_hold_runs_out() {
        let mut terminal = terminal(20, 3);
        terminal.receive(b"temp=25\r\n", "1");
        assert!(terminal.holds());
        assert_eq!(rows(&terminal)[0], plain(""));
        terminal.flush();
        assert!(!terminal.holds());
        assert_eq!(rows(&terminal)[0], stamped("1", "temp=25"));
        terminal.flush();
        terminal.receive(b"50%\r", "2");
        assert_eq!(terminal.cursor_position(), Some((1, 0)));
        terminal.flush();
        terminal.receive(b"100%", "3");
        assert_eq!(
            rows(&terminal),
            vec![stamped("1", "temp=25"), stamped("2", "100%"), plain("")]
        );
    }

    /// A note printed while a read is kept back comes after it, on a
    /// line of its own.
    #[test]
    fn a_note_comes_after_the_read_kept_back() {
        let mut terminal = terminal(20, 4);
        terminal.receive(b"line one\r\n", "1");
        terminal.note("Port closed.", "2");
        assert_eq!(
            rows(&terminal)[..2],
            [stamped("1", "line one"), stamped("2", "Port closed.")]
        );
    }

    /// Lines are numbered from one, a wrapped line once, and the line the
    /// cursor waits on not until it holds something.
    #[test]
    fn lines_are_numbered_from_one() {
        let mut terminal = terminal(5, 5);
        terminal.feed(b"one\r\ntwo\r\nabcdefgh\r\n", "1");
        assert_eq!(
            numbers(&terminal),
            vec![Some(1), Some(2), Some(3), None, None],
            "the wrapped third line takes one number, the empty fourth none yet"
        );
        terminal.feed(b"x", "2");
        assert_eq!(numbers(&terminal)[4], Some(4));
    }

    /// Once the scrollback is full the oldest lines go, but the ones that
    /// stay keep their numbers, and the gutter is sized to the highest.
    #[test]
    fn numbers_keep_counting_past_the_scrollback() {
        let mut terminal = Terminal::with_size(GridSize { columns: 8, lines: 2 }, 3);
        for index in 1..=250 {
            terminal.feed(format!("l{index}\r\n").as_bytes(), "1");
        }
        assert_eq!(numbers(&terminal), vec![Some(250), None]);
        assert!(terminal.first_number > 1, "the stamps of lost lines went");
        assert_eq!(terminal.number_digits(), MIN_NUMBER_DIGITS);
        terminal.first_number = 99_996;
        assert_eq!(terminal.number_digits(), 6);
    }

    /// A clear from the device or the workbench starts the count over; an
    /// erase of the scrollback alone does not.
    #[test]
    fn a_clear_starts_the_numbering_over() {
        let mut terminal = terminal(8, 3);
        terminal.feed(b"one\r\ntwo\r\nthree\r\n", "1");
        terminal.feed(b"\x1b[3J", "2");
        terminal.feed(b"four", "3");
        assert_eq!(numbers(&terminal), vec![Some(2), Some(3), Some(4)]);
        terminal.feed(b"\x1b[2J", "4");
        terminal.feed(b"\r\nfive", "5");
        assert_eq!(
            numbers(&terminal),
            vec![None, Some(1), Some(2)],
            "the cleared row above is no line; the line kept is the first again"
        );
        terminal.clear();
        terminal.feed(b"six", "6");
        assert_eq!(numbers(&terminal)[0], Some(1));
    }

    /// The scrollback can be made smaller or larger as the log runs.
    #[test]
    fn the_scrollback_can_be_resized() {
        let mut terminal = Terminal::with_size(GridSize { columns: 8, lines: 2 }, 100);
        for index in 1..=50 {
            terminal.feed(format!("l{index}\r\n").as_bytes(), "1");
        }
        terminal.set_scrollback(10);
        assert_eq!(terminal.scrollback(), 10);
        assert_eq!(terminal.term.grid().history_size(), 10);
        terminal.set_scrollback(1_000);
        for index in 51..=500 {
            terminal.feed(format!("l{index}\r\n").as_bytes(), "1");
        }
        assert_eq!(terminal.term.grid().history_size(), 460);
        assert_eq!(numbers(&terminal), vec![Some(500), None]);
    }

    /// The find reads wrapped lines whole, answers in cells, and minds
    /// case only when told to.
    #[test]
    fn find_locates_matches_across_a_wrap() {
        let mut terminal = terminal(5, 4);
        terminal.feed(b"abcdefgh\r\nDEF\r\n", "1");
        let mut matcher = OutputFilter::default();
        matcher.set_pattern("def");
        let found = terminal.find(&matcher, |_| true);
        assert_eq!(
            found,
            vec![
                FindMatch {
                    line: 1,
                    offset: 3,
                    spans: vec![
                        FindSpan { line: 0, column: 3, width: 2 },
                        FindSpan { line: 1, column: 0, width: 1 },
                    ],
                },
                FindMatch {
                    line: 2,
                    offset: 0,
                    spans: vec![FindSpan { line: 2, column: 0, width: 3 }],
                },
            ]
        );
        matcher.toggle_match_case();
        assert_eq!(terminal.find(&matcher, |_| true).len(), 1);
        matcher.set_pattern("");
        assert!(terminal.find(&matcher, |_| true).is_empty());
    }

    /// A wide character is one match cell of two columns, and a line in
    /// the scrollback is found at its negative row.
    #[test]
    fn find_counts_wide_characters_and_reaches_the_scrollback() {
        let mut terminal = terminal(10, 2);
        terminal.feed("温度=25\r\nnext\r\nlast\r\n".as_bytes(), "1");
        let mut matcher = OutputFilter::default();
        matcher.set_pattern("度=2");
        assert_eq!(
            terminal.find(&matcher, |_| true),
            vec![FindMatch {
                line: 1,
                offset: 1,
                spans: vec![FindSpan { line: -2, column: 2, width: 4 }],
            }]
        );
        terminal.scroll_to_line(-2);
        assert!(!terminal.is_at_bottom());
        assert_eq!(terminal.render(&TerminalPalette::DARK).rows[0].text, "温度=25");
    }

    /// A match answers to the same name after the rows under it have
    /// moved — new lines pushing them up, or a reflow — so the find keeps
    /// its place by the text and not by the row.
    #[test]
    fn a_match_keeps_its_name_as_the_rows_move() {
        let mut terminal = Terminal::with_size(GridSize { columns: 40, lines: 3 }, 100);
        terminal.feed(b"lost 1\r\nfine\r\nlost 2\r\n", "1");
        let mut matcher = OutputFilter::default();
        matcher.set_pattern("lost");
        let before: Vec<_> = terminal.find(&matcher, |_| true).iter().map(FindMatch::key).collect();
        assert_eq!(before, vec![(1, 0), (3, 0)]);
        terminal.feed(b"lost 3\r\nmore\r\n", "2");
        terminal.resize(8, 6);
        let after: Vec<_> = terminal.find(&matcher, |_| true).iter().map(FindMatch::key).collect();
        assert_eq!(after, vec![(1, 0), (3, 0), (4, 0)]);
    }

    #[test]
    fn a_wrapped_line_carries_its_time_once() {
        let mut terminal = terminal(10, 4);
        terminal.feed(b"abcdefghijklmno\r\n", "1");
        terminal.feed(b"next\r\n", "2");
        assert_eq!(
            rows(&terminal),
            vec![
                stamped("1", "abcdefghij"),
                plain("klmno"),
                stamped("2", "next"),
                plain(""),
            ]
        );
    }

    #[test]
    fn a_note_takes_a_line_of_its_own() {
        let mut terminal = terminal(40, 4);
        terminal.feed(b"login: ", "1");
        terminal.note("Disconnected from /dev/tty", "2");
        assert_eq!(
            rows(&terminal)[..3],
            [
                stamped("1", "login:"),
                stamped("2", "Disconnected from /dev/tty"),
                plain(""),
            ]
        );
    }

    #[test]
    fn the_scrollback_keeps_its_times() {
        let mut terminal = terminal(20, 3);
        for index in 1..=6 {
            terminal.feed(format!("line {index}\r\n").as_bytes(), &index.to_string());
        }
        assert_eq!(rows(&terminal)[0], stamped("5", "line 5"));
        assert!(terminal.is_at_bottom());
        terminal.scroll(3);
        assert_eq!(
            rows(&terminal),
            vec![stamped("2", "line 2"), stamped("3", "line 3"), stamped("4", "line 4")]
        );
        terminal.scroll_to_bottom();
        assert_eq!(rows(&terminal)[1], stamped("6", "line 6"));
        assert_eq!(terminal.cursor_position(), Some((2, 0)));
    }

    /// `clear` on an xterm: home, then erase the whole screen. The log
    /// goes with it — the screen, the scrollback and their times — and the
    /// prompt that follows starts at the top of an empty terminal.
    #[test]
    fn a_clear_from_the_device_wipes_the_log() {
        let mut terminal = terminal(20, 4);
        terminal.feed(b"one\r\ntwo\r\n", "1");
        terminal.feed(b"\x1b[H\x1b[2J$ ", "2");
        assert_eq!(
            rows(&terminal),
            vec![stamped("2", "$"), plain(""), plain(""), plain("")]
        );
        assert_eq!(terminal.cursor_position(), Some((0, 2)));
        assert!(terminal.is_at_bottom());
        // There is nothing above the prompt to scroll back to.
        terminal.scroll(3);
        assert!(terminal.is_at_bottom());
        assert_eq!(rows(&terminal)[0], stamped("2", "$"));
        terminal.feed(b"ls\r\nREADME\r\n", "3");
        assert_eq!(
            rows(&terminal),
            vec![stamped("2", "$ ls"), stamped("3", "README"), plain(""), plain("")]
        );
    }

    /// The lines a clear took out of the scrollback stay out: the wheel
    /// finds nothing above the prompt, and what comes after scrolls as any
    /// log does.
    #[test]
    fn a_clear_empties_the_scrollback() {
        let mut terminal = terminal(20, 3);
        for index in 1..=6 {
            terminal.feed(format!("line {index}\r\n").as_bytes(), &index.to_string());
        }
        terminal.scroll(2);
        assert!(!terminal.is_at_bottom());
        terminal.feed(b"\x1b[H\x1b[2J", "7");
        assert!(terminal.is_at_bottom());
        assert_eq!(rows(&terminal), vec![stamped("7", ""), plain(""), plain("")]);
        terminal.scroll(1);
        assert_eq!(rows(&terminal), vec![stamped("7", ""), plain(""), plain("")]);
        for index in 8..=11 {
            terminal.feed(format!("line {index}\r\n").as_bytes(), &index.to_string());
        }
        assert_eq!(rows(&terminal)[0], stamped("10", "line 10"));
        terminal.scroll(10);
        assert_eq!(
            rows(&terminal),
            vec![stamped("7", "line 8"), stamped("9", "line 9"), stamped("10", "line 10")]
        );
    }

    /// A view made taller shows more of the log, pulled back out of the
    /// scrollback with its times; made shorter, it keeps the bottom.
    #[test]
    fn a_resized_view_keeps_the_bottom_of_the_log() {
        let mut terminal = terminal(20, 3);
        for index in 1..=6 {
            terminal.feed(format!("line {index}\r\n").as_bytes(), &index.to_string());
        }
        terminal.feed(b"$ ", "7");
        terminal.resize(20, 5);
        assert_eq!(
            rows(&terminal),
            vec![
                stamped("3", "line 3"),
                stamped("4", "line 4"),
                stamped("5", "line 5"),
                stamped("6", "line 6"),
                stamped("7", "$"),
            ]
        );
        assert_eq!(terminal.cursor_position(), Some((4, 2)));
        terminal.resize(20, 2);
        assert_eq!(rows(&terminal), vec![stamped("6", "line 6"), stamped("7", "$")]);
        assert!(terminal.is_at_bottom());
    }

    /// `clear` on a vt100: home, then erase to the end of the screen. From
    /// the home position that is the whole screen, and it is taken for the
    /// clear it is; from anywhere else it erases just what it says.
    #[test]
    fn an_erase_from_the_top_is_a_clear() {
        let mut terminal = terminal(20, 3);
        terminal.feed(b"one\r\ntwo\r\n", "1");
        terminal.feed(b"\x1b[H\x1b[J$ ", "2");
        assert_eq!(rows(&terminal), vec![stamped("2", "$"), plain(""), plain("")]);
        terminal.feed(b"abc\x1b[2D\x1b[J", "3");
        assert_eq!(rows(&terminal)[0], stamped("2", "$ a"));
        terminal.feed(b"\r\nnext", "4");
        assert_eq!(
            rows(&terminal),
            vec![stamped("2", "$ a"), stamped("4", "next"), plain("")]
        );
        // Below the cursor, from the top of a screen that has scrolled, is
        // the whole screen again.
        terminal.feed(b"\r\nmore\r\n", "5");
        terminal.feed(b"\x1b[H\x1b[0J# ", "6");
        assert_eq!(rows(&terminal), vec![stamped("6", "#"), plain(""), plain("")]);
        terminal.scroll(5);
        assert!(terminal.is_at_bottom());
    }

    #[test]
    fn an_erase_cut_short_by_a_read_is_still_seen() {
        let mut terminal = terminal(20, 3);
        terminal.feed(b"one\r\ntwo\r\n\x1b[H\x1b", "1");
        terminal.feed(b"[", "2");
        assert_eq!(rows(&terminal)[0], stamped("1", "one"));
        terminal.feed(b"J$ ", "3");
        assert_eq!(rows(&terminal), vec![stamped("1", "$"), plain(""), plain("")]);
        // What is held back is only ever an erase in the making.
        terminal.feed(b"\x1b[1", "4");
        terminal.feed(b"mbold\x1b[0", "4");
        terminal.feed(b"m\r\n", "4");
        let content = terminal.render(&TerminalPalette::DARK);
        assert_eq!(content.rows[0].text, "$ bold");
        assert!(content.rows[0].runs.iter().any(|run| run.text == "bold" && run.style.bold));
    }

    /// `ESC [ 3 J` on its own empties the scrollback and leaves the screen,
    /// and the times of the lines on screen stay with them.
    #[test]
    fn the_device_can_empty_the_scrollback_alone() {
        let mut terminal = terminal(20, 2);
        terminal.feed(b"one\r\ntwo\r\n", "1");
        terminal.feed(b"three", "2");
        terminal.scroll(1);
        assert_eq!(rows(&terminal), vec![stamped("1", "one"), stamped("1", "two")]);
        terminal.feed(b"\x1b[3J", "3");
        assert!(terminal.is_at_bottom());
        assert_eq!(rows(&terminal), vec![stamped("1", "two"), stamped("2", "three")]);
        terminal.scroll(1);
        assert_eq!(rows(&terminal), vec![stamped("1", "two"), stamped("2", "three")]);
    }

    /// `clear` as ncurses sends it to an xterm: home, the screen, and the
    /// scrollback too. Nothing is left, in whichever reads it arrives.
    #[test]
    fn a_full_clear_leaves_nothing() {
        let mut terminal = terminal(20, 2);
        terminal.feed(b"one\r\ntwo\r\nthree\r\n", "1");
        terminal.feed(b"\x1b[H\x1b[2J", "2");
        terminal.feed(b"\x1b[3J$ ", "2");
        assert_eq!(rows(&terminal), vec![stamped("2", "$"), plain("")]);
        terminal.scroll(1);
        assert_eq!(rows(&terminal), vec![stamped("2", "$"), plain("")]);
    }

    /// A full-screen program clears the alternate screen as it draws; the
    /// log under it is untouched, and comes back when the program ends.
    #[test]
    fn the_alternate_screen_clears_itself_and_not_the_log() {
        let mut terminal = terminal(20, 2);
        terminal.feed(b"one\r\ntwo\r\n", "1");
        terminal.feed(b"\x1b[?1049h\x1b[H\x1b[2Jmenu", "2");
        assert_eq!(rows(&terminal)[0].1, "menu");
        terminal.feed(b"\x1b[2J\x1b[3J", "3");
        terminal.feed(b"\x1b[?1049l", "4");
        assert_eq!(rows(&terminal), vec![stamped("1", "two"), stamped("2", "")]);
        terminal.scroll(1);
        assert_eq!(rows(&terminal), vec![stamped("1", "one"), stamped("1", "two")]);
    }

    #[test]
    fn a_short_log_starts_at_the_top() {
        let mut terminal = terminal(20, 4);
        terminal.feed(b"one\r\n", "1");
        assert_eq!(
            rows(&terminal),
            vec![stamped("1", "one"), plain(""), plain(""), plain("")]
        );
        assert_eq!(terminal.cursor_position(), Some((1, 0)));
        // A clear with nothing behind it leaves the prompt at the top too.
        terminal.feed(b"\x1b[H\x1b[2J$ ", "2");
        assert_eq!(
            rows(&terminal),
            vec![stamped("2", "$"), plain(""), plain(""), plain("")]
        );
    }

    #[test]
    fn erases_are_told_apart() {
        use super::{Erase, erase};
        assert_eq!(erase(b"\x1b"), Erase::Unfinished);
        assert_eq!(erase(b"\x1b["), Erase::Unfinished);
        assert_eq!(erase(b"\x1b[2"), Erase::Unfinished);
        assert_eq!(erase(b"\x1b[J"), Erase::Screen { length: 3, mode: 0 });
        assert_eq!(erase(b"\x1b[0Jx"), Erase::Screen { length: 4, mode: 0 });
        assert_eq!(erase(b"\x1b[2J"), Erase::Screen { length: 4, mode: 2 });
        assert_eq!(erase(b"\x1b[3J"), Erase::Screen { length: 4, mode: 3 });
        assert_eq!(erase(b"\x1b[2K"), Erase::None);
        assert_eq!(erase(b"\x1b[1;32m"), Erase::None);
        assert_eq!(erase(b"\x1b[?J"), Erase::None);
        assert_eq!(erase(b"\x1b]0;title\x07"), Erase::None);
    }

    #[test]
    fn the_device_draws_the_way_a_terminal_shows_it() {
        let mut terminal = terminal(30, 3);
        terminal.feed(b"\x1b[1;32mroot@board\x1b[0m:~# lss\x08 -la\r\n", "1");
        terminal.feed(b"10%\r50%\r100%\r\n", "2");
        terminal.feed("温度 25°C".as_bytes(), "3");
        let content = terminal.render(&TerminalPalette::DARK);
        assert_eq!(content.rows[0].text, "root@board:~# ls -la");
        assert_eq!(content.rows[1].text, "100%");
        assert_eq!(content.rows[2].text, "温度 25°C");
        let prompt = &content.rows[0].runs[0];
        assert_eq!(prompt.text, "root@board");
        assert!(prompt.style.bold);
        assert_eq!(prompt.style.foreground, TerminalPalette::DARK.ansi[2]);
        let cursor = content.cursor.unwrap();
        assert_eq!((cursor.line, cursor.column), (2, 9));
        assert_eq!(cursor.shape, CaretShape::Block);
        // A wide character takes two cells: nine for the seven characters.
        let temperature = &content.rows[2].runs[0];
        assert_eq!(temperature.column, 0);
        assert_eq!(temperature.text.trim_end(), "温度 25°C");
        assert!(temperature.width >= 9);
    }

    /// The run on `line` whose text is exactly `text`.
    fn run<'a>(content: &'a RenderContent, line: usize, text: &str) -> &'a RenderRun {
        content.rows[line]
            .runs
            .iter()
            .find(|run| run.text == text)
            .unwrap_or_else(|| panic!("a run {text:?} on row {line}"))
    }

    #[test]
    fn plain_text_is_coloured_by_what_it_says() {
        let mut terminal = terminal(60, 3);
        terminal.feed(b"E (1234) wifi: lost 192.168.1.20 after 350ms\r\n", "1");
        terminal.feed(b"GET /index.html\r\n", "2");
        let palette = TerminalPalette::DARK;
        let content = terminal.render(&palette);
        let colour = |role: Role| role.style(palette.theme).color;
        // A pill: the page's ink on a ground of the role's own.
        let method = run(&content, 1, "GET");
        assert_eq!(method.style.foreground, palette.background);
        assert_eq!(method.style.background, Some(0x5be49b));
        let level = run(&content, 0, "E");
        assert_eq!(level.style.foreground, colour(Role::Error));
        assert!(level.style.bold);
        assert_eq!(
            run(&content, 0, "1234").style.foreground,
            colour(Role::Uptime)
        );
        assert_eq!(run(&content, 0, "wifi").style.foreground, colour(Role::Tag));
        assert_eq!(
            run(&content, 0, "lost").style.foreground,
            colour(Role::Warning)
        );
        assert_eq!(
            run(&content, 0, "192").style.foreground,
            colour(Role::IpDigit)
        );
        let unit = run(&content, 0, "ms");
        assert_eq!(unit.style.foreground, colour(Role::DurationUnit));
        assert!(unit.style.italic);
        // Words with no role keep the terminal's ink, and the text the
        // filter reads is the text the device sent.
        assert_eq!(
            run(&content, 0, " after ").style.foreground,
            palette.foreground
        );
        assert_eq!(
            content.rows[0].text,
            "E (1234) wifi: lost 192.168.1.20 after 350ms"
        );
        // The light theme picks the same roles in its own colours.
        let light = terminal.render(&TerminalPalette::LIGHT);
        assert_eq!(
            run(&light, 0, "E").style.foreground,
            Role::Error.style(TerminalPalette::LIGHT.theme).color
        );
        assert_ne!(run(&light, 0, "E").style.foreground, level.style.foreground);
    }

    #[test]
    fn a_devices_own_colours_are_kept() {
        let mut terminal = terminal(40, 2);
        terminal.feed(b"\x1b[32mERROR\x1b[0m ERROR \x1b[7mERROR\x1b[0m\r\n", "1");
        let palette = TerminalPalette::DARK;
        let content = terminal.render(&palette);
        // Blank stretches are not shaped, so the three words are the runs.
        let runs = &content.rows[0].runs;
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "ERROR");
        assert_eq!(runs[0].style.foreground, palette.ansi[2]);
        assert_eq!(runs[1].text, "ERROR");
        assert_eq!(
            runs[1].style.foreground,
            Role::Error.style(palette.theme).color
        );
        // Inverse video is the device's colouring too.
        assert_eq!(runs[2].text, "ERROR");
        assert_eq!(runs[2].style.foreground, palette.background);
        assert_eq!(runs[2].style.background, Some(palette.foreground));
    }

    #[test]
    fn the_line_being_written_waits_for_its_colour() {
        let mut terminal = terminal(10, 4);
        let palette = TerminalPalette::DARK;
        terminal.feed(b"ERROR one\r\n", "1");
        // Seventeen characters wrap onto a second row; the cursor is on it.
        terminal.feed(b"ERROR 1234 ERROR", "2");
        let content = terminal.render(&palette);
        assert_eq!(run(&content, 0, "ERROR").style.foreground, Role::Error.style(palette.theme).color);
        for row in 1..3 {
            assert_eq!(content.rows[row].runs.len(), 1, "row {row} is one plain run");
            assert_eq!(content.rows[row].runs[0].style.foreground, palette.foreground);
        }
        // Ending the line finishes it, and the colour arrives.
        terminal.feed(b"\r\n", "3");
        let content = terminal.render(&palette);
        assert_eq!(run(&content, 1, "ERROR").style.foreground, Role::Error.style(palette.theme).color);
        assert_eq!(run(&content, 2, "ERROR").style.foreground, Role::Error.style(palette.theme).color);
    }

    #[test]
    fn a_split_character_is_whole_on_arrival() {
        let mut terminal = terminal(20, 2);
        terminal.feed(b"T=\xe6\xb8", "1");
        terminal.feed(b"\xa9\r\n", "2");
        assert_eq!(rows(&terminal)[0], stamped("1", "T=温"));
    }

    #[test]
    fn clearing_starts_over() {
        let mut terminal = terminal(20, 2);
        terminal.feed(b"old\r\n", "1");
        terminal.clear();
        terminal.feed(b"new", "2");
        assert_eq!(rows(&terminal), vec![stamped("2", "new"), plain("")]);
    }

    /// `CSI Ps SP q` picks the cursor's shape and whether it blinks.
    #[test]
    fn a_program_can_ask_for_a_steady_cursor() {
        let mut terminal = terminal(20, 2);
        assert!(terminal.cursor_blinks());
        terminal.feed(b"\x1b[2 q", "1");
        assert!(!terminal.cursor_blinks());
        terminal.feed(b"\x1b[1 q", "2");
        assert!(terminal.cursor_blinks());
        terminal.feed(b"\x1b[0 q", "3");
        assert!(terminal.cursor_blinks());
    }

    fn cell(line: i32, column: usize, right_half: bool) -> GridCell {
        GridCell {
            line,
            column,
            right_half,
        }
    }

    /// A drag picks up the text under it — a wrapped line whole, without
    /// the wrap's break — and the screen says which cells it covers.
    #[test]
    fn a_selection_reads_back_as_text() {
        let mut terminal = terminal(6, 4);
        terminal.feed(b"hello world\r\nnext\r\n", "1");
        assert!(!terminal.has_selection());
        terminal.begin_selection(cell(0, 1, false), SelectionKind::Cells);
        assert!(!terminal.has_selection(), "a press alone selects nothing");
        terminal.extend_selection(cell(1, 2, true));
        assert!(terminal.has_selection());
        assert_eq!(terminal.selection_text().as_deref(), Some("ello wor"));
        let content = terminal.render(&TerminalPalette::DARK);
        let selected: Vec<_> = content.rows.iter().map(|row| row.selected.clone()).collect();
        assert_eq!(selected, vec![Some(1..6), Some(0..3), None, None]);
        assert!(terminal.clear_selection());
        assert!(!terminal.has_selection());
        assert!(!terminal.clear_selection());
    }

    /// The copy that brings the times along puts each line's stamp at its
    /// head, in a column the text lines up after, and leaves out the lines
    /// the mask is holding back, as the plain copy does.
    #[test]
    fn a_selection_can_be_copied_with_its_times() {
        let mut terminal = terminal(20, 4);
        terminal.feed(b"first\r\n", "14:32:40.018");
        terminal.feed(b"second\r\n", "14:32:41.700");
        terminal.select_all();
        assert_eq!(
            terminal.selection_text_stamped(|_| true).as_deref(),
            Some("14:32:40.018  first\n14:32:41.700  second\n")
        );
        assert_eq!(
            terminal.selection_text_stamped(|number| number == 2).as_deref(),
            Some("14:32:41.700  second\n")
        );
        terminal.clear_selection();
        assert_eq!(terminal.selection_text_stamped(|_| true), None);
    }

    /// Two clicks take the word, three the line, and select all the log;
    /// a selection past the grid's edges is clamped onto it.
    #[test]
    fn words_lines_and_the_whole_log_can_be_selected() {
        let mut terminal = terminal(6, 4);
        terminal.feed(b"hello world\r\nnext\r\n", "1");
        terminal.begin_selection(cell(1, 1, false), SelectionKind::Words);
        assert_eq!(terminal.selection_text().as_deref(), Some("world"));
        terminal.begin_selection(cell(2, 3, false), SelectionKind::Lines);
        assert_eq!(terminal.selection_text().as_deref(), Some("next\n"));
        terminal.select_all();
        assert_eq!(terminal.selection_text().as_deref(), Some("hello world\nnext\n"));
        terminal.begin_selection(cell(-10, 0, false), SelectionKind::Cells);
        terminal.extend_selection(cell(10, 99, true));
        assert_eq!(
            terminal.selection_text().map(|text| text.trim_end().to_string()).as_deref(),
            Some("hello world\nnext")
        );
    }

    /// The selection stays on its text as lines push it up the screen and
    /// into the scrollback, and goes when the log is cleared or its
    /// columns change.
    #[test]
    fn a_selection_rides_with_its_text() {
        let mut terminal = terminal(10, 2);
        terminal.feed(b"first\r\n", "1");
        terminal.begin_selection(cell(0, 0, false), SelectionKind::Words);
        assert_eq!(terminal.selection_text().as_deref(), Some("first"));
        terminal.feed(b"second\r\nthird\r\n", "2");
        assert_eq!(terminal.selection_text().as_deref(), Some("first"));
        terminal.resize(10, 3);
        assert_eq!(terminal.selection_text().as_deref(), Some("first"));
        terminal.resize(12, 3);
        assert!(!terminal.has_selection(), "a reflow lets it go");
        terminal.begin_selection(cell(0, 0, false), SelectionKind::Lines);
        assert!(terminal.has_selection());
        terminal.clear();
        assert!(!terminal.has_selection());
    }

    fn bytes(keys: &str, mode: TermMode) -> Option<Vec<u8>> {
        key_bytes(&Keystroke::parse(keys).unwrap(), mode)
    }

    #[test]
    fn keys_send_what_a_terminal_sends() {
        let normal = TermMode::empty();
        assert_eq!(bytes("enter", normal), Some(b"\r".to_vec()));
        assert_eq!(bytes("backspace", normal), Some(b"\x7f".to_vec()));
        assert_eq!(bytes("tab", normal), Some(b"\t".to_vec()));
        assert_eq!(bytes("shift-tab", normal), Some(b"\x1b[Z".to_vec()));
        assert_eq!(bytes("escape", normal), Some(b"\x1b".to_vec()));
        assert_eq!(bytes("up", normal), Some(b"\x1b[A".to_vec()));
        assert_eq!(bytes("up", TermMode::APP_CURSOR), Some(b"\x1bOA".to_vec()));
        assert_eq!(bytes("shift-up", normal), Some(b"\x1b[1;2A".to_vec()));
        assert_eq!(bytes("ctrl-left", TermMode::APP_CURSOR), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(bytes("delete", normal), Some(b"\x1b[3~".to_vec()));
        assert_eq!(bytes("f1", normal), Some(b"\x1bOP".to_vec()));
        assert_eq!(bytes("f5", normal), Some(b"\x1b[15~".to_vec()));
        assert_eq!(bytes("ctrl-c", normal), Some(vec![3]));
        assert_eq!(bytes("ctrl-z", normal), Some(vec![26]));
        assert_eq!(bytes("ctrl-[", normal), Some(vec![0x1b]));
        assert_eq!(bytes("cmd-n", normal), None);
        assert_eq!(bytes("a", normal), None);
    }

    #[test]
    fn a_paste_types_its_lines_the_way_return_does() {
        let normal = TermMode::empty();
        assert_eq!(paste_bytes("ls -la", normal), b"ls -la".to_vec());
        assert_eq!(paste_bytes("one\r\ntwo\n", normal), b"one\rtwo\r".to_vec());
        assert_eq!(
            paste_bytes("ls", TermMode::BRACKETED_PASTE),
            b"\x1b[200~ls\x1b[201~".to_vec()
        );
    }
}
