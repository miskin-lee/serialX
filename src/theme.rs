//! The workbench design system: colour palette, type scale and font resolution.
//!
//! Two layers live here. [`WorkbenchPalette`] is the raw colour token set the
//! workbench paints with directly, and [`apply_interface_theme`] projects the
//! same tokens onto `gpui-component`'s own `Theme` so the shipped widgets
//! (buttons, dialogs, inputs, menus) match the hand-rolled chrome.

use std::rc::Rc;
use std::sync::OnceLock;

use gpui_kit::component::{Theme, ThemeConfig, ThemeConfigColors, ThemeMode};
use gpui_kit::{
    App, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, Rgba, SharedString, Styled,
    Window, WindowAppearance, px, rgb,
};
use serde::{Deserialize, Serialize};

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
    pub(crate) category_signal: u32,
    /// The wordmark's three inks, taken from `docs/logo.svg` so the identity
    /// in the app and the one at the top of the README are the same drawing.
    /// Only the body switches with the canvas; the orange `s` and the green
    /// `X` are the logo's own colours in both themes.
    pub(crate) wordmark_lead: u32,
    pub(crate) wordmark_body: u32,
    pub(crate) wordmark_tail: u32,
    /// The hues a session can be tagged with, in [`TagColor::HUES`] order.
    /// Ink-strength on paper and pastel on ink, like the category hues, so a
    /// tag paints the same fills and glyphs in both themes.
    pub(crate) tags: [u32; TAG_HUE_COUNT],
}

/// How many colours a session can be tagged with.
pub(crate) const TAG_HUE_COUNT: usize = 24;

/// The colour a session is tagged with. Every session has one.
///
/// The tag is a label, not a state: the dot on a tab says whether the port
/// is open, and the tag colours the plate under it, so the sessions to three
/// boards on the same bench can be told apart without reading their names —
/// the way iTerm2, Windows Terminal and Firefox's containers colour a tab.
/// The colours come in two dozens, as Google Calendar's grid does: a bright
/// dozen around the wheel, then a deep dozen — crimson, rust, cocoa, mustard,
/// olive, emerald and on — that sit between the bright hues and a step darker
/// and more saturated, so no two cells of the grid read as the same colour.
///
/// A new session is offered the first colour no open tab is using; a session
/// saved before there were tags loads as the neutral grey.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TagColor {
    // The bright dozen, around the wheel.
    Red,
    Orange,
    Amber,
    Yellow,
    Lime,
    Green,
    Teal,
    Sky,
    Blue,
    Purple,
    Pink,
    #[default]
    Gray,
    // The deep dozen, around it again.
    Crimson,
    Rust,
    Cocoa,
    Mustard,
    Olive,
    Emerald,
    Cyan,
    Cobalt,
    Navy,
    Grape,
    Magenta,
    Graphite,
}

impl TagColor {
    /// The colours in the order the picker lays them out: the bright dozen
    /// from red to pink and a cool grey, then the deep dozen from crimson to
    /// magenta and graphite.
    pub(crate) const HUES: [Self; TAG_HUE_COUNT] = [
        Self::Red,
        Self::Orange,
        Self::Amber,
        Self::Yellow,
        Self::Lime,
        Self::Green,
        Self::Teal,
        Self::Sky,
        Self::Blue,
        Self::Purple,
        Self::Pink,
        Self::Gray,
        Self::Crimson,
        Self::Rust,
        Self::Cocoa,
        Self::Mustard,
        Self::Olive,
        Self::Emerald,
        Self::Cyan,
        Self::Cobalt,
        Self::Navy,
        Self::Grape,
        Self::Magenta,
        Self::Graphite,
    ];

    /// The tag's name, for the swatch tooltip.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Amber => "Amber",
            Self::Yellow => "Yellow",
            Self::Lime => "Lime",
            Self::Green => "Green",
            Self::Teal => "Teal",
            Self::Sky => "Sky",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Pink => "Pink",
            Self::Gray => "Gray",
            Self::Crimson => "Crimson",
            Self::Rust => "Rust",
            Self::Cocoa => "Cocoa",
            Self::Mustard => "Mustard",
            Self::Olive => "Olive",
            Self::Emerald => "Emerald",
            Self::Cyan => "Cyan",
            Self::Cobalt => "Cobalt",
            Self::Navy => "Navy",
            Self::Grape => "Grape",
            Self::Magenta => "Magenta",
            Self::Graphite => "Graphite",
        }
    }

    /// Where the tag's colour sits in [`WorkbenchPalette::tags`].
    fn hue_index(self) -> usize {
        Self::HUES
            .iter()
            .position(|hue| *hue == self)
            .expect("every tag is listed in HUES")
    }
}

