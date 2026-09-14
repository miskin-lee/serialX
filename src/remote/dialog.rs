//! The connect dialog: the hosts the workbench knows in a list, the one to
//! reach in a field, and how — a key file when the account's usual ones
//! are not it, a password when the host asks for one. `Save & Connect`
//! keeps the host in the list and connects; `Connect` only connects.

use gpui_kit::base::Scrollbar;
use gpui_kit::component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::SerialWorkspace;
use crate::controls::{SecondaryConfirm, dialog_footer, eyebrow, tag};
use crate::icons::{Glyph, icon_chip};
use crate::remote::RemoteHost;
use crate::theme::{
    CAPTION, InterfaceTheme, LABEL, MICRO, TITLE, Typography, WorkbenchPalette, tint,
};

const DIALOG_WIDTH: f32 = 560.;
const FIELD_HEIGHT: f32 = 32.;
const ROW_HEIGHT: f32 = 44.;
const ROWS_SHOWN: f32 = 4.5;
const SECTION_GAP: f32 = 14.;
const COLUMN_GAP: f32 = 10.;
const ROW_ACTION_SIZE: f32 = 22.;
const ROW_ACTION_REST: f32 = 0.55;

/// The connection as it is being set up. Nothing is tried until the
/// dialog is confirmed.
struct RemoteEditor {
    theme: InterfaceTheme,
    workspace: WeakEntity<SerialWorkspace>,
    /// The host book, as it stood when the dialog opened and as rows are
    /// taken out of it.
    hosts: Vec<RemoteHost>,
    /// The saved host the fields were filled from, kept so `Save` writes
    /// over it rather than adding another.
    selected: Option<u64>,
    address_input: Entity<InputState>,
    name_input: Entity<InputState>,
    identity_input: Entity<InputState>,
    password_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    /// The host the workbench is on now, if any, to say so.
    connected: Option<String>,
    /// Why the dialog would not confirm, after a try.
    complaint: Option<&'static str>,
    /// Set by `Connect` just before it confirms, and taken by the confirm.
    connect_only: bool,
}

