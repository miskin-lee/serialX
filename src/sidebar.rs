//! The right panel: the library of things you keep between sessions.
//!
//! Two stacked sections, each a header over a scrolling list of cards. Rows are
//! cards rather than table lines so a saved item reads as an object you can act
//! on, and every one is anchored by a coloured glyph in the manner of VS Code's
//! Material Icon Theme: a command is a green prompt, and a session wears its
//! own tag colour, so the card and the tab it opens match.
//!
//! Both lists file under groups — folders, each a row with a chevron over
//! its cards, the way an explorer shows a directory — with the cards in no
//! group at the top level beneath them. A session card opens on a
//! double-click, as a session does in any session manager; a single click
//! only picks it out, and its two buttons edit it and forget it, so nothing
//! happens by a slip of the pointer. A command card sends on a click, since
//! sending is what it is for, and its two buttons edit and forget it too.
//! A group comes from a right-click, on a section's header or anywhere in
//! its list; the session in front is saved from the Session menu, and a
//! command from the composer's bookmark, so the headers carry no buttons of
//! their own. Each list has a search box over it once it holds anything,
//! with the title bar filter's `Aa` switch for a search that minds case.
//!
//! Two things fold here, at different grains: a section collapses to its own
//! header, and the whole panel collapses to an icon rail that still carries the
//! counts and reopens on a click. A collapsed section gives its height back to
//! its neighbour, so the panel never holds a half-empty list above a full one.
//!
//! Along the panel's foot sits the composer: the one box commands are typed
//! into, under the commands kept for reuse, so a line and the bookmark that
//! saves it are a hand's width apart. It is built the way a chat composer
//! is — Slack's, Linear's — one card with the text on top and a rail of
//! switches under it: the encoding, UTF-8 or HEX, and what follows the
//! line, `CRLF`, `LF` or nothing, with the send button at the rail's end.
//! The bookmark that keeps the line sits at the end of the box itself, as
//! a browser's star sits at the end of its address bar. Over the card a
//! line names the tab it sends to, the way a mail's `To` does, with the
//! tab's connection dot.

use std::rc::Rc;

