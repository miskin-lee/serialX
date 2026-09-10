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
- Every session has a tag colour: pick one of 24 when creating or editing a session (two rows: bright hues over deeper, more saturated ones), and a new session is offered a colour drawn at random from those no open tab is wearing; the tab's plate and the saved session's colour square in the side panel carry the colour, while the dot on the tab still shows the connection state
- Sessions can be named: fill in the `Tab` field of the dialog and the tab and the saved session show that name instead of the port path, with the port and its parameters moved to the tooltip; leave it empty and the port path shows as before
- The side panel keeps sessions: saved sessions can be filed into groups (make one with the folder button on the section header; group rows collapse, rename and delete, and deleting a group keeps its sessions and moves them back to the top level).
  A click selects a card, a double-click opens it (connecting at once when the device is attached, or switching to the tab already on that port), and the pencil and trash buttons on the card edit and delete it; the trash asks first, and `Enter` in the prompt confirms.
  Below are the quick-send commands: give one a name and a group when saving, commands have groups of their own (collapsible, renamable and deletable too), a click sends, the pencil edits, the trash deletes and asks first as the sessions' does; the composer sits at the foot of the panel
- The side panel collapses in two steps: click a section header to fold that section, ⌘B or the switch at the right end of the title bar to fold the whole panel into an icon rail with count badges
- The composer at the foot of the side panel is a chat-style card: the input on top, a `To` line along the card's upper edge naming the session it sends to and its connection state; along the lower edge a row of switches — the `UTF-8 / HEX` segmented switch picks the encoding (HEX parses the input as hex bytes such as `41 54 0D 0A`; input that is not valid hex outlines the card in red, Enter sends nothing and the log says why), the `↵ CRLF` dropdown picks the line ending (`CRLF` = `\r\n`, `LF` = `\n`, `None` adds nothing), UTF-8 defaults to `CRLF` and HEX to `None`, and each encoding remembers its own ending; Enter or the round send button sends to the active tab, and what was sent stays in the box so the same line can go out again with a press — the box is empty only when the workspace opens
- The receive area is a real terminal: the emulation core is `alacritty_terminal` (the one Alacritty and Zed's terminal share), so colours, bold, underline, inverse video, 256 and true colour, cursor movement, backspace, progress bars redrawn with `\r` and wide characters all show as a terminal shows them, with 50,000 lines of scrollback by default and its text set at 13 pt (both in `Settings…` in the `serialX` menu, `⌘,`, where the recordings' folder is set too: 100 to 1,000,000 lines typed into a field, the size picked from a list of whole points from 8 to 32 — the line numbers, timestamps and line height follow it — applied to every session at once and saved in the workspace file), and the wheel to page through history (scrolling up pauses following the output, scrolling back to the bottom resumes it), with a scrollbar down the log's right edge — the thumb is as much of the track as the screen is of the whole log and stands where the view stands in it, so a long log says so at a glance; the pointer widens it, a press anywhere on the track goes there and goes on dragging, and one drag reaches from one end of fifty thousand lines to the other;
  a clear from the device is a clear: `clear` sent by the device (`ESC[2J`, or `ESC[J` from the home position on a vt100) wipes the screen, the scrollback and the timestamps, the same as Clear in the `Session` menu; `ESC[3J` only empties the scrollback; a full-screen program clearing the alternate screen leaves the log alone
- A session's log reads one of two ways, chosen when the session is made — the `Text` / `Hex` switch beside the tag swatches in the New session dialog — and kept with the saved session: `Text` runs what arrives through the emulation and shows what the device drew; `Hex` sets it out the way a hex editor sets a file — sixteen bytes in hex in two halves of eight, and those same sixteen as characters between two bars, with a full stop standing for each one that is not text (where a row stands is the gutter's business, as it is for text).
  Colour says what a byte is, in both panes at once: the zeroes and the frame in grey, printable text plain, the bytes that end a line green, the other control bytes magenta, and everything above ASCII amber, so the shape of a frame reads before a single pair of digits does.
  Rows fill from the reads as they come, and a read that does not fill one goes out as a short row of its own the moment the device pauses, so a device streaming comes out in an even grid and a device that says three bytes and stops shows them at once.
  Everything the log can do it can do over a dump: the timestamps and line numbers in the gutter, the scrollback and its scrollbar, the filter, the find, the selection and the copy
- The terminal can be typed into: click to focus it, and every character goes straight to the port, with the device's echo on screen; Enter, Backspace, Tab, Esc, the arrows, Home/End, F1–F12 and Ctrl+letter send the bytes or escape sequences a terminal sends, with application cursor mode and modifier encoding; text an input method is composing shows underlined at the cursor and is sent only once committed;
  the `Interactive` checkbox in the dialog's `Tab` section is on by default, and unchecking it opens the tab read-only: no cursor in the terminal, nothing typed goes to the port, and sending is done from the composer at the foot of the panel or from Quick send — the tab's tooltip and the saved session both remember it
