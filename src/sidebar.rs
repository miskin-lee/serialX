//! The right panel: the library of things you keep between sessions.
//!
//! Two stacked sections, each a header over a scrolling list of cards. Rows are
//! cards rather than table lines so a saved item reads as an object you can act
//! on, and every one is anchored by a coloured glyph in the manner of VS Code's
//! Material Icon Theme: a command is green, and a session wears its own tag
//! colour, so the card and the tab it opens match.
//!
//! Saved sessions file under groups — folders, each a row with a chevron over
//! its cards, the way an explorer shows a directory — with the sessions in no
//! group at the top level beneath them. A card opens on a double-click, as a
//! session does in any session manager; a single click only picks it out, and
//! its two buttons edit it and forget it, so nothing happens by a slip of the
//! pointer.
//!
//! Two things fold here, at different grains: a section collapses to its own
//! header, and the whole panel collapses to an icon rail that still carries the
//! counts and reopens on a click. A collapsed section gives its height back to
//! its neighbour, so the panel never holds a half-empty list above a full one.
//!
//! Along the panel's foot sits the composer: the one box commands are typed
//! into, under the commands kept for reuse, so a line and the bookmark that
//! saves it are a hand's width apart. It sends to whichever tab is in front,
//! and says which.

use gpui_kit::component::{
    Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app_menu::SaveCurrentSession;
use crate::controls::{Choice, ChoiceText, segmented, spaced_caps};
use crate::groups::GroupPrompt;
use crate::icons::{Glyph, icon_chip};
use crate::presets::{StoredCommand, StoredGroup, StoredSession};
use crate::theme::{
    CAPTION, EYEBROW, LABEL, MICRO, MONO_SMALL, TagColor, Typography, WorkbenchPalette, tint,
};
use crate::{SerialConfiguration, SerialTabSnapshot, SerialWorkspace};

/// Width the panel opens at. Wide enough for a port path plus its baud
/// summary on one line at the caption size; the edge drags from there.
pub(crate) const SIDEBAR_WIDTH: f32 = 296.;
/// The narrowest the panel drags to: one card with its glyph and a name.
pub(crate) const SIDEBAR_MIN_WIDTH: f32 = 220.;
/// The widest: past this the cards are mostly air.
pub(crate) const SIDEBAR_MAX_WIDTH: f32 = 560.;
/// Width of the collapsed rail: one icon chip plus breathing room.
const RAIL_WIDTH: f32 = 52.;
/// The chip beside a section's title, in the section's colour.
const SECTION_CHIP: f32 = 20.;
/// The search box over the Quick send list, and the row it sits in.
const SEARCH_HEIGHT: f32 = 26.;
const SEARCH_ROW_HEIGHT: f32 = 34.;
/// What the search box says while it is empty.
pub(crate) const COMMAND_SEARCH_PLACEHOLDER: &str = "Search commands";
/// Height of the composer at the panel's foot: the box, the row of switches
/// under it, and the air around them.
const COMPOSER_HEIGHT: f32 = 82.;
/// Height of the box commands are typed into.
const COMPOSER_INPUT_HEIGHT: f32 = 32.;
/// What the composer says while it is empty.
pub(crate) const SEND_PLACEHOLDER: &str = "Enter a command…";
const SECTION_HEADER_HEIGHT: f32 = 38.;
/// The most of the panel the saved sessions take while Quick send is open.
const SESSIONS_SHARE: f32 = 0.5;
/// The rows of a list are of fixed heights, and the list is given the sum:
/// a scroll region only scrolls from a definite height, and a list sized to
/// its rows by the layout alone came out empty. A card is two lines of type
/// with the padding around them; the gap is the list's `gap_1p5`; the
/// bottom padding its `pb_2`.
const CARD_HEIGHT: f32 = 52.;
const LIST_GAP: f32 = 6.;
const LIST_PAD_BOTTOM: f32 = 8.;
/// Height of the prompt an empty list shows.
const EMPTY_HINT_HEIGHT: f32 = 122.;
/// Height of a group's row: a line of label type with air, shorter than a
/// card so the folders read as headings over their cards, not as more cards.
const GROUP_ROW_HEIGHT: f32 = 28.;
/// Height of the line an empty group shows in place of cards.
const GROUP_EMPTY_HEIGHT: f32 = 26.;
/// Where a group's guide line falls: under the centre of its chevron.
const GROUP_GUIDE_INSET: f32 = 11.;
/// The gap between the guide and the cards it groups.
const GROUP_BODY_GAP: f32 = 8.;
/// A row's button, sized the way an explorer's row actions are: a glyph of
/// sixteen pixels in a target of twenty-two, large enough to read and to hit.
const ROW_ACTION_SIZE: f32 = 22.;
/// Resting opacity of a row's buttons. Visible enough to find, quiet enough
/// not to compete with the row they act on.
const ROW_ACTION_REST: f32 = 0.35;
/// How strongly the accent washes the picked-out card, and rings it.
const SELECTED_PLATE: f32 = 0.07;
const SELECTED_RING: f32 = 0.55;

/// Which of the two libraries a header or rail button stands for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelSection {
    Sessions,
    Commands,
}

