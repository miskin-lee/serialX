//! The session dialog: where a port and its parameters are chosen, for a new
//! tab or for a saved session being edited.
//!
//! The form is laid out as a choice, not a settings sheet. The device is a
//! list you pick from, because there are only ever a few and the one you want
//! is named by what it is plugged into; the baud rate is a row of chips, since
//! there are six and one of them is nearly always right; and the frame — data
//! bits, parity, stop bits, flow control — is four segmented switches, since
//! each has two to four values and the whole frame should be readable at a
//! glance. A summary line at the foot restates the choice in the `115200 8N1`
//! shorthand the rest of the workbench prints.

use gpui_kit::component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::{Cancel, Confirm, DialogFooter},
    h_flex,
    kbd::Kbd,
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::controls::{Choice, ChoiceText, chip_row, eyebrow, segmented, tag};
use crate::icons::{Glyph, icon_chip, port_glyph};
use crate::theme::{
    BODY_STRONG, CAPTION, InterfaceTheme, LABEL, MICRO, TITLE, Typography, WorkbenchPalette, tint,
};
use crate::{
    BAUD_RATES, DATA_BITS, FLOW_CONTROLS, LineKind, PARITIES, PortItem, PortKind, STOP_BITS,
    SerialConfiguration, SerialWorkspace, discover_ports, presets::StoredSession,
};

/// Width of the dialog: room for four baud chips and a two-column frame.
const DIALOG_WIDTH: f32 = 540.;
/// Height of one device row.
const PORT_ROW_HEIGHT: f32 = 44.;
/// The most device rows the list shows before it scrolls.
const PORT_ROWS_MAX: usize = 4;
/// The fewest it shows when the window is short, so the list stays a list.
const PORT_ROWS_MIN: usize = 2;
/// The dialog's share of the window height, so the footer stays reachable on
/// the smallest window the workbench allows.
const DIALOG_MAX_HEIGHT_FRACTION: f32 = 0.82;
/// What the dialog stands at with an empty device list: padding, header, the
/// other three sections, the summary and the footer.
const DIALOG_FIXED_HEIGHT: f32 = 430.;

/// The height the device list is given, so it scrolls instead of growing.
///
/// A scroll region needs a definite height: given only a maximum it sizes to
/// its rows and the clip does the cutting, which looks the same and scrolls
/// not at all. When the ports outnumber the rows that fit, the last row shows
/// by half as the hint that there are more.
fn port_list_height(port_count: usize, viewport_height: f32) -> f32 {
    let room = viewport_height * DIALOG_MAX_HEIGHT_FRACTION - DIALOG_FIXED_HEIGHT;
    let fit =
        ((room / PORT_ROW_HEIGHT).floor().max(0.) as usize).clamp(PORT_ROWS_MIN, PORT_ROWS_MAX);
    if port_count <= fit {
        port_count.max(1) as f32 * PORT_ROW_HEIGHT
    } else {
        (fit as f32 - 0.5) * PORT_ROW_HEIGHT
    }
}

#[derive(Clone, Copy)]
enum ConfigurationTarget {
    NewTab,
    SavedSession(u64),
}

#[derive(Clone, Copy)]
enum ConfigurationField {
    BaudRate,
    DataBits,
    StopBits,
    Parity,
    FlowControl,
}

struct SerialConfigurationEditor {
    target: ConfigurationTarget,
    theme: InterfaceTheme,
    ports: Vec<PortItem>,
    selected_port: usize,
    configuration: SerialConfiguration,
}

impl SerialConfigurationEditor {
    fn new(
        target: ConfigurationTarget,
        theme: InterfaceTheme,
        saved: Option<&StoredSession>,
    ) -> Self {
        let mut ports = discover_ports();
        let mut selected_port = 0;
        let configuration = saved
            .map(|saved| saved.configuration.sanitized())
            .unwrap_or_default();

        if let Some(saved) = saved {
            if let Some(index) = ports.iter().position(|port| port.name == saved.port_name) {
                selected_port = index;
            } else {
                ports.push(PortItem::unavailable(
                    saved.port_name.clone(),
                    "Saved device · currently unavailable",
                ));
                selected_port = ports.len() - 1;
            }
        }

        Self {
            target,
            theme,
            ports,
            selected_port,
            configuration,
        }
    }

