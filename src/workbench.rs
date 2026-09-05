//! The centre column: everything between the tab bar and the status bar.
//!
//! The workbench is laid out as four stacked bands — a device toolbar, the
//! terminal log, the composer, and (owned by `main`) the status bar. Each band
//! has a fixed height so the log is the only thing that grows, which keeps the
//! composer parked under the cursor no matter how much traffic arrives.

use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    kbd::Kbd,
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app_icon::application_icon_image;
use crate::app_menu::NewSerialTab;
use crate::icons::{Glyph, icon_chip};
use crate::theme::{
    BODY, CAPTION, LABEL, MICRO, MONO, MONO_SMALL, MONO_TAG, TITLE, Typography, WORDMARK,
    WorkbenchPalette, tint,
};
use crate::{LineKind, SerialTabSnapshot, SerialWorkspace, TerminalLine};

/// Height of the device toolbar above the terminal.
const TOOLBAR_HEIGHT: f32 = 46.;
/// Height of the composer below the terminal.
const COMPOSER_HEIGHT: f32 = 52.;
/// Width of the timestamp gutter, wide enough for `14:32:40.018`.
const TIME_GUTTER: f32 = 82.;
/// Square size of a toolbar icon button. `Button` derives the glyph from this
/// at 75%, and the two-tone glyphs need the room to stay legible.
const TOOL_BUTTON: f32 = 28.;

/// The content of one launcher card on the empty workspace.
struct ActionCard {
    id: &'static str,
    glyph: Glyph,
    category: u32,
    title: &'static str,
    description: &'static str,
}

impl SerialWorkspace {
    /// The direction tag, its colour, and the colour its payload prints in.
    fn line_colors(kind: LineKind, palette: WorkbenchPalette) -> (&'static str, u32, u32) {
        match kind {
            LineKind::Rx => ("RX", palette.success, palette.foreground),
            LineKind::Tx => ("TX", palette.accent, palette.foreground),
            LineKind::System => ("SYS", palette.muted, palette.muted),
        }
    }

    fn format_line_payload(hex_mode: bool, line: &TerminalLine) -> String {
        if let Some(note) = &line.note {
            return note.clone();
        }
        if hex_mode {
            line.payload
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            String::from_utf8_lossy(&line.payload)
                .trim_end_matches(['\r', '\n'])
                .to_string()
        }
    }

    /// A small filled dot in the colour of the current connection state.
    pub(crate) fn status_dot(size: f32, color: u32) -> impl IntoElement {
        div()
            .flex_none()
            .size(px(size))
            .rounded_full()
            .bg(rgb(color))
    }

    fn connection_color(tab: &SerialTabSnapshot, palette: WorkbenchPalette) -> u32 {
        if tab.connected {
            palette.success
        } else if tab.connecting {
            palette.warning
        } else {
            palette.muted
        }
    }

