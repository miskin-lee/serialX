//! The workbench design system: colour palette, type scale and font resolution.
//!
//! Two layers live here. [`WorkbenchPalette`] is the raw colour token set the
//! workbench paints with directly, and [`apply_interface_theme`] projects the
//! same tokens onto `gpui-component`'s own `Theme` so the shipped widgets
//! (buttons, dialogs, inputs, menus) match the hand-rolled chrome.

use std::rc::Rc;
use std::sync::OnceLock;

use gpui_kit::component::{Theme, ThemeConfig, ThemeConfigColors, ThemeMode};
use gpui_kit::{App, FontWeight, Rgba, SharedString, Styled, Window, WindowAppearance, px, rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InterfaceTheme {
    Light,
    Dark,
}

/// Every colour the workbench paints with.
///
/// Surfaces are ordered by elevation — `editor` is the canvas the terminal
/// sits on, `panel` the quieter chrome around it, and `surface` the raised
/// cards and toolbars that sit on top of either.
#[derive(Clone, Copy)]
pub(crate) struct WorkbenchPalette {
    pub(crate) title_bar: u32,
    pub(crate) tab_bar: u32,
    pub(crate) editor: u32,
    pub(crate) panel: u32,
    pub(crate) surface: u32,
    /// A list row or launcher card. Raised above `panel` in both themes, since
    /// "lighter" is what reads as nearer on a white page and on a black one.
    pub(crate) card: u32,
    pub(crate) status_bar: u32,
    pub(crate) border: u32,
    /// Hairline used inside a surface, where the structural `border` would
    /// chop the layout into boxes.
    pub(crate) border_subtle: u32,
    pub(crate) foreground: u32,
    pub(crate) strong_foreground: u32,
    pub(crate) muted: u32,
    pub(crate) faint: u32,
    pub(crate) input: u32,
    pub(crate) input_border: u32,
    pub(crate) hover: u32,
    pub(crate) active: u32,
    pub(crate) accent: u32,
    pub(crate) accent_hover: u32,
    pub(crate) accent_active: u32,
    pub(crate) selection: u32,
    pub(crate) success: u32,
    pub(crate) warning: u32,
    pub(crate) danger: u32,
    pub(crate) badge: u32,
    /// Category hues, in the spirit of VS Code's Material Icon Theme: an icon's
    /// colour, not its shape, is what says which kind of thing it labels.
    pub(crate) category_device: u32,
    pub(crate) category_session: u32,
    pub(crate) category_command: u32,
    pub(crate) category_terminal: u32,
    pub(crate) category_signal: u32,
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

    pub(crate) fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
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
            // Paper: a pure white canvas with warm-neutral chrome, so the
            // terminal reads as the page and everything else as its margin.
            Self::Light => WorkbenchPalette {
                title_bar: 0xf4f4f2,
                tab_bar: 0xf7f7f5,
                editor: 0xffffff,
                panel: 0xfbfbf9,
                surface: 0xf7f7f5,
                card: 0xffffff,
                status_bar: 0xf4f4f2,
                border: 0xe4e4e0,
                border_subtle: 0xeeeeea,
                foreground: 0x3b3b42,
                strong_foreground: 0x17171b,
                muted: 0x86868f,
                faint: 0xacacb4,
                input: 0xffffff,
                input_border: 0xdededa,
                hover: 0xf0f0ec,
                active: 0xe7e7e2,
                accent: 0x5b57d8,
                accent_hover: 0x4d49cc,
                accent_active: 0x413ebb,
                selection: 0xd6d5f7,
                success: 0x2f8a5b,
                warning: 0xa8730f,
                danger: 0xcb4b40,
                badge: 0xc6c6cd,
                category_device: 0x2f7fd4,
                category_session: 0xb2740b,
                category_command: 0x3d8a45,
                category_terminal: 0x8250c8,
                category_signal: 0x0f8593,
            },
            // Ink: a near-black canvas lifted by a hint of blue, so the panels
            // separate from the terminal without a single hard grey.
            Self::Dark => WorkbenchPalette {
                title_bar: 0x0f1116,
                tab_bar: 0x0f1116,
                editor: 0x0b0d11,
                panel: 0x0e1015,
                surface: 0x13161c,
                card: 0x171b22,
                status_bar: 0x0f1116,
                border: 0x1f232c,
                border_subtle: 0x171a21,
                foreground: 0xb2b8c4,
                strong_foreground: 0xedf0f5,
                muted: 0x767d8c,
                faint: 0x555c6a,
                input: 0x13161c,
                input_border: 0x252a34,
                hover: 0x181b23,
                active: 0x1f242e,
                accent: 0x8b87ff,
                accent_hover: 0xa3a0ff,
                accent_active: 0x6d68e0,
                selection: 0x2b2b48,
                success: 0x4fc38a,
                warning: 0xe0b070,
                danger: 0xef8a83,
                badge: 0x3d434f,
                category_device: 0x62b0f5,
                category_session: 0xf2b45c,
                category_command: 0x86cf6a,
                category_terminal: 0xc08ae8,
                category_signal: 0x4fc9dc,
            },
        }
    }
}