impl WorkbenchPalette {
    /// The colour a tag paints with in this theme.
    pub(crate) fn tag(self, color: TagColor) -> u32 {
        self.tags[color.hue_index()]
    }
}

/// The workbench opens dark, whatever the system is set to; the terminal is
/// where the eyes stay, and a near-black page is the one that holds them.
impl Default for InterfaceTheme {
    fn default() -> Self {
        Self::Dark
    }
}

impl InterfaceTheme {
    /// The other theme, for the one-key switch.
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
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

    /// The colours the terminal draws with in this theme.
    pub(crate) fn terminal_palette(self) -> TerminalPalette {
        match self {
            Self::Light => TerminalPalette::LIGHT,
            Self::Dark => TerminalPalette::DARK,
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
                category_signal: 0x0f8593,
                wordmark_lead: 0xff754c,
                wordmark_body: 0x4d75ff,
                wordmark_tail: 0x35b873,
                tags: [
                    0xdc3d3d, 0xe5661d, 0xd68a08, 0xbfa10a, 0x6ea41a, 0x28994f, 0x149487, 0x0f88c0,
                    0x2f6fe0, 0x8a3ee0, 0xd6338f, 0x737b8a, 0xb01a41, 0xa8431a, 0x6f4a33, 0x967810,
                    0x62701a, 0x116a40, 0x076a7d, 0x194fa8, 0x2a3c8f, 0x5a2287, 0x8a1c75, 0x4a505b,
                ],
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
                category_signal: 0x4fc9dc,
                wordmark_lead: 0xff754c,
                wordmark_body: 0xf4f3ef,
                wordmark_tail: 0x35b873,
                tags: [
                    0xf27171, 0xfb9a4a, 0xf5b93a, 0xecd23a, 0xa9e04a, 0x5ad487, 0x3fd1bd, 0x4ac2f5,
                    0x6b9cf8, 0xb98af5, 0xf07ac2, 0x9aa3b2, 0xdb3a5e, 0xd0602a, 0xa06c48, 0xc9a21e,
                    0x8fa321, 0x22a866, 0x13a5bf, 0x2f7fe6, 0x4157c9, 0x9143c9, 0xcc3ab0, 0x646c7a,
                ],
            },
        }
    }
}

/// A colour at partial strength, for tinted chips and soft fills.
///
/// GPUI blends this over whatever is painted underneath, so one tint works on
/// every surface elevation instead of needing a pre-blended constant each.
/// What the terminal paints with: the page it sits on, its default ink,
/// its cursor, and the sixteen ANSI colours a device names by number — the
/// eight plain ones, then their bright twins. The plain red, green and
/// yellow are the workbench's own danger, success and warning, so a
/// device's colours and the chrome's agree.
#[derive(Clone, Copy)]
pub(crate) struct TerminalPalette {
    /// Which theme this is, for the colours that are picked per role rather
    /// than named by the device: see [`crate::highlight::Role::style`].
    pub(crate) theme: InterfaceTheme,
    pub(crate) background: u32,
    pub(crate) foreground: u32,
    pub(crate) cursor: u32,
    pub(crate) ansi: [u32; 16],
}

impl TerminalPalette {
    pub(crate) const DARK: Self = Self {
        theme: InterfaceTheme::Dark,
        background: 0x0b0d11,
        foreground: 0xb2b8c4,
        cursor: 0x8b87ff,
        ansi: [
            0x1b1f27, 0xef8a83, 0x4fc38a, 0xe0b070, 0x7aa2f7, 0xc792ea, 0x5fd1d8, 0xb2b8c4, //
            0x767d8c, 0xff9f98, 0x6ee0a2, 0xf0c88a, 0x99b8ff, 0xd8aaff, 0x7fe6ec, 0xedf0f5,
        ],
    };

