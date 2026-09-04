#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod updater;

use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use gpui_kit::assets::Assets;
use gpui_kit::component::{
    Disableable, GlobalState, Icon, IconName, Root, Sizable, TitleBar, WindowExt,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{AppMenuBar, DropdownMenu, PopupMenuItem},
    notification::Notification,
    scroll::ScrollableElement,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use smol::Timer;
use updater::{CheckResult, UpdateEvent, UpdateInfo, spawn_update_check, spawn_update_install};

const NAVY: u32 = 0x17191f;
const NAVY_SOFT: u32 = 0x22252d;
const CANVAS: u32 = 0xf4f3ef;
const PANEL: u32 = 0xffffff;
const INK: u32 = 0x202229;
const MUTED: u32 = 0x777a83;
const LINE: u32 = 0xe7e5df;
const MINT: u32 = 0xc9f2d2;
const GREEN: u32 = 0x287a4a;
const ORANGE: u32 = 0xff754c;
const REPOSITORY_URL: &str = "https://github.com/miskin-lee/serialX";

const BAUD_RATES: &[u32] = &[9_600, 19_200, 38_400, 57_600, 115_200, 230_400];
const DATA_BITS: &[&str] = &["5", "6", "7", "8"];
const STOP_BITS: &[&str] = &["1", "2"];
const PARITIES: &[&str] = &["无", "奇校验", "偶校验"];
const FLOW_CONTROLS: &[&str] = &["无", "软件", "硬件"];

actions!(
    serialx_menu,
    [
        NewSerialTab,
        CloseSerialTab,
        RefreshPorts,
        ToggleConnection,
        TogglePause,
        ClearTerminal,
        ToggleHex,
        ToggleTimestamps,
        ToggleAutoScroll,
        CheckForUpdates,
        ShowAbout,
        QuitSerialX
    ]
);

#[derive(Clone)]
struct PortItem {
    name: String,
    subtitle: String,
    is_demo: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Rx,
    Tx,
    System,
}

#[derive(Clone)]
struct TerminalLine {
    time: String,
    kind: LineKind,
    payload: Vec<u8>,
    note: Option<String>,
}

enum SerialCommand {
    Write(Vec<u8>),
    Stop,
}

enum SerialEvent {
    Connected,
    Data(Vec<u8>),
    Error(String),
    Closed,
}

#[derive(Clone, Copy)]
struct SerialConfiguration {
    baud_index: usize,
    data_bits_index: usize,
    stop_bits_index: usize,
    parity_index: usize,
    flow_control_index: usize,
}

impl Default for SerialConfiguration {
    fn default() -> Self {
        Self {
            baud_index: 4,
            data_bits_index: 3,
            stop_bits_index: 0,
            parity_index: 0,
            flow_control_index: 0,
        }
    }
}

impl SerialConfiguration {
    fn baud_rate(self) -> u32 {
        BAUD_RATES[self.baud_index]
    }

    fn data_bits(self) -> serialport::DataBits {
        match self.data_bits_index {
            0 => serialport::DataBits::Five,
            1 => serialport::DataBits::Six,
            2 => serialport::DataBits::Seven,
            _ => serialport::DataBits::Eight,
        }
    }

    fn stop_bits(self) -> serialport::StopBits {
        match self.stop_bits_index {
            1 => serialport::StopBits::Two,
            _ => serialport::StopBits::One,
        }
    }

    fn parity(self) -> serialport::Parity {
        match self.parity_index {
            1 => serialport::Parity::Odd,
            2 => serialport::Parity::Even,
            _ => serialport::Parity::None,
        }
    }

    fn flow_control(self) -> serialport::FlowControl {
        match self.flow_control_index {
            1 => serialport::FlowControl::Software,
            2 => serialport::FlowControl::Hardware,
            _ => serialport::FlowControl::None,
        }
    }

    fn summary(self) -> String {
        let parity = match self.parity_index {
            1 => 'O',
            2 => 'E',
            _ => 'N',
        };
        format!(
            "{} {}{}{}",
            self.baud_rate(),
            DATA_BITS[self.data_bits_index],
            parity,
            STOP_BITS[self.stop_bits_index]
        )
    }
}

struct SerialTabState {
    id: usize,
    ports: Vec<PortItem>,
    selected_port: usize,
    configuration: SerialConfiguration,
    connected: bool,
    connecting: bool,
    paused: bool,
    hex_mode: bool,
    timestamps: bool,
    auto_scroll: bool,
    terminal_lines: Vec<TerminalLine>,
    clock_tick: usize,
    send_input: Entity<InputState>,
    command_tx: Option<Sender<SerialCommand>>,
    event_tx: Sender<SerialEvent>,
    event_rx: Receiver<SerialEvent>,
    _input_subscription: Subscription,
}

impl SerialTabState {
    fn new(id: usize, send_input: Entity<InputState>, input_subscription: Subscription) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            id,
            ports: discover_ports(),
            selected_port: 0,
            configuration: SerialConfiguration::default(),
            connected: false,
            connecting: false,
            paused: false,
            hex_mode: false,
            timestamps: true,
            auto_scroll: true,
            terminal_lines: vec![TerminalLine {
                time: "14:32:40.018".into(),
                kind: LineKind::System,
                payload: Vec::new(),
                note: Some("请配置串口参数，然后连接设备".into()),
            }],
            clock_tick: 0,
            send_input,
            command_tx: None,
            event_tx,
            event_rx,
            _input_subscription: input_subscription,
        }
    }

    fn selected_port(&self) -> &PortItem {
        &self.ports[self.selected_port.min(self.ports.len().saturating_sub(1))]
    }

    fn status_label(&self) -> &'static str {
        if self.connecting {
            "连接中…"
        } else if self.connected {
            "已连接"
        } else {
            "未连接"
        }
    }

    fn now(&mut self) -> String {
        self.clock_tick = self.clock_tick.wrapping_add(1);
        let seconds = 40 + (self.clock_tick % 19);
        format!("14:32:{seconds:02}.{:03}", (self.clock_tick * 73) % 1000)
    }

    fn push_line(&mut self, kind: LineKind, payload: Vec<u8>, note: Option<String>) {
        let time = self.now();
        self.terminal_lines.push(TerminalLine {
            time,
            kind,
            payload,
            note,
        });
        if self.terminal_lines.len() > 400 {
            self.terminal_lines.drain(..80);
        }
    }

    fn disconnect(&mut self) {
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(SerialCommand::Stop);
        }
        self.connected = false;
        self.connecting = false;
    }
}

