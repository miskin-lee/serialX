//! The command dialog: where a command is kept for Quick send, under a name
//! and in a group.
//!
//! Four fields. The command, as it will go to the port, with what follows
//! it beside — the ending the composer is set to when the dialog opens,
//! kept with the card and changed here whenever; the name its card is
//! headed with, which stands in for the command wherever the card is shown
//! and is the command itself when the field is left blank; and the group
//! the card files under, a field that opens the list of the groups there
//! are with an offer to make one, the way the session dialog's does.
//! The composer's bookmark opens the dialog with what the composer holds,
//! the plus in the Quick send header opens it empty, and a card's pencil
//! opens it on the card.

use gpui_kit::component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::SerialWorkspace;
use crate::controls::{dialog_footer, eyebrow};
use crate::groups::GroupPrompt;
use crate::icons::{Glyph, icon_chip};
use crate::presets::{Library, StoredGroup};
use crate::serial::LineEnding;
use crate::theme::{InterfaceTheme, LABEL, MONO_SMALL, TITLE, Typography, WorkbenchPalette};

/// Width of the dialog: a command of some length, and a name beside a group.
const DIALOG_WIDTH: f32 = 460.;
/// Height of a field. The dialog holds few of them, so they stand a little
/// taller than the session dialog's, where a screenful of fields has to fit.
const FIELD_HEIGHT: f32 = 34.;
/// Width of the group field, and the narrowest its list opens: as the
/// session dialog sizes them, so the two dialogs' fields line up.
const GROUP_FIELD_WIDTH: f32 = 136.;
const GROUP_LIST_MIN_WIDTH: f32 = 180.;
/// Width of the ending field, and the narrowest its list opens: enough for
/// `CRLF` beside its glyph, and for the list to spell the bytes.
const ENDING_FIELD_WIDTH: f32 = 96.;
const ENDING_LIST_MIN_WIDTH: f32 = 140.;
/// What the group field says while the command is in no group.
const NO_GROUP: &str = "No group";
/// What the name field says while it is empty and so is the command.
const NAME_FALLBACK: &str = "Name";
/// The gap between the two sections.
const SECTION_GAP: f32 = 14.;

/// What the dialog is for: a command not yet kept, or one that is.
#[derive(Clone, Copy)]
pub(crate) enum CommandTarget {
    New,
    Saved(u64),
}

struct CommandEditor {
    theme: InterfaceTheme,
    /// The workspace the dialog was opened from: where a group made from
    /// here is kept.
    workspace: WeakEntity<SerialWorkspace>,
    /// The command as it will be sent. Its text is the name field's
    /// placeholder, so an empty name shows what the card will say instead.
    command_input: Entity<InputState>,
    _command_subscription: Subscription,
    alias_input: Entity<InputState>,
    /// What goes out after the command. It opens on the composer's own and
    /// is the card's from then on.
    ending: LineEnding,
    /// The group the command files under, and the groups there are to pick
    /// from — a copy, refreshed when one is made from the dialog.
    group: Option<u64>,
    groups: Vec<StoredGroup>,
}