    fn selected_port(&self) -> &PortItem {
        &self.ports[self.selected_port.min(self.ports.len().saturating_sub(1))]
    }

    fn select_port(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected_port = index.min(self.ports.len().saturating_sub(1));
        cx.notify();
    }

    /// Looks for devices again, keeping the current choice if it is still
    /// there — and keeping an absent saved device listed, so editing a saved
    /// session never silently retargets it.
    fn rescan(&mut self, cx: &mut Context<Self>) {
        let current = self.selected_port().clone();
        let mut ports = discover_ports();
        if current.kind == PortKind::Unavailable
            && !ports.iter().any(|port| port.name == current.name)
        {
            ports.push(current.clone());
        }
        self.selected_port = ports
            .iter()
            .position(|port| port.name == current.name)
            .unwrap_or(0);
        self.ports = ports;
        cx.notify();
    }

    fn select(&mut self, field: ConfigurationField, selected_index: usize, cx: &mut Context<Self>) {
        match field {
            ConfigurationField::BaudRate => {
                self.configuration.baud_index = selected_index.min(BAUD_RATES.len() - 1);
            }
            ConfigurationField::DataBits => {
                self.configuration.data_bits_index = selected_index.min(DATA_BITS.len() - 1);
            }
            ConfigurationField::StopBits => {
                self.configuration.stop_bits_index = selected_index.min(STOP_BITS.len() - 1);
            }
            ConfigurationField::Parity => {
                self.configuration.parity_index = selected_index.min(PARITIES.len() - 1);
            }
            ConfigurationField::FlowControl => {
                self.configuration.flow_control_index = selected_index.min(FLOW_CONTROLS.len() - 1);
            }
        }
        cx.notify();
    }

