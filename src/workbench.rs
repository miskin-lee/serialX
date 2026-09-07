//! The centre column: everything between the title bar and the window's
//! bottom edge.
//!
//! The workbench is two stacked bands — the tab strip and the terminal log.
//! The strip has a fixed height so the log is the only thing that grows.
//! What acts on the session in front lives elsewhere: connect beside the
//! filter in the title bar, and the composer at the foot of the side panel.

use gpui_kit::component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    kbd::Kbd,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app_icon::application_icon_image;
use crate::app_menu::{NewSerialTab, TERMINAL_CONTEXT};
use crate::icons::Glyph;
use crate::filter::{FilterMode, OutputFilter};
use crate::find::FindView;
use crate::terminal::{CaretShape, RenderContent};
use crate::presets::DEFAULT_TERMINAL_FONT_SIZE;
use crate::theme::{
    BODY, CAPTION, LABEL, MONO_SMALL, TerminalPalette, Typography, WORDMARK, WorkbenchPalette,
    fonts, tint,
};
use crate::{SerialTabSnapshot, SerialTabState, SerialWorkspace};

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
/// was always set on, over the 12.5pt type that sat on them. Kept as the
/// ratio so a size the setting names brings its own leading with it.
const TERMINAL_LEADING: f32 = 18. / DEFAULT_TERMINAL_FONT_SIZE;
/// The cursor when it is drawn as a bar or an underline.
const CARET_THICKNESS: f32 = 2.;
/// Breathing room between the tab strip and the first line.
const TERMINAL_TOP_INSET: f32 = 8.;
/// What the log says while the filter's mask keeps every line back.
const MASK_EMPTY_HINT: &str = "No line matches the filter";

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

    /// The band above the terminal: one tab per session, and the way to a
    /// new one. Connecting is done beside the filter in the title bar, and
    /// pausing, clearing and the log's switches live in the menus with their
    /// shortcuts, so the strip holds nothing that is not about *which*
    /// session.
    pub(crate) fn render_tab_strip(
        &mut self,
        active: Option<&SerialTabSnapshot>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        // No strip without a tab: the empty state has the whole column.
        active?;
        let palette = self.interface_theme.palette();
        let active_index = self.active_tab;

        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| Self::render_tab(index, tab, index == active_index, palette, cx))
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
            "{} · {}{}",
            tab.selected_port().name,
            tab.configuration.summary(),
            if tab.interactive { "" } else { " · read-only" }
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

    /// The terminal. It is a place to type as well as to read: a click gives
    /// it focus, and from then on keys go to the port of this tab — unless
    /// the tab was made read-only, when the log is only to read and the
    /// composer does the sending; the wheel moves through the scrollback,
    /// and a drag selects, to be copied with ⌘C.
    pub(crate) fn render_active_tab(
        &mut self,
        tab: SerialTabSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let focused = self.terminal_focus.is_focused(window) && window.is_window_active();
        if focused && tab.interactive {
            self.start_blinking(window, cx);
        }
        let terminal = self.render_terminal(&tab, focused, cx);
        // The find bar floats over the log's top-right corner while it is
        // open, the way VS Code's find widget does.
        let find_bar = tab.find.open.then(|| self.render_find_bar(&tab, cx));

        v_flex()
            .flex_1()
            .min_h_0()
            .bg(rgb(palette.editor))
            .overflow_hidden()
            .child(
                div()
                    .id("terminal")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .track_focus(&self.terminal_focus)
                    .key_context(TERMINAL_CONTEXT)
                    .cursor(CursorStyle::IBeam)
                    .on_key_down(cx.listener(|this, event, window, cx| {
                        this.terminal_key(event, window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.terminal_mouse_down(event, cx)
                        }),
                    )
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        let line_height = px(this.terminal_type().line_height);
                        let delta = event.delta.pixel_delta(line_height).y / line_height;
                        this.scroll_terminal(delta, cx);
                    }))
                    .child(terminal)
                    .children(find_bar),
            )
            .into_any_element()
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
        let focus = self.terminal_focus.clone();
        let composing = self.composing.clone();
        let cursor_shown = self.cursor_shown;
        let selecting = self.selecting;
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
                    this.terminal_metrics = TerminalMetrics {
                        cell_width: f32::from(layout.cell_width),
                        line_height: text.line_height,
                        text_left: f32::from(layout.gutter),
                        origin_x: f32::from(bounds.origin.x),
                        origin_y: f32::from(bounds.origin.y),
                        columns,
                        lines,
                    };
                    if let Some(tab) = this.tab_mut(tab_id) {
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
                                this.terminal_drag(event.position, cx);
                            } else {
                                this.terminal_release(cx);
                            }
                        });
                    });
                    let release = paint.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                            release.update(cx, |this, cx| this.terminal_release(cx));
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
