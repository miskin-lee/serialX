//! The title bar: the workbench's menu bar, laid out as VS Code lays out its
//! own.
//!
//! Centre, the *command centre*: tab arrows, the output filter and, at its
//! right, the connect switch with the session's byte counters beside it —
//! what belongs to the session in front of you, side by side, and all of it
//! changing with the tab. It is sized the way VS Code sizes its own. Right,
//! the switches for how the window is divided: the split, the way back from
//! one while there is one, and the side panel. Left, only what the platform
//! puts there — the traffic lights on macOS, the application menus
//! elsewhere. Which session is in front of you is said by its tab, and the
//! ways to any other session are the tab strip, the side panel and the
//! Session menu; a pill here saying it again would only be a second thing
//! to look at.
//!
//! The bar is a little taller than the component default so its pills have
//! room to be pills, and it is painted with a faint top light rather than a
//! flat fill: the one gradient in the workbench, reserved for the edge that
//! holds the window up.
//!
//! The centre group is centred on the *window*, not on the strip left over
//! after the traffic lights: on macOS the right column starts with as much
//! flex basis as `TitleBar` pads on the left, so the two columns split the
//! remaining width evenly around the true middle.

use gpui_kit::component::{
    Disableable, Icon, IconName, Selectable, Sizable, TitleBar,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    h_flex,
    input::Input,
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app_menu::{
    JoinSplit, NextTab, PreviousTab, SplitRight, ToggleConnection, ToggleSidePanel,
};
use crate::panes::{MAX_PANES, SplitAxis};
use crate::filter::FilterMode;
use crate::icons::Glyph;
use crate::theme::{LABEL, MICRO, Typography, WorkbenchPalette, mix, tint};
use crate::{SerialTabSnapshot, SerialWorkspace};

/// Height of the bar. Four more than the component default: a 26px pill needs
/// six of air above and below it to read as floating rather than wedged.
pub(crate) const TITLE_BAR_HEIGHT: f32 = 38.;
/// Diameter of a macOS traffic light, for centring them on the taller bar.
const TRAFFIC_LIGHT_DIAMETER: f32 = 12.;
/// Height of every pill and icon button in the bar.
const CONTROL_HEIGHT: f32 = 26.;
/// Height of the filter box. Two more than the pills either side of it, so the
/// thing you type into is the biggest thing in the bar.
const FILTER_HEIGHT: f32 = 28.;
/// Square size of the tab arrows, a step smaller than the icon buttons.
const NAV_BUTTON: f32 = 24.;
/// The centre group's share of the bar, as VS Code sizes its command centre
/// with a connect pill and the byte counters added, clamped so it neither
/// swallows a wide window nor collapses on a narrow one.
const CENTER_FRACTION: f32 = 0.46;
const CENTER_MAX_WIDTH: f32 = 780.;
const CENTER_MIN_WIDTH: f32 = 460.;
/// What the number in a byte counter is given, right-aligned in it, so a
/// count climbing from `0 B` to `1.2 MB` grows leftwards into its own space
/// rather than shoving the filter box along with it. Wide enough for the
/// longest thing it says — a hair under a megabyte, spelled out in bytes.
const COUNTER_VALUE_WIDTH: f32 = 54.;
/// How faint the filter box and the connect pill go without a tab.
const IDLE_OPACITY: f32 = 0.6;
/// What `TitleBar` pads on the left for the macOS traffic lights.
const TRAFFIC_LIGHT_INSET: f32 = 80.;
/// Width of the application menu bar on the platforms that draw it in the bar.
const MENU_BAR_WIDTH: f32 = 300.;
/// Placeholder in the filter box, echoed by the inert box shown without a tab.
pub(crate) const FILTER_PLACEHOLDER: &str = "Filter output";

/// Where the macOS traffic lights sit so their centre line is the bar's.
pub(crate) fn traffic_light_position() -> Point<Pixels> {
    point(
        px(12.),
        px(((TITLE_BAR_HEIGHT - TRAFFIC_LIGHT_DIAMETER) / 2.).round()),
    )
}

/// Wraps a control in the bar so its press stays its own.
///
/// The bar takes any press it sees for the start of a window move, and any
/// double click for a zoom — which is not what a press on a button, or in
/// the filter box, means. Stopping the press here leaves the bar nothing to
/// move the window from and nothing to count towards a double click, and
/// what is inside the wrapper has already run: the bubble phase goes from
/// the innermost element out. The bar is still dragged by everything else —
/// the traffic lights' end, the gaps between the controls, the strip past
/// the last one.
fn keeps_its_press(control: impl IntoElement) -> Div {
    div()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(control)
}

impl SerialWorkspace {
    pub(crate) fn render_title_bar(
        &mut self,
        active: Option<&SerialTabSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        // What the split switch offers, and — when it offers nothing —
        // which of the three reasons to say so in its tooltip.
        let can_split = self.can_split_toward(SplitAxis::Across);
        let split_reason = if can_split {
            "Show this session beside the others"
        } else if self.panes.len() >= MAX_PANES {
            "The window is divided as far as it goes"
        } else if self.active_pane().tabs.len() < 2 {
            "Open a second session to show one beside the other"
        } else {
            "The window is too narrow for another pane"
        };
        let strip = self.active_pane().tabs.len();
        let position = self.active_strip_position();
        let has_previous = position.is_some_and(|index| index > 0);
        let has_next = position.is_some_and(|index| index + 1 < strip);
        let filter_box = match active {
            Some(tab) => self.render_filter_box(tab, cx),
            None => Self::render_idle_filter_box(palette),
        };
        let connect = self.render_connect_pill(active, cx);
        let traffic = Self::render_traffic(active, palette);

        // Empty on macOS, where the traffic lights are all the left end holds.
        let left_column = h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .overflow_hidden()
            .when(cfg!(not(target_os = "macos")), |column| {
                column.min_w(px(MENU_BAR_WIDTH)).child(
                    keeps_its_press(self.menu_bar.clone())
                        .flex_none()
                        .w(px(MENU_BAR_WIDTH))
                        .h(px(CONTROL_HEIGHT)),
                )
            });

        let center_column = h_flex()
            .flex_none()
            .w(relative(CENTER_FRACTION))
            .max_w(px(CENTER_MAX_WIDTH))
            .min_w(px(CENTER_MIN_WIDTH))
            .items_center()
            .gap_0p5()
            .child(
                keeps_its_press(
                    Button::new("previous-tab")
                        .ghost()
                        .with_size(px(NAV_BUTTON))
                        .icon(IconName::ArrowLeft)
                        .disabled(!has_previous)
                        .tooltip_with_action("Previous session", &PreviousTab, None)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.select_previous_tab(cx);
                        })),
                )
                .flex_none(),
            )
            .child(
                keeps_its_press(
                    Button::new("next-tab")
                        .ghost()
                        .with_size(px(NAV_BUTTON))
                        .icon(IconName::ArrowRight)
                        .disabled(!has_next)
                        .tooltip_with_action("Next session", &NextTab, None)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.select_next_tab(cx);
                        })),
                )
                .flex_none(),
            )
            .child(div().flex_1().min_w_0().ml_1p5().child(filter_box))
            .child(keeps_its_press(connect).flex_none().ml_1p5())
            .child(keeps_its_press(traffic).flex_none().ml_1p5());

        let right_column = h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .justify_end()
            .gap_1()
            .when(cfg!(target_os = "macos"), |column| {
                column.flex_basis(px(TRAFFIC_LIGHT_INSET))
            })
            // Splitting stands beside the panel switch: both are about how
            // the window is divided, not about what is in it.
            .when(self.can_join(), |column| {
                column.child(
                    keeps_its_press(
                        Button::new("title-join-split")
                            .ghost()
                            .with_size(px(CONTROL_HEIGHT))
                            .icon(Glyph::Join)
                            .tooltip_with_action(
                                "Fold this pane back into the others",
                                &JoinSplit,
                                None,
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.join_active_pane(window, cx);
                            })),
                    )
                    .flex_none(),
                )
            })
            .child(
                keeps_its_press(
                    Button::new("title-split")
                        .ghost()
                        .with_size(px(CONTROL_HEIGHT))
                        .icon(Glyph::Split)
                        .disabled(!can_split)
                        .tooltip_with_action(split_reason, &SplitRight, None)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.split_active_tab(SplitAxis::Across, window, cx);
                        })),
                )
                .flex_none(),
            )
            .child(
                keeps_its_press(
                    Button::new("title-side-panel")
                        .ghost()
                        .with_size(px(CONTROL_HEIGHT))
                        // The panel's switch is drawn in the workbench's own
                        // family, as the split and the join beside it are:
                        // the three sit on one 24-grid frame, so the row
                        // reads as three switches of one size rather than a
                        // hairline icon lodged between two solid ones.
                        .icon(if self.side_panel_collapsed {
                            Glyph::ShowPanel
                        } else {
                            Glyph::HidePanel
                        })
                        .tooltip_with_action("Show / hide the side panel", &ToggleSidePanel, None)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_side_panel(cx);
                        })),
                )
                .flex_none(),
            );

        // A faint light along the top edge, fading into the bar's own colour.
        let top_light = mix(palette.title_bar, palette.strong_foreground, 0.035);

        TitleBar::new()
            .h(px(TITLE_BAR_HEIGHT))
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgb(top_light), 0.),
                linear_color_stop(rgb(palette.title_bar), 1.),
            ))
            .border_b_1()
            .border_color(rgb(palette.border))
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .items_center()
                    .pr_2()
                    .gap_2()
                    .child(left_column)
                    .child(center_column)
                    .child(right_column),
            )
            .into_any_element()
    }

    /// The pill at the filter's right: connect or disconnect the session in
    /// front. Filled while the port is shut and outlined while it is open, so
    /// the bar says the state as well as offering the switch. Without a tab
    /// it stands faint and inert, keeping the filter box company.
    fn render_connect_pill(
        &mut self,
        active: Option<&SerialTabSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let connected = active.is_some_and(|tab| tab.connected || tab.connecting);
        let connecting = active.is_some_and(|tab| tab.connecting);
        let tab_id = active.map(|tab| tab.id);

        Button::new("toggle-connection")
            .when(connected, |button| button.outline())
            .when(!connected, |button| button.primary())
            .small()
            .compact()
            .h(px(CONTROL_HEIGHT))
            .rounded(px(CONTROL_HEIGHT / 2.))
            .px_2p5()
            .icon(if connected { Glyph::Cable } else { Glyph::Bolt })
            .label(if connecting {
                "Connecting…"
            } else if connected {
                "Disconnect"
            } else {
                "Connect"
            })
            .disabled(tab_id.is_none())
            .when(tab_id.is_none(), |button| button.opacity(IDLE_OPACITY))
            .tooltip_with_action(
                if connected {
                    "Disconnect this session"
                } else {
                    "Connect this session"
                },
                &ToggleConnection,
                None,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(tab_id) = tab_id {
                    this.toggle_connection(tab_id, cx);
                }
            }))
            .into_any_element()
    }

    /// The counters at the connect pill's right: how many bytes this
    /// session has taken off the port and how many it has put on it.
    ///
    /// They are the session's, not the window's — every tab counts its own —
    /// and they start over when its log is cleared, so the pair reads as
    /// "what has gone past since I last looked". A plate rather than a pill,
    /// and in the chrome's monospace: numbers that change every read should
    /// not be numbers that move. Without a tab the plate stands faint and
    /// zeroed, keeping the filter box and the pill company.
    fn render_traffic(active: Option<&SerialTabSnapshot>, palette: WorkbenchPalette) -> AnyElement {
        let rx = active.map_or(0, |tab| tab.rx_bytes);
        let tx = active.map_or(0, |tab| tab.tx_bytes);

        h_flex()
            .id("traffic-counters")
            .flex_none()
            .h(px(CONTROL_HEIGHT))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(CONTROL_HEIGHT / 2.))
            .bg(rgb(palette.surface))
            .border_1()
            .border_color(rgb(palette.border_subtle))
            .when(active.is_none(), |plate| plate.opacity(IDLE_OPACITY))
            .child(Self::byte_counter("RX", rx, palette))
            .child(
                div()
                    .flex_none()
                    .w(px(1.))
                    .h(px(12.))
                    .bg(rgb(palette.border_subtle)),
            )
            .child(Self::byte_counter("TX", tx, palette))
            .tooltip(|window, cx| {
                Tooltip::new("Bytes received and sent on this session, since its log was cleared")
                    .build(window, cx)
            })
            .into_any_element()
    }

    /// One counter: its two letters, then the count in the monospace face.
    fn byte_counter(label: &'static str, bytes: u64, palette: WorkbenchPalette) -> impl IntoElement {
        h_flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(
                div()
                    .text_token(MICRO)
                    .text_color(rgb(palette.muted))
                    .child(label),
            )
            .child(
                div()
                    .min_w(px(COUNTER_VALUE_WIDTH))
                    .text_right()
                    .ui_mono_font()
                    .text_token(MICRO)
                    .text_color(rgb(palette.foreground))
                    .whitespace_nowrap()
                    .child(format_bytes(bytes)),
            )
    }

    /// The accent-tinted pill a switched-on control wears: a wash of the
    /// accent rather than a solid fill, so it reads as on without shouting.
    pub(crate) fn accent_pill(palette: WorkbenchPalette, cx: &App) -> ButtonCustomVariant {
        ButtonCustomVariant::new(cx)
            .color(tint(palette.accent, 0.14).into())
            .foreground(rgb(palette.accent).into())
            .hover(tint(palette.accent, 0.22).into())
            .active(tint(palette.accent, 0.3).into())
    }

    /// The command-centre box, wired to the active tab's filter. Its right
    /// end says why the pattern will not compile, when it will not, ahead of
    /// the three switches: match case, regular expressions, and the mask that
    /// shows only the lines that match.
    fn render_filter_box(&mut self, tab: &SerialTabSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let tab_id = tab.id;
        let filter = &tab.filter;
        let error = filter.error().map(str::to_owned);

        let status = error.clone().map(|message| {
            div()
                .flex_none()
                .max_w(px(200.))
                .truncate()
                .text_token(MICRO)
                .text_color(rgb(palette.danger))
                .child(message)
                .into_any_element()
        });

        let field = Input::new(&tab.filter_input)
            .small()
            .min_h(px(FILTER_HEIGHT))
            .text_token(LABEL)
            .font_weight(FontWeight::NORMAL)
            .bg(rgb(palette.input))
            .border_color(rgb(if error.is_some() {
                palette.danger
            } else {
                palette.input_border
            }))
            .rounded(px(FILTER_HEIGHT / 2.))
            .px_3()
            .focus_bordered(error.is_none())
            .prefix(
                Icon::new(IconName::Search)
                    .size(px(13.))
                    .text_color(rgb(palette.muted)),
            )
            .cleanable(true)
            .suffix(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .children(status)
                    .child(
                        Self::filter_switch(
                            ("filter-match-case", tab_id),
                            "Aa",
                            filter.match_case(),
                            "Match case",
                            palette,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_filter_match_case(cx);
                        })),
                    )
                    .child(
                        Self::filter_switch(
                            ("filter-regex", tab_id),
                            ".*",
                            filter.use_regex(),
                            "Use regular expression",
                            palette,
                            cx,
                        )
                        .ui_mono_font()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_filter_regex(cx);
                        })),
                    )
                    .child(
                        Self::filter_icon_switch(
                            ("filter-mask", tab_id),
                            IconName::EyeOff,
                            filter.mode() == FilterMode::Mask,
                            "Show only matching lines",
                            palette,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_filter_mode(cx);
                        })),
                    ),
            );

        // In the box a press is a caret, a drag is a selection and a double
        // click is a word, none of which the bar above may take for its own.
        keeps_its_press(field).into_any_element()
    }

    /// One of the switches inside the box, drawn like the toggles in VS
    /// Code's find widget: a bare glyph that takes the accent when it is on.
    /// The side panel's search box borrows it for its `Aa`.
    pub(crate) fn filter_switch(
        id: impl Into<ElementId>,
        glyph: &'static str,
        on: bool,
        tooltip: &'static str,
        palette: WorkbenchPalette,
        cx: &App,
    ) -> Button {
        Self::switch(id, on, tooltip, palette, cx).label(glyph)
    }

    /// The same switch wearing an icon rather than letters: the mask's
    /// crossed eye, for what it does to the lines that do not match.
    pub(crate) fn filter_icon_switch(
        id: impl Into<ElementId>,
        icon: IconName,
        on: bool,
        tooltip: &'static str,
        palette: WorkbenchPalette,
        cx: &App,
    ) -> Button {
        Self::switch(id, on, tooltip, palette, cx).icon(icon)
    }

    fn switch(
        id: impl Into<ElementId>,
        on: bool,
        tooltip: &'static str,
        palette: WorkbenchPalette,
        cx: &App,
    ) -> Button {
        let button = Button::new(id)
            .xsmall()
            .compact()
            .rounded(px(6.))
            .tab_stop(false)
            .toggled(on)
            .tooltip(tooltip);
        if on {
            button.custom(Self::accent_pill(palette, cx)).selected(true)
        } else {
            button.ghost()
        }
    }

    /// The box when there is no tab: the same footprint, nothing to type into.
    fn render_idle_filter_box(palette: WorkbenchPalette) -> AnyElement {
        h_flex()
            .h(px(FILTER_HEIGHT))
            .w_full()
            .px_3()
            .gap_1p5()
            .items_center()
            .rounded(px(FILTER_HEIGHT / 2.))
            .bg(rgb(palette.input))
            .border_1()
            .border_color(rgb(palette.input_border))
            .opacity(IDLE_OPACITY)
            .child(
                Icon::new(IconName::Search)
                    .size(px(13.))
                    .text_color(rgb(palette.faint)),
            )
            .child(
                div()
                    .text_token(LABEL)
                    .font_weight(FontWeight::NORMAL)
                    .text_color(rgb(palette.faint))
                    .child(FILTER_PLACEHOLDER),
            )
            .into_any_element()
    }
}

