#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app_menu;
mod configuration;
mod presets;
mod serial;
mod sidebar;
mod theme;
mod updater;

use std::{
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use app_menu::{bind_window_actions, configure_application_menus};
use gpui_kit::assets::Assets;
use gpui_kit::component::{
    Icon, IconName, Root, Sizable, TitleBar, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{InputEvent, InputState},
    menu::AppMenuBar,
    notification::Notification,
    scroll::ScrollableElement,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use presets::PresetStore;
use serial::*;
use smol::Timer;
use theme::{InterfaceTheme, apply_interface_theme};
use updater::{CheckResult, UpdateEvent, UpdateInfo, spawn_update_check, spawn_update_install};

const REPOSITORY_URL: &str = "https://github.com/miskin-lee/serialX";

enum UpdateStatus {
    Checking,
    UpToDate,
    Available(UpdateInfo),
    Downloading { version: String, downloaded: u64 },
    InstallerLaunched,
    Failed,
}

pub struct SerialWorkspace {
    tabs: Vec<SerialTabState>,
    active_tab: usize,
    next_tab_id: usize,
    interface_theme: InterfaceTheme,
    presets: PresetStore,
    update_status: UpdateStatus,
    update_tx: Sender<UpdateEvent>,
    update_rx: Receiver<UpdateEvent>,
    manual_update_check: bool,
    menu_bar: Entity<AppMenuBar>,
}

impl SerialWorkspace {
    fn new(interface_theme: InterfaceTheme, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (update_tx, update_rx) = mpsc::channel();
        let menu_bar = AppMenuBar::new(cx);

        let workspace = Self {
            tabs: Vec::new(),
            active_tab: 0,
            next_tab_id: 1,
            interface_theme,
            presets: PresetStore::load(),
            update_status: UpdateStatus::Checking,
            update_tx,
            update_rx,
            manual_update_check: false,
            menu_bar,
        };

        cx.spawn_in(window, async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(80)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let changed = this.drain_serial_events();
                        if this.drain_update_events(window, cx) || changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        spawn_update_check(workspace.update_tx.clone());
        workspace
    }

    fn set_interface_theme(
        &mut self,
        theme: InterfaceTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.interface_theme == theme {
            return;
        }
        self.interface_theme = theme;
        apply_interface_theme(theme, window, cx);
        cx.notify();
    }

    fn build_tab(id: usize, window: &mut Window, cx: &mut Context<Self>) -> SerialTabState {
        let send_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Enter a command…")
                .default_value("AT+STATUS?")
        });
        let subscription = cx.subscribe_in(
            &send_input,
            window,
            move |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.send_current(id, window, cx);
                }
            },
        );
        SerialTabState::new(id, send_input, subscription)
    }

    fn active_tab(&self) -> Option<&SerialTabState> {
        self.tabs.get(self.active_tab)
    }

    fn tab_index(&self, id: usize) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn close_tab(&mut self, id: usize, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(id) else {
            return;
        };
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active_tab = 0;
        } else if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if index < self.active_tab {
            self.active_tab -= 1;
        }
        cx.notify();
    }

    fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active_tab().map(|tab| tab.id) {
            self.close_tab(id, cx);
        }
    }

    fn refresh_ports(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if tab.connected || tab.connecting {
            return;
        }
        let current = tab.selected_port().name.clone();
        tab.ports = discover_ports();
        tab.selected_port = tab
            .ports
            .iter()
            .position(|port| port.name == current)
            .unwrap_or(0);
        tab.push_line(
            LineKind::System,
            Vec::new(),
            Some(format!("Scan complete · {} devices found", tab.ports.len())),
        );
        cx.notify();
    }

    fn toggle_connection(&mut self, tab_id: usize, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let tab = &mut self.tabs[index];
        if tab.connected || tab.connecting {
            let device = tab.selected_port().name.clone();
            tab.disconnect();
            tab.push_line(
                LineKind::System,
                Vec::new(),
                Some(format!("Disconnected from {device}")),
            );
            cx.notify();
            return;
        }

        let selected = tab.selected_port().clone();
        if selected.is_demo {
            tab.connected = true;
            tab.push_line(
                LineKind::System,
                Vec::new(),
                Some(format!(
                    "Loopback session ready · {}",
                    tab.configuration.summary()
                )),
            );
            cx.notify();
            return;
        }

        let (command_tx, command_rx) = mpsc::channel();
        tab.command_tx = Some(command_tx);
        tab.connecting = true;
        spawn_serial_worker(
            selected.name,
            tab.configuration,
            command_rx,
            tab.event_tx.clone(),
        );
        cx.notify();
    }

    fn toggle_active_connection(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active_tab().map(|tab| tab.id) {
            self.toggle_connection(id, cx);
        }
    }

    fn send_current(&mut self, tab_id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let input = self.tabs[index].send_input.clone();
        let value = input.read(cx).value().trim().to_string();
        if value.is_empty() {
            return;
        }

        let tab = &mut self.tabs[index];
        if !tab.connected {
            tab.push_line(
                LineKind::System,
                Vec::new(),
                Some("Connect a serial port before sending data.".into()),
            );
            cx.notify();
            return;
        }

        let mut bytes = if tab.hex_mode {
            parse_hex(&value).unwrap_or_else(|| value.as_bytes().to_vec())
        } else {
            value.as_bytes().to_vec()
        };
        bytes.extend_from_slice(b"\r\n");
        tab.push_line(LineKind::Tx, bytes.clone(), None);

        if tab.selected_port().is_demo {
            tab.push_line(LineKind::Rx, demo_response(&value), None);
        } else if let Some(tx) = &tab.command_tx {
            let _ = tx.send(SerialCommand::Write(bytes));
        }

        input.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn drain_serial_events(&mut self) -> bool {
        let mut changed = false;
        for tab in &mut self.tabs {
            let events: Vec<_> = tab.event_rx.try_iter().collect();
            if !events.is_empty() {
                changed = true;
            }
            for event in events {
                match event {
                    SerialEvent::Connected => {
                        tab.connecting = false;
                        tab.connected = true;
                        tab.push_line(
                            LineKind::System,
                            Vec::new(),
                            Some("Serial port opened; receiving data.".into()),
                        );
                    }
                    SerialEvent::Data(bytes) => {
                        if !tab.paused {
                            tab.push_line(LineKind::Rx, bytes, None);
                        }
                    }
                    SerialEvent::Error(message) => {
                        tab.disconnect();
                        tab.push_line(LineKind::System, Vec::new(), Some(message));
                    }
                    SerialEvent::Closed => tab.disconnect(),
                }
            }
        }
        changed
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.paused = !tab.paused;
            cx.notify();
        }
    }

    fn clear_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.terminal_lines.clear();
            cx.notify();
        }
    }

    fn toggle_hex(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.hex_mode = !tab.hex_mode;
            cx.notify();
        }
    }

    fn toggle_timestamps(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.timestamps = !tab.timestamps;
            cx.notify();
        }
    }

    fn toggle_auto_scroll(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.auto_scroll = !tab.auto_scroll;
            cx.notify();
        }
    }

    fn drain_update_events(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let events: Vec<_> = self.update_rx.try_iter().collect();
        if events.is_empty() {
            return false;
        }

        for event in events {
            match event {
                UpdateEvent::CheckCompleted(Ok(CheckResult::UpToDate { version })) => {
                    let show_result = self.manual_update_check;
                    self.manual_update_check = false;
                    self.update_status = UpdateStatus::UpToDate;
                    if show_result {
                        Self::show_up_to_date_dialog(version, window, cx);
                    }
                }
                UpdateEvent::CheckCompleted(Ok(CheckResult::Available(info))) => {
                    self.manual_update_check = false;
                    self.update_status = UpdateStatus::Available(info.clone());
                    self.show_update_available_dialog(info, window, cx);
                }
                UpdateEvent::CheckCompleted(Err(message)) => {
                    let show_result = self.manual_update_check;
                    self.manual_update_check = false;
                    self.update_status = UpdateStatus::Failed;
                    if show_result {
                        Self::show_update_error_dialog(message, window, cx);
                    }
                }
                UpdateEvent::DownloadProgress { downloaded } => {
                    if let UpdateStatus::Downloading { version, .. } = &self.update_status {
                        self.update_status = UpdateStatus::Downloading {
                            version: version.clone(),
                            downloaded,
                        };
                    }
                }
                UpdateEvent::InstallerLaunched(Ok(version)) => {
                    self.update_status = UpdateStatus::InstallerLaunched;
                    #[cfg(target_os = "windows")]
                    cx.quit();
                    #[cfg(not(target_os = "windows"))]
                    window.push_notification(
                        Notification::success(format!(
                            "The serialX v{version} installer is open. Follow the system prompts to finish updating."
                        ))
                        .title("Software Update"),
                        cx,
                    );
                }
                UpdateEvent::InstallerLaunched(Err(message)) => {
                    self.update_status = UpdateStatus::Failed;
                    Self::show_update_error_dialog(message, window, cx);
                }
            }
        }
        true
    }

    fn begin_update(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if let UpdateStatus::Available(info) = &self.update_status {
            let info = info.clone();
            let version = info.version.clone();
            self.update_status = UpdateStatus::Downloading {
                version: version.clone(),
                downloaded: 0,
            };
            spawn_update_install(info, self.update_tx.clone());
            cx.notify();
            Some(version)
        } else {
            None
        }
    }

    fn check_for_updates_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.update_status {
            UpdateStatus::Available(info) => {
                self.show_update_available_dialog(info.clone(), window, cx);
            }
            UpdateStatus::Downloading {
                version,
                downloaded,
            } => {
                window.push_notification(
                    Notification::info(format!(
                        "Downloading serialX v{version} ({} downloaded)",
                        format_bytes(*downloaded)
                    ))
                    .title("Software Update"),
                    cx,
                );
            }
            UpdateStatus::Checking => {
                self.manual_update_check = true;
                window.push_notification(
                    Notification::info("Checking GitHub Releases…").title("Software Update"),
                    cx,
                );
            }
            _ => {
                self.manual_update_check = true;
                self.update_status = UpdateStatus::Checking;
                spawn_update_check(self.update_tx.clone());
                window.push_notification(
                    Notification::info("Checking GitHub Releases…").title("Software Update"),
                    cx,
                );
                cx.notify();
            }
        }
    }

    fn show_update_available_dialog(
        &self,
        info: UpdateInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.weak_entity();
        let latest_version = info.version.clone();
        let package_name = info.asset_name.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let version_for_notice = latest_version.clone();
            alert
                .icon(Icon::new(IconName::RotateCw).size_5())
                .title(format!("serialX v{latest_version} is available"))
                .description(format!(
                    "Current version: v{}\nPackage: {package_name}\n\nDownload and install now?",
                    env!("CARGO_PKG_VERSION")
                ))
                .show_cancel(true)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Download and Install")
                        .cancel_text("Later"),
                )
                .on_ok(move |_, window, cx| {
                    let started = workspace
                        .update(cx, |workspace, cx| workspace.begin_update(cx))
                        .ok()
                        .flatten()
                        .is_some();
                    if started {
                        window.push_notification(
                            Notification::info(format!(
                                "Downloading serialX v{version_for_notice}…"
                            ))
                            .title("Software Update"),
                            cx,
                        );
                    }
                    true
                })
        });
    }

    fn show_up_to_date_dialog(version: String, window: &mut Window, cx: &mut App) {
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .icon(Icon::new(IconName::CircleCheck).size_5())
                .title("serialX is up to date")
                .description(format!("Current version: v{version}"))
                .button_props(DialogButtonProps::default().ok_text("OK"))
        });
    }

    fn show_update_error_dialog(message: String, window: &mut Window, cx: &mut App) {
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .icon(Icon::new(IconName::CircleX).size_5())
                .title("Unable to Check for or Install Update")
                .description(message.clone())
                .button_props(DialogButtonProps::default().ok_text("Close"))
        });
    }

    fn show_about_dialog(window: &mut Window, cx: &mut App) {
        window.open_alert_dialog(cx, |alert, _, _| {
            alert
                .icon(Icon::new(IconName::SquareTerminal).size_5())
                .title("serialX")
                .description(format!(
                    "Version {}\nA modern serial port workspace\n\nGNU GPL v3\n© 2026 miskin",
                    env!("CARGO_PKG_VERSION")
                ))
                .show_cancel(true)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Close")
                        .cancel_text("View on GitHub"),
                )
                .on_cancel(|_, _, cx| {
                    cx.open_url(REPOSITORY_URL);
                    true
                })
        });
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

    fn render_active_tab(&mut self, tab: SerialTabSnapshot, _cx: &mut Context<Self>) -> AnyElement {
        let palette = self.interface_theme.palette();
        let selected = tab.ports[tab.selected_port.min(tab.ports.len().saturating_sub(1))].clone();
        let status_label = if tab.connecting {
            "Connecting…"
        } else if tab.connected {
            "Connected"
        } else {
            "Disconnected"
        };
        let connection_color = if tab.connected {
            palette.success
        } else if tab.connecting {
            palette.warning
        } else {
            palette.muted
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .bg(rgb(palette.editor))
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(35.))
                    .flex_none()
                    .px_3()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .bg(rgb(palette.editor))
                    .child(
                        h_flex()
                            .h_full()
                            .gap_3()
                            .child(
                                div()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_b_1()
                                    .border_color(rgb(palette.accent))
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.strong_foreground))
                                    .child("TERMINAL"),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .text_color(rgb(connection_color))
                                    .text_xs()
                                    .child(div().size_2().rounded_full().bg(rgb(connection_color)))
                                    .child(status_label),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(if tab.hex_mode { "HEX" } else { "ASCII" })
                            .when(tab.paused, |row| row.child("· Paused"))
                            .when(!tab.auto_scroll, |row| row.child("· Manual Scroll")),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .py_2()
                    .children(tab.terminal_lines.iter().enumerate().map(|(index, line)| {
                        let badge = match line.kind {
                            LineKind::Rx => "RX",
                            LineKind::Tx => "TX",
                            LineKind::System => "SYS",
                        };
                        let badge_color = match line.kind {
                            LineKind::Rx => palette.success,
                            LineKind::Tx => palette.accent,
                            LineKind::System => palette.muted,
                        };
                        h_flex()
                            .id(("terminal-line", index))
                            .items_start()
                            .min_h(px(24.))
                            .px_3()
                            .py_1()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(30.))
                                    .flex_none()
                                    .text_color(rgb(badge_color))
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(badge),
                            )
                            .when(tab.timestamps, |row| {
                                row.child(
                                    div()
                                        .w(px(76.))
                                        .flex_none()
                                        .font_family("SF Mono")
                                        .text_size(px(10.))
                                        .text_color(rgb(palette.muted))
                                        .child(line.time.clone()),
                                )
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .font_family("SF Mono")
                                    .text_size(px(12.))
                                    .text_color(if line.kind == LineKind::System {
                                        rgb(palette.muted)
                                    } else {
                                        rgb(palette.foreground)
                                    })
                                    .child(Self::format_line_payload(tab.hex_mode, line)),
                            )
                    })),
            )
            .child(
                h_flex()
                    .h(px(23.))
                    .flex_none()
                    .px_3()
                    .gap_4()
                    .border_t_1()
                    .border_color(rgb(palette.border))
                    .bg(rgb(palette.status_bar))
                    .text_size(px(10.))
                    .text_color(rgb(palette.muted))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(div().size_1p5().rounded_full().bg(rgb(connection_color)))
                            .child(status_label),
                    )
                    .child(selected.name)
                    .child(tab.configuration.summary())
                    .child(if tab.hex_mode { "HEX" } else { "ASCII" })
                    .child(self.interface_theme.name()),
            )
            .into_any_element()
    }
}

