<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/logo-dark.svg?v=3">
    <img src="docs/logo.svg?v=3" alt="serialX" width="380">
  </picture>
</p>

<p align="center">English · <a href="README.zh-CN.md">简体中文</a></p>

serialX is a cross-platform serial port workspace built with Rust, [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) and [GPUI Component](https://github.com/longbridge/gpui-component), available on macOS, Windows and Linux.

## Screenshots

**Dark theme** — syntax-highlighted logs, search, colour-tagged sessions and quick-send commands.

![serialX dark theme with colourful device logs, search and grouped sessions](docs/screenshots/dark-workspace.png)

**Light theme** — text and hex views in split panes, with the session library and send composer alongside.

![serialX light theme with split text and hex views, saved sessions and quick send](docs/screenshots/light-split-view.png)

*App screenshots use demonstration data.*

## Features

- **Send and receive**: Interactive terminal, text and hex views, and UTF-8 / HEX sending.
- **Log viewing**: Millisecond timestamps, line numbers, syntax highlighting, filtering and search with regular expression support.
- **Session management**: Save serial port settings and organise sessions by name, tag colour and group.
- **Quick send**: Save common commands for one-click sending, with configurable line endings.
- **Layout and themes**: Multiple tabs, horizontal and vertical splits, a collapsible sidebar, and light and dark themes.
- **Recording**: Save raw received bytes for each connection, automatically organised by date.
- **File transfer**: XMODEM, XMODEM-1K, YMODEM and ZMODEM, with automatic responses to `sz` / `rz` on the device.
- **Remote connections**: Access remote serial ports, sessions and files over SSH. The host needs `python3`; no agent preinstallation is required.
- **Preferences**: Adjust scrollback length, terminal font size and file locations, and check for updates in the app.

## Install

Download a package for your platform from [Releases](https://github.com/miskin-lee/serialX/releases):

| Platform | Package |
|---|---|
| macOS (Apple Silicon) | DMG |
| Windows (x86_64) | Installer / portable ZIP |
| Linux (x86_64, ARM64) | DEB / portable tar.gz |

The packages are not yet notarized by Apple or code-signed on Windows, so the system may show a security prompt on first launch. The app checks for stable updates automatically; you can also check manually via `Help > Check for Updates…`.

## Getting started

1. Click `+` in the tab bar to create a session, then choose a serial port and connection settings. The default is `115200 8N1`.
2. Choose `Text` or `Hex` view, then click `Save & Connect` to save and connect, or `Connect` to connect directly.
3. Type in the interactive terminal or send data through the sidebar composer. Save frequently used content as quick-send commands.

## Licence

serialX is released under the GNU GPL v3. The application icon and name are excluded from this open-source licence. Copyright © 2026 miskin.