    /// On white, "bright" cannot mean lighter or it would vanish; the bright
    /// eight are the plain ones lifted a little instead.
    pub(crate) const LIGHT: Self = Self {
        theme: InterfaceTheme::Light,
        background: 0xffffff,
        foreground: 0x3b3b42,
        cursor: 0x5b57d8,
        ansi: [
            0x17171b, 0xcb4b40, 0x2f8a5b, 0xa8730f, 0x2f6fd6, 0x8e44ad, 0x1a8a96, 0x3b3b42, //
            0x86868f, 0xe0665b, 0x3aa66f, 0xc08a2a, 0x4f89e8, 0xa85ec8, 0x2aa3b1, 0x5c5c66,
        ],
    };
}

pub(crate) fn tint(color: u32, alpha: f32) -> Rgba {
    let mut color = rgb(color);
    color.a = alpha;
    color
}

/// The colour `amount` of the way from `from` to `to`, per channel.
///
/// For the places a translucent tint will not do: a gradient stop has to be
/// opaque to paint the same on every frame, and an outline drawn over a
/// gradient has to be a single colour or it bands with it.
pub(crate) fn mix(from: u32, to: u32, amount: f32) -> u32 {
    let amount = amount.clamp(0., 1.);
    let channel = |shift: u32| {
        let a = ((from >> shift) & 0xff) as f32;
        let b = ((to >> shift) & 0xff) as f32;
        ((a + (b - a) * amount).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------

/// A font stack: the families to try, best first, and the family to fall back
/// on when this machine has none of them.
///
/// GPUI resolves one family name per text run, so a CSS-style stack has to be
/// walked against the installed fonts here rather than handed to the text
/// system whole.
struct FontStack {
    candidates: &'static [&'static str],
    fallback: &'static str,
}

/// What one platform reads its interface and its terminal in.
///
/// The two monospace roles are VS Code's own split: `mono` is the editor font,
/// which the terminal and everything else printed as device output uses, and
/// `ui_mono` is the `--monaco-monospace-font` the chrome sets inline code in.
/// They differ on macOS, where the editor opens in Menlo but the chrome reads
/// in SF Mono.
struct PlatformFonts {
    ui: FontStack,
    mono: FontStack,
    ui_mono: FontStack,
    cjk: CjkFallbacks,
}

/// The CJK families VS Code appends to its stacks, per language.
///
/// VS Code keys these off the language its own interface is in — the
/// `:lang(zh-Hans)` and friends variants of every rule above. serialX has one
/// English interface, so CJK never arrives as interface text here; it arrives
/// as device output, in whichever language the device speaks. The user's own
/// language is the best guess at which Han variant they want to read, so it
/// leads, and the rest follow it for coverage rather than being dropped.
struct CjkFallbacks {
    simplified_chinese: &'static [&'static str],
    traditional_chinese: &'static [&'static str],
    japanese: &'static [&'static str],
    korean: &'static [&'static str],
}

// serialX borrows VS Code's layout, so it borrows how VS Code picks type as
// well: no bundled family, one hard-coded stack per platform, and the platform
// font first. The three below are the stacks VS Code 1.136 asks for — the UI
// ones from the `.monaco-workbench.mac | .windows | .linux` rules in
// `workbench.desktop.main.css`, the mono ones from the per-platform
// `EDITOR_FONT_DEFAULTS.fontFamily`, which is also what its integrated terminal
// inherits. All three are compiled into every build, so a name can only be
// wrong, never unbuildable on the platform nobody is compiling for today.

/// `-apple-system, BlinkMacSystemFont, sans-serif` over
/// `Menlo, Monaco, 'Courier New', monospace`.
///
/// Both UI names are the browser's alias for the system font, so that stack has
/// nothing to walk: GPUI spells the same font `.SystemUIFont` and resolves it
/// to `.AppleSystemUIFont` here.
const MAC_FONTS: PlatformFonts = PlatformFonts {
    ui: FontStack {
        candidates: &[],
        fallback: ".SystemUIFont",
    },
    mono: FontStack {
        candidates: &["Menlo", "Monaco", "Courier New"],
        fallback: "Menlo",
    },
    // `"SF Mono", Monaco, Menlo, Courier, monospace`.
    ui_mono: FontStack {
        candidates: &["SF Mono", "Monaco", "Menlo", "Courier"],
        fallback: "Menlo",
    },
    cjk: CjkFallbacks {
        simplified_chinese: &["PingFang SC", "Hiragino Sans GB"],
        traditional_chinese: &["PingFang TC"],
        japanese: &["Hiragino Kaku Gothic Pro"],
        korean: &["Apple SD Gothic Neo", "Nanum Gothic", "AppleGothic"],
    },
};

