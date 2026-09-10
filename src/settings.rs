//! Settings: what is the workbench's to set rather than a session's.
//!
//! Three so far — how many lines a terminal keeps above its screen, typed as
//! a number; the size its log is set in, picked from the list of sizes the
//! setting offers; and the folder the sessions that record write into,
//! chosen with the platform's own folder picker — opened from the
//! application menu with ⌘, as every macOS application opens its own. What
//! is set is written to the workspace file beside the presets, and takes
//! effect at once in every open session.
//!
//! The recordings keep a tree of their own — `serialX`, then the day —
//! inside the folder that is chosen, so the setting can name the desktop
//! without a year of loose days landing on it. The field shows the folder
//! the files will actually be written to, which is that tree's head; what
//! the picker chooses is the folder it stands in.

use std::path::PathBuf;

use gpui_kit::component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::SerialWorkspace;
use crate::controls::{dialog_footer, eyebrow};
use crate::icons::{Glyph, icon_chip};
use crate::presets::{
    MAX_SCROLLBACK_LINES, MIN_SCROLLBACK_LINES, Settings, TERMINAL_FONT_SIZES, usable_font_size,
};
use crate::recorder::recordings_folder;
use crate::theme::{CAPTION, InterfaceTheme, LABEL, TITLE, Typography, WorkbenchPalette};

/// Width of the dialog: a number, and a sentence about it.
const DIALOG_WIDTH: f32 = 420.;
/// Height of a field, the same as the session dialog's fields.
const FIELD_HEIGHT: f32 = 30.;
/// The gap between the two settings.
const SECTION_GAP: f32 = 14.;
/// Width of the size field: a couple of digits, a `pt` and a caret. The list
/// it opens is at least as wide as the field it hangs from.
const SIZE_FIELD_WIDTH: f32 = 108.;
/// What the folder field says when the platform will not name a documents
/// folder and none has been chosen — nowhere to record to.
const NO_FOLDER: &str = "No folder";

/// Reads a count of lines as typed: digits, with the thousands separators
/// anyone might put in, within the bounds the setting takes.
pub(crate) fn parse_scrollback_lines(text: &str) -> Result<usize, &'static str> {
    let digits: String = text
        .chars()
        .filter(|ch| !matches!(ch, ',' | '_' | ' ' | '\u{2009}'))
        .collect();
    if digits.is_empty() {
        return Err("Enter a number of lines");
    }
    let lines: usize = digits.parse().map_err(|_| "Whole numbers only")?;
    if lines < MIN_SCROLLBACK_LINES {
        return Err("At least 100 lines");
    }
    if lines > MAX_SCROLLBACK_LINES {
        return Err("At most 1,000,000 lines");
    }
    Ok(lines)
}

/// A type size as the field and its list show it: a whole number of points.
pub(crate) fn format_font_size(size: f32) -> String {
    format!("{}", size.round() as i32)
}