impl Drop for SerialTabState {
    fn drop(&mut self) {
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(SerialCommand::Stop);
        }
    }
}

#[derive(Clone)]
struct SerialTabSnapshot {
    id: usize,
    ports: Vec<PortItem>,
    selected_port: usize,
    configuration: SerialConfiguration,
    connected: bool,
    connecting: bool,
    paused: bool,
    hex_mode: bool,
    timestamps: bool,
    auto_scroll: bool,
    terminal_lines: Vec<TerminalLine>,
    send_input: Entity<InputState>,
}

impl From<&SerialTabState> for SerialTabSnapshot {
    fn from(tab: &SerialTabState) -> Self {
        Self {
            id: tab.id,
            ports: tab.ports.clone(),
            selected_port: tab.selected_port,
            configuration: tab.configuration,
            connected: tab.connected,
            connecting: tab.connecting,
            paused: tab.paused,
            hex_mode: tab.hex_mode,
            timestamps: tab.timestamps,
            auto_scroll: tab.auto_scroll,
            terminal_lines: tab.terminal_lines.clone(),
            send_input: tab.send_input.clone(),
        }
    }
}

enum UpdateStatus {
    Checking,
    UpToDate,
    Available(UpdateInfo),
    Downloading { version: String, downloaded: u64 },
    InstallerLaunched,
    Failed,
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

struct ConfigurationButton {
    id: String,
    label: &'static str,
    value: String,
    options: Vec<String>,
    selected_index: usize,
    field: ConfigurationField,
}

pub struct SerialWorkspace {
    tabs: Vec<SerialTabState>,
    active_tab: usize,
    next_tab_id: usize,
    update_status: UpdateStatus,
    update_tx: Sender<UpdateEvent>,
    update_rx: Receiver<UpdateEvent>,
    manual_update_check: bool,
    menu_bar: Entity<AppMenuBar>,
}

impl SerialWorkspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tab = Self::build_tab(1, window, cx);
        let (update_tx, update_rx) = mpsc::channel();
        let menu_bar = AppMenuBar::new(cx);

