//! The centre column: everything between the title bar and the window's
//! bottom edge.
//!
//! A pane of the workbench is two stacked bands — the tab strip and the
//! terminal log. The strip has a fixed height so the log is the only thing
//! that grows. Unsplit, that pane is the whole column; split, the column
//! holds two or three of them side by side or one above the other, each
//! with its own strip and its own log ([`crate::panes`] divides them).
//! What acts on the session in front lives elsewhere: connect beside the
//! filter in the title bar, and the composer at the foot of the side panel.

use gpui_kit::component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    kbd::Kbd,
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app_icon::application_icon_image;
use crate::app_menu::{
    CopyTerminalSelection, JoinSplit, NewSerialTab, PasteIntoTerminal, SelectAllInTerminal,
    SendBackTab, SendTab, SplitDown, SplitRight, TERMINAL_CONTEXT,
};
use crate::icons::Glyph;
use crate::filter::{FilterMode, OutputFilter};
use crate::find::FindView;
use crate::terminal::{CaretShape, RenderContent};
use crate::presets::{DEFAULT_TERMINAL_FONT_SIZE, usable_font_size};
use crate::theme::{
    BODY, CAPTION, LABEL, MONO_SMALL, TerminalPalette, Typography, WORDMARK, WorkbenchPalette,
    fonts, tint,
};
use crate::panes::{DROP_TARGET_WASH, DraggedTab, SplitAxis};
use crate::{SerialTabSnapshot, SerialTabState, SerialWorkspace};

/// Height of the tab strip above the terminal: well under the title bar's,
/// so the band that only says *which* session reads as the slimmer of the
/// two and gives its rows back to the log.
const TAB_STRIP_HEIGHT: f32 = 28.;
/// Height of a tab, and of every control that shares the strip with it.
const TAB_HEIGHT: f32 = 22.;
/// The close mark inside a tab.
const TAB_CLOSE: f32 = 16.;
/// The widest a tab grows before its name truncates.
const TAB_MAX_WIDTH: f32 = 260.;
/// The narrowest a tab shrinks to when the strip is full.
const TAB_MIN_WIDTH: f32 = 120.;
/// How strongly a tag's hue washes the plate of the tab being worked in, and
/// its ring: a filled plate under a bright outline, so the session the title
/// bar is speaking to is picked out of a full strip at a glance.
const TAG_PLATE_ACTIVE: f32 = 0.34;
const TAG_RING_ACTIVE: f32 = 0.9;
/// The front tab of a pane that is *not* being worked in: still plated and
/// ringed, so a split says which session each half holds, but a step back
/// from the one in hand.
const TAG_PLATE_BEHIND: f32 = 0.12;
const TAG_RING_BEHIND: f32 = 0.22;
/// The wash on a tab that is not in front, at rest and under the pointer:
/// faint enough to sit flat on the strip, strong enough to keep saying which
/// tag the session wears.
const TAG_PLATE_REST: f32 = 0.06;
const TAG_PLATE_HOVER: f32 = 0.16;
/// Width of the timestamp gutter, wide enough for `14:32:40.018`.
const TIME_GUTTER: f32 = 82.;
/// Side padding of a log row, and the gap between its gutters and its text.
const ROW_INSET: f32 = 16.;
const ROW_GAP: f32 = 12.;
/// How strongly the find washes an occurrence on screen, and the one in
/// hand: amber, as VS Code marks its finds, translucent so the text reads
/// through.
const FIND_WASH: f32 = 0.28;
const FIND_WASH_CURRENT: f32 = 0.55;
/// How tall a line stands against the type it holds: the 18px lines the log
/// was always set on, over the size it is set in out of the box. Kept as the
/// ratio so a size picked from the list brings its own leading with it.
const TERMINAL_LEADING: f32 = 18. / DEFAULT_TERMINAL_FONT_SIZE;
/// The cursor when it is drawn as a bar or an underline.
const CARET_THICKNESS: f32 = 2.;
/// Breathing room between the tab strip and the first line.
const TERMINAL_TOP_INSET: f32 = 8.;
/// The scrollbar down the log's right edge: the width of the track, which
/// lies in the margin the rows already leave at their end, and the width
/// of the thumb in it at rest and once the pointer is on the track.
const SCROLLBAR_WIDTH: f32 = 12.;
const SCROLLBAR_THUMB: f32 = 6.;
const SCROLLBAR_THUMB_WIDE: f32 = 10.;
/// The shortest the thumb is drawn, however long the log grows, so there
/// is always something to take hold of.
const SCROLLBAR_THUMB_MIN: f32 = 28.;
/// How strongly the thumb inks: at rest, with the pointer on the track,
/// and while it is being dragged.
const THUMB_REST: f32 = 0.2;
const THUMB_HOVER: f32 = 0.34;
const THUMB_HELD: f32 = 0.48;
/// What the log says while the filter's mask keeps every line back.
const MASK_EMPTY_HINT: &str = "No line matches the filter";

/// Where a tab stands and what can be done with it there: its pane and
/// its place in that pane's strip, whether it is the tab in front and
/// whether that pane is the one being worked in, and — asked of the pane
/// once, before its tabs are drawn — whether this strip has a session to
/// spare for a pane of its own on each side, and whether there is a split
/// to fold back.
#[derive(Clone, Copy)]
struct TabPlace {
    pane: usize,
    index: usize,
    active: bool,
    in_front: bool,
    across: bool,
    down: bool,
    can_join: bool,
}

/// How the terminal's cells map to pixels, measured each frame from the
/// mono font. Kept on the workspace so the input method's candidate window
/// can be put under the cursor, and so a press on the log can be told
/// which cell it landed on.
#[derive(Clone, Copy, Default)]
pub(crate) struct TerminalMetrics {
    pub(crate) cell_width: f32,
    pub(crate) line_height: f32,
    /// Where the cells start, from the terminal's left edge.
    pub(crate) text_left: f32,
    /// The terminal's top-left corner, in the window.
    pub(crate) origin_x: f32,
    pub(crate) origin_y: f32,
    /// The grid's size, as last fitted to the terminal.
    pub(crate) columns: usize,
    pub(crate) lines: usize,
    /// How wide the log stands, which is its pane's width.
    pub(crate) width: f32,
    /// How tall the log stands, which is the scrollbar's track.
    pub(crate) height: f32,
}

