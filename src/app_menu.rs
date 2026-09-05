//! The application menu: what the platform draws along the top of the screen
//! on macOS and what `AppMenuBar` draws inside the title bar elsewhere.
//!
//! Menus are grouped by what they act on rather than by widget. `Session` is
//! everything that happens to the tab in front of you — open it, save it, talk
//! to the device — and `View` is everything about how the log is shown. Every
//! item that is used more than once a sitting has a shortcut, and the shortcut
//! is the one the platform's editors already taught: ⌘N, ⌘W, ⌘S, ⌘K, ⌘F.

use gpui_kit::component::GlobalState;
use gpui_kit::*;

use crate::{REPOSITORY_URL, SerialWorkspace, theme::InterfaceTheme};

actions!(
    serialx_menu,
    [
        NewSerialTab,
        CloseSerialTab,
        SaveCurrentSession,
        RefreshPorts,
        ToggleConnection,
        TogglePause,
        ClearTerminal,
        ToggleHex,
        ToggleTimestamps,
        ToggleAutoScroll,
        ToggleSidePanel,
        PreviousTab,
        NextTab,
        FocusOutputFilter,
        UseLightTheme,
        UseDarkTheme,
        ToggleTheme,
        CheckForUpdates,
        OpenRepository,
        ReportIssue,
        ShowAbout,
        QuitApplication
    ]
);

/// One keystroke per platform family: `⌘` on macOS, `Ctrl` everywhere else.
macro_rules! keystroke {
    ($name:ident, $mac:literal, $other:literal) => {
        #[cfg(target_os = "macos")]
        const $name: &str = $mac;
        #[cfg(not(target_os = "macos"))]
        const $name: &str = $other;
    };
}

keystroke!(NEW_TAB_KEYSTROKE, "cmd-n", "ctrl-n");
keystroke!(CLOSE_TAB_KEYSTROKE, "cmd-w", "ctrl-w");
keystroke!(SAVE_SESSION_KEYSTROKE, "cmd-s", "ctrl-s");
keystroke!(CONNECT_KEYSTROKE, "cmd-shift-c", "ctrl-shift-c");
keystroke!(REFRESH_PORTS_KEYSTROKE, "cmd-r", "ctrl-r");
keystroke!(PAUSE_KEYSTROKE, "cmd-shift-p", "ctrl-shift-p");
// ⌘K clears in every terminal on the platform; ⌘L is taken by the location
// bar in every browser, so the pair is left alone here.
keystroke!(CLEAR_KEYSTROKE, "cmd-k", "ctrl-k");
keystroke!(HEX_KEYSTROKE, "cmd-shift-h", "ctrl-shift-h");
keystroke!(TIMESTAMPS_KEYSTROKE, "cmd-shift-t", "ctrl-shift-t");
keystroke!(AUTO_SCROLL_KEYSTROKE, "cmd-shift-a", "ctrl-shift-a");
keystroke!(THEME_KEYSTROKE, "cmd-shift-l", "ctrl-shift-l");
keystroke!(SIDE_PANEL_KEYSTROKE, "cmd-b", "ctrl-b");
// Tab navigation follows VS Code's editor bindings on each platform.
keystroke!(PREVIOUS_TAB_KEYSTROKE, "cmd-shift-[", "ctrl-pageup");
keystroke!(NEXT_TAB_KEYSTROKE, "cmd-shift-]", "ctrl-pagedown");
keystroke!(FILTER_KEYSTROKE, "cmd-f", "ctrl-f");
keystroke!(QUIT_KEYSTROKE, "cmd-q", "ctrl-q");

