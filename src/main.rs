#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app_icon;
mod app_menu;
mod commands;
mod configuration;
mod controls;
mod filter;
mod find;
mod groups;
mod highlight;
mod icons;
mod mask;
mod presets;
mod serial;
mod settings;
mod sidebar;
mod terminal;
mod theme;
mod title_bar;
mod updater;
mod workbench;

use std::{
    collections::HashSet,
    ops::Range,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use app_icon::apply_application_icon;
use app_menu::{bind_window_actions, configure_application_menus, prepare_application_menus};
use gpui_kit::component::{
    Icon, IconName, Root, TitleBar, WindowExt,
    dialog::DialogButtonProps,
    h_flex,
    input::{InputEvent, InputState},
    menu::AppMenuBar,
    notification::Notification,
    resizable::{ResizableState, h_resizable, resizable_panel},
    v_flex,
};
use gpui_kit::*;
use icons::WorkbenchAssets;
use presets::PresetStore;
use serial::*;
use sidebar::{
    ListSearch, SEND_PLACEHOLDER, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH,
    panel_divider,
};
use smol::Timer;
use theme::{InterfaceTheme, Typography, apply_interface_theme, resolve_fonts};
use terminal::{GridCell, SelectionKind, key_bytes};
use title_bar::{FILTER_PLACEHOLDER, TITLE_BAR_HEIGHT, traffic_light_position};
use workbench::TerminalMetrics;
use updater::{
    CheckResult, InstallError, ReadyUpdate, UpdateEvent, UpdateInfo, open_package, relaunch,
    spawn_update_check, spawn_update_install,
};

const REPOSITORY_URL: &str = "https://github.com/miskin-lee/serialX";
/// Half a period of the cursor's blink: this long on, this long off.
const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// How long a read that leaves the cursor at the head of a line is kept
/// back for what follows. A shell answers Enter with the newline at once
/// and its prompt a moment later, in a read of its own — bash on a board
/// behind a USB-UART bridge was measured at 10 ms between the two — and
/// shown as they come, the cursor lands at the head of the line and then
/// jumps to the prompt, at every keystroke. Kept back, the two go in
/// together. A read nothing follows — a line logged, a progress bar's
/// return — goes in on its own when the hold runs out, which is too soon
/// to be seen; the hold is long enough for a slower board's shell.
const LINE_HOLD: Duration = Duration::from_millis(80);

enum UpdateStatus {
    Checking,
    UpToDate,
    Available(UpdateInfo),
    Downloading { version: String, downloaded: u64 },
    /// The package is verified and is being put in place.
    Installing { version: String },
    /// In place: runs on the next start, or now if the user relaunches.
    Ready(ReadyUpdate),
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
    /// Panel layout. Kept in memory rather than in `PresetStore`, because a
    /// collapsed panel describes this sitting, not this workspace.
    side_panel_collapsed: bool,
    sessions_collapsed: bool,
    commands_collapsed: bool,
    /// The saved groups folded to their row, by id. In memory for the same
    /// reason as the sections.
    collapsed_groups: HashSet<u64>,
    /// The saved session last clicked, picked out in the list until another
    /// is; a double-click is what opens one.
    selected_saved: Option<u64>,
    /// The dragged width of the side panel. Owned here rather than by the
    /// resizable group, so collapsing to the rail and back keeps the width.
    panel_layout: Entity<ResizableState>,
    /// The composer in the side panel: one box for the workspace, sending
    /// to whichever tab is in front, so what was typed survives a switch.
    send_input: Entity<InputState>,
    _send_subscription: Subscription,
    /// The search box over the Quick send list.
    command_search: ListSearch,
    /// The terminal log as a place to type: while it holds focus, keys go to
    /// the port of the tab in front.
    terminal_focus: FocusHandle,
    /// Text an input method is still composing over the terminal. Nothing
    /// is sent until the composition is committed.
    composing: Option<String>,
    /// The terminal's cell size as last laid out, for placing the input
    /// method's candidate window under the cursor and for telling which
    /// cell a press landed on.
    terminal_metrics: TerminalMetrics,
    /// Whether a selection is being dragged out over the terminal: from
    /// the press that began it to the release that ends it.
    selecting: bool,
    /// Wheel travel short of a whole line, carried to the next event.
    scroll_remainder: f32,
    /// The cursor's blink: whether it is in its visible half, whether a
    /// timer is running it, and which timer — a keystroke starts a new one
    /// and the old one, when it fires, sees it is stale and does nothing.
    cursor_shown: bool,
    blinking: bool,
    blink_epoch: usize,
    /// Whether a frame is already on its way for a find scan the interval
    /// held back, so one wait is enough.
    find_refresh_pending: bool,
}

impl SerialWorkspace {
    fn new(interface_theme: InterfaceTheme, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (update_tx, update_rx) = mpsc::channel();
        let menu_bar = AppMenuBar::new(cx);
        let panel_layout = cx.new(|_| ResizableState::default());
        let send_input = cx.new(|cx| InputState::new(window, cx).placeholder(SEND_PLACEHOLDER));
        let send_subscription = cx.subscribe_in(
            &send_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.send_to_active_tab(cx);
                }
            },
        );

        let command_search = ListSearch::new(window, cx);

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
            side_panel_collapsed: false,
            sessions_collapsed: false,
            commands_collapsed: false,
            collapsed_groups: HashSet::new(),
            selected_saved: None,
            panel_layout,
            send_input,
            _send_subscription: send_subscription,
            command_search,
            terminal_focus: cx.focus_handle(),
            composing: None,
            terminal_metrics: TerminalMetrics::default(),
            selecting: false,
            scroll_remainder: 0.,
            cursor_shown: true,
            blinking: false,
            blink_epoch: 0,
            find_refresh_pending: false,
        };

        // The cursor is drawn hollow while another window is in front, so
        // the terminal has to redraw when this one comes and goes.
        cx.observe_window_activation(window, |_, _, cx| cx.notify())
            .detach();

        cx.spawn_in(window, async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(80)).await;
                if this
                    .update_in(cx, |this, window, cx| {
                        if this.drain_update_events(window, cx) {
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

    /// A tab with its two boxes wired — the title bar's filter and the
    /// find bar's — keeping the scrollback the settings say.
    fn build_tab(
        id: usize,
        scrollback: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SerialTabState {
        let filter_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(FILTER_PLACEHOLDER)
                .clean_on_escape()
        });
        let filter_subscription = cx.subscribe_in(
            &filter_input,
            window,
            move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let pattern = input.read(cx).value().to_string();
                    this.set_filter_pattern(id, &pattern, cx);
                }
            },
        );
        let (find_input, find_subscription) = Self::build_find_input(id, window, cx);
        let mut tab = SerialTabState::new(
            id,
            filter_input,
            filter_subscription,
            find_input,
            find_subscription,
            scrollback,
        );
        Self::listen_to_port(id, tab.take_events(), cx);
        tab
    }

    /// Keeps a tab's terminal fed: a task that sleeps until the port's
    /// threads report something, then hands it over at once — together with
    /// whatever else has arrived meanwhile, so a burst is one update and
    /// one frame rather than many. It ends when the tab is gone.
    ///
    /// While the terminal is keeping back a read the sleep is bounded by
    /// [`LINE_HOLD`]: if nothing follows within it, an empty batch goes
    /// over, and the read goes in on its own.
    fn listen_to_port(
        id: usize,
        events: Option<smol::channel::Receiver<SerialEvent>>,
        cx: &mut Context<Self>,
    ) {
        let Some(events) = events else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let mut holding = false;
            loop {
                let next = if holding {
                    let more = async { events.recv().await.map(Some) };
                    let hold = async {
                        Timer::after(LINE_HOLD).await;
                        Ok(None)
                    };
                    smol::future::or(more, hold).await
                } else {
                    events.recv().await.map(Some)
                };
                let batch = match next {
                    Ok(Some(event)) => {
                        let mut batch = vec![event];
                        while let Ok(event) = events.try_recv() {
                            batch.push(event);
                        }
                        batch
                    }
                    Ok(None) => Vec::new(),
                    Err(_) => break,
                };
                match this.update(cx, |this, cx| this.handle_serial_events(id, batch, cx)) {
                    Ok(still_holding) => holding = still_holding,
                    Err(_) => break,
                }
            }
        })
        .detach();
    }

    fn active_tab(&self) -> Option<&SerialTabState> {
        self.tabs.get(self.active_tab)
    }

    fn tab_index(&self, id: usize) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    pub(crate) fn tab(&self, id: usize) -> Option<&SerialTabState> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub(crate) fn tab_mut(&mut self, id: usize) -> Option<&mut SerialTabState> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
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

    /// Moves to the tab on the left, if there is one. No wrapping: the arrow
    /// that goes grey is what tells you where you are in the row.
    fn select_previous_tab(&mut self, cx: &mut Context<Self>) {
        if self.active_tab > 0 && !self.tabs.is_empty() {
            self.active_tab -= 1;
            cx.notify();
        }
    }

    fn select_next_tab(&mut self, cx: &mut Context<Self>) {
        if self.active_tab + 1 < self.tabs.len() {
            self.active_tab += 1;
            cx.notify();
        }
    }

    /// Called as the filter box changes, so the matcher is compiled once per
    /// keystroke rather than once per frame.
    fn set_filter_pattern(&mut self, tab_id: usize, pattern: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.tab_index(tab_id)
            && self.tabs[index].filter.set_pattern(pattern)
        {
            self.tabs[index].filter_changed();
            cx.notify();
        }
    }

    fn toggle_filter_regex(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.filter.toggle_regex();
            tab.filter_changed();
            cx.notify();
        }
    }

    fn toggle_filter_match_case(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.filter.toggle_match_case();
            tab.filter_changed();
            cx.notify();
        }
    }

    /// Switches the filter between tinting the lines that match and
    /// showing only them. The mask opens at the newest line, and what the
    /// find found is looked for again among the lines now shown.
    fn toggle_filter_mode(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.filter.toggle_mode();
            tab.mask.scroll_to_bottom();
            tab.find.forget();
            cx.notify();
        }
    }

    /// Puts the cursor in the title bar filter box, for the View menu.
    fn focus_output_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = self.active_tab().map(|tab| tab.filter_input.clone()) {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn refresh_ports(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if tab.connected || tab.connecting {
            return;
        }
        // The tab stays bound to its port: one that the scan no longer finds
        // is kept on the list as unavailable rather than silently swapped.
        let current = tab.selected_port().clone();
        let mut ports = discover_ports();
        let found = ports.len();
        if !ports.iter().any(|port| port.name == current.name) {
            ports.push(PortItem::unavailable(
                current.name.clone(),
                "Configured device · currently unavailable",
            ));
        }
        tab.selected_port = ports
            .iter()
            .position(|port| port.name == current.name)
            .unwrap_or(0);
        tab.ports = ports;
        tab.note(format!("Scan complete · {found} devices found"));
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
            tab.note(format!("Disconnected from {device}"));
            cx.notify();
            return;
        }

        let selected = tab.selected_port().clone();
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

    /// Opens the port of a tab just made, when its device is attached. A tab
    /// on a device that is not there keeps its note and waits.
    pub(crate) fn connect_if_attached(&mut self, tab_id: usize, cx: &mut Context<Self>) {
        let attached = self
            .tab(tab_id)
            .is_some_and(|tab| tab.selected_port().kind != PortKind::Unavailable);
        if attached {
            self.toggle_connection(tab_id, cx);
        }
    }

    fn toggle_active_connection(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active_tab().map(|tab| tab.id) {
            self.toggle_connection(id, cx);
        }
    }

    /// Sends what the composer holds to the tab in front.
    ///
    /// What was sent stays in the box afterwards, so the same line can go
    /// out again with a press rather than being typed afresh; the box is
    /// empty only when the workspace opens.
    fn send_to_active_tab(&mut self, cx: &mut Context<Self>) {
        let line = self.send_input.read(cx).value().to_string();
        let ending = self.composer_line_ending();
        self.send_line(&line, ending, cx);
    }

    /// What the composer puts after a line: the ending the tab in front is
    /// set to, and with no tab the one a line is likeliest to want.
    pub(crate) fn composer_line_ending(&self) -> LineEnding {
        self.active_tab()
            .map_or_else(LineEnding::default, SerialTabState::line_ending)
    }

    /// Sends one line to the tab in front, in the tab's encoding and with
    /// `ending` after it — the composer's own, or the one a saved card was
    /// kept with. A line that is not the hex it claims to be goes nowhere,
    /// with a note in the log saying why: a typo sent as text would reach
    /// the device as something else entirely.
    pub(crate) fn send_line(&mut self, line: &str, ending: LineEnding, cx: &mut Context<Self>) {
        let value = line.trim().to_string();
        if value.is_empty() {
            return;
        }

        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if !tab.connected {
            tab.note("Connect a serial port before sending data.");
            cx.notify();
            return;
        }

        let mut bytes = if tab.hex_mode {
            match parse_hex(&value) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tab.note(error.message());
                    cx.notify();
                    return;
                }
            }
        } else {
            value.into_bytes()
        };
        bytes.extend_from_slice(ending.bytes());
        tab.write(bytes);
        if tab.auto_scroll {
            tab.scroll_to_bottom();
        }

        cx.notify();
    }

    /// Text typed into the terminal goes to the port as it is typed, the
    /// way a serial console works: what shows on screen is what the device
    /// echoes back. A read-only tab takes nothing. Typing lets go of the
    /// selection, as it does in a terminal.
    fn type_into_terminal(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if !tab.connected || !tab.interactive || text.is_empty() {
            return;
        }
        tab.terminal.clear_selection();
        tab.write(text.as_bytes().to_vec());
        if tab.auto_scroll {
            tab.scroll_to_bottom();
        }
        self.wake_cursor(window, cx);
        cx.notify();
    }

    /// Starts the cursor blinking, if it is not already. Called from the
    /// terminal's render while it holds focus, so the blink runs exactly
    /// while there is a cursor to blink.
    pub(crate) fn start_blinking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blinking || !self.cursor_blinks() {
            return;
        }
        self.blinking = true;
        self.cursor_shown = true;
        self.schedule_blink(window, cx);
    }

    /// Shows the cursor at once and starts its blink over, so it never
    /// vanishes under the finger.
    fn wake_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cursor_shown = true;
        if self.blinking {
            self.schedule_blink(window, cx);
        }
    }

    fn schedule_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.blink_epoch = self.blink_epoch.wrapping_add(1);
        let epoch = self.blink_epoch;
        cx.spawn_in(window, async move |this, cx| {
            Timer::after(CURSOR_BLINK_INTERVAL).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.blink_epoch != epoch {
                    return;
                }
                let focused =
                    this.terminal_focus.is_focused(window) && window.is_window_active();
                if !focused || !this.cursor_blinks() {
                    this.blinking = false;
                    this.cursor_shown = true;
                    cx.notify();
                    return;
                }
                this.cursor_shown = !this.cursor_shown;
                cx.notify();
                this.schedule_blink(window, cx);
            });
        })
        .detach();
    }

    /// Whether the terminal in front wants its cursor to blink at all.
    fn cursor_blinks(&self) -> bool {
        self.active_tab()
            .is_none_or(|tab| tab.terminal.cursor_blinks())
    }

    /// A key pressed with the terminal focused: editing and control keys
    /// go out as the bytes a terminal sends for them. Plain text does not
    /// come this way but through the input handler, so an input method can
    /// compose it first; keys the terminal has no meaning for, and every
    /// command shortcut, pass on to the menus. A read-only tab sends
    /// nothing.
    ///
    /// Where Ctrl+C is the platform's copy, it copies while there is a
    /// selection to copy and is the interrupt it is in every terminal
    /// otherwise, as VS Code's terminal has it; on macOS ⌘C copies and
    /// Ctrl+C is always the interrupt.
    fn terminal_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if cfg!(not(target_os = "macos"))
            && modifiers.control
            && !modifiers.shift
            && !modifiers.alt
            && !modifiers.platform
            && event.keystroke.key == "c"
            && self.copy_terminal_selection(cx)
        {
            cx.stop_propagation();
            return;
        }
        if self.write_key(&event.keystroke, window, cx) {
            cx.stop_propagation();
        }
    }

    /// Sends what a keystroke means to a device to the tab in front, and
    /// says whether it went: a tab with nothing open, one that is read
    /// only, and a key the terminal has no bytes for all leave the
    /// keystroke to whoever else wants it.
    fn write_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        if !tab.connected || !tab.interactive {
            return false;
        }
        let Some(bytes) = key_bytes(keystroke, tab.terminal.mode()) else {
            return false;
        };
        tab.terminal.clear_selection();
        tab.write(bytes);
        if tab.auto_scroll {
            tab.scroll_to_bottom();
        }
        self.wake_cursor(window, cx);
        cx.notify();
        true
    }

    /// Tab in a terminal completes what is being typed, so while the log
    /// has focus it goes to the device instead of moving on to the next
    /// control the way it does everywhere else in the window. With nothing
    /// to send it to it is handed back, so the log is never a place the
    /// keyboard cannot leave.
    pub(crate) fn type_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = Keystroke {
            key: "tab".to_string(),
            key_char: None,
            modifiers: Modifiers {
                shift: back,
                ..Modifiers::default()
            },
        };
        if !self.write_key(&keystroke, window, cx) {
            cx.propagate();
        }
    }

    /// A press on the terminal starts a selection at the cell under it —
    /// the cell, its word or its line, by how many times the button was
    /// clicked — or, with shift held, moves the end of the one there is.
    /// The drag that follows is watched from the terminal's paint, so it
    /// goes on when the pointer leaves the log.
    fn terminal_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let metrics = self.terminal_metrics;
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let Some(cell) = Self::cell_at(tab, metrics, event.position) else {
            return;
        };
        if event.modifiers.shift {
            tab.terminal.extend_selection(cell);
        } else {
            tab.terminal
                .begin_selection(cell, SelectionKind::for_clicks(event.click_count));
        }
        self.selecting = true;
        cx.notify();
    }

    /// The pointer moving with the button down: the selection's end
    /// follows it. Past the top or bottom of the log the view scrolls a
    /// line that way, so a drag can reach into the scrollback.
    fn terminal_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let metrics = self.terminal_metrics;
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let y = f32::from(position.y) - metrics.origin_y;
        let past_edge = if y < 0. {
            tab.scroll_view(1);
            true
        } else if y > metrics.lines as f32 * metrics.line_height {
            tab.scroll_view(-1);
            true
        } else {
            false
        };
        if past_edge {
            tab.auto_scroll = tab.view_at_bottom();
        }
        if let Some(cell) = Self::cell_at(tab, metrics, position) {
            tab.terminal.extend_selection(cell);
        }
        cx.notify();
    }

    /// The button released: the drag is over. A press that moved over
    /// nothing leaves nothing selected.
    fn terminal_release(&mut self, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        if let Some(tab) = self.tabs.get_mut(self.active_tab)
            && !tab.terminal.has_selection()
        {
            tab.terminal.clear_selection();
        }
        cx.notify();
    }

    /// The cell under a point of the window, on the grid as the tab's
    /// view shows it: past the edges of the log, the nearest cell. Nothing
    /// while the view has no rows to land on.
    fn cell_at(
        tab: &SerialTabState,
        metrics: TerminalMetrics,
        position: Point<Pixels>,
    ) -> Option<GridCell> {
        let columns = metrics.columns.max(1);
        let lines = metrics.lines.max(1);
        let x = (f32::from(position.x) - metrics.origin_x - metrics.text_left)
            / metrics.cell_width.max(1.);
        let y = (f32::from(position.y) - metrics.origin_y) / metrics.line_height.max(1.);
        let row = y.floor().clamp(0., (lines - 1) as f32) as usize;
        let column = x.floor().clamp(0., (columns - 1) as f32) as usize;
        Some(GridCell {
            line: tab.grid_line_at(row)?,
            column,
            right_half: x >= column as f32 + 0.5,
        })
    }

    /// Selects the whole log of the tab in front, for ⌘A.
    fn select_all_in_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.terminal.select_all();
            cx.notify();
        }
    }

    /// Puts the selected text on the clipboard, for ⌘C, and says whether
    /// there was any: with nothing selected the clipboard keeps what it
    /// had.
    fn copy_terminal_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.active_tab().and_then(SerialTabState::selection_text) else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    /// The wheel over the terminal moves through its scrollback. Scrolling
    /// up stops the view following new output, which would otherwise pull
    /// it straight back down; reaching the bottom again resumes following.
    fn scroll_terminal(&mut self, delta_lines: f32, cx: &mut Context<Self>) {
        self.scroll_remainder += delta_lines;
        let lines = self.scroll_remainder.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_remainder -= lines as f32;
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        tab.scroll_view(lines);
        tab.auto_scroll = tab.view_at_bottom();
        cx.notify();
    }

    /// Feeds a tab what its port reported. An empty batch is the hold on
    /// a read running out (see [`LINE_HOLD`]): the read goes in by itself.
    /// Says whether the terminal is now keeping one back, so the listener
    /// knows to bound its next sleep.
    fn handle_serial_events(
        &mut self,
        tab_id: usize,
        events: Vec<SerialEvent>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tab_mut(tab_id) else {
            return false;
        };
        if events.is_empty() {
            tab.flush();
        }
        for event in events {
            match event {
                SerialEvent::Connected => {
                    tab.connecting = false;
                    tab.connected = true;
                    tab.note("Serial port opened; receiving data.");
                }
                SerialEvent::Data(bytes) => {
                    if !tab.paused {
                        tab.receive(&bytes);
                    }
                }
                SerialEvent::Error(message) => {
                    tab.disconnect();
                    tab.note(message);
                }
            }
        }
        if tab.auto_scroll {
            tab.scroll_to_bottom();
        }
        cx.notify();
        tab.terminal.holds()
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.paused = !tab.paused;
            cx.notify();
        }
    }

    fn clear_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.terminal.clear();
            cx.notify();
        }
    }

    /// Flips the composer between text and hex, for the menu's shortcut.
    fn toggle_hex(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.hex_mode = !tab.hex_mode;
            cx.notify();
        }
    }

    /// Selects an encoding outright, for the segmented UTF-8 / HEX switch
    /// where each half names the mode it turns on rather than flipping it.
    fn set_hex_mode(&mut self, hex_mode: bool, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab)
            && tab.hex_mode != hex_mode
        {
            tab.hex_mode = hex_mode;
            cx.notify();
        }
    }

    /// Sets what follows a sent line, for the composer's ending switch. The
    /// tab keeps one ending per encoding, so this sets the current one.
    fn set_line_ending(&mut self, ending: LineEnding, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab)
            && tab.line_ending() != ending
        {
            tab.set_line_ending(ending);
            cx.notify();
        }
    }

    fn toggle_auto_scroll(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.auto_scroll = !tab.auto_scroll;
            if tab.auto_scroll {
                tab.scroll_to_bottom();
            }
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
                UpdateEvent::Installing => {
                    if let UpdateStatus::Downloading { version, .. } = &self.update_status {
                        self.update_status = UpdateStatus::Installing {
                            version: version.clone(),
                        };
                    }
                }
                UpdateEvent::InstallCompleted(Ok(update)) => {
                    self.update_status = UpdateStatus::Ready(update.clone());
                    Self::show_update_ready_dialog(update, window, cx);
                }
                UpdateEvent::InstallCompleted(Err(error)) => {
                    self.update_status = UpdateStatus::Failed;
                    Self::show_install_error_dialog(error, window, cx);
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
            UpdateStatus::Installing { version } => {
                window.push_notification(
                    Notification::info(format!("Installing serialX v{version}…"))
                        .title("Software Update"),
                    cx,
                );
            }
            UpdateStatus::Ready(update) => {
                Self::show_update_ready_dialog(update.clone(), window, cx);
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
                    "Current version: v{}\nPackage: {package_name}\n\nDownload and install it in place now? serialX keeps running until you choose to relaunch.",
                    env!("CARGO_PKG_VERSION")
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Download and Install")
                        .show_cancel(true)
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

    fn show_update_ready_dialog(update: ReadyUpdate, window: &mut Window, cx: &mut App) {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let update = update.clone();
            alert
                .icon(Icon::new(IconName::CircleCheck).size_5())
                .title(format!("serialX v{} is ready", update.version))
                .description(format!("{}\n\nRelaunch now?", update.summary()))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Relaunch Now")
                        .show_cancel(true)
                        .cancel_text("Later"),
                )
                .on_ok(move |_, window, cx| {
                    // The helper waits for this process to end, then starts
                    // the new version; quitting is what hands over to it.
                    match relaunch(&update) {
                        Ok(()) => cx.quit(),
                        Err(message) => Self::show_update_error_dialog(message, window, cx),
                    }
                    true
                })
        });
    }

    fn show_install_error_dialog(error: InstallError, window: &mut Window, cx: &mut App) {
        let InstallError { message, package } = error;
        window.open_alert_dialog(cx, move |alert, _, _| {
            let alert = alert
                .icon(Icon::new(IconName::CircleX).size_5())
                .title("Unable to Install Update");
            let Some(package) = package.clone() else {
                return alert
                    .description(message.clone())
                    .button_props(DialogButtonProps::default().ok_text("Close"));
            };
            alert
                .description(format!(
                    "{message}\n\nThe verified package is at {}. Open it to finish the update by hand.",
                    package.display()
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Close")
                        .show_cancel(true)
                        .cancel_text("Open Package"),
                )
                .on_cancel(move |_, window, cx| {
                    if let Err(message) = open_package(&package) {
                        Self::show_update_error_dialog(message, window, cx);
                    }
                    true
                })
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

    /// The line under the version is the package description from
    /// `Cargo.toml`, which is also the repository's About on GitHub and
    /// the READMEs' first line: the four are changed together.
    fn show_about_dialog(window: &mut Window, cx: &mut App) {
        window.open_alert_dialog(cx, |alert, _, _| {
            alert
                .icon(Icon::new(IconName::SquareTerminal).size_5())
                .title("serialX")
                .description(format!(
                    "Version {}\n{}\n\nGNU GPL v3\n© 2026 miskin",
                    env!("CARGO_PKG_VERSION"),
                    env!("CARGO_PKG_DESCRIPTION")
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Close")
                        .show_cancel(true)
                        .cancel_text("View on GitHub"),
                )
                .on_cancel(|_, _, cx| {
                    cx.open_url(REPOSITORY_URL);
                    true
                })
        });
    }
}
/// The terminal as a text input, so that typing into it goes through the
/// platform's text system: an input method composes Chinese or Japanese
/// before anything is sent, and dead keys and option-combinations yield the
/// character they name. There is no text to edit, so ranges are moot: every
/// insertion is sent as it comes, and only the composition in progress is
/// held.
impl EntityInputHandler for SerialWorkspace {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.marked_text_len();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.composing.as_ref().map(|_| 0..self.marked_text_len())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.composing = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = None;
        self.type_into_terminal(text, window, cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    /// Where the input method puts its candidate window: the terminal's
    /// cursor cell.
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let metrics = self.terminal_metrics;
        let (line, column) = self
            .active_tab()
            .and_then(|tab| tab.terminal.cursor_position())
            .unwrap_or((0, 0));
        Some(Bounds {
            origin: point(
                element_bounds.left() + px(metrics.text_left + metrics.cell_width * column as f32),
                element_bounds.top() + px(metrics.line_height * line as f32),
            ),
            size: size(px(metrics.cell_width.max(1.)), px(metrics.line_height.max(1.))),
        })
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.active_tab()
            .is_some_and(|tab| tab.connected && tab.interactive)
    }
}

impl SerialWorkspace {
    fn marked_text_len(&self) -> usize {
        self.composing
            .as_ref()
            .map_or(0, |text| text.encode_utf16().count())
    }
}

impl Render for SerialWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.interface_theme.palette();
        // The mask and the find are brought up to the log before the frame
        // reads any of them.
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.refresh_mask();
        }
        self.refresh_find(cx);
        let active_snapshot = self.active_tab().map(SerialTabSnapshot::from);
        let tab_strip = self.render_tab_strip(active_snapshot.as_ref(), cx);
        let title_bar = self.render_title_bar(active_snapshot.as_ref(), cx);
        let content = match active_snapshot.clone() {
            Some(tab) => self.render_active_tab(tab, window, cx),
            None => self.render_empty_state(window, cx),
        };
        // The panel's height, for the saved sessions list to take its share
        // of in pixels: a percentage of a flex parent resolves to nothing.
        let panel_height: f32 = window.viewport_size().height.into();
        let sidebar =
            self.render_right_sidebar(active_snapshot, panel_height - TITLE_BAR_HEIGHT, window, cx);

        // `Root` only renders the window chrome; dialogs, sheets and
        // notifications live in overlay layers the window content has to render
        // itself, otherwise opening one silently does nothing.
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);

        let centre = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .children(tab_strip)
            .child(content);

        // Expanded, the panel's left edge is a drag handle; collapsed, the
        // rail is a fixed strip and there is nothing to drag.
        let body = if self.side_panel_collapsed {
            h_flex()
                .flex_1()
                .min_h_0()
                .child(centre)
                .child(sidebar)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(
                    h_resizable("workbench-panels")
                        .with_state(&self.panel_layout)
                        .with_handle_appearance(panel_divider(palette))
                        .child(resizable_panel().child(centre))
                        .child(
                            resizable_panel()
                                .size(px(SIDEBAR_WIDTH))
                                .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
                                .flex_none()
                                .child(sidebar),
                        ),
                )
                .into_any_element()
        };

        let workbench = v_flex()
            .size_full()
            .ui_font()
            .bg(rgb(palette.editor))
            .text_color(rgb(palette.foreground))
            .child(title_bar)
            .child(body);

        div()
            .relative()
            .size_full()
            .child(workbench)
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
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
    prepare_application_menus();
    let app = gpui_kit::application().with_assets(WorkbenchAssets);
    app.run(move |cx| {
        gpui_kit::init(cx);
        // Before the first theme is built: the theme carries the resolved
        // families, and every `ui_font()` call reads the same cache.
        resolve_fonts(cx);
        apply_application_icon();
        configure_application_menus(cx);

        let mut options = TitleBar::window_options();
        // The bar is taller than the component's default, so the traffic
        // lights move down to stay on its centre line.
        options.titlebar = Some(TitlebarOptions {
            traffic_light_position: Some(traffic_light_position()),
            ..TitleBar::title_bar_options()
        });
        options.window_bounds = Some(WindowBounds::centered(size(px(1280.), px(800.)), cx));
        options.window_min_size = Some(size(px(960.), px(640.)));

        // serialX is its one window: closing it — the red light on macOS,
        // where an application would otherwise linger in the Dock with
        // nothing to show — quits, ports and all.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                let interface_theme = InterfaceTheme::default();
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
