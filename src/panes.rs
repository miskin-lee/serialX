//! The workbench split: the panes the open sessions are shown in.
//!
//! A pane is a strip of tabs with one log under it — an editor group, as
//! VS Code has them. A workspace opens with one pane holding every session,
//! which is the workbench as it has always been; splitting hands the
//! session in front to a pane of its own beside or below the first, so two
//! boards can be watched at once. The panes divide the centre column and
//! nothing else: the title bar, the side panel and the composer go on
//! speaking to *the* session in front, which is now the front tab of the
//! pane last worked in.
//!
//! There is one axis for the whole split rather than a tree of them: panes
//! stand in a row or in a column, and splitting the other way turns the
//! row into a column. A grid of four logs on a laptop screen would leave
//! each one a gutter and a dozen columns of text; a row of two, or three
//! at the most, is what the log's timestamps and line numbers leave room
//! for, and [`MAX_PANES`] says so.
//!
//! A session belongs to exactly one pane: its grid is fitted to the log
//! that draws it, so the same session cannot stand in two panes at two
//! sizes at once. That is what makes a split cheap — the tabs are moved,
//! never copied — and it is why closing the last tab of a pane closes the
//! pane with it rather than leaving an empty one behind.

use std::rc::Rc;

use gpui_kit::base::{ResizeHandleContext, ResizeHandleRenderer};
use gpui_kit::component::{
    h_flex,
    resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::SerialWorkspace;
use crate::theme::{Typography, WorkbenchPalette, tint};

/// How many panes the centre column takes. A log spends its first hundred
/// and fifty pixels on timestamps and line numbers, so a third pane is the
/// last one with room for a line of output beside them.
pub(crate) const MAX_PANES: usize = 3;
/// The narrowest a pane is dragged to, and the shortest. Below this the
/// gutter is the pane.
const PANE_MIN_WIDTH: f32 = 320.;
const PANE_MIN_HEIGHT: f32 = 140.;
/// Width of the line a pane's edge lights up to under the pointer, as the
/// side panel's seam does.
const DIVIDER_LIT_WIDTH: f32 = 3.;
/// The ring of its own hue that a tab wears while it is being dragged.
const TAG_RING_DRAGGED: f32 = 0.55;
/// How strongly a pane lights up while a tab is held over it.
pub(crate) const DROP_TARGET_WASH: f32 = 0.1;

/// Which way the panes stand. `Across` is a row, as an editor splits to
/// the right; `Down` is a column, as one splits below.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SplitAxis {
    Across,
    Down,
}

impl SplitAxis {
    fn is_across(self) -> bool {
        self == Self::Across
    }
}

/// The tabs a pane holds and which of them is in front — the bookkeeping
/// of one strip, with no window in it, so what a close or a drag does to
/// the row can be said in a test.
#[derive(Default)]
pub(crate) struct TabStrip {
    /// The tabs, in strip order, by tab id.
    pub(crate) tabs: Vec<usize>,
    /// The tab in front, by id. Meaningless while `tabs` is empty.
    pub(crate) active: usize,
}

impl TabStrip {
    pub(crate) fn active_tab(&self) -> Option<usize> {
        self.tabs.contains(&self.active).then_some(self.active)
    }

    fn index_of(&self, tab_id: usize) -> Option<usize> {
        self.tabs.iter().position(|id| *id == tab_id)
    }

    /// Puts a tab in the strip — at `at`, or at its end — and in front.
    fn insert(&mut self, tab_id: usize, at: Option<usize>) {
        let at = at.unwrap_or(self.tabs.len()).min(self.tabs.len());
        self.tabs.insert(at, tab_id);
        self.active = tab_id;
    }