/// `Segoe WPC, Segoe UI, sans-serif` over `Consolas, 'Courier New', monospace`.
///
/// `Segoe WPC` is VS Code's own name for the shell font; where neither Segoe is
/// installed, GPUI's `.SystemUIFont` lands where the CSS generic would.
const WINDOWS_FONTS: PlatformFonts = PlatformFonts {
    ui: FontStack {
        candidates: &["Segoe WPC", "Segoe UI"],
        fallback: ".SystemUIFont",
    },
    mono: FontStack {
        candidates: &["Consolas", "Courier New"],
        fallback: "Consolas",
    },
    // `Consolas, "Courier New", monospace`: the only platform where VS Code
    // reads its chrome and its editor in the same face.
    ui_mono: FontStack {
        candidates: &["Consolas", "Courier New"],
        fallback: "Consolas",
    },
    cjk: CjkFallbacks {
        simplified_chinese: &["Microsoft YaHei"],
        traditional_chinese: &["Microsoft Jhenghei"],
        japanese: &["Yu Gothic UI", "Meiryo UI"],
        korean: &["Malgun Gothic", "Dotom"],
    },
};

/// `system-ui, Ubuntu, Droid Sans, sans-serif` over
/// `'Droid Sans Mono', monospace`.
///
/// `system-ui` cannot be forwarded as-is, because GPUI resolves
/// `.SystemUIFont` on Linux to the font Zed bundles rather than to the
/// desktop's own. The desktop faces fontconfig would land on are spelled out
/// after VS Code's two names instead, and the monospace generic is filled in
/// with the families VS Code itself names in its Linux monospace stack.
const LINUX_FONTS: PlatformFonts = PlatformFonts {
    ui: FontStack {
        candidates: &[
            "Ubuntu",
            "Droid Sans",
            "Cantarell",
            "Noto Sans",
            "DejaVu Sans",
            "Liberation Sans",
        ],
        fallback: "sans-serif",
    },
    mono: FontStack {
        candidates: &[
            "Droid Sans Mono",
            "Ubuntu Mono",
            "Liberation Mono",
            "DejaVu Sans Mono",
            "Noto Sans Mono",
        ],
        fallback: "monospace",
    },
    // `"Ubuntu Mono", "Liberation Mono", "DejaVu Sans Mono", "Courier New",
    // monospace`.
    ui_mono: FontStack {
        candidates: &[
            "Ubuntu Mono",
            "Liberation Mono",
            "DejaVu Sans Mono",
            "Courier New",
        ],
        fallback: "monospace",
    },
    cjk: CjkFallbacks {
        simplified_chinese: &[
            "Source Han Sans SC",
            "Source Han Sans CN",
            "Source Han Sans",
        ],
        traditional_chinese: &[
            "Source Han Sans TC",
            "Source Han Sans TW",
            "Source Han Sans",
        ],
        japanese: &["Source Han Sans J", "Source Han Sans JP", "Source Han Sans"],
        korean: &[
            "Source Han Sans K",
            "Source Han Sans JR",
            "Source Han Sans",
            "UnDotum",
            "FBaekmuk Gulim",
        ],
    },
};

/// The stacks for the platform this binary was built for.
const fn platform_fonts() -> PlatformFonts {
    if cfg!(target_os = "macos") {
        MAC_FONTS
    } else if cfg!(target_os = "windows") {
        WINDOWS_FONTS
    } else {
        LINUX_FONTS
    }
}

/// The families this machine actually has, resolved once at startup.
#[derive(Clone)]
pub(crate) struct WorkbenchFonts {
    pub(crate) ui: SharedString,
    /// The terminal, and anything else printing what a device said.
    pub(crate) mono: SharedString,
    /// Monospace inside the chrome: config summaries, saved commands.
    pub(crate) ui_mono: SharedString,
    /// Consulted for the glyphs the family above has none of, which for a
    /// serial monitor means whatever the device on the other end sends.
    pub(crate) cjk: Option<FontFallbacks>,
}

