//! Find in the log: the bar ⌘F opens over the terminal.
//!
//! The filter in the title bar and the find are two different questions of
//! the same pattern. The filter asks *whether* a line belongs on screen and
//! tints the ones that do; the find asks *where* the pattern occurs, in the
//! scrollback as much as on screen, and walks from one occurrence to the
//! next. So the find is not the filter with a shortcut: it is its own bar,
//! drawn the way VS Code draws its find widget — a pill in the terminal's
//! top-right corner holding the box, the `Aa` and `.*` switches, the count,
//! the two arrows and a close mark — with ⌘F to open it, `Enter` and
//! `Shift-Enter` (or ⌘G and ⌘⇧G) to step, and `Escape` to put it away.
//!
//! Every occurrence on screen is washed amber, the one in hand a stronger
//! amber ringed in the accent, and stepping scrolls the terminal so the
//! occurrence stands in the middle of the screen. The box starts literal —
//! `.` and `+` are usually the characters looked for — and the `.*` switch
//! turns regular expressions on; `Aa` makes it mind case.
//!
//! Where the pattern occurs is asked of the grid ([`Terminal::find`]) and
//! remembered against the grid's revision, so a frame with nothing new
//! costs nothing. While a device streams, the log changes every frame, and
//! a scan of fifty thousand lines a frame would be too much: a scan is
//! taken at most every [`SCAN_INTERVAL`] while data flows, and once more
//! after it stops, so the count settles. What is painted never waits on
//! the scan — the rows on screen are matched as they are drawn — only the
//! count and the place in hand do.

use std::time::{Duration, Instant};

use gpui_kit::component::{
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
};
use gpui_kit::*;
use smol::Timer;

use crate::app_menu::{CloseFind, FindNext, FindPrevious};
use crate::filter::OutputFilter;
use crate::terminal::{FindMatch, FindSpan, Terminal};
use crate::theme::{CAPTION, MICRO, Typography};
use crate::{SerialTabSnapshot, SerialWorkspace};

/// How often the log is scanned while data flows.
pub(crate) const SCAN_INTERVAL: Duration = Duration::from_millis(300);
/// The bar: its height, the box's, and where it floats over the terminal.
const FIND_BAR_HEIGHT: f32 = 38.;
const FIND_BOX_HEIGHT: f32 = 28.;
const FIND_BOX_WIDTH: f32 = 230.;
const FIND_BAR_INSET_TOP: f32 = 8.;
const FIND_BAR_INSET_RIGHT: f32 = 20.;
/// The arrows and the close mark, sized like the title bar's tab arrows.
const FIND_BUTTON: f32 = 24.;
/// Room for `1234 of 5678` without the bar shifting as the count changes.
const FIND_STATUS_WIDTH: f32 = 84.;
/// What the box says while it is empty.
const FIND_PLACEHOLDER: &str = "Find in log";

/// What a tab's find is up to: the pattern, where it was last found, and
/// which occurrence is in hand.
pub(crate) struct FindState {
    pub(crate) open: bool,
    pub(crate) matcher: OutputFilter,
    matches: Vec<FindMatch>,
    current: Option<usize>,
    /// The grid revision the matches were found in, if any.
    scanned: Option<u64>,
    last_scan: Option<Instant>,
    /// A place to scroll to that the last scan chose — the newest match of
    /// a fresh pattern — waiting for whoever holds the terminal.
    landing: Option<FindSpan>,
}

impl Default for FindState {
    fn default() -> Self {
        Self {
            open: false,
            matcher: OutputFilter::literal(),
            matches: Vec::new(),
            current: None,
            scanned: None,
            last_scan: None,
            landing: None,
        }
    }
}

impl FindState {
    pub(crate) fn total(&self) -> usize {
        self.matches.len()
    }

    pub(crate) fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub(crate) fn current_span(&self) -> Option<FindSpan> {
        self.current
            .and_then(|index| self.matches.get(index))
            .map(FindMatch::start)
    }

    /// The pattern or a switch changed: what was found no longer holds,
    /// and the next scan picks a place afresh.
    pub(crate) fn forget(&mut self) {
        self.matches.clear();
        self.current = None;
        self.scanned = None;
        self.landing = None;
    }