/// A colour at partial strength, for tinted chips and soft fills.
///
/// GPUI blends this over whatever is painted underneath, so one tint works on
/// every surface elevation instead of needing a pre-blended constant each.
pub(crate) fn tint(color: u32, alpha: f32) -> Rgba {
    let mut color = rgb(color);
    color.a = alpha;
    color
}

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------

/// Families that read well as UI text, best first.
///
/// GPUI resolves a single family name per run, so the "stack" has to be walked
/// against the installed fonts rather than handed to the text system whole.
const UI_FONT_CANDIDATES: &[&str] = &[
    "Inter",
    "Inter Display",
    "SF Pro Text",
    "SF Pro Display",
    "Segoe UI Variable Text",
    "Segoe UI",
    "Helvetica Neue",
    "Noto Sans",
];

const MONO_FONT_CANDIDATES: &[&str] = &[
    "JetBrains Mono",
    "SF Mono",
    "Menlo",
    "Cascadia Mono",
    "Cascadia Code",
    "Consolas",
    "Liberation Mono",
    "DejaVu Sans Mono",
];

const UI_FONT_FALLBACK: &str = ".SystemUIFont";

const MONO_FONT_FALLBACK: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(windows) {
    "Consolas"
} else {
    "monospace"
};

/// The families this machine actually has, resolved once at startup.
#[derive(Clone)]
pub(crate) struct WorkbenchFonts {
    pub(crate) ui: SharedString,
    pub(crate) mono: SharedString,
}

static FONTS: OnceLock<WorkbenchFonts> = OnceLock::new();

/// Picks the best installed family for each role.
///
/// Call once, before the first window opens; later calls are ignored so the
/// whole workbench keeps rendering in one typeface.
pub(crate) fn resolve_fonts(cx: &App) {
    let installed = cx.text_system().all_font_names();
    let pick = |candidates: &[&str], fallback: &str| -> SharedString {
        candidates
            .iter()
            .find(|candidate| {
                installed
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(candidate))
            })
            .map(|name| SharedString::from(name.to_string()))
            .unwrap_or_else(|| SharedString::from(fallback.to_string()))
    };

    let _ = FONTS.set(WorkbenchFonts {
        ui: pick(UI_FONT_CANDIDATES, UI_FONT_FALLBACK),
        mono: pick(MONO_FONT_CANDIDATES, MONO_FONT_FALLBACK),
    });
}

pub(crate) fn fonts() -> &'static WorkbenchFonts {
    FONTS.get_or_init(|| WorkbenchFonts {
        ui: UI_FONT_FALLBACK.into(),
        mono: MONO_FONT_FALLBACK.into(),
    })
}

/// One step of the type scale: size, leading and weight travel together so a
/// call site cannot set an 11px label on 20px leading by accident.
#[derive(Clone, Copy)]
pub(crate) struct TextToken {
    size: f32,
    line_height: f32,
    weight: FontWeight,
}

impl TextToken {
    const fn new(size: f32, line_height: f32, weight: FontWeight) -> Self {
        Self {
            size,
            line_height,
            weight,
        }
    }
}

/// Hero copy on the empty workspace.
pub(crate) const DISPLAY: TextToken = TextToken::new(30., 36., FontWeight::SEMIBOLD);
/// Card and dialog titles.
pub(crate) const TITLE: TextToken = TextToken::new(15., 22., FontWeight::SEMIBOLD);
/// Panel and section headings.
pub(crate) const HEADING: TextToken = TextToken::new(12.5, 18., FontWeight::SEMIBOLD);
/// Default running text.
pub(crate) const BODY: TextToken = TextToken::new(13., 20., FontWeight::NORMAL);
/// Running text that carries the emphasis in its row.
pub(crate) const BODY_STRONG: TextToken = TextToken::new(13., 20., FontWeight::MEDIUM);
/// Control labels, list rows, tabs.
pub(crate) const LABEL: TextToken = TextToken::new(12., 17., FontWeight::MEDIUM);
/// Secondary line under a label.
pub(crate) const CAPTION: TextToken = TextToken::new(11.5, 16., FontWeight::NORMAL);
/// Status bar and other chrome that should recede.
pub(crate) const MICRO: TextToken = TextToken::new(10.5, 14., FontWeight::MEDIUM);
/// Terminal payloads.
pub(crate) const MONO: TextToken = TextToken::new(12.5, 20., FontWeight::NORMAL);
/// Timestamps, byte counts, inline command previews.
pub(crate) const MONO_SMALL: TextToken = TextToken::new(11., 16., FontWeight::NORMAL);
/// The RX / TX / SYS direction tags.
pub(crate) const MONO_TAG: TextToken = TextToken::new(10., 14., FontWeight::BOLD);

/// Applies the type scale to any styled element.
pub(crate) trait Typography: Styled + Sized {
    fn text_token(self, token: TextToken) -> Self {
        self.text_size(px(token.size))
            .line_height(px(token.line_height))
            .font_weight(token.weight)
    }

    fn ui_font(self) -> Self {
        self.font_family(fonts().ui.clone())
    }

