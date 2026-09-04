#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use gpui_kit::assets::Assets;
use gpui_kit::component::{
    Icon, IconName, Root, Sizable, TitleBar,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use smol::Timer;

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
const BLUE: u32 = 0x4d75ff;

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

pub struct SerialWorkspace {
    ports: Vec<PortItem>,
    selected_port: usize,
    baud_rates: Vec<u32>,
    baud_index: usize,
    connected: bool,
    connecting: bool,
    paused: bool,
    hex_mode: bool,
    timestamps: bool,
    auto_scroll: bool,
    terminal_lines: Vec<TerminalLine>,
    rx_bytes: usize,
    tx_bytes: usize,
    clock_tick: usize,
    send_input: Entity<InputState>,
    command_tx: Option<Sender<SerialCommand>>,
    event_tx: Sender<SerialEvent>,
    event_rx: Receiver<SerialEvent>,
    _subscriptions: Vec<Subscription>,
}

impl SerialWorkspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let send_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入要发送的数据，例如 AT+VERSION?")
                .default_value("AT+STATUS?")
        });
        let (event_tx, event_rx) = mpsc::channel();

        let mut workspace = Self {
            ports: discover_ports(),
            selected_port: 0,
            baud_rates: vec![9_600, 19_200, 38_400, 57_600, 115_200, 230_400],
            baud_index: 4,
            connected: true,
            connecting: false,
            paused: false,
            hex_mode: false,
            timestamps: true,
            auto_scroll: true,
            terminal_lines: demo_lines(),
            rx_bytes: 18_642,
            tx_bytes: 4_291,
            clock_tick: 8,
            send_input: send_input.clone(),
            command_tx: None,
            event_tx,
            event_rx,
            _subscriptions: Vec::new(),
        };

        workspace._subscriptions.push(cx.subscribe_in(
            &send_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.send_current(window, cx);
                }
            },
        ));

        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(80)).await;
                if this
                    .update(cx, |this, cx| {
                        if this.drain_serial_events() {
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

        workspace
    }

    fn selected(&self) -> &PortItem {
        &self.ports[self.selected_port.min(self.ports.len().saturating_sub(1))]
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

    fn toggle_connection(&mut self, cx: &mut Context<Self>) {
        if self.connected || self.connecting {
            let device = self.selected().name.clone();
            self.disconnect();
            self.push_line(
                LineKind::System,
                Vec::new(),
                Some(format!("已断开 {device}")),
            );
            cx.notify();
            return;
        }

        let selected = self.selected().clone();
        if selected.is_demo {
            self.connected = true;
            self.push_line(
                LineKind::System,
                Vec::new(),
                Some("Loopback 会话已就绪 · 延迟 2 ms".into()),
            );
            cx.notify();
            return;
        }

        let baud = self.baud_rates[self.baud_index];
        let (command_tx, command_rx) = mpsc::channel();
        self.command_tx = Some(command_tx);
        self.connecting = true;
        spawn_serial_worker(selected.name, baud, command_rx, self.event_tx.clone());
        cx.notify();
    }

    fn send_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.send_input.read(cx).value().trim().to_string();
        if value.is_empty() {
            return;
        }

        if !self.connected {
            self.push_line(
                LineKind::System,
                Vec::new(),
                Some("请先连接一个串口设备".into()),
            );
            cx.notify();
            return;
        }

        let mut bytes = if self.hex_mode {
            parse_hex(&value).unwrap_or_else(|| value.as_bytes().to_vec())
        } else {
            value.as_bytes().to_vec()
        };
        bytes.extend_from_slice(b"\r\n");

        self.tx_bytes += bytes.len();
        self.push_line(LineKind::Tx, bytes.clone(), None);

        if self.selected().is_demo {
            let response = demo_response(&value);
            self.rx_bytes += response.len();
            self.push_line(LineKind::Rx, response, None);
        } else if let Some(tx) = &self.command_tx {
            let _ = tx.send(SerialCommand::Write(bytes));
        }

        self.send_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    fn drain_serial_events(&mut self) -> bool {
        let events: Vec<_> = self.event_rx.try_iter().collect();
        if events.is_empty() {
            return false;
        }

        for event in events {
            match event {
                SerialEvent::Connected => {
                    self.connecting = false;
                    self.connected = true;
                    self.push_line(
                        LineKind::System,
                        Vec::new(),
                        Some("串口已打开，开始接收数据".into()),
                    );
                }
                SerialEvent::Data(bytes) => {
                    self.rx_bytes += bytes.len();
                    if !self.paused {
                        self.push_line(LineKind::Rx, bytes, None);
                    }
                }
                SerialEvent::Error(message) => {
                    self.disconnect();
                    self.push_line(LineKind::System, Vec::new(), Some(message));
                }
                SerialEvent::Closed => {
                    self.disconnect();
                }
            }
        }
        true
    }

    fn refresh_ports(&mut self, cx: &mut Context<Self>) {
        let current = self.selected().name.clone();
        self.ports = discover_ports();
        self.selected_port = self
            .ports
            .iter()
            .position(|port| port.name == current)
            .unwrap_or(0);
        self.push_line(
            LineKind::System,
            Vec::new(),
            Some(format!("扫描完成 · 发现 {} 个设备", self.ports.len())),
        );
        cx.notify();
    }

    fn render_logo() -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(
                div()
                    .size_9()
                    .rounded_lg()
                    .bg(rgb(ORANGE))
                    .text_color(rgb(0xffffff))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(IconName::SquareTerminal).size_5()),
            )
            .child(
                v_flex()
                    .gap_px()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Serial X"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgba(0xffffff88))
                            .child("DEVICE CONSOLE"),
                    ),
            )
    }

    fn toggle_pill(
        id: &'static str,
        label: &'static str,
        active: bool,
        on_click: impl Fn(&mut Self, &ClickEvent, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .h_7()
            .px_3()
            .rounded_full()
            .border_1()
            .border_color(if active { rgb(INK) } else { rgb(LINE) })
            .bg(if active { rgb(INK) } else { rgb(PANEL) })
            .text_color(if active { rgb(0xffffff) } else { rgb(MUTED) })
            .text_xs()
            .flex()
            .items_center()
            .cursor_pointer()
            .hover(|style| style.opacity(0.82))
            .child(label)
            .on_click(cx.listener(on_click))
    }

    fn parameter_row(label: &'static str, value: String) -> impl IntoElement {
        h_flex()
            .h_9()
            .justify_between()
            .border_b_1()
            .border_color(rgb(LINE))
            .child(div().text_sm().text_color(rgb(MUTED)).child(label))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(INK))
                    .child(value),
            )
    }

    fn format_line_payload(&self, line: &TerminalLine) -> String {
        if let Some(note) = &line.note {
            return note.clone();
        }
        if self.hex_mode {
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
}

impl Render for SerialWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected().clone();
        let baud = self.baud_rates[self.baud_index];
        let status_label = if self.connecting {
            "连接中…"
        } else if self.connected {
            "已连接"
        } else {
            "未连接"
        };
        let connection_color = if self.connected { GREEN } else { MUTED };

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
                                .gap_2()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child("Serial X")
                                .child(div().text_color(rgb(MUTED)).child("/"))
                                .child(
                                    div()
                                        .text_color(rgb(MUTED))
                                        .child(selected.name.clone()),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .size_2()
                                        .rounded_full()
                                        .bg(rgb(connection_color)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(MUTED))
                                        .child(status_label),
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
                            .w(px(250.))
                            .h_full()
                            .flex_none()
                            .bg(rgb(NAVY))
                            .text_color(rgb(0xffffff))
                            .p_4()
                            .gap_5()
                            .child(Self::render_logo())
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgba(0xffffff78))
                                            .child("可用设备"),
                                    )
                                    .child(
                                        Button::new("refresh-ports")
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::RotateCw)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.refresh_ports(cx)
                                            })),
                                    ),
                            )
                            .child(
                                v_flex().gap_2().children(self.ports.iter().enumerate().map(
                                    |(index, port)| {
                                        let active = index == self.selected_port;
                                        let name = port.name.clone();
                                        let subtitle = port.subtitle.clone();
                                        div()
                                            .id(("port", index))
                                            .w_full()
                                            .rounded_xl()
                                            .p_3()
                                            .gap_3()
                                            .flex()
                                            .items_center()
                                            .cursor_pointer()
                                            .bg(if active {
                                                rgb(NAVY_SOFT)
                                            } else {
                                                rgba(0x00000000)
                                            })
                                            .border_1()
                                            .border_color(if active {
                                                rgba(0xffffff18)
                                            } else {
                                                rgba(0x00000000)
                                            })
                                            .hover(|style| style.bg(rgb(NAVY_SOFT)))
                                            .child(
                                                div()
                                                    .size_9()
                                                    .flex_none()
                                                    .rounded_lg()
                                                    .bg(if active {
                                                        rgb(MINT)
                                                    } else {
                                                        rgba(0xffffff12)
                                                    })
                                                    .text_color(if active {
                                                        rgb(GREEN)
                                                    } else {
                                                        rgba(0xffffffaa)
                                                    })
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .child(Icon::new(IconName::Network).size_4()),
                                            )
                                            .child(
                                                v_flex()
                                                    .min_w_0()
                                                    .gap_px()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .truncate()
                                                            .child(name),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgba(0xffffff70))
                                                            .truncate()
                                                            .child(subtitle),
                                                    ),
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if this.selected_port != index {
                                                    this.disconnect();
                                                    this.selected_port = index;
                                                    cx.notify();
                                                }
                                            }))
                                    },
                                )),
                            )
                            .child(div().flex_1())
                            .child(
                                v_flex()
                                    .rounded_xl()
                                    .bg(rgba(0xffffff0d))
                                    .p_3()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgba(0xffffff78))
                                                    .child("会话流量"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(MINT))
                                                    .child("LIVE"),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_4()
                                            .child(
                                                v_flex()
                                                    .child(
                                                        div()
                                                            .text_base()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .child(format_bytes(self.rx_bytes)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgba(0xffffff68))
                                                            .child("接收"),
                                                    ),
                                            )
                                            .child(
                                                v_flex()
                                                    .child(
                                                        div()
                                                            .text_base()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .child(format_bytes(self.tx_bytes)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgba(0xffffff68))
                                                            .child("发送"),
                                                    ),
                                            ),
                                    ),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .p_5()
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_h_0()
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .h_full()
                                            .rounded_2xl()
                                            .border_1()
                                            .border_color(rgb(LINE))
                                            .bg(rgb(PANEL))
                                            .shadow_sm()
                                            .overflow_hidden()
                                            .child(
                                                h_flex()
                                                    .h(px(56.))
                                                    .flex_none()
                                                    .px_4()
                                                    .justify_between()
                                                    .border_b_1()
                                                    .border_color(rgb(LINE))
                                                    .child(
                                                        h_flex()
                                                            .gap_2()
                                                            .child(
                                                                div()
                                                                    .font_weight(FontWeight::SEMIBOLD)
                                                                    .child("实时终端"),
                                                            )
                                                            .child(
                                                                div()
                                                                    .px_2()
                                                                    .py_1()
                                                                    .rounded_full()
                                                                    .bg(rgb(MINT))
                                                                    .text_color(rgb(GREEN))
                                                                    .text_xs()
                                                                    .child(status_label),
                                                            ),
                                                    )
                                                    .child(
                                                        h_flex()
                                                            .gap_2()
                                                            .child(Self::toggle_pill(
                                                                "toggle-hex",
                                                                "HEX",
                                                                self.hex_mode,
                                                                |this, _, _, cx| {
                                                                    this.hex_mode = !this.hex_mode;
                                                                    cx.notify();
                                                                },
                                                                cx,
                                                            ))
                                                            .child(Self::toggle_pill(
                                                                "toggle-time",
                                                                "时间戳",
                                                                self.timestamps,
                                                                |this, _, _, cx| {
                                                                    this.timestamps = !this.timestamps;
                                                                    cx.notify();
                                                                },
                                                                cx,
                                                            ))
                                                            .child(
                                                                Button::new("pause-terminal")
                                                                    .ghost()
                                                                    .xsmall()
                                                                    .icon(if self.paused {
                                                                        IconName::Play
                                                                    } else {
                                                                        IconName::Pause
                                                                    })
                                                                    .on_click(cx.listener(
                                                                        |this, _, _, cx| {
                                                                            this.paused =
                                                                                !this.paused;
                                                                            cx.notify();
                                                                        },
                                                                    )),
                                                            )
                                                            .child(
                                                                Button::new("clear-terminal")
                                                                    .ghost()
                                                                    .xsmall()
                                                                    .icon(IconName::Delete)
                                                                    .on_click(cx.listener(
                                                                        |this, _, _, cx| {
                                                                            this.terminal_lines
                                                                                .clear();
                                                                            cx.notify();
                                                                        },
                                                                    )),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                v_flex()
                                                    .id("terminal-scroll")
                                                    .flex_1()
                                                    .min_h_0()
                                                    .overflow_y_scroll()
                                                    .bg(rgb(0xfdfdfc))
                                                    .px_4()
                                                    .py_3()
                                                    .gap_1()
                                                    .children(self.terminal_lines.iter().map(
                                                        |line| {
                                                            let (tag, color, tint) = match line.kind {
                                                                LineKind::Rx => {
                                                                    ("RX", GREEN, 0xe9f8ed)
                                                                }
                                                                LineKind::Tx => {
                                                                    ("TX", BLUE, 0xeef1ff)
                                                                }
                                                                LineKind::System => {
                                                                    ("•", MUTED, 0xf1f1ef)
                                                                }
                                                            };
                                                            h_flex()
                                                                .min_h_8()
                                                                .items_start()
                                                                .gap_3()
                                                                .text_sm()
                                                                .child(
                                                                    div()
                                                                        .w_8()
                                                                        .h_6()
                                                                        .flex_none()
                                                                        .rounded_md()
                                                                        .bg(rgb(tint))
                                                                        .text_color(rgb(color))
                                                                        .text_xs()
                                                                        .font_weight(FontWeight::SEMIBOLD)
                                                                        .flex()
                                                                        .items_center()
                                                                        .justify_center()
                                                                        .child(tag),
                                                                )
                                                                .when(self.timestamps, |row| {
                                                                    row.child(
                                                                        div()
                                                                            .w(px(88.))
                                                                            .flex_none()
                                                                            .pt_1()
                                                                            .text_xs()
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
                                                                        .text_color(if line.kind
                                                                            == LineKind::System
                                                                        {
                                                                            rgb(MUTED)
                                                                        } else {
                                                                            rgb(INK)
                                                                        })
                                                                        .child(
                                                                            self.format_line_payload(
                                                                                line,
                                                                            ),
                                                                        ),
                                                                )
                                                        },
                                                    )),
                                            )
                                            .child(
                                                h_flex()
                                                    .flex_none()
                                                    .p_3()
                                                    .gap_2()
                                                    .border_t_1()
                                                    .border_color(rgb(LINE))
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .child(Input::new(&self.send_input)),
                                                    )
                                                    .child(
                                                        Button::new("send")
                                                            .custom(
                                                                ButtonCustomVariant::new(cx)
                                                                    .color(rgb(ORANGE).into())
                                                                    .foreground(
                                                                        rgb(0xffffff).into(),
                                                                    )
                                                                    .hover(rgb(0xee633d).into())
                                                                    .active(rgb(0xd95734).into())
                                                                    .shadow(true),
                                                            )
                                                            .icon(IconName::ArrowUp)
                                                            .label("发送")
                                                            .on_click(cx.listener(
                                                                |this, _, window, cx| {
                                                                    this.send_current(window, cx)
                                                                },
                                                            )),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .w(px(268.))
                                            .h_full()
                                            .flex_none()
                                            .gap_4()
                                            .child(
                                                v_flex()
                                                    .rounded_2xl()
                                                    .border_1()
                                                    .border_color(rgb(LINE))
                                                    .bg(rgb(PANEL))
                                                    .shadow_sm()
                                                    .p_4()
                                                    .gap_3()
                                                    .child(
                                                        h_flex()
                                                            .justify_between()
                                                            .child(
                                                                div()
                                                                    .font_weight(FontWeight::SEMIBOLD)
                                                                    .child("连接设置"),
                                                            )
                                                            .child(
                                                                Icon::new(IconName::Settings)
                                                                    .size_4()
                                                                    .text_color(rgb(MUTED)),
                                                            ),
                                                    )
                                                    .child(Self::parameter_row(
                                                        "端口",
                                                        selected.name.clone(),
                                                    ))
                                                    .child(
                                                        h_flex()
                                                            .id("cycle-baud")
                                                            .h_9()
                                                            .justify_between()
                                                            .border_b_1()
                                                            .border_color(rgb(LINE))
                                                            .cursor_pointer()
                                                            .hover(|style| {
                                                                style.text_color(rgb(BLUE))
                                                            })
                                                            .child(
                                                                div()
                                                                    .text_sm()
                                                                    .text_color(rgb(MUTED))
                                                                    .child("波特率"),
                                                            )
                                                            .child(
                                                                h_flex()
                                                                    .gap_1()
                                                                    .child(
                                                                        div()
                                                                            .text_sm()
                                                                            .font_weight(FontWeight::MEDIUM)
                                                                            .child(baud.to_string()),
                                                                    )
                                                                    .child(
                                                                        Icon::new(
                                                                            IconName::ChevronDown,
                                                                        )
                                                                        .size_3(),
                                                                    ),
                                                            )
                                                            .on_click(cx.listener(
                                                                |this, _, _, cx| {
                                                                    if !this.connected {
                                                                        this.baud_index =
                                                                            (this.baud_index + 1)
                                                                                % this
                                                                                    .baud_rates
                                                                                    .len();
                                                                        cx.notify();
                                                                    }
                                                                },
                                                            )),
                                                    )
                                                    .child(Self::parameter_row(
                                                        "数据位",
                                                        "8 bits".into(),
                                                    ))
                                                    .child(Self::parameter_row(
                                                        "停止位",
                                                        "1 bit".into(),
                                                    ))
                                                    .child(Self::parameter_row(
                                                        "校验位",
                                                        "None".into(),
                                                    ))
                                                    .child(Self::parameter_row(
                                                        "流控制",
                                                        "None".into(),
                                                    ))
                                                    .child(
                                                        Button::new("connect")
                                                            .w_full()
                                                            .when(self.connected, |button| {
                                                                button.outline().label("断开连接")
                                                            })
                                                            .when(!self.connected, |button| {
                                                                button
                                                                    .custom(
                                                                        ButtonCustomVariant::new(cx)
                                                                            .color(rgb(INK).into())
                                                                            .foreground(
                                                                                rgb(0xffffff)
                                                                                    .into(),
                                                                            )
                                                                            .hover(
                                                                                rgb(NAVY_SOFT)
                                                                                    .into(),
                                                                            )
                                                                            .active(
                                                                                rgb(NAVY).into(),
                                                                            )
                                                                            .shadow(true),
                                                                    )
                                                                    .label(if self.connecting {
                                                                        "连接中…"
                                                                    } else {
                                                                        "连接设备"
                                                                    })
                                                            })
                                                            .on_click(cx.listener(
                                                                |this, _, _, cx| {
                                                                    this.toggle_connection(cx)
                                                                },
                                                            )),
                                                    ),
                                            )
                                            .child(
                                                v_flex()
                                                    .rounded_2xl()
                                                    .bg(rgb(NAVY))
                                                    .text_color(rgb(0xffffff))
                                                    .p_4()
                                                    .gap_3()
                                                    .child(
                                                        h_flex()
                                                            .justify_between()
                                                            .child(
                                                                div()
                                                                    .text_sm()
                                                                    .font_weight(FontWeight::SEMIBOLD)
                                                                    .child("会话选项"),
                                                            )
                                                            .child(
                                                                div()
                                                                    .size_2()
                                                                    .rounded_full()
                                                                    .bg(rgb(MINT)),
                                                            ),
                                                    )
                                                    .child(
                                                        h_flex()
                                                            .justify_between()
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(rgba(0xffffff88))
                                                                    .child("自动滚动"),
                                                            )
                                                            .child(
                                                                div()
                                                                    .id("auto-scroll")
                                                                    .w_9()
                                                                    .h_5()
                                                                    .p_px()
                                                                    .rounded_full()
                                                                    .cursor_pointer()
                                                                    .bg(if self.auto_scroll {
                                                                        rgb(MINT)
                                                                    } else {
                                                                        rgba(0xffffff22)
                                                                    })
                                                                    .flex()
                                                                    .justify_end()
                                                                    .when(!self.auto_scroll, |el| {
                                                                        el.justify_start()
                                                                    })
                                                                    .child(
                                                                        div()
                                                                            .size_4()
                                                                            .rounded_full()
                                                                            .bg(rgb(0xffffff)),
                                                                    )
                                                                    .on_click(cx.listener(
                                                                        |this, _, _, cx| {
                                                                            this.auto_scroll =
                                                                                !this.auto_scroll;
                                                                            cx.notify();
                                                                        },
                                                                    )),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgba(0xffffff72))
                                                            .child(
                                                                "Enter 快速发送 · Shift + Enter 换行\n数据仅在本机处理",
                                                            ),
                                                    ),
                                            ),
                                    ),
                            ),
                    ),
            )
    }
}