    /// The device band: what is being talked to, and the controls that talk.
    fn render_toolbar(&mut self, tab: &SerialTabSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let tab_id = tab.id;
        let selected = tab.selected_port().clone();
        let connected = tab.connected || tab.connecting;
        let status_color = Self::connection_color(tab, palette);
        let (glyph, category) = if selected.is_demo {
            (Glyph::Waveform, palette.category_signal)
        } else {
            (Glyph::Usb, palette.category_device)
        };

        h_flex()
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .px_3()
            .gap_3()
            .justify_between()
            .bg(rgb(palette.editor))
            .border_b_1()
            .border_color(rgb(palette.border))
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2p5()
                    .child(icon_chip(glyph, category, 28.))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_token(LABEL)
                                    .text_color(rgb(palette.strong_foreground))
                                    .child(selected.name.clone()),
                            )
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .gap_1p5()
                                    .text_token(CAPTION)
                                    .text_color(rgb(palette.muted))
                                    .child(
                                        div()
                                            .flex_none()
                                            .ui_mono_token(MONO_SMALL)
                                            .child(tab.configuration.summary()),
                                    )
                                    .child(div().flex_none().child("·"))
                                    .child(
                                        h_flex()
                                            .flex_none()
                                            .gap_1p5()
                                            .items_center()
                                            .text_color(rgb(status_color))
                                            .child(Self::status_dot(5., status_color))
                                            .child(tab.status_label()),
                                    )
                                    .child(div().flex_none().child("·"))
                                    .child(div().truncate().child(selected.subtitle.clone())),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new(("toggle-connection", tab_id))
                            .when(connected, |button| button.outline())
                            .when(!connected, |button| button.primary())
                            .small()
                            .icon(if connected { Glyph::Cable } else { Glyph::Bolt })
                            .label(if tab.connecting {
                                "Connecting…"
                            } else if tab.connected {
                                "Disconnect"
                            } else {
                                "Connect"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_connection(tab_id, cx);
                            })),
                    )
                    .child(Self::toolbar_divider(palette))
                    .child(
                        Button::new(("refresh-ports", tab_id))
                            .ghost()
                            .with_size(px(TOOL_BUTTON))
                            .icon(Glyph::Refresh)
                            .tooltip("Rescan serial devices")
                            .disabled(connected)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_ports(cx);
                            })),
                    )
                    .child(
                        Button::new(("toggle-pause", tab_id))
                            .ghost()
                            .with_size(px(TOOL_BUTTON))
                            .icon(if tab.paused {
                                Glyph::Play
                            } else {
                                Glyph::Pause
                            })
                            .toggled(tab.paused)
                            .tooltip(if tab.paused {
                                "Resume receiving"
                            } else {
                                "Pause receiving"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_pause(cx);
                            })),
                    )
                    .child(
                        Button::new(("clear-terminal", tab_id))
                            .ghost()
                            .with_size(px(TOOL_BUTTON))
                            .icon(Glyph::Sweep)
                            .tooltip("Clear the terminal")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_terminal(cx);
                            })),
                    )
                    .child(Self::toolbar_divider(palette))
                    .child(
                        Button::new(("toggle-timestamps", tab_id))
                            .ghost()
                            .with_size(px(TOOL_BUTTON))
                            .icon(Glyph::Clock)
                            .toggled(tab.timestamps)
                            .tooltip("Show timestamps")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_timestamps(cx);
                            })),
                    )
                    .child(
                        Button::new(("toggle-auto-scroll", tab_id))
                            .ghost()
                            .with_size(px(TOOL_BUTTON))
                            .icon(Glyph::Scroll)
                            .toggled(tab.auto_scroll)
                            .tooltip("Follow new output")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_auto_scroll(cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn toolbar_divider(palette: WorkbenchPalette) -> impl IntoElement {
        div()
            .flex_none()
            .w(px(1.))
            .h(px(16.))
            .mx_1()
            .bg(rgb(palette.border))
    }

    /// One line of traffic: timestamp gutter, direction tag, payload.
    fn render_terminal_line(
        index: usize,
        line: &TerminalLine,
        tab: &SerialTabSnapshot,
        palette: WorkbenchPalette,
    ) -> impl IntoElement {
        let (tag, tag_color, text_color) = Self::line_colors(line.kind, palette);

        h_flex()
            .id(("terminal-line", index))
            .items_start()
            .w_full()
            .px_4()
            .py(px(2.))
            .gap_3()
            .hover(|row| row.bg(tint(palette.foreground, 0.04)))
            .when(tab.timestamps, |row| {
                row.child(
                    div()
                        .flex_none()
                        .w(px(TIME_GUTTER))
                        .mono_token(MONO_SMALL)
                        .py(px(2.))
                        .text_color(rgb(palette.faint))
                        .child(line.time.clone()),
                )
            })
            .child(
                div()
                    .flex_none()
                    .w(px(30.))
                    .py(px(1.))
                    .my(px(2.))
                    .rounded(px(5.))
                    .bg(tint(tag_color, 0.14))
                    .mono_token(MONO_TAG)
                    .text_color(rgb(tag_color))
                    .text_center()
                    .child(tag),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .mono_token(MONO)
                    .text_color(rgb(text_color))
                    .child(Self::format_line_payload(tab.hex_mode, line)),
            )
    }

    /// The ASCII / HEX switch, styled as one segmented pill.
    fn render_mode_switch(&mut self, hex_mode: bool, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let segment = |label: &'static str, active: bool| {
            div()
                .id(label)
                .px_2()
                .py(px(3.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_token(MICRO)
                .when(active, |segment| {
                    segment
                        .bg(rgb(palette.editor))
                        .text_color(rgb(palette.strong_foreground))
                })
                .when(!active, |segment| {
                    segment
                        .text_color(rgb(palette.muted))
                        .hover(|segment| segment.text_color(rgb(palette.foreground)))
                })
                .child(label)
        };

        h_flex()
            .flex_none()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(8.))
            .bg(rgb(palette.surface))
            .border_1()
            .border_color(rgb(palette.border_subtle))
            .child(
                segment("ASCII", !hex_mode).on_click(cx.listener(|this, _, _, cx| {
                    this.set_hex_mode(false, cx);
                })),
            )
            .child(
                segment("HEX", hex_mode).on_click(cx.listener(|this, _, _, cx| {
                    this.set_hex_mode(true, cx);
                })),
            )
            .into_any_element()
    }

    /// The send band. Lives under the log rather than in the sidebar, so the
    /// thing you type into sits directly below the thing it prints to.
    fn render_composer(&mut self, tab: &SerialTabSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let tab_id = tab.id;
        let mode_switch = self.render_mode_switch(tab.hex_mode, cx);

        h_flex()
            .h(px(COMPOSER_HEIGHT))
            .flex_none()
            .px_3()
            .gap_2()
            .bg(rgb(palette.editor))
            .border_t_1()
            .border_color(rgb(palette.border))
            .child(mode_switch)
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&tab.send_input)
                        .h(px(32.))
                        .prefix(
                            Icon::new(if tab.hex_mode {
                                Glyph::Hex
                            } else {
                                Glyph::Terminal
                            })
                            .size(px(15.))
                            .text_color(rgb(palette.muted)),
                        )
                        .cleanable(true),
                ),
            )
            .child(
                Button::new(("save-command", tab_id))
                    .ghost()
                    .small()
                    .icon(Glyph::Bookmark)
                    .tooltip("Save this command to Quick Send")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.save_current_command(cx);
                    })),
            )
            .child(
                Button::new(("send-command", tab_id))
                    .primary()
                    .small()
                    .icon(Glyph::Send)
                    .tooltip("Send")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.send_current(tab_id, window, cx);
                    })),
            )
            .into_any_element()
    }

    pub(crate) fn render_active_tab(
        &mut self,
        tab: SerialTabSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let toolbar = self.render_toolbar(&tab, cx);
        let composer = self.render_composer(&tab, cx);
        let lines = tab
            .terminal_lines
            .iter()
            .enumerate()
            .map(|(index, line)| Self::render_terminal_line(index, line, &tab, palette))
            .collect::<Vec<_>>();

        v_flex()
            .flex_1()
            .min_h_0()
            .bg(rgb(palette.editor))
            .overflow_hidden()
            .child(toolbar)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .py_2()
                    .overflow_y_scrollbar()
                    .children(lines),
            )
            .child(composer)
            .into_any_element()
    }

    /// A launcher card on the empty workspace.
    fn render_action_card(
        &mut self,
        card: ActionCard,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let ActionCard {
            id,
            glyph,
            category,
            title,
            description,
        } = card;

        v_flex()
            .id(id)
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .w(px(208.))
            .p_3p5()
            .gap_2p5()
            .rounded_xl()
            .bg(rgb(palette.card))
            .border_1()
            .border_color(rgb(palette.border))
            .cursor_pointer()
            .hover(|card| {
                card.bg(rgb(palette.hover))
                    .border_color(tint(palette.accent, 0.55))
            })
            .child(icon_chip(glyph, category, 34.))
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_token(TITLE)
                            .text_color(rgb(palette.strong_foreground))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_token(CAPTION)
                            .text_color(rgb(palette.muted))
                            .child(description),
                    ),
            )
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

    /// Shown when no tab is open: identity first, then the three ways in.
    pub(crate) fn render_empty_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let saved_sessions = self.presets.sessions.len();

        let new_session = self.render_action_card(
            ActionCard {
                id: "empty-new-session",
                glyph: Glyph::Bolt,
                category: palette.category_device,
                title: "New session",
                description: "Pick a port and its parameters, then open a tab.",
            },
            |this, window, cx| this.open_new_serial_tab_dialog(window, cx),
            cx,
        );
        let loopback = self.render_action_card(
            ActionCard {
                id: "empty-loopback",
                glyph: Glyph::Waveform,
                category: palette.category_signal,
                title: "Loopback demo",
                description: "Explore the full workbench with no hardware attached.",
            },
            |this, window, cx| this.open_loopback_tab(window, cx),
            cx,
        );
        let saved = self.render_action_card(
            ActionCard {
                id: "empty-saved",
                glyph: Glyph::Bookmark,
                category: palette.category_session,
                title: "Saved sessions",
                description: if saved_sessions == 0 {
                    "Keep a port configuration and reopen it in one click."
                } else {
                    "Reopen a configuration from the panel on the right."
                },
            },
            |this, window, cx| this.open_first_saved_session(window, cx),
            cx,
        );

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
                                    .child("A serial port workspace for embedded work."),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .text_token(CAPTION)
                            .text_color(rgb(palette.faint))
                            .child("Press")
                            .children(
                                Kbd::binding_for_action(&NewSerialTab, None, window)
                                    .map(Kbd::outline),
                            )
                            .child("to start a session"),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_stretch()
                    .child(new_session)
                    .child(loopback)
                    .child(saved),
            )
            .into_any_element()
    }
}