    /// Takes a tab out of the strip. What was beside it comes to the front
    /// in its place: the tab on its right, as a browser does, or the one on
    /// its left when it was last in the row.
    fn remove(&mut self, tab_id: usize) {
        let Some(index) = self.index_of(tab_id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active == tab_id {
            let next = index.min(self.tabs.len().saturating_sub(1));
            self.active = self.tabs.get(next).copied().unwrap_or(0);
        }
    }

    /// Moves a tab that is already here to `at`, counted in the strip as
    /// it stands *before* the move — where the pointer was let go — and
    /// brings it to the front. Dropping a tab on itself, or in the gap it
    /// already fills, leaves the row alone.
    fn reorder(&mut self, tab_id: usize, at: Option<usize>) {
        let Some(current) = self.index_of(tab_id) else {
            return;
        };
        let at = at.unwrap_or(self.tabs.len());
        // Past its own place, the tab it is dropped before has already
        // shifted left by the one being lifted out.
        let at = if at > current { at - 1 } else { at };
        self.active = tab_id;
        if at == current {
            return;
        }
        self.tabs.remove(current);
        self.tabs.insert(at.min(self.tabs.len()), tab_id);
    }
}

/// One pane: a strip of tabs, and the log's focus while the pane holds it.
pub(crate) struct Pane {
    pub(crate) id: usize,
    strip: TabStrip,
    /// The log's focus handle. Each pane's log is its own place to type,
    /// so a click in one pane leaves the other pane's cursor behind.
    pub(crate) focus: FocusHandle,
}

impl Pane {
    pub(crate) fn new(id: usize, cx: &mut Context<SerialWorkspace>) -> Self {
        Self {
            id,
            strip: TabStrip::default(),
            focus: cx.focus_handle(),
        }
    }
}

impl std::ops::Deref for Pane {
    type Target = TabStrip;

    fn deref(&self) -> &Self::Target {
        &self.strip
    }
}

impl std::ops::DerefMut for Pane {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.strip
    }
}

/// A tab under the pointer, on its way to another place in a strip or to
/// another pane. Carries what the tab looks like as well as which tab it
/// is, so the plate that follows the pointer is the tab itself rather than
/// a label standing in for it.
#[derive(Clone)]
pub(crate) struct DraggedTab {
    pub(crate) tab: usize,
    pub(crate) name: SharedString,
    pub(crate) hue: u32,
    pub(crate) status: u32,
    pub(crate) ink: u32,
    /// The plate the tab is drawn on while it travels. Opaque, unlike the
    /// wash a tab wears in its strip: this one floats over the log.
    pub(crate) plate: u32,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .h(px(26.))
            .px_2p5()
            .gap_2()
            .items_center()
            .rounded(px(7.))
            .bg(rgb(self.plate))
            .border_1()
            .border_color(tint(self.hue, TAG_RING_DRAGGED))
            .child(SerialWorkspace::status_dot(6., self.status))
            .child(
                div()
                    .text_token(crate::theme::LABEL)
                    .text_color(rgb(self.ink))
                    .child(self.name.clone()),
            )
    }
}

impl SerialWorkspace {
    /// The pane worked in last: what the title bar, the composer and every
    /// action that says "the session in front" mean by it.
    pub(crate) fn active_pane(&self) -> &Pane {
        &self.panes[self.active_pane.min(self.panes.len() - 1)]
    }

    fn active_pane_mut(&mut self) -> &mut Pane {
        let index = self.active_pane.min(self.panes.len() - 1);
        &mut self.panes[index]
    }

    /// The tab in front of the pane in front.
    pub(crate) fn active_tab_id(&self) -> Option<usize> {
        self.active_pane().active_tab()
    }

    /// Where the pane in front stands among the panes, clamped: the field
    /// is an index, and a pane it named may since have been folded away.
    pub(crate) fn active_pane_index(&self) -> usize {
        self.active_pane.min(self.panes.len() - 1)
    }

    /// The tabs on screen: the front tab of every pane. What has to be
    /// kept up to date each frame, rather than only the one in front.
    pub(crate) fn visible_tabs(&self) -> Vec<usize> {
        self.panes
            .iter()
            .filter_map(|pane| pane.active_tab())
            .collect()
    }

    /// What a pane shows, as the render pass reads it.
    pub(crate) fn pane_snapshot(&self, pane: usize) -> Option<crate::SerialTabSnapshot> {
        let tab = self.panes.get(pane)?.active_tab()?;
        self.tab(tab).map(crate::SerialTabSnapshot::from)
    }

    /// Where the tab in front stands in its own strip, for the arrows that
    /// walk it.
    pub(crate) fn active_strip_position(&self) -> Option<usize> {
        let pane = self.active_pane();
        pane.index_of(pane.active_tab()?)
    }

    /// Which pane shows a tab.
    pub(crate) fn pane_of(&self, tab_id: usize) -> Option<usize> {
        self.panes
            .iter()
            .position(|pane| pane.tabs.contains(&tab_id))
    }

    /// Shows a tab just made in the pane in front, at the end of its strip.
    pub(crate) fn show_new_tab(&mut self, tab_id: usize) {
        self.active_pane_mut().insert(tab_id, None);
    }

