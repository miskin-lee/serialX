//! The right panel: the library of things you keep between sessions.
//!
//! Two stacked sections, each a header over a scrolling list of cards. Rows are
//! cards rather than table lines so a saved item reads as an object you can act
//! on, and every one is anchored by a coloured glyph in the manner of VS Code's
//! Material Icon Theme: a command is green, and a session wears its own tag
//! colour, so the card and the tab it opens match.
//!
//! Two things fold here, at different grains: a section collapses to its own
//! header, and the whole panel collapses to an icon rail that still carries the
//! counts and reopens on a click. A collapsed section gives its height back to
//! its neighbour, so the panel never holds a half-empty list above a full one.

use gpui_kit::component::{
    Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{Glyph, icon_chip};
use crate::theme::{
    CAPTION, HEADING, LABEL, MICRO, MONO_SMALL, Typography, WorkbenchPalette, tint,
};
use crate::{SerialTabSnapshot, SerialWorkspace};

/// Width the panel opens at. Wide enough for a port path plus its baud
/// summary on one line at the caption size; the edge drags from there.
pub(crate) const SIDEBAR_WIDTH: f32 = 296.;
/// The narrowest the panel drags to: one card with its glyph and a name.
pub(crate) const SIDEBAR_MIN_WIDTH: f32 = 220.;
/// The widest: past this the cards are mostly air.
pub(crate) const SIDEBAR_MAX_WIDTH: f32 = 560.;
/// Width of the collapsed rail: one icon chip plus breathing room.
const RAIL_WIDTH: f32 = 52.;
const SECTION_HEADER_HEIGHT: f32 = 38.;
/// Resting opacity of a row's remove button. Visible enough to find, quiet
/// enough not to compete with the row it would delete.
const ROW_ACTION_REST: f32 = 0.3;

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
        let configuration = tab.configuration;
        let label = format!("{} · {}", port_name, configuration.summary());
        self.presets.add_session(
            label,
            port_name,
            configuration,
            tab.color,
            tab.alias.clone(),
        );
        self.sessions_collapsed = false;
        cx.notify();
    }

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

        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut tab = Self::build_tab(id, window, cx);
        tab.configuration = saved.configuration.sanitized();
        tab.color = saved.color;
        tab.alias = saved.alias.clone();
        if let Some(index) = tab
            .ports
            .iter()
            .position(|port| port.name == saved.port_name)
        {
            tab.selected_port = index;
        } else {
            tab.ports.push(crate::PortItem::unavailable(
                saved.port_name.clone(),
                "Saved device · currently unavailable",
            ));
            tab.selected_port = tab.ports.len() - 1;
        }
        tab.note(format!("Restored saved session: {}", saved.label));
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
    }

    fn remove_saved_session(&mut self, saved_id: u64, cx: &mut Context<Self>) {
        self.presets.remove_session(saved_id);
        cx.notify();
    }

    pub(crate) fn save_current_command(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let value = tab.send_input.read(cx).value().trim().to_string();
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
        let Some(tab_id) = self.active_tab().map(|tab| tab.id) else {
            return;
        };
        let input = self.tabs[self.active_tab].send_input.clone();
        input.update(cx, |input, cx| input.set_value(command, window, cx));
        self.send_current(tab_id, window, cx);
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

    /// A section header: a disclosure chevron, the title, a count, an action.
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
                    .child(
                        div()
                            .truncate()
                            .text_token(HEADING)
                            .text_color(rgb(palette.strong_foreground))
                            .child(title),
                    )
                    .child(
                        div()
                            .flex_none()
                            .px(px(6.))
                            .py(px(1.))
                            .rounded_full()
                            .bg(tint(palette.muted, 0.14))
                            .text_token(MICRO)
                            .text_color(rgb(palette.muted))
                            .child(count.to_string()),
                    ),
            )
            .children(action)
            .into_any_element()
    }

    /// The prompt shown where a list has nothing in it yet.
    fn empty_hint(
        palette: WorkbenchPalette,
        glyph: Glyph,
        headline: &'static str,
        hint: &'static str,
    ) -> impl IntoElement {
        v_flex()
            .items_center()
            .px_4()
            .py_5()
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
    /// only brings its remove button forward once the pointer is on it.
    fn row_card(
        palette: WorkbenchPalette,
        id: impl Into<ElementId>,
        group: &'static str,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .group(group)
            .flex()
            .items_center()
            .p_2()
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

    fn row_remove_button(
        id: impl Into<ElementId>,
        group: &'static str,
        tooltip: &'static str,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .flex_none()
            .opacity(ROW_ACTION_REST)
            .group_hover(group, |action| action.opacity(1.))
            .child(
                Button::new(id)
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(tooltip)
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        on_click(window, cx);
                    }),
            )
    }

    fn render_saved_sessions(
        &mut self,
        has_active_tab: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let collapsed = self.sessions_collapsed;
        let saved_sessions = self.presets.sessions.clone();
        let open_ports = saved_sessions
            .iter()
            .map(|saved| self.port_is_open(&saved.port_name))
            .collect::<Vec<_>>();
        let workspace = cx.weak_entity();

        let header = self.section_header(
            PanelSection::Sessions,
            "Sessions",
            saved_sessions.len(),
            Some(
                Button::new("save-active-session")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("Save the active port configuration")
                    .disabled(!has_active_tab)
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.save_active_session(cx);
                    }))
                    .into_any_element(),
            ),
            cx,
        );

        v_flex()
            .flex_none()
            .min_h_0()
            .child(header)
            .when(!collapsed, |section| {
                section.child(
                    v_flex()
                        .flex_none()
                        .max_h(relative(0.5))
                        .px_2()
                        .pb_2()
                        .gap_1p5()
                        .overflow_y_scrollbar()
                        .when(saved_sessions.is_empty(), |list| {
                            list.child(Self::empty_hint(
                                palette,
                                Glyph::Bookmark,
                                "No saved sessions yet",
                                "Use + to keep the active port configuration.",
                            ))
                        })
                        .children(saved_sessions.into_iter().zip(open_ports).map(
                            |(saved, is_open)| {
                                let open_workspace = workspace.clone();
                                let edit_workspace = workspace.clone();
                                let remove_workspace = workspace.clone();
                                let saved_id = saved.id;
                                // A named session shows its name, and keeps
                                // the port on the line below beside the rate.
                                let summary = saved.configuration.summary();
                                let (title, detail) = match &saved.alias {
                                    Some(alias) => {
                                        (alias.clone(), format!("{} · {summary}", saved.port_name))
                                    }
                                    None => (saved.port_name.clone(), summary),
                                };

                                Self::row_card(
                                    palette,
                                    ("saved-session", saved_id as usize),
                                    "saved-session",
                                )
                                .on_click(move |_, window, cx| {
                                    let _ = open_workspace.update(cx, |this, cx| {
                                        this.open_saved_session(saved_id, window, cx);
                                    });
                                })
                                .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                                    cx.stop_propagation();
                                    let _ = edit_workspace.update(cx, |this, cx| {
                                        this.open_saved_session_editor(saved_id, window, cx);
                                    });
                                })
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
                                .child(Self::row_remove_button(
                                    ("remove-session", saved_id as usize),
                                    "saved-session",
                                    "Forget this session",
                                    move |_, cx| {
                                        let _ = remove_workspace.update(cx, |this, cx| {
                                            this.remove_saved_session(saved_id, cx);
                                        });
                                    },
                                ))
                            },
                        )),
                )
            })
            .into_any_element()
    }

    fn render_quick_send(&mut self, has_active_tab: bool, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let collapsed = self.commands_collapsed;
        let saved_commands = self.presets.commands.clone();
        let workspace = cx.weak_entity();

        let header = self.section_header(
            PanelSection::Commands,
            "Quick send",
            saved_commands.len(),
            None,
            cx,
        );

        v_flex()
            .when(!collapsed, |section| section.flex_1().min_h_0())
            .when(collapsed, |section| section.flex_none())
            .border_t_1()
            .border_color(rgb(palette.border))
            .child(header)
            .when(!collapsed, |section| {
                section.child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .px_2()
                        .pb_2()
                        .gap_1p5()
                        .overflow_y_scrollbar()
                        .when(saved_commands.is_empty(), |list| {
                            list.child(Self::empty_hint(
                                palette,
                                Glyph::Run,
                                "No saved commands yet",
                                "Type a command below, then save it with the bookmark.",
                            ))
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
                            .child(Self::row_remove_button(
                                ("remove-command", command_id as usize),
                                "saved-command",
                                "Forget this command",
                                move |_, cx| {
                                    let _ = remove_workspace.update(cx, |this, cx| {
                                        this.remove_saved_command(command_id, cx);
                                    });
                                },
                            ))
                        })),
                )
            })
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

    pub(crate) fn render_right_sidebar(
        &mut self,
        active_tab: Option<SerialTabSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.side_panel_collapsed {
            return self.render_rail(cx);
        }

        let palette = self.interface_theme.palette();
        let has_active_tab = active_tab.is_some();
        let sessions = self.render_saved_sessions(has_active_tab, cx);
        let commands = self.render_quick_send(has_active_tab, cx);

        // The width is the resizable panel's to set; the column fills it.
        v_flex()
            .size_full()
            .min_w_0()
            .border_l_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.panel))
            .child(sessions)
            .child(commands)
            .into_any_element()
    }
}