/// A byte count as the bar says it: plain bytes up to a megabyte, then MB
/// and GB — the decimal units the platforms label a transfer with, carrying
/// one decimal only while there is one worth reading. A session's traffic is
/// read as a count first and a size second, so there is no kilobyte step in
/// between: the exact number of bytes stands until it is too long to read.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 2] = ["MB", "GB"];
    const STEP: f64 = 1000.;
    const MEGABYTE: u64 = 1_000_000;

    if bytes < MEGABYTE {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / MEGABYTE as f64;
    let mut unit = UNITS[0];
    for next in &UNITS[1..] {
        if value < STEP {
            break;
        }
        value /= STEP;
        unit = next;
    }
    if value < 10. {
        format!("{value:.1} {unit}")
    } else {
        format!("{} {unit}", value.round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::{TITLE_BAR_HEIGHT, format_bytes, traffic_light_position};
    use gpui_kit::px;

    /// The lights are 12px tall; on a 38px bar their centre has to be 19px.
    #[test]
    fn traffic_lights_sit_on_the_bars_centre_line() {
        let position = traffic_light_position();
        assert_eq!(position.y + px(6.), px(TITLE_BAR_HEIGHT / 2.));
    }

    /// Everything under a megabyte is said as the count it is: no kilobyte
    /// step rounds a session's first few thousand bytes away.
    #[test]
    fn counts_under_a_megabyte_are_said_in_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1), "1 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_234), "1234 B");
        assert_eq!(format_bytes(999_999), "999999 B");
    }

    /// A decimal while the number is one digit wide, none once it is two.
    #[test]
    fn larger_counts_climb_through_the_units() {
        assert_eq!(format_bytes(1_000_000), "1.0 MB");
        assert_eq!(format_bytes(1_500_000), "1.5 MB");
        assert_eq!(format_bytes(45_600_000), "46 MB");
        assert_eq!(format_bytes(999_000_000), "999 MB");
        assert_eq!(format_bytes(1_500_000_000), "1.5 GB");
        assert_eq!(format_bytes(12_000_000_000), "12 GB");
    }

    /// Past the last unit the number keeps growing rather than wrapping.
    #[test]
    fn the_biggest_counts_stay_in_gigabytes() {
        assert_eq!(format_bytes(4_000_000_000_000), "4000 GB");
    }
}
