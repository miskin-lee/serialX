//! Settings: what is the workbench's to set rather than a session's.
//!
//! There is one so far — how many lines a terminal keeps above its screen —
//! so the dialog is one field, opened from the application menu with ⌘, as
//! every macOS application opens its own. What is set is written to the
//! workspace file beside the presets, and takes effect at once in every
//! open session.

use gpui_kit::component::{
    Sizable, WindowExt,
    input::{Input, InputState},
};
use gpui_kit::*;

use crate::SerialWorkspace;
use crate::controls::{dialog_footer, eyebrow};
use crate::icons::{Glyph, icon_chip};
use crate::presets::{MAX_SCROLLBACK_LINES, MIN_SCROLLBACK_LINES, Settings};
use crate::theme::{CAPTION, LABEL, TITLE, Typography};

/// Width of the dialog: a number, and a sentence about it.
const DIALOG_WIDTH: f32 = 420.;
/// Height of the field, the same as the session dialog's fields.
const FIELD_HEIGHT: f32 = 30.;

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

impl SerialWorkspace {
    /// Opens the dialog on the settings as they are.
    pub(crate) fn open_settings_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let palette = self.interface_theme.palette();
        let current = self.presets.settings;
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(format_lines(Settings::default().scrollback_lines))
                .default_value(format_lines(current.scrollback_lines))
        });
        let field = input.clone();
        let workspace = cx.weak_entity();

        window.open_alert_dialog(cx, move |alert, _, cx| {
            let workspace = workspace.clone();
            let input = input.clone();
            // The field says as it is typed whether the number will do.
            let verdict = parse_scrollback_lines(input.read(cx).value().as_ref());
            let (hint, hint_color) = match &verdict {
                Ok(_) => (
                    "Lines kept above the screen, in every session. 100 to 1,000,000; the change takes effect at once.".to_owned(),
                    palette.muted,
                ),
                Err(reason) => ((*reason).to_owned(), palette.danger),
            };
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
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(eyebrow(palette, "Scrollback"))
                        .child(
                            Input::new(&input)
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
                                ),
                        )
                        .child(
                            div()
                                .text_token(CAPTION)
                                .text_color(rgb(hint_color))
                                .child(hint),
                        ),
                )
                .footer(dialog_footer(palette, "Save", Glyph::Bookmark, None))
                .on_ok(move |_, window, cx| {
                    // A number that will not do keeps the dialog open with
                    // the field in focus, and the reason under it.
                    let Ok(scrollback_lines) =
                        parse_scrollback_lines(input.read(cx).value().as_ref())
                    else {
                        input.update(cx, |input, cx| input.focus(window, cx));
                        return false;
                    };
                    let _ = workspace.update(cx, |workspace, cx| {
                        workspace.apply_settings(Settings { scrollback_lines }, cx);
                    });
                    true
                })
        });

        // The dialog takes focus as it opens; the field takes it back once
        // the dialog is there, with the number selected so a new one can
        // be typed over it.
        cx.defer_in(window, move |_, window, cx| {
            field.update(cx, |field, cx| {
                field.focus(window, cx);
                field.select_all(window, cx);
            });
        });
    }

    /// Takes the settings as given: writes them down, and hands every open
    /// terminal its new scrollback.
    pub(crate) fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        self.presets.set_settings(settings);
        for tab in &mut self.tabs {
            tab.terminal.set_scrollback(settings.scrollback_lines);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{format_lines, parse_scrollback_lines};

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

    #[test]
    fn counts_are_shown_with_separators() {
        assert_eq!(format_lines(100), "100");
        assert_eq!(format_lines(50_000), "50,000");
        assert_eq!(format_lines(1_000_000), "1,000,000");
    }
}
