//! The centre column: everything between the title bar and the window's
//! bottom edge.
//!
//! The workbench is laid out as three stacked bands — the tab strip, the
//! terminal log, and the composer. The strip and the composer have fixed
//! heights so the log is the only thing that grows, which keeps the composer
//! parked under the cursor no matter how much traffic arrives.

use gpui_kit::component::{
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    kbd::Kbd,
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app_icon::application_icon_image;
use crate::app_menu::NewSerialTab;
use crate::controls::{Choice, ChoiceText, segmented};
use crate::icons::Glyph;
use crate::theme::{
    BODY, CAPTION, LABEL, MICRO, MONO, MONO_SMALL, MONO_TAG, Typography, WORDMARK,
    WorkbenchPalette, tint,
};
use crate::{LineKind, SerialTabSnapshot, SerialTabState, SerialWorkspace, TerminalLine};

/// Height of the tab strip above the terminal: two under the title bar's,
/// so the two bands read as one piece of chrome without repeating it.
const TAB_STRIP_HEIGHT: f32 = 36.;
/// Height of a tab, and of every control that shares the strip with it.
const TAB_HEIGHT: f32 = 26.;
/// The close mark inside a tab.
const TAB_CLOSE: f32 = 18.;
/// The widest a tab grows before its name truncates.
const TAB_MAX_WIDTH: f32 = 260.;
/// The narrowest a tab shrinks to when the strip is full.
const TAB_MIN_WIDTH: f32 = 120.;
/// How strongly a tag's hue washes the active tab's plate, and its ring.
const TAG_PLATE_ACTIVE: f32 = 0.18;
const TAG_RING_ACTIVE: f32 = 0.45;
/// The wash on a tab that is not in front, at rest and under the pointer:
/// faint enough to sit nearly flat, strong enough to read as the tag.
const TAG_PLATE_REST: f32 = 0.1;
const TAG_PLATE_HOVER: f32 = 0.18;
/// Height of the composer below the terminal.
const COMPOSER_HEIGHT: f32 = 52.;
/// Width of the timestamp gutter, wide enough for `14:32:40.018`.
const TIME_GUTTER: f32 = 82.;

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

    /// The band above the terminal: one tab per session on the left, and on
    /// the right the controls that act on the session in front of you —
    /// connect, pause, clear, timestamps, follow. What the session *is* is
    /// already named by its tab, so the strip does not say it twice.
    pub(crate) fn render_tab_strip(
        &mut self,
        active: Option<&SerialTabSnapshot>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let active = active?;
        let palette = self.interface_theme.palette();
        let active_index = self.active_tab;

        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| Self::render_tab(index, tab, index == active_index, palette, cx))
            .collect::<Vec<_>>();
        let actions = self.render_session_actions(active, palette, cx);

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
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .items_center()
                        .children(tabs)
                        .child(
                            Button::new("new-tab")
                                .ghost()
                                .with_size(px(TAB_HEIGHT))
                                .icon(IconName::Plus)
                                .tooltip_with_action("New session", &NewSerialTab, None)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_new_serial_tab_dialog(window, cx);
                                })),
                        ),
                )
                .child(actions)
                .into_any_element(),
        )
    }

    /// One tab: a status dot, its name — the alias it was given, else the
    /// port's path — and a close mark that shows on the active tab and on
    /// hover; the port and its parameters are in the tooltip, so a named tab
    /// still tells you what it is plugged into. The active tab is a raised plate and the
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
        index: usize,
        tab: &SerialTabState,
        active: bool,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = tab.id;
        let name = tab.title().to_string();
        let detail: SharedString = format!(
            "{} · {}",
            tab.selected_port().name,
            tab.configuration.summary()
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
            .when(active, |tab| {
                tab.bg(tint(hue, TAG_PLATE_ACTIVE))
                    .border_color(tint(hue, TAG_RING_ACTIVE))
            })
            .when(!active, |tab| {
                tab.bg(tint(hue, TAG_PLATE_REST))
                    .border_color(transparent_black())
                    .hover(move |tab| tab.bg(tint(hue, TAG_PLATE_HOVER)))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if index < this.tabs.len() {
                    this.active_tab = index;
                    cx.notify();
                }
            }))
            .child(Self::status_dot(6., status))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_token(LABEL)
                    .text_color(rgb(if active {
                        palette.strong_foreground
                    } else {
                        palette.muted
                    }))
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
            .into_any_element()
    }

    /// The controls that act on the session in front of you, at the right
    /// end of the strip: connect or disconnect, then pause and clear, then
    /// the two switches that change how the log reads.
    fn render_session_actions(
        &mut self,
        tab: &SerialTabSnapshot,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = tab.id;
        let connected = tab.connected || tab.connecting;

        h_flex()
            .flex_none()
            .gap_0p5()
            .items_center()
            .child(
                Button::new(("toggle-connection", tab_id))
                    .when(connected, |button| button.outline())
                    .when(!connected, |button| button.primary())
                    .small()
                    .compact()
                    .h(px(TAB_HEIGHT))
                    .rounded(px(TAB_HEIGHT / 2.))
                    .px_2p5()
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
            .child(Self::strip_divider(palette))
            .child(
                Button::new(("toggle-pause", tab_id))
                    .ghost()
                    .with_size(px(TAB_HEIGHT))
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
                    .with_size(px(TAB_HEIGHT))
                    .icon(Glyph::Sweep)
                    .tooltip("Clear the terminal")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clear_terminal(cx);
                    })),
            )
            .child(Self::strip_divider(palette))
            .child(
                Button::new(("toggle-timestamps", tab_id))
                    .ghost()
                    .with_size(px(TAB_HEIGHT))
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
                    .with_size(px(TAB_HEIGHT))
                    .icon(Glyph::Scroll)
                    .toggled(tab.auto_scroll)
                    .tooltip("Follow new output")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_auto_scroll(cx);
                    })),
            )
            .into_any_element()
    }

    fn strip_divider(palette: WorkbenchPalette) -> impl IntoElement {
        div()
            .flex_none()
            .w(px(1.))
            .h(px(14.))
            .mx_1p5()
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
