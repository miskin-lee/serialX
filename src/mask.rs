//! The mask: the filter's other mode, where the lines that do not match
//! are not drawn at all.
//!
//! The title bar filter's first mode tints the lines that match where they
//! stand. This one takes them out of the log and shows them alone, the
//! way a log viewer's filter does: the lines that match, oldest first,
//! with their numbers and stamps in the gutter so the gaps between them
//! can be read, and the newest at the bottom. It is a view over the grid
//! rather than a copy of it — the terminal keeps every line, the
//! highlight mode has them all back, and the find, the selection and the
//! copy work on what is shown.
//!
//! The view is the rows of the lines that match, and it has a scroll
//! position of its own — how far up from the newest row it is — since the
//! terminal's own is over rows the mask is not showing. While it is
//! scrolled up, the row at the foot of the view is remembered by its
//! line, so rows arriving at the bottom do not move what is being read.
//!
//! Which lines match is asked again whenever the grid changes, and that
//! is every frame while a device streams. The walk over the grid is
//! cheap; the matching is not, over fifty thousand lines, so it is done
//! once per line for the lines in the scrollback, which alacritty never
//! writes to again, and their answers are kept by line number. Only the
//! lines on screen, the ones the cursor can still reach, are matched
//! afresh each time. The kept answers go when the pattern or a switch
//! changes, and when the log is cleared, since the numbering starts over
//! then.

use std::collections::VecDeque;

use crate::filter::OutputFilter;
use crate::serial::SerialTabState;
use crate::terminal::{LogicalLine, RenderContent, Terminal};
use crate::theme::TerminalPalette;

/// One row of the masked view: the row of the grid it draws, and which
/// row of which line it is, the name it keeps while the grid moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MaskRow {
    pub(crate) line: i32,
    number: i64,
    part: usize,
}

/// What the mask shows of a tab's log, and where in it the view is.
#[derive(Default)]
pub(crate) struct MaskState {
    /// Whether each line in the scrollback matched, by number from
    /// `cache_base` up: the lines that cannot change, answered once.
    cache: VecDeque<bool>,
    cache_base: i64,
    /// The grid revision and matcher generation the rows below are for.
    scanned: Option<(u64, u64)>,
    /// The terminal's epoch and the matcher's generation the cache was
    /// filled for: a clear starts the numbering over, and another matcher
    /// gives other answers, so either starts the cache over.
    cache_for: (u64, u64),
    /// The lines shown, oldest first.
    shown: Vec<LogicalLine>,
    /// The rows of the lines shown, top to bottom.
    rows: Vec<MaskRow>,
    /// How many lines the log holds, matching or not.
    total: usize,
    /// How far up from the newest row the view is, in rows: zero at the
    /// bottom, following the output.
    offset: usize,
    /// While scrolled up, the row at the foot of the view, so the view
    /// keeps its place as rows come and go.
    anchor: Option<(i64, usize)>,
}

impl MaskState {
    /// Brings the view up to the grid and the filter. Nothing is done
    /// while neither has changed, and nothing is kept while the mask is
    /// off.
    pub(crate) fn refresh(&mut self, terminal: &Terminal, filter: &OutputFilter) {
        if !filter.masking() {
            self.shown.clear();
            self.rows.clear();
            self.total = 0;
            self.scanned = None;
            return;
        }
        let key = (terminal.revision(), filter.generation());
        if self.scanned == Some(key) {
            return;
        }
        let cache_for = (terminal.epoch(), filter.generation());
        if self.cache_for != cache_for {
            self.cache.clear();
            self.cache_for = cache_for;
        }

        self.shown.clear();
        self.rows.clear();
        self.total = 0;
        let mut first = None;
        for line in terminal.logical_lines() {
            self.total += 1;
            first.get_or_insert(line.number);
            let matched = match self.cached(line.number) {
                Some(matched) if line.settled() => matched,
                _ => {
                    let matched = filter.matches(&terminal.line_text(&line));
                    if line.settled() {
                        self.remember(line.number, matched);
                    }
                    matched
                }
            };
            if matched {
                self.shown.push(line);
                self.rows
                    .extend(line.rows().enumerate().map(|(part, at)| MaskRow {
                        line: at,
                        number: line.number,
                        part,
                    }));
            }
        }
        // Answers for lines the scrollback has let go are not asked for
        // again.
        match first {
            Some(first) => {
                while self.cache_base < first && !self.cache.is_empty() {
                    self.cache.pop_front();
                    self.cache_base += 1;
                }
            }
            None => self.cache.clear(),
        }
        self.scanned = Some(key);
        self.keep_place(terminal.screen_lines());
    }

    fn cached(&self, number: i64) -> Option<bool> {
        let index = usize::try_from(number - self.cache_base).ok()?;
        self.cache.get(index).copied()
    }

    /// Keeps a settled line's answer. Lines settle in order, so each is
    /// the next after the last kept; one that is not — a line settled
    /// again after a reflow brought it back on screen — is already there.
    fn remember(&mut self, number: i64, matched: bool) {
        if self.cache.is_empty() {
            self.cache_base = number;
        }
        if number == self.cache_base + self.cache.len() as i64 {
            self.cache.push_back(matched);
        }
    }