use gpui_kit::base::{ResizeHandleContext, ResizeHandleRenderer};
use gpui_kit::component::{
    Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::commands::CommandTarget;
use crate::controls::{Choice, ChoiceText, segmented, spaced_caps, tag};
use crate::groups::GroupPrompt;
use crate::icons::{Glyph, icon_chip};
use crate::presets::{Library, StoredCommand, StoredGroup, StoredSession};
use crate::theme::{
    CAPTION, EYEBROW, LABEL, MICRO, MONO_SMALL, TagColor, Typography, WorkbenchPalette, tint,
};
use crate::{
    HexError, LineEnding, SerialConfiguration, SerialTabSnapshot, SerialWorkspace, parse_hex,
};

/// Width the panel opens at. Wide enough for a port path plus its baud
/// summary on one line at the caption size; the edge drags from there.
pub(crate) const SIDEBAR_WIDTH: f32 = 296.;
/// The narrowest the panel drags to: one card with its glyph and a name,
/// and the composer's two switches with the send button beside them.
pub(crate) const SIDEBAR_MIN_WIDTH: f32 = 240.;
/// The widest: past this the cards are mostly air.
pub(crate) const SIDEBAR_MAX_WIDTH: f32 = 560.;
/// Width of the line the panel's edge lights up to under the pointer: wide
/// enough to read as something held, narrow enough to stay a seam.
const DIVIDER_LIT_WIDTH: f32 = 3.;
/// Width of the collapsed rail: one icon chip plus breathing room.
const RAIL_WIDTH: f32 = 52.;
/// The chip beside a section's title, in the section's colour.
const SECTION_CHIP: f32 = 20.;
/// The search box over a list — the height of the title bar's filter box,
/// the same kind of thing — and the row it sits in with the air under it.
const SEARCH_HEIGHT: f32 = 28.;
const SEARCH_ROW_HEIGHT: f32 = 36.;
/// What the search boxes say while they are empty.
const SESSION_SEARCH_PLACEHOLDER: &str = "Search sessions";
const COMMAND_SEARCH_PLACEHOLDER: &str = "Search commands";
/// Height of the composer at the panel's foot: the line naming where a
/// command goes, the card under it, and the air around them.
const COMPOSER_HEIGHT: f32 = 104.;
/// Height of the line over the card.
const COMPOSER_TARGET_HEIGHT: f32 = 20.;
/// Height of the box commands are typed into, at the top of the card.
const COMPOSER_INPUT_HEIGHT: f32 = 34.;
/// Height of the rail of switches under it, and of each switch on it —
/// the segmented rail and the ending's pill are the same height, so they
/// read as one family.
const COMPOSER_RAIL_HEIGHT: f32 = 32.;
const COMPOSER_SWITCH_HEIGHT: f32 = 24.;
/// Diameter of the two discs at the card's right edge — the bookmark at
/// the box's end and the send button under it — as every chat composer's
/// send is. One size, so the pair reads as a pair: the quiet one keeps,
/// the accent one sends.
const ACTION_BUTTON_SIZE: f32 = 26.;
/// The glyph in each: the plane is wide, the bookmark tall, so the
/// bookmark takes a little more to weigh the same.
const SEND_ICON_SIZE: f32 = 14.;
const BOOKMARK_ICON_SIZE: f32 = 16.;
/// The narrowest the ending's list opens: room for a name and its bytes.
const ENDING_LIST_MIN_WIDTH: f32 = 148.;
/// What the composer says while it is empty, in each encoding.
pub(crate) const SEND_PLACEHOLDER: &str = "Enter a command…";
const HEX_PLACEHOLDER: &str = "Hex bytes, e.g. 41 54 0D 0A";
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

/// What the divider between the log and the panel paints. At rest, nothing:
/// the panel's own border is the seam. Under the pointer, and for as long as
/// it is being dragged, an accent line straddles that border, so the edge
/// says it can be moved before the cursor does.
pub(crate) fn panel_divider(palette: WorkbenchPalette) -> ResizeHandleRenderer {
    Rc::new(
        move |handle: &ResizeHandleContext, _: &mut Window, _: &mut App| {
            let lit = rgb(palette.accent);
            Some(
                div()
                    .flex_none()
                    .h_full()
                    .w(px(DIVIDER_LIT_WIDTH))
                    // The handle's content box is the seam itself, zero wide;
                    // the line grows out of it to the right, and is pulled
                    // back so it sits astride the border instead.
                    .ml(px(-((DIVIDER_LIT_WIDTH - 1.) / 2.)))
                    .when(handle.is_active(), |line| line.bg(lit))
                    .group_hover("handle", move |line| line.bg(lit))
                    .into_any_element(),
            )
        },
    )
}

/// The search box over a list: what it says, and how that is read. The text
/// is kept as typed and folded at match time, unless the box's `Aa` switch
/// is on — the same switch, meaning the same, as in the title bar's filter.
pub(crate) struct ListSearch {
    pub(crate) input: Entity<InputState>,
    _subscription: Subscription,
    pub(crate) query: String,
    pub(crate) match_case: bool,
}

impl ListSearch {
    /// Makes the box for one library, and keeps `query` at what it says.
    pub(crate) fn new(
        library: Library,
        window: &mut Window,
        cx: &mut Context<SerialWorkspace>,
    ) -> Self {
        let placeholder = match library {
            Library::Sessions => SESSION_SEARCH_PLACEHOLDER,
            Library::Commands => COMMAND_SEARCH_PLACEHOLDER,
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .clean_on_escape()
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search_mut(library).query = input.read(cx).value().trim().to_string();
                    cx.notify();
                }
            },
        );
        Self {
            input,
            _subscription: subscription,
            query: String::new(),
            match_case: false,
        }
    }

    fn searching(&self) -> bool {
        !self.query.is_empty()
    }

    /// Whether `text` holds what the box says.
    fn matches(&self, text: &str) -> bool {
        contains_query(text, &self.query, self.match_case)
    }
}

/// Whether `text` holds `query`: as written when case matters, and with both
/// folded when it does not. An empty query is held by anything.
fn contains_query(text: &str, query: &str, match_case: bool) -> bool {
    if match_case {
        text.contains(query)
    } else {
        text.to_lowercase().contains(&query.to_lowercase())
    }
}

impl SerialWorkspace {
    fn search(&self, library: Library) -> &ListSearch {
        match library {
            Library::Sessions => &self.session_search,
            Library::Commands => &self.command_search,
        }
    }

    pub(crate) fn search_mut(&mut self, library: Library) -> &mut ListSearch {
        match library {
            Library::Sessions => &mut self.session_search,
            Library::Commands => &mut self.command_search,
        }
    }