impl SerialWorkspace {
    pub(crate) fn save_active_session(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let port_name = tab.selected_port().name.clone();
        let (configuration, color, alias, group) =
            (tab.configuration, tab.color, tab.alias.clone(), tab.group);
        self.save_session_preset(port_name, configuration, color, alias, group);
        cx.notify();
    }

    /// Keeps a session in the panel, filed under its group, which is
    /// unfolded — as is the section — so the new card is in view.
    pub(crate) fn save_session_preset(
        &mut self,
        port_name: String,
        configuration: SerialConfiguration,
        color: TagColor,
        alias: Option<String>,
        group: Option<u64>,
    ) {
        let label = format!("{} · {}", port_name, configuration.summary());
        let group = self.presets.resolve_group(group);
        self.presets
            .add_session(label, port_name, configuration, color, alias, group);
        self.sessions_collapsed = false;
        if let Some(group) = group {
            self.collapsed_groups.remove(&group);
        }
    }

    /// Opens a saved session: a tab on its port, connecting at once when the
    /// device is attached, so a double-click on the card is the whole way
    /// from the list to a live terminal. A tab already on that port is
    /// brought to the front instead — a port cannot be opened twice.
    pub(crate) fn open_saved_session(
        &mut self,
        saved_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(saved) = self
            .presets
            .sessions
            .iter()
            .find(|saved| saved.id == saved_id)
            .cloned()
        else {
            return;
        };
        self.selected_saved = Some(saved_id);

        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.selected_port().name == saved.port_name)
        {
            self.active_tab = index;
            cx.notify();
            return;
        }

        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut tab = Self::build_tab(id, window, cx);
        tab.configuration = saved.configuration.sanitized();
        tab.color = saved.color;
        tab.alias = saved.alias.clone();
        tab.group = self.presets.resolve_group(saved.group);
        match tab
            .ports
            .iter()
            .position(|port| port.name == saved.port_name)
        {
            Some(index) => tab.selected_port = index,
            None => {
                tab.ports.push(crate::PortItem::unavailable(
                    saved.port_name.clone(),
                    "Saved device · currently unavailable",
                ));
                tab.selected_port = tab.ports.len() - 1;
            }
        }
        tab.note(format!("Restored saved session: {}", saved.label));
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        self.connect_if_attached(id, cx);
        cx.notify();
    }

    fn select_saved_session(&mut self, saved_id: u64, cx: &mut Context<Self>) {
        if self.selected_saved != Some(saved_id) {
            self.selected_saved = Some(saved_id);
            cx.notify();
        }
    }

    fn remove_saved_session(&mut self, saved_id: u64, cx: &mut Context<Self>) {
        self.presets.remove_session(saved_id);
        if self.selected_saved == Some(saved_id) {
            self.selected_saved = None;
        }
        cx.notify();
    }

    fn toggle_group(&mut self, group_id: u64, cx: &mut Context<Self>) {
        if !self.collapsed_groups.remove(&group_id) {
            self.collapsed_groups.insert(group_id);
        }
        cx.notify();
    }

    /// Removes a group. Its sessions stay, at the top of the list.
    fn remove_group(&mut self, group_id: u64, cx: &mut Context<Self>) {
        self.presets.remove_group(group_id);
        self.collapsed_groups.remove(&group_id);
        cx.notify();
    }

    pub(crate) fn save_current_command(&mut self, cx: &mut Context<Self>) {
        let value = self.send_input.read(cx).value().trim().to_string();
        if value.is_empty() {
            return;
        }
        self.presets.add_command(value);
        self.commands_collapsed = false;
        cx.notify();
    }

    fn send_saved_command(&mut self, command_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = self
            .presets
            .commands
            .iter()
            .find(|command| command.id == command_id)
            .map(|command| command.command.clone())
        else {
            return;
        };
        if self.active_tab().is_none() {
            return;
        }
        let input = self.send_input.clone();
        input.update(cx, |input, cx| input.set_value(command, window, cx));
        self.send_to_active_tab(window, cx);
    }

    fn remove_saved_command(&mut self, command_id: u64, cx: &mut Context<Self>) {
        self.presets.remove_command(command_id);
        cx.notify();
    }

    pub(crate) fn toggle_side_panel(&mut self, cx: &mut Context<Self>) {
        self.side_panel_collapsed = !self.side_panel_collapsed;
        cx.notify();
    }

    fn toggle_section(&mut self, section: PanelSection, cx: &mut Context<Self>) {
        match section {
            PanelSection::Sessions => self.sessions_collapsed = !self.sessions_collapsed,
            PanelSection::Commands => self.commands_collapsed = !self.commands_collapsed,
        }
        cx.notify();
    }

    /// Expands the panel with one section showing, for the rail buttons.
    fn reveal_section(&mut self, section: PanelSection, cx: &mut Context<Self>) {
        self.side_panel_collapsed = false;
        match section {
            PanelSection::Sessions => self.sessions_collapsed = false,
            PanelSection::Commands => self.commands_collapsed = false,
        }
        cx.notify();
    }

    fn is_collapsed(&self, section: PanelSection) -> bool {
        match section {
            PanelSection::Sessions => self.sessions_collapsed,
            PanelSection::Commands => self.commands_collapsed,
        }
    }

    /// True when a tab is currently pointed at this port, so a saved row can
    /// say "this one is already open" without opening anything to find out.
    fn port_is_open(&self, port_name: &str) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.selected_port().name == port_name)
    }

    /// A section header: a disclosure chevron, the section's chip in its
    /// colour, the title in tracked small caps the way an explorer names
    /// its views, a count, an action.
    ///
    /// The whole strip is the disclosure target — a 12px chevron is a poor
    /// thing to ask anyone to hit.
    fn section_header(
        &mut self,
        section: PanelSection,
        title: &'static str,
        count: usize,
        action: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let collapsed = self.is_collapsed(section);
        let (glyph, category) = match section {
            PanelSection::Sessions => (Glyph::Bookmark, palette.category_session),
            PanelSection::Commands => (Glyph::Run, palette.category_command),
        };

        h_flex()
            .id(match section {
                PanelSection::Sessions => "sessions-header",
                PanelSection::Commands => "commands-header",
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_section(section, cx);
            }))
            .h(px(SECTION_HEADER_HEIGHT))
            .flex_none()
            .px_2()
            .pr_1p5()
            .gap_1()
            .justify_between()
            .cursor_pointer()
            .hover(|header| header.bg(rgb(palette.hover)))
            .child(
                h_flex()
                    .min_w_0()
                    .gap_1p5()
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(13.))
                        .text_color(rgb(palette.faint)),
                    )
                    .child(icon_chip(glyph, category, SECTION_CHIP))
                    .child(
                        div()
                            .truncate()
                            .text_token(EYEBROW)
                            .text_color(rgb(palette.strong_foreground))
                            .child(spaced_caps(title)),
                    )
                    .child(Self::count_pill(palette, count)),
            )
            .children(action)
            .into_any_element()
    }

    /// A count in a small grey pill, after a heading.
    fn count_pill(palette: WorkbenchPalette, count: usize) -> impl IntoElement {
        div()
            .flex_none()
            .px(px(6.))
            .py(px(1.))
            .rounded_full()
            .bg(tint(palette.muted, 0.14))
            .text_token(MICRO)
            .text_color(rgb(palette.muted))
            .child(count.to_string())
    }

    /// The prompt shown where a list has nothing in it yet.
    fn empty_hint(
        palette: WorkbenchPalette,
        glyph: Glyph,
        headline: &'static str,
        hint: &'static str,
    ) -> impl IntoElement {
        v_flex()
            .h(px(EMPTY_HINT_HEIGHT))
            .flex_none()
            .items_center()
            .justify_center()
            .px_4()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .size(px(34.))
                    .rounded_xl()
                    .bg(tint(palette.muted, 0.08))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(glyph)
                            .size(px(17.))
                            .text_color(tint(palette.muted, 0.7)),
                    ),
            )
            .child(
                div()
                    .text_token(CAPTION)
                    .text_color(rgb(palette.muted))
                    .child(headline),
            )
            .child(
                div()
                    .text_token(CAPTION)
                    .text_color(rgb(palette.faint))
                    .text_center()
                    .child(hint),
            )
    }

    /// The shell every list row shares: a raised card that lifts on hover and
    /// only brings its buttons forward once the pointer is on it.
    fn row_card(
        palette: WorkbenchPalette,
        id: impl Into<ElementId>,
        group: &'static str,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .group(group)
            .h(px(CARD_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .gap_2p5()
            .rounded_lg()
            .bg(rgb(palette.card))
            .border_1()
            .border_color(rgb(palette.border_subtle))
            .cursor_pointer()
            .hover(|row| {
                row.bg(rgb(palette.hover))
                    .border_color(tint(palette.accent, 0.45))
            })
    }

    /// A row's buttons: faint at rest, full once the row is under the
    /// pointer. Each stops its click short of the row, so pressing one
    /// never also does what the row does.
    fn row_actions(group: &'static str, buttons: Vec<Button>) -> impl IntoElement {
        h_flex()
            .flex_none()
            .gap_0p5()
            .opacity(ROW_ACTION_REST)
            .group_hover(group, |actions| actions.opacity(1.))
            .children(buttons)
    }

    fn row_action(
        id: impl Into<ElementId>,
        glyph: Glyph,
        tooltip: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Button {
        Button::new(id)
            .ghost()
            .with_size(px(ROW_ACTION_SIZE))
            .icon(glyph)
            .tooltip(tooltip)
            .on_click(move |event, window, cx| {
                cx.stop_propagation();
                on_click(event, window, cx);
            })
    }

    /// One saved session. A single click picks it out; a double-click opens
    /// it; the pencil edits it and the bin forgets it. A named session shows
    /// its name, and keeps the port on the line below beside the rate.
    fn render_session_card(
        &mut self,
        saved: &StoredSession,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let saved_id = saved.id;
        let is_open = self.port_is_open(&saved.port_name);
        let selected = self.selected_saved == Some(saved_id);
        let summary = saved.configuration.summary();
        let (title, detail) = match &saved.alias {
            Some(alias) => (alias.clone(), format!("{} · {summary}", saved.port_name)),
            None => (saved.port_name.clone(), summary),
        };

        Self::row_card(
            palette,
            ("saved-session", saved_id as usize),
            "saved-session",
        )
        .when(selected, |card| {
            card.bg(tint(palette.accent, SELECTED_PLATE))
                .border_color(tint(palette.accent, SELECTED_RING))
        })
        .tooltip(|window, cx| Tooltip::new("Double-click to open").build(window, cx))
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if event.click_count() >= 2 {
                this.open_saved_session(saved_id, window, cx);
            } else {
                this.select_saved_session(saved_id, cx);
            }
        }))
        .child(icon_chip(Glyph::Bookmark, palette.tag(saved.color), 28.))
        .child(
            v_flex()
                .min_w_0()
                .flex_1()
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_1p5()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_token(LABEL)
                                .text_color(rgb(palette.strong_foreground))
                                .child(title),
                        )
                        .when(is_open, |row| {
                            row.child(Self::status_dot(5., palette.success))
                        }),
                )
                .child(
                    div()
                        .truncate()
                        .ui_mono_token(MONO_SMALL)
                        .text_color(rgb(palette.faint))
                        .child(detail),
                ),
        )
        .child(Self::row_actions(
            "saved-session",
            vec![
                Self::row_action(
                    ("edit-session", saved_id as usize),
                    Glyph::Pencil,
                    "Edit this session",
                    cx.listener(move |this, _, window, cx| {
                        this.open_saved_session_editor(saved_id, window, cx);
                    }),
                ),
                Self::row_action(
                    ("remove-session", saved_id as usize),
                    Glyph::Trash,
                    "Forget this session",
                    cx.listener(move |this, _, _, cx| {
                        this.remove_saved_session(saved_id, cx);
                    }),
                ),
            ],
        ))
        .into_any_element()
    }

    /// A group's row: a chevron, a folder, its name, how many it holds, and
    /// the buttons to rename and remove it. The row folds its cards away,
    /// like a directory in an explorer.
    fn render_group_row(
        &mut self,
        group: &StoredGroup,
        count: usize,
        folded: bool,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group_id = group.id;

        h_flex()
            .id(("saved-group", group_id as usize))
            .group("saved-group")
            .h(px(GROUP_ROW_HEIGHT))
            .flex_none()
            .pl_1p5()
            .pr_1()
            .gap_1p5()
            .items_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|row| row.bg(rgb(palette.hover)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_group(group_id, cx);
            }))
            .child(
                Icon::new(if folded {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(px(12.))
                .text_color(rgb(palette.faint)),
            )
            .child(
                Icon::new(Glyph::Folder)
                    .size(px(15.))
                    .text_color(rgb(palette.category_session)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_token(LABEL)
                    .text_color(rgb(palette.strong_foreground))
                    .child(group.name.clone()),
            )
            .child(Self::count_pill(palette, count))
            .child(Self::row_actions(
                "saved-group",
                vec![
                    Self::row_action(
                        ("rename-group", group_id as usize),
                        Glyph::Pencil,
                        "Rename this group",
                        cx.listener(move |this, _, window, cx| {
                            this.open_group_prompt(
                                GroupPrompt::Rename(group_id),
                                |_, _, _| {},
                                window,
                                cx,
                            );
                        }),
                    ),
                    Self::row_action(
                        ("remove-group", group_id as usize),
                        Glyph::Trash,
                        "Remove this group · its sessions are kept",
                        cx.listener(move |this, _, _, cx| {
                            this.remove_group(group_id, cx);
                        }),
                    ),
                ],
            ))
            .into_any_element()
    }

    /// The cards of one group, set in from its row behind a guide line that
    /// hangs from the chevron, so the eye can follow the group down.
    fn render_group_body(
        &mut self,
        group_id: u64,
        members: &[StoredSession],
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cards = members
            .iter()
            .map(|saved| self.render_session_card(saved, palette, cx))
            .collect::<Vec<_>>();

        v_flex()
            .id(("saved-group-body", group_id as usize))
            .flex_none()
            .ml(px(GROUP_GUIDE_INSET))
            .pl(px(GROUP_BODY_GAP))
            .border_l_1()
            .border_color(rgb(palette.border))
            .gap_1p5()
            .when(cards.is_empty(), |body| {
                body.child(
                    div()
                        .h(px(GROUP_EMPTY_HEIGHT))
                        .flex()
                        .items_center()
                        .px_2()
                        .text_token(CAPTION)
                        .text_color(rgb(palette.faint))
                        .child("Empty · pick this group in a session's dialog"),
                )
            })
            .children(cards)
            .into_any_element()
    }

    fn render_saved_sessions(
        &mut self,
        has_active_tab: bool,
        panel_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let collapsed = self.sessions_collapsed;
        let total = self.presets.sessions.len();
        let groups = self.presets.groups.clone();

        let header = self.section_header(
            PanelSection::Sessions,
            "Sessions",
            total,
            Some(
                h_flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        Button::new("new-session-group")
                            .ghost()
                            .with_size(px(ROW_ACTION_SIZE))
                            .icon(Glyph::FolderPlus)
                            .tooltip("New group")
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.open_group_prompt(GroupPrompt::New, |_, _, _| {}, window, cx);
                            })),
                    )
                    .child(
                        Button::new("save-active-session")
                            .ghost()
                            .with_size(px(ROW_ACTION_SIZE))
                            .icon(IconName::Plus)
                            .tooltip_with_action("Save the active session", &SaveCurrentSession, None)
                            .disabled(!has_active_tab)
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.save_active_session(cx);
                            })),
                    )
                    .into_any_element(),
            ),
            cx,
        );

        // Groups first, each over its cards, then the sessions in none. The
        // rows' heights are summed as they are made, for the list's own.
        let mut rows = Vec::new();
        let mut content = 0.;
        for group in &groups {
            let members = self
                .presets
                .sessions_in(Some(group.id))
                .cloned()
                .collect::<Vec<_>>();
            let folded = self.collapsed_groups.contains(&group.id);
            rows.push(self.render_group_row(group, members.len(), folded, palette, cx));
            content += GROUP_ROW_HEIGHT;
            if !folded {
                rows.push(self.render_group_body(group.id, &members, palette, cx));
                content += match members.len() {
                    0 => GROUP_EMPTY_HEIGHT,
                    count => count as f32 * CARD_HEIGHT + (count - 1) as f32 * LIST_GAP,
                };
            }
        }
        let loose = self.presets.sessions_in(None).cloned().collect::<Vec<_>>();
        for saved in &loose {
            rows.push(self.render_session_card(saved, palette, cx));
            content += CARD_HEIGHT;
        }
        let content = if rows.is_empty() {
            EMPTY_HINT_HEIGHT
        } else {
            content + (rows.len() - 1) as f32 * LIST_GAP
        } + LIST_PAD_BOTTOM;

        // The list is as tall as its rows, up to what the panel can spare:
        // half of it at least, and all that Quick send does not need for its
        // own rows. Past that it scrolls. With Quick send folded it takes
        // the rest of the panel instead, through the flex chain down from
        // the panel.
        let fills = !collapsed && self.commands_collapsed;
        let commands = self.presets.commands.len();
        let quick_send_need = SECTION_HEADER_HEIGHT
            + match commands {
                0 => EMPTY_HINT_HEIGHT,
                count => {
                    SEARCH_ROW_HEIGHT + count as f32 * CARD_HEIGHT + (count - 1) as f32 * LIST_GAP
                }
            }
            + LIST_PAD_BOTTOM;
        let cap = (panel_height * SESSIONS_SHARE)
            .max(panel_height - quick_send_need)
            .round()
            - SECTION_HEADER_HEIGHT;
        let list_height = content.min(cap).max(0.);
        v_flex()
            .min_h_0()
            .when(fills, |section| section.flex_1())
            .when(!fills, |section| section.flex_none())
            .child(header)
            .when(!collapsed, |section| {
                section.child(
                    v_flex()
                        .when(fills, |list| list.flex_1().min_h_0())
                        .when(!fills, |list| list.flex_none().h(px(list_height)))
                        .px_2()
                        .pb_2()
                        .gap_1p5()
                        .overflow_y_scrollbar()
                        .when(rows.is_empty(), |list| {
                            list.child(Self::empty_hint(
                                palette,
                                Glyph::Bookmark,
                                "No saved sessions yet",
                                "Save the active session with +, or start a group with the folder.",
                            ))
                        })
                        .children(rows),
                )
            })
            .into_any_element()
    }

    /// Whether a saved command answers to the search box: its name or its
    /// text holds the query, case folded.
    fn command_matches(command: &StoredCommand, query: &str) -> bool {
        query.is_empty()
            || command.label.to_lowercase().contains(query)
            || command.command.to_lowercase().contains(query)
    }

    fn render_quick_send(&mut self, has_active_tab: bool, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let collapsed = self.commands_collapsed;
        let query = self.command_query.clone();
        let searching = !query.is_empty();
        let total = self.presets.commands.len();
        let saved_commands = self
            .presets
            .commands
            .iter()
            .filter(|command| Self::command_matches(command, &query))
            .cloned()
            .collect::<Vec<_>>();
        let workspace = cx.weak_entity();

        // While a search is on, the count is of what it found.
        let header = self.section_header(
            PanelSection::Commands,
            "Quick send",
            if searching { saved_commands.len() } else { total },
            None,
            cx,
        );
        // The box sits over the list once there is something to search.
        let search = (total > 0).then(|| {
            div().flex_none().px_2().pb_2().child(
                Input::new(&self.command_search)
                    .small()
                    .h(px(SEARCH_HEIGHT))
                    .text_token(CAPTION)
                    .rounded(px(SEARCH_HEIGHT / 2.))
                    .prefix(
                        Icon::new(IconName::Search)
                            .size(px(13.))
                            .text_color(rgb(palette.muted)),
                    )
                    .cleanable(true),
            )
        });

        v_flex()
            .when(!collapsed, |section| section.flex_1().min_h_0())
            .when(collapsed, |section| section.flex_none())
            .border_t_1()
            .border_color(rgb(palette.border))
            .child(header)
            .when(!collapsed, |section| {
                section.children(search).child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .px_2()
                        .pb_2()
                        .gap_1p5()
                        .overflow_y_scrollbar()
                        .when(total == 0, |list| {
                            list.child(Self::empty_hint(
                                palette,
                                Glyph::Run,
                                "No saved commands yet",
                                "Type a command below, then save it with the bookmark.",
                            ))
                        })
                        .when(total > 0 && saved_commands.is_empty(), |list| {
                            list.child(
                                div()
                                    .h(px(CARD_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_token(CAPTION)
                                    .text_color(rgb(palette.faint))
                                    .child("No command matches the search"),
                            )
                        })
                        .children(saved_commands.into_iter().map(|saved| {
                            let send_workspace = workspace.clone();
                            let remove_workspace = workspace.clone();
                            let command_id = saved.id;

                            Self::row_card(
                                palette,
                                ("saved-command", command_id as usize),
                                "saved-command",
                            )
                            // Without a session there is nowhere to send, so the
                            // row states that rather than swallowing the click.
                            .when(!has_active_tab, |row| row.cursor_default().opacity(0.55))
                            .when(has_active_tab, |row| {
                                row.on_click(move |_, window, cx| {
                                    let _ = send_workspace.update(cx, |this, cx| {
                                        this.send_saved_command(command_id, window, cx);
                                    });
                                })
                            })
                            .child(icon_chip(Glyph::Run, palette.category_command, 28.))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_token(LABEL)
                                            .text_color(rgb(palette.strong_foreground))
                                            .child(saved.label),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .ui_mono_token(MONO_SMALL)
                                            .text_color(rgb(palette.faint))
                                            .child(saved.command),
                                    ),
                            )
                            .child(Self::row_actions(
                                "saved-command",
                                vec![Self::row_action(
                                    ("remove-command", command_id as usize),
                                    Glyph::Trash,
                                    "Forget this command",
                                    move |_, _, cx| {
                                        let _ = remove_workspace.update(cx, |this, cx| {
                                            this.remove_saved_command(command_id, cx);
                                        });
                                    },
                                )],
                            ))
                        })),
                )
            })
            .into_any_element()
    }

    /// The UTF-8 / HEX switch: the same segmented rail the session dialog
    /// sets its framing with, so the two places a mode is picked look alike.
    fn render_mode_switch(&mut self, hex_mode: bool, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        segmented(
            "mode-switch",
            palette,
            ChoiceText::ui(MICRO),
            vec![
                Choice::new(
                    "UTF-8",
                    !hex_mode,
                    cx.listener(|this, _, _, cx| this.set_hex_mode(false, cx)),
                ),
                Choice::new(
                    "HEX",
                    hex_mode,
                    cx.listener(|this, _, _, cx| this.set_hex_mode(true, cx)),
                ),
            ],
        )
        .into_any_element()
    }

    /// The composer: the box on top, and under it the mode switch, where
    /// the line will go, and the two buttons — bookmark and send. Without a
    /// tab the box and the send button are put to rest; the bookmark still
    /// works, since a command can be kept before there is anywhere to send it.
    fn render_composer(
        &mut self,
        active: Option<&SerialTabSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let hex_mode = active.is_some_and(|tab| tab.hex_mode);
        let (target, status) = match self.active_tab() {
            Some(tab) => (
                tab.title().to_string(),
                if tab.connected {
                    palette.success
                } else if tab.connecting {
                    palette.warning
                } else {
                    palette.faint
                },
            ),
            None => ("No session open".to_string(), palette.faint),
        };
        let mode_switch = self.render_mode_switch(hex_mode, cx);

        v_flex()
            .h(px(COMPOSER_HEIGHT))
            .flex_none()
            .px_2()
            .py_2()
            .gap_1p5()
            .border_t_1()
            .border_color(rgb(palette.border))
            .child(
                Input::new(&self.send_input)
                    .h(px(COMPOSER_INPUT_HEIGHT))
                    .disabled(active.is_none())
                    .prefix(
                        Icon::new(if hex_mode {
                            Glyph::Hex
                        } else {
                            Glyph::Terminal
                        })
                        .size(px(15.))
                        .text_color(rgb(palette.muted)),
                    )
                    .cleanable(true),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(mode_switch)
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_1p5()
                            .child(Self::status_dot(6., status))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_token(CAPTION)
                                    .text_color(rgb(palette.muted))
                                    .child(target),
                            ),
                    )
                    .child(
                        Button::new("save-command")
                            .ghost()
                            .small()
                            .icon(Glyph::Bookmark)
                            .tooltip("Save this command to Quick send")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_current_command(cx);
                            })),
                    )
                    .child(
                        Button::new("send-command")
                            .primary()
                            .small()
                            .icon(Glyph::Send)
                            .disabled(active.is_none())
                            .tooltip("Send to the session in front")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.send_to_active_tab(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    /// One rail button: the section's glyph, with its count riding the corner.
    fn rail_button(
        &mut self,
        section: PanelSection,
        glyph: Glyph,
        category: u32,
        tooltip: &'static str,
        count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();

        div()
            .id(match section {
                PanelSection::Sessions => "rail-sessions",
                PanelSection::Commands => "rail-commands",
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.reveal_section(section, cx);
            }))
            .relative()
            .p(px(4.))
            .rounded_lg()
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .hover(|button| button.bg(rgb(palette.hover)))
            .child(icon_chip(glyph, category, 30.))
            .when(count > 0, |button| {
                button.child(
                    div()
                        .absolute()
                        .top(px(-1.))
                        .right(px(-1.))
                        .min_w(px(15.))
                        .h(px(15.))
                        .px(px(3.))
                        .rounded_full()
                        .bg(rgb(palette.panel))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .min_w(px(13.))
                                .h(px(13.))
                                .rounded_full()
                                .bg(rgb(category))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_token(MICRO)
                                .text_color(rgb(palette.editor))
                                .child(count.to_string()),
                        ),
                )
            })
            .into_any_element()
    }

    /// The collapsed panel: the two libraries as icons, and the way back out.
    fn render_rail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let sessions = self.presets.sessions.len();
        let commands = self.presets.commands.len();

        let sessions_button = self.rail_button(
            PanelSection::Sessions,
            Glyph::Bookmark,
            palette.category_session,
            "Saved sessions",
            sessions,
            cx,
        );
        let commands_button = self.rail_button(
            PanelSection::Commands,
            Glyph::Run,
            palette.category_command,
            "Quick send",
            commands,
            cx,
        );

        v_flex()
            .w(px(RAIL_WIDTH))
            .h_full()
            .flex_none()
            .items_center()
            .gap_2()
            .border_l_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.panel))
            .pt_2()
            .child(sessions_button)
            .child(commands_button)
            .into_any_element()
    }

    /// The panel, given its height so the saved sessions can take a share
    /// of it.
    pub(crate) fn render_right_sidebar(
        &mut self,
        active_tab: Option<SerialTabSnapshot>,
        panel_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.side_panel_collapsed {
            return self.render_rail(cx);
        }

        let palette = self.interface_theme.palette();
        let has_active_tab = active_tab.is_some();
        // The composer has the foot of the panel; the lists share the rest.
        let sessions =
            self.render_saved_sessions(has_active_tab, panel_height - COMPOSER_HEIGHT, cx);
        let commands = self.render_quick_send(has_active_tab, cx);
        let composer = self.render_composer(active_tab.as_ref(), cx);

        // The width is the resizable panel's to set; the column fills it.
        v_flex()
            .size_full()
            .min_w_0()
            .border_l_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.panel))
            .child(sessions)
            .child(commands)
            .child(composer)
            .into_any_element()
    }
}