- A session can record what its device says: tick `Record` in the `Tab` section of the New session dialog and every connection writes a file of its own — the bytes exactly as the port read them, so a recording can be replayed or diffed — named for the session and the time it opened, filed under the day: `~/Documents/serialX/2026-09-09/Motor board-20260909-142312.log`. The folder is set in `Settings…` (`⌘,`) with the platform's own folder picker, and the recordings keep their `serialX` tree inside whatever is chosen, so pointing it at the desktop puts them all in one folder there; the log says where the file went as it opens, the tab's tooltip says a recording is running, and disconnecting closes it
- A session can send a file to the device and take one from it, over XMODEM, XMODEM-1K, YMODEM or ZMODEM: `Send File…` and `Receive File…` in the `Session` menu open one dialog with the protocol as a segmented switch (a line under it says what the choice means), then the file to send or the folder to receive into — and, under XMODEM, which carries no name of its own, the name the file is to get. While a transfer runs a strip lies along the top of the log — which way the file is going and its name, the protocol, a bar of how far it has got with the bytes and the rate beside it, and `Cancel` — the port is the transfer's for the duration (typing and the composer go nowhere), and the log says how it ended: the files with their sizes and how long they took, or why it stopped. ZMODEM needs no dialog when the device starts it: `sz` at a shell begins a receive on its own into the folder the receive dialog last used (the account's `Downloads` until one is chosen), and `rz` opens the send dialog with ZMODEM chosen. A received file is written under the name the sender gave, or the one typed, with a number added when that name is taken; a name from the wire is only a name, never a path out of the folder
- Text in the terminal can be selected with the mouse: drag to select a stretch, double-click a word, triple-click a whole line, Shift-click to extend, and drag past the top or bottom edge to scroll through the scrollback; `⌘A` selects the whole log and `⌘C` copies the selection, with lines the terminal wrapped joined back into one; `⌘V` pastes the clipboard at the device, typing it as it was copied (line endings sent as Return, and wrapped in the paste markers when the device asked for bracketed paste); a right-click on the terminal opens a menu with Copy, `Copy with Timestamps` — the same text with each line's time at its head, in a column of its own, as the gutter shows it — Paste and Select All; the selection follows its text as new lines push it up, and clears when you type or clear the screen
- Every line has a timestamp and a line number on the left: numbers count from 1 the way an editor's do, and when the scrollback fills, the oldest lines go but the rest keep their numbers (the first number on screen climbs past 1); a clear (from the menu or the device's `clear`) starts again at 1; the timestamp is the local time the line's first byte arrived, to the millisecond, and is not repeated on rows the terminal wrapped; the title bar filter highlights matching lines where they stand, or, with its mask switch on, shows only the lines that match
- Find in the terminal (`⌘F`): a find bar floats over the terminal's top-right corner and searches the whole scrollback as you type; every match on screen is washed amber, the current one is ringed in the accent colour and scrolled to the middle of the screen, and the bar shows "n of m"; `Enter` / `⌘G` step forward, `⇧Enter` / `⌘⇧G` back, `Aa` matches case, `.*` switches to regular expressions (literal by default), `Esc` closes;
  with text selected in the terminal, `⌘F` opens the bar on that text (a selection on one line; escaped when the bar is in regular-expression mode) and keeps the selected occurrence as the current one, so the view does not jump; find and the title bar's filter are two different things — the filter decides which lines stay on screen, find locates within the whole log — so the filter box no longer takes `⌘F`, and while the filter's mask is on, find looks only through the lines the mask shows