    fn toggle_search_case(&mut self, library: Library, cx: &mut Context<Self>) {
        let search = self.search_mut(library);
        search.match_case = !search.match_case;
        cx.notify();
    }

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
        let group = self.presets.resolve_group(Library::Sessions, group);
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
        let scrollback = self.presets.settings.scrollback_lines;
        let mut tab = Self::build_tab(id, scrollback, window, cx);
        tab.configuration = saved.configuration.sanitized();
        tab.color = saved.color;
        tab.alias = saved.alias.clone();
        tab.group = self.presets.resolve_group(Library::Sessions, saved.group);
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

    /// Removes a group. What it held stays, at the top of its list.
    fn remove_group(&mut self, group_id: u64, cx: &mut Context<Self>) {
        self.presets.remove_group(group_id);
        self.collapsed_groups.remove(&group_id);
        cx.notify();
    }

    /// Opens the command dialog on what the composer holds, so the line
    /// can be kept under a name and in a group.
    pub(crate) fn save_current_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.send_input.read(cx).value().to_string();
        self.open_command_dialog(CommandTarget::New, &draft, window, cx);
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

    fn toggle_section(&mut self, section: Library, cx: &mut Context<Self>) {
        match section {
            Library::Sessions => self.sessions_collapsed = !self.sessions_collapsed,
            Library::Commands => self.commands_collapsed = !self.commands_collapsed,
        }
        cx.notify();
    }

    /// Expands the panel with one section showing, for the rail buttons.
    fn reveal_section(&mut self, section: Library, cx: &mut Context<Self>) {
        self.side_panel_collapsed = false;
        match section {
            Library::Sessions => self.sessions_collapsed = false,
            Library::Commands => self.commands_collapsed = false,
        }
        cx.notify();
    }

    fn is_collapsed(&self, section: Library) -> bool {
        match section {
            Library::Sessions => self.sessions_collapsed,
            Library::Commands => self.commands_collapsed,
        }
    }

    /// A library's glyph and colour: what its section chip, its rail
    /// button and its group folders are drawn in. Quick send takes the
    /// paper plane, for what the section does, and leaves the prompt to
    /// the cards, for what they are — a header that wore the cards' own
    /// glyph read as one more of them.
    fn library_mark(palette: WorkbenchPalette, library: Library) -> (Glyph, u32) {
        match library {
            Library::Sessions => (Glyph::Bookmark, palette.category_session),
            Library::Commands => (Glyph::Send, palette.category_command),
        }
    }