impl Drop for SerialWorkspace {
    fn drop(&mut self) {
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(SerialCommand::Stop);
        }
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
    baud: u32,
    commands: Receiver<SerialCommand>,
    events: Sender<SerialEvent>,
) {
    thread::spawn(move || {
        let mut port = match serialport::new(&port_name, baud)
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

fn demo_lines() -> Vec<TerminalLine> {
    vec![
        TerminalLine {
            time: "14:32:40.018".into(),
            kind: LineKind::System,
            payload: vec![],
            note: Some("Loopback 会话已就绪 · 115200 8N1".into()),
        },
        TerminalLine {
            time: "14:32:41.106".into(),
            kind: LineKind::Tx,
            payload: b"AT+VERSION?\r\n".to_vec(),
            note: None,
        },
        TerminalLine {
            time: "14:32:41.108".into(),
            kind: LineKind::Rx,
            payload: b"+VERSION:SerialX-Demo/1.4.2\r\nOK\r\n".to_vec(),
            note: None,
        },
        TerminalLine {
            time: "14:32:44.221".into(),
            kind: LineKind::Rx,
            payload: b"$SENSOR,24.6,48.2,3.301*7A\r\n".to_vec(),
            note: None,
        },
        TerminalLine {
            time: "14:32:47.073".into(),
            kind: LineKind::Tx,
            payload: b"AT+STATUS?\r\n".to_vec(),
            note: None,
        },
        TerminalLine {
            time: "14:32:47.075".into(),
            kind: LineKind::Rx,
            payload: b"+STATUS:READY,RSSI=-48,TEMP=24.6\r\nOK\r\n".to_vec(),
            note: None,
        },
    ]
}

fn format_bytes(bytes: usize) -> String {
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

        let mut options = TitleBar::window_options();
        options.window_bounds = Some(WindowBounds::centered(size(px(1280.), px(800.)), cx));
        options.window_min_size = Some(size(px(1024.), px(680.)));

        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                let workspace = cx.new(|cx| SerialWorkspace::new(window, cx));
                cx.new(|cx| Root::new(workspace, window, cx).bg(rgb(CANVAS)))
            })
            .expect("failed to open Serial X window");
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn parses_spaced_hex() {
        assert_eq!(parse_hex("41 54 0D 0A"), Some(b"AT\r\n".to_vec()));
        assert_eq!(parse_hex("123"), None);
        assert_eq!(parse_hex("GG"), None);
    }

    #[test]
    fn formats_byte_counts() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(18_642), "18.2 KB");
    }
}
