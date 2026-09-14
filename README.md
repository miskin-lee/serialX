<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/logo-dark.svg?v=3">
    <img src="docs/logo.svg?v=3" alt="serialX" width="380">
  </picture>
</p>

<p align="center">English · <a href="README.zh-CN.md">简体中文</a></p>

Cross-platform serial port workspace built in Rust with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) and
[GPUI Component](https://github.com/longbridge/gpui-component). A real terminal for your devices in an editor-like workbench:
organise ports as saved sessions and split panes, read traffic as text or hex with timestamps, filter and find, record every
byte, and move files over XMODEM/YMODEM/ZMODEM.

## Highlights

- **A real terminal, not a text box.** The receive area runs the same emulation core as Alacritty and Zed, so colours, cursor movement, progress bars and full-screen programs look the way they do in a terminal. Type into it and every key goes to the device.
- **Every line dated and numbered.** A gutter carries the arrival time of each line to the millisecond and a line number that survives scrollback trimming. Filter what you see, find what you need, copy with or without the timestamps.
- **Text or hex, your choice per session.** A hex session lays the bytes out like a hex editor, sixteen to a row, with the control bytes and line endings coloured so a frame's shape reads at a glance.
- **Sessions you keep.** Save a port with its parameters, a name, a tag colour and a group. Open it again with a double-click. Keep the commands you send often as quick-send buttons.
- **A workbench, not a window.** Split the view right or down, drag tabs between panes, fold the side panel to a rail. Light and dark themes.
- **Recording and file transfer.** Tick `Record` and every connection writes the raw bytes to a file. Send and receive files over XMODEM, XMODEM-1K, YMODEM or ZMODEM, and let `sz` / `rz` on the device start the transfer for you.
- **Remote hosts over SSH.** Connect to a machine the way an editor opens a remote folder: its serial devices, its saved sessions and commands, its folders. Nothing is installed there.
- **macOS, Windows and Linux.** One code base, native window on each, updates installed in place from GitHub Releases.

## Install

Prebuilt packages are on the [Releases](https://github.com/miskin-lee/serialX/releases) page:

| Platform | Package |
|---|---|
| macOS (Apple Silicon) | DMG |
| Windows (x86_64) | Installer, or a portable ZIP |
| Linux (x86_64, ARM64) | DEB, or a portable tar.gz |

The packages are not yet notarized by Apple or code-signed on Windows, so the system may show a security prompt on first launch.

serialX checks for a newer stable release in the background after it starts and offers to install it. `Help > Check for Updates…` checks at any time. The download is verified against the SHA-256 digest published with the release and swapped into place; choose `Relaunch` to switch now, or `Later` to pick it up at the next start. Only public release information is read, so no GitHub account is needed.

## Getting started

1. **Open a session.** `Session > New Session…`, the `+` in the tab strip, or `⌘N` / `Ctrl+N`. Pick the device from the list (`Rescan` if it was just plugged in), choose the baud rate and the framing switches, and read the summary at the foot of the dialog: `115200 8N1` is the default.
2. **Name it, colour it, file it.** The `Tab` field names the tab and the saved session; leave it empty and the port path is used. Pick one of 24 tag colours, and a group if you want the session filed under one. `Text` or `Hex` decides how the log will read what arrives.
3. **Connect.** `Save & Connect` (or `Enter`) keeps the session in the side panel and opens it connected. `Connect` only opens the tab. Every tab has its own connection and its own log, and the dot on the tab shows whether it is connected.
4. **Talk to the device.** Click the terminal and type: keys, arrows, function keys and `Ctrl` combinations are sent the way a terminal sends them, and the device's echo shows on screen. Or use the composer at the foot of the side panel, where a line is typed, sent with `Enter` and kept in the box to be sent again.

The connect / disconnect button for the active tab sits in the title bar, right of the filter box, with the session's `RX` and `TX` byte counters beside it.

## The terminal

**What it shows.** Colours, bold, underline, inverse video, 256 and true colour, wide characters, and lines redrawn with `\r`, all rendered as a terminal renders them. On top of that, serialX colours the parts of a line a reader picks out anyway: log levels, times, addresses, paths and numbers with units. The device's own colours always win.

**Scrollback.** 50,000 lines by default (100 to 1,000,000 in Settings). Scroll up with the wheel to pause following the output, scroll back to the bottom to resume, or use the scrollbar down the right edge. A `clear` sent by the device wipes the screen the way `Session > Clear` does.

**Line numbers and timestamps.** Numbers count from 1 like an editor's and keep counting when the oldest lines are dropped. The timestamp is the local time the line's first byte arrived, to the millisecond. A clear starts the numbers again at 1.

**Hex view.** A session made with `Hex` shows sixteen bytes per row in two halves, with the same bytes as characters between two bars. Zeroes and the frame are grey, printable text plain, line endings green, other control bytes magenta, anything above ASCII amber. Rows fill as the reads come, and a short read shows at once when the device pauses. Timestamps, line numbers, filter, find, selection and copy all work over a hex log.

**Selection and clipboard.** Drag to select, double-click a word, triple-click a line, `Shift`-click to extend. `⌘A` selects the whole log and `⌘C` copies it with wrapped lines joined back together. `⌘V` pastes at the device, using bracketed paste when the device asked for it. A right-click offers `Copy`, `Copy with Timestamps`, `Paste` and `Select All`.

**Read-only tabs.** Untick `Interactive` in the session dialog and the tab shows no cursor and takes no typing; sending is done from the composer or quick send.

## Filter and find

The **filter box** in the title bar sifts the output as you type: plain text by default, `.*` for regular expressions, `Aa` for case, with a live count of matching lines against the total. By default matching lines are highlighted where they stand; the crossed-eye switch turns on the mask, which shows only the matching lines from the whole scrollback, with their numbers and timestamps so the gaps read. A mistyped expression is marked red rather than hiding anything. `View > Filter Output…` jumps to the box and `Esc` clears it.

**Find** (`⌘F` / `Ctrl+F`) opens a bar over the terminal's top-right corner and searches the whole scrollback as you type. Matches are washed amber, the current one is ringed and scrolled to the middle of the screen, and the bar counts "n of m". `Enter` steps forward, `⇧Enter` back, `Aa` and `.*` work as in the filter, `Esc` closes. With text selected, `⌘F` opens the bar on that text. While the mask is on, find looks only through the lines the mask shows.

## Sessions and quick send

The side panel keeps the things you reuse between runs.

**Sessions.** Saved sessions are cards with their tag colour and name; the port and its parameters are in the tooltip. A double-click opens one, connecting at once when the device is attached, or switches to the tab already on that port. The pencil edits a card and the trash deletes it, after asking. Sessions file into groups: the folder button on the section header makes one, and groups collapse, rename and delete (deleting a group keeps its sessions). `⌘S` / `Ctrl+S` saves the session in front.

**Quick send.** A saved command is a button: click it and the line goes to the active tab. Give it a name and a group when saving; the search box at the top of the section filters by name or content. The composer at the foot of the panel is where a command starts: type it, choose `UTF-8` or `HEX` (`41 54 0D 0A` sends four bytes; invalid hex outlines the card in red and sends nothing), pick the line ending from the `↵` dropdown (`CRLF`, `LF` or `None`), and press `Save` on the `To` line to keep it. The `To` line also names the session the composer sends to and whether it is connected.

**Folding the panel.** Click a section header to fold that section. `⌘B` / `Ctrl+B`, or the switch at the right end of the title bar, folds the whole panel into an icon rail with count badges. Drag the panel's left edge to resize it.

## Layout and themes

`⌘\` / `Ctrl+\` gives the session in front a pane of its own beside the others, `⌘⇧\` / `Ctrl+Shift+\` one below, up to three panes. The same items are in the `View` menu, on a right-click on a tab, and as switches at the right end of the title bar; `Join Split` folds a pane back. Drag the seam between panes to resize them, and drag a tab into another pane's strip or log to move the session there. A pane closes with its last tab.

Two themes, pure white and near-black, starting in the dark one: `View > Theme`, or `⌘⇧L` / `Ctrl+Shift+L`.

## Recording

Tick `Record` in the session dialog and every connection writes a file of its own with the bytes exactly as the port read them, so a recording can be replayed or diffed. Files are named for the session and the time the connection opened, filed under the day:

```
~/Documents/serialX/2026-09-09/Motor board-20260909-142312.log
```

The folder is chosen in Settings; the recordings keep their `serialX` tree inside it. The log says where the file went as the connection opens, the tab's tooltip says a recording is running, and disconnecting closes the file.

## File transfer

`Session > Send File…` and `Session > Receive File…` open one dialog: pick the protocol (`XMODEM`, `XMODEM-1K`, `YMODEM` or `ZMODEM`, with a line under the switch saying what the choice means), then the file to send or the folder to receive into. XMODEM carries no file name, so a receive under it asks for one.

While a transfer runs, a strip along the top of the log shows the direction, the file, the protocol, a progress bar with bytes and rate, and `Cancel`. The port belongs to the transfer until it ends, and the log then says how it went: the files with their sizes and times, or why it stopped.

ZMODEM needs no dialog when the device starts it: `sz` at the device's shell begins a receive on its own, into the folder the receive dialog last used (`Downloads` until one is chosen), and `rz` opens the send dialog with ZMODEM selected. A received file keeps the name the sender gave, with a number added when that name is taken, and is always written inside the chosen folder.

## Remote hosts

`Remote > Connect to Host…` (`⌘⇧R` / `Ctrl+Shift+R`, or the plate at the left end of the title bar) moves the workbench to a machine reached over SSH, the way an editor opens a folder on one. Type what you would give `ssh` — a host from `~/.ssh/config`, or `user@host:port` — and `Save & Connect` keeps it in the list for next time. Keys, agents and jump hosts are used as they are; a password or key passphrase, when the host wants one, is typed into the dialog and kept nowhere.

Connected, everything is the host's: the device list in the session dialog, the port a session opens, the saved sessions and quick-send commands (read from and written to the host's own workspace file), the folder recordings are written in, and the files a transfer sends or receives, picked from a browser of the host's folders. The plate in the title bar names the host; click it to move to another or to disconnect. Open sessions close when the workbench moves, and come back to this machine's devices and workspace on disconnect. If the connection drops, the host's sessions stay open to read until the workbench moves again.

Nothing is installed on the host: the connection starts the platform's own `ssh` with a small Python agent in its command line, so any Linux, macOS or BSD host with `python3` and a serial port will do. The list of saved hosts stays on this machine.

## Settings

`⌘,` / `Ctrl+,` opens Settings: the scrollback length, the terminal text size (8 to 32 pt; the gutter follows it), and the folder recordings go in. Settings apply to every session at once and are kept with the workspace.

serialX bundles no fonts. The terminal uses the platform's editor face (Menlo on macOS, Consolas on Windows, Droid Sans Mono on Linux), the interface uses the system UI font, and Chinese, Japanese and Korean text from a device falls back to the system's own CJK families, with your language first.

## Keyboard shortcuts

| Action | macOS | Windows / Linux |
|---|---|---|
| New session | `⌘N` | `Ctrl+N` |
| Save session | `⌘S` | `Ctrl+S` |
| Close session | `⌘W` | `Ctrl+W` |
| Connect / disconnect | `⌘⇧C` | `Ctrl+Shift+C` |
| Rescan ports | `⌘R` | `Ctrl+R` |
| Pause / resume receiving | `⌘⇧P` | `Ctrl+Shift+P` |
| Clear terminal | `⌘K` | `Ctrl+K` |
| Select whole log | `⌘A` | `Ctrl+Alt+A` |
| Copy selection | `⌘C` | `Ctrl+Insert` (`Ctrl+C` too while text is selected) |
| Paste to device | `⌘V` | `Ctrl+Shift+V` or `Shift+Insert` |
| Find / next / previous | `⌘F` / `⌘G` / `⌘⇧G` | `Ctrl+F` / `F3` / `Shift+F3` |
| Composer `UTF-8` / `HEX` | `⌘⇧H` | `Ctrl+Shift+H` |
| Auto-scroll | `⌘⇧A` | `Ctrl+Shift+A` |
| Previous / next tab | `⌘⇧[` / `⌘⇧]` | `Ctrl+PageUp` / `Ctrl+PageDown` |
| Split right / split down | `⌘\` / `⌘⇧\` | `Ctrl+\` / `Ctrl+Shift+\` |
| Side panel | `⌘B` | `Ctrl+B` |
| Light / dark theme | `⌘⇧L` | `Ctrl+Shift+L` |
| Settings | `⌘,` | `Ctrl+,` |
| Connect to a host | `⌘⇧R` | `Ctrl+Shift+R` |
| Send the composer's line | `Enter` | `Enter` |

On Windows and Linux, `Ctrl+A` and `Ctrl+C` themselves go to the device, as they would in a terminal; `Ctrl+C` copies only while something is selected. `Esc` closes the find bar or clears the filter.

## Build from source

```bash
cargo run
```

The latest stable Rust is recommended. The first build fetches GPUI's dependencies and takes a while; macOS also needs Xcode or the Command Line Tools.

## Licence

serialX is released under the GNU GPL v3. The application icon and the derived icon assets under `assets/icons/` were designed by miskin, copyright © 2026 miskin, and are licensed under the same terms.