static FONTS: OnceLock<WorkbenchFonts> = OnceLock::new();

/// The first family in the stack this machine has installed.
fn pick_family(stack: &FontStack, installed: &[String]) -> SharedString {
    stack
        .candidates
        .iter()
        .find(|candidate| {
            installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(candidate))
        })
        .copied()
        .unwrap_or(stack.fallback)
        .to_string()
        .into()
}

/// Which of the CJK lists the reader most likely wants to be read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CjkLanguage {
    SimplifiedChinese,
    TraditionalChinese,
    Japanese,
    Korean,
}

impl CjkLanguage {
    /// Reads a language out of a tag as either a locale or an environment
    /// variable spells it: `zh-Hant-TW`, `zh_TW.UTF-8`, `ja`.
    fn from_tag(tag: &str) -> Option<Self> {
        let tag = tag.to_ascii_lowercase().replace('_', "-");
        let mut parts = tag.split(['-', '.', '@']).filter(|part| !part.is_empty());
        let language = parts.next()?;
        let rest = parts.collect::<Vec<_>>();
        let tagged = |name: &str| rest.contains(&name);

        match language {
            // Simplified is the default for an unqualified `zh`: it is what
            // the majority of `zh` systems and every `zh-CN` one reads.
            "zh" => {
                if tagged("hant") || tagged("tw") || tagged("hk") || tagged("mo") {
                    Some(Self::TraditionalChinese)
                } else {
                    Some(Self::SimplifiedChinese)
                }
            }
            "ja" => Some(Self::Japanese),
            "ko" => Some(Self::Korean),
            _ => None,
        }
    }
}

/// The platform's CJK families, the reader's own language first.
fn cjk_fallbacks(cjk: &CjkFallbacks, language: Option<CjkLanguage>) -> Vec<String> {
    let lists = [
        (CjkLanguage::SimplifiedChinese, cjk.simplified_chinese),
        (CjkLanguage::TraditionalChinese, cjk.traditional_chinese),
        (CjkLanguage::Japanese, cjk.japanese),
        (CjkLanguage::Korean, cjk.korean),
    ];

    let mut families: Vec<String> = Vec::new();
    let mut push = |list: &[&str]| {
        for family in list {
            // Linux names the same Source Han Sans in several lists, and a
            // family repeated in a cascade is just a slower cascade.
            if !families.iter().any(|seen| seen == family) {
                families.push((*family).to_string());
            }
        }
    };

    if let Some(preferred) = language.and_then(|language| {
        lists
            .iter()
            .find_map(|(candidate, list)| (*candidate == language).then_some(*list))
    }) {
        push(preferred);
    }
    for (_, list) in lists {
        push(list);
    }

    families
}

/// Picks the best installed family for each role.
///
/// Call once, before the first window opens; later calls are ignored so the
/// whole workbench keeps rendering in one typeface.
pub(crate) fn resolve_fonts(cx: &App) {
    let installed = cx.text_system().all_font_names();
    let platform = platform_fonts();
    let language = sys_locale::get_locale().and_then(|tag| CjkLanguage::from_tag(&tag));

    let _ = FONTS.set(WorkbenchFonts {
        ui: pick_family(&platform.ui, &installed),
        mono: pick_family(&platform.mono, &installed),
        ui_mono: pick_family(&platform.ui_mono, &installed),
        cjk: Some(FontFallbacks::from_fonts(cjk_fallbacks(
            &platform.cjk,
            language,
        ))),
    });
}

pub(crate) fn fonts() -> &'static WorkbenchFonts {
    FONTS.get_or_init(|| {
        let platform = platform_fonts();
        WorkbenchFonts {
            ui: platform.ui.fallback.into(),
            mono: platform.mono.fallback.into(),
            ui_mono: platform.ui_mono.fallback.into(),
            cjk: None,
        }
    })
}