    /// After a scan: the view scrolled up stays on the row it was
    /// anchored to, or as near as the rows left allow.
    fn keep_place(&mut self, lines: usize) {
        if self.offset == 0 {
            return;
        }
        let anchored = self.anchor.and_then(|key| {
            self.rows
                .binary_search_by_key(&key, |row| (row.number, row.part))
                .ok()
        });
        self.offset = match anchored {
            Some(index) => self.rows.len() - 1 - index,
            None => self.offset.min(self.max_offset(lines)),
        };
    }

    fn max_offset(&self, lines: usize) -> usize {
        self.rows.len().saturating_sub(lines)
    }

    /// Notes the row at the foot of the view, for the next scan to find
    /// again.
    fn drop_anchor(&mut self) {
        self.anchor = if self.offset == 0 {
            None
        } else {
            self.rows
                .len()
                .checked_sub(self.offset + 1)
                .and_then(|index| self.rows.get(index))
                .map(|row| (row.number, row.part))
        };
    }

    /// The rows on screen, top to bottom, on a screen of `lines` rows: the
    /// newest rows less the offset, or every row while there are fewer
    /// than the screen holds.
    pub(crate) fn visible(&self, lines: usize) -> &[MaskRow] {
        let end = self.rows.len().saturating_sub(self.offset);
        let start = end.saturating_sub(lines);
        &self.rows[start..end]
    }

    /// The row of the grid drawn on a row of the screen, or the last one
    /// drawn when the screen has rows to spare; nothing while nothing is
    /// shown.
    pub(crate) fn line_at(&self, row: usize, lines: usize) -> Option<i32> {
        let visible = self.visible(lines);
        visible
            .get(row)
            .or_else(|| visible.last())
            .map(|row| row.line)
    }

    /// Whether the line with a number is shown.
    pub(crate) fn is_shown(&self, number: i64) -> bool {
        self.shown
            .binary_search_by_key(&number, |line| line.number)
            .is_ok()
    }

    /// How many lines are shown, out of how many the log holds.
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.shown.len(), self.total)
    }

    /// Moves the view through the rows shown: positive is back in time,
    /// as the terminal's own scroll has it.
    pub(crate) fn scroll(&mut self, delta: i32, lines: usize) {
        let wanted = (self.offset as i64 + i64::from(delta)).max(0) as usize;
        self.offset = wanted.min(self.max_offset(lines));
        self.drop_anchor();
    }

    /// Brings a row of the grid to the middle of the view, as near as the
    /// ends of the rows shown allow. A row the mask is not showing leaves
    /// the view where it is.
    pub(crate) fn scroll_to_line(&mut self, line: i32, lines: usize) {
        let Ok(index) = self.rows.binary_search_by_key(&line, |row| row.line) else {
            return;
        };
        // The row lands where the terminal's own scroll puts one: half
        // the screen down from the top.
        let end = index + lines - lines / 2;
        self.offset = self
            .rows
            .len()
            .saturating_sub(end)
            .min(self.max_offset(lines));
        self.drop_anchor();
    }

    pub(crate) fn scroll_to_bottom(&mut self) {
        self.offset = 0;
        self.anchor = None;
    }

    pub(crate) fn is_at_bottom(&self) -> bool {
        self.offset == 0
    }
}

/// The tab's view of its log: the screen, or the mask over it, whichever
/// the filter says. Everything that scrolls, selects or reads the log
/// goes through here, so the two views answer the same questions.
impl SerialTabState {
    /// Whether the mask is holding lines back.
    pub(crate) fn masking(&self) -> bool {
        self.filter.masking()
    }

    /// Brings the mask up to the grid: called before anything reads it,
    /// and cheap when nothing has changed.
    pub(crate) fn refresh_mask(&mut self) {
        self.mask.refresh(&self.terminal, &self.filter);
    }

    /// The pattern or a switch changed: the lines the mask shows may not
    /// be the lines they were, and what the find found among them is
    /// looked for again.
    pub(crate) fn filter_changed(&mut self) {
        if self.masking() {
            self.find.forget();
        }
    }

    /// The rows to paint: the screen, or the rows of the lines the mask
    /// keeps, with no cursor — there is nowhere on a masked view to put
    /// one.
    pub(crate) fn view_content(&self, palette: &TerminalPalette) -> RenderContent {
        if !self.masking() {
            return self.terminal.render(palette, self.highlight);
        }
        let specs: Vec<(i32, Option<i64>)> = self
            .mask
            .visible(self.terminal.screen_lines())
            .iter()
            .map(|row| (row.line, (row.part == 0).then_some(row.number)))
            .collect();
        RenderContent {
            rows: self.terminal.render_rows(&specs, palette, self.highlight),
            cursor: None,
        }
    }

    /// Moves the view in front through its rows: positive is back in time.
    pub(crate) fn scroll_view(&mut self, lines: i32) {
        if self.masking() {
            self.mask.scroll(lines, self.terminal.screen_lines());
        } else {
            self.terminal.scroll(lines);
        }
    }