    /// The options of one field as choices, each wired back to [`select`].
    ///
    /// [`select`]: Self::select
    fn choices<'a>(
        &self,
        field: ConfigurationField,
        labels: impl IntoIterator<Item = &'a str>,
        selected: usize,
        cx: &mut Context<Self>,
    ) -> Vec<Choice> {
        labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                Choice::new(
                    label.to_string(),
                    index == selected,
                    cx.listener(move |editor, _, _, cx| editor.select(field, index, cx)),
                )
            })
            .collect()
    }

    /// A section: an eyebrow, an optional right-hand aside, and the control.
    fn section(
        palette: WorkbenchPalette,
        title: &str,
        aside: Option<AnyElement>,
        body: impl IntoElement,
    ) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .h(px(20.))
                    .items_center()
                    .justify_between()
                    .child(eyebrow(palette, title))
                    .children(aside),
            )
            .child(body)
    }

    /// A caption over a control, for the four frame switches.
    fn labelled(palette: WorkbenchPalette, label: &'static str, control: impl IntoElement) -> Div {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .text_token(CAPTION)
                    .text_color(rgb(palette.muted))
                    .child(label),
            )
            .child(control)
    }

    /// The mark at the end of a device row: a filled disc with a check when it
    /// is the chosen one, an empty ring when it is not.
    fn radio_mark(palette: WorkbenchPalette, selected: bool) -> impl IntoElement {
        div()
            .flex_none()
            .size(px(16.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .when(selected, |mark| {
                mark.bg(rgb(palette.accent)).child(
                    Icon::new(IconName::Check)
                        .size(px(10.))
                        .text_color(rgb(0xffffff)),
                )
            })
            .when(!selected, |mark| {
                mark.border_1().border_color(rgb(palette.badge))
            })
    }

    fn render_devices(
        &mut self,
        palette: WorkbenchPalette,
        viewport_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected_index = self.selected_port;
        let list_height = port_list_height(self.ports.len(), viewport_height);
        let rows = self
            .ports
            .iter()
            .enumerate()
            .map(|(index, port)| {
                let selected = index == selected_index;
                let (glyph, hue) = port_glyph(port.kind, palette);
                let name_color = if port.kind == PortKind::Unavailable {
                    palette.muted
                } else {
                    palette.strong_foreground
                };

                h_flex()
                    .id(("config-port", index))
                    .h(px(PORT_ROW_HEIGHT))
                    .flex_none()
                    .px_3()
                    .gap_3()
                    .items_center()
                    .cursor_pointer()
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(rgb(palette.border_subtle))
                    })
                    .when(selected, |row| row.bg(tint(palette.accent, 0.1)))
                    .when(!selected, |row| row.hover(|row| row.bg(rgb(palette.hover))))
                    .on_click(cx.listener(move |editor, _, _, cx| {
                        editor.select_port(index, cx);
                    }))
                    .child(icon_chip(glyph, hue, 26.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .ui_mono_token(LABEL)
                                    .text_color(rgb(name_color))
                                    .child(port.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_token(CAPTION)
                                    .text_color(rgb(palette.muted))
                                    .child(port.subtitle.clone()),
                            ),
                    )
                    .when(port.is_demo(), |row| {
                        row.child(tag(palette, palette.category_signal, MICRO, "Demo"))
                    })
                    .when(port.kind == PortKind::Unavailable, |row| {
                        row.child(tag(palette, palette.warning, MICRO, "Offline"))
                    })
                    .child(Self::radio_mark(palette, selected))
            })
            .collect::<Vec<_>>();

        let count = self.ports.len();
        let aside = h_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_token(CAPTION)
                    .text_color(rgb(palette.faint))
                    .child(if count == 1 {
                        "1 device".to_string()
                    } else {
                        format!("{count} devices")
                    }),
            )
            .child(
                Button::new("config-rescan")
                    .ghost()
                    .xsmall()
                    .icon(Glyph::Refresh)
                    .label("Rescan")
                    .on_click(cx.listener(|editor, _, _, cx| editor.rescan(cx))),
            )
            .into_any_element();

        Self::section(
            palette,
            "Device",
            Some(aside),
            div()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(palette.border_subtle))
                .bg(rgb(palette.card))
                .overflow_hidden()
                .child(
                    v_flex()
                        .h(px(list_height))
                        .overflow_y_scrollbar()
                        .children(rows),
                ),
        )
        .into_any_element()
    }

    fn render_baud_rate(
        &mut self,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let labels = BAUD_RATES.iter().map(u32::to_string).collect::<Vec<_>>();
        let choices = self.choices(
            ConfigurationField::BaudRate,
            labels.iter().map(String::as_str),
            self.configuration.baud_index,
            cx,
        );
        Self::section(
            palette,
            "Baud rate",
            None,
            chip_row("config-baud", palette, ChoiceText::mono(LABEL), choices),
        )
        .into_any_element()
    }

    fn render_framing(&mut self, palette: WorkbenchPalette, cx: &mut Context<Self>) -> AnyElement {
        let text = ChoiceText::ui(LABEL);
        let data_bits = segmented(
            "config-data-bits",
            palette,
            ChoiceText::mono(LABEL),
            self.choices(
                ConfigurationField::DataBits,
                DATA_BITS.iter().copied(),
                self.configuration.data_bits_index,
                cx,
            ),
        );
        let parity = segmented(
            "config-parity",
            palette,
            text,
            self.choices(
                ConfigurationField::Parity,
                PARITIES.iter().copied(),
                self.configuration.parity_index,
                cx,
            ),
        );
        let stop_bits = segmented(
            "config-stop-bits",
            palette,
            ChoiceText::mono(LABEL),
            self.choices(
                ConfigurationField::StopBits,
                STOP_BITS.iter().copied(),
                self.configuration.stop_bits_index,
                cx,
            ),
        );
        let flow_control = segmented(
            "config-flow-control",
            palette,
            text,
            self.choices(
                ConfigurationField::FlowControl,
                FLOW_CONTROLS.iter().copied(),
                self.configuration.flow_control_index,
                cx,
            ),
        );

        Self::section(
            palette,
            "Frame",
            None,
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(Self::labelled(palette, "Data bits", data_bits))
                        .child(Self::labelled(palette, "Parity", parity)),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(Self::labelled(palette, "Stop bits", stop_bits))
                        .child(Self::labelled(palette, "Flow control", flow_control)),
                ),
        )
        .into_any_element()
    }

    /// The choice restated: the shorthand the tab bar and saved sessions
    /// print, and under it the same thing in words.
    fn render_summary(&self, palette: WorkbenchPalette) -> AnyElement {
        let port = self.selected_port();
        let (glyph, hue) = port_glyph(port.kind, palette);
        let configuration = self.configuration;
        let stop_bits = if STOP_BITS[configuration.stop_bits_index] == "1" {
            "1 stop bit"
        } else {
            "2 stop bits"
        };
        let flow_control = match configuration.flow_control_index {
            0 => "no flow control".to_string(),
            index => format!("{} flow control", FLOW_CONTROLS[index].to_lowercase()),
        };
        let spelled_out = format!(
            "{} data bits · {} parity · {stop_bits} · {flow_control}",
            DATA_BITS[configuration.data_bits_index],
            PARITIES[configuration.parity_index].to_lowercase(),
        );

        h_flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .rounded(px(10.))
            .bg(rgb(palette.surface))
            .border_1()
            .border_color(rgb(palette.border_subtle))
            .child(icon_chip(glyph, hue, 28.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .ui_mono_token(BODY_STRONG)
                            .text_color(rgb(palette.strong_foreground))
                            .child(format!("{} · {}", port.name, configuration.summary())),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_token(CAPTION)
                            .text_color(rgb(palette.muted))
                            .child(spelled_out),
                    ),
            )
            .into_any_element()
    }
}