    /// Brings a tab that is already open to the front, pane and all: what
    /// a saved session opened twice, or a card double-clicked, lands on.
    pub(crate) fn reveal_tab(&mut self, tab_id: usize, window: &mut Window, cx: &mut App) {
        let Some(pane) = self.pane_of(tab_id) else {
            return;
        };
        self.panes[pane].active = tab_id;
        self.focus_pane(pane, window, cx);
    }

    /// Takes a closed tab out of its pane, and the pane with it when that
    /// was its last tab — a split is worth keeping only while there is
    /// something in it. The last pane stays, empty, and shows the splash.
    pub(crate) fn forget_tab(&mut self, tab_id: usize, cx: &mut Context<Self>) {
        let Some(pane) = self.pane_of(tab_id) else {
            return;
        };
        self.panes[pane].remove(tab_id);
        if self.panes[pane].tabs.is_empty() && self.panes.len() > 1 {
            self.panes.remove(pane);
            self.rebuild_pane_layout(cx);
        }
        self.active_pane = self.active_pane.min(self.panes.len() - 1);
    }

    /// Whether the session in front can be handed to a pane of its own on
    /// a given side: there has to be another tab left behind, and room.
    pub(crate) fn can_split_toward(&self, axis: SplitAxis) -> bool {
        self.active_tab_id()
            .is_some_and(|tab| self.can_split_tab_toward(tab, axis))
    }

    /// Whether a tab can be handed a pane of its own on a given side.
    /// Three things have to hold: the strip has a session to spare, the
    /// window is not already divided as far as it goes, and what the panes
    /// would each be left with is still a log rather than a gutter.
    pub(crate) fn can_split_tab_toward(&self, tab_id: usize, axis: SplitAxis) -> bool {
        self.panes.len() < MAX_PANES
            && self
                .pane_of(tab_id)
                .is_some_and(|pane| self.panes[pane].tabs.len() > 1)
            && self.room_for_another_pane(axis)
    }

    /// Whether the centre column, divided once more, would still leave
    /// every pane something to read.
    ///
    /// Splitting re-divides the whole column evenly, so the question is
    /// what *each* pane would get, not what the one being split has. The
    /// column's extent along the new axis is the panes' sizes added up
    /// when they already stand that way, and one pane's size when they
    /// stand the other way — they all span the column then. Nothing is
    /// measured before the first frame, and a split then is let through.
    fn room_for_another_pane(&self, axis: SplitAxis) -> bool {
        let sizes = self.visible_tabs().into_iter().filter_map(|tab| {
            self.tab(tab).map(|tab| match axis {
                SplitAxis::Across => tab.metrics.width,
                SplitAxis::Down => tab.metrics.height,
            })
        });
        let extent = if self.panes.len() > 1 && self.split_axis == axis {
            sizes.sum::<f32>()
        } else {
            sizes.fold(0., f32::max)
        };
        let minimum = match axis {
            SplitAxis::Across => PANE_MIN_WIDTH,
            SplitAxis::Down => PANE_MIN_HEIGHT,
        };
        extent <= 0. || extent / (self.panes.len() + 1) as f32 >= minimum
    }

    /// Whether the split can be folded back up.
    pub(crate) fn can_join(&self) -> bool {
        self.panes.len() > 1
    }

    /// Hands the session in front to a new pane beside or below its own,
    /// and works there from then on. The tab is moved, not copied: a
    /// session is one log, fitted to the one pane that draws it.
    pub(crate) fn split_active_tab(
        &mut self,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.active_tab_id() {
            self.split_tab(tab_id, axis, window, cx);
        }
    }

    /// The same for a named tab, wherever it stands: the pane it is in
    /// keeps the rest of its strip.
    pub(crate) fn split_tab(
        &mut self,
        tab_id: usize,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_split_tab_toward(tab_id, axis) {
            return;
        }
        let Some(index) = self.pane_of(tab_id) else {
            return;
        };
        self.panes[index].remove(tab_id);

        let id = self.next_pane_id;
        self.next_pane_id += 1;
        let mut pane = Pane::new(id, cx);
        pane.insert(tab_id, None);
        self.panes.insert(index + 1, pane);
        self.split_axis = axis;
        self.rebuild_pane_layout(cx);
        self.focus_pane(index + 1, window, cx);
        cx.notify();
    }

