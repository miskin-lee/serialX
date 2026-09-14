//! A file picker for the host: the platform's own cannot see a folder on
//! another machine, so a dialog of the workbench's shows one — the folder's
//! path in a field, its entries in a list, folders first — and hands back
//! what was picked. It stands in for the picker wherever a session on a
//! host would have opened one: the file to send, the folder to receive
//! into, the folder the recordings go in.

use std::{cell::RefCell, rc::Rc};

use gpui_kit::base::Scrollbar;
use gpui_kit::component::{
    Disableable, Icon, IconName, Sizable, WindowExt,
    button::Button,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::controls::{dialog_footer, eyebrow, tag};
use crate::format_bytes;
use crate::icons::{Glyph, icon_chip};
use crate::remote::{Listing, RemoteLink, remote_join};
use crate::theme::{
    CAPTION, InterfaceTheme, LABEL, MICRO, TITLE, Typography, WorkbenchPalette, tint,
};

const DIALOG_WIDTH: f32 = 560.;
const FIELD_HEIGHT: f32 = 32.;
const ROW_HEIGHT: f32 = 30.;
/// Rows shown before the list scrolls; a scroll region needs a definite height.
const ROWS_SHOWN: f32 = 9.5;
const SECTION_GAP: f32 = 14.;

/// What the picker is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// One file, or several when the transfer can take a batch.
    Files { several: bool },
    /// The folder being shown.
    Folder,
}

/// What the dialog is handed back with.
type OnChoose = Rc<RefCell<Option<Box<dyn FnOnce(Vec<String>, &mut Window, &mut App)>>>>;

pub(crate) struct RemoteBrowser {
    theme: InterfaceTheme,
    link: RemoteLink,
    pick: Pick,
    home: String,
    /// The folder shown, once one has been listed.
    listing: Listing,
    /// Which request the list is waiting on, so a slow answer to an
    /// earlier one does not land over a later one.
    request: usize,
    loading: bool,
    error: Option<String>,
    /// The entries picked, by their place in the listing.
    selected: Vec<usize>,
    show_hidden: bool,
    path_input: Entity<InputState>,
    _path_subscription: Subscription,
    complaint: Option<&'static str>,
}