impl Render for SerialWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.interface_theme.palette();
        let active_snapshot = self.active_tab().map(SerialTabSnapshot::from);
        let sidebar_snapshot = active_snapshot.clone();
        let (title, status_label, connection_color) = self
            .active_tab()
            .map(|tab| {
                (
                    format!("Serial {} · {}", tab.id, tab.selected_port().name),
                    tab.status_label(),
                    if tab.connected {
                        palette.success
                    } else if tab.connecting {
                        palette.warning
                    } else {
                        palette.muted
                    },
                )
            })
            .unwrap_or_else(|| ("No Serial Tab".into(), "Idle", palette.muted));

        let mut tab_items = Vec::with_capacity(self.tabs.len());
        for tab in &self.tabs {
            let tab_id = tab.id;
            let tab_label = format!("Serial {}  {}", tab.id, tab.selected_port().name);
            let dot_color = if tab.connected {
                palette.success
            } else if tab.connecting {
                palette.warning
            } else {
                palette.muted
            };
            tab_items.push(
                Tab::new()
                    .label(tab_label)
                    .prefix(div().size_2().rounded_full().bg(rgb(dot_color)))
                    .suffix(
                        Button::new(("close-tab", tab_id))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(tab_id, cx);
                            })),
                    ),
            );
        }

        let workspace = cx.weak_entity();
        let tab_bar = TabBar::new("serial-tabs")
            .children(tab_items)
            .selected_index(self.active_tab)
            .menu(true)
            .on_click(move |index, _, cx| {
                let index = *index;
                let _ = workspace.update(cx, |workspace, cx| {
                    if index < workspace.tabs.len() {
                        workspace.active_tab = index;
                        cx.notify();
                    }
                });
            });

        let content = if let Some(tab) = active_snapshot {
            self.render_active_tab(tab, cx)
        } else {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .bg(rgb(palette.editor))
                .gap_2()
                .child(
                    div()
                        .text_color(rgb(palette.muted))
                        .child(Icon::new(IconName::SquareTerminal).size_8()),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.foreground))
                        .child("No Serial Tabs Open"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Use Session > New Serial Tab to start a session."),
                )
                .into_any_element()
        };
        let sidebar = self.render_right_sidebar(sidebar_snapshot, cx);

        v_flex()
            .size_full()
            .bg(rgb(palette.editor))
            .text_color(rgb(palette.foreground))
            .child(
                TitleBar::new()
                    .bg(rgb(palette.title_bar))
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .child(
                        h_flex()
                            .w_full()
                            .px_3()
                            .justify_between()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .when(cfg!(not(target_os = "macos")), |row| {
                                        row.child(
                                            div().w(px(310.)).h_8().child(self.menu_bar.clone()),
                                        )
                                    })
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(
                                                div()
                                                    .text_color(rgb(palette.strong_foreground))
                                                    .child("serialX"),
                                            )
                                            .child(div().text_color(rgb(palette.muted)).child("/"))
                                            .child(
                                                div().text_color(rgb(palette.muted)).child(title),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(palette.muted))
                                            .child(self.interface_theme.name()),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .size(px(6.))
                                                    .rounded_full()
                                                    .bg(rgb(connection_color)),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(palette.muted))
                                                    .child(status_label),
                                            ),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .min_h_0()
                            .child(tab_bar)
                            .child(content),
                    )
                    .child(sidebar),
            )
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(Assets);
    app.run(move |cx| {
        gpui_kit::init(cx);
        configure_application_menus(cx);

        let mut options = TitleBar::window_options();
        options.window_bounds = Some(WindowBounds::centered(size(px(1280.), px(800.)), cx));
        options.window_min_size = Some(size(px(960.), px(640.)));

        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                let interface_theme = InterfaceTheme::from_appearance(window.appearance());
                apply_interface_theme(interface_theme, window, cx);
                let workspace = cx.new(|cx| SerialWorkspace::new(interface_theme, window, cx));
                bind_window_actions(&workspace, cx);
                cx.new(|cx| {
                    Root::new(workspace, window, cx).bg(rgb(interface_theme.palette().editor))
                })
            })
            .expect("failed to open serialX window");
        })
        .detach();
    });
}
