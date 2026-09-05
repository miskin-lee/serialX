//! The workbench icon set.
//!
//! `gpui-kit` ships a hairline outline set, which is right for generic chrome
//! (a close cross, a chevron) but says nothing about *what kind of thing* a row
//! holds. The glyphs here take the other half of the job, the way VS Code's
//! Material Icon Theme does: a solid, rounded, two-tone silhouette whose colour
//! carries the category. GPUI paints an SVG as an alpha mask tinted with the
//! text colour, so the two tones come from `fill-opacity` inside the file and
//! survive the tint, while the hue is chosen at the call site.

use std::borrow::Cow;

use gpui_kit::assets::Assets as DefaultAssets;
use gpui_kit::component::{Icon, IconNamed};
use gpui_kit::{
    AssetSource, IntoElement, ParentElement, Result, SharedString, Styled, div, px, rgb,
};

use crate::theme::tint;

/// The app's own glyphs, usable anywhere `gpui-component` takes an icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    Bolt,
    Bookmark,
    Cable,
    Folder,
    FolderPlus,
    Hex,
    Pencil,
    Port,
    Refresh,
    Run,
    Send,
    Tag,
    Terminal,
    Trash,
}

impl Glyph {
    const fn asset_path(self) -> &'static str {
        match self {
            Self::Bolt => "icons/ui/bolt.svg",
            Self::Bookmark => "icons/ui/bookmark.svg",
            Self::Cable => "icons/ui/cable.svg",
            Self::Folder => "icons/ui/folder.svg",
            Self::FolderPlus => "icons/ui/folder-plus.svg",
            Self::Hex => "icons/ui/hex.svg",
            Self::Pencil => "icons/ui/pencil.svg",
            Self::Port => "icons/ui/port.svg",
            Self::Refresh => "icons/ui/refresh.svg",
            Self::Run => "icons/ui/run.svg",
            Self::Send => "icons/ui/send.svg",
            Self::Tag => "icons/ui/tag.svg",
            Self::Terminal => "icons/ui/terminal.svg",
            Self::Trash => "icons/ui/trash.svg",
        }
    }
}

impl IconNamed for Glyph {
    fn path(self) -> SharedString {
        self.asset_path().into()
    }
}

/// Every glyph, embedded so the binary needs no asset directory beside it.
const GLYPHS: &[(&str, &[u8])] = &[
    (
        "icons/ui/bolt.svg",
        include_bytes!("../assets/icons/ui/bolt.svg"),
    ),
    (
        "icons/ui/bookmark.svg",
        include_bytes!("../assets/icons/ui/bookmark.svg"),
    ),
    (
        "icons/ui/cable.svg",
        include_bytes!("../assets/icons/ui/cable.svg"),
    ),
    (
        "icons/ui/folder.svg",
        include_bytes!("../assets/icons/ui/folder.svg"),
    ),
    (
        "icons/ui/folder-plus.svg",
        include_bytes!("../assets/icons/ui/folder-plus.svg"),
    ),
    (
        "icons/ui/hex.svg",
        include_bytes!("../assets/icons/ui/hex.svg"),
    ),
    (
        "icons/ui/pencil.svg",
        include_bytes!("../assets/icons/ui/pencil.svg"),
    ),
    (
        "icons/ui/port.svg",
        include_bytes!("../assets/icons/ui/port.svg"),
    ),
    (
        "icons/ui/refresh.svg",
        include_bytes!("../assets/icons/ui/refresh.svg"),
    ),
    (
        "icons/ui/run.svg",
        include_bytes!("../assets/icons/ui/run.svg"),
    ),
    (
        "icons/ui/send.svg",
        include_bytes!("../assets/icons/ui/send.svg"),
    ),
    (
        "icons/ui/tag.svg",
        include_bytes!("../assets/icons/ui/tag.svg"),
    ),
    (
        "icons/ui/terminal.svg",
        include_bytes!("../assets/icons/ui/terminal.svg"),
    ),
    (
        "icons/ui/trash.svg",
        include_bytes!("../assets/icons/ui/trash.svg"),
    ),
];

/// Serves the app's glyphs and falls through to the ones `gpui-kit` ships.
///
/// The component library asks for `icons/*.svg` by name from whatever asset
/// source the application was built with, so replacing that source means taking
/// on the default set as well as our own.
pub(crate) struct WorkbenchAssets;

impl AssetSource for WorkbenchAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = GLYPHS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        DefaultAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names = DefaultAssets.list(path)?;
        names.extend(
            GLYPHS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(names)
    }
}

/// A glyph on a soft disc of its own colour.
///
/// This is the Material Icon Theme move: the tinted plate gives a list row a
/// stable optical anchor, and the hue tells you what kind of row it is before
/// you read a word of it.
pub(crate) fn icon_chip(glyph: Glyph, color: u32, size: f32) -> impl IntoElement {
    let glyph_size = (size * 0.55).round();
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.32))
        .bg(tint(color, 0.14))
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(glyph).size(px(glyph_size)).text_color(rgb(color)))
}

#[cfg(test)]
mod tests {
    use super::{GLYPHS, Glyph, WorkbenchAssets};
    use gpui_kit::AssetSource;
    use gpui_kit::component::IconNamed;

    const ALL: &[Glyph] = &[
        Glyph::Bolt,
        Glyph::Bookmark,
        Glyph::Cable,
        Glyph::Folder,
        Glyph::FolderPlus,
        Glyph::Hex,
        Glyph::Pencil,
        Glyph::Port,
        Glyph::Refresh,
        Glyph::Run,
        Glyph::Send,
        Glyph::Tag,
        Glyph::Terminal,
        Glyph::Trash,
    ];

    /// A glyph whose file is missing renders as nothing at all, with no error,
    /// so the lookup is worth pinning down in a test rather than in the UI.
    #[test]
    fn every_glyph_resolves_to_an_embedded_file() {
        for glyph in ALL {
            let path = glyph.path();
            let loaded = WorkbenchAssets
                .load(&path)
                .unwrap_or_else(|error| panic!("{path} failed to load: {error}"));
            assert!(loaded.is_some(), "{path} is not embedded");
        }
        assert_eq!(ALL.len(), GLYPHS.len(), "a glyph is missing from GLYPHS");
    }

    #[test]
    fn the_default_icon_set_still_resolves() {
        assert!(
            WorkbenchAssets.load("icons/close.svg").unwrap().is_some(),
            "the bundled gpui-kit icons must survive the custom asset source"
        );
    }

    #[test]
    fn embedded_glyphs_are_two_tone_svgs() {
        for (path, bytes) in GLYPHS {
            let source = std::str::from_utf8(bytes).expect("{path} is not UTF-8");
            assert!(
                source.contains("viewBox=\"0 0 24 24\""),
                "{path} is off-grid"
            );
            assert!(
                source.contains("opacity="),
                "{path} has no second tone, so it will read flat next to the rest"
            );
        }
    }
}