        let workspace = Self {
            tabs: vec![tab],
            active_tab: 0,
            next_tab_id: 2,
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

    fn build_tab(id: usize, window: &mut Window, cx: &mut Context<Self>) -> SerialTabState {
        let send_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入要发送的数据，例如 AT+VERSION?")
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

    fn add_serial_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let tab = Self::build_tab(id, window, cx);
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        cx.notify();
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

    fn select_configuration(
        &mut self,
        tab_id: usize,
        field: ConfigurationField,
        selected_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let tab = &mut self.tabs[index];
        if tab.connected || tab.connecting {
            return;
        }
        match field {
            ConfigurationField::Port => {
                tab.selected_port = selected_index.min(tab.ports.len().saturating_sub(1));
            }
            ConfigurationField::BaudRate => {
                tab.configuration.baud_index = selected_index.min(BAUD_RATES.len() - 1);
            }
            ConfigurationField::DataBits => {
                tab.configuration.data_bits_index = selected_index.min(DATA_BITS.len() - 1);
            }
            ConfigurationField::StopBits => {
                tab.configuration.stop_bits_index = selected_index.min(STOP_BITS.len() - 1);
            }
            ConfigurationField::Parity => {
                tab.configuration.parity_index = selected_index.min(PARITIES.len() - 1);
            }
            ConfigurationField::FlowControl => {
                tab.configuration.flow_control_index = selected_index.min(FLOW_CONTROLS.len() - 1);
            }
        }
        cx.notify();
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
            Some(format!("扫描完成 · 发现 {} 个设备", tab.ports.len())),
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
                Some(format!("已断开 {device}")),
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
                    "Loopback 会话已就绪 · {}",
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
                Some("请先连接一个串口设备".into()),
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
                            Some("串口已打开，开始接收数据".into()),
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
                            "serialX v{version} 安装程序已打开，请按系统提示完成更新。"
                        ))
                        .title("软件更新"),
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
                        "正在下载 serialX v{version}（已下载 {}）",
                        format_bytes(*downloaded)
                    ))
                    .title("软件更新"),
                    cx,
                );
            }
            UpdateStatus::Checking => {
                self.manual_update_check = true;
                window.push_notification(
                    Notification::info("正在检查 GitHub Releases…").title("软件更新"),
                    cx,
                );
            }
            _ => {
                self.manual_update_check = true;
                self.update_status = UpdateStatus::Checking;
                spawn_update_check(self.update_tx.clone());
                window.push_notification(
                    Notification::info("正在检查 GitHub Releases…").title("软件更新"),
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
                .title(format!("发现 serialX v{latest_version}"))
                .description(format!(
                    "当前版本：v{}\n安装包：{package_name}\n\n是否现在下载并安装？",
                    env!("CARGO_PKG_VERSION")
                ))
                .show_cancel(true)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("下载并安装")
                        .cancel_text("稍后"),
                )
                .on_ok(move |_, window, cx| {
                    let started = workspace
                        .update(cx, |workspace, cx| workspace.begin_update(cx))
                        .ok()
                        .flatten()
                        .is_some();
                    if started {
                        window.push_notification(
                            Notification::info(format!("正在下载 serialX v{version_for_notice}…"))
                                .title("软件更新"),
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
                .title("serialX 已是最新版本")
                .description(format!("当前版本：v{version}"))
                .button_props(DialogButtonProps::default().ok_text("确定"))
        });
    }

    fn show_update_error_dialog(message: String, window: &mut Window, cx: &mut App) {
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .icon(Icon::new(IconName::CircleX).size_5())
                .title("无法检查或安装更新")
                .description(message.clone())
                .button_props(DialogButtonProps::default().ok_text("关闭"))
        });
    }

    fn show_about_dialog(window: &mut Window, cx: &mut App) {
        window.open_alert_dialog(cx, |alert, _, _| {
            alert
                .icon(Icon::new(IconName::SquareTerminal).size_5())
                .title("serialX")
                .description(format!(
                    "版本 {}\n现代串口调试工作台\n\nGNU GPL v3\n© 2026 miskin",
                    env!("CARGO_PKG_VERSION")
                ))
                .show_cancel(true)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("关闭")
                        .cancel_text("查看 GitHub"),
                )
                .on_cancel(|_, _, cx| {
                    cx.open_url(REPOSITORY_URL);
                    true
                })
        });
    }

    fn configuration_button(
        button: ConfigurationButton,
        tab_id: usize,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ConfigurationButton {
            id,
            label,
            value,
            options,
            selected_index,
            field,
        } = button;
        let workspace = cx.weak_entity();
        Button::new(id)
            .outline()
            .small()
            .label(format!("{label}  {value}"))
            .dropdown_caret(true)
            .disabled(disabled)
            .dropdown_menu(move |mut menu, _, _| {
                for (index, option) in options.iter().enumerate() {
                    let workspace = workspace.clone();
                    menu = menu.item(
                        PopupMenuItem::new(option.clone())
                            .checked(index == selected_index)
                            .on_click(move |_, _, cx| {
                                let _ = workspace.update(cx, |workspace, cx| {
                                    workspace.select_configuration(tab_id, field, index, cx);
                                });
                            }),
                    );
                }
                menu
            })
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

    fn render_active_tab(&mut self, tab: SerialTabSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let selected = tab.ports[tab.selected_port.min(tab.ports.len().saturating_sub(1))].clone();
        let locked = tab.connected || tab.connecting;
        let status_label = if tab.connecting {
            "连接中…"
        } else if tab.connected {
            "已连接"
        } else {
            "未连接"
        };
        let connection_color = if tab.connected { GREEN } else { MUTED };
        let tab_id = tab.id;

        let port_options = tab
            .ports
            .iter()
            .map(|port| format!("{} — {}", port.name, port.subtitle))
            .collect();
        let baud_options = BAUD_RATES.iter().map(u32::to_string).collect();
        let data_bit_options = DATA_BITS.iter().map(|value| (*value).into()).collect();
        let stop_bit_options = STOP_BITS.iter().map(|value| (*value).into()).collect();
        let parity_options = PARITIES.iter().map(|value| (*value).into()).collect();
        let flow_options = FLOW_CONTROLS.iter().map(|value| (*value).into()).collect();

        v_flex()
            .flex_1()
            .min_h_0()
            .rounded_b_2xl()
            .border_1()
            .border_t_0()
            .border_color(rgb(LINE))
            .bg(rgb(PANEL))
            .shadow_sm()
            .overflow_hidden()
            .child(
                v_flex()
                    .flex_none()
                    .p_4()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(0xfafaf8))
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(Icon::new(IconName::Settings).size_4())
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("串口配置"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(MUTED))
                                            .child("连接后锁定参数"),
                                    ),
                            )
                            .child(
                                Button::new(("connect-tab", tab_id))
                                    .when(tab.connected, |button| {
                                        button.outline().label("断开连接")
                                    })
                                    .when(!tab.connected, |button| {
                                        button
                                            .custom(
                                                ButtonCustomVariant::new(cx)
                                                    .color(rgb(INK).into())
                                                    .foreground(rgb(0xffffff).into())
                                                    .hover(rgb(NAVY_SOFT).into())
                                                    .active(rgb(NAVY).into())
                                                    .shadow(true),
                                            )
                                            .label(if tab.connecting {
                                                "连接中…"
                                            } else {
                                                "连接设备"
                                            })
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_connection(tab_id, cx)
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .gap_2()
                            .child(Self::configuration_button(
                                ConfigurationButton {
                                    id: format!("port-{tab_id}"),
                                    label: "端口",
                                    value: selected.name.clone(),
                                    options: port_options,
                                    selected_index: tab.selected_port,
                                    field: ConfigurationField::Port,
                                },
                                tab_id,
                                locked,
                                cx,
                            ))
                            .child(Self::configuration_button(
                                ConfigurationButton {
                                    id: format!("baud-{tab_id}"),
                                    label: "波特率",
                                    value: tab.configuration.baud_rate().to_string(),
                                    options: baud_options,
                                    selected_index: tab.configuration.baud_index,
                                    field: ConfigurationField::BaudRate,
                                },
                                tab_id,
                                locked,
                                cx,
                            ))
                            .child(Self::configuration_button(
                                ConfigurationButton {
                                    id: format!("data-bits-{tab_id}"),
                                    label: "数据位",
                                    value: DATA_BITS[tab.configuration.data_bits_index].into(),
                                    options: data_bit_options,
                                    selected_index: tab.configuration.data_bits_index,
                                    field: ConfigurationField::DataBits,
                                },
                                tab_id,
                                locked,
                                cx,
                            ))
                            .child(Self::configuration_button(
                                ConfigurationButton {
                                    id: format!("stop-bits-{tab_id}"),
                                    label: "停止位",
                                    value: STOP_BITS[tab.configuration.stop_bits_index].into(),
                                    options: stop_bit_options,
                                    selected_index: tab.configuration.stop_bits_index,
                                    field: ConfigurationField::StopBits,
                                },
                                tab_id,
                                locked,
                                cx,
                            ))
                            .child(Self::configuration_button(
                                ConfigurationButton {
                                    id: format!("parity-{tab_id}"),
                                    label: "校验位",
                                    value: PARITIES[tab.configuration.parity_index].into(),
                                    options: parity_options,
                                    selected_index: tab.configuration.parity_index,
                                    field: ConfigurationField::Parity,
                                },
                                tab_id,
                                locked,
                                cx,
                            ))
                            .child(Self::configuration_button(
                                ConfigurationButton {
                                    id: format!("flow-{tab_id}"),
                                    label: "流控制",
                                    value: FLOW_CONTROLS[tab.configuration.flow_control_index]
                                        .into(),
                                    options: flow_options,
                                    selected_index: tab.configuration.flow_control_index,
                                    field: ConfigurationField::FlowControl,
                                },
                                tab_id,
                                locked,
                                cx,
                            )),
                    ),
            )
            .child(
                h_flex()
                    .h(px(52.))
                    .flex_none()
                    .px_4()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("实时终端"))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .px_2()
                                    .py_1()
                                    .rounded_full()
                                    .bg(if tab.connected {
                                        rgb(MINT)
                                    } else {
                                        rgb(0xefefec)
                                    })
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
                            .text_color(rgb(MUTED))
                            .child(if tab.hex_mode { "HEX" } else { "ASCII" })
                            .when(tab.paused, |row| row.child("· 已暂停"))
                            .when(!tab.auto_scroll, |row| row.child("· 手动滚动")),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .children(tab.terminal_lines.iter().enumerate().map(|(index, line)| {
                        let badge = match line.kind {
                            LineKind::Rx => "RX",
                            LineKind::Tx => "TX",
                            LineKind::System => "SYS",
                        };
                        let badge_color = match line.kind {
                            LineKind::Rx => GREEN,
                            LineKind::Tx => ORANGE,
                            LineKind::System => MUTED,
                        };
                        h_flex()
                            .id(("terminal-line", index))
                            .items_start()
                            .gap_3()
                            .child(
                                div()
                                    .mt_1()
                                    .w_8()
                                    .flex_none()
                                    .rounded_md()
                                    .bg(rgba((badge_color << 8) | 0x18))
                                    .text_color(rgb(badge_color))
                                    .text_xs()
                                    .text_center()
                                    .child(badge),
                            )
                            .when(tab.timestamps, |row| {
                                row.child(
                                    div()
                                        .w(px(88.))
                                        .flex_none()
                                        .pt_1()
                                        .font_family("SF Mono")
                                        .text_size(px(11.))
                                        .text_color(rgb(MUTED))
                                        .child(line.time.clone()),
                                )
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .pt_1()
                                    .font_family("SF Mono")
                                    .text_size(px(12.))
                                    .text_color(if line.kind == LineKind::System {
                                        rgb(MUTED)
                                    } else {
                                        rgb(INK)
                                    })
                                    .child(Self::format_line_payload(tab.hex_mode, line)),
                            )
                    })),
            )
            .child(
                h_flex()
                    .flex_none()
                    .p_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(div().flex_1().min_w_0().child(Input::new(&tab.send_input)))
                    .child(
                        Button::new(("send", tab_id))
                            .custom(
                                ButtonCustomVariant::new(cx)
                                    .color(rgb(ORANGE).into())
                                    .foreground(rgb(0xffffff).into())
                                    .hover(rgb(0xee633d).into())
                                    .active(rgb(0xd95734).into())
                                    .shadow(true),
                            )
                            .icon(IconName::ArrowUp)
                            .label("发送")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.send_current(tab_id, window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for SerialWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_snapshot = self.active_tab().map(SerialTabSnapshot::from);
        let (title, status_label, connection_color) = self
            .active_tab()
            .map(|tab| {
                (
                    format!("串口 {} · {}", tab.id, tab.selected_port().name),
                    tab.status_label(),
                    if tab.connected { GREEN } else { MUTED },
                )
            })
            .unwrap_or_else(|| ("未打开串口标签".into(), "空闲", MUTED));

        let mut tab_items = Vec::with_capacity(self.tabs.len());
        for tab in &self.tabs {
            let tab_id = tab.id;
            let tab_label = format!("串口 {} · {}", tab.id, tab.selected_port().name);
            let dot_color = if tab.connected { GREEN } else { MUTED };
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
                .rounded_b_2xl()
                .border_1()
                .border_t_0()
                .border_color(rgb(LINE))
                .bg(rgb(PANEL))
                .shadow_sm()
                .gap_3()
                .child(
                    div()
                        .size_12()
                        .rounded_xl()
                        .bg(rgb(0xf3f2ee))
                        .text_color(rgb(MUTED))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::SquareTerminal).size_6()),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("没有打开的串口标签"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child("使用“文件 > 新建串口标签”开始一个会话"),
                )
                .into_any_element()
        };

        v_flex()
            .size_full()
            .bg(rgb(CANVAS))
            .text_color(rgb(INK))
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .px_4()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_4()
                                .items_center()
                                .when(cfg!(not(target_os = "macos")), |row| {
                                    row.child(div().w(px(310.)).h_8().child(self.menu_bar.clone()))
                                })
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("serialX")
                                        .child(div().text_color(rgb(MUTED)).child("/"))
                                        .child(div().text_color(rgb(MUTED)).child(title)),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(div().size_2().rounded_full().bg(rgb(connection_color)))
                                .child(div().text_xs().text_color(rgb(MUTED)).child(status_label)),
                        ),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .p_5()
                    .child(tab_bar)
                    .child(content),
            )
    }
}

