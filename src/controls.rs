//! The small controls the chrome is assembled from: segmented switches, choice
//! chips and section eyebrows.
//!
//! They are plain elements rather than `gpui-component` widgets so the dialog,
//! the composer and the title bar all pick from one drawer. Every control here
//! is a pill or a rounded plate on a hairline: the same vocabulary the title
//! bar speaks, so a form opened from the bar reads as part of the same surface.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{EYEBROW, TextToken, Typography, WorkbenchPalette, tint};

/// What picking a [`Choice`] does.
type OnChoose = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// One option in a [`segmented`] switch or a [`chip_row`].
pub(crate) struct Choice {
    label: SharedString,
    active: bool,
    on_click: OnChoose,
}

impl Choice {
    pub(crate) fn new(
        label: impl Into<SharedString>,
        active: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            active,
            on_click: Box::new(on_click),
        }
    }
}

/// How the options of a control are set: the type token, and whether they
/// read in the chrome's monospace (numbers, port parameters) or the UI face.
#[derive(Clone, Copy)]
pub(crate) struct ChoiceText {
    pub(crate) token: TextToken,
    pub(crate) mono: bool,
}

impl ChoiceText {
    pub(crate) const fn ui(token: TextToken) -> Self {
        Self { token, mono: false }
    }

    pub(crate) const fn mono(token: TextToken) -> Self {
        Self { token, mono: true }
    }

    fn apply<T: Styled>(self, element: T) -> T {
        // The family has to be set before the token: a whole `Font` resets
        // the weight the token then puts back.
        let element = if self.mono {
            element.ui_mono_font()
        } else {
            element
        };
        element.text_token(self.token)
    }
}

/// A segmented switch: every option shares one rail, and the active one is a
/// raised plate. For a handful of exclusive options that all fit on one line.
pub(crate) fn segmented(
    id: impl Into<ElementId>,
    palette: WorkbenchPalette,
    text: ChoiceText,
    choices: Vec<Choice>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(8.))
        .bg(rgb(palette.surface))
        .border_1()
        .border_color(rgb(palette.border_subtle))
        .children(choices.into_iter().enumerate().map(|(index, choice)| {
            text.apply(
                div()
                    .id(("segment", index))
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .py(px(3.))
                    .rounded(px(6.))
                    .border_1()
                    .text_center()
                    .whitespace_nowrap()
                    .cursor_pointer(),
            )
            .when(choice.active, |segment| {
                segment
                    .bg(rgb(palette.card))
                    .border_color(tint(palette.strong_foreground, 0.08))
                    .text_color(rgb(palette.strong_foreground))
            })
            .when(!choice.active, |segment| {
                segment
                    .border_color(transparent_black())
                    .text_color(rgb(palette.muted))
                    .hover(|segment| {
                        segment
                            .bg(rgb(palette.hover))
                            .text_color(rgb(palette.foreground))
                    })
            })
            .on_click(choice.on_click)
            .child(choice.label)
        }))
}

/// A row of choice chips: free-standing pills that wrap when the row is full.
/// For options too many, or too long, to share one segmented rail.
pub(crate) fn chip_row(
    id: impl Into<ElementId>,
    palette: WorkbenchPalette,
    text: ChoiceText,
    choices: Vec<Choice>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_row()
        .flex_wrap()
        .gap_1p5()
        .children(choices.into_iter().enumerate().map(|(index, choice)| {
            text.apply(
                div()
                    .id(("chip", index))
                    .h(px(26.))
                    .px_2p5()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .cursor_pointer()
                    .whitespace_nowrap(),
            )
            .when(choice.active, |chip| {
                chip.bg(tint(palette.accent, 0.14))
                    .border_color(tint(palette.accent, 0.55))
                    .text_color(rgb(palette.accent))
            })
            .when(!choice.active, |chip| {
                chip.bg(rgb(palette.card))
                    .border_color(rgb(palette.border_subtle))
                    .text_color(rgb(palette.foreground))
                    .hover(|chip| {
                        chip.bg(rgb(palette.hover))
                            .border_color(rgb(palette.border))
                    })
            })
            .on_click(choice.on_click)
            .child(choice.label)
        }))
}

/// A small tinted pill carrying one word: `Demo`, `Offline`, `3 / 40`.
pub(crate) fn tag(
    palette: WorkbenchPalette,
    color: u32,
    token: TextToken,
    text: impl Into<SharedString>,
) -> impl IntoElement {
    let _ = palette;
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded_full()
        .bg(tint(color, 0.14))
        .text_token(token)
        .text_color(rgb(color))
        .whitespace_nowrap()
        .child(text.into())
}

/// A section label in small caps. Tracking has to be typed in, so the label is
/// upper-cased here and the letters carry the spacing.
pub(crate) fn eyebrow(palette: WorkbenchPalette, text: &str) -> impl IntoElement {
    div()
        .text_token(EYEBROW)
        .text_color(rgb(palette.muted))
        .whitespace_nowrap()
        .child(spaced_caps(text))
}

/// `Baud rate` as `B A U D  R A T E`, with thin spaces: the closest GPUI gets
/// to letter-spacing.
fn spaced_caps(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push_str("  ");
        }
        let mut letters = word.chars().flat_map(char::to_uppercase).peekable();
        while let Some(letter) = letters.next() {
            out.push(letter);
            if letters.peek().is_some() {
                out.push('\u{2009}');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::spaced_caps;

    #[test]
    fn eyebrows_are_upper_cased_and_thin_spaced() {
        assert_eq!(
            spaced_caps("Device"),
            "D\u{2009}E\u{2009}V\u{2009}I\u{2009}C\u{2009}E"
        );
        assert_eq!(
            spaced_caps("Baud rate"),
            "B\u{2009}A\u{2009}U\u{2009}D  R\u{2009}A\u{2009}T\u{2009}E"
        );
        assert_eq!(spaced_caps(""), "");
    }
}