    /// Whether the view in front is following the newest output.
    pub(crate) fn view_at_bottom(&self) -> bool {
        if self.masking() {
            self.mask.is_at_bottom()
        } else {
            self.terminal.is_at_bottom()
        }
    }

    /// Both views to the newest output, so switching between them lands
    /// at the bottom of either.
    pub(crate) fn scroll_to_bottom(&mut self) {
        self.terminal.scroll_to_bottom();
        self.mask.scroll_to_bottom();
    }

    /// Brings a row of the grid to the middle of the view in front.
    pub(crate) fn reveal_line(&mut self, line: i32) {
        if self.masking() {
            self.mask.scroll_to_line(line, self.terminal.screen_lines());
        } else {
            self.terminal.scroll_to_line(line);
        }
    }

    /// The row of the grid drawn on a row of the screen, for a press to
    /// find its cell; nothing while the mask has nothing to show.
    pub(crate) fn grid_line_at(&self, row: usize) -> Option<i32> {
        if self.masking() {
            self.mask.line_at(row, self.terminal.screen_lines())
        } else {
            Some(row as i32 - self.terminal.display_offset() as i32)
        }
    }

    /// The selected text, as the view in front shows it: through the
    /// mask, only the lines shown.
    pub(crate) fn selection_text(&self) -> Option<String> {
        if self.masking() {
            self.terminal
                .selection_text_within(|number| self.mask.is_shown(number))
        } else {
            self.terminal.selection_text()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MaskState;
    use crate::filter::OutputFilter;
    use crate::terminal::Terminal;

    fn masked() -> OutputFilter {
        let mut filter = OutputFilter::default();
        filter.toggle_mode();
        filter.set_pattern("err");
        filter
    }

    fn lines(mask: &MaskState, screen: usize) -> Vec<i32> {
        mask.visible(screen).iter().map(|row| row.line).collect()
    }

    /// The mask shows the rows of the lines that match, a wrapped line
    /// whole, and counts the lines of the log.
    #[test]
    fn the_mask_keeps_the_lines_that_match() {
        let mut terminal = Terminal::new(100);
        terminal.resize(8, 4);
        terminal.receive(b"ok\r\nERROR one\r\nok\r\nERR\r\n", "1");
        let filter = masked();
        let mut mask = MaskState::default();
        mask.refresh(&terminal, &filter);
        // Rows: -2 "ok", -1 "ERROR on", 0 "e", 1 "ok", 2 "ERR"; the
        // cursor's empty row under them is not a line.
        assert_eq!(lines(&mask, 4), vec![-1, 0, 2]);
        assert_eq!(mask.counts(), (2, 4));
        assert!(mask.is_shown(2) && mask.is_shown(4));
        assert!(!mask.is_shown(1) && !mask.is_shown(3));
        assert_eq!(mask.line_at(0, 4), Some(-1));
        assert_eq!(mask.line_at(9, 4), Some(2), "past the rows, the last");
        // Off, the mask shows nothing of its own.
        let mut off = masked();
        off.toggle_mode();
        mask.refresh(&terminal, &off);
        assert_eq!(mask.counts(), (0, 0));
        assert_eq!(mask.line_at(0, 4), None);
    }

    /// Scrolled up, the view stays on its row as lines arrive, and comes
    /// back to the bottom on request; the scan keeps its answers for the
    /// scrollback and forgets them when the pattern changes.
    #[test]
    fn the_view_keeps_its_place_as_rows_arrive() {
        let mut terminal = Terminal::new(100);
        terminal.resize(20, 2);
        for index in 0..6 {
            terminal.receive(format!("err {index}\r\n").as_bytes(), "1");
        }
        let mut filter = masked();
        let mut mask = MaskState::default();
        mask.refresh(&terminal, &filter);
        assert_eq!(mask.counts(), (6, 6));
        // Rows -5 to 0 hold `err 0` to `err 5`; the cursor's row is empty.
        assert_eq!(lines(&mask, 2), vec![-1, 0]);
        mask.scroll(3, 2);
        assert!(!mask.is_at_bottom());
        assert_eq!(lines(&mask, 2), vec![-4, -3]);
        terminal.receive(b"err 6\r\nok\r\nerr 7\r\n", "2");
        mask.refresh(&terminal, &filter);
        assert_eq!(lines(&mask, 2), vec![-7, -6], "the same rows, moved up");
        assert_eq!(mask.cache.len(), 8, "the settled lines, answered once");
        mask.scroll(-100, 2);
        assert!(mask.is_at_bottom());
        assert_eq!(lines(&mask, 2), vec![-2, 0], "`ok` at -1 is held back");
        mask.scroll_to_line(-6, 2);
        assert_eq!(lines(&mask, 2), vec![-7, -6]);
        filter.toggle_regex();
        filter.set_pattern("err [0-3]");
        mask.refresh(&terminal, &filter);
        assert_eq!(mask.counts(), (4, 9));
        mask.scroll_to_bottom();
        terminal.clear();
        mask.refresh(&terminal, &filter);
        assert_eq!(mask.counts(), (0, 0));
        assert!(mask.cache.is_empty(), "a clear starts the numbering over");
    }
}