impl RemoteBrowser {
    fn new(
        theme: InterfaceTheme,
        link: RemoteLink,
        pick: Pick,
        start: String,
        home: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("/path/on/the/host"));
        let path_subscription = cx.subscribe_in(
            &path_input,
            window,
            |browser: &mut Self, input, event: &InputEvent, window, cx: &mut Context<Self>| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let typed = input.read(cx).value().trim().to_string();
                    if !typed.is_empty() {
                        browser.load(typed, window, cx);
                    }
                }
            },
        );
        let mut browser = Self {
            theme,
            link,
            pick,
            home,
            listing: Listing::default(),
            request: 0,
            loading: false,
            error: None,
            selected: Vec::new(),
            show_hidden: false,
            path_input,
            _path_subscription: path_subscription,
            complaint: None,
        };
        browser.load(start, window, cx);
        browser
    }

    /// Asks the host for a folder and shows it when the answer comes. A
    /// folder that cannot be listed leaves the last one on screen with the
    /// reason under it.
    fn load(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.request += 1;
        let request = self.request;
        self.loading = true;
        self.error = None;
        self.complaint = None;
        let link = self.link.clone();
        cx.notify();
        cx.spawn_in(window, async move |browser, cx| {
            let outcome = link.list_dir(&path).await;
            let _ = browser.update_in(cx, |browser, window, cx| {
                if browser.request != request {
                    return;
                }
                browser.loading = false;
                match outcome {
                    Ok(listing) => {
                        browser.selected.clear();
                        browser.listing = listing;
                        // The field says where the list is.
                        let path = browser.listing.path.clone();
                        browser.path_input.update(cx, |input, cx| {
                            if input.value().as_ref() != path {
                                input.set_value(path, window, cx);
                            }
                        });
                    }
                    Err(error) => browser.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn go_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(parent) = self.listing.parent.clone() {
            self.load(parent, window, cx);
        }
    }

    fn go_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let home = self.home.clone();
        self.load(home, window, cx);
    }

    /// The entries on show: the hidden ones only when asked for. Each with
    /// its place in the listing, which the selection is kept by.
    fn shown(&self) -> Vec<(usize, &crate::remote::Entry)> {
        self.listing
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.show_hidden || !entry.hidden)
            .collect()
    }

    fn press_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.listing.entries.get(index) else {
            return;
        };
        if entry.dir {
            let path = remote_join(&self.listing.path, &entry.name);
            self.load(path, window, cx);
            return;
        }
        match self.pick {
            Pick::Files { several: true } => {
                if let Some(at) = self.selected.iter().position(|&picked| picked == index) {
                    self.selected.remove(at);
                } else {
                    self.selected.push(index);
                }
            }
            Pick::Files { several: false } => self.selected = vec![index],
            Pick::Folder => {}
        }
        self.complaint = None;
        cx.notify();
    }

    fn toggle_hidden(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.show_hidden = shown;
        cx.notify();
    }

    /// What was picked, or a complaint left under the list.
    fn choice(&mut self, cx: &mut Context<Self>) -> Result<Vec<String>, ()> {
        match self.pick {
            Pick::Folder => {
                if self.listing.path.is_empty() {
                    self.complaint = Some("Wait for a folder to be listed.");
                    cx.notify();
                    return Err(());
                }
                Ok(vec![self.listing.path.clone()])
            }
            Pick::Files { .. } => {
                let mut picked: Vec<String> = self
                    .selected
                    .iter()
                    .filter_map(|&index| self.listing.entries.get(index))
                    .map(|entry| remote_join(&self.listing.path, &entry.name))
                    .collect();
                if picked.is_empty() {
                    self.complaint = Some("Pick a file from the list.");
                    cx.notify();
                    return Err(());
                }
                picked.sort();
                Ok(picked)
            }
        }
    }

    fn render_path_row(&self, palette: WorkbenchPalette, cx: &mut Context<Self>) -> AnyElement {
        let field = Input::new(&self.path_input)
            .small()
            .ui_mono_token(LABEL)
            .bg(rgb(palette.input))
            .border_color(rgb(palette.input_border))
            .rounded(px(8.))
            .px_2p5();
        h_flex()
            .items_center()
            .gap_1p5()
            .child(div().flex_1().min_w_0().h(px(FIELD_HEIGHT)).child(field))
            .child(
                Button::new("remote-browser-up")
                    .outline()
                    .small()
                    .h(px(FIELD_HEIGHT))
                    .icon(IconName::ArrowUp)
                    .disabled(self.listing.parent.is_none())
                    .tooltip("The folder above")
                    .on_click(cx.listener(|browser, _, window, cx| browser.go_up(window, cx))),
            )
            .child(
                Button::new("remote-browser-home")
                    .outline()
                    .small()
                    .h(px(FIELD_HEIGHT))
                    .label("Home")
                    .tooltip("The account's home folder on the host")
                    .on_click(cx.listener(|browser, _, window, cx| browser.go_home(window, cx))),
            )
            .into_any_element()
    }

    fn render_list(&self, palette: WorkbenchPalette, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.shown();
        let list_height = ROWS_SHOWN * ROW_HEIGHT;
        let scrolls = shown.len() as f32 * ROW_HEIGHT > list_height;
        let files_pickable = matches!(self.pick, Pick::Files { .. });
        let mut rows: Vec<AnyElement> = shown
            .iter()
            .map(|&(index, entry)| {
                let selected = self.selected.contains(&index);
                let dim = !entry.dir && !files_pickable;
                h_flex()
                    .id(("remote-entry", index))
                    .h(px(ROW_HEIGHT))
                    .flex_none()
                    .px_3()
                    .when(scrolls, |row| row.pr(px(12.) + Scrollbar::width()))
                    .gap_2p5()
                    .items_center()
                    .when(!dim, |row| row.cursor_pointer())
                    .when(selected, |row| row.bg(tint(palette.accent, 0.1)))
                    .when(!selected && !dim, |row| {
                        row.hover(|row| row.bg(rgb(palette.hover)))
                    })
                    .on_click(cx.listener(move |browser, _, window, cx| {
                        browser.press_row(index, window, cx);
                    }))
                    .child(
                        Icon::new(if entry.dir {
                            IconName::Folder
                        } else {
                            IconName::File
                        })
                        .size(px(14.))
                        .text_color(rgb(if entry.dir {
                            palette.category_session
                        } else if dim {
                            palette.faint
                        } else {
                            palette.muted
                        })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .ui_mono_token(LABEL)
                            .text_color(rgb(if dim {
                                palette.faint
                            } else {
                                palette.strong_foreground
                            }))
                            .child(entry.name.clone()),
                    )
                    .when(!entry.dir, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .text_token(CAPTION)
                                .text_color(rgb(palette.faint))
                                .child(format_bytes(entry.size)),
                        )
                    })
                    .when(selected, |row| {
                        row.child(
                            Icon::new(IconName::Check)
                                .size(px(12.))
                                .text_color(rgb(palette.accent)),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            let text = if self.loading {
                "Listing…"
            } else if self.listing.path.is_empty() {
                "Nothing listed yet"
            } else {
                "An empty folder"
            };
            rows.push(
                h_flex()
                    .h(px(ROW_HEIGHT))
                    .px_3()
                    .items_center()
                    .child(
                        div()
                            .text_token(LABEL)
                            .text_color(rgb(palette.muted))
                            .child(text),
                    )
                    .into_any_element(),
            );
        }

        div()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(palette.border_subtle))
            .bg(rgb(palette.card))
            .overflow_hidden()
            .child(
                v_flex()
                    .h(px(list_height))
                    .overflow_y_scrollbar()
                    .children(rows),
            )
            .into_any_element()
    }
}

impl Render for RemoteBrowser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        let picked = self.selected.len();
        let aside = h_flex()
            .items_center()
            .gap_3()
            .when(self.loading, |aside| {
                aside.child(tag(palette, palette.accent, MICRO, "Listing…"))
            })
            .when(picked > 0, |aside| {
                aside.child(tag(
                    palette,
                    palette.accent,
                    MICRO,
                    if picked == 1 {
                        "1 picked".to_string()
                    } else {
                        format!("{picked} picked")
                    },
                ))
            })
            .child(
                Checkbox::new("remote-browser-hidden")
                    .small()
                    .checked(self.show_hidden)
                    .label("Hidden files")
                    .on_click(cx.listener(|browser, checked: &bool, _, cx| {
                        browser.toggle_hidden(*checked, cx);
                    })),
            );
        let (hint, hint_color) = match (self.complaint, &self.error) {
            (Some(complaint), _) => (complaint.to_string(), palette.danger),
            (None, Some(error)) => (
                format!("Could not list that folder: {error}"),
                palette.danger,
            ),
            (None, None) => (
                match self.pick {
                    Pick::Files { several: true } => {
                        "Click a folder to open it; click files to pick them, again to let one go."
                    }
                    Pick::Files { several: false } => {
                        "Click a folder to open it, and the file to pick it."
                    }
                    Pick::Folder => {
                        "Click a folder to open it. Choose takes the folder being shown."
                    }
                }
                .to_string(),
                palette.muted,
            ),
        };

        v_flex().gap(px(SECTION_GAP)).child(
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .h(px(20.))
                        .items_center()
                        .justify_between()
                        .child(eyebrow(palette, "Folder on the host"))
                        .child(aside),
                )
                .child(self.render_path_row(palette, cx))
                .child(self.render_list(palette, cx))
                .child(
                    div()
                        .text_token(CAPTION)
                        .text_color(rgb(hint_color))
                        .child(hint),
                ),
        )
    }
}