    fn mono_font(self) -> Self {
        self.font_family(fonts().mono.clone())
    }

    /// The type scale plus the monospace family, for terminal-shaped text.
    fn mono_token(self, token: TextToken) -> Self {
        self.mono_font().text_token(token)
    }
}

impl<T: Styled> Typography for T {}

// ---------------------------------------------------------------------------
// gpui-component theme
// ---------------------------------------------------------------------------

fn theme_color(value: u32) -> Option<SharedString> {
    Some(format!("#{value:06X}").into())
}

fn workbench_theme_config(theme: InterfaceTheme) -> ThemeConfig {
    let palette = theme.palette();
    let white = theme_color(0xffffff);
    let foreground = theme_color(palette.foreground);
    let background = theme_color(palette.editor);
    let surface = theme_color(palette.surface);
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
    colors.button = surface.clone();
    colors.button_active = theme_color(palette.active);
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
    colors.muted = surface.clone();
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
    colors.secondary = surface.clone();
    colors.secondary_active = theme_color(palette.active);
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
    colors.table_head = panel;
    colors.table_head_foreground = foreground;
    colors.table_hover = hover;
    colors.table_row_border = border.clone();
    colors.title_bar = theme_color(palette.title_bar);
    colors.title_bar_border = border.clone();
    colors.status_bar = theme_color(palette.status_bar);
    colors.status_bar_border = border;
    colors.warning = warning;
    colors.warning_foreground = theme_color(palette.strong_foreground);

    let fonts = fonts();
    ThemeConfig {
        name: format!("serialX {}", theme.name()).into(),
        mode: theme.mode(),
        font_family: Some(fonts.ui.clone()),
        font_size: Some(BODY.size),
        mono_font_family: Some(fonts.mono.clone()),
        mono_font_size: Some(MONO.size),
        radius: Some(8),
        radius_lg: Some(12),
        shadow: Some(false),
        colors,
        ..Default::default()
    }
}

pub(crate) fn apply_interface_theme(theme: InterfaceTheme, window: &mut Window, cx: &mut App) {
    let config = Rc::new(workbench_theme_config(theme));
    Theme::global_mut(cx).apply_config(&config);
    Theme::sync_base(cx);
    cx.set_window_appearance(Some(theme.window_appearance()));
    window.refresh();
}

#[cfg(test)]
mod tests {
    use super::{BODY, DISPLAY, InterfaceTheme, MICRO, MONO, tint};

    #[test]
    fn light_theme_keeps_a_pure_white_canvas() {
        let light = InterfaceTheme::Light.palette();
        assert_eq!(light.editor, 0xffffff);
        assert_eq!(light.accent, 0x5b57d8);
        assert_eq!(InterfaceTheme::Light.name(), "Light");
        assert!(!InterfaceTheme::Light.is_dark());
    }

    #[test]
    fn dark_theme_keeps_a_near_black_canvas() {
        let dark = InterfaceTheme::Dark.palette();
        assert_eq!(dark.editor, 0x0b0d11);
        assert_eq!(dark.accent, 0x8b87ff);
        assert_eq!(InterfaceTheme::Dark.name(), "Dark");
        assert!(InterfaceTheme::Dark.is_dark());
    }

    /// Surfaces have to climb in one direction or cards stop reading as raised.
    #[test]
    fn surfaces_are_ordered_by_elevation() {
        let light = InterfaceTheme::Light.palette();
        assert!(
            light.panel < light.editor,
            "light panel sits under the canvas"
        );
        assert!(
            light.surface < light.panel,
            "light cards sit under the panel"
        );

        let dark = InterfaceTheme::Dark.palette();
        assert!(dark.panel > dark.editor, "dark panel sits above the canvas");
        assert!(dark.surface > dark.panel, "dark cards sit above the panel");
    }

    /// Every category needs its own hue, otherwise the icon colour says nothing.
    #[test]
    fn category_hues_are_distinct() {
        for theme in [InterfaceTheme::Light, InterfaceTheme::Dark] {
            let palette = theme.palette();
            let mut hues = vec![
                palette.category_device,
                palette.category_session,
                palette.category_command,
                palette.category_terminal,
                palette.category_signal,
            ];
            let total = hues.len();
            hues.sort_unstable();
            hues.dedup();
            assert_eq!(hues.len(), total, "{} reuses a category hue", theme.name());
        }
    }

    #[test]
    fn type_scale_climbs_without_ties() {
        let sizes = [MICRO, MONO, BODY, DISPLAY].map(|token| token.size);
        assert!(
            sizes.windows(2).all(|pair| pair[0] < pair[1]),
            "type scale steps must be strictly increasing: {sizes:?}"
        );
    }

    #[test]
    fn tint_keeps_the_hue_and_takes_the_alpha() {
        let tinted = tint(0x5b57d8, 0.12);
        let opaque = super::rgb(0x5b57d8);
        assert_eq!(
            (tinted.r, tinted.g, tinted.b),
            (opaque.r, opaque.g, opaque.b)
        );
        assert_eq!(tinted.a, 0.12);
    }
}