fn application_menus() -> Vec<Menu> {
    let mut help = vec![
        MenuItem::action("serialX on GitHub", OpenRepository),
        MenuItem::action("Report an Issue…", ReportIssue),
    ];
    // macOS keeps these under the application menu; the other platforms have
    // no such menu, so `Help` carries them as well.
    if cfg!(not(target_os = "macos")) {
        help.extend([
            MenuItem::separator(),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::action("About serialX", ShowAbout),
        ]);
    }

    vec![
        // macOS always titles the first menu after the application, so the
        // application menu has to come first for "Session" to keep its own name.
        Menu::new("serialX").items([
            MenuItem::action("About serialX", ShowAbout),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Quit serialX", QuitApplication),
        ]),
        Menu::new("Session").items([
            MenuItem::action("New Session…", NewSerialTab),
            MenuItem::action("Save Session", SaveCurrentSession),
            MenuItem::action("Close Session", CloseSerialTab),
            MenuItem::separator(),
            MenuItem::action("Connect / Disconnect", ToggleConnection),
            MenuItem::action("Rescan Ports", RefreshPorts),
            MenuItem::separator(),
            MenuItem::action("Pause / Resume Receiving", TogglePause),
            MenuItem::action("Clear Terminal", ClearTerminal),
            MenuItem::separator(),
            MenuItem::action("Previous Session", PreviousTab),
            MenuItem::action("Next Session", NextTab),
        ]),
        Menu::new("View").items([
            MenuItem::action("Filter Output…", FocusOutputFilter),
            MenuItem::separator(),
            MenuItem::action("Toggle HEX Display", ToggleHex),
            MenuItem::action("Toggle Timestamps", ToggleTimestamps),
            MenuItem::action("Toggle Auto-scroll", ToggleAutoScroll),
            MenuItem::separator(),
            MenuItem::action("Toggle Side Panel", ToggleSidePanel),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Appearance").items([
                MenuItem::action("Light", UseLightTheme),
                MenuItem::action("Dark", UseDarkTheme),
                MenuItem::separator(),
                MenuItem::action("Switch Appearance", ToggleTheme),
            ])),
        ]),
        Menu::new("Help").items(help),
    ]
}

pub(crate) fn configure_application_menus(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(NEW_TAB_KEYSTROKE, NewSerialTab, None),
        KeyBinding::new(CLOSE_TAB_KEYSTROKE, CloseSerialTab, None),
        KeyBinding::new(SAVE_SESSION_KEYSTROKE, SaveCurrentSession, None),
        KeyBinding::new(CONNECT_KEYSTROKE, ToggleConnection, None),
        KeyBinding::new(REFRESH_PORTS_KEYSTROKE, RefreshPorts, None),
        KeyBinding::new(PAUSE_KEYSTROKE, TogglePause, None),
        KeyBinding::new(CLEAR_KEYSTROKE, ClearTerminal, None),
        KeyBinding::new(HEX_KEYSTROKE, ToggleHex, None),
        KeyBinding::new(TIMESTAMPS_KEYSTROKE, ToggleTimestamps, None),
        KeyBinding::new(AUTO_SCROLL_KEYSTROKE, ToggleAutoScroll, None),
        KeyBinding::new(THEME_KEYSTROKE, ToggleTheme, None),
        KeyBinding::new(SIDE_PANEL_KEYSTROKE, ToggleSidePanel, None),
        KeyBinding::new(PREVIOUS_TAB_KEYSTROKE, PreviousTab, None),
        KeyBinding::new(NEXT_TAB_KEYSTROKE, NextTab, None),
        KeyBinding::new(FILTER_KEYSTROKE, FocusOutputFilter, None),
        KeyBinding::new(QUIT_KEYSTROKE, QuitApplication, None),
    ]);
    GlobalState::global_mut(cx)
        .set_app_menus(application_menus().into_iter().map(Menu::owned).collect());
    cx.set_menus(application_menus());
}

fn defer_window_action(
    cx: &mut App,
    workspace: WeakEntity<SerialWorkspace>,
    action: impl FnOnce(&mut SerialWorkspace, &mut Window, &mut Context<SerialWorkspace>) + 'static,
) {
    cx.defer(move |cx| {
        if let Err(error) = workspace.update_in(cx, action) {
            eprintln!("failed to run serialX window action: {error}");
        }
    });
}

