use gpui_kit::component::{
    Sizable, WindowExt,
    button::Button,
    dialog::DialogButtonProps,
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::*;

use crate::icons::{Glyph, icon_chip};
use crate::theme::{BODY_STRONG, InterfaceTheme, MICRO, Typography, WorkbenchPalette};
use crate::{
    BAUD_RATES, DATA_BITS, FLOW_CONTROLS, LineKind, PARITIES, PortItem, STOP_BITS,
    SerialConfiguration, SerialWorkspace, discover_ports, presets::StoredSession,
};

#[derive(Clone, Copy)]
enum ConfigurationTarget {
    NewTab,
    SavedSession(u64),
}

#[derive(Clone, Copy)]
enum ConfigurationField {
    Port,
    BaudRate,
    DataBits,
    StopBits,
    Parity,
    FlowControl,
}

struct ConfigurationSelector {
    id: &'static str,
    label: &'static str,
    value: String,
    options: Vec<String>,
    selected_index: usize,
    field: ConfigurationField,
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
                ports.push(PortItem {
                    name: saved.port_name.clone(),
                    subtitle: "Saved device · currently unavailable".into(),
                    is_demo: false,
                });
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

    fn select(&mut self, field: ConfigurationField, selected_index: usize, cx: &mut Context<Self>) {
        match field {
            ConfigurationField::Port => {
                self.selected_port = selected_index.min(self.ports.len().saturating_sub(1));
            }
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

    /// One labelled control: the field name above, the current value inside a
    /// full-width button that drops its options down.
    fn selector(
        selector: ConfigurationSelector,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let editor = cx.weak_entity();
        let label = selector.label;

        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1p5()
            .child(
                div()
                    .text_token(MICRO)
                    .text_color(rgb(palette.muted))
                    .child(label),
            )
            .child(
                Button::new(selector.id)
                    .outline()
                    .small()
                    .w_full()
                    .justify_between()
                    .label(selector.value.clone())
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (index, option) in selector.options.iter().enumerate() {
                            let editor = editor.clone();
                            menu = menu.item(
                                PopupMenuItem::new(option.clone())
                                    .checked(index == selector.selected_index)
                                    .on_click(move |_, _, cx| {
                                        let _ = editor.update(cx, |editor, cx| {
                                            editor.select(selector.field, index, cx);
                                        });
                                    }),
                            );
                        }
                        menu
                    }),
            )
    }
}

impl Render for SerialConfigurationEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        let selected = self.selected_port().clone();
        let port_options = self
            .ports
            .iter()
            .map(|port| format!("{} — {}", port.name, port.subtitle))
            .collect();
        v_flex()
            .gap_5()
            .child(
                v_flex()
                    .gap_4()
                    .child(Self::selector(
                        ConfigurationSelector {
                            id: "config-port",
                            label: "Port",
                            value: format!("{} — {}", selected.name, selected.subtitle),
                            options: port_options,
                            selected_index: self.selected_port,
                            field: ConfigurationField::Port,
                        },
                        palette,
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_3()
                            .items_end()
                            .child(Self::selector(
                                ConfigurationSelector {
                                    id: "config-baud",
                                    label: "Baud rate",
                                    value: self.configuration.baud_rate().to_string(),
                                    options: BAUD_RATES.iter().map(u32::to_string).collect(),
                                    selected_index: self.configuration.baud_index,
                                    field: ConfigurationField::BaudRate,
                                },
                                palette,
                                cx,
                            ))
                            .child(Self::selector(
                                ConfigurationSelector {
                                    id: "config-data-bits",
                                    label: "Data bits",
                                    value: DATA_BITS[self.configuration.data_bits_index].into(),
                                    options: DATA_BITS
                                        .iter()
                                        .map(|value| (*value).into())
                                        .collect(),
                                    selected_index: self.configuration.data_bits_index,
                                    field: ConfigurationField::DataBits,
                                },
                                palette,
                                cx,
                            ))
                            .child(Self::selector(
                                ConfigurationSelector {
                                    id: "config-stop-bits",
                                    label: "Stop bits",
                                    value: STOP_BITS[self.configuration.stop_bits_index].into(),
                                    options: STOP_BITS
                                        .iter()
                                        .map(|value| (*value).into())
                                        .collect(),
                                    selected_index: self.configuration.stop_bits_index,
                                    field: ConfigurationField::StopBits,
                                },
                                palette,
                                cx,
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_end()
                            .child(Self::selector(
                                ConfigurationSelector {
                                    id: "config-parity",
                                    label: "Parity",
                                    value: PARITIES[self.configuration.parity_index].into(),
                                    options: PARITIES.iter().map(|value| (*value).into()).collect(),
                                    selected_index: self.configuration.parity_index,
                                    field: ConfigurationField::Parity,
                                },
                                palette,
                                cx,
                            ))
                            .child(Self::selector(
                                ConfigurationSelector {
                                    id: "config-flow-control",
                                    label: "Flow control",
                                    value: FLOW_CONTROLS[self.configuration.flow_control_index]
                                        .into(),
                                    options: FLOW_CONTROLS
                                        .iter()
                                        .map(|value| (*value).into())
                                        .collect(),
                                    selected_index: self.configuration.flow_control_index,
                                    field: ConfigurationField::FlowControl,
                                },
                                palette,
                                cx,
                            ))
                            // Keeps the two controls on this row the same width
                            // as the three above them.
                            .child(div().flex_1().min_w_0()),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .p_2p5()
                    .rounded_lg()
                    .bg(rgb(palette.surface))
                    .border_1()
                    .border_color(rgb(palette.border_subtle))
                    .child(
                        div()
                            .text_token(MICRO)
                            .text_color(rgb(palette.muted))
                            .child("Summary"),
                    )
                    .child(
                        div()
                            .mono_token(BODY_STRONG)
                            .text_color(rgb(palette.strong_foreground))
                            .child(format!(
                                "{} · {} · {}",
                                selected.name,
                                self.configuration.summary(),
                                FLOW_CONTROLS[self.configuration.flow_control_index],
                            )),
                    ),
            )
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
        let (title, blurb, confirm) = match target {
            ConfigurationTarget::NewTab => (
                "New serial session",
                "These parameters have to match the device on the other end.",
                "Open Session",
            ),
            ConfigurationTarget::SavedSession(_) => (
                "Edit saved session",
                "Changes apply the next time this session is opened.",
                "Save Changes",
            ),
        };
        let palette = theme.palette();

        // `Dialog` only renders a footer it is handed, so the confirm and cancel
        // buttons come from `AlertDialog`, which builds one out of the button
        // props.
        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let editor = editor_for_submit.clone();
            alert
                .width(px(560.))
                .icon(icon_chip(Glyph::Port, palette.category_device, 34.))
                .title(title)
                .description(blurb)
                .close_button(true)
                .child(editor_for_dialog.clone())
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(confirm)
                        .show_cancel(true)
                        .cancel_text("Cancel"),
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
            tab.ports.push(PortItem {
                name: port_name.clone(),
                subtitle: "Configured device · currently unavailable".into(),
                is_demo: false,
            });
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
