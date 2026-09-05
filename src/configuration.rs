//! The session dialog: where a port and its parameters are chosen, for a new
//! tab or for a saved session being edited.
//!
//! The form is laid out as a choice, not a settings sheet. The device is a
//! list you pick from, because there are only ever a few and the one you want
//! is named by what it is plugged into; the baud rate is a field with the
//! standard rates in a list behind it, since one of them is nearly always
//! right and the odd device wants one of its own; and the frame — data bits,
//! parity, stop bits, flow control — is four segmented switches, since each
//! has two to four values and the whole frame should be readable at a glance.
//! Last, the tab itself: a name, which stands in for the port's path wherever
//! the session is listed, and two rows of colour swatches, twenty-four in all,
//! for telling this session's tab from the others; a new session is offered
//! the first colour no open tab wears. A summary line at the foot restates the
//! choice in the `115200 8N1` shorthand the rest of the workbench prints,
//! behind a tag glyph in the chosen colour.

use gpui_kit::component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::{Cancel, Confirm, DialogFooter},
    h_flex,
    input::{Input, InputEvent, InputState},
    kbd::Kbd,
    menu::{DropdownMenu, PopupMenuItem},
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::controls::{Choice, ChoiceText, eyebrow, segmented, tag};
use crate::icons::{Glyph, icon_chip};
use crate::serial::{BaudRateError, DEFAULT_BAUD_RATE, is_listed_baud_rate, parse_baud_rate};
use crate::theme::{
    BODY_STRONG, CAPTION, InterfaceTheme, LABEL, MICRO, TAG_HUE_COUNT, TITLE, TagColor, Typography,
    WorkbenchPalette, tint,
};
use crate::{
    BAUD_RATES, DATA_BITS, FLOW_CONTROLS, LineKind, PARITIES, PortItem, PortKind, STOP_BITS,
    SerialConfiguration, SerialWorkspace, discover_ports, presets::StoredSession,
};

/// Width of the dialog: room for a two-column frame, and for the baud field
/// with a sentence beside it.
const DIALOG_WIDTH: f32 = 540.;
/// Width of the baud rate field: seven digits in the chrome's monospace, the
/// caret that opens the list, and air around both.
const BAUD_FIELD_WIDTH: f32 = 176.;
/// Height of the baud rate field, a little over a chip so the caret has room.
const BAUD_FIELD_HEIGHT: f32 = 30.;
/// How tall the list of standard rates grows before it scrolls: every
/// standard rate on a tall window, a scrollbar on a short one.
const BAUD_LIST_MAX_HEIGHT: f32 = 320.;
/// How far the caret's right edge sits in from the field's. The list hangs
/// from the caret, so it is narrowed by this much to line up with the field.
const BAUD_LIST_INSET: f32 = 8.;
/// Height of one device row.
const PORT_ROW_HEIGHT: f32 = 44.;
/// The most device rows the list shows before it scrolls.
const PORT_ROWS_MAX: usize = 4;
/// The fewest it shows when the window is short, so the list stays a list.
const PORT_ROWS_MIN: usize = 2;
/// The dialog's share of the window height, so the footer stays reachable on
/// the smallest window the workbench allows.
const DIALOG_MAX_HEIGHT_FRACTION: f32 = 0.9;
/// What the dialog stands at with an empty device list: padding, header, the
/// other four sections, the summary and the footer.
const DIALOG_FIXED_HEIGHT: f32 = 510.;
/// What the name field says while it is empty and no device is chosen.
const NAME_FALLBACK: &str = "Name";
/// A tag swatch: the disc, the hit target around it that also carries the
/// ring when the swatch is the chosen one, and the gap between targets.
const SWATCH_SIZE: f32 = 16.;
const SWATCH_TARGET: f32 = 22.;
const SWATCH_GAP: f32 = 2.;
/// How many swatches to a row. The two dozen hues make two rows, the vivid
/// dozen over the soft one.
const SWATCH_COLUMNS: usize = 12;
const SWATCH_ROWS: usize = TAG_HUE_COUNT / SWATCH_COLUMNS;
/// The tag grid's height: its rows of targets and the gaps between them. The
/// name field beside it is shorter, so this is the Tab section's body height.
const TAG_BLOCK_HEIGHT: f32 =
    SWATCH_ROWS as f32 * SWATCH_TARGET + (SWATCH_ROWS as f32 - 1.) * SWATCH_GAP;

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