/// One step of the type scale: size, leading and weight travel together so a
/// call site cannot set an 11px label on 20px leading by accident.
#[derive(Clone, Copy)]
pub(crate) struct TextToken {
    pub(crate) size: f32,
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

/// The `serialX` wordmark on the empty workspace. Heavier than the rest of the
/// scale, and a step past the 750 the logo is drawn at: set at a fraction of
/// the README's size, the same numeric weight would read thinner than the mark
/// standing next to it.
pub(crate) const WORDMARK: TextToken = TextToken::new(34., 42., FontWeight(800.));
/// Dialog titles: one step above the body, so a sheet opens with a headline
/// rather than a bold sentence.
pub(crate) const TITLE: TextToken = TextToken::new(15., 20., FontWeight::SEMIBOLD);
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
/// Section labels set in small caps: `DEVICE`, `BAUD RATE`. The tracking is
/// carried by the letters themselves, since GPUI has no letter-spacing.
pub(crate) const EYEBROW: TextToken = TextToken::new(10.5, 14., FontWeight::SEMIBOLD);
/// Terminal payloads.
pub(crate) const MONO: TextToken = TextToken::new(12.5, 20., FontWeight::NORMAL);
/// Timestamps, byte counts, inline command previews.
pub(crate) const MONO_SMALL: TextToken = TextToken::new(11., 16., FontWeight::NORMAL);

/// Applies the type scale to any styled element.
pub(crate) trait Typography: Styled + Sized {
    fn text_token(self, token: TextToken) -> Self {
        self.text_size(px(token.size))
            .line_height(px(token.line_height))
            .font_weight(token.weight)
    }

    /// A family plus the CJK cascade behind it.
    ///
    /// Fallbacks cannot be set on their own, only as part of a whole `Font`,
    /// which resets the weight — so this has to run before [`text_token`],
    /// exactly the order the helpers below use.
    ///
    /// [`text_token`]: Typography::text_token
    fn family_with_fallbacks(self, family: SharedString) -> Self {
        self.font(Font {
            family,
            features: FontFeatures::default(),
            fallbacks: fonts().cjk.clone(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        })
    }

    fn ui_font(self) -> Self {
        self.family_with_fallbacks(fonts().ui.clone())
    }


    /// The chrome family: monospace that labels the interface rather than
    /// carrying traffic — a port's parameters, a saved command.
    fn ui_mono_font(self) -> Self {
        self.family_with_fallbacks(fonts().ui_mono.clone())
    }


    /// The type scale plus the chrome's monospace family.
    fn ui_mono_token(self, token: TextToken) -> Self {
        self.ui_mono_font().text_token(token)
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
        mono_font_family: Some(fonts.ui_mono.clone()),
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
    use super::{
        BODY, CjkLanguage, FontStack, InterfaceTheme, LINUX_FONTS, MAC_FONTS, MICRO, MONO,
        TagColor, WINDOWS_FONTS, WORDMARK, cjk_fallbacks, mix, pick_family, tint,
    };

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn light_theme_keeps_a_pure_white_canvas() {
        let light = InterfaceTheme::Light.palette();
        assert_eq!(light.editor, 0xffffff);
        assert_eq!(light.accent, 0x5b57d8);
        assert_eq!(InterfaceTheme::Light.name(), "Light");
    }

    #[test]
    fn dark_theme_keeps_a_near_black_canvas() {
        let dark = InterfaceTheme::Dark.palette();
        assert_eq!(dark.editor, 0x0b0d11);
        assert_eq!(dark.accent, 0x8b87ff);
        assert_eq!(InterfaceTheme::Dark.name(), "Dark");
        assert_eq!(
            InterfaceTheme::default().name(),
            "Dark",
            "the workbench opens dark"
        );
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
                palette.category_signal,
            ];
            let total = hues.len();
            hues.sort_unstable();
            hues.dedup();
            assert_eq!(hues.len(), total, "{} reuses a category hue", theme.name());
        }
    }

    /// Every tag needs its own colour, none of them the accent, or a tagged
    /// tab looks selected rather than labelled.
    #[test]
    fn tag_hues_are_distinct_and_none_is_the_accent() {
        for theme in [InterfaceTheme::Light, InterfaceTheme::Dark] {
            let palette = theme.palette();
            let mut hues = TagColor::HUES
                .iter()
                .map(|&tag| palette.tag(tag))
                .collect::<Vec<_>>();
            assert!(
                !hues.contains(&palette.accent),
                "{} paints a tag in the accent",
                theme.name()
            );
            let total = hues.len();
            hues.sort_unstable();
            hues.dedup();
            assert_eq!(hues.len(), total, "{} reuses a tag hue", theme.name());
        }
    }

