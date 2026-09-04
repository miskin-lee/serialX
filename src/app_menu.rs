use gpui_kit::component::GlobalState;
use gpui_kit::*;

use crate::{SerialWorkspace, theme::InterfaceTheme};

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
        UseLightTheme,
        UseDarkTheme,
        CheckForUpdates,
        ShowAbout
    ]
);

fn application_menus() -> Vec<Menu> {
    vec![
        Menu::new("Session").items([
            MenuItem::action("New Serial Tab", NewSerialTab),
            MenuItem::action("Close Current Tab", CloseSerialTab),
            MenuItem::action("Save Current Session", SaveCurrentSession),
            MenuItem::separator(),
            MenuItem::action("Connect / Disconnect", ToggleConnection),
            MenuItem::action("Refresh Port List", RefreshPorts),
            MenuItem::separator(),
            MenuItem::action("Pause / Resume Receiving", TogglePause),
            MenuItem::action("Clear Terminal", ClearTerminal),
        ]),
        Menu::new("View").items([
            MenuItem::action("ASCII / HEX", ToggleHex),
            MenuItem::action("Show / Hide Timestamps", ToggleTimestamps),
            MenuItem::action("Auto / Manual Scroll", ToggleAutoScroll),
            MenuItem::separator(),
            MenuItem::action("Theme: One Light", UseLightTheme),
            MenuItem::action("Theme: One Dark", UseDarkTheme),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("About serialX", ShowAbout),
        ]),
    ]
}

pub(crate) fn configure_application_menus(cx: &mut App) {
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
}