    /// Brings the matches up to the grid, unless a scan ran within the
    /// interval and `force` is off. Says whether a scan is still owed.
    pub(crate) fn refresh(&mut self, terminal: &Terminal, force: bool) -> bool {
        if !self.open || self.scanned == Some(terminal.revision()) {
            return false;
        }
        if !force
            && self
                .last_scan
                .is_some_and(|at| at.elapsed() < SCAN_INTERVAL)
        {
            return true;
        }
        self.rescan(terminal);
        false
    }

    fn rescan(&mut self, terminal: &Terminal) {
        let held = self
            .current
            .and_then(|index| self.matches.get(index))
            .map(FindMatch::key);
        let fresh = self.scanned.is_none();
        self.matches = terminal.find(&self.matcher);
        self.scanned = Some(terminal.revision());
        self.last_scan = Some(Instant::now());
        // The occurrence in hand stays in hand while it is still there —
        // it is known by its line and place, not by the row it was on — and
        // one the scrollback let go gives way to its neighbour. A fresh
        // pattern lands on its newest occurrence, the one nearest what the
        // device is saying now.
        self.current = match held {
            Some(key) => self
                .matches
                .iter()
                .position(|found| found.key() == key)
                .or_else(|| {
                    self.current
                        .map(|index| index.min(self.matches.len().saturating_sub(1)))
                }),
            None => self.matches.len().checked_sub(1),
        }
        .filter(|&index| index < self.matches.len());
        if fresh {
            self.landing = self.current_span();
        }
    }

    /// Where a fresh scan wants the terminal scrolled to, once.
    pub(crate) fn take_landing(&mut self) -> Option<FindSpan> {
        self.landing.take()
    }

    /// Moves to the next occurrence, or the previous, around the ends, and
    /// says where it is.
    pub(crate) fn step(&mut self, forward: bool) -> Option<FindSpan> {
        let total = self.matches.len();
        if total == 0 {
            return None;
        }
        self.current = Some(match (self.current, forward) {
            (Some(index), true) => (index + 1) % total,
            (Some(index), false) => (index + total - 1) % total,
            (None, true) => 0,
            (None, false) => total - 1,
        });
        self.current_span()
    }

    /// What the bar says beside the box: the place in hand out of how
    /// many, that there are none, or why the pattern will not compile.
    pub(crate) fn status(&self) -> Option<(String, bool)> {
        if let Some(error) = self.matcher.error() {
            return Some((error.to_owned(), true));
        }
        if self.matcher.pattern().is_empty() {
            return None;
        }
        Some(match (self.current_index(), self.total()) {
            (_, 0) => ("No results".to_owned(), false),
            (Some(index), total) => (format!("{} of {total}", index + 1), false),
            (None, total) => (format!("{total} found"), false),
        })
    }
}

/// What the render pass reads of a tab's find.
#[derive(Clone)]
pub(crate) struct FindView {
    pub(crate) open: bool,
    pub(crate) matcher: OutputFilter,
    /// The occurrence in hand, to paint apart from the rest.
    pub(crate) current: Option<FindSpan>,
    pub(crate) status: Option<(String, bool)>,
    pub(crate) input: Entity<InputState>,
}