impl Render for SerialConfigurationEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        // The device list is the one part that gives way on a short window,
        // so the rest of the form and the footer never leave the screen.
        let viewport_height: f32 = window.viewport_size().height.into();
        v_flex()
            .gap_4()
            .child(self.render_devices(palette, viewport_height, cx))
            .child(self.render_baud_rate(palette, cx))
            .child(self.render_framing(palette, cx))
            .child(self.render_summary(palette))
    }
}

impl SerialWorkspace {
    pub(crate) fn open_new_serial_tab_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_configuration_dialog(ConfigurationTarget::NewTab, window, cx);
    }

    pub(crate) fn open_saved_session_editor(
        &mut self,
        saved_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_configuration_dialog(ConfigurationTarget::SavedSession(saved_id), window, cx);
    }

    fn open_configuration_dialog(
        &mut self,
        target: ConfigurationTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved = match target {
            ConfigurationTarget::NewTab => None,
            ConfigurationTarget::SavedSession(saved_id) => self
                .presets
                .sessions
                .iter()
                .find(|saved| saved.id == saved_id),
        };
        let theme = self.interface_theme;
        let editor = cx.new(|_| SerialConfigurationEditor::new(target, theme, saved));
        let editor_for_dialog = editor.clone();
        let editor_for_submit = editor.clone();
        let workspace = cx.weak_entity();
        let (title, blurb, confirm, confirm_glyph) = match target {
            ConfigurationTarget::NewTab => (
                "New session",
                "Pick the device and the parameters it expects. They open in a tab of their own.",
                "Open Session",
                Glyph::Bolt,
            ),
            ConfigurationTarget::SavedSession(_) => (
                "Edit saved session",
                "Changes apply the next time this session is opened.",
                "Save Changes",
                Glyph::Bookmark,
            ),
        };
        let palette = theme.palette();

        // `Dialog` only renders a footer it is handed; the alert variant
        // supplies the modal behaviour (Esc cancels, Enter confirms, the
        // backdrop does not dismiss) and the footer below supplies the buttons.
        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let editor = editor_for_submit.clone();
            alert
                .width(px(DIALOG_WIDTH))
                .p_5()
                .icon(icon_chip(Glyph::Port, palette.category_device, 36.))
                .title(
                    div()
                        .text_token(TITLE)
                        .text_color(rgb(palette.strong_foreground))
                        .child(title),
                )
                .description(blurb)
                .close_button(true)
                .child(editor_for_dialog.clone())
                .footer(
                    DialogFooter::new()
                        .justify_between()
                        .items_center()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1p5()
                                .text_token(CAPTION)
                                .text_color(rgb(palette.faint))
                                .children(
                                    Keystroke::parse("enter")
                                        .ok()
                                        .map(|stroke| Kbd::new(stroke).outline()),
                                )
                                .child("to confirm"),
                        )
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Button::new("config-cancel")
                                        .ghost()
                                        .label("Cancel")
                                        .on_click(|_, window, cx| {
                                            window.dispatch_action(Box::new(Cancel), cx)
                                        }),
                                )
                                .child(
                                    Button::new("config-confirm")
                                        .primary()
                                        .icon(confirm_glyph)
                                        .label(confirm)
                                        .on_click(|_, window, cx| {
                                            window.dispatch_action(
                                                Box::new(Confirm { secondary: false }),
                                                cx,
                                            )
                                        }),
                                ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let editor = editor.read(cx);
                    let port_name = editor.selected_port().name.clone();
                    let configuration = editor.configuration.sanitized();
                    let target = editor.target;

                    let _ = workspace.update(cx, |workspace, cx| match target {
                        ConfigurationTarget::NewTab => {
                            workspace.create_configured_tab(port_name, configuration, window, cx)
                        }
                        ConfigurationTarget::SavedSession(saved_id) => {
                            workspace
                                .presets
                                .update_session(saved_id, port_name, configuration);
                            cx.notify();
                        }
                    });
                    true
                })
        });
    }

    pub(crate) fn create_configured_tab(
        &mut self,
        port_name: String,
        configuration: SerialConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut tab = Self::build_tab(id, window, cx);
        tab.configuration = configuration.sanitized();
        if let Some(index) = tab.ports.iter().position(|port| port.name == port_name) {
            tab.selected_port = index;
        } else {
            tab.ports.push(PortItem::unavailable(
                port_name.clone(),
                "Configured device · currently unavailable",
            ));
            tab.selected_port = tab.ports.len() - 1;
        }
        tab.push_line(
            LineKind::System,
            Vec::new(),
            Some(format!(
                "Session created for {} · {}",
                port_name,
                configuration.summary()
            )),
        );
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{DIALOG_FIXED_HEIGHT, PORT_ROW_HEIGHT, port_list_height};

    /// A tall window shows up to four rows, and a list that fits is exactly
    /// as tall as its rows.
    #[test]
    fn the_list_is_as_tall_as_its_rows_until_they_stop_fitting() {
        assert_eq!(port_list_height(1, 800.), PORT_ROW_HEIGHT);
        assert_eq!(port_list_height(3, 800.), 3. * PORT_ROW_HEIGHT);
        assert_eq!(port_list_height(9, 800.), 3.5 * PORT_ROW_HEIGHT);
    }

    /// The smallest window still gets a list, and the dialog still fits.
    #[test]
    fn a_short_window_keeps_two_rows_and_a_reachable_footer() {
        let height = port_list_height(9, 640.);
        assert_eq!(height, 1.5 * PORT_ROW_HEIGHT);
        assert!(DIALOG_FIXED_HEIGHT + height <= 640. * 0.82);
    }

    #[test]
    fn an_empty_scan_still_draws_one_row_of_list() {
        assert_eq!(port_list_height(0, 800.), PORT_ROW_HEIGHT);
    }
}