    /// The deep row has to be darker than the bright row in both themes, in
    /// every column, or the two rows of the picker read as one palette
    /// printed twice. On paper the bright row is already ink, so the gap
    /// there is narrower than on the dark canvas.
    #[test]
    fn the_deep_dozen_is_darker_than_the_bright_dozen() {
        fn luminance(color: u32) -> f32 {
            let channel = |shift: u32| ((color >> shift) & 0xff) as f32;
            0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
        }
        for theme in [InterfaceTheme::Light, InterfaceTheme::Dark] {
            let palette = theme.palette();
            let (bright, deep) = TagColor::HUES.split_at(TagColor::HUES.len() / 2);
            let mut gaps = bright
                .iter()
                .zip(deep)
                .map(|(&bright, &deep)| {
                    luminance(palette.tag(bright)) - luminance(palette.tag(deep))
                })
                .collect::<Vec<_>>();
            let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
            gaps.sort_by(f32::total_cmp);
            assert!(
                mean > 30. && gaps[0] > 20.,
                "{}: the deep row is not clearly darker (mean gap {mean:.0}, smallest {:.0})",
                theme.name(),
                gaps[0]
            );
        }
    }

    /// The tag is stored by name, so a workspace file reads back by hand and
    /// a session saved before tags loads as the neutral grey.
    #[test]
    fn tags_are_stored_by_name() {
        assert_eq!(serde_json::to_string(&TagColor::Sky).unwrap(), "\"sky\"");
        assert_eq!(
            serde_json::from_str::<TagColor>("\"purple\"").unwrap(),
            TagColor::Purple
        );
        assert_eq!(TagColor::default(), TagColor::Gray);
        assert_eq!(
            serde_json::from_str::<TagColor>("\"magenta\"")
                .unwrap()
                .name(),
            "Magenta"
        );
    }

    #[test]
    fn type_scale_climbs_without_ties() {
        let sizes = [MICRO, MONO, BODY, WORDMARK].map(|token| token.size);
        assert!(
            sizes.windows(2).all(|pair| pair[0] < pair[1]),
            "type scale steps must be strictly increasing: {sizes:?}"
        );
    }

    /// A stack is a preference order, not a list of equals: the second name is
    /// only reached when the machine is missing the first.
    #[test]
    fn a_font_stack_takes_the_best_family_the_machine_has() {
        let stack = FontStack {
            candidates: &["Consolas", "Courier New"],
            fallback: "monospace",
        };

        assert_eq!(
            pick_family(&stack, &installed(&["Courier New", "consolas"])),
            "Consolas",
            "matching is case-insensitive, and the first candidate still wins"
        );
        assert_eq!(
            pick_family(&stack, &installed(&["Courier New"])),
            "Courier New"
        );
        assert_eq!(pick_family(&stack, &installed(&["Arial"])), "monospace");
    }

    /// A name that is blank, or a fallback that is, would leave the workbench
    /// with no font at all — and only on the platform nobody is building for.
    #[test]
    fn every_platform_stack_names_something() {
        for platform in [MAC_FONTS, WINDOWS_FONTS, LINUX_FONTS] {
            for stack in [&platform.ui, &platform.mono, &platform.ui_mono] {
                assert!(!stack.fallback.trim().is_empty());
                assert!(stack.candidates.iter().all(|name| !name.trim().is_empty()));
            }

            let cascade = cjk_fallbacks(&platform.cjk, None);
            assert!(!cascade.is_empty(), "every platform names CJK families");
            assert!(cascade.iter().all(|name| !name.trim().is_empty()));
        }
    }