/// The colour to offer a new session, given the colours of the open tabs:
/// the first in picker order that none of them wears, and once every colour
/// is taken, the picker wraps around by how many tabs there are.
fn suggest_tag(used: impl Iterator<Item = TagColor>) -> TagColor {
    let used = used.collect::<Vec<_>>();
    TagColor::HUES
        .iter()
        .copied()
        .find(|color| !used.contains(color))
        .unwrap_or(TagColor::HUES[used.len() % TagColor::HUES.len()])
}

/// The frame parameters: the ones picked from a fixed list. The baud rate is
/// typed or chosen and is handled on its own.
#[derive(Clone, Copy)]
enum ConfigurationField {
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
    /// The colour the session's tab will be tagged with.
    color: TagColor,
    /// The name the tab will carry. Its placeholder is the chosen device, so
    /// an empty field shows what the tab will say instead.
    alias_input: Entity<InputState>,
    _alias_subscription: Subscription,
    /// The baud rate as typed. Picking from the list writes into it too, so
    /// the field always shows the rate the session will open at.
    baud_input: Entity<InputState>,
    _baud_subscription: Subscription,
    /// Why the field's text is not a rate, while it is not one. The
    /// configuration keeps the last rate that was, and the dialog will not
    /// confirm until the text is one again.
    baud_error: Option<BaudRateError>,
}

impl SerialConfigurationEditor {
    fn new(
        target: ConfigurationTarget,
        theme: InterfaceTheme,
        saved: Option<&StoredSession>,
        color: TagColor,
        window: &mut Window,
        cx: &mut Context<Self>,
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

        let placeholder = ports
            .get(selected_port)
            .map_or_else(|| NAME_FALLBACK.to_string(), |port| port.name.clone());
        let alias = saved.and_then(|saved| saved.alias.clone());
        let alias_input = cx.new(|cx| {
            let input = InputState::new(window, cx).placeholder(placeholder);
            match alias {
                Some(alias) => input.default_value(alias),
                None => input,
            }
        });
        // The summary strip echoes the name as it is typed.
        let alias_subscription =
            cx.subscribe_in(&alias_input, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });

        let baud_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(DEFAULT_BAUD_RATE.to_string())
                .default_value(configuration.baud_rate().to_string())
        });
        let baud_subscription = cx.subscribe_in(
            &baud_input,
            window,
            |editor, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value();
                    editor.set_baud_text(&text, cx);
                }
            },
        );

        Self {
            target,
            theme,
            ports,
            selected_port,
            configuration,
            color,
            alias_input,
            _alias_subscription: alias_subscription,
            baud_input,
            _baud_subscription: baud_subscription,
            baud_error: None,
        }
    }

    /// The name as typed, or none when the field is empty or only spaces.
    fn alias(&self, cx: &App) -> Option<String> {
        let text = self.alias_input.read(cx).value();
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    /// Keeps the empty name field saying which device the tab would be named
    /// after, as the device changes under it.
    fn sync_name_placeholder(&self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = self
            .selected_port()
            .map_or_else(|| NAME_FALLBACK.to_string(), |port| port.name.clone());
        self.alias_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
    }

    /// Reads the field. A rate goes into the configuration; anything else
    /// leaves the configuration alone and marks the field.
    fn set_baud_text(&mut self, text: &str, cx: &mut Context<Self>) {
        match parse_baud_rate(text) {
            Ok(rate) => {
                self.configuration.baud_rate = rate;
                self.baud_error = None;
            }
            Err(error) => self.baud_error = Some(error),
        }
        cx.notify();
    }

    /// A rate picked from the list: into the configuration, and into the
    /// field, replacing whatever was typed.
    fn choose_baud_rate(&mut self, rate: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.configuration.baud_rate = rate;
        self.baud_error = None;
        self.baud_input.update(cx, |input, cx| {
            input.set_value(rate.to_string(), window, cx);
        });
        cx.notify();
    }

    fn focus_baud_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.baud_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// The chosen device, or none when the scan found nothing.
    fn selected_port(&self) -> Option<&PortItem> {
        self.ports
            .get(self.selected_port.min(self.ports.len().saturating_sub(1)))
    }

    fn select_port(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_port = index.min(self.ports.len().saturating_sub(1));
        self.sync_name_placeholder(window, cx);
        cx.notify();
    }

    /// Looks for devices again, keeping the current choice if it is still
    /// there — and keeping an absent saved device listed, so editing a saved
    /// session never silently retargets it.
    fn rescan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.selected_port().cloned();
        let mut ports = discover_ports();
        if let Some(current) = &current {
            if current.kind == PortKind::Unavailable
                && !ports.iter().any(|port| port.name == current.name)
            {
                ports.push(current.clone());
            }
        }
        self.selected_port = current
            .and_then(|current| ports.iter().position(|port| port.name == current.name))
            .unwrap_or(0);
        self.ports = ports;
        self.sync_name_placeholder(window, cx);
        cx.notify();
    }

    fn select_color(&mut self, color: TagColor, cx: &mut Context<Self>) {
        self.color = color;
        cx.notify();
    }

    fn select(&mut self, field: ConfigurationField, selected_index: usize, cx: &mut Context<Self>) {
        match field {
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

    /// What the list shows when the scan found nothing: the fact, and what to
    /// do about it.
    fn no_devices_row(palette: WorkbenchPalette) -> impl IntoElement {
        h_flex()
            .h(px(PORT_ROW_HEIGHT))
            .px_3()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_token(LABEL)
                    .text_color(rgb(palette.muted))
                    .child("No serial devices found"),
            )
            .child(
                div()
                    .text_token(CAPTION)
                    .text_color(rgb(palette.faint))
                    .child("Plug one in and rescan"),
            )
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
        let mut rows = self
            .ports
            .iter()
            .enumerate()
            .map(|(index, port)| {
                let selected = index == selected_index;
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
                    .on_click(cx.listener(move |editor, _, window, cx| {
                        editor.select_port(index, window, cx);
                    }))
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
                    .when(port.kind == PortKind::Unavailable, |row| {
                        row.child(tag(palette, palette.warning, MICRO, "Offline"))
                    })
                    .child(Self::radio_mark(palette, selected))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            rows.push(Self::no_devices_row(palette).into_any_element());
        }

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
                    .on_click(cx.listener(|editor, _, window, cx| editor.rescan(window, cx))),
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

    /// The baud rate: a field you type into, with the standard rates in a
    /// list behind the caret at its end. Beside it, what the field holds — a
    /// hint while it is a listed rate, a `Custom` tag when it is one the list
    /// does not have, and the reason when it is not a rate at all.
    fn render_baud_rate(
        &mut self,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rate = self.configuration.baud_rate();
        let error = self.baud_error;
        let editor = cx.weak_entity();

        let list = Button::new("config-baud-list")
            .ghost()
            .with_size(px(22.))
            .icon(
                Icon::new(IconName::ChevronDown)
                    .size(px(12.))
                    .text_color(rgb(palette.muted)),
            )
            .tooltip("Standard rates")
            .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
                menu = menu
                    .min_w(px(BAUD_FIELD_WIDTH - BAUD_LIST_INSET))
                    .max_h(px(BAUD_LIST_MAX_HEIGHT))
                    .scrollable(true);
                for &listed in BAUD_RATES {
                    let editor = editor.clone();
                    menu = menu.item(
                        PopupMenuItem::new(listed.to_string())
                            .checked(listed == rate)
                            .on_click(move |_, window, cx| {
                                let _ = editor.update(cx, |editor, cx| {
                                    editor.choose_baud_rate(listed, window, cx);
                                });
                            }),
                    );
                }
                menu
            });

        let field = Input::new(&self.baud_input)
            .small()
            .w(px(BAUD_FIELD_WIDTH))
            .h(px(BAUD_FIELD_HEIGHT))
            .ui_mono_token(LABEL)
            .bg(rgb(palette.input))
            .border_color(rgb(if error.is_some() {
                palette.danger
            } else {
                palette.input_border
            }))
            .rounded(px(8.))
            .pl_2p5()
            .pr_1()
            .focus_bordered(error.is_none())
            .suffix(list);

        let caption = |color: u32, text: &'static str| {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_token(CAPTION)
                .text_color(rgb(color))
                .child(text)
        };
        let aside = match error {
            Some(error) => h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(tag(palette, palette.danger, MICRO, "Invalid"))
                .child(caption(palette.danger, error.message())),
            None if is_listed_baud_rate(rate) => h_flex().flex_1().min_w_0().child(caption(
                palette.faint,
                "Pick from the list, or type any rate",
            )),
            None => h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(tag(palette, palette.category_signal, MICRO, "Custom"))
                .child(caption(
                    palette.muted,
                    "Not a standard rate · needs device support",
                )),
        };

        Self::section(
            palette,
            "Baud rate",
            None,
            h_flex().items_center().gap_3().child(field).child(aside),
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

    /// One swatch: a disc of the colour in a round target, ringed in the
    /// colour when it is the chosen one.
    fn render_swatch(
        &self,
        index: usize,
        color: TagColor,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hue = palette.tag(color);
        let selected = color == self.color;
        let name = color.name();

        div()
            .id(("config-tag", index))
            .flex_none()
            .size(px(SWATCH_TARGET))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(tint(hue, if selected { 1. } else { 0. }))
            .when(!selected, |target| {
                target.hover(move |target| target.border_color(tint(hue, 0.5)))
            })
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(name).build(window, cx))
            .on_click(cx.listener(move |editor, _, _, cx| editor.select_color(color, cx)))
            .child(div().size(px(SWATCH_SIZE)).rounded_full().bg(rgb(hue)))
            .into_any_element()
    }

    /// The tab: its name and its colour, side by side under one eyebrow. The
    /// name field takes what the swatches leave; the swatches are the bright
    /// dozen over the deep dozen, so a column holds two colours that read as
    /// kin. Two rows beside the field, rather than under it, keep the footer
    /// on screen at the smallest window.
    fn render_tab_section(
        &mut self,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = Input::new(&self.alias_input)
            .small()
            .h(px(BAUD_FIELD_HEIGHT))
            .text_token(LABEL)
            .font_weight(FontWeight::NORMAL)
            .bg(rgb(palette.input))
            .border_color(rgb(palette.input_border))
            .rounded(px(8.))
            .px_2p5()
            .cleanable(true);

        let mut rows = Vec::with_capacity(SWATCH_ROWS);
        for (row, hues) in TagColor::HUES.chunks(SWATCH_COLUMNS).enumerate() {
            let mut swatches = Vec::with_capacity(SWATCH_COLUMNS);
            for (column, &color) in hues.iter().enumerate() {
                let index = row * SWATCH_COLUMNS + column;
                swatches.push(self.render_swatch(index, color, palette, cx));
            }
            rows.push(
                h_flex()
                    .items_center()
                    .gap(px(SWATCH_GAP))
                    .children(swatches),
            );
        }

        let aside = div()
            .text_token(CAPTION)
            .text_color(rgb(palette.faint))
            .child("Name is optional · the device stands in")
            .into_any_element();

        Self::section(
            palette,
            "Tab",
            Some(aside),
            h_flex()
                .h(px(TAG_BLOCK_HEIGHT))
                .items_center()
                .gap_3()
                .child(div().flex_1().min_w_0().child(name))
                .child(v_flex().gap(px(SWATCH_GAP)).children(rows)),
        )
        .into_any_element()
    }

    /// The choice restated the way the tab will state it: the name or the
    /// device, then the shorthand the tab bar and saved sessions print, and
    /// under it the same thing in words — with the device first when a name
    /// has taken its place above. All behind a tag glyph in the chosen
    /// colour, the one place the tag is named as a tag.
    fn render_summary(&self, palette: WorkbenchPalette, cx: &App) -> AnyElement {
        let port = self.selected_port();
        let hue = palette.tag(self.color);
        let device = port.map_or("No device", |port| port.name.as_str());
        let alias = self.alias(cx);
        let title = alias.as_deref().unwrap_or(device);
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
        let mut spelled_out = format!(
            "{} data bits · {} parity · {stop_bits} · {flow_control}",
            DATA_BITS[configuration.data_bits_index],
            PARITIES[configuration.parity_index].to_lowercase(),
        );
        if alias.is_some() {
            spelled_out = format!("{device} · {spelled_out}");
        }

        h_flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .rounded(px(10.))
            .bg(rgb(palette.surface))
            .border_1()
            .border_color(rgb(palette.border_subtle))
            .child(icon_chip(Glyph::Tag, hue, 28.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .ui_mono_token(BODY_STRONG)
                            .text_color(rgb(palette.strong_foreground))
                            .child(format!("{title} · {}", configuration.summary())),
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
            .child(self.render_tab_section(palette, cx))
            .child(self.render_summary(palette, cx))
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
        // A saved session keeps its colour; a new one is offered a colour no
        // open tab wears, so the strip tells its sessions apart from the start.
        let color = match (target, saved) {
            (ConfigurationTarget::SavedSession(_), Some(saved)) => saved.color,
            _ => suggest_tag(self.tabs.iter().map(|tab| tab.color)),
        };
        let editor =
            cx.new(|cx| SerialConfigurationEditor::new(target, theme, saved, color, window, cx));
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
                    // A field that does not hold a rate keeps the dialog
                    // open, with the field in focus, rather than opening the
                    // port at the last rate that was one.
                    if editor.read(cx).baud_error.is_some() {
                        editor.update(cx, |editor, cx| editor.focus_baud_input(window, cx));
                        return false;
                    }
                    let editor = editor.read(cx);
                    let Some(port_name) = editor.selected_port().map(|port| port.name.clone())
                    else {
                        return false;
                    };
                    let configuration = editor.configuration.sanitized();
                    let color = editor.color;
                    let alias = editor.alias(cx);
                    let target = editor.target;

                    let _ = workspace.update(cx, |workspace, cx| match target {
                        ConfigurationTarget::NewTab => workspace.create_configured_tab(
                            port_name,
                            configuration,
                            color,
                            alias,
                            window,
                            cx,
                        ),
                        ConfigurationTarget::SavedSession(saved_id) => {
                            workspace.presets.update_session(
                                saved_id,
                                port_name,
                                configuration,
                                color,
                                alias,
                            );
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
        color: TagColor,
        alias: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut tab = Self::build_tab(id, window, cx);
        tab.configuration = configuration.sanitized();
        tab.color = color;
        tab.alias = alias;
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
    use super::{
        DIALOG_FIXED_HEIGHT, DIALOG_MAX_HEIGHT_FRACTION, PORT_ROW_HEIGHT, SWATCH_COLUMNS,
        SWATCH_GAP, SWATCH_ROWS, SWATCH_TARGET, TAG_BLOCK_HEIGHT, TAG_HUE_COUNT, TagColor,
        port_list_height, suggest_tag,
    };

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
        assert!(DIALOG_FIXED_HEIGHT + height <= 640. * DIALOG_MAX_HEIGHT_FRACTION);
    }

    #[test]
    fn an_empty_scan_still_draws_one_row_of_list() {
        assert_eq!(port_list_height(0, 800.), PORT_ROW_HEIGHT);
    }

    /// The hues have to fill their rows, or the grid ends in a ragged line.
    #[test]
    fn tag_hues_fill_two_whole_rows() {
        assert_eq!(TAG_HUE_COUNT % SWATCH_COLUMNS, 0);
        assert_eq!(SWATCH_ROWS, 2);
        assert_eq!(TAG_BLOCK_HEIGHT, 2. * SWATCH_TARGET + SWATCH_GAP);
    }

    /// A new session takes the first free colour, and never a used one while
    /// a free one is left.
    #[test]
    fn a_new_session_is_offered_a_colour_no_tab_wears() {
        assert_eq!(suggest_tag([].into_iter()), TagColor::Red);
        assert_eq!(
            suggest_tag([TagColor::Red, TagColor::Amber].into_iter()),
            TagColor::Orange
        );
        let all = TagColor::HUES.into_iter();
        assert_eq!(suggest_tag(all), TagColor::Red);
        let all_and_one = TagColor::HUES.into_iter().chain([TagColor::Red]);
        assert_eq!(suggest_tag(all_and_one), TagColor::Orange);
    }
}