    /// True when a tab is currently pointed at this port, so a saved row can
    /// say "this one is already open" without opening anything to find out.
    fn port_is_open(&self, port_name: &str) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.selected_port().name == port_name)
    }

    /// What a right-click in a section offers: a new group, the one thing
    /// a section can be given that nothing else in the window makes. The
    /// one menu goes on the section's header and on its list, so a click
    /// on either, or on the space under the last card, finds it.
    fn library_menu(
        &self,
        library: Library,
        cx: &mut Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let workspace = cx.weak_entity();
        move |menu, _, _| {
            let workspace = workspace.clone();
            menu.item(
                PopupMenuItem::new("New group…")
                    .icon(Icon::new(Glyph::FolderPlus))
                    .on_click(move |_, window, cx| {
                        let _ = workspace.update(cx, |this, cx| {
                            this.open_group_prompt(
                                GroupPrompt::New(library),
                                |_, _, _| {},
                                window,
                                cx,
                            );
                        });
                    }),
            )
        }
    }

    /// The search box over a list: a pill with the glass, the text, and the
    /// `Aa` switch borrowed from the title bar's filter box, for a search
    /// that minds case. Escape and the cross clear it.
    fn render_search_box(&mut self, library: Library, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let search = self.search(library);
        let (input, match_case) = (search.input.clone(), search.match_case);
        let switch = Self::filter_switch(
            match library {
                Library::Sessions => "sessions-match-case",
                Library::Commands => "commands-match-case",
            },
            "Aa",
            match_case,
            "Match case",
            palette,
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_search_case(library, cx);
        }));
        let input = Input::new(&input)
            .small()
            .text_token(CAPTION)
            .rounded(px(SEARCH_HEIGHT / 2.))
            .prefix(
                Icon::new(IconName::Search)
                    .size(px(13.))
                    .text_color(rgb(palette.muted)),
            )
            .cleanable(true)
            .suffix(switch);
        div()
            .flex_none()
            .px_2()
            .pb_2()
            // `Input::h` is the multi-line box's own and shadows the style
            // height, so the style's is named in full.
            .child(Styled::h(input, px(SEARCH_HEIGHT)))
            .into_any_element()
    }

    /// A section header: a disclosure chevron, the section's chip in its
    /// colour, the title in tracked small caps the way an explorer names
    /// its views, a count. A right-click on it opens the section's menu.
    ///
    /// The whole strip is the disclosure target — a 12px chevron is a poor
    /// thing to ask anyone to hit.
    fn section_header(
        &mut self,
        section: Library,
        title: &'static str,
        count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let collapsed = self.is_collapsed(section);
        let (glyph, category) = Self::library_mark(palette, section);
        let menu = self.library_menu(section, cx);

        h_flex()
            .id(match section {
                Library::Sessions => "sessions-header",
                Library::Commands => "commands-header",
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_section(section, cx);
            }))
            .h(px(SECTION_HEADER_HEIGHT))
            .flex_none()
            .px_2()
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
            .context_menu(menu)
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

    /// A group's row: a chevron, a folder in its library's colour, its
    /// name, how many it holds, and the buttons to rename and remove it.
    /// The row folds its cards away, like a directory in an explorer.
    fn render_group_row(
        &mut self,
        group: &StoredGroup,
        count: usize,
        folded: bool,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group_id = group.id;
        let (_, hue) = Self::library_mark(palette, group.library);
        let remove_tooltip = match group.library {
            Library::Sessions => "Remove this group · its sessions are kept",
            Library::Commands => "Remove this group · its commands are kept",
        };

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
                    .text_color(rgb(hue)),
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
                        remove_tooltip,
                        cx.listener(move |this, _, _, cx| {
                            this.remove_group(group_id, cx);
                        }),
                    ),
                ],
            ))
            .into_any_element()
    }

    /// The cards of one group, set in from its row behind a guide line that
    /// hangs from the chevron, so the eye can follow the group down. An
    /// empty group shows `empty` in place of cards.
    fn render_group_body(
        group_id: u64,
        cards: Vec<AnyElement>,
        empty: &'static str,
        palette: WorkbenchPalette,
    ) -> AnyElement {
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
                        .child(empty),
                )
            })
            .children(cards)
            .into_any_element()
    }

    /// The height a group takes in its list: its row, and its body while
    /// it is unfolded.
    fn group_height(count: usize, folded: bool) -> f32 {
        GROUP_ROW_HEIGHT
            + if folded {
                0.
            } else {
                match count {
                    0 => GROUP_EMPTY_HEIGHT,
                    count => count as f32 * CARD_HEIGHT + (count - 1) as f32 * LIST_GAP,
                }
            }
    }

    /// Whether a saved session answers the search box: its name, its port
    /// or its rate and framing — what its card shows — holds the query.
    fn session_matches(saved: &StoredSession, search: &ListSearch) -> bool {
        search.matches(&saved.port_name)
            || saved
                .alias
                .as_deref()
                .is_some_and(|alias| search.matches(alias))
            || search.matches(&saved.configuration.summary())
    }

    /// The saved sessions: the header, the search box, and the cards, filed
    /// under their groups with the ones in none beneath. While a search is
    /// on, the cards that answer it are listed flat, groups set aside, and
    /// the count is of them. Given the panel's height and how much of it
    /// Quick send wants, so the list can take its share.
    fn render_saved_sessions(
        &mut self,
        panel_height: f32,
        quick_send_need: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let collapsed = self.sessions_collapsed;
        let total = self.presets.sessions.len();
        let searching = self.session_search.searching();

        let header = self.section_header(
            Library::Sessions,
            "Sessions",
            if searching {
                self.presets
                    .sessions
                    .iter()
                    .filter(|saved| Self::session_matches(saved, &self.session_search))
                    .count()
            } else {
                total
            },
            cx,
        );
        let menu = self.library_menu(Library::Sessions, cx);
        // The box sits over the list once there is something to search.
        let search = (total > 0).then(|| self.render_search_box(Library::Sessions, cx));

        // Groups first, each over its cards, then the sessions in none — or,
        // under a search, the cards that answer it. The rows' heights are
        // summed as they are made, for the list's own.
        let mut rows = Vec::new();
        let mut content = 0.;
        if searching {
            let matches = self
                .presets
                .sessions
                .iter()
                .filter(|saved| Self::session_matches(saved, &self.session_search))
                .cloned()
                .collect::<Vec<_>>();
            for saved in &matches {
                rows.push(self.render_session_card(saved, palette, cx));
                content += CARD_HEIGHT;
            }
        } else {
            let groups = self
                .presets
                .groups_in(Library::Sessions)
                .cloned()
                .collect::<Vec<_>>();
            for group in &groups {
                let members = self
                    .presets
                    .sessions_in(Some(group.id))
                    .cloned()
                    .collect::<Vec<_>>();
                let folded = self.collapsed_groups.contains(&group.id);
                rows.push(self.render_group_row(group, members.len(), folded, palette, cx));
                if !folded {
                    let cards = members
                        .iter()
                        .map(|saved| self.render_session_card(saved, palette, cx))
                        .collect();
                    rows.push(Self::render_group_body(
                        group.id,
                        cards,
                        "Empty · pick this group in a session's dialog",
                        palette,
                    ));
                }
                content += Self::group_height(members.len(), folded);
            }
            let loose = self.presets.sessions_in(None).cloned().collect::<Vec<_>>();
            for saved in &loose {
                rows.push(self.render_session_card(saved, palette, cx));
                content += CARD_HEIGHT;
            }
        }
        let no_match = searching && rows.is_empty();
        let empty = !searching && rows.is_empty();
        let content = if empty {
            EMPTY_HINT_HEIGHT
        } else if no_match {
            CARD_HEIGHT
        } else {
            content + rows.len().saturating_sub(1) as f32 * LIST_GAP
        } + LIST_PAD_BOTTOM;

        // The list is as tall as its rows, up to what the panel can spare:
        // half of it at least, and all that Quick send does not need for its
        // own rows, less the header and the search box. Past that it
        // scrolls. With Quick send folded it takes the rest of the panel
        // instead, through the flex chain down from the panel.
        let fills = !collapsed && self.commands_collapsed;
        let cap = (panel_height * SESSIONS_SHARE)
            .max(panel_height - quick_send_need)
            .round()
            - SECTION_HEADER_HEIGHT
            - if total > 0 { SEARCH_ROW_HEIGHT } else { 0. };
        let list_height = content.min(cap).max(0.);
        v_flex()
            .min_h_0()
            .when(fills, |section| section.flex_1())
            .when(!fills, |section| section.flex_none())
            .child(header)
            .when(!collapsed, |section| {
                section.children(search).child(
                    v_flex()
                        .when(fills, |list| list.flex_1().min_h_0())
                        .when(!fills, |list| list.flex_none().h(px(list_height)))
                        .px_2()
                        .pb_2()
                        .gap_1p5()
                        .overflow_y_scrollbar()
                        .when(empty, |list| {
                            list.child(Self::empty_hint(
                                palette,
                                Glyph::Bookmark,
                                "No saved sessions yet",
                                "Save the session in front from the Session menu, or right-click here for a group.",
                            ))
                        })
                        .when(no_match, |list| {
                            list.child(Self::no_match_hint(palette, "No session matches the search"))
                        })
                        .children(rows)
                        .context_menu(menu),
                )
            })
            .into_any_element()
    }

    /// The line a list shows when a search finds nothing.
    fn no_match_hint(palette: WorkbenchPalette, text: &'static str) -> impl IntoElement {
        div()
            .h(px(CARD_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .text_token(CAPTION)
            .text_color(rgb(palette.faint))
            .child(text)
    }

    /// Whether a saved command answers the search box: its name or its
    /// text holds the query.
    fn command_matches(command: &StoredCommand, search: &ListSearch) -> bool {
        search.matches(&command.label) || search.matches(&command.command)
    }

    /// One saved command. A click sends it to the session in front; the
    /// pencil edits it and the bin forgets it. A named command shows its
    /// name over the line it sends; one without goes by the line alone.
    fn render_command_card(
        &mut self,
        saved: &StoredCommand,
        has_active_tab: bool,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let command_id = saved.id;
        let alias = saved.alias().map(str::to_owned);
        let command = saved.command.clone();

        Self::row_card(
            palette,
            ("saved-command", command_id as usize),
            "saved-command",
        )
        // Without a session there is nowhere to send, so the row states
        // that rather than swallowing the click.
        .when(!has_active_tab, |row| row.cursor_default().opacity(0.55))
        .when(has_active_tab, |row| {
            row.tooltip(|window, cx| Tooltip::new("Click to send").build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.send_saved_command(command_id, window, cx);
                }))
        })
        .child(icon_chip(Glyph::Prompt, palette.category_command, 28.))
        .child(
            v_flex()
                .min_w_0()
                .flex_1()
                .when_some(alias, |body, alias| {
                    body.child(
                        div()
                            .truncate()
                            .text_token(LABEL)
                            .text_color(rgb(palette.strong_foreground))
                            .child(alias),
                    )
                    .child(
                        div()
                            .truncate()
                            .ui_mono_token(MONO_SMALL)
                            .text_color(rgb(palette.faint))
                            .child(command.clone()),
                    )
                })
                .when(saved.alias().is_none(), |body| {
                    body.child(
                        div()
                            .truncate()
                            .ui_mono_token(LABEL)
                            .text_color(rgb(palette.strong_foreground))
                            .child(command),
                    )
                }),
        )
        .child(Self::row_actions(
            "saved-command",
            vec![
                Self::row_action(
                    ("edit-command", command_id as usize),
                    Glyph::Pencil,
                    "Edit this command",
                    cx.listener(move |this, _, window, cx| {
                        this.open_command_dialog(
                            CommandTarget::Saved(command_id),
                            "",
                            window,
                            cx,
                        );
                    }),
                ),
                Self::row_action(
                    ("remove-command", command_id as usize),
                    Glyph::Trash,
                    "Forget this command",
                    cx.listener(move |this, _, _, cx| {
                        this.remove_saved_command(command_id, cx);
                    }),
                ),
            ],
        ))
        .into_any_element()
    }

    /// Quick send: the header, the search box, and the cards, filed under
    /// their groups with the ones in none beneath. While a search is on,
    /// the cards that answer it are listed flat, groups set aside, and the
    /// count is of them. Returns the section and the height its rows want,
    /// for the saved sessions to leave it.
    fn render_quick_send(
        &mut self,
        has_active_tab: bool,
        cx: &mut Context<Self>,
    ) -> (AnyElement, f32) {
        let palette = self.interface_theme.palette();
        let collapsed = self.commands_collapsed;
        let searching = self.command_search.searching();
        let total = self.presets.commands.len();

        let header = self.section_header(
            Library::Commands,
            "Quick send",
            if searching {
                self.presets
                    .commands
                    .iter()
                    .filter(|command| Self::command_matches(command, &self.command_search))
                    .count()
            } else {
                total
            },
            cx,
        );
        let menu = self.library_menu(Library::Commands, cx);
        // The box sits over the list once there is something to search.
        let search = (total > 0).then(|| self.render_search_box(Library::Commands, cx));

        // The rows' heights are summed as they are made, for the sessions
        // list to know how much of the panel to leave.
        let mut rows = Vec::new();
        let mut content = 0.;
        if searching {
            let matches = self
                .presets
                .commands
                .iter()
                .filter(|command| Self::command_matches(command, &self.command_search))
                .cloned()
                .collect::<Vec<_>>();
            for saved in &matches {
                rows.push(self.render_command_card(saved, has_active_tab, palette, cx));
                content += CARD_HEIGHT;
            }
        } else {
            let groups = self
                .presets
                .groups_in(Library::Commands)
                .cloned()
                .collect::<Vec<_>>();
            for group in &groups {
                let members = self
                    .presets
                    .commands_in(Some(group.id))
                    .cloned()
                    .collect::<Vec<_>>();
                let folded = self.collapsed_groups.contains(&group.id);
                rows.push(self.render_group_row(group, members.len(), folded, palette, cx));
                if !folded {
                    let cards = members
                        .iter()
                        .map(|saved| self.render_command_card(saved, has_active_tab, palette, cx))
                        .collect();
                    rows.push(Self::render_group_body(
                        group.id,
                        cards,
                        "Empty · pick this group as you save a command",
                        palette,
                    ));
                }
                content += Self::group_height(members.len(), folded);
            }
            let loose = self.presets.commands_in(None).cloned().collect::<Vec<_>>();
            for saved in &loose {
                rows.push(self.render_command_card(saved, has_active_tab, palette, cx));
                content += CARD_HEIGHT;
            }
        }
        let no_match = searching && rows.is_empty();
        let empty = !searching && rows.is_empty();
        let content = if empty {
            EMPTY_HINT_HEIGHT
        } else if no_match {
            CARD_HEIGHT
        } else {
            content + rows.len().saturating_sub(1) as f32 * LIST_GAP
        };
        let need = SECTION_HEADER_HEIGHT
            + if total > 0 { SEARCH_ROW_HEIGHT } else { 0. }
            + content
            + LIST_PAD_BOTTOM;

        let section = v_flex()
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
                        .when(empty, |list| {
                            list.child(Self::empty_hint(
                                palette,
                                Glyph::Prompt,
                                "No saved commands yet",
                                "Type a command below and keep it with the bookmark, or right-click here for a group.",
                            ))
                        })
                        .when(no_match, |list| {
                            list.child(Self::no_match_hint(palette, "No command matches the search"))
                        })
                        .children(rows)
                        .context_menu(menu),
                )
            })
            .into_any_element();
        (section, need)
    }

    /// The UTF-8 / HEX switch: the same segmented rail the session dialog
    /// sets its framing with, so the two places a mode is picked look alike.
    fn render_mode_switch(&mut self, hex_mode: bool, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        div()
            .flex_none()
            .h(px(COMPOSER_SWITCH_HEIGHT))
            .child(segmented(
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
            ))
            .into_any_element()
    }

    /// The line-ending switch: a pill on the same rail as the mode switch,
    /// a return glyph, the ending's name and a caret, opening the list of
    /// the three above it — the way Arduino's monitor and VS Code's serial
    /// monitor put the ending beside the box. Each row names the ending
    /// and spells its bytes, so `CRLF` and `\r\n` are read together.
    fn render_ending_switch(&mut self, ending: LineEnding, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let workspace = cx.weak_entity();

        Button::new("line-ending")
            .ghost()
            .compact()
            .h(px(COMPOSER_SWITCH_HEIGHT))
            .px(px(7.))
            .bg(rgb(palette.surface))
            .border_1()
            .border_color(rgb(palette.border_subtle))
            .rounded(px(8.))
            .tab_stop(false)
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_token(MICRO)
                            .text_color(rgb(palette.faint))
                            .child("↵"),
                    )
                    .child(
                        div()
                            .text_token(MICRO)
                            .text_color(rgb(palette.strong_foreground))
                            .whitespace_nowrap()
                            .child(ending.label()),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(px(10.))
                            .text_color(rgb(palette.muted)),
                    ),
            )
            .tooltip("What follows the line")
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
                menu = menu.min_w(px(ENDING_LIST_MIN_WIDTH));
                for choice in LineEnding::ALL {
                    let workspace = workspace.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            h_flex()
                                .w_full()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .child(choice.label())
                                .when(!choice.spelled().is_empty(), |row| {
                                    row.child(
                                        div()
                                            .ui_mono_font()
                                            .text_token(MONO_SMALL)
                                            .text_color(rgb(palette.muted))
                                            .child(choice.spelled()),
                                    )
                                })
                        })
                        .checked(choice == ending)
                        .on_click(move |_, _, cx| {
                            let _ = workspace.update(cx, |this, cx| {
                                this.set_line_ending(choice, cx);
                            });
                        }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// The composer: a line naming the tab it sends to, and under it one
    /// card — the box on top, with the bookmark at its end, and along the
    /// card's foot the rail of switches, the encoding and the line ending,
    /// with the send disc at its end. The card carries the frame the box
    /// would: the accent while the box has focus, red while a line in HEX
    /// is not hex, with a `Not hex` tag on the line above saying so.
    /// Without a tab the box and the send button are put to rest; the
    /// bookmark still works, since a command can be kept before there is
    /// anywhere to send it.
    fn render_composer(
        &mut self,
        active: Option<&SerialTabSnapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let hex_mode = active.is_some_and(|tab| tab.hex_mode);
        let ending = active.map_or(LineEnding::default(), |tab| tab.line_ending);
        let (target, status) = match self.active_tab() {
            Some(tab) => (
                Some(tab.title().to_string()),
                if tab.connected {
                    palette.success
                } else if tab.connecting {
                    palette.warning
                } else {
                    palette.faint
                },
            ),
            None => (None, palette.faint),
        };

        // The box says what it takes, in the encoding it is in. The state
        // keeps the placeholder, so it is put right here, where the mode
        // is known, whenever it has drifted.
        let placeholder = if hex_mode {
            HEX_PLACEHOLDER
        } else {
            SEND_PLACEHOLDER
        };
        if self.send_input.read(cx).presentation().placeholder().as_ref() != placeholder {
            self.send_input
                .update(cx, |input, cx| input.set_placeholder(placeholder, window, cx));
        }
        let focused = self.send_input.read(cx).focus_handle(cx).is_focused(window);
        // A digit short of a byte is what every other keystroke leaves, so
        // only a character that can never be hex reddens the frame; the
        // odd digit is caught on send.
        let not_hex = hex_mode
            && matches!(
                parse_hex(self.send_input.read(cx).value().as_ref()),
                Err(HexError::NotHex)
            );
        let ring = if not_hex {
            palette.danger
        } else if focused {
            palette.accent
        } else {
            palette.input_border
        };

        let mode_switch = self.render_mode_switch(hex_mode, cx);
        let ending_switch = self.render_ending_switch(ending, cx);

        let target_line = h_flex()
            .h(px(COMPOSER_TARGET_HEIGHT))
            .flex_none()
            .items_center()
            .gap_1p5()
            .px_1()
            .when(target.is_some(), |line| {
                line.child(
                    div()
                        .text_token(CAPTION)
                        .text_color(rgb(palette.faint))
                        .child("To"),
                )
            })
            .child(Self::status_dot(6., status))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_token(CAPTION)
                    .text_color(rgb(if target.is_some() {
                        palette.foreground
                    } else {
                        palette.muted
                    }))
                    .child(target.unwrap_or_else(|| "No session open".to_string())),
            )
            .child(div().flex_1())
            .when(not_hex, |line| {
                line.child(tag(palette, palette.danger, MICRO, "Not hex"))
            });

        let input = Styled::h(
            Input::new(&self.send_input)
                .appearance(false)
                .focus_bordered(false)
                .px_2p5(),
            px(COMPOSER_INPUT_HEIGHT),
        )
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
        .when(hex_mode, |input| input.ui_mono_font())
        .cleanable(true)
        .suffix(
            Button::new("save-command")
                .ghost()
                .with_size(px(ACTION_BUTTON_SIZE))
                .rounded(px(ACTION_BUTTON_SIZE / 2.))
                .bg(rgb(palette.surface))
                .border_1()
                .border_color(rgb(palette.border_subtle))
                .tab_stop(false)
                .icon(Icon::new(Glyph::Bookmark).size(px(BOOKMARK_ICON_SIZE)))
                .tooltip("Save this command to Quick send")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.save_current_command(window, cx);
                })),
        );

        // The rail keeps the box's side padding, so the send disc hangs
        // under the bookmark and the switches start under the glyph.
        let rail = h_flex()
            .h(px(COMPOSER_RAIL_HEIGHT))
            .flex_none()
            .items_center()
            .gap_1()
            .px_2p5()
            .pb_1p5()
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .when(active.is_none(), |switches| switches.opacity(0.5))
                    .child(mode_switch)
                    .child(ending_switch),
            )
            .child(div().flex_1())
            .child(
                Button::new("send-command")
                    .primary()
                    .with_size(px(ACTION_BUTTON_SIZE))
                    .rounded(px(ACTION_BUTTON_SIZE / 2.))
                    .icon(Icon::new(Glyph::Send).size(px(SEND_ICON_SIZE)))
                    .disabled(active.is_none())
                    .tooltip("Send to the session in front")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.send_to_active_tab(window, cx);
                    })),
            );

        let card = v_flex()
            .flex_none()
            .rounded(px(10.))
            .bg(rgb(palette.input))
            .border_1()
            .border_color(rgb(ring))
            .child(input)
            .child(rail);

        v_flex()
            .h(px(COMPOSER_HEIGHT))
            .flex_none()
            .px_2()
            .pt_1p5()
            .pb_2()
            .gap_1()
            .border_t_1()
            .border_color(rgb(palette.border))
            .child(target_line)
            .child(card)
            .into_any_element()
    }

    /// One rail button: the section's glyph, with its count riding the corner.
    fn rail_button(
        &mut self,
        section: Library,
        tooltip: &'static str,
        count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let (glyph, category) = Self::library_mark(palette, section);

        div()
            .id(match section {
                Library::Sessions => "rail-sessions",
                Library::Commands => "rail-commands",
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

        let sessions_button = self.rail_button(Library::Sessions, "Saved sessions", sessions, cx);
        let commands_button = self.rail_button(Library::Commands, "Quick send", commands, cx);

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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.side_panel_collapsed {
            return self.render_rail(cx);
        }

        let palette = self.interface_theme.palette();
        let has_active_tab = active_tab.is_some();
        // The composer has the foot of the panel; the lists share the rest,
        // and Quick send says first how much of it its rows want.
        let (commands, quick_send_need) = self.render_quick_send(has_active_tab, cx);
        let sessions =
            self.render_saved_sessions(panel_height - COMPOSER_HEIGHT, quick_send_need, cx);
        let composer = self.render_composer(active_tab.as_ref(), window, cx);

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

#[cfg(test)]
mod tests {
    use super::contains_query;

    /// A search folds case unless its `Aa` is on, and an empty one is held
    /// by anything.
    #[test]
    fn a_search_minds_case_only_when_told_to() {
        assert!(contains_query("AT+GMR", "gmr", false));
        assert!(!contains_query("AT+GMR", "gmr", true));
        assert!(contains_query("AT+GMR", "GMR", true));
        assert!(contains_query("/dev/cu.usbserial", "USB", false));
        assert!(contains_query("anything", "", true));
        assert!(!contains_query("", "x", false));
    }
}