    /// Locales reach us in several spellings, and the one that matters most —
    /// which Han variant to draw — hides in a subtag rather than the language.
    #[test]
    fn a_language_tag_resolves_to_the_script_it_is_read_in() {
        use CjkLanguage::*;

        for (tag, expected) in [
            ("zh", Some(SimplifiedChinese)),
            ("zh-CN", Some(SimplifiedChinese)),
            ("zh_CN.UTF-8", Some(SimplifiedChinese)),
            ("zh-Hans-CN", Some(SimplifiedChinese)),
            ("zh-Hant", Some(TraditionalChinese)),
            ("zh_TW.UTF-8", Some(TraditionalChinese)),
            ("zh-HK", Some(TraditionalChinese)),
            ("ja-JP", Some(Japanese)),
            ("ko_KR.UTF-8", Some(Korean)),
            ("en-US", None),
            ("", None),
        ] {
            assert_eq!(CjkLanguage::from_tag(tag), expected, "{tag}");
        }
    }

    /// The reader's own language leads, but nothing is dropped: a device that
    /// speaks a different one still has a font to be printed in.
    #[test]
    fn the_cjk_cascade_leads_with_the_readers_language() {
        let cascade = cjk_fallbacks(&MAC_FONTS.cjk, Some(CjkLanguage::Japanese));
        assert_eq!(
            cascade.first().map(String::as_str),
            Some("Hiragino Kaku Gothic Pro")
        );
        assert!(cascade.iter().any(|family| family == "PingFang SC"));

        assert_eq!(
            cjk_fallbacks(&MAC_FONTS.cjk, None)
                .first()
                .map(String::as_str),
            Some("PingFang SC"),
            "with no language to go on, the cascade keeps its written order"
        );
    }

    /// Linux names one Source Han Sans in all four lists, and a family
    /// repeated in a cascade is just a slower cascade.
    #[test]
    fn the_cjk_cascade_names_each_family_once() {
        let cascade = cjk_fallbacks(&LINUX_FONTS.cjk, Some(CjkLanguage::TraditionalChinese));
        let mut unique = cascade.clone();
        unique.sort();
        unique.dedup();

        assert_eq!(cascade.len(), unique.len(), "{cascade:?}");
        assert_eq!(
            cascade.first().map(String::as_str),
            Some("Source Han Sans TC")
        );
    }

    /// The macOS UI stack is shaped like this: `-apple-system` is the system
    /// font itself, so there is nothing to walk and the fallback is the answer.
    #[test]
    fn a_stack_with_nothing_to_choose_takes_its_fallback() {
        let stack = FontStack {
            candidates: &[],
            fallback: ".SystemUIFont",
        };

        assert_eq!(
            pick_family(&stack, &installed(&["Helvetica Neue"])),
            ".SystemUIFont"
        );
    }

    /// The `fill="#RRGGBB"` inks of a logo's `<text>` element, in file order:
    /// the body first, then the `s` and the `X` overrides.
    fn logo_wordmark_inks(svg: &str) -> Vec<u32> {
        let wordmark = svg
            .split_once("<text")
            .expect("the logo carries a wordmark")
            .1;
        wordmark
            .split("fill=\"#")
            .skip(1)
            .map(|ink| u32::from_str_radix(&ink[..6], 16).expect("a six-digit hex ink"))
            .collect()
    }

    /// Nothing but this test keeps the wordmark in the app and the one in the
    /// README from drifting apart, since they are drawn by different engines.
    #[test]
    fn the_wordmark_is_inked_like_the_logo() {
        let light = InterfaceTheme::Light.palette();
        assert_eq!(
            logo_wordmark_inks(include_str!("../docs/logo.svg")),
            vec![
                light.wordmark_body,
                light.wordmark_lead,
                light.wordmark_tail
            ],
        );

        let dark = InterfaceTheme::Dark.palette();
        assert_eq!(
            logo_wordmark_inks(include_str!("../docs/logo-dark.svg")),
            vec![dark.wordmark_body, dark.wordmark_lead, dark.wordmark_tail],
        );
    }

    #[test]
    fn mix_walks_each_channel_from_one_colour_to_the_other() {
        assert_eq!(mix(0x000000, 0xffffff, 0.), 0x000000);
        assert_eq!(mix(0x000000, 0xffffff, 1.), 0xffffff);
        assert_eq!(mix(0x000000, 0xffffff, 0.5), 0x808080);
        assert_eq!(mix(0x0b0d11, 0xffffff, 0.06), 0x1a1c1f);
        assert_eq!(mix(0x102030, 0x304050, 2.), 0x304050, "amount is clamped");
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