/// The scrollbar as it is to be drawn and dragged: where its track stands
/// in the window, where the thumb sits in the track, and how many rows the
/// thumb's travel is worth. Measured once per frame and read by both the
/// element and the pointer, so a press lands where the thumb was drawn.
#[derive(Clone, Copy)]
pub(crate) struct ScrollbarGeometry {
    pub(crate) track_top: f32,
    pub(crate) track_height: f32,
    pub(crate) thumb_top: f32,
    pub(crate) thumb_height: f32,
    /// The rows above the view when the thumb is at the foot of its
    /// travel: the whole log less what is on screen.
    pub(crate) extent: usize,
}

impl ScrollbarGeometry {
    /// How far the thumb can travel, which is what the rows are spread
    /// over; nothing when the thumb fills the track.
    fn travel(&self) -> f32 {
        (self.track_height - self.thumb_height).max(0.)
    }

    /// The rows that would stand above the view with the thumb's top at
    /// `top`, clamped to the ends of the log.
    pub(crate) fn rows_above(&self, top: f32) -> usize {
        let travel = self.travel();
        if travel <= 0. {
            return 0;
        }
        ((top / travel).clamp(0., 1.) * self.extent as f32).round() as usize
    }

    /// Whether a point down the track is on the thumb.
    pub(crate) fn holds(&self, y: f32) -> bool {
        (self.thumb_top..self.thumb_top + self.thumb_height).contains(&y)
    }
}

impl SerialWorkspace {
    /// The type the log is set in, as the settings have it.
    pub(crate) fn terminal_type(&self) -> TerminalType {
        TerminalType::new(self.presets.settings.terminal_font_size)
    }

    /// A small filled dot in the colour of the current connection state.
    pub(crate) fn status_dot(size: f32, color: u32) -> impl IntoElement {
        div()
            .flex_none()
            .size(px(size))
            .rounded_full()
            .bg(rgb(color))
    }