- One strip of tabs above the terminal: session tabs (connection dot, port name, close) and the new-tab button; the connect / disconnect button sits right of the filter box in the title bar, and the filter and the connection state belong to their tab and switch with it; pause, clear, auto-scroll and rescan live in the menus, each with a shortcut
- The workbench splits, the way an editor does: `⌘\` (`Ctrl+\`) hands the session in front a pane of its own beside the others, `⌘⇧\` one below, and the same three items — `Split Right`, `Split Down`, `Join Split` — are in the `View` menu and on a right-click on a tab; the switch at the right end of the title bar splits, and the one beside it folds a split back. Every pane has its own strip of tabs and its own log, up to three panes; the seam between them drags to resize, and a tab dragged from one strip to another (or onto another pane's log) moves that session across — dragged within its own strip, it just changes places. A pane closes with its last tab, and what the title bar, the composer and the menus act on is the front tab of the pane last worked in
- Two workbench themes, pure white and near-black, starting in the dark one and switched from the `View` menu
- Checks GitHub Releases at start-up, and can verify and install the latest version
- A VS Code-style title bar (menu bar): the command centre in the middle, arrows to switch tabs, a filter box that sifts the terminal output by plain text or regular expression, the active tab's connect / disconnect button at its right, and beside it the session's `RX` and `TX` byte counters — what this session has taken off the port and put on it, counted per tab (a paused log keeps counting, so the numbers still say the device is talking) and cleared with the log;
  the filter can match case, shows "matching lines / total lines" live at its right end, and marks a mis-typed expression in red rather than hiding any output;
  the filter has two modes: by default it highlights the lines that match where they stand, and the crossed-eye switch at its right end turns on the mask, which shows only the lines that match — from the whole scrollback, with their line numbers and timestamps so the gaps read; the mask scrolls on its own, find, selection and copy work on the lines shown, and the view says so when no line matches;
  the right end holds the split switch, the switch that folds a split back while there is one, and the side-panel switch; the left holds only the platform's own things (the traffic lights on macOS, the application menu on Windows / Linux)
- The side panel's left edge drags to resize it between 220 and 560 pixels, and the width survives collapsing to the rail and back
- The New session dialog is laid out as a choice rather than a form: the device is a scrollable single-choice list, the baud rate a dropdown you can type into (the standard rates in the list, any custom rate accepted),
  data bits, parity, stop bits and flow control are four segmented switches, the `Tab` section is a full-width name field beside a group dropdown of the same size (with a way to make a group on the spot), the `Record` and `Interactive` checkboxes at the right end of the section's header, and under them two rows of tag swatches with the `Text` / `Hex` switch beside them, which picks how the log will read what arrives;
  the foot restates the choice as a `115200 8N1`-style summary with the parameters spelled out, the group named at its end when one is chosen, `Hex` when the log will be a dump, `Read-only` when `Interactive` is off and `Recording` when `Record` is on; the dialog is modal: a press outside it does not close it but makes it flash
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
- `⌘,` / `Ctrl+,`: settings (scrollback lines, terminal text size, the folder the recordings go in)
- `⌘⇧[` / `⌘⇧]` (`Ctrl+PageUp` / `Ctrl+PageDown`): switch to the tab on the left / right
- `⌘\` / `Ctrl+\`: show the session in front in a pane beside the others; `⌘⇧\` / `Ctrl+Shift+\` in a pane below; `Join Split` in the `View` menu (or the switch in the title bar) folds the pane back
- `⌘B` / `Ctrl+B`: show or hide the side panel; `⌘⇧L` / `Ctrl+Shift+L` switch between the light and dark themes
- `Session` menu: new, save or close a session, connect, clear the terminal, send a file, receive a file, previous / next session
- `View` menu: find (and previous / next match), output filter, send as HEX, auto-scroll, split right / split down / join split, the side panel, and the light and dark themes in the `Theme` submenu; `serialX` menu: about, settings, quit
- Drag the seam between two panes to resize them; drag a tab into another pane's strip or log to move that session there
- Drag the side panel's left edge to resize it
- Title bar filter box: plain text, any case, by default; `.*` turns on regular expressions, `Aa` case sensitivity, the crossed eye switches between highlighting the matching lines and showing only them, × clears
- Right of the title bar filter box: the active tab's connect / disconnect button, and beside it the session's `RX` and `TX` byte counters
- New session dialog: pick a port from the device list (`Rescan` at any time), the baud rate dropdown, the data bits / parity / stop bits / flow control segmented switches,
  a name in the `Tab` field with two rows of tag swatches, the `Record` and `Interactive` checkboxes at the right of the section's header deciding whether the session keeps a file of what it hears and whether the tab can be typed into, and a live summary at the foot; `Save & Connect` (or `Enter`) saves, then opens and connects, `Connect` only opens and connects, `Esc` cancels
- `Sessions` on the right: save and restore session configurations, double-click to open, pencil to edit, trash to delete (a prompt asks first; `Enter` confirms, `Esc` keeps the session); a card carries the session's colour square and its name, with the port and its parameters in the tooltip; right-click the section header or the list to make a group; a port already open in a tab shows a green dot
- `Quick send` on the right: the search box at the top filters by name or command text (matches shown flat while searching), `Aa` toggles case sensitivity; a click sends the saved command to the active tab, pencil to edit, trash to delete (a prompt asks first; `Enter` confirms, `Esc` keeps the command); a named command shows its name alone, with the line it sends in the tooltip; right-click the section header or the list to make a group;
  the composer at the foot of the panel sends on Enter, `UTF-8 / HEX` picks the encoding, the `↵` dropdown the line ending (`CRLF` / `LF` / `None`), and `Save` at the end of the `To` line opens the save dialog, which keeps the current input as a new command with a name and a group (an empty name uses the command itself)
- Click a section header to collapse the section; the collapsed panel keeps an icon rail, and a click on an icon expands its section

## Icon copyright

The serialX application icon and the derived icon assets under `assets/icons/` were designed by miskin,
copyright © 2026 miskin, and are licensed under the GNU GPL v3 like the rest of the project.