impl CommandEditor {
    #[allow(clippy::too_many_arguments)]
    fn new(
        theme: InterfaceTheme,
        workspace: WeakEntity<SerialWorkspace>,
        command: String,
        alias: Option<String>,
        ending: LineEnding,
        group: Option<u64>,
        groups: Vec<StoredGroup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = Self::name_placeholder(&command);
        let command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Command")
                .default_value(command)
        });
        let alias_input = cx.new(|cx| {
            let input = InputState::new(window, cx).placeholder(placeholder);
            match alias {
                Some(alias) => input.default_value(alias),
                None => input,
            }
        });
        let name_field = alias_input.clone();
        let command_subscription = cx.subscribe_in(
            &command_input,
            window,
            move |_, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let placeholder = Self::name_placeholder(&input.read(cx).value());
                    name_field.update(cx, |field, cx| {
                        field.set_placeholder(placeholder, window, cx);
                    });
                }
            },
        );

        Self {
            theme,
            workspace,
            command_input,
            _command_subscription: command_subscription,
            alias_input,
            ending,
            group,
            groups,
        }
    }

    /// What an empty name field shows: the command the card would be headed
    /// with instead.
    fn name_placeholder(command: &str) -> String {
        let command = command.trim();
        if command.is_empty() {
            NAME_FALLBACK.to_string()
        } else {
            command.to_string()
        }
    }

    fn command(&self, cx: &App) -> String {
        self.command_input.read(cx).value().trim().to_string()
    }

    /// The name as typed, or none when the field is empty or only spaces.
    fn alias(&self, cx: &App) -> Option<String> {
        let text = self.alias_input.read(cx).value();
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    /// Puts the cursor where typing should start: on the command while
    /// there is none, else on the name — which is what the dialog is for
    /// when it opens with a command already typed.
    fn focus_first(&self, window: &mut Window, cx: &mut Context<Self>) {
        let field = if self.command(cx).is_empty() {
            &self.command_input
        } else {
            &self.alias_input
        };
        field.update(cx, |field, cx| field.focus(window, cx));
    }

    fn focus_command(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.command_input
            .update(cx, |field, cx| field.focus(window, cx));
    }

    fn select_ending(&mut self, ending: LineEnding, cx: &mut Context<Self>) {
        self.ending = ending;
        cx.notify();
    }

    fn select_group(&mut self, group: Option<u64>, cx: &mut Context<Self>) {
        self.group = group;
        cx.notify();
    }

    fn group_name(&self) -> Option<String> {
        let id = self.group?;
        self.groups
            .iter()
            .find(|group| group.id == id)
            .map(|group| group.name.clone())
    }

    /// Opens the group prompt over the dialog. The group it makes becomes
    /// the pick.
    fn new_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.weak_entity();
        let _ = self.workspace.update(cx, |workspace, cx| {
            workspace.open_group_prompt(
                GroupPrompt::New(Library::Commands),
                move |group, _, cx| {
                    let _ = editor.update(cx, |editor, cx| editor.adopt_group(group, cx));
                },
                window,
                cx,
            );
        });
    }

    /// Takes the workspace's groups again and picks one of them.
    fn adopt_group(&mut self, group: u64, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.upgrade() {
            self.groups = workspace
                .read(cx)
                .presets
                .groups_in(Library::Commands)
                .cloned()
                .collect();
        }
        self.group = Some(group);
        cx.notify();
    }

    /// A section: an eyebrow over the control.
    fn section(palette: WorkbenchPalette, title: &str, body: impl IntoElement) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .h(px(20.))
                    .items_center()
                    .child(eyebrow(palette, title)),
            )
            .child(body)
    }

    /// A text field drawn as the session dialog draws its own. `Input::h`
    /// is the multi-line box's own and shadows the style, so the frame is
    /// given its height through `Styled`, or a single line keeps the height
    /// of its size.
    fn field(input: &Entity<InputState>, palette: WorkbenchPalette) -> Input {
        Styled::h(
            Input::new(input)
                .small()
                .text_token(LABEL)
                .font_weight(FontWeight::NORMAL)
                .bg(rgb(palette.input))
                .border_color(rgb(palette.input_border))
                .rounded(px(8.))
                .px_2p5()
                .cleanable(true),
            px(FIELD_HEIGHT),
        )
    }

    /// A field that is picked from rather than typed into: a glyph, what is
    /// picked, and a caret, in the frame the text fields wear so a row of
    /// the two reads as one.
    fn picker(
        id: &'static str,
        palette: WorkbenchPalette,
        width: f32,
        glyph: impl IntoElement,
        label: String,
        ink: u32,
    ) -> Button {
        Button::new(id)
            .ghost()
            .with_size(px(FIELD_HEIGHT))
            .w(px(width))
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
                            .items_center()
                            .gap_1p5()
                            .child(glyph)
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_token(LABEL)
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(rgb(ink))
                                    .child(label),
                            ),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(px(12.))
                            .text_color(rgb(palette.muted)),
                    ),
            )
    }

    /// The ending field: what the card sends after the command, listed the
    /// way the composer's own switch lists it, with the bytes spelled out.
    fn render_ending_field(
        &mut self,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.ending;
        let editor = cx.weak_entity();

        Self::picker(
            "command-ending",
            palette,
            ENDING_FIELD_WIDTH,
            div()
                .text_token(LABEL)
                .text_color(rgb(palette.faint))
                .child("↵"),
            current.label().to_string(),
            palette.foreground,
        )
        .tooltip("What goes out after this command")
        .dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu, _, _| {
            menu = menu.min_w(px(ENDING_FIELD_WIDTH.max(ENDING_LIST_MIN_WIDTH)));
            for choice in LineEnding::ALL {
                let editor = editor.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(choice.label())
                            .when(!choice.spelled().is_empty(), |row| {
                                row.child(
                                    div()
                                        .ui_mono_token(MONO_SMALL)
                                        .text_color(rgb(palette.muted))
                                        .child(choice.spelled()),
                                )
                            })
                    })
                    .checked(choice == current)
                    .on_click(move |_, _, cx| {
                        let _ = editor.update(cx, |editor, cx| editor.select_ending(choice, cx));
                    }),
                );
            }
            menu
        })
        .into_any_element()
    }

    /// The group field: a folder, the group's name or `No group`, and a
    /// caret, opening the list of groups with a way to make one at its
    /// foot — the session dialog's field, in the commands' colour.
    fn render_group_field(
        &mut self,
        palette: WorkbenchPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = self.group_name();
        let current = self.group;
        let groups = self.groups.clone();
        let editor = cx.weak_entity();
        let ink = if name.is_some() {
            palette.category_command
        } else {
            palette.muted
        };

        Self::picker(
            "command-group",
            palette,
            GROUP_FIELD_WIDTH,
            Icon::new(Glyph::Folder).size(px(14.)).text_color(rgb(ink)),
            name.unwrap_or_else(|| NO_GROUP.to_string()),
            ink,
        )
        .tooltip("The group this command files under in Quick send")
        .dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu, _, _| {
            menu = menu.min_w(px(GROUP_FIELD_WIDTH.max(GROUP_LIST_MIN_WIDTH)));
            let pick = |group: Option<u64>| {
                let editor = editor.clone();
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    let _ = editor.update(cx, |editor, cx| editor.select_group(group, cx));
                }
            };
            menu = menu.item(
                PopupMenuItem::new(NO_GROUP)
                    .checked(current.is_none())
                    .on_click(pick(None)),
            );
            if !groups.is_empty() {
                menu = menu.separator();
            }
            for group in &groups {
                menu = menu.item(
                    PopupMenuItem::new(group.name.clone())
                        .icon(Icon::new(Glyph::Folder))
                        .checked(current == Some(group.id))
                        .on_click(pick(Some(group.id))),
                );
            }
            let editor = editor.clone();
            menu.separator().item(
                PopupMenuItem::new("New group…")
                    .icon(Icon::new(Glyph::FolderPlus))
                    .on_click(move |_, window, cx| {
                        let _ = editor.update(cx, |editor, cx| editor.new_group(window, cx));
                    }),
            )
        })
        .into_any_element()
    }
}

