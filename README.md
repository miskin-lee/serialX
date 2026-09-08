<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/logo-dark.svg?v=2">
    <img src="docs/logo.svg?v=2" alt="serialX" width="480">
  </picture>
</p>

<p align="center">English · <a href="README.zh-CN.md">简体中文</a></p>

A modern serial port workspace built with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) and
[GPUI Component](https://github.com/longbridge/gpui-component) — a real terminal for your devices, with saved
sessions and quick send.

## What it does

- Discovers the serial devices on the machine, with full port configuration, connect, disconnect, and reading in the background
- The `Session` menu, the `+` in the tab strip or ⌘N open the New session dialog: `Save & Connect` (also `Enter`) keeps the session in the side panel and opens it in a connected tab, and `Connect` beside it only opens the tab and connects; every tab keeps its own connection and terminal contents
- Every session has a tag colour: pick one of 24 when creating or editing a session (two rows: bright hues over deeper, more saturated ones), and a new session is offered a colour drawn at random from those no open tab is wearing; the tab's plate and the saved session's icon in the side panel carry the colour, while the dot on the tab still shows the connection state
- Sessions can be named: fill in the `Tab` field of the dialog and the tab and the saved session show that name instead of the port path, with the port and its parameters moved to the tooltip and the card's subtitle; leave it empty and the port path shows as before
- The side panel keeps sessions: saved sessions can be filed into groups (make one with the folder button on the section header; group rows collapse, rename and delete, and deleting a group keeps its sessions and moves them back to the top level).
  A click selects a card, a double-click opens it (connecting at once when the device is attached, or switching to the tab already on that port), and the pencil and trash buttons on the card edit and delete it; the trash asks first, and `Enter` in the prompt confirms.
  Below are the quick-send commands: give one a name and a group when saving, commands have groups of their own (collapsible, renamable and deletable too), a click sends, the pencil edits, the trash deletes; the composer sits at the foot of the panel
- The side panel collapses in two steps: click a section header to fold that section, ⌘B or the switch at the right end of the title bar to fold the whole panel into an icon rail with count badges
- The composer at the foot of the side panel is a chat-style card: the input on top, a `To` line along the card's upper edge naming the session it sends to and its connection state; along the lower edge a row of switches — the `UTF-8 / HEX` segmented switch picks the encoding (HEX parses the input as hex bytes such as `41 54 0D 0A`; input that is not valid hex outlines the card in red, Enter sends nothing and the log says why), the `↵ CRLF` dropdown picks the line ending (`CRLF` = `\r\n`, `LF` = `\n`, `None` adds nothing), UTF-8 defaults to `CRLF` and HEX to `None`, and each encoding remembers its own ending; Enter or the round send button sends to the active tab, and what was sent stays in the box so the same line can go out again with a press — the box is empty only when the workspace opens
- The receive area is a real terminal: the emulation core is `alacritty_terminal` (the one Alacritty and Zed's terminal share), so colours, bold, underline, inverse video, 256 and true colour, cursor movement, backspace, progress bars redrawn with `\r` and wide characters all show as a terminal shows them, with 50,000 lines of scrollback by default and its text set at 13 pt (both in `Settings…` in the `serialX` menu, `⌘,`: 100 to 1,000,000 lines typed into a field, the size picked from a list of whole points from 8 to 32 — the line numbers, timestamps and line height follow it — applied to every session at once and saved in the workspace file), and the wheel to page through history (scrolling up pauses following the output, scrolling back to the bottom resumes it);
  a clear from the device is a clear: `clear` sent by the device (`ESC[2J`, or `ESC[J` from the home position on a vt100) wipes the screen, the scrollback and the timestamps, the same as Clear in the `Session` menu; `ESC[3J` only empties the scrollback; a full-screen program clearing the alternate screen leaves the log alone
- The terminal can be typed into: click to focus it, and every character goes straight to the port, with the device's echo on screen; Enter, Backspace, Tab, Esc, the arrows, Home/End, F1–F12 and Ctrl+letter send the bytes or escape sequences a terminal sends, with application cursor mode and modifier encoding; text an input method is composing shows underlined at the cursor and is sent only once committed;
  the `Interactive` checkbox in the dialog's `Tab` section is on by default, and unchecking it opens the tab read-only: no cursor in the terminal, nothing typed goes to the port, and sending is done from the composer at the foot of the panel or from Quick send — the tab's tooltip and the saved session both remember it
- Text in the terminal can be selected with the mouse: drag to select a stretch, double-click a word, triple-click a whole line, Shift-click to extend, and drag past the top or bottom edge to scroll through the scrollback; `⌘A` selects the whole log and `⌘C` copies the selection, with lines the terminal wrapped joined back into one; `⌘V` pastes the clipboard at the device, typing it as it was copied (line endings sent as Return, and wrapped in the paste markers when the device asked for bracketed paste); a right-click on the terminal opens a menu with Copy, Paste and Select All; the selection follows its text as new lines push it up, and clears when you type or clear the screen
- Every line has a timestamp and a line number on the left: numbers count from 1 the way an editor's do, and when the scrollback fills, the oldest lines go but the rest keep their numbers (the first number on screen climbs past 1); a clear (from the menu or the device's `clear`) starts again at 1; the timestamp is the local time the line's first byte arrived, to the millisecond, and is not repeated on rows the terminal wrapped; the title bar filter highlights matching lines where they stand, or, with its mask switch on, shows only the lines that match
- Find in the terminal (`⌘F`): a find bar floats over the terminal's top-right corner and searches the whole scrollback as you type; every match on screen is washed amber, the current one is ringed in the accent colour and scrolled to the middle of the screen, and the bar shows "n of m"; `Enter` / `⌘G` step forward, `⇧Enter` / `⌘⇧G` back, `Aa` matches case, `.*` switches to regular expressions (literal by default), `Esc` closes;
  with text selected in the terminal, `⌘F` opens the bar on that text (a selection on one line; escaped when the bar is in regular-expression mode) and keeps the selected occurrence as the current one, so the view does not jump; find and the title bar's filter are two different things — the filter decides which lines stay on screen, find locates within the whole log — so the filter box no longer takes `⌘F`, and while the filter's mask is on, find looks only through the lines the mask shows
- One strip of tabs above the terminal: session tabs (connection dot, port name, close) and the new-tab button; the connect / disconnect button sits right of the filter box in the title bar, and the filter and the connection state belong to their tab and switch with it; pause, clear, auto-scroll and rescan live in the menus, each with a shortcut
- Two workbench themes, pure white and near-black, starting in the dark one and switched from the `View` menu
- Checks GitHub Releases at start-up, and can verify and install the latest version
- A VS Code-style title bar (menu bar): the command centre in the middle, arrows to switch tabs, a filter box that sifts the terminal output by plain text or regular expression, and the active tab's connect / disconnect button at its right;
  the filter can match case, shows "matching lines / total lines" live at its right end, and marks a mis-typed expression in red rather than hiding any output;
  the filter has two modes: by default it highlights the lines that match where they stand, and the crossed-eye switch at its right end turns on the mask, which shows only the lines that match — from the whole scrollback, with their line numbers and timestamps so the gaps read; the mask scrolls on its own, find, selection and copy work on the lines shown, and the view says so when no line matches;
  the right end holds only the side-panel switch, and the left holds only the platform's own things (the traffic lights on macOS, the application menu on Windows / Linux)
- The side panel's left edge drags to resize it between 220 and 560 pixels, and the width survives collapsing to the rail and back
- The New session dialog is laid out as a choice rather than a form: the device is a scrollable single-choice list, the baud rate a dropdown you can type into (the standard rates in the list, any custom rate accepted),
  data bits, parity, stop bits and flow control are four segmented switches, the `Tab` section is a full-width name field beside a group dropdown of the same size (with a way to make a group on the spot), the `Interactive` checkbox at the right end of the section's header, and two rows of tag swatches under them;
  the foot restates the choice as a `115200 8N1`-style summary with the parameters spelled out, the group named at its end when one is chosen and `Read-only` when `Interactive` is off; the dialog is modal: a press outside it does not close it but makes it flash
- A custom macOS title bar and a compact, low-noise, editor-like workbench layout
- A two-tone rounded icon set after the Material Icon Theme: devices, sessions, commands and signals each have a hue of their own
- One typographic scale, with the fonts chosen as VS Code chooses them: no bundled fonts — the platform's VS Code font stack is reused, and the first family
  installed on the machine is taken at start-up. The terminal uses the editor family (Menlo on macOS, Consolas on Windows, Droid Sans Mono on Linux),
  monospace text in the interface follows VS Code's `--monaco-monospace-font` (SF Mono / Monaco on macOS), and the interface font is
  each platform's system UI font or Segoe UI
- CJK fallback families (PingFang SC, Microsoft YaHei, Source Han Sans and so on) are mounted by the system language, so Chinese, Japanese
  and Korean text from a device shows as it should, with the reader's own language first

## Running

```bash
cargo run
```

The first build downloads GPUI's dependencies and takes a while. The latest stable Rust is recommended;
macOS also needs a full Xcode / Command Line Tools installation.

## Downloads

GitHub Releases carry prebuilt packages:

- macOS Apple Silicon: DMG
- Windows x86_64: installer and portable ZIP
- Linux x86_64 / ARM64: DEB and portable tar.gz

The packages are not yet notarized by Apple or code-signed on Windows, so the system may show a security prompt on first launch.

## Software updates

serialX checks the repository's latest stable GitHub Release in the background after it starts, and shows an
update prompt when there is a newer version. `Help > Check for Updates…` checks at any time, and then says
plainly that this is the latest version or offers "Download and Install". The app downloads the package for
the current system, verifies it against the SHA-256 digest attached to the Release, replaces the running copy
of serialX in place, and asks whether to relaunch now; choose "Later" and the new version takes over at the
next start. The version, licence and project address are under `Help > About serialX`:

- macOS: the DMG is mounted and its serialX.app swapped into place where the current bundle lives (`/Applications`, say)
- Windows: a copy put there by the installer runs the new installer silently, and a portable ZIP copy is unpacked over itself;
  both are finished by a background script after serialX exits, and serialX then restarts by itself
- Linux: a DEB-installed copy upgrades with `pkexec dpkg -i` (a password is asked for), and a portable tar.gz copy
  has its executable replaced directly

When replacing in place fails (no write permission, or serialX is not running from an application bundle),
the prompt names where the verified package is, so the update can be finished by hand.

The automatic check only reads public Release information and needs no GitHub login or token; drafts and
pre-releases are not offered as updates.

Before publishing a release, bump the version in `Cargo.toml` and `Cargo.lock` together with the script:

```bash
scripts/version-bump.sh 0.2.0
```

The script checks the version number, makes the `chore: bump version to 0.2.0` commit and pushes the current branch;
the Release workflow reads the version from `Cargo.toml` and creates the matching `v0.2.0` tag.

## Shortcuts

- `Enter`: send what the composer holds (it stays in the box afterwards)
- `⌘N` / `Ctrl+N`: new session; `⌘S` / `Ctrl+S` save the current session; `⌘W` / `Ctrl+W` close the current session
- `⌘⇧C` / `Ctrl+Shift+C`: connect / disconnect; `⌘R` / `Ctrl+R` rescan ports
- `⌘⇧P` / `Ctrl+Shift+P`: pause / resume receiving; `⌘K` / `Ctrl+K` clear the terminal
- `⌘A` / `Ctrl+Alt+A`: select the whole log; `⌘C` / `Ctrl+Insert`: copy the selection (on Windows / Linux `Ctrl+C` copies too while something is selected and sends the interrupt as usual otherwise; `Ctrl+A` and `Ctrl+C` themselves stay with the device); `⌘V` / `Ctrl+Shift+V` (also `Shift+Insert`): paste the clipboard at the device
- `⌘⇧H` / `Ctrl+Shift+H`: switch the composer between UTF-8 and HEX; `⌘⇧A` / `Ctrl+Shift+A` auto-scroll
- `⌘F` / `Ctrl+F`: find in the terminal, seeded with the selected text when there is a selection; `⌘G` / `F3` next match, `⌘⇧G` / `Shift+F3` previous, `Esc` closes the find bar; the title bar's output filter is reached from `Filter Output…` in the `View` menu, and `Esc` clears it
- `⌘,` / `Ctrl+,`: settings (scrollback lines, terminal text size)
- `⌘⇧[` / `⌘⇧]` (`Ctrl+PageUp` / `Ctrl+PageDown`): switch to the tab on the left / right
- `⌘B` / `Ctrl+B`: show or hide the side panel; `⌘⇧L` / `Ctrl+Shift+L` switch between the light and dark themes
- `Session` menu: new, save or close a session, connect, clear the terminal, previous / next session
- `View` menu: find (and previous / next match), output filter, send as HEX, auto-scroll, the side panel, and the light and dark themes in the `Theme` submenu; `serialX` menu: about, settings, quit
- Drag the side panel's left edge to resize it
- Title bar filter box: plain text, any case, by default; `.*` turns on regular expressions, `Aa` case sensitivity, the crossed eye switches between highlighting the matching lines and showing only them, × clears
- Right of the title bar filter box: the active tab's connect / disconnect button
- New session dialog: pick a port from the device list (`Rescan` at any time), the baud rate dropdown, the data bits / parity / stop bits / flow control segmented switches,
  a name in the `Tab` field with two rows of tag swatches, the `Interactive` checkbox at the right of the section's header deciding whether the tab can be typed into, and a live summary at the foot; `Save & Connect` (or `Enter`) saves, then opens and connects, `Connect` only opens and connects, `Esc` cancels
- `Sessions` on the right: save and restore session configurations, double-click to open, pencil to edit, trash to delete (a prompt asks first; `Enter` confirms, `Esc` keeps the session); right-click the section header or the list to make a group; a port already open in a tab shows a green dot
- `Quick send` on the right: the search box at the top filters by name or command text (matches shown flat while searching), `Aa` toggles case sensitivity; a click sends the saved command to the active tab, pencil to edit, trash to delete; right-click the section header or the list to make a group;
  the composer at the foot of the panel sends on Enter, `UTF-8 / HEX` picks the encoding, the `↵` dropdown the line ending (`CRLF` / `LF` / `None`), and the bookmark button opens the save dialog, which keeps the current input as a new command with a name and a group (an empty name uses the command itself)
- Click a section header to collapse the section; the collapsed panel keeps an icon rail, and a click on an icon expands its section

## Icon copyright

The serialX application icon and the derived icon assets under `assets/icons/` were designed by miskin,
copyright © 2026 miskin, and are licensed under the GNU GPL v3 like the rest of the project.