fn discover_ports() -> Vec<PortItem> {
    let mut ports = vec![PortItem {
        name: "Loopback".into(),
        subtitle: "内置演示设备".into(),
        is_demo: true,
    }];

    if let Ok(detected) = serialport::available_ports() {
        ports.extend(detected.into_iter().map(|port| PortItem {
            subtitle: match port.port_type {
                serialport::SerialPortType::UsbPort(info) => {
                    info.product.unwrap_or_else(|| "USB Serial".into())
                }
                serialport::SerialPortType::BluetoothPort => "Bluetooth Serial".into(),
                serialport::SerialPortType::PciPort => "PCI Serial".into(),
                serialport::SerialPortType::Unknown => "Serial Device".into(),
            },
            name: port.port_name,
            is_demo: false,
        }));
    }
    ports
}

fn spawn_serial_worker(
    port_name: String,
    configuration: SerialConfiguration,
    commands: Receiver<SerialCommand>,
    events: Sender<SerialEvent>,
) {
    thread::spawn(move || {
        let mut port = match serialport::new(&port_name, configuration.baud_rate())
            .data_bits(configuration.data_bits())
            .stop_bits(configuration.stop_bits())
            .parity(configuration.parity())
            .flow_control(configuration.flow_control())
            .timeout(Duration::from_millis(24))
            .open()
        {
            Ok(port) => port,
            Err(error) => {
                let _ = events.send(SerialEvent::Error(format!("无法打开 {port_name}: {error}")));
                return;
            }
        };

        let _ = events.send(SerialEvent::Connected);
        let mut buffer = [0_u8; 2048];
        loop {
            while let Ok(command) = commands.try_recv() {
                match command {
                    SerialCommand::Write(bytes) => {
                        if let Err(error) = port.write_all(&bytes) {
                            let _ = events.send(SerialEvent::Error(format!("发送失败: {error}")));
                            return;
                        }
                    }
                    SerialCommand::Stop => {
                        let _ = events.send(SerialEvent::Closed);
                        return;
                    }
                }
            }

            match port.read(&mut buffer) {
                Ok(count) if count > 0 => {
                    let _ = events.send(SerialEvent::Data(buffer[..count].to_vec()));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    let _ = events.send(SerialEvent::Error(format!("读取失败: {error}")));
                    return;
                }
            }
        }
    });
}

