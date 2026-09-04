use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeConfigColors, ThemeMode};
use gpui_kit::{App, SharedString, Window, WindowAppearance};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InterfaceTheme {
    Light,
    Dark,
}

#[derive(Clone, Copy)]
pub(crate) struct WorkbenchPalette {
    pub(crate) title_bar: u32,
    pub(crate) tab_bar: u32,
    pub(crate) editor: u32,
    pub(crate) panel: u32,
    pub(crate) status_bar: u32,
    pub(crate) border: u32,
    pub(crate) foreground: u32,
    pub(crate) strong_foreground: u32,
    pub(crate) muted: u32,
    pub(crate) input: u32,
    pub(crate) input_border: u32,
    pub(crate) hover: u32,
    pub(crate) accent: u32,
    pub(crate) accent_hover: u32,
    pub(crate) accent_active: u32,
    pub(crate) selection: u32,
    pub(crate) success: u32,
    pub(crate) warning: u32,
    pub(crate) danger: u32,
    pub(crate) badge: u32,
}

impl InterfaceTheme {
    pub(crate) fn from_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    fn mode(self) -> ThemeMode {
        match self {
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
        }
    }

    fn window_appearance(self) -> WindowAppearance {
        match self {
            Self::Light => WindowAppearance::Light,
            Self::Dark => WindowAppearance::Dark,
        }
    }

    pub(crate) fn palette(self) -> WorkbenchPalette {
        match self {
            Self::Light => WorkbenchPalette {
                title_bar: 0xf2f2f3,
                tab_bar: 0xf2f2f3,
                editor: 0xffffff,
                panel: 0xfafafa,
                status_bar: 0xf2f2f3,
                border: 0xdcdcdd,
                foreground: 0x2f3037,
                strong_foreground: 0x18191d,
                muted: 0x6b6d76,
                input: 0xffffff,
                input_border: 0xdcdcdd,
                hover: 0xebebec,
                accent: 0x5c78e2,
                accent_hover: 0x4d69d5,
                accent_active: 0x3f5fc9,
                selection: 0xcbcdf6,
                success: 0x669f59,
                warning: 0xa48819,
                danger: 0xd36151,
                badge: 0xc4c4c9,
            },
            Self::Dark => WorkbenchPalette {
                title_bar: 0x14171c,
                tab_bar: 0x14171c,
                editor: 0x0d1017,
                panel: 0x11141a,
                status_bar: 0x14171c,
                border: 0x232732,
                foreground: 0xaeb4c0,
                strong_foreground: 0xe6e9ef,
                muted: 0x7f8695,
                input: 0x14171c,
                input_border: 0x232732,
                hover: 0x1c202a,
                accent: 0x74ade8,
                accent_hover: 0x85c1ff,
                accent_active: 0x47679e,
                selection: 0x2b3140,
                success: 0xa1c181,
                warning: 0xdec184,
                danger: 0xd07277,
                badge: 0x454b58,
            },
        }
    }
}

fn theme_color(value: u32) -> Option<SharedString> {
    Some(format!("#{value:06X}").into())
}