    /// Folds the pane in front back into the one before it — or, for the
    /// first pane, into the one after — keeping its tabs. Nothing is
    /// closed: the sessions carry on in the pane that takes them.
    pub(crate) fn join_active_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_join() {
            return;
        }
        let index = self.active_pane.min(self.panes.len() - 1);
        let into = if index == 0 { 1 } else { index - 1 };
        let folded = self.panes.remove(index);
        let into = if into > index { into - 1 } else { into };
        let front = folded.active_tab();
        self.panes[into].tabs.extend(folded.strip.tabs);
        if let Some(tab_id) = front {
            self.panes[into].active = tab_id;
        }
        self.rebuild_pane_layout(cx);
        self.focus_pane(into, window, cx);
        cx.notify();
    }

    /// Moves a tab into another pane, at `at` in its strip, and works
    /// there: what a tab dragged from one strip to another does. A pane
    /// left empty by the move goes with it.
    pub(crate) fn move_tab_to_pane(
        &mut self,
        tab_id: usize,
        pane: usize,
        at: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(from), true) = (self.pane_of(tab_id), pane < self.panes.len()) else {
            return;
        };
        if from == pane {
            // Same strip: a reorder, and the tab keeps its place in front.
            self.panes[pane].reorder(tab_id, at);
            self.focus_pane(pane, window, cx);
            cx.notify();
            return;
        }

        self.panes[from].remove(tab_id);
        self.panes[pane].insert(tab_id, at);
        let mut target = pane;
        if self.panes[from].tabs.is_empty() {
            self.panes.remove(from);
            if from < target {
                target -= 1;
            }
            self.rebuild_pane_layout(cx);
        }
        self.focus_pane(target, window, cx);
        cx.notify();
    }

    /// Whether the log of the pane in front holds the cursor, with the
    /// window itself in front: what the blink and the drawn cursor ask.
    pub(crate) fn terminal_in_focus(&self, window: &Window) -> bool {
        self.active_pane().focus.is_focused(window) && window.is_window_active()
    }

    /// Works in a pane from now on, and puts the cursor in its log.
    pub(crate) fn focus_pane(&mut self, pane: usize, window: &mut Window, cx: &mut App) {
        if pane >= self.panes.len() {
            return;
        }
        self.active_pane = pane;
        let focus = self.panes[pane].focus.clone();
        window.focus(&focus, cx);
    }

    /// Works in a pane from now on without moving the cursor there: what a
    /// press on one of its tabs does, so a strip can be shuffled without
    /// the keyboard leaving the box it was in.
    pub(crate) fn select_pane(&mut self, pane: usize) {
        if pane < self.panes.len() {
            self.active_pane = pane;
        }
    }

    /// The sizes a split was dragged to belong to the panes that were
    /// there: a pane added or taken away starts the division over, evenly.
    fn rebuild_pane_layout(&mut self, cx: &mut Context<Self>) {
        self.pane_layout = cx.new(|_| ResizableState::default());
    }

    /// The centre column: one pane, or the panes divided along the split's
    /// axis with a draggable seam between them.
    pub(crate) fn render_panes(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        if self.panes.len() == 1 {
            return self.render_pane(0, window, cx);
        }

        let across = self.split_axis.is_across();
        let panes = (0..self.panes.len())
            .map(|index| self.render_pane(index, window, cx))
            .collect::<Vec<_>>();
        let minimum = if across {
            px(PANE_MIN_WIDTH)
        } else {
            px(PANE_MIN_HEIGHT)
        };

        let group = if across {
            h_resizable("workbench-panes")
        } else {
            v_resizable("workbench-panes")
        };
        group
            .with_state(&self.pane_layout)
            .with_handle_appearance(pane_divider(palette, across))
            .children(panes.into_iter().map(|pane| {
                resizable_panel()
                    .size_range(minimum..px(f32::MAX))
                    .child(pane)
            }))
            .into_any_element()
    }

    /// One pane: its strip of tabs, and under it the log of the tab in
    /// front — or, in a workspace with nothing open, the splash.
    fn render_pane(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let strip = self.render_tab_strip(index, cx);
        let content = match self.pane_snapshot(index) {
            Some(tab) => self.render_pane_log(index, tab, window, cx),
            None => self.render_empty_state(window, cx),
        };
        // The seam is the pane's own edge, as the side panel's is: the
        // handle over it paints nothing until the pointer is on it.
        let across = self.split_axis.is_across();
        let seam = index > 0;

        v_flex()
            .id(("pane", self.panes[index].id))
            .flex_1()
            .min_w_0()
            .min_h_0()
            .size_full()
            .bg(rgb(palette.editor))
            .overflow_hidden()
            .when(seam && across, |pane| {
                pane.border_l_1().border_color(rgb(palette.border))
            })
            .when(seam && !across, |pane| {
                pane.border_t_1().border_color(rgb(palette.border))
            })
            // A press anywhere in a pane — its strip, its log, the air
            // beside its tabs — is what says which session the title bar
            // and the composer are now speaking to.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.active_pane != index {
                        this.select_pane(index);
                        cx.notify();
                    }
                }),
            )
            .children(strip)
            .child(content)
            .into_any_element()
    }
}

