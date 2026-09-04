use gpui_kit::component::{
    Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{SerialTabSnapshot, SerialWorkspace};

impl SerialWorkspace {
    pub(crate) fn save_active_session(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let port_name = tab.selected_port().name.clone();
        let configuration = tab.configuration;
        let label = format!("{} · {}", port_name, configuration.summary());
        self.presets.add_session(label, port_name, configuration);
        cx.notify();
    }

    fn open_saved_session(&mut self, saved_id: u64, window: &mut Window, cx: &mut Context<Self>) {
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
        if let Some(index) = tab
            .ports
            .iter()
            .position(|port| port.name == saved.port_name)
        {
            tab.selected_port = index;
        } else {
            tab.ports.push(crate::PortItem {
                name: saved.port_name.clone(),
                subtitle: "Saved device · currently unavailable".into(),
                is_demo: false,
            });
            tab.selected_port = tab.ports.len() - 1;
        }
        tab.push_line(
            crate::LineKind::System,
            Vec::new(),
            Some(format!("Restored saved session: {}", saved.label)),
        );
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
    }

    fn remove_saved_session(&mut self, saved_id: u64, cx: &mut Context<Self>) {
        self.presets.remove_session(saved_id);
        cx.notify();
    }

    fn save_current_command(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let value = tab.send_input.read(cx).value().trim().to_string();
        if value.is_empty() {
            return;
        }
        self.presets.add_command(value);
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

    pub(crate) fn render_right_sidebar(
        &mut self,
        active_tab: Option<SerialTabSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let saved_sessions = self.presets.sessions.clone();
        let saved_commands = self.presets.commands.clone();
        let workspace = cx.weak_entity();

        let sessions = v_flex()
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_none()
                    .px_3()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.strong_foreground))
                            .child("SAVED SESSIONS"),
                    )
                    .child(
                        Button::new("save-active-session")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .disabled(active_tab.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_active_session(cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(saved_sessions.is_empty(), |list| {
                        list.child(
                            v_flex()
                                .p_3()
                                .gap_1()
                                .text_size(px(11.))
                                .text_color(rgb(palette.muted))
                                .child("No saved sessions")
                                .child("Use + to save the active port configuration."),
                        )
                    })
                    .children(saved_sessions.into_iter().map(|saved| {
                        let open_workspace = workspace.clone();
                        let edit_workspace = workspace.clone();
                        let remove_workspace = workspace.clone();
                        let saved_id = saved.id;
                        h_flex()
                            .id(format!("saved-session-{saved_id}"))
                            .group("saved-session")
                            .min_h(px(43.))
                            .px_3()
                            .gap_2()
                            .border_b_1()
                            .border_color(rgb(palette.border))
                            .hover(|row| row.bg(rgb(palette.hover)))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
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
                            .child(
                                Icon::new(IconName::SquareTerminal)
                                    .xsmall()
                                    .text_color(rgb(palette.muted)),
                            )
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.))
                                            .text_color(rgb(palette.foreground))
                                            .child(saved.label),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(palette.muted))
                                            .child(format!(
                                                "{} · Right-click to edit",
                                                saved.port_name
                                            )),
                                    ),
                            )
                            .child(
                                Button::new(format!("remove-session-{saved_id}"))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        let _ = remove_workspace.update(cx, |this, cx| {
                                            this.remove_saved_session(saved_id, cx);
                                        });
                                    }),
                            )
                    })),
            );

        let commands = v_flex()
            .flex_1()
            .min_h_0()
            .border_t_1()
            .border_color(rgb(palette.border))
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_none()
                    .px_3()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.strong_foreground))
                            .child("QUICK SEND"),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette.muted))
                            .child(format!("{} SAVED", saved_commands.len())),
                    ),
            )
            .child(v_flex().flex_1().min_h_0().overflow_y_scrollbar().children(
                saved_commands.into_iter().map(|saved| {
                    let send_workspace = workspace.clone();
                    let remove_workspace = workspace.clone();
                    let command_id = saved.id;
                    h_flex()
                        .id(format!("saved-command-{command_id}"))
                        .min_h(px(42.))
                        .px_3()
                        .gap_2()
                        .border_b_1()
                        .border_color(rgb(palette.border))
                        .hover(|row| row.bg(rgb(palette.hover)))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            let _ = send_workspace.update(cx, |this, cx| {
                                this.send_saved_command(command_id, window, cx);
                            });
                        })
                        .child(div().size(px(6.)).rounded_full().bg(rgb(palette.accent)))
                        .child(
                            v_flex()
                                .min_w_0()
                                .flex_1()
                                .gap_0p5()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(palette.foreground))
                                        .child(saved.label),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .font_family("SF Mono")
                                        .text_size(px(10.))
                                        .text_color(rgb(palette.muted))
                                        .child(saved.command),
                                ),
                        )
                        .child(
                            Button::new(format!("remove-command-{command_id}"))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    let _ = remove_workspace.update(cx, |this, cx| {
                                        this.remove_saved_command(command_id, cx);
                                    });
                                }),
                        )
                }),
            ))
            .child(if let Some(tab) = active_tab {
                let tab_id = tab.id;
                h_flex()
                    .flex_none()
                    .p_2()
                    .gap_1()
                    .border_t_1()
                    .border_color(rgb(palette.border))
                    .child(div().flex_1().min_w_0().child(Input::new(&tab.send_input)))
                    .child(
                        Button::new(("save-command", tab_id))
                            .outline()
                            .small()
                            .label("Save")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_current_command(cx);
                            })),
                    )
                    .child(
                        Button::new(("send-command", tab_id))
                            .primary()
                            .small()
                            .icon(IconName::ArrowUp)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.send_current(tab_id, window, cx);
                            })),
                    )
                    .into_any_element()
            } else {
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(palette.border))
                    .text_size(px(10.))
                    .text_color(rgb(palette.muted))
                    .child("Open a serial tab to send commands.")
                    .into_any_element()
            });

        v_flex()
            .w(px(282.))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.panel))
            .child(sessions)
            .child(commands)
            .into_any_element()
    }
}