/// A count with thousands separators, as the field shows it.
pub(crate) fn format_lines(lines: usize) -> String {
    let digits = lines.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The settings as they are being set: the scrollback typed into a field, the
/// type size picked from a list. Nothing here reaches the workspace until the
/// dialog is confirmed.
struct SettingsEditor {
    theme: InterfaceTheme,
    lines_input: Entity<InputState>,
    _lines_subscription: Subscription,
    font_size: f32,
    /// The folder the recordings' own tree is made in, while one has been
    /// chosen; none leaves them in the account's documents.
    recording_root: Option<PathBuf>,
}

impl SettingsEditor {
    fn new(
        theme: InterfaceTheme,
        current: Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let lines_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(format_lines(Settings::default().scrollback_lines))
                .default_value(format_lines(current.scrollback_lines))
        });
        // The line under the field answers what has been typed, so the
        // dialog is drawn again at every keystroke.
        let lines_subscription = cx.subscribe_in(
            &lines_input,
            window,
            |_, _, event: &InputEvent, _, cx: &mut Context<Self>| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );

        Self {
            theme,
            lines_input,
            _lines_subscription: lines_subscription,
            // A file from before the sizes were whole opens on the size the
            // log is actually laid out at, so the list has a row ticked.
            font_size: usable_font_size(current.terminal_font_size),
            recording_root: current.recording_root,
        }
    }

    /// The scrollback as typed, or why it will not do.
    fn scrollback_lines(&self, cx: &App) -> Result<usize, &'static str> {
        parse_scrollback_lines(self.lines_input.read(cx).value().as_ref())
    }

    /// Puts the cursor in the scrollback field with its number selected, so
    /// a new one can be typed over it.
    fn focus_lines(&self, select: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.lines_input.update(cx, |field, cx| {
            field.focus(window, cx);
            if select {
                field.select_all(window, cx);
            }
        });
    }

    fn select_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.font_size = size;
        cx.notify();
    }

    /// Asks the platform for a folder to keep the recordings in. The
    /// picker is the system's, so it opens where the user last was and can
    /// make a folder on the spot; a cancelled pick leaves the setting
    /// where it stood.
    fn choose_recording_folder(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |editor, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let _ = editor.update(cx, |editor, cx| {
                editor.recording_root = Some(folder);
                cx.notify();
            });
        })
        .detach();
    }

    /// Back to the folder the workbench picks itself.
    fn use_default_recording_folder(&mut self, cx: &mut Context<Self>) {
        self.recording_root = None;
        cx.notify();
    }

    /// The recordings' folder: the path they will actually be written
    /// under, read whole in a tooltip when it is too long for the field,
    /// then the picker, and the way back to the default while the folder
    /// is not it.
    fn render_recording_field(
        &mut self,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder: SharedString = recordings_folder(self.recording_root.as_deref())
            .map_or_else(|| NO_FOLDER.into(), |folder| folder.display().to_string())
            .into();
        let chosen = self.recording_root.is_some();
        let full = folder.clone();

        h_flex()
            .items_center()
            .gap_2()
            .child(
                h_flex()
                    .id("settings-recording-folder")
                    .flex_1()
                    .min_w_0()
                    .h(px(FIELD_HEIGHT))
                    .px_2p5()
                    .items_center()
                    .rounded(px(8.))
                    .bg(rgb(palette.input))
                    .border_1()
                    .border_color(rgb(palette.input_border))
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .ui_mono_token(LABEL)
                            .text_color(rgb(palette.strong_foreground))
                            .child(folder),
                    )
                    .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx)),
            )
            .child(
                Button::new("settings-recording-choose")
                    .outline()
                    .small()
                    .h(px(FIELD_HEIGHT))
                    .label("Choose…")
                    .tooltip("Pick the folder the serialX recordings folder sits in")
                    .on_click(cx.listener(|editor, _, _, cx| {
                        editor.choose_recording_folder(cx);
                    })),
            )
            .when(chosen, |row| {
                row.child(
                    Button::new("settings-recording-default")
                        .ghost()
                        .small()
                        .h(px(FIELD_HEIGHT))
                        .label("Default")
                        .tooltip("Keep the recordings in the account's documents again")
                        .on_click(cx.listener(|editor, _, _, cx| {
                            editor.use_default_recording_folder(cx);
                        })),
                )
            })
            .into_any_element()
    }

    /// The size field: the size it is set to and a caret, drawn as the other
    /// field is drawn, opening the list of sizes with the current one ticked.
    fn render_size_field(
        &mut self,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.font_size;
        let editor = cx.weak_entity();

        Button::new("settings-font-size")
            .ghost()
            .with_size(px(FIELD_HEIGHT))
            .w(px(SIZE_FIELD_WIDTH))
            .h(px(FIELD_HEIGHT))
            .pl_2p5()
            .pr_2()
            .bg(rgb(palette.input))
            .border_1()
            .border_color(rgb(palette.input_border))
            .rounded(px(8.))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex()
                            .min_w_0()
                            .items_baseline()
                            .gap_1()
                            .child(
                                div()
                                    .ui_mono_token(LABEL)
                                    .text_color(rgb(palette.strong_foreground))
                                    .child(format_font_size(current)),
                            )
                            .child(
                                div()
                                    .text_token(CAPTION)
                                    .text_color(rgb(palette.faint))
                                    .child("pt"),
                            ),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(px(12.))
                            .text_color(rgb(palette.muted)),
                    ),
            )
            .tooltip("The size the terminal's log is set in")
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu, _, _| {
                menu = menu.min_w(px(SIZE_FIELD_WIDTH));
                for size in TERMINAL_FONT_SIZES {
                    let editor = editor.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format_font_size(size))
                            .checked((current - size).abs() < f32::EPSILON)
                            .on_click(move |_, _, cx| {
                                let _ = editor
                                    .update(cx, |editor, cx| editor.select_font_size(size, cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
}

impl Render for SettingsEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        // The field says as it is typed whether the number will do.
        let verdict = self.scrollback_lines(cx);
        let lines = Input::new(&self.lines_input)
            .small()
            .h(px(FIELD_HEIGHT))
            .ui_mono_token(LABEL)
            .bg(rgb(palette.input))
            .border_color(rgb(if verdict.is_ok() {
                palette.input_border
            } else {
                palette.danger
            }))
            .rounded(px(8.))
            .px_2p5()
            .focus_bordered(verdict.is_ok())
            .suffix(
                div()
                    .text_token(CAPTION)
                    .text_color(rgb(palette.faint))
                    .child("lines"),
            );
        let size = self.render_size_field(palette, cx);
        let recording = self.render_recording_field(palette, cx);

        v_flex()
            .gap(px(SECTION_GAP))
            .child(setting_section(
                palette,
                "Scrollback",
                lines,
                match &verdict {
                    Ok(_) => Ok("Lines kept above the screen, in every session. 100 to 1,000,000; the change takes effect at once."),
                    Err(reason) => Err(*reason),
                },
            ))
            .child(setting_section(
                palette,
                "Terminal text",
                size,
                Ok("The size the log is set in, in every session. The line numbers, the timestamps and the line height follow it."),
            ))
            .child(setting_section(
                palette,
                "Recordings",
                recording,
                Ok("Where a session with Record on writes what its device says: a file of its own for every connection, filed under the day. Choose a folder to keep this one somewhere else."),
            ))
    }
}

/// One setting: its label, the control it is set with, and a line under that
/// — what the setting means while what is set will do, the reason it will
/// not otherwise, in the colour that says which.
fn setting_section(
    palette: WorkbenchPalette,
    label: &str,
    control: impl IntoElement,
    hint: Result<&str, &str>,
) -> impl IntoElement {
    let (hint, hint_color) = match hint {
        Ok(blurb) => (blurb.to_owned(), palette.muted),
        Err(reason) => (reason.to_owned(), palette.danger),
    };
    v_flex()
        .gap_2()
        .child(eyebrow(palette, label))
        .child(control)
        .child(
            div()
                .text_token(CAPTION)
                .text_color(rgb(hint_color))
                .child(hint),
        )
}

impl SerialWorkspace {
    /// Opens the dialog on the settings as they are.
    pub(crate) fn open_settings_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.interface_theme;
        let palette = theme.palette();
        let current = self.presets.settings.clone();
        // What the transfer dialog keeps here is not this dialog's to
        // change, so it goes back as it was.
        let transfer_folder = current.transfer_folder.clone();
        let transfer_protocol = current.transfer_protocol;
        let editor = cx.new(|cx| SettingsEditor::new(theme, current, window, cx));
        let field = editor.clone();
        let workspace = cx.weak_entity();

        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let editor = editor.clone();
            let transfer_folder = transfer_folder.clone();
            alert
                .width(px(DIALOG_WIDTH))
                .p_5()
                .icon(icon_chip(Glyph::Terminal, palette.accent, 36.))
                .title(
                    div()
                        .text_token(TITLE)
                        .text_color(rgb(palette.strong_foreground))
                        .child("Settings"),
                )
                .description("Kept with the workspace, for every session.")
                .close_button(true)
                .child(editor.clone())
                .footer(dialog_footer(palette, "Save", Glyph::Bookmark, None))
                .on_ok(move |_, window, cx| {
                    // A number that will not do keeps the dialog open with
                    // the field in focus, and the reason under it. The size
                    // comes from a list, so there is nothing to check.
                    let Ok(scrollback_lines) = editor.read(cx).scrollback_lines(cx) else {
                        editor.update(cx, |editor, cx| editor.focus_lines(false, window, cx));
                        return false;
                    };
                    let terminal_font_size = editor.read(cx).font_size;
                    let recording_root = editor.read(cx).recording_root.clone();
                    let _ = workspace.update(cx, |workspace, cx| {
                        workspace.apply_settings(
                            Settings {
                                scrollback_lines,
                                terminal_font_size,
                                recording_root,
                                transfer_folder: transfer_folder.clone(),
                                transfer_protocol,
                            },
                            cx,
                        );
                    });
                    true
                })
        });

        // The dialog takes focus as it opens; the field takes it back once
        // the dialog is there, with the number selected so a new one can
        // be typed over it.
        cx.defer_in(window, move |_, window, cx| {
            field.update(cx, |editor, cx| editor.focus_lines(true, window, cx));
        });
    }

    /// Takes the settings as given: writes them down, and hands every open
    /// terminal its new scrollback.
    pub(crate) fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        let scrollback_lines = settings.scrollback_lines;
        self.presets.set_settings(settings);
        for tab in &mut self.tabs {
            tab.terminal.set_scrollback(scrollback_lines);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{format_font_size, format_lines, parse_scrollback_lines};
    use crate::presets::{
        DEFAULT_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE,
        TERMINAL_FONT_SIZES, usable_font_size,
    };

    #[test]
    fn a_count_of_lines_is_read_as_typed_and_kept_in_bounds() {
        assert_eq!(parse_scrollback_lines("50000"), Ok(50_000));
        assert_eq!(parse_scrollback_lines(" 50,000 "), Ok(50_000));
        assert_eq!(parse_scrollback_lines("1_000_000"), Ok(1_000_000));
        assert_eq!(parse_scrollback_lines(""), Err("Enter a number of lines"));
        assert_eq!(parse_scrollback_lines("lots"), Err("Whole numbers only"));
        assert_eq!(parse_scrollback_lines("99"), Err("At least 100 lines"));
        assert_eq!(
            parse_scrollback_lines("1000001"),
            Err("At most 1,000,000 lines")
        );
    }

    /// The list the field opens: sizes in order, none outside the bounds, and
    /// the size a fresh workspace starts at among them, so the field always
    /// opens with one of its rows ticked.
    #[test]
    fn the_sizes_offered_climb_within_the_bounds_and_hold_the_default() {
        assert!(TERMINAL_FONT_SIZES.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(TERMINAL_FONT_SIZES.first(), Some(&MIN_TERMINAL_FONT_SIZE));
        assert_eq!(TERMINAL_FONT_SIZES.last(), Some(&MAX_TERMINAL_FONT_SIZE));
        assert!(TERMINAL_FONT_SIZES.contains(&DEFAULT_TERMINAL_FONT_SIZE));
    }

    /// A size out of an older workspace file is shown, and used, as the
    /// whole point nearest it.
    #[test]
    fn type_sizes_are_whole_points() {
        assert_eq!(format_font_size(14.), "14");
        assert_eq!(format_font_size(12.5), "13");
        assert_eq!(usable_font_size(12.5), 13.);
        assert_eq!(usable_font_size(0.), MIN_TERMINAL_FONT_SIZE);
        assert_eq!(usable_font_size(400.), MAX_TERMINAL_FONT_SIZE);
        assert_eq!(usable_font_size(f32::NAN), DEFAULT_TERMINAL_FONT_SIZE);
    }

    #[test]
    fn counts_are_shown_with_separators() {
        assert_eq!(format_lines(100), "100");
        assert_eq!(format_lines(50_000), "50,000");
        assert_eq!(format_lines(1_000_000), "1,000,000");
    }
}
