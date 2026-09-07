//! Settings: what is the workbench's to set rather than a session's.
//!
//! Two so far — how many lines a terminal keeps above its screen, and the
//! size its log is set in — so the dialog is two fields, opened from the
//! application menu with ⌘, as every macOS application opens its own. What
//! is set is written to the workspace file beside the presets, and takes
//! effect at once in every open session.

use gpui_kit::component::{
    Sizable, WindowExt,
    input::{Input, InputState},
};
use gpui_kit::*;

use crate::SerialWorkspace;
use crate::controls::{dialog_footer, eyebrow};
use crate::icons::{Glyph, icon_chip};
use crate::presets::{
    MAX_SCROLLBACK_LINES, MAX_TERMINAL_FONT_SIZE, MIN_SCROLLBACK_LINES, MIN_TERMINAL_FONT_SIZE,
    Settings,
};
use crate::theme::{CAPTION, LABEL, TITLE, Typography, WorkbenchPalette};

/// Width of the dialog: a number, and a sentence about it.
const DIALOG_WIDTH: f32 = 420.;
/// Height of a field, the same as the session dialog's fields.
const FIELD_HEIGHT: f32 = 30.;
/// The gap between the two settings.
const SECTION_GAP: f32 = 14.;

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

/// Reads a type size as typed: a number of points, whole or with a half,
/// within the bounds the setting takes. Anything finer is rounded to the
/// nearest half point, which is as fine as the size is worth setting.
pub(crate) fn parse_font_size(text: &str) -> Result<f32, &'static str> {
    let digits = text.trim().trim_end_matches("pt").trim();
    if digits.is_empty() {
        return Err("Enter a size in points");
    }
    let size: f32 = digits.parse().map_err(|_| "Numbers only")?;
    if !size.is_finite() {
        return Err("Numbers only");
    }
    let size = (size * 2.).round() / 2.;
    if size < MIN_TERMINAL_FONT_SIZE {
        return Err("At least 8 points");
    }
    if size > MAX_TERMINAL_FONT_SIZE {
        return Err("At most 32 points");
    }
    Ok(size)
}

/// A type size as the field shows it: the half point only when there is one.
pub(crate) fn format_font_size(size: f32) -> String {
    if (size - size.round()).abs() < f32::EPSILON {
        format!("{}", size.round() as i32)
    } else {
        format!("{size:.1}")
    }
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

/// One setting: its label, the field its number is typed in with the unit
/// after it, and a line under the field — what the number means and the
/// bounds it keeps within while it will do, the reason it will not when it
/// will not, in the colour that says which.
fn setting_field(
    palette: WorkbenchPalette,
    label: &str,
    input: &Entity<InputState>,
    unit: &'static str,
    hint: Result<&str, &str>,
) -> impl IntoElement {
    let good = hint.is_ok();
    let (hint, hint_color) = match hint {
        Ok(blurb) => (blurb.to_owned(), palette.muted),
        Err(reason) => (reason.to_owned(), palette.danger),
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(eyebrow(palette, label))
        .child(
            Input::new(input)
                .small()
                .h(px(FIELD_HEIGHT))
                .ui_mono_token(LABEL)
                .bg(rgb(palette.input))
                .border_color(rgb(if good {
                    palette.input_border
                } else {
                    palette.danger
                }))
                .rounded(px(8.))
                .px_2p5()
                .focus_bordered(good)
                .suffix(
                    div()
                        .text_token(CAPTION)
                        .text_color(rgb(palette.faint))
                        .child(unit),
                ),
        )
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
        let palette = self.interface_theme.palette();
        let current = self.presets.settings;
        let defaults = Settings::default();
        let lines_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(format_lines(defaults.scrollback_lines))
                .default_value(format_lines(current.scrollback_lines))
        });
        let size_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(format_font_size(defaults.terminal_font_size))
                .default_value(format_font_size(current.terminal_font_size))
        });
        let field = lines_input.clone();
        let workspace = cx.weak_entity();

        window.open_alert_dialog(cx, move |alert, _, cx| {
            let workspace = workspace.clone();
            let lines_input = lines_input.clone();
            let size_input = size_input.clone();
            // Each field says as it is typed whether its number will do.
            let lines = parse_scrollback_lines(lines_input.read(cx).value().as_ref());
            let size = parse_font_size(size_input.read(cx).value().as_ref());
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
                        .gap(px(SECTION_GAP))
                        .child(setting_field(
                            palette,
                            "Scrollback",
                            &lines_input,
                            "lines",
                            match &lines {
                                Ok(_) => Ok("Lines kept above the screen, in every session. 100 to 1,000,000; the change takes effect at once."),
                                Err(reason) => Err(*reason),
                            },
                        ))
                        .child(setting_field(
                            palette,
                            "Terminal text",
                            &size_input,
                            "pt",
                            match &size {
                                Ok(_) => Ok("The size the log is set in, in every session. 8 to 32 points; the line numbers and timestamps follow it."),
                                Err(reason) => Err(*reason),
                            },
                        )),
                )
                .footer(dialog_footer(palette, "Save", Glyph::Bookmark, None))
                .on_ok(move |_, window, cx| {
                    // A number that will not do keeps the dialog open with
                    // that field in focus, and the reason under it.
                    let Ok(scrollback_lines) =
                        parse_scrollback_lines(lines_input.read(cx).value().as_ref())
                    else {
                        lines_input.update(cx, |input, cx| input.focus(window, cx));
                        return false;
                    };
                    let Ok(terminal_font_size) =
                        parse_font_size(size_input.read(cx).value().as_ref())
                    else {
                        size_input.update(cx, |input, cx| input.focus(window, cx));
                        return false;
                    };
                    let _ = workspace.update(cx, |workspace, cx| {
                        workspace.apply_settings(
                            Settings {
                                scrollback_lines,
                                terminal_font_size,
                            },
                            cx,
                        );
                    });
                    true
                })
        });

        // The dialog takes focus as it opens; the first field takes it back
        // once the dialog is there, with the number selected so a new one
        // can be typed over it.
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
    use super::{format_font_size, format_lines, parse_font_size, parse_scrollback_lines};

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
    fn a_type_size_is_read_as_typed_and_kept_in_bounds() {
        assert_eq!(parse_font_size("14"), Ok(14.));
        assert_eq!(parse_font_size(" 12.5 "), Ok(12.5));
        assert_eq!(parse_font_size("13pt"), Ok(13.));
        // Finer than a half point is as good as a half point.
        assert_eq!(parse_font_size("13.4"), Ok(13.5));
        assert_eq!(parse_font_size(""), Err("Enter a size in points"));
        assert_eq!(parse_font_size("big"), Err("Numbers only"));
        assert_eq!(parse_font_size("7"), Err("At least 8 points"));
        assert_eq!(parse_font_size("33"), Err("At most 32 points"));
    }

    #[test]
    fn type_sizes_are_shown_without_a_needless_half() {
        assert_eq!(format_font_size(14.), "14");
        assert_eq!(format_font_size(12.5), "12.5");
    }

    #[test]
    fn counts_are_shown_with_separators() {
        assert_eq!(format_lines(100), "100");
        assert_eq!(format_lines(50_000), "50,000");
        assert_eq!(format_lines(1_000_000), "1,000,000");
    }
}