impl RemoteEditor {
    fn new(
        theme: InterfaceTheme,
        workspace: WeakEntity<SerialWorkspace>,
        hosts: Vec<RemoteHost>,
        connected: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let address_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("user@host, or user@host:port"));
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Name (optional)"));
        let identity_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Key file (optional; the usual ones otherwise)")
        });
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password or key passphrase (optional)")
                .masked(true)
        });
        let mut subscriptions = Vec::new();
        for input in [
            &address_input,
            &name_input,
            &identity_input,
            &password_input,
        ] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |editor: &mut Self, _, event: &InputEvent, _, cx: &mut Context<Self>| {
                    if matches!(event, InputEvent::Change) {
                        editor.complaint = None;
                        cx.notify();
                    }
                },
            ));
        }
        Self {
            theme,
            workspace,
            hosts,
            selected: None,
            address_input,
            name_input,
            identity_input,
            password_input,
            _subscriptions: subscriptions,
            connected,
            complaint: None,
            connect_only: false,
        }
    }

    fn focus_address(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.address_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Fills the fields from a saved host.
    fn pick_host(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.hosts.iter().find(|host| host.id == id).cloned() else {
            return;
        };
        self.selected = Some(id);
        self.complaint = None;
        let address = host.address();
        let name = host.name.clone();
        let identity = host.identity.clone().unwrap_or_default();
        self.address_input
            .update(cx, |input, cx| input.set_value(address, window, cx));
        self.name_input
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.identity_input
            .update(cx, |input, cx| input.set_value(identity, window, cx));
        cx.notify();
    }

    /// Takes a host out of the book, here and in the workbench.
    fn forget_host(&mut self, id: u64, cx: &mut Context<Self>) {
        self.hosts.retain(|host| host.id != id);
        if self.selected == Some(id) {
            self.selected = None;
        }
        let _ = self.workspace.update(cx, |workspace, cx| {
            workspace.hosts.remove(id);
            cx.notify();
        });
        cx.notify();
    }

    /// Asks the platform for the key file.
    fn choose_identity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |editor, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = editor.update_in(cx, |editor, window, cx| {
                let text = path.to_string_lossy().into_owned();
                editor.identity_input.update(cx, |input, cx| {
                    input.set_value(text, window, cx);
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// The host as typed, or a complaint left under the field.
    fn host(&mut self, cx: &mut Context<Self>) -> Result<(RemoteHost, Option<String>), ()> {
        let address = self.address_input.read(cx).value().to_string();
        let (user, host, port) = match RemoteHost::parse_address(&address) {
            Ok(parts) => parts,
            Err(complaint) => {
                self.complaint = Some(complaint);
                cx.notify();
                return Err(());
            }
        };
        let name = self.name_input.read(cx).value().trim().to_string();
        let identity = self.identity_input.read(cx).value().trim().to_string();
        let password = self.password_input.read(cx).value().to_string();
        Ok((
            RemoteHost {
                id: self.selected.unwrap_or(0),
                name,
                host,
                user,
                port,
                identity: (!identity.is_empty()).then_some(identity),
            },
            (!password.is_empty()).then_some(password),
        ))
    }

    /// A text field at the dialog's field height. `Input::h` is the
    /// multi-line box's own and shadows the style, so the frame is given
    /// its height through `Styled`, or a single line keeps the height of
    /// its size.
    fn field(palette: WorkbenchPalette, input: &Entity<InputState>) -> Input {
        Styled::h(
            Input::new(input)
                .small()
                .ui_mono_token(LABEL)
                .bg(rgb(palette.input))
                .border_color(rgb(palette.input_border))
                .rounded(px(8.))
                .px_2p5(),
            px(FIELD_HEIGHT),
        )
    }

    fn section(
        palette: WorkbenchPalette,
        label: &str,
        aside: Option<AnyElement>,
        control: impl IntoElement,
        hint: Result<&str, &str>,
    ) -> impl IntoElement {
        let (hint, color) = match hint {
            Ok(blurb) => (blurb.to_owned(), palette.muted),
            Err(complaint) => (complaint.to_owned(), palette.danger),
        };
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .h(px(20.))
                    .items_center()
                    .justify_between()
                    .child(eyebrow(palette, label))
                    .children(aside),
            )
            .child(control)
            .child(div().text_token(CAPTION).text_color(rgb(color)).child(hint))
    }

    fn render_hosts(&self, palette: WorkbenchPalette, cx: &mut Context<Self>) -> AnyElement {
        let list_height = ROWS_SHOWN.min(self.hosts.len().max(1) as f32) * ROW_HEIGHT;
        let scrolls = self.hosts.len() as f32 * ROW_HEIGHT > list_height;
        let connected = self.connected.clone();
        let mut rows: Vec<AnyElement> = self
            .hosts
            .iter()
            .enumerate()
            .map(|(index, host)| {
                let id = host.id;
                let selected = self.selected == Some(id);
                let title = host.title();
                let address = host.address();
                let on_it = connected.as_deref() == Some(title.as_str());
                h_flex()
                    .id(("remote-host", index))
                    .group("remote-host")
                    .h(px(ROW_HEIGHT))
                    .flex_none()
                    .px_3()
                    .when(scrolls, |row| row.pr(px(12.) + Scrollbar::width()))
                    .gap_3()
                    .items_center()
                    .cursor_pointer()
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(rgb(palette.border_subtle))
                    })
                    .when(selected, |row| row.bg(tint(palette.accent, 0.1)))
                    .when(!selected, |row| row.hover(|row| row.bg(rgb(palette.hover))))
                    .on_click(cx.listener(move |editor, _, window, cx| {
                        editor.pick_host(id, window, cx);
                    }))
                    .child(icon_chip(Glyph::Remote, palette.category_session, 26.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_token(LABEL)
                                    .text_color(rgb(palette.strong_foreground))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .ui_mono_token(CAPTION)
                                    .text_color(rgb(palette.muted))
                                    .child(address),
                            ),
                    )
                    .when(on_it, |row| {
                        row.child(tag(palette, palette.success, MICRO, "Connected"))
                    })
                    .child(
                        h_flex()
                            .flex_none()
                            .opacity(ROW_ACTION_REST)
                            .group_hover("remote-host", |actions| actions.opacity(1.))
                            .child(
                                Button::new(("remote-host-forget", index))
                                    .ghost()
                                    .with_size(px(ROW_ACTION_SIZE))
                                    .icon(Glyph::Trash)
                                    .tooltip("Forget this host")
                                    .on_click(cx.listener(move |editor, _, _, cx| {
                                        cx.stop_propagation();
                                        editor.forget_host(id, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .size(px(16.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(selected, |mark| {
                                mark.bg(rgb(palette.accent)).child(
                                    Icon::new(IconName::Check)
                                        .size(px(10.))
                                        .text_color(rgb(0xffffff)),
                                )
                            })
                            .when(!selected, |mark| {
                                mark.border_1().border_color(rgb(palette.badge))
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            rows.push(
                h_flex()
                    .h(px(ROW_HEIGHT))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_token(LABEL)
                            .text_color(rgb(palette.muted))
                            .child("No saved hosts yet"),
                    )
                    .child(
                        div()
                            .text_token(CAPTION)
                            .text_color(rgb(palette.faint))
                            .child("Type one below and Save & Connect"),
                    )
                    .into_any_element(),
            );
        }
        let count = self.hosts.len();
        let aside = div()
            .text_token(CAPTION)
            .text_color(rgb(palette.faint))
            .child(if count == 1 {
                "1 host".to_string()
            } else {
                format!("{count} hosts")
            })
            .into_any_element();

        Self::section(
            palette,
            "Saved hosts",
            Some(aside),
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
                ),
            Ok("Click a host to fill the fields from it. The list is this machine's, whichever host is connected."),
        )
        .into_any_element()
    }
}

impl Render for RemoteEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        let hosts = self.render_hosts(palette, cx);
        let address = Self::field(palette, &self.address_input);
        let name = Self::field(palette, &self.name_input);
        let identity = Self::field(palette, &self.identity_input);
        let password = Self::field(palette, &self.password_input);
        let host_hint = match self.complaint {
            Some(complaint) => Err(complaint),
            None => Ok(
                "What you would give ssh: a host from ~/.ssh/config works, and so does an address. Open sessions close when the workspace moves to the host.",
            ),
        };
        let connected = self.connected.clone();

        v_flex()
            .gap(px(SECTION_GAP))
            .child(hosts)
            .child(Self::section(
                palette,
                "Host",
                connected.map(|title| {
                    tag(palette, palette.success, MICRO, format!("On {title}")).into_any_element()
                }),
                h_flex()
                    .items_center()
                    .gap(px(COLUMN_GAP))
                    .child(div().flex_1().min_w_0().child(address))
                    .child(div().w(px(180.)).flex_none().child(name)),
                host_hint,
            ))
            .child(Self::section(
                palette,
                "Authentication",
                None,
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(identity))
                            .child(
                                Button::new("remote-identity-choose")
                                    .outline()
                                    .small()
                                    .h(px(FIELD_HEIGHT))
                                    .label("Choose…")
                                    .tooltip("Pick a private key file on this machine")
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.choose_identity(window, cx);
                                    })),
                            ),
                    )
                    .child(password),
                Ok("Keys, agents and jump hosts from ~/.ssh/config are used as they are. A password is handed to ssh only when typed here, and is kept nowhere."),
            ))
    }
}

impl SerialWorkspace {
    /// `Remote > Connect to Host…`: the dialog, on the host book as it is.
    pub(crate) fn open_remote_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.interface_theme;
        let palette = theme.palette();
        let hosts = self.hosts.hosts.clone();
        let connected = self.remote_title();
        let workspace = cx.weak_entity();
        let editor =
            cx.new(|cx| RemoteEditor::new(theme, workspace.clone(), hosts, connected, window, cx));
        let field = editor.clone();
        let editor_for_footer = editor.clone();

        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let editor = editor.clone();
            let secondary = {
                let editor = editor_for_footer.clone();
                Some(SecondaryConfirm {
                    label: "Connect",
                    glyph: Glyph::Bolt,
                    before: Box::new(move |_, cx| {
                        editor.update(cx, |editor, _| editor.connect_only = true);
                    }),
                })
            };
            alert
                .width(px(DIALOG_WIDTH))
                .p_5()
                .icon(icon_chip(Glyph::Remote, palette.accent, 36.))
                .title(
                    div()
                        .text_token(TITLE)
                        .text_color(rgb(palette.strong_foreground))
                        .child("Connect to a host"),
                )
                .description(
                    "The workbench moves to the host: its serial devices, its saved sessions and commands, its folders. Nothing is installed there.",
                )
                .close_button(true)
                .child(editor.clone())
                .footer(dialog_footer(palette, "Save & Connect", Glyph::Bookmark, secondary))
                .on_ok(move |_, _, cx| {
                    let connect_only =
                        editor.update(cx, |editor, _| std::mem::take(&mut editor.connect_only));
                    let Ok((host, password)) = editor.update(cx, |editor, cx| editor.host(cx))
                    else {
                        return false;
                    };
                    let _ = workspace.update(cx, |workspace, cx| {
                        let host = if connect_only {
                            host
                        } else {
                            let id = workspace.hosts.keep(host.clone());
                            RemoteHost { id, ..host }
                        };
                        workspace.connect_remote(host, password, cx);
                    });
                    true
                })
        });

        cx.defer_in(window, move |_, window, cx| {
            field.update(cx, |editor, cx| editor.focus_address(window, cx));
        });
    }
}