pub(crate) fn bind_window_actions(workspace: &Entity<SerialWorkspace>, cx: &mut App) {
    let view = workspace.downgrade();
    cx.on_action(move |_: &NewSerialTab, cx| {
        defer_window_action(cx, view.clone(), |view, window, cx| {
            view.open_new_serial_tab_dialog(window, cx);
        });
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &CloseSerialTab, cx| {
        let _ = view.update(cx, |view, cx| view.close_active_tab(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &SaveCurrentSession, cx| {
        let _ = view.update(cx, |view, cx| view.save_active_session(cx));
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

    let view = workspace.downgrade();
    cx.on_action(move |_: &UseLightTheme, cx| {
        defer_window_action(cx, view.clone(), |view, window, cx| {
            view.set_interface_theme(InterfaceTheme::Light, window, cx);
        });
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &UseDarkTheme, cx| {
        defer_window_action(cx, view.clone(), |view, window, cx| {
            view.set_interface_theme(InterfaceTheme::Dark, window, cx);
        });
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &ToggleTheme, cx| {
        defer_window_action(cx, view.clone(), |view, window, cx| {
            let next = view.interface_theme.toggled();
            view.set_interface_theme(next, window, cx);
        });
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &ToggleSidePanel, cx| {
        let _ = view.update(cx, |view, cx| view.toggle_side_panel(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &PreviousTab, cx| {
        let _ = view.update(cx, |view, cx| view.select_previous_tab(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &NextTab, cx| {
        let _ = view.update(cx, |view, cx| view.select_next_tab(cx));
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &FocusOutputFilter, cx| {
        defer_window_action(cx, view.clone(), |view, window, cx| {
            view.focus_output_filter(window, cx);
        });
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &CheckForUpdates, cx| {
        defer_window_action(cx, view.clone(), |view, window, cx| {
            view.check_for_updates_from_menu(window, cx);
        });
    });

    let view = workspace.downgrade();
    cx.on_action(move |_: &ShowAbout, cx| {
        defer_window_action(cx, view.clone(), |_, window, cx| {
            SerialWorkspace::show_about_dialog(window, cx);
        });
    });

    cx.on_action(|_: &OpenRepository, cx| {
        cx.open_url(REPOSITORY_URL);
    });

    cx.on_action(|_: &ReportIssue, cx| {
        cx.open_url(&format!("{REPOSITORY_URL}/issues/new"));
    });

    cx.on_action(|_: &QuitApplication, cx| {
        cx.quit();
    });
}

#[cfg(test)]
mod tests {
    use super::application_menus;
    use gpui_kit::MenuItem;

    fn labels(menu: &gpui_kit::Menu) -> Vec<String> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                MenuItem::Submenu(menu) => Some(menu.name.to_string()),
                _ => None,
            })
            .collect()
    }

    /// The menu is grouped by what an item acts on, and the first item of the
    /// session menu is the way in.
    #[test]
    fn session_menu_opens_with_new_session() {
        let menus = application_menus();
        let session = menus
            .iter()
            .find(|menu| menu.name == "Session")
            .expect("a Session menu");
        assert_eq!(
            labels(session).first().map(String::as_str),
            Some("New Session…")
        );
    }

    #[test]
    fn view_menu_folds_appearance_into_a_submenu() {
        let menus = application_menus();
        let view = menus
            .iter()
            .find(|menu| menu.name == "View")
            .expect("a View menu");
        let appearance = view
            .items
            .iter()
            .find_map(|item| match item {
                MenuItem::Submenu(menu) if menu.name == "Appearance" => Some(menu),
                _ => None,
            })
            .expect("an Appearance submenu");
        assert_eq!(labels(appearance), ["Light", "Dark", "Switch Appearance"]);
    }
}
