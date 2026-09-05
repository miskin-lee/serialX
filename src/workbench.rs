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
use crate::controls::{Choice, ChoiceText, segmented};
use crate::icons::{Glyph, icon_chip, port_glyph};
use crate::theme::{
    BODY, CAPTION, LABEL, MICRO, MONO, MONO_SMALL, MONO_TAG, Typography, WORDMARK,
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

impl SerialWorkspace {
    /// The direction tag, its colour, and the colour its payload prints in.
    fn line_colors(kind: LineKind, palette: WorkbenchPalette) -> (&'static str, u32, u32) {
        match kind {
            LineKind::Rx => ("RX", palette.success, palette.foreground),
            LineKind::Tx => ("TX", palette.accent, palette.foreground),
            LineKind::System => ("SYS", palette.muted, palette.muted),
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

    pub(crate) fn connection_color(tab: &SerialTabSnapshot, palette: WorkbenchPalette) -> u32 {
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
        let (glyph, category) = port_glyph(selected.kind, palette);

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
                    .child(line.display_text(tab.hex_mode)),
            )
    }

    /// The ASCII / HEX switch: the same segmented rail the session dialog
    /// sets its framing with, so the two places a mode is picked look alike.
    fn render_mode_switch(&mut self, hex_mode: bool, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        segmented(
            "mode-switch",
            palette,
            ChoiceText::ui(MICRO),
            vec![
                Choice::new(
                    "ASCII",
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
            .visible_lines()
            .map(|(index, line)| Self::render_terminal_line(index, line, &tab, palette))
            .collect::<Vec<_>>();
        let nothing_matches = lines.is_empty() && tab.filter.is_active();

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
                    .children(lines)
                    .when(nothing_matches, |log| {
                        log.child(
                            div()
                                .w_full()
                                .py_6()
                                .text_center()
                                .text_token(CAPTION)
                                .text_color(rgb(palette.faint))
                                .child("No lines match the filter"),
                        )
                    }),
            )
            .child(composer)
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