fn parse_hex(value: &str) -> Option<Vec<u8>> {
    let compact: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.is_empty() || !compact.len().is_multiple_of(2) {
        return None;
    }
    (0..compact.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&compact[index..index + 2], 16).ok())
        .collect()
}

fn demo_response(command: &str) -> Vec<u8> {
    if command.trim().eq_ignore_ascii_case("AT+STATUS?") {
        b"+STATUS:READY,RSSI=-48,TEMP=24.6\r\nOK\r\n".to_vec()
    } else if command.trim().eq_ignore_ascii_case("AT+VERSION?") {
        b"+VERSION:SerialX-Demo/1.4.2\r\nOK\r\n".to_vec()
    } else {
        format!("ECHO:{}\r\nOK\r\n", command.trim()).into_bytes()
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

fn application_menus() -> Vec<Menu> {
    vec![
        Menu::new("serialX").items([
            MenuItem::action("关于 serialX", ShowAbout),
            MenuItem::separator(),
            MenuItem::action("退出 serialX", QuitSerialX),
        ]),
        Menu::new("文件").items([
            MenuItem::action("新建串口标签", NewSerialTab),
            MenuItem::action("关闭当前标签", CloseSerialTab),
        ]),
        Menu::new("会话").items([
            MenuItem::action("连接 / 断开", ToggleConnection),
            MenuItem::action("刷新串口列表", RefreshPorts),
            MenuItem::separator(),
            MenuItem::action("暂停 / 继续接收", TogglePause),
            MenuItem::action("清空终端", ClearTerminal),
        ]),
        Menu::new("显示").items([
            MenuItem::action("ASCII / HEX", ToggleHex),
            MenuItem::action("显示 / 隐藏时间戳", ToggleTimestamps),
            MenuItem::action("自动 / 手动滚动", ToggleAutoScroll),
        ]),
        Menu::new("帮助").items([
            MenuItem::action("检查更新…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("关于 serialX", ShowAbout),
        ]),
    ]
}

fn configure_application_menus(cx: &mut App) {
    cx.on_action(|_: &QuitSerialX, cx| cx.quit());
    GlobalState::global_mut(cx)
        .set_app_menus(application_menus().into_iter().map(Menu::owned).collect());
    cx.set_menus(application_menus());
}

fn bind_window_actions(workspace: &Entity<SerialWorkspace>, window: &mut Window, cx: &mut App) {
    let window_handle = window.window_handle();
    let view = workspace.downgrade();
    cx.on_action(move |_: &NewSerialTab, cx| {
        let _ = window_handle.update(cx, |_, window, cx| {
            let _ = view.update(cx, |view, cx| view.add_serial_tab(window, cx));
        });
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &CloseSerialTab, cx| {
        let _ = view.update(cx, |view, cx| view.close_active_tab(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &RefreshPorts, cx| {
        let _ = view.update(cx, |view, cx| view.refresh_ports(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &ToggleConnection, cx| {
        let _ = view.update(cx, |view, cx| view.toggle_active_connection(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &TogglePause, cx| {
        let _ = view.update(cx, |view, cx| view.toggle_pause(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &ClearTerminal, cx| {
        let _ = view.update(cx, |view, cx| view.clear_terminal(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &ToggleHex, cx| {
        let _ = view.update(cx, |view, cx| view.toggle_hex(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &ToggleTimestamps, cx| {
        let _ = view.update(cx, |view, cx| view.toggle_timestamps(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &ToggleAutoScroll, cx| {
        let _ = view.update(cx, |view, cx| view.toggle_auto_scroll(cx));
    });

    let window_handle = window.window_handle();
    let view = workspace.downgrade();
    cx.on_action(move |_: &CheckForUpdates, cx| {
        let _ = window_handle.update(cx, |_, window, cx| {
            let _ = view.update(cx, |view, cx| {
                view.check_for_updates_from_menu(window, cx);
            });
        });
    });

    let window_handle = window.window_handle();
    cx.on_action(move |_: &ShowAbout, cx| {
        let _ = window_handle.update(cx, |_, window, cx| {
            SerialWorkspace::show_about_dialog(window, cx);
        });
    });
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
                let workspace = cx.new(|cx| SerialWorkspace::new(window, cx));
                bind_window_actions(&workspace, window, cx);
                cx.new(|cx| Root::new(workspace, window, cx).bg(rgb(CANVAS)))
            })
            .expect("failed to open serialX window");
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use super::parse_hex;

    #[test]
    fn parses_hex_with_or_without_spaces() {
        assert_eq!(parse_hex("41 54 0D 0A"), Some(b"AT\r\n".to_vec()));
        assert_eq!(parse_hex("41540d0a"), Some(b"AT\r\n".to_vec()));
        assert_eq!(parse_hex("123"), None);
        assert_eq!(parse_hex("GG"), None);
    }
}
