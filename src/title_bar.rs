//! The title bar, laid out the way VS Code lays out its own: tab navigation
//! and the output filter in the centre of the window, the side panel switch
//! at the right, and the menu bar at the left on the platforms that draw one.
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
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app_menu::{NextTab, PreviousTab, ToggleSidePanel};
use crate::theme::{LABEL, MICRO, MONO_SMALL, Typography, WorkbenchPalette, tint};
use crate::{SerialTabSnapshot, SerialWorkspace};

/// Height of the filter box. VS Code's command centre is 22; two more keep
/// the 12px label clear of its border.
const FILTER_HEIGHT: f32 = 24.;
/// Square size of a title bar icon button.
const TITLE_BUTTON: f32 = 26.;
/// The centre group's share of the bar, as VS Code sizes its command centre,
/// clamped so it neither swallows a wide window nor collapses on a narrow one.
const CENTER_FRACTION: f32 = 0.42;
const CENTER_MAX_WIDTH: f32 = 660.;
const CENTER_MIN_WIDTH: f32 = 300.;
/// What `TitleBar` pads on the left for the macOS traffic lights.
const TRAFFIC_LIGHT_INSET: f32 = 80.;
/// Width of the application menu bar on the platforms that draw it in the bar.
const MENU_BAR_WIDTH: f32 = 310.;
/// Placeholder in the filter box, echoed by the inert box shown without a tab.
pub(crate) const FILTER_PLACEHOLDER: &str = "Filter output";

impl SerialWorkspace {
    pub(crate) fn render_title_bar(
        &mut self,
        active: Option<&SerialTabSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let has_previous = self.active_tab > 0 && !self.tabs.is_empty();
        let has_next = self.active_tab + 1 < self.tabs.len();
        let filter_box = match active {
            Some(tab) => self.render_filter_box(tab, cx),
            None => Self::render_idle_filter_box(palette),
        };
        // Empty on macOS, where the traffic lights are all the left end holds.
        let menu_column = h_flex().flex_1().min_w_0().h_full().items_center().when(
            cfg!(not(target_os = "macos")),
            |column| {
                column.min_w(px(MENU_BAR_WIDTH)).child(
                    div()
                        .w(px(MENU_BAR_WIDTH))
                        .h_8()
                        .child(self.menu_bar.clone()),
                )
            },
        );

        TitleBar::new()
            .bg(rgb(palette.title_bar))
            .border_b_1()
            .border_color(rgb(palette.border))
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .items_center()
                    .pr_2()
                    .child(menu_column)
                    .child(
                        h_flex()
                            .flex_none()
                            .w(relative(CENTER_FRACTION))
                            .max_w(px(CENTER_MAX_WIDTH))
                            .min_w(px(CENTER_MIN_WIDTH))
                            .items_center()
                            .gap_0p5()
                            .child(
                                Button::new("previous-tab")
                                    .ghost()
                                    .with_size(px(TITLE_BUTTON))
                                    .icon(IconName::ArrowLeft)
                                    .disabled(!has_previous)
                                    .tooltip_with_action("Previous tab", &PreviousTab, None)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.select_previous_tab(cx);
                                    })),
                            )
                            .child(
                                Button::new("next-tab")
                                    .ghost()
                                    .with_size(px(TITLE_BUTTON))
                                    .icon(IconName::ArrowRight)
                                    .disabled(!has_next)
                                    .tooltip_with_action("Next tab", &NextTab, None)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.select_next_tab(cx);
                                    })),
                            )
                            .child(div().flex_1().min_w_0().ml_1p5().child(filter_box)),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .items_center()
                            .justify_end()
                            .gap_0p5()
                            .when(cfg!(target_os = "macos"), |column| {
                                column.flex_basis(px(TRAFFIC_LIGHT_INSET))
                            })
                            .child(
                                Button::new("title-side-panel")
                                    .ghost()
                                    .with_size(px(TITLE_BUTTON))
                                    .icon(if self.side_panel_collapsed {
                                        IconName::PanelRightOpen
                                    } else {
                                        IconName::PanelRightClose
                                    })
                                    .tooltip_with_action(
                                        "Show / hide the side panel",
                                        &ToggleSidePanel,
                                        None,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_side_panel(cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The command-centre box, wired to the active tab's filter. Its right
    /// end reports how much of the log is showing, or why the pattern will
    /// not compile, ahead of the two switches.
    fn render_filter_box(&mut self, tab: &SerialTabSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let tab_id = tab.id;
        let filter = &tab.filter;
        let error = filter.error().map(str::to_owned);
        let showing = filter
            .is_active()
            .then(|| (tab.visible_lines().count(), tab.terminal_lines.len()));

        let status = match (&error, showing) {
            (Some(message), _) => Some(
                div()
                    .flex_none()
                    .max_w(px(200.))
                    .truncate()
                    .text_token(MICRO)
                    .text_color(rgb(palette.danger))
                    .child(message.clone())
                    .into_any_element(),
            ),
            (None, Some((visible, total))) => Some(
                div()
                    .flex_none()
                    .ui_mono_token(MONO_SMALL)
                    .text_color(rgb(palette.muted))
                    .child(format!("{visible} of {total}"))
                    .into_any_element(),
            ),
            (None, None) => None,
        };

        Input::new(&tab.filter_input)
            .small()
            .text_token(LABEL)
            .font_weight(FontWeight::NORMAL)
            .bg(rgb(palette.input))
            .border_color(rgb(if error.is_some() {
                palette.danger
            } else {
                palette.input_border
            }))
            .rounded(px(6.))
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
                    .gap_0p5()
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
                    ),
            )
            .into_any_element()
    }

    /// One of the two switches inside the box, drawn like the toggles in VS
    /// Code's find widget: a bare glyph that takes the accent when it is on.
    fn filter_switch(
        id: impl Into<ElementId>,
        glyph: &'static str,
        on: bool,
        tooltip: &'static str,
        palette: WorkbenchPalette,
        cx: &App,
    ) -> Button {
        let button = Button::new(id)
            .xsmall()
            .compact()
            .tab_stop(false)
            .label(glyph)
            .toggled(on)
            .tooltip(tooltip);
        if on {
            button
                .custom(
                    ButtonCustomVariant::new(cx)
                        .color(tint(palette.accent, 0.14).into())
                        .foreground(rgb(palette.accent).into())
                        .hover(tint(palette.accent, 0.22).into())
                        .active(tint(palette.accent, 0.3).into()),
                )
                .selected(true)
        } else {
            button.ghost()
        }
    }

    /// The box when there is no tab: the same footprint, nothing to type into.
    fn render_idle_filter_box(palette: WorkbenchPalette) -> AnyElement {
        h_flex()
            .h(px(FILTER_HEIGHT))
            .w_full()
            .px_2()
            .gap_1p5()
            .items_center()
            .rounded(px(6.))
            .bg(rgb(palette.input))
            .border_1()
            .border_color(rgb(palette.input_border))
            .opacity(0.6)
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