/// Opens the picker on `start`, and hands `chosen` what was picked once
/// the dialog is confirmed — full paths on the host.
pub(crate) fn open_remote_browser(
    link: RemoteLink,
    pick: Pick,
    start: String,
    home: String,
    theme: InterfaceTheme,
    window: &mut Window,
    cx: &mut App,
    chosen: impl FnOnce(Vec<String>, &mut Window, &mut App) + 'static,
) {
    let palette = theme.palette();
    let browser = cx.new(|cx| RemoteBrowser::new(theme, link, pick, start, home, window, cx));
    let chosen: OnChoose = Rc::new(RefCell::new(Some(Box::new(chosen))));
    let (glyph, heading, blurb) = match pick {
        Pick::Files { several: true } => (
            Glyph::Upload,
            "Choose files on the host",
            "The files to send are on the host, so this list is the host's.",
        ),
        Pick::Files { several: false } => (
            Glyph::Upload,
            "Choose a file on the host",
            "The file to send is on the host, so this list is the host's.",
        ),
        Pick::Folder => (
            Glyph::Folder,
            "Choose a folder on the host",
            "The folder is on the host, so this list is the host's.",
        ),
    };

    window.open_alert_dialog(cx, move |alert, _, _| {
        let browser = browser.clone();
        let chosen = chosen.clone();
        alert
            .width(px(DIALOG_WIDTH))
            .p_5()
            .icon(icon_chip(glyph, palette.accent, 36.))
            .title(
                div()
                    .text_token(TITLE)
                    .text_color(rgb(palette.strong_foreground))
                    .child(heading),
            )
            .description(blurb)
            .close_button(true)
            .child(browser.clone())
            .footer(dialog_footer(palette, "Choose", Glyph::Bookmark, None))
            .on_ok(move |_, window, cx| {
                let Ok(paths) = browser.update(cx, |browser, cx| browser.choice(cx)) else {
                    return false;
                };
                if let Some(chosen) = chosen.borrow_mut().take() {
                    chosen(paths, window, cx);
                }
                true
            })
    });
}