/// What the seam between two panes paints: nothing at rest — the strip's
/// own border is the line — and an accent line while the pointer is on it
/// or dragging it, as the side panel's edge does.
fn pane_divider(palette: WorkbenchPalette, across: bool) -> ResizeHandleRenderer {
    Rc::new(
        move |handle: &ResizeHandleContext, _: &mut Window, _: &mut App| {
            let lit = rgb(palette.accent);
            let line = if across {
                div()
                    .h_full()
                    .w(px(DIVIDER_LIT_WIDTH))
                    .ml(px(-((DIVIDER_LIT_WIDTH - 1.) / 2.)))
            } else {
                div()
                    .w_full()
                    .h(px(DIVIDER_LIT_WIDTH))
                    .mt(px(-((DIVIDER_LIT_WIDTH - 1.) / 2.)))
            };
            Some(
                line.flex_none()
                    .when(handle.is_active(), |line| line.bg(lit))
                    .group_hover("handle", move |line| line.bg(lit))
                    .into_any_element(),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::TabStrip;

    fn strip(tabs: &[usize], active: usize) -> TabStrip {
        TabStrip {
            tabs: tabs.to_vec(),
            active,
        }
    }

    /// Closing the tab in front hands the front to the one on its right,
    /// the way a browser does, so the eye lands where the row goes on.
    #[test]
    fn closing_the_front_tab_moves_right() {
        let mut row = strip(&[1, 2, 3], 2);
        row.remove(2);
        assert_eq!(row.tabs, vec![1, 3]);
        assert_eq!(row.active_tab(), Some(3));
    }

    /// Unless it was the last one, when there is only the left.
    #[test]
    fn closing_the_last_tab_moves_left() {
        let mut row = strip(&[1, 2, 3], 3);
        row.remove(3);
        assert_eq!(row.active_tab(), Some(2));
    }

    #[test]
    fn closing_another_tab_leaves_the_front_alone() {
        let mut row = strip(&[1, 2, 3], 3);
        row.remove(1);
        assert_eq!(row.tabs, vec![2, 3]);
        assert_eq!(row.active_tab(), Some(3));
    }

    /// An emptied strip has no tab in front to name.
    #[test]
    fn the_last_tab_out_leaves_nothing_in_front() {
        let mut row = strip(&[1], 1);
        row.remove(1);
        assert!(row.tabs.is_empty());
        assert_eq!(row.active_tab(), None);
    }

    #[test]
    fn a_tab_arrives_where_it_was_dropped_and_in_front() {
        let mut row = strip(&[1, 2], 1);
        row.insert(9, Some(1));
        assert_eq!(row.tabs, vec![1, 9, 2]);
        assert_eq!(row.active_tab(), Some(9));

        row.insert(8, None);
        assert_eq!(row.tabs, vec![1, 9, 2, 8]);
    }

    /// Dragged rightwards, a tab lands before the one it was dropped on:
    /// the row it is counted in still holds the tab being lifted out.
    #[test]
    fn a_tab_dragged_along_its_own_strip_lands_where_it_was_let_go() {
        let mut row = strip(&[1, 2, 3], 1);
        row.reorder(1, Some(2));
        assert_eq!(row.tabs, vec![2, 1, 3]);
        assert_eq!(row.active_tab(), Some(1));

        let mut row = strip(&[1, 2, 3], 1);
        row.reorder(3, Some(0));
        assert_eq!(row.tabs, vec![3, 1, 2]);

        let mut row = strip(&[1, 2, 3], 1);
        row.reorder(1, None);
        assert_eq!(row.tabs, vec![2, 3, 1]);
    }

    #[test]
    fn a_tab_dropped_where_it_already_stands_leaves_the_row_alone() {
        let mut row = strip(&[1, 2, 3], 2);
        row.reorder(2, Some(1));
        assert_eq!(row.tabs, vec![1, 2, 3]);
        row.reorder(2, Some(2));
        assert_eq!(row.tabs, vec![1, 2, 3]);
    }
}
