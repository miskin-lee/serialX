//! Groups: the folders the side panel files saved sessions and commands
//! under.
//!
//! A group is only a name, so one small prompt makes one or renames one. The
//! session dialog and the command dialog open the same prompt when none of
//! the groups there are fits what is being saved, and take the new group as
//! their pick.

use std::rc::Rc;

use gpui_kit::component::{
    Sizable, WindowExt,
    input::{Input, InputState},
};
use gpui_kit::*;

use crate::SerialWorkspace;
use crate::controls::dialog_footer;
use crate::icons::{Glyph, icon_chip};
use crate::presets::Library;
use crate::theme::{LABEL, TITLE, Typography};

/// Width of the prompt: a name, and a sentence about where it goes.
const PROMPT_WIDTH: f32 = 420.;
/// Height of the name field, the same as the session dialog's fields.
const FIELD_HEIGHT: f32 = 30.;

/// What the prompt is for: a group there is not yet, in one of the two
/// libraries, or a new name for one there is.
#[derive(Clone, Copy)]
pub(crate) enum GroupPrompt {
    New(Library),
    Rename(u64),
}

impl SerialWorkspace {
    /// Opens the prompt. Once the group has been made or renamed, `on_saved`
    /// runs with its id, for a caller that wants to pick it straight away.
    pub(crate) fn open_group_prompt(
        &mut self,
        prompt: GroupPrompt,
        on_saved: impl Fn(u64, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (current, library) = match prompt {
            GroupPrompt::New(library) => (None, library),
            GroupPrompt::Rename(id) => match self.presets.group(id) {
                Some(group) => (Some(group.name.clone()), group.library),
                // The group went while the button was under the pointer.
                None => return,
            },
        };
        let palette = self.interface_theme.palette();
        let (title, blurb, confirm) = match (prompt, library) {
            (GroupPrompt::New(_), Library::Sessions) => (
                "New group",
                "A folder in the saved sessions list. Pick it for a session in the session dialog.",
                "Create Group",
            ),
            (GroupPrompt::New(_), Library::Commands) => (
                "New group",
                "A folder in the Quick send list. Pick it for a command as you save one.",
                "Create Group",
            ),
            (GroupPrompt::Rename(_), Library::Sessions) => (
                "Rename group",
                "The sessions in it stay where they are.",
                "Rename",
            ),
            (GroupPrompt::Rename(_), Library::Commands) => (
                "Rename group",
                "The commands in it stay where they are.",
                "Rename",
            ),
        };
        let hue = match library {
            Library::Sessions => palette.category_session,
            Library::Commands => palette.category_command,
        };

        let input = cx.new(|cx| {
            let input = InputState::new(window, cx).placeholder("Group name");
            match current {
                Some(name) => input.default_value(name),
                None => input,
            }
        });
        let field = input.clone();
        let workspace = cx.weak_entity();
        let on_saved = Rc::new(on_saved);

        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let input = input.clone();
            let on_saved = on_saved.clone();
            alert
                .width(px(PROMPT_WIDTH))
                .p_5()
                .icon(icon_chip(Glyph::Folder, hue, 36.))
                .title(
                    div()
                        .text_token(TITLE)
                        .text_color(rgb(palette.strong_foreground))
                        .child(title),
                )
                .description(blurb)
                .close_button(true)
                .child(
                    Input::new(&input)
                        .small()
                        .h(px(FIELD_HEIGHT))
                        .text_token(LABEL)
                        .font_weight(FontWeight::NORMAL)
                        .bg(rgb(palette.input))
                        .border_color(rgb(palette.input_border))
                        .rounded(px(8.))
                        .px_2p5()
                        .cleanable(true),
                )
                .footer(dialog_footer(palette, confirm, Glyph::Folder, None))
                .on_ok(move |_, window, cx| {
                    // A blank name keeps the prompt open with the field in
                    // focus, rather than making a group with nothing on it.
                    let name = input.read(cx).value().trim().to_string();
                    if name.is_empty() {
                        input.update(cx, |input, cx| input.focus(window, cx));
                        return false;
                    }
                    let saved = workspace
                        .update(cx, |workspace, cx| {
                            let id = match prompt {
                                GroupPrompt::New(library) => {
                                    workspace.presets.add_group(library, &name)
                                }
                                GroupPrompt::Rename(id) => {
                                    workspace.presets.rename_group(id, &name);
                                    Some(id)
                                }
                            };
                            if let Some(id) = id {
                                workspace.reveal_group(id);
                            }
                            cx.notify();
                            id
                        })
                        .ok()
                        .flatten();
                    if let Some(id) = saved {
                        on_saved(id, window, cx);
                    }
                    true
                })
        });

        // The dialog takes focus as it opens; the field takes it back once
        // the dialog is there, so typing can start at once.
        cx.defer_in(window, move |_, window, cx| {
            field.update(cx, |field, cx| field.focus(window, cx));
        });
    }

    /// Unfolds the list down to a group, so one just made or renamed is in
    /// view.
    pub(crate) fn reveal_group(&mut self, id: u64) {
        match self.presets.group(id).map(|group| group.library) {
            Some(Library::Sessions) => self.sessions_collapsed = false,
            Some(Library::Commands) => self.commands_collapsed = false,
            None => return,
        }
        self.collapsed_groups.remove(&id);
    }
}