impl SerialWorkspace {
    /// Opens the bar over the tab in front and puts the cursor in its box,
    /// with what the box already holds selected, so typing starts afresh
    /// and `Enter` steps on from where it was.
    pub(crate) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        tab.find.open = true;
        let input = tab.find_input.clone();
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.find_step(None, cx);
    }

    /// Puts the bar away and hands focus back to the terminal. What was
    /// typed stays in the box for the next time.
    pub(crate) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.find.open = false;
        }
        window.focus(&self.terminal_focus, cx);
        cx.notify();
    }

    /// Called as the find box changes, so the pattern compiles once per
    /// keystroke and the scan runs at once.
    pub(crate) fn set_find_pattern(
        &mut self,
        tab_id: usize,
        pattern: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.tab_index(tab_id)
            && self.tabs[index].find.matcher.set_pattern(pattern)
        {
            self.tabs[index].find.forget();
            self.find_step(None, cx);
        }
    }

    pub(crate) fn toggle_find_match_case(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.find.matcher.toggle_match_case();
            tab.find.forget();
            self.find_step(None, cx);
        }
    }

    pub(crate) fn toggle_find_regex(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.find.matcher.toggle_regex();
            tab.find.forget();
            self.find_step(None, cx);
        }
    }

    /// Brings the matches up to date now and, with a direction, steps to
    /// the next or the previous occurrence; either way the terminal
    /// scrolls to whatever the find lands on.
    pub(crate) fn find_step(&mut self, forward: Option<bool>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        tab.find.refresh(&tab.terminal, true);
        let landing = match forward {
            Some(forward) => tab.find.step(forward),
            None => tab.find.take_landing(),
        };
        if let Some(span) = landing {
            tab.terminal.scroll_to_line(span.line);
            tab.auto_scroll = tab.terminal.is_at_bottom();
        }
        cx.notify();
    }

    /// Keeps the find current as the log runs: called each frame, it scans
    /// when the grid has changed and the interval allows, and when it may
    /// not yet, arranges for a frame once it may.
    pub(crate) fn refresh_find(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let owed = tab.find.refresh(&tab.terminal, false);
        if let Some(span) = tab.find.take_landing() {
            tab.terminal.scroll_to_line(span.line);
            tab.auto_scroll = tab.terminal.is_at_bottom();
        }
        if owed && !self.find_refresh_pending {
            self.find_refresh_pending = true;
            cx.spawn(async move |this, cx| {
                Timer::after(SCAN_INTERVAL).await;
                let _ = this.update(cx, |this, cx| {
                    this.find_refresh_pending = false;
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The bar: the box with its two switches, the count, the arrows and
    /// the close mark, floating in the terminal's top-right corner.
    pub(crate) fn render_find_bar(
        &mut self,
        tab: &SerialTabSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let find = &tab.find;
        let tab_id = tab.id;
        let error = find.status.as_ref().is_some_and(|(_, error)| *error);

        let switches = h_flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(
                Self::filter_switch(
                    ("find-match-case", tab_id),
                    "Aa",
                    find.matcher.match_case(),
                    "Match case",
                    palette,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_find_match_case(cx))),
            )
            .child(
                Self::filter_switch(
                    ("find-regex", tab_id),
                    ".*",
                    find.matcher.use_regex(),
                    "Use regular expression",
                    palette,
                    cx,
                )
                .ui_mono_font()
                .on_click(cx.listener(|this, _, _, cx| this.toggle_find_regex(cx))),
            );

        let input = Styled::h(
            Input::new(&find.input)
                .small()
                .w(px(FIND_BOX_WIDTH))
                .text_token(CAPTION)
                .bg(rgb(palette.input))
                .border_color(rgb(if error {
                    palette.danger
                } else {
                    palette.input_border
                }))
                .rounded(px(FIND_BOX_HEIGHT / 2.))
                .px_2p5()
                .focus_bordered(!error)
                .prefix(
                    Icon::new(IconName::Search)
                        .size(px(13.))
                        .text_color(rgb(palette.muted)),
                )
                .suffix(switches),
            px(FIND_BOX_HEIGHT),
        );

        let status = div()
            .flex_none()
            .w(px(FIND_STATUS_WIDTH))
            .truncate()
            .text_right()
            .text_token(MICRO)
            .text_color(rgb(if error { palette.danger } else { palette.muted }))
            .children(find.status.as_ref().map(|(text, _)| text.clone()));

        let arrow = |id: &'static str, glyph: IconName| {
            Button::new(id)
                .ghost()
                .with_size(px(FIND_BUTTON))
                .tab_stop(false)
                .icon(glyph)
        };

        h_flex()
            .id("find-bar")
            .key_context("FindBar")
            .on_action(cx.listener(|this, _: &CloseFind, window, cx| this.close_find(window, cx)))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.find_step(Some(true), cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.find_step(Some(false), cx)))
            .absolute()
            .top(px(FIND_BAR_INSET_TOP))
            .right(px(FIND_BAR_INSET_RIGHT))
            .h(px(FIND_BAR_HEIGHT))
            .px(px(5.))
            .gap_1()
            .items_center()
            .rounded(px(FIND_BAR_HEIGHT / 2.))
            .bg(rgb(palette.card))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            .child(input)
            .child(status)
            .child(
                arrow("find-previous", IconName::ChevronUp)
                    .tooltip_with_action("Previous match", &FindPrevious, None)
                    .on_click(cx.listener(|this, _, _, cx| this.find_step(Some(false), cx))),
            )
            .child(
                arrow("find-next", IconName::ChevronDown)
                    .tooltip_with_action("Next match", &FindNext, None)
                    .on_click(cx.listener(|this, _, _, cx| this.find_step(Some(true), cx))),
            )
            .child(
                arrow("find-close", IconName::Close)
                    .tooltip_with_action("Close", &CloseFind, Some("FindBar"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
            )
            .into_any_element()
    }

    /// The find box of a new tab, wired so a change scans and `Enter`
    /// steps: forward, or back with `Shift`.
    pub(crate) fn build_find_input(
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InputState>, Subscription) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(FIND_PLACEHOLDER));
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &gpui_kit::component::input::InputEvent, _, cx| {
                use gpui_kit::component::input::InputEvent;
                match event {
                    InputEvent::Change => {
                        let pattern = input.read(cx).value().to_string();
                        this.set_find_pattern(id, &pattern, cx);
                    }
                    InputEvent::PressEnter { shift, .. } => this.find_step(Some(!shift), cx),
                    _ => {}
                }
            },
        );
        (input, subscription)
    }
}

#[cfg(test)]
mod tests {
    use super::FindState;
    use crate::terminal::Terminal;

    fn log() -> Terminal {
        let mut terminal = Terminal::new(100);
        terminal.receive(b"ok\r\nERROR one\r\nok\r\nERROR two\r\n", "1");
        terminal
    }

    /// A fresh pattern lands on its newest occurrence; stepping goes round
    /// the ends; the count says where it is.
    #[test]
    fn a_find_lands_on_the_newest_and_steps_around() {
        let terminal = log();
        let mut find = FindState {
            open: true,
            ..FindState::default()
        };
        assert_eq!(find.status(), None, "nothing to say without a pattern");
        find.matcher.set_pattern("error");
        assert!(
            !find.refresh(&terminal, false),
            "a first scan is never owed"
        );
        assert_eq!(find.total(), 2);
        assert_eq!(find.current_index(), Some(1));
        assert_eq!(find.take_landing().map(|span| span.line), Some(3));
        assert_eq!(find.take_landing(), None, "a landing is taken once");
        assert_eq!(find.status(), Some(("2 of 2".to_owned(), false)));
        assert_eq!(find.step(true).map(|span| span.line), Some(1));
        assert_eq!(find.step(false).map(|span| span.line), Some(3));
        assert_eq!(find.step(false).map(|span| span.line), Some(1));
    }

    /// The place in hand stays through a rescan, and a switch starts over.
    #[test]
    fn a_rescan_keeps_the_place_and_a_switch_forgets_it() {
        let mut terminal = log();
        let mut find = FindState {
            open: true,
            ..FindState::default()
        };
        find.matcher.set_pattern("error");
        find.refresh(&terminal, true);
        find.step(true);
        assert_eq!(find.current_index(), Some(0));
        terminal.receive(b"ERROR three\r\n", "2");
        terminal.resize(12, 3);
        find.refresh(&terminal, true);
        assert_eq!(find.total(), 3);
        assert_eq!(find.current_index(), Some(0), "still on the first");
        assert_eq!(
            find.current_span().map(|span| span.line),
            Some(-2),
            "which has moved up into the scrollback"
        );
        find.matcher.toggle_match_case();
        find.forget();
        assert_eq!(find.status(), Some(("No results".to_owned(), false)));
        find.refresh(&terminal, true);
        assert_eq!(find.total(), 0, "the log is upper case");
        find.matcher.set_pattern("ERROR");
        find.forget();
        find.refresh(&terminal, true);
        assert_eq!(find.total(), 3);
        find.matcher.set_pattern("error(");
        find.matcher.toggle_regex();
        assert_eq!(find.status(), Some(("Unclosed group".to_owned(), true)));
    }

    /// A closed find never scans, and a scan taken is not owed again until
    /// the grid moves on.
    #[test]
    fn a_closed_find_is_idle() {
        let mut terminal = log();
        let mut find = FindState::default();
        find.matcher.set_pattern("ok");
        assert!(!find.refresh(&terminal, true));
        assert_eq!(find.total(), 0);
        find.open = true;
        find.refresh(&terminal, true);
        assert_eq!(find.total(), 2);
        assert!(!find.refresh(&terminal, false), "nothing changed");
        terminal.receive(b"ok\r\n", "3");
        assert!(
            find.refresh(&terminal, false),
            "changed within the interval: owed"
        );
        assert!(!find.refresh(&terminal, true));
        assert_eq!(find.total(), 3);
    }
}