fn zed_theme_config(theme: InterfaceTheme) -> ThemeConfig {
    let palette = theme.palette();
    let white = theme_color(0xffffff);
    let foreground = theme_color(palette.foreground);
    let background = theme_color(palette.editor);
    let panel = theme_color(palette.panel);
    let hover = theme_color(palette.hover);
    let accent = theme_color(palette.accent);
    let accent_hover = theme_color(palette.accent_hover);
    let accent_active = theme_color(palette.accent_active);
    let border = theme_color(palette.border);
    let success = theme_color(palette.success);
    let warning = theme_color(palette.warning);
    let danger = theme_color(palette.danger);

    let mut colors = ThemeConfigColors::default();
    colors.accent = hover.clone();
    colors.accent_foreground = foreground.clone();
    colors.background = background.clone();
    colors.border = border.clone();
    colors.button = panel.clone();
    colors.button_active = hover.clone();
    colors.button_foreground = foreground.clone();
    colors.button_hover = hover.clone();
    colors.button_primary = accent.clone();
    colors.button_primary_active = accent_active.clone();
    colors.button_primary_foreground = white.clone();
    colors.button_primary_hover = accent_hover.clone();
    colors.caret = accent.clone();
    colors.danger = danger.clone();
    colors.danger_active = danger.clone();
    colors.danger_foreground = white.clone();
    colors.danger_hover = danger;
    colors.foreground = foreground.clone();
    colors.input = theme_color(palette.input_border);
    colors.link = accent.clone();
    colors.link_active = accent_active.clone();
    colors.link_hover = accent_hover.clone();
    colors.list = background.clone();
    colors.list_active = theme_color(palette.selection);
    colors.list_active_border = accent.clone();
    colors.list_even = background.clone();
    colors.list_head = panel.clone();
    colors.list_hover = hover.clone();
    colors.muted = panel.clone();
    colors.muted_foreground = theme_color(palette.muted);
    colors.popover = theme_color(palette.input);
    colors.popover_foreground = foreground.clone();
    colors.primary = accent.clone();
    colors.primary_active = accent_active;
    colors.primary_foreground = white;
    colors.primary_hover = accent_hover;
    colors.ring = accent;
    colors.scrollbar = background.clone();
    colors.scrollbar_thumb = theme_color(palette.badge);
    colors.scrollbar_thumb_hover = theme_color(palette.muted);
    colors.secondary = panel.clone();
    colors.secondary_active = hover.clone();
    colors.secondary_foreground = foreground.clone();
    colors.secondary_hover = hover.clone();
    colors.selection = theme_color(palette.selection);
    colors.sidebar = panel.clone();
    colors.sidebar_accent = hover.clone();
    colors.sidebar_accent_foreground = foreground.clone();
    colors.sidebar_border = border.clone();
    colors.sidebar_foreground = foreground.clone();
    colors.success = success;
    colors.success_foreground = theme_color(palette.strong_foreground);
    colors.tab = theme_color(palette.tab_bar);
    colors.tab_active = background.clone();
    colors.tab_active_foreground = theme_color(palette.strong_foreground);
    colors.tab_bar = theme_color(palette.tab_bar);
    colors.tab_foreground = theme_color(palette.muted);
    colors.table = background;
    colors.table_active = hover.clone();
    colors.table_active_border = theme_color(palette.accent);
    colors.table_even = theme_color(palette.editor);
    colors.table_head = panel.clone();
    colors.table_head_foreground = foreground;
    colors.table_hover = hover;
    colors.table_row_border = border.clone();
    colors.title_bar = theme_color(palette.title_bar);
    colors.title_bar_border = border.clone();
    colors.status_bar = theme_color(palette.status_bar);
    colors.status_bar_border = border;
    colors.warning = warning;
    colors.warning_foreground = theme_color(palette.strong_foreground);

    ThemeConfig {
        name: format!("serialX {}", theme.name()).into(),
        mode: theme.mode(),
        font_size: Some(13.),
        mono_font_size: Some(12.),
        radius: Some(6),
        radius_lg: Some(8),
        shadow: Some(false),
        colors,
        ..Default::default()
    }
}

pub(crate) fn apply_interface_theme(theme: InterfaceTheme, window: &mut Window, cx: &mut App) {
    let config = Rc::new(zed_theme_config(theme));
    Theme::global_mut(cx).apply_config(&config);
    Theme::sync_base(cx);
    cx.set_window_appearance(Some(theme.window_appearance()));
    window.refresh();
}

#[cfg(test)]
mod tests {
    use super::InterfaceTheme;

    #[test]
    fn light_theme_is_pure_white_with_zed_accents() {
        let light = InterfaceTheme::Light.palette();
        assert_eq!(light.editor, 0xffffff);
        assert_eq!(light.tab_bar, 0xf2f2f3);
        assert_eq!(light.border, 0xdcdcdd);
        assert_eq!(light.accent, 0x5c78e2);
        assert_eq!(InterfaceTheme::Light.name(), "Light");
    }

    #[test]
    fn dark_theme_is_near_black_with_zed_accents() {
        let dark = InterfaceTheme::Dark.palette();
        assert_eq!(dark.editor, 0x0d1017);
        assert_eq!(dark.tab_bar, 0x14171c);
        assert_eq!(dark.border, 0x232732);
        assert_eq!(dark.accent, 0x74ade8);
        assert_eq!(InterfaceTheme::Dark.name(), "Dark");
    }
}
