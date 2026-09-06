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

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point},
    term::{Config, Term, TermMode, cell::Flags, color::Colors},
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
    /// Lines kept above the screen to scroll back through.
    scrollback: usize,
    /// The number of the oldest line still stamped, counting from one at
    /// the last clear: what the scrollback let go is added here, so the
    /// lines that remain keep the numbers they had.
    first_number: usize,
    /// Goes up whenever the grid changes — bytes in, a resize, a clear —
    /// so a search over it can tell whether its answer is still current.
    revision: u64,
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
            scrollback,
            first_number: 1,
            revision: 0,
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

    /// Feeds what the port read, stamped with the time it arrived. Returns
    /// what the terminal wants written back, usually nothing. A view that
    /// was following the output goes on following it.
    pub(crate) fn receive(&mut self, bytes: &[u8], time: &str) -> Vec<u8> {
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
        std::mem::take(&mut *self.outbox.0.borrow_mut())
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
        let mut bytes = Vec::with_capacity(text.len() + 16);
        if self.term.grid().cursor.point.column.0 != 0 {
            bytes.extend_from_slice(b"\r\n");
        }
        bytes.extend_from_slice(b"\x1b[90m");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[0m\r\n");
        let _ = self.receive(&bytes, time);
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
        *self = Self::with_size(self.size, self.scrollback);
        self.revision = revision;
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

    /// Where a pattern occurs in the log, oldest first, as cells. Each
    /// logical line is read whole — the rows the terminal wrapped it over
    /// joined back — and a match that straddles the wrap comes back as one
    /// match of two spans. A match is named by its line's number and how
    /// far along the line it starts, which is the same match through a
    /// scroll, a reflow or new lines pushing the rows up.
    pub(crate) fn find(&self, matcher: &OutputFilter) -> Vec<FindMatch> {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let last = self.size.lines as i32 - 1;
        let is_spacer = |flags: Flags| {
            flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        };
        let mut matches = Vec::new();
        let mut text = String::new();
        let mut number = self.first_line_number();
        let mut line = -history;
        while line <= last {
            // The logical line's text, and the row after its last.
            text.clear();
            let mut end = line;
            loop {
                let row = &grid[Line(end)];
                text.extend(row[..].iter().filter(|cell| !is_spacer(cell.flags)).map(|cell| cell.c));
                let wrapped = row
                    .last()
                    .is_some_and(|cell| cell.flags.contains(Flags::WRAPLINE));
                end += 1;
                if !wrapped || end > last {
                    break;
                }
            }
            text.truncate(text.trim_end().len());
            let ranges = matcher.find_ranges(&text);
            if !ranges.is_empty() {
                // One cell per character of the text, in order, for the
                // few lines that hold a match.
                let cells: Vec<FindSpan> = (line..end)
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
                            line: number,
                            offset: start,
                            spans,
                        });
                    }
                }
            }
            number += 1;
            line = end;
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

    /// The text of each row on screen, for the filter to count.
    pub(crate) fn visible_texts(&self) -> Vec<String> {
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        let mut rows = vec![String::new(); self.size.lines];
        for cell in content.display_iter {
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            if let Some(row) = rows.get_mut((cell.point.line.0 + offset) as usize) {
                row.push(cell.c);
            }
        }
        for row in &mut rows {
            row.truncate(row.trim_end().len());
        }
        rows
    }

    /// The screen as it is to be drawn, colours resolved against the theme.
    /// With `highlight`, text the device left in the default colour takes
    /// the colour of what it says once its line is finished; the line the
    /// cursor is still on stays plain, and what the device coloured itself
    /// is kept either way.
    pub(crate) fn render(&self, palette: &TerminalPalette, highlight: bool) -> RenderContent {
        let grid = self.term.grid();
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        let lines = self.size.lines;
        let history = grid.history_size() as i32;

        let is_continuation = |line: Line| self.is_continuation(line.0);
        // Rows that begin a line, numbered from the top of the scrollback:
        // one pass up to the cursor, noting the number as it passes the
        // first row on screen.
        let first_visible = -offset;
        let cursor_line = grid.cursor.point.line.0;
        let (mut hard_at_first, mut hard_at_cursor) = (0, 0);
        let mut count = 0usize;
        for at in -history..=cursor_line.max(first_visible) {
            if !is_continuation(Line(at)) {
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
        // the cursor's line is still empty. A hard row's stamp is found by
        // counting back from it, and its number is the stamp's place in
        // the log.
        let anchor = hard_at_cursor as i64 - i64::from(self.unstamped);
        let stamp_index = |hard: usize| -> Option<usize> {
            let back = usize::try_from(anchor - hard as i64).ok()?;
            self.stamps.len().checked_sub(back + 1)
        };

        let mut hard = hard_at_first;
        let mut rows: Vec<RenderRow> = (0..lines)
            .map(|index| {
                let line = Line(index as i32 - offset);
                let continuation = is_continuation(line);
                if index > 0 && !continuation {
                    hard += 1;
                }
                let stamped = if continuation { None } else { stamp_index(hard) };
                RenderRow {
                    stamp: stamped.map(|index| self.stamps[index].clone()),
                    number: stamped.map(|index| self.first_number + index),
                    runs: Vec::new(),
                    text: String::new(),
                    columns: Vec::new(),
                }
            })
            .collect();

        // Every cell first, unstyled: a row's text has to be whole before
        // the parts of it worth a colour can be found.
        struct Pending {
            column: usize,
            width: usize,
            text: String,
            flags: Flags,
            fg: Color,
            bg: Color,
        }
        let mut pending: Vec<Vec<Pending>> = (0..lines).map(|_| Vec::new()).collect();
        for cell in content.display_iter {
            let flags = cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let index = (cell.point.line.0 + offset) as usize;
            let (Some(row), Some(cells)) = (rows.get_mut(index), pending.get_mut(index)) else {
                continue;
            };
            let mut text = String::new();
            if flags.contains(Flags::HIDDEN) {
                text.push(' ');
            } else {
                text.push(cell.c);
                if let Some(marks) = cell.zerowidth() {
                    text.extend(marks);
                }
            }
            row.text.push(cell.c);
            row.columns.push(cell.point.column.0);
            cells.push(Pending {
                column: cell.point.column.0,
                width: if flags.contains(Flags::WIDE_CHAR) {
                    2
                } else {
                    1
                },
                text,
                flags,
                fg: cell.fg,
                bg: cell.bg,
            });
        }

        let highlighter = highlight.then(Highlighter::shared);
        let plain = (
            Color::Named(NamedColor::Foreground),
            Color::Named(NamedColor::Background),
        );
        // The line the cursor is on is still being written — by the device,
        // or by whoever is typing at its prompt — and stays in plain ink
        // until it is done: colours shifting under the caret as each
        // character lands are a distraction, not a reading. The whole
        // logical line, wrapped rows included, waits together.
        let mut unfinished = cursor_line..cursor_line + 1;
        while is_continuation(Line(unfinished.start)) {
            unfinished.start -= 1;
        }
        while unfinished.end < lines as i32 && is_continuation(Line(unfinished.end)) {
            unfinished.end += 1;
        }
        for (index, (row, cells)) in rows.iter_mut().zip(pending).enumerate() {
            // One character of the row's text per cell, so a role found at
            // a position in the text belongs to the cell at that position.
            let roles = highlighter
                .filter(|_| !unfinished.contains(&(index as i32 - offset)))
                .map(|highlighter| highlighter.roles(&row.text));
            for (index, cell) in cells.into_iter().enumerate() {
                let flags = cell.flags;
                let (mut fg, mut bg) = (cell.fg, cell.bg);
                if flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let background = resolve(bg, content.colors, palette);
                let mut style = RunStyle {
                    foreground: resolve(fg, content.colors, palette),
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
                // Only ink the device left plain takes a role's colour: what
                // it coloured itself, it meant.
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
                match row.runs.last_mut() {
                    Some(run) if run.column + run.width == cell.column && run.style == style => {
                        run.text.push_str(&cell.text);
                        run.width += cell.width;
                    }
                    _ => row.runs.push(RenderRun {
                        column: cell.column,
                        width: cell.width,
                        text: cell.text,
                        style,
                    }),
                }
            }
        }
        for row in &mut rows {
            row.text.truncate(row.text.trim_end().len());
            row.columns.truncate(row.text.chars().count());
            // Blank stretches with nothing to show are not worth shaping.
            row.runs.retain(|run| {
                run.style.background.is_some()
                    || run.style.underline
                    || run.style.strikeout
                    || !run.text.trim().is_empty()
            });
        }

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

        RenderContent {
            rows,
            cursor,
            offset,
        }
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
    /// How far the view is scrolled back: the grid line drawn on the
    /// first row is `-offset`.
    pub(crate) offset: i32,
}

pub(crate) struct RenderRow {
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
        CaretShape, DEFAULT_SCROLLBACK_LINES, FindMatch, FindSpan, GridSize, MIN_NUMBER_DIGITS,
        RenderContent, RenderRun, Terminal, key_bytes,
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
            .render(&TerminalPalette::DARK, false)
            .rows
            .iter()
            .map(|row| row.number)
            .collect()
    }

    fn rows(terminal: &Terminal) -> Vec<(Option<String>, String)> {
        terminal
            .render(&TerminalPalette::DARK, true)
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
        terminal.receive(b"temp=25\r\n", "10:00:00.000");
        terminal.receive(b"hum", "10:00:01.000");
        terminal.receive(b"=61\r\n", "10:00:02.000");
        terminal.receive(b"\r\n", "10:00:03.000");
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
        terminal.receive(b"OK", "10:00:04.000");
        assert_eq!(rows(&terminal)[3], stamped("10:00:04.000", "OK"));
    }

    /// Lines are numbered from one, a wrapped line once, and the line the
    /// cursor waits on not until it holds something.
    #[test]
    fn lines_are_numbered_from_one() {
        let mut terminal = terminal(5, 5);
        terminal.receive(b"one\r\ntwo\r\nabcdefgh\r\n", "1");
        assert_eq!(
            numbers(&terminal),
            vec![Some(1), Some(2), Some(3), None, None],
            "the wrapped third line takes one number, the empty fourth none yet"
        );
        terminal.receive(b"x", "2");
        assert_eq!(numbers(&terminal)[4], Some(4));
    }

    /// Once the scrollback is full the oldest lines go, but the ones that
    /// stay keep their numbers, and the gutter is sized to the highest.
    #[test]
    fn numbers_keep_counting_past_the_scrollback() {
        let mut terminal = Terminal::with_size(GridSize { columns: 8, lines: 2 }, 3);
        for index in 1..=250 {
            terminal.receive(format!("l{index}\r\n").as_bytes(), "1");
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
        terminal.receive(b"one\r\ntwo\r\nthree\r\n", "1");
        terminal.receive(b"\x1b[3J", "2");
        terminal.receive(b"four", "3");
        assert_eq!(numbers(&terminal), vec![Some(2), Some(3), Some(4)]);
        terminal.receive(b"\x1b[2J", "4");
        terminal.receive(b"\r\nfive", "5");
        assert_eq!(
            numbers(&terminal),
            vec![None, Some(1), Some(2)],
            "the cleared row above is no line; the line kept is the first again"
        );
        terminal.clear();
        terminal.receive(b"six", "6");
        assert_eq!(numbers(&terminal)[0], Some(1));
    }

    /// The scrollback can be made smaller or larger as the log runs.
    #[test]
    fn the_scrollback_can_be_resized() {
        let mut terminal = Terminal::with_size(GridSize { columns: 8, lines: 2 }, 100);
        for index in 1..=50 {
            terminal.receive(format!("l{index}\r\n").as_bytes(), "1");
        }
        terminal.set_scrollback(10);
        assert_eq!(terminal.scrollback(), 10);
        assert_eq!(terminal.term.grid().history_size(), 10);
        terminal.set_scrollback(1_000);
        for index in 51..=500 {
            terminal.receive(format!("l{index}\r\n").as_bytes(), "1");
        }
        assert_eq!(terminal.term.grid().history_size(), 460);
        assert_eq!(numbers(&terminal), vec![Some(500), None]);
    }

    /// The find reads wrapped lines whole, answers in cells, and minds
    /// case only when told to.
    #[test]
    fn find_locates_matches_across_a_wrap() {
        let mut terminal = terminal(5, 4);
        terminal.receive(b"abcdefgh\r\nDEF\r\n", "1");
        let mut matcher = OutputFilter::literal();
        matcher.set_pattern("def");
        let found = terminal.find(&matcher);
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
        assert_eq!(terminal.find(&matcher).len(), 1);
        matcher.set_pattern("");
        assert!(terminal.find(&matcher).is_empty());
    }

    /// A wide character is one match cell of two columns, and a line in
    /// the scrollback is found at its negative row.
    #[test]
    fn find_counts_wide_characters_and_reaches_the_scrollback() {
        let mut terminal = terminal(10, 2);
        terminal.receive("温度=25\r\nnext\r\nlast\r\n".as_bytes(), "1");
        let mut matcher = OutputFilter::literal();
        matcher.set_pattern("度=2");
        assert_eq!(
            terminal.find(&matcher),
            vec![FindMatch {
                line: 1,
                offset: 1,
                spans: vec![FindSpan { line: -2, column: 2, width: 4 }],
            }]
        );
        terminal.scroll_to_line(-2);
        assert!(!terminal.is_at_bottom());
        assert_eq!(terminal.render(&TerminalPalette::DARK, false).rows[0].text, "温度=25");
    }

    /// A match answers to the same name after the rows under it have
    /// moved — new lines pushing them up, or a reflow — so the find keeps
    /// its place by the text and not by the row.
    #[test]
    fn a_match_keeps_its_name_as_the_rows_move() {
        let mut terminal = Terminal::with_size(GridSize { columns: 40, lines: 3 }, 100);
        terminal.receive(b"lost 1\r\nfine\r\nlost 2\r\n", "1");
        let mut matcher = OutputFilter::literal();
        matcher.set_pattern("lost");
        let before: Vec<_> = terminal.find(&matcher).iter().map(FindMatch::key).collect();
        assert_eq!(before, vec![(1, 0), (3, 0)]);
        terminal.receive(b"lost 3\r\nmore\r\n", "2");
        terminal.resize(8, 6);
        let after: Vec<_> = terminal.find(&matcher).iter().map(FindMatch::key).collect();
        assert_eq!(after, vec![(1, 0), (3, 0), (4, 0)]);
    }

    #[test]
    fn a_wrapped_line_carries_its_time_once() {
        let mut terminal = terminal(10, 4);
        terminal.receive(b"abcdefghijklmno\r\n", "1");
        terminal.receive(b"next\r\n", "2");
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
        terminal.receive(b"login: ", "1");
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
            terminal.receive(format!("line {index}\r\n").as_bytes(), &index.to_string());
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
        terminal.receive(b"one\r\ntwo\r\n", "1");
        terminal.receive(b"\x1b[H\x1b[2J$ ", "2");
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
        terminal.receive(b"ls\r\nREADME\r\n", "3");
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
            terminal.receive(format!("line {index}\r\n").as_bytes(), &index.to_string());
        }
        terminal.scroll(2);
        assert!(!terminal.is_at_bottom());
        terminal.receive(b"\x1b[H\x1b[2J", "7");
        assert!(terminal.is_at_bottom());
        assert_eq!(rows(&terminal), vec![stamped("7", ""), plain(""), plain("")]);
        terminal.scroll(1);
        assert_eq!(rows(&terminal), vec![stamped("7", ""), plain(""), plain("")]);
        for index in 8..=11 {
            terminal.receive(format!("line {index}\r\n").as_bytes(), &index.to_string());
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
            terminal.receive(format!("line {index}\r\n").as_bytes(), &index.to_string());
        }
        terminal.receive(b"$ ", "7");
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
        terminal.receive(b"one\r\ntwo\r\n", "1");
        terminal.receive(b"\x1b[H\x1b[J$ ", "2");
        assert_eq!(rows(&terminal), vec![stamped("2", "$"), plain(""), plain("")]);
        terminal.receive(b"abc\x1b[2D\x1b[J", "3");
        assert_eq!(rows(&terminal)[0], stamped("2", "$ a"));
        terminal.receive(b"\r\nnext", "4");
        assert_eq!(
            rows(&terminal),
            vec![stamped("2", "$ a"), stamped("4", "next"), plain("")]
        );
        // Below the cursor, from the top of a screen that has scrolled, is
        // the whole screen again.
        terminal.receive(b"\r\nmore\r\n", "5");
        terminal.receive(b"\x1b[H\x1b[0J# ", "6");
        assert_eq!(rows(&terminal), vec![stamped("6", "#"), plain(""), plain("")]);
        terminal.scroll(5);
        assert!(terminal.is_at_bottom());
    }

    #[test]
    fn an_erase_cut_short_by_a_read_is_still_seen() {
        let mut terminal = terminal(20, 3);
        terminal.receive(b"one\r\ntwo\r\n\x1b[H\x1b", "1");
        terminal.receive(b"[", "2");
        assert_eq!(rows(&terminal)[0], stamped("1", "one"));
        terminal.receive(b"J$ ", "3");
        assert_eq!(rows(&terminal), vec![stamped("1", "$"), plain(""), plain("")]);
        // What is held back is only ever an erase in the making.
        terminal.receive(b"\x1b[1", "4");
        terminal.receive(b"mbold\x1b[0", "4");
        terminal.receive(b"m\r\n", "4");
        let content = terminal.render(&TerminalPalette::DARK, false);
        assert_eq!(content.rows[0].text, "$ bold");
        assert!(content.rows[0].runs.iter().any(|run| run.text == "bold" && run.style.bold));
    }

    /// `ESC [ 3 J` on its own empties the scrollback and leaves the screen,
    /// and the times of the lines on screen stay with them.
    #[test]
    fn the_device_can_empty_the_scrollback_alone() {
        let mut terminal = terminal(20, 2);
        terminal.receive(b"one\r\ntwo\r\n", "1");
        terminal.receive(b"three", "2");
        terminal.scroll(1);
        assert_eq!(rows(&terminal), vec![stamped("1", "one"), stamped("1", "two")]);
        terminal.receive(b"\x1b[3J", "3");
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
        terminal.receive(b"one\r\ntwo\r\nthree\r\n", "1");
        terminal.receive(b"\x1b[H\x1b[2J", "2");
        terminal.receive(b"\x1b[3J$ ", "2");
        assert_eq!(rows(&terminal), vec![stamped("2", "$"), plain("")]);
        terminal.scroll(1);
        assert_eq!(rows(&terminal), vec![stamped("2", "$"), plain("")]);
    }

    /// A full-screen program clears the alternate screen as it draws; the
    /// log under it is untouched, and comes back when the program ends.
    #[test]
    fn the_alternate_screen_clears_itself_and_not_the_log() {
        let mut terminal = terminal(20, 2);
        terminal.receive(b"one\r\ntwo\r\n", "1");
        terminal.receive(b"\x1b[?1049h\x1b[H\x1b[2Jmenu", "2");
        assert_eq!(rows(&terminal)[0].1, "menu");
        terminal.receive(b"\x1b[2J\x1b[3J", "3");
        terminal.receive(b"\x1b[?1049l", "4");
        assert_eq!(rows(&terminal), vec![stamped("1", "two"), stamped("2", "")]);
        terminal.scroll(1);
        assert_eq!(rows(&terminal), vec![stamped("1", "one"), stamped("1", "two")]);
    }

    #[test]
    fn a_short_log_starts_at_the_top() {
        let mut terminal = terminal(20, 4);
        terminal.receive(b"one\r\n", "1");
        assert_eq!(
            rows(&terminal),
            vec![stamped("1", "one"), plain(""), plain(""), plain("")]
        );
        assert_eq!(terminal.cursor_position(), Some((1, 0)));
        // A clear with nothing behind it leaves the prompt at the top too.
        terminal.receive(b"\x1b[H\x1b[2J$ ", "2");
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
        terminal.receive(b"\x1b[1;32mroot@board\x1b[0m:~# lss\x08 -la\r\n", "1");
        terminal.receive(b"10%\r50%\r100%\r\n", "2");
        terminal.receive("温度 25°C".as_bytes(), "3");
        let content = terminal.render(&TerminalPalette::DARK, false);
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
        terminal.receive(b"E (1234) wifi: lost 192.168.1.20 after 350ms\r\n", "1");
        terminal.receive(b"GET /index.html\r\n", "2");
        let palette = TerminalPalette::DARK;
        let content = terminal.render(&palette, true);
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
        let light = terminal.render(&TerminalPalette::LIGHT, true);
        assert_eq!(
            run(&light, 0, "E").style.foreground,
            Role::Error.style(TerminalPalette::LIGHT.theme).color
        );
        assert_ne!(run(&light, 0, "E").style.foreground, level.style.foreground);
    }

    #[test]
    fn a_devices_own_colours_are_kept() {
        let mut terminal = terminal(40, 2);
        terminal.receive(b"\x1b[32mERROR\x1b[0m ERROR \x1b[7mERROR\x1b[0m\r\n", "1");
        let palette = TerminalPalette::DARK;
        let content = terminal.render(&palette, true);
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
        terminal.receive(b"ERROR one\r\n", "1");
        // Seventeen characters wrap onto a second row; the cursor is on it.
        terminal.receive(b"ERROR 1234 ERROR", "2");
        let content = terminal.render(&palette, true);
        assert_eq!(run(&content, 0, "ERROR").style.foreground, Role::Error.style(palette.theme).color);
        for row in 1..3 {
            assert_eq!(content.rows[row].runs.len(), 1, "row {row} is one plain run");
            assert_eq!(content.rows[row].runs[0].style.foreground, palette.foreground);
        }
        // Ending the line finishes it, and the colour arrives.
        terminal.receive(b"\r\n", "3");
        let content = terminal.render(&palette, true);
        assert_eq!(run(&content, 1, "ERROR").style.foreground, Role::Error.style(palette.theme).color);
        assert_eq!(run(&content, 2, "ERROR").style.foreground, Role::Error.style(palette.theme).color);
    }

    #[test]
    fn colouring_can_be_switched_off() {
        let mut terminal = terminal(40, 2);
        terminal.receive(b"ERROR at 10:00:00\r\n", "1");
        let palette = TerminalPalette::DARK;
        let content = terminal.render(&palette, false);
        assert_eq!(content.rows[0].runs.len(), 1);
        assert_eq!(content.rows[0].runs[0].style.foreground, palette.foreground);
        assert!(terminal.render(&palette, true).rows[0].runs.len() > 1);
    }

    #[test]
    fn a_split_character_is_whole_on_arrival() {
        let mut terminal = terminal(20, 2);
        terminal.receive(b"T=\xe6\xb8", "1");
        terminal.receive(b"\xa9\r\n", "2");
        assert_eq!(rows(&terminal)[0], stamped("1", "T=温"));
    }

    #[test]
    fn clearing_starts_over() {
        let mut terminal = terminal(20, 2);
        terminal.receive(b"old\r\n", "1");
        terminal.clear();
        terminal.receive(b"new", "2");
        assert_eq!(rows(&terminal), vec![stamped("2", "new"), plain("")]);
    }

    /// `CSI Ps SP q` picks the cursor's shape and whether it blinks.
    #[test]
    fn a_program_can_ask_for_a_steady_cursor() {
        let mut terminal = terminal(20, 2);
        assert!(terminal.cursor_blinks());
        terminal.receive(b"\x1b[2 q", "1");
        assert!(!terminal.cursor_blinks());
        terminal.receive(b"\x1b[1 q", "2");
        assert!(terminal.cursor_blinks());
        terminal.receive(b"\x1b[0 q", "3");
        assert!(terminal.cursor_blinks());
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
}