impl Render for CommandEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        let command = Self::field(&self.command_input, palette)
            .ui_mono_token(LABEL)
            .prefix(
                Icon::new(Glyph::Terminal)
                    .size(px(14.))
                    .text_color(rgb(palette.muted)),
            );
        let name = Self::field(&self.alias_input, palette);
        let ending = self.render_ending_field(palette, cx);
        let group = self.render_group_field(palette, cx);

        v_flex()
            .gap(px(SECTION_GAP))
            .child(Self::section(
                palette,
                "Command",
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(command))
                    .child(ending),
            ))
            .child(Self::section(
                palette,
                "Save as",
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(name))
                    .child(group),
            ))
    }
}

impl SerialWorkspace {
    /// Opens the dialog: on a saved command, or on a new one with `draft`
    /// — what the composer holds — already in the command field.
    pub(crate) fn open_command_dialog(
        &mut self,
        target: CommandTarget,
        draft: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (command, alias, ending, group) = match target {
            // A new card is ended the way the composer is set, so keeping
            // what was just typed keeps how it would have gone out.
            CommandTarget::New => (
                draft.trim().to_string(),
                None,
                self.composer_line_ending(),
                None,
            ),
            CommandTarget::Saved(id) => match self.presets.command(id) {
                Some(saved) => (
                    saved.command.clone(),
                    saved.alias().map(str::to_owned),
                    saved.ending,
                    self.presets.resolve_group(Library::Commands, saved.group),
                ),
                // The card went while the button was under the pointer.
                None => return,
            },
        };
        let theme = self.interface_theme;
        let palette = theme.palette();
        let groups = self
            .presets
            .groups_in(Library::Commands)
            .cloned()
            .collect::<Vec<_>>();
        let workspace = cx.weak_entity();
        let editor = cx.new(|cx| {
            CommandEditor::new(
                theme,
                workspace.clone(),
                command,
                alias,
                ending,
                group,
                groups,
                window,
                cx,
            )
        });
        let (title, blurb, confirm) = match target {
            CommandTarget::New => (
                "Save command",
                "Kept in Quick send, to send to the session in front with a click. A name stands in for the command on its card.",
                "Save Command",
            ),
            CommandTarget::Saved(_) => (
                "Edit command",
                "Changes apply to the card in Quick send.",
                "Save Changes",
            ),
        };
        let field = editor.clone();

        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let editor = editor.clone();
            alert
                .width(px(DIALOG_WIDTH))
                .p_5()
                .icon(icon_chip(Glyph::Prompt, palette.category_command, 36.))
                .title(
                    div()
                        .text_token(TITLE)
                        .text_color(rgb(palette.strong_foreground))
                        .child(title),
                )
                .description(blurb)
                .close_button(true)
                .child(editor.clone())
                .footer(dialog_footer(palette, confirm, Glyph::Bookmark, None))
                .on_ok(move |_, window, cx| {
                    let (command, alias, ending, group) = {
                        let editor = editor.read(cx);
                        (
                            editor.command(cx),
                            editor.alias(cx),
                            editor.ending,
                            editor.group,
                        )
                    };
                    // A blank command keeps the dialog open with the field
                    // in focus, rather than keeping a card that sends
                    // nothing.
                    if command.is_empty() {
                        editor.update(cx, |editor, cx| editor.focus_command(window, cx));
                        return false;
                    }
                    let _ = workspace.update(cx, |workspace, cx| {
                        match target {
                            CommandTarget::New => {
                                workspace.presets.add_command(alias, command, group, ending);
                            }
                            CommandTarget::Saved(id) => {
                                workspace
                                    .presets
                                    .update_command(id, alias, command, group, ending);
                            }
                        }
                        workspace.commands_collapsed = false;
                        if let Some(group) =
                            workspace.presets.resolve_group(Library::Commands, group)
                        {
                            workspace.reveal_group(group);
                        }
                        cx.notify();
                    });
                    true
                })
        });

        // The dialog takes focus as it opens; the field takes it back once
        // the dialog is there, so typing can start at once.
        cx.defer_in(window, move |_, window, cx| {
            field.update(cx, |editor, cx| editor.focus_first(window, cx));
        });
    }
}