    /// The band above a pane's log: one tab per session it holds, and
    /// nothing else. A new session is opened from the File menu, its
    /// shortcut, or the empty state's own button; connecting is done beside
    /// the filter in the title bar, and pausing, clearing and the log's
    /// switches live in the menus with their shortcuts, so the strip carries
    /// only the sessions themselves.
    ///
    /// Every pane has one, so a split reads as two workbenches side by
    /// side rather than as one strip over two logs; the strip of the pane
    /// not being worked in stands back, its front tab wearing the plate a
    /// tab at rest wears.
    pub(crate) fn render_tab_strip(
        &mut self,
        pane: usize,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let palette = self.interface_theme.palette();
        // No strip without a tab: the empty state has the whole column.
        if self.panes[pane].tabs.is_empty() {
            return None;
        }
        let in_front = pane == self.active_pane_index();
        let front = self.panes[pane].active_tab();
        let ids = self.panes[pane].tabs.clone();
        let (across, down, can_join) = (
            self.can_split_tab_toward(ids[0], SplitAxis::Across),
            self.can_split_tab_toward(ids[0], SplitAxis::Down),
            self.can_join(),
        );

        let tabs = ids
            .iter()
            .enumerate()
            .filter_map(|(index, id)| {
                let tab = self.tab(*id)?;
                let place = TabPlace {
                    pane,
                    index,
                    active: front == Some(*id),
                    in_front,
                    across,
                    down,
                    can_join,
                };
                Some(Self::render_tab(place, tab, palette, cx))
            })
            .collect::<Vec<_>>();

        Some(
            h_flex()
                .h(px(TAB_STRIP_HEIGHT))
                .flex_none()
                .px_2()
                .gap_3()
                .items_center()
                .bg(rgb(palette.tab_bar))
                .border_b_1()
                .border_color(rgb(palette.border))
                .child(
                    h_flex()
                        .id(("strip", pane))
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .items_center()
                        // Past the last tab is the end of the row: a tab
                        // dropped in the air beside them lands there.
                        .drag_over::<DraggedTab>(move |style, _, _, _| {
                            style.bg(tint(palette.accent, DROP_TARGET_WASH))
                        })
                        .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                            this.move_tab_to_pane(dragged.tab, pane, None, window, cx);
                        }))
                        .children(tabs),
                )
                .into_any_element(),
        )
    }

    /// One tab: a status dot, its name — the alias it was given, else the
    /// port's path — and a close mark that shows on the active tab and on
    /// hover; the port and its parameters are in the tooltip, along with
    /// whether the log is a dump, read-only, or being written to a file, so
    /// a named tab still tells you what it is plugged into. The active tab is a raised plate and the
    /// others sit nearly flat on the strip until pointed at — the rule the
    /// segmented switches follow, so every exclusive choice in the workbench
    /// reads the same way. Tabs share the strip: when it fills, they shrink
    /// together and their names truncate, the way a browser's do.
    ///
    /// The plate is the tag's: a wash of the hue with a ring of it when
    /// active, a fainter wash when not, so the tag shows in both states
    /// without the name changing colour. The dot keeps saying whether the
    /// port is open.
    fn render_tab(
        place: TabPlace,
        tab: &SerialTabState,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let TabPlace {
            pane,
            index,
            active,
            in_front,
            ..
        } = place;
        let tab_id = tab.id;
        let name = tab.title().to_string();
        let detail: SharedString = format!(
            "{} · {}{}{}{}{}",
            tab.selected_port().name,
            tab.configuration.summary(),
            if tab.view.is_hex() { " · hex" } else { "" },
            if tab.interactive { "" } else { " · read-only" },
            // The live state, not the switch: it says a file is being
            // written now, which is what you hover a recording tab to ask.
            if tab.recording() { " · recording" } else { "" },
            if tab.transfer.is_some() { " · transferring a file" } else { "" }
        )
        .into();
        let status = if tab.connected {
            palette.success
        } else if tab.connecting {
            palette.warning
        } else {
            palette.faint
        };
        let hue = palette.tag(tab.color);
        let group: SharedString = format!("session-tab-{tab_id}").into();
        let dragged = DraggedTab {
            tab: tab_id,
            name: name.clone().into(),
            hue,
            status,
            ink: palette.strong_foreground,
            plate: palette.card,
        };

        h_flex()
            .id(("session-tab", tab_id))
            .group(group.clone())
            .h(px(TAB_HEIGHT))
            .min_w(px(TAB_MIN_WIDTH))
            .max_w(px(TAB_MAX_WIDTH))
            .pl_2p5()
            .pr_1()
            .gap_2()
            .items_center()
            .rounded(px(7.))
            .border_1()
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
            // Only the pane being worked in raises its front tab all the
            // way: two plates that bright in two strips would both claim to
            // be the session the title bar is speaking to.
            .when(active && in_front, |tab| {
                tab.bg(tint(hue, TAG_PLATE_ACTIVE))
                    .border_color(tint(hue, TAG_RING_ACTIVE))
            })
            .when(active && !in_front, |tab| {
                tab.bg(tint(hue, TAG_PLATE_BEHIND))
                    .border_color(tint(hue, TAG_RING_BEHIND))
            })
            .when(!active, |tab| {
                tab.bg(tint(hue, TAG_PLATE_REST))
                    .border_color(transparent_black())
                    .hover(move |tab| tab.bg(tint(hue, TAG_PLATE_HOVER)))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_pane(pane);
                if let Some(pane) = this.panes.get_mut(pane) {
                    pane.active = tab_id;
                }
                cx.notify();
            }))
            // A tab is dragged the way an editor's is: to another place in
            // its own strip, or into another pane's, which is what moves a
            // session across a split.
            .on_drag(dragged, |dragged, _, _, cx| {
                let dragged = dragged.clone();
                cx.new(|_| dragged)
            })
            .drag_over::<DraggedTab>(move |style, dragged, _, _| {
                if dragged.tab == tab_id {
                    style
                } else {
                    style.border_l_2().border_color(rgb(palette.accent))
                }
            })
            .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                this.move_tab_to_pane(dragged.tab, pane, Some(index), window, cx);
            }))
            .child(Self::status_dot(6., status))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_token(LABEL)
                    .text_color(rgb(if active && in_front {
                        palette.strong_foreground
                    } else if active {
                        palette.foreground
                    } else {
                        palette.muted
                    }))
                    // The name of the session in hand is set a weight above
                    // the rest, so the strip reads as one tab in front of
                    // the others even where the plates are hard to tell
                    // apart — a colour-blind bench, a dimmed screen.
                    .when(active && in_front, |name| {
                        name.font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(name),
            )
            .child(
                div()
                    .flex_none()
                    .when(!active, |close| {
                        close
                            .opacity(0.)
                            .group_hover(group, |close| close.opacity(1.))
                    })
                    .child(
                        Button::new(("close-tab", tab_id))
                            .ghost()
                            .with_size(px(TAB_CLOSE))
                            .icon(IconName::Close)
                            .tooltip("Close session")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(tab_id, cx);
                            })),
                    ),
            )
            .context_menu(Self::tab_menu(tab_id, place, cx))
            .into_any_element()
    }

    /// What a right-click on a tab offers: the two ways to hand this
    /// session a pane of its own, the way back from a split, and the close
    /// the cross beside the name does. The splits grey out when there is
    /// nowhere to split to — one session in the strip, or three panes
    /// already — so the menu says what the window can do rather than
    /// failing quietly.
    fn tab_menu(
        tab_id: usize,
        place: TabPlace,
        cx: &mut Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let workspace = cx.weak_entity();
        let TabPlace {
            across,
            down,
            can_join,
            ..
        } = place;
        move |menu, _, _| {
            let (right, below, join, close) = (
                workspace.clone(),
                workspace.clone(),
                workspace.clone(),
                workspace.clone(),
            );
            menu.item(
                PopupMenuItem::new("Split Right")
                    .action(Box::new(SplitRight))
                    .disabled(!across)
                    .on_click(move |_, window, cx| {
                        let _ = right.update(cx, |this, cx| {
                            this.split_tab(tab_id, SplitAxis::Across, window, cx);
                        });
                    }),
            )
            .item(
                PopupMenuItem::new("Split Down")
                    .action(Box::new(SplitDown))
                    .disabled(!down)
                    .on_click(move |_, window, cx| {
                        let _ = below.update(cx, |this, cx| {
                            this.split_tab(tab_id, SplitAxis::Down, window, cx);
                        });
                    }),
            )
            .item(
                PopupMenuItem::new("Join Split")
                    .action(Box::new(JoinSplit))
                    .disabled(!can_join)
                    .on_click(move |_, window, cx| {
                        let _ = join.update(cx, |this, cx| {
                            this.join_active_pane(window, cx);
                        });
                    }),
            )
            .separator()
            .item(
                PopupMenuItem::new("Close Session").on_click(move |_, _, cx| {
                    let _ = close.update(cx, |this, cx| {
                        this.close_tab(tab_id, cx);
                    });
                }),
            )
        }
    }

    /// The terminal. It is a place to type as well as to read: a click gives
    /// it focus, and from then on keys go to the port of this tab — unless
    /// the tab was made read-only, when the log is only to read and the
    /// composer does the sending; the wheel moves through the scrollback,
    /// and a drag selects, to be copied with ⌘C or pasted back at the
    /// device with ⌘V. A right-click offers the same three by name.
    pub(crate) fn render_pane_log(
        &mut self,
        pane: usize,
        tab: SerialTabSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let focus = self.panes[pane].focus.clone();
        let focused = focus.is_focused(window) && window.is_window_active();
        if focused && tab.interactive {
            self.start_blinking(window, cx);
        }
        let tab_id = tab.id;
        let terminal = self.render_terminal(&tab, pane, focused, cx);
        let scrollbar = self.render_scrollbar(tab_id, cx);
        let menu = self.terminal_menu(pane, cx);
        // The find bar floats over the log's top-right corner while it is
        // open, the way VS Code's find widget does.
        let find_bar = tab.find.open.then(|| self.render_find_bar(&tab, cx));
        // A file going over the port has a strip along the top of the
        // log, until it is there.
        let transfer = tab
            .transfer
            .as_ref()
            .map(|transfer| self.render_transfer_strip(tab_id, transfer, cx));

        v_flex()
            .id(("pane-log", tab_id))
            .flex_1()
            .min_h_0()
            .bg(rgb(palette.editor))
            .overflow_hidden()
            // A tab let go over a log joins that pane, at the end of its
            // strip. The log washes over while the tab is held above it,
            // so the pane that would take it says so before the release.
            .drag_over::<DraggedTab>(move |style, _, _, _| {
                style.bg(tint(palette.accent, DROP_TARGET_WASH))
            })
            .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                this.move_tab_to_pane(dragged.tab, pane, None, window, cx);
            }))
            .children(transfer)
            .child(
                div()
                    .id(("terminal", tab_id))
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .track_focus(&focus)
                    .key_context(TERMINAL_CONTEXT)
                    .cursor(CursorStyle::IBeam)
                    // A key arriving here says which pane holds the
                    // cursor, whatever was clicked last: what is typed
                    // goes to the log it is typed at.
                    .on_key_down(cx.listener(move |this, event, window, cx| {
                        this.select_pane(pane);
                        this.terminal_key(event, window, cx)
                    }))
                    // Bound keys are matched before the key handler runs,
                    // so Tab arrives as an action of the log's own.
                    .on_action(cx.listener(move |this, _: &SendTab, window, cx| {
                        this.select_pane(pane);
                        this.type_tab(false, window, cx)
                    }))
                    .on_action(cx.listener(move |this, _: &SendBackTab, window, cx| {
                        this.select_pane(pane);
                        this.type_tab(true, window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.select_pane(pane);
                            this.terminal_mouse_down(tab_id, event, cx)
                        }),
                    )
                    // The menu a right-click opens is this pane's, so the
                    // press that opens it works here from then on.
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            this.select_pane(pane);
                            cx.notify();
                        }),
                    )
                    .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                        let line_height = px(this.terminal_type().line_height);
                        let delta = event.delta.pixel_delta(line_height).y / line_height;
                        this.scroll_terminal(tab_id, delta, cx);
                    }))
                    .child(terminal)
                    .children(scrollbar)
                    .children(find_bar)
                    .context_menu(menu),
            )
            .into_any_element()
    }

    /// What a right-click on the log offers: the copy and paste every
    /// terminal has there, the copy that brings the timestamps with it,
    /// and the select all that goes with them. The items are greyed by
    /// what the log can do at that moment — nothing selected, or a tab
    /// that takes no typing — and each carries its keystroke, read in the
    /// log's own key context.
    fn terminal_menu(
        &self,
        pane: usize,
        cx: &mut Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let workspace = cx.weak_entity();
        let focus = self.panes[pane].focus.clone();
        // What the items can do is asked of *this* pane's log, not of the
        // one in front: a right-click in the pane beside you is about the
        // log you right-clicked.
        let tab = self.panes[pane].active_tab().and_then(|id| self.tab(id));
        let has_selection = tab.and_then(SerialTabState::selection_text).is_some();
        let takes_input = tab.is_some_and(|tab| tab.connected && tab.interactive);
        move |menu, _, _| {
            let (copy, stamped, paste, select_all) = (
                workspace.clone(),
                workspace.clone(),
                workspace.clone(),
                workspace.clone(),
            );
            menu.action_context(focus.clone())
                .item(
                    PopupMenuItem::new("Copy")
                        .action(Box::new(CopyTerminalSelection))
                        .disabled(!has_selection)
                        .on_click(move |_, _, cx| {
                            let _ = copy.update(cx, |this, cx| {
                                this.select_pane(pane);
                                this.copy_terminal_selection(cx);
                            });
                        }),
                )
                // The gutter's times come along with the text, each at the
                // head of its line — no keystroke of its own, since it is
                // the copy you go to the menu for.
                .item(
                    PopupMenuItem::new("Copy with Timestamps")
                        .disabled(!has_selection)
                        .on_click(move |_, _, cx| {
                            let _ = stamped.update(cx, |this, cx| {
                                this.select_pane(pane);
                                this.copy_terminal_selection_with_stamps(cx);
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new("Paste")
                        .action(Box::new(PasteIntoTerminal))
                        .disabled(!takes_input)
                        .on_click(move |_, window, cx| {
                            let _ = paste.update(cx, |this, cx| {
                                this.select_pane(pane);
                                this.paste_into_terminal(window, cx);
                            });
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Select All")
                        .action(Box::new(SelectAllInTerminal))
                        .on_click(move |_, _, cx| {
                            let _ = select_all.update(cx, |this, cx| {
                                this.select_pane(pane);
                                this.select_all_in_terminal(cx);
                            });
                        }),
                )
        }
    }

    /// The terminal itself: a canvas that fits alacritty's grid to its
    /// bounds before painting, and paints the cells straight from it. The
    /// platform's text input is wired to it while it holds focus, so an
    /// input method can compose before anything is sent.
    ///
    /// The gutter is measured here: the timestamps first, then a column of
    /// line numbers as wide as the highest number in the log, each with air
    /// after it.
    ///
    /// While a selection is being dragged out, the pointer is followed
    /// from here at the window rather than at the log, so the drag goes on
    /// past the terminal's edges and the release that ends it can land
    /// anywhere.
    fn render_terminal(
        &mut self,
        tab: &SerialTabSnapshot,
        pane: usize,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let terminal_palette = self.interface_theme.terminal_palette();
        let tab_id = tab.id;
        let filter = tab.filter.clone();
        let find = tab.find.clone();
        let digits = self
            .tab(tab_id)
            .map_or(0, |tab| tab.terminal.number_digits());
        let focus = self.panes[pane].focus.clone();
        let composing = self.composing.clone();
        let cursor_shown = self.cursor_shown;
        let selecting = self.tab(tab_id).is_some_and(|tab| tab.selecting);
        let interactive = tab.interactive;
        let fit = cx.entity();
        let paint = cx.entity();
        let text = self.terminal_type();

        canvas(
            move |bounds, window, cx| {
                let layout = TerminalLayout::measure(window, digits, text);
                let columns = (bounds.size.width - layout.gutter - px(ROW_INSET)) / layout.cell_width;
                let lines = bounds.size.height / px(text.line_height);
                let columns = columns.floor().max(0.) as usize;
                let lines = lines.floor().max(0.) as usize;
                fit.update(cx, |this, _| {
                    if let Some(tab) = this.tab_mut(tab_id) {
                        tab.metrics = TerminalMetrics {
                            cell_width: f32::from(layout.cell_width),
                            line_height: text.line_height,
                            text_left: f32::from(layout.gutter),
                            origin_x: f32::from(bounds.origin.x),
                            origin_y: f32::from(bounds.origin.y),
                            columns,
                            lines,
                            width: f32::from(bounds.size.width),
                            height: f32::from(bounds.size.height),
                        };
                        tab.terminal.resize(columns, lines);
                    }
                });
                layout
            },
            move |bounds, layout, window, cx| {
                window.handle_input(&focus, ElementInputHandler::new(bounds, paint.clone()), cx);
                if selecting {
                    // In the capture phase, so nothing the pointer passes
                    // over can swallow the drag or the release.
                    let drag = paint.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        drag.update(cx, |this, cx| {
                            // A release that went unseen — one that fell
                            // between frames — shows as a move with the
                            // button up, and ends the drag the same.
                            if event.pressed_button == Some(MouseButton::Left) {
                                this.terminal_drag(tab_id, event.position, cx);
                            } else {
                                this.terminal_release(tab_id, cx);
                            }
                        });
                    });
                    let release = paint.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                            release.update(cx, |this, cx| this.terminal_release(tab_id, cx));
                        }
                    });
                }
                // The grid was just fitted to the bounds, so the mask is
                // brought up to it here rather than left from the frame's
                // start; nothing to do when nothing changed.
                let Some(content) = paint.update(cx, |this, _| {
                    this.tab_mut(tab_id).map(|tab| {
                        tab.refresh_mask();
                        tab.view_content(&terminal_palette)
                    })
                }) else {
                    return;
                };
                paint_terminal(
                    bounds,
                    layout,
                    &content,
                    focused,
                    interactive,
                    cursor_shown,
                    composing.as_deref(),
                    &filter,
                    &find,
                    palette,
                    terminal_palette,
                    window,
                    cx,
                );
            },
        )
        .absolute()
        .top(px(TERMINAL_TOP_INSET))
        .left_0()
        .right_0()
        .bottom_0()
        .into_any_element()
    }

    /// Where the scrollbar's track and its thumb stand this frame, read
    /// from the log the tab in front shows and the box the terminal was
    /// last laid out in. Nothing while the whole log is on screen — there
    /// is nothing to scroll, and so no bar — or before the log has been
    /// laid out at all.
    pub(crate) fn scrollbar_geometry(&self, tab_id: usize) -> Option<ScrollbarGeometry> {
        let tab = self.tab(tab_id)?;
        let scroll = tab.view_scroll();
        let track_height = tab.metrics.height;
        if scroll.visible == 0 || scroll.total <= scroll.visible || track_height <= 0. {
            return None;
        }
        // As much of the track as the screen is of the log, but never so
        // little that it cannot be aimed at.
        let thumb_height = (track_height * scroll.visible as f32 / scroll.total as f32)
            .max(SCROLLBAR_THUMB_MIN)
            .min(track_height);
        let extent = scroll.total - scroll.visible;
        let thumb_top =
            (track_height - thumb_height) * (scroll.above as f32 / extent as f32).clamp(0., 1.);
        Some(ScrollbarGeometry {
            track_top: tab.metrics.origin_y,
            track_height,
            thumb_top,
            thumb_height,
            extent,
        })
    }

    /// The scrollbar: a slim bar over the log's right edge, in the margin
    /// the rows already leave at their end, so it costs the text no
    /// columns. The thumb is as much of the track as the screen is of the
    /// whole log and stands where the view stands in it, so a long log
    /// says so at a glance; the pointer anywhere on the track widens the
    /// thumb and brings it forward, and it can be thrown from one end of
    /// fifty thousand lines to the other in a single drag, which the wheel
    /// cannot. It is drawn only while there is more log than screen.
    fn render_scrollbar(&mut self, tab_id: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Some(geometry) = self.scrollbar_geometry(tab_id) else {
            // A bar that has gone — the log cleared, the tab closed —
            // takes the pointer's hold on it with it, since there is
            // nothing left to see the release.
            if let Some(tab) = self.tab_mut(tab_id) {
                tab.scrollbar_grab = None;
                tab.scrollbar_hovered = false;
            }
            return None;
        };
        let palette = self.interface_theme.palette();
        let held = self.tab(tab_id).is_some_and(|tab| tab.scrollbar_grab.is_some());
        // The pointer's state is kept rather than left to a hover style:
        // a hover style is laid on at paint, after the layout that would
        // have to widen the thumb.
        let near = held || self.tab(tab_id).is_some_and(|tab| tab.scrollbar_hovered);
        let ink = if held {
            THUMB_HELD
        } else if near {
            THUMB_HOVER
        } else {
            THUMB_REST
        };
        let watch = held.then(|| self.watch_scrollbar_drag(tab_id, cx));

        Some(
            div()
                .id(("terminal-scrollbar", tab_id))
                .absolute()
                .top(px(TERMINAL_TOP_INSET))
                .right_0()
                .bottom_0()
                .w(px(SCROLLBAR_WIDTH))
                .cursor(CursorStyle::Arrow)
                // Over the log, not of it, as the find bar is: a press on
                // the bar neither focuses the log nor starts a selection
                // in it, while the wheel over the bar still scrolls it.
                .block_mouse_except_scroll()
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if let Some(tab) = this.tab_mut(tab_id)
                        && tab.scrollbar_hovered != *hovered
                    {
                        tab.scrollbar_hovered = *hovered;
                        cx.notify();
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.scrollbar_press(tab_id, event.position, cx);
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(geometry.thumb_top))
                        .left_0()
                        .right_0()
                        .h(px(geometry.thumb_height))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .h_full()
                                .w(px(if near {
                                    SCROLLBAR_THUMB_WIDE
                                } else {
                                    SCROLLBAR_THUMB
                                }))
                                .rounded_full()
                                .bg(tint(palette.foreground, ink)),
                        ),
                )
                .children(watch)
                .into_any_element(),
        )
    }

    /// While the thumb is held the pointer is followed at the window
    /// rather than at the bar, so the drag goes on when the pointer leaves
    /// the track — sideways over the log, or past the window's edge — and
    /// the release that ends it can land anywhere.
    fn watch_scrollbar_drag(&self, tab_id: usize, cx: &mut Context<Self>) -> AnyElement {
        let workspace = cx.entity();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let drag = workspace.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    drag.update(cx, |this, cx| {
                        // A release that went unseen — one that fell
                        // between frames — shows as a move with the button
                        // up, and ends the drag the same.
                        if event.pressed_button == Some(MouseButton::Left) {
                            this.scrollbar_drag(tab_id, event.position, cx);
                        } else {
                            this.scrollbar_release(tab_id, cx);
                        }
                    });
                });
                let release = workspace.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        release.update(cx, |this, cx| this.scrollbar_release(tab_id, cx));
                    }
                });
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }

    /// The `serialX` wordmark, inked like the logo: an orange `s`, the body in
    /// the theme's own ink, and a green `X`.
    ///
    /// GPUI shapes a text element as a single run, so the three inks come from
    /// highlight ranges over one string rather than from three labels set side
    /// by side, which would lose the kerning between the letters.
    fn render_wordmark(palette: WorkbenchPalette) -> impl IntoElement {
        let ink = |color: u32| HighlightStyle {
            color: Some(rgb(color).into()),
            ..Default::default()
        };

        div()
            .text_token(WORDMARK)
            .text_color(rgb(palette.wordmark_body))
            .child(StyledText::new("serialX").with_highlights([
                (0..1, ink(palette.wordmark_lead)),
                (6..7, ink(palette.wordmark_tail)),
            ]))
    }

    /// Shown when no tab is open: identity first, then the one way in.
    pub(crate) fn render_empty_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();

        v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_7()
            .bg(rgb(palette.editor))
            .child(
                v_flex()
                    .items_center()
                    .gap_5()
                    .child(img(application_icon_image()).size(px(104.)))
                    .child(
                        v_flex()
                            .items_center()
                            .gap_1p5()
                            .child(Self::render_wordmark(palette))
                            .child(
                                div()
                                    .text_token(BODY)
                                    .text_color(rgb(palette.muted))
                                    .child("A serial port workspace for your work."),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("empty-new-session")
                            .primary()
                            .px_4()
                            .icon(Glyph::Bolt)
                            .label("New session")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_new_serial_tab_dialog(window, cx);
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .text_token(CAPTION)
                            .text_color(rgb(palette.faint))
                            .child("or press")
                            .children(
                                Kbd::binding_for_action(&NewSerialTab, None, window)
                                    .map(Kbd::outline),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// The mono family as the terminal sets it, in one of its four faces.
fn terminal_font(bold: bool, italic: bool) -> Font {
    Font {
        family: fonts().mono.clone(),
        features: FontFeatures::default(),
        fallbacks: fonts().cjk.clone(),
        weight: if bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    }
}

/// The terminal's type as the settings have it: the size the log is set in,
/// with everything measured in type scaled to it — the leading, the line
/// numbers and timestamps in the gutter, and the column they need. So a
/// larger size grows the whole log rather than leaving small numbers under
/// big text.
#[derive(Clone, Copy)]
pub(crate) struct TerminalType {
    /// The size the cells are set in.
    font_size: f32,
    /// How tall a row stands, in whole pixels so rows land on the grid.
    line_height: f32,
    /// The size the gutter's numbers and timestamps are set in.
    gutter_size: f32,
    /// Width of the timestamp column at that size.
    time_gutter: f32,
    /// The size against the default, for measurements taken at the default.
    scale: f32,
}

impl TerminalType {
    fn new(font_size: f32) -> Self {
        let font_size = usable_font_size(font_size);
        let scale = font_size / DEFAULT_TERMINAL_FONT_SIZE;
        Self {
            font_size,
            line_height: (font_size * TERMINAL_LEADING).round(),
            gutter_size: MONO_SMALL.size * scale,
            time_gutter: (TIME_GUTTER * scale).round(),
            scale,
        }
    }
}

/// How the terminal's cells and gutters map to pixels, measured from the
/// fonts each frame.
#[derive(Clone, Copy)]
struct TerminalLayout {
    /// The type the measurements were taken at.
    text: TerminalType,
    /// A cell's width: the advance of `m` in the terminal's font.
    cell_width: Pixels,
    /// The right edge of the line-number column, which the numbers are set
    /// against, from the terminal's left edge.
    number_right: Pixels,
    /// Where the timestamps start, from the terminal's left edge.
    stamp_left: Pixels,
    /// Where the cells start, from the terminal's left edge.
    gutter: Pixels,
}

impl TerminalLayout {
    /// Measures for numbers `digits` wide, in the type given.
    fn measure(window: &Window, digits: usize, text: TerminalType) -> Self {
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&terminal_font(false, false));
        let advance = |size: f32, glyph: char, fallback: f32| {
            text_system
                .advance(font_id, px(size), glyph)
                .map(|advance| advance.width)
                .unwrap_or(px(fallback * text.scale))
        };
        let cell_width = advance(text.font_size, 'm', 7.5);
        let number_width = advance(text.gutter_size, '0', 6.6) * digits as f32;
        // The time a line arrived is what the eye goes to first, so it has
        // the left edge; the numbers stand between it and the text.
        let stamp_left = px(ROW_INSET);
        let number_right = stamp_left + px(text.time_gutter + ROW_GAP) + number_width;
        Self {
            text,
            cell_width,
            number_right,
            stamp_left,
            gutter: number_right + px(ROW_GAP),
        }
    }
}

/// Paints the screen: under everything the filter's tint on the rows it
/// matches and the selection's plate on the cells it covers, then a
/// filled cursor so its glyph stays readable on it, then row by row the
/// timestamp, the line number, each run's background and text and the
/// find's washes over what it found, and last the cursor when it is an
/// outline. With focus the cursor blinks — it is left out in the off
/// half — and without focus it stands as a steady outline. A read-only
/// tab has no cursor at all: there is nowhere to type. Under the mask the
/// rows are the lines that match and nothing is tinted; with no line
/// matching, the screen says so rather than standing empty.
#[allow(clippy::too_many_arguments)]
fn paint_terminal(
    bounds: Bounds<Pixels>,
    layout: TerminalLayout,
    content: &RenderContent,
    focused: bool,
    interactive: bool,
    cursor_shown: bool,
    composing: Option<&str>,
    filter: &OutputFilter,
    find: &FindView,
    palette: WorkbenchPalette,
    terminal_palette: TerminalPalette,
    window: &mut Window,
    cx: &mut App,
) {
    let line_height = px(layout.text.line_height);
    let font_size = px(layout.text.font_size);
    let cell_width = layout.cell_width;
    let text_left = bounds.origin.x + layout.gutter;
    let text_system = window.text_system().clone();
    let gutter_font = terminal_font(false, false);
    let cursor_color = rgb(terminal_palette.cursor);
    let finding = find.open && find.matcher.is_active();
    let tinting = filter.is_active() && filter.mode() == FilterMode::Highlight;

    for (index, row) in content.rows.iter().enumerate() {
        let y = bounds.origin.y + line_height * index as f32;
        if tinting && filter.matches(&row.text) {
            window.paint_quad(fill(
                Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, line_height)),
                tint(palette.accent, 0.12),
            ));
        }
        // The plate is the theme's selection colour, the one the inputs
        // select in, laid under the text so the letters keep their ink.
        if let Some(cells) = &row.selected {
            window.paint_quad(fill(
                Bounds::new(
                    point(text_left + cell_width * cells.start as f32, y),
                    size(cell_width * cells.len() as f32, line_height),
                ),
                rgb(palette.selection),
            ));
        }
    }

    let cursor_cell = content
        .cursor
        .as_ref()
        .filter(|_| interactive)
        .map(|cursor| {
            Bounds::new(
                point(
                    text_left + cell_width * cursor.column as f32,
                    bounds.origin.y + line_height * cursor.line as f32,
                ),
                size(cell_width * if cursor.wide { 2. } else { 1. }, line_height),
            )
        });
    if let (Some(cursor), Some(cell)) = (&content.cursor, cursor_cell)
        && focused
        && cursor_shown
        && cursor.shape == CaretShape::Block
    {
        window.paint_quad(fill(cell, cursor_color));
    }

    // A line of gutter type: a number or a stamp, in the faint ink.
    let gutter_line = |text: &str| {
        let run = TextRun {
            len: text.len(),
            font: gutter_font.clone(),
            color: rgb(palette.faint).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        text_system.shape_line(
            SharedString::from(text.to_owned()),
            px(layout.text.gutter_size),
            &[run],
            None,
        )
    };

    for (index, row) in content.rows.iter().enumerate() {
        let y = bounds.origin.y + line_height * index as f32;
        if let Some(stamp) = &row.stamp {
            let line = gutter_line(stamp);
            let _ = line.paint(
                point(bounds.origin.x + layout.stamp_left, y),
                line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        // The number sits against the right edge of its column, as an
        // editor's do, so the units line up.
        if let Some(number) = row.number {
            let line = gutter_line(&number.to_string());
            let x = bounds.origin.x + layout.number_right - line.width;
            let _ = line.paint(point(x, y), line_height, TextAlign::Left, None, window, cx);
        }
        for run in &row.runs {
            let x = text_left + cell_width * run.column as f32;
            if let Some(background) = run.style.background {
                window.paint_quad(fill(
                    Bounds::new(point(x, y), size(cell_width * run.width as f32, line_height)),
                    rgb(background),
                ));
            }
            let text_run = TextRun {
                len: run.text.len(),
                font: terminal_font(run.style.bold, run.style.italic),
                color: rgb(run.style.foreground).into(),
                background_color: None,
                underline: run.style.underline.then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                }),
                strikethrough: run.style.strikeout.then(|| StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                }),
            };
            let line = text_system.shape_line(
                SharedString::from(run.text.clone()),
                font_size,
                &[text_run],
                None,
            );
            let _ = line.paint(point(x, y), line_height, TextAlign::Left, None, window, cx);
        }
        // What the find finds on this row, washed over the text: every
        // occurrence lightly, the one in hand more so and ringed. The rows
        // on screen are matched as they are painted, so the wash never
        // lags the log the way the count may.
        if finding {
            for range in find.matcher.find_ranges(&row.text) {
                let start = row.text[..range.start].chars().count();
                let end = start + row.text[range].chars().count();
                let (Some(&first), Some(&last)) = (row.columns.get(start), row.columns.get(end - 1))
                else {
                    continue;
                };
                // A wide character at the end is as wide as the cell after
                // it says; at the row's end there is nothing to say, and it
                // is taken for one.
                let after = row.columns.get(end).map_or(last + 1, |&next| next.max(last + 1));
                let cell = Bounds::new(
                    point(text_left + cell_width * first as f32, y),
                    size(cell_width * (after - first) as f32, line_height),
                );
                let current = find
                    .current
                    .is_some_and(|span| span.line == row.line && span.column == first);
                if current {
                    window.paint_quad(
                        fill(cell, tint(palette.warning, FIND_WASH_CURRENT))
                            .border_widths(Edges::all(px(1.)))
                            .border_color(rgb(palette.accent)),
                    );
                } else {
                    window.paint_quad(fill(cell, tint(palette.warning, FIND_WASH)));
                }
            }
        }
    }

    // A mask that keeps nothing says so where the first line would be, so
    // an empty screen is not taken for a quiet device.
    if filter.masking() && content.rows.is_empty() {
        let run = TextRun {
            len: MASK_EMPTY_HINT.len(),
            font: terminal_font(false, true),
            color: rgb(palette.faint).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = text_system.shape_line(SharedString::from(MASK_EMPTY_HINT), font_size, &[run], None);
        let _ = line.paint(
            point(text_left, bounds.origin.y),
            line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    // Text an input method is still composing sits at the cursor, underlined,
    // until it is committed and sent.
    if let (Some(text), Some(cell)) = (composing, cursor_cell)
        && !text.is_empty()
    {
        let run = TextRun {
            len: text.len(),
            font: terminal_font(false, false),
            color: rgb(palette.strong_foreground).into(),
            background_color: Some(rgb(palette.editor).into()),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(rgb(palette.accent).into()),
                wavy: false,
            }),
            strikethrough: None,
        };
        let line = text_system.shape_line(SharedString::from(text.to_owned()), font_size, &[run], None);
        window.paint_quad(fill(
            Bounds::new(cell.origin, size(line.width, line_height)),
            rgb(palette.editor),
        ));
        let _ = line.paint(cell.origin, line_height, TextAlign::Left, None, window, cx);
    }

    if let (Some(cursor), Some(cell)) = (&content.cursor, cursor_cell) {
        let outline = || {
            fill(cell, transparent_black())
                .border_widths(Edges::all(px(1.)))
                .border_color(cursor_color)
        };
        match cursor.shape {
            _ if !focused => window.paint_quad(outline()),
            _ if !cursor_shown => {}
            CaretShape::Block => {}
            CaretShape::Hollow => window.paint_quad(outline()),
            CaretShape::Underline => window.paint_quad(fill(
                Bounds::new(
                    point(cell.origin.x, cell.bottom() - px(CARET_THICKNESS)),
                    size(cell.size.width, px(CARET_THICKNESS)),
                ),
                cursor_color,
            )),
            CaretShape::Beam => window.paint_quad(fill(
                Bounds::new(cell.origin, size(px(CARET_THICKNESS), cell.size.height)),
                cursor_color,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SCROLLBAR_THUMB_MIN, ScrollbarGeometry};

    /// The geometry as `scrollbar_geometry` builds it, for a track of
    /// `height` over a log of `total` rows showing `visible` of them with
    /// `above` of them above the view.
    fn geometry(height: f32, total: usize, visible: usize, above: usize) -> ScrollbarGeometry {
        let thumb_height = (height * visible as f32 / total as f32)
            .max(SCROLLBAR_THUMB_MIN)
            .min(height);
        let extent = total - visible;
        ScrollbarGeometry {
            track_top: 100.,
            track_height: height,
            thumb_top: (height - thumb_height) * (above as f32 / extent as f32).clamp(0., 1.),
            thumb_height,
            extent,
        }
    }

    /// The thumb is the screen's share of the log, and stands where the
    /// view stands: at the top with the whole log above it, at the foot
    /// while the view follows the newest output.
    #[test]
    fn the_thumb_is_the_screens_share_of_the_log() {
        let bar = geometry(400., 100, 25, 0);
        assert_eq!(bar.thumb_height, 100.);
        assert_eq!(bar.thumb_top, 0.);
        let bar = geometry(400., 100, 25, 75);
        assert_eq!(bar.thumb_top, 300.);
        assert!(bar.holds(300.));
        assert!(bar.holds(399.));
        assert!(!bar.holds(299.));
    }

    /// However long the log grows the thumb stays big enough to aim at,
    /// and its travel still reaches both ends of the log.
    #[test]
    fn a_long_log_keeps_a_thumb_to_take_hold_of() {
        let bar = geometry(400., 50_000, 25, 0);
        assert_eq!(bar.thumb_height, SCROLLBAR_THUMB_MIN);
        assert_eq!(bar.rows_above(0.), 0);
        assert_eq!(bar.rows_above(400. - SCROLLBAR_THUMB_MIN), 49_975);
        // Past either end of the travel is that end, not beyond it.
        assert_eq!(bar.rows_above(-40.), 0);
        assert_eq!(bar.rows_above(4_000.), 49_975);
    }

    /// A thumb that fills its track has nowhere to go.
    #[test]
    fn a_thumb_that_fills_the_track_stays_put() {
        let bar = ScrollbarGeometry {
            track_top: 0.,
            track_height: 400.,
            thumb_top: 0.,
            thumb_height: 400.,
            extent: 3,
        };
        assert_eq!(bar.rows_above(200.), 0);
    }
}
