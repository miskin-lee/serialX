//! Semantic colour laid over what the device prints: the parts of a line a
//! reader picks out anyway — a level, a time, an address, a path, a unit —
//! each in a colour of its own, without touching the bytes the device sent.
//!
//! The terminal grid stays exactly what the device drew. When a row is
//! rendered its text is run through an ordered set of patterns, and each
//! pattern claims the characters it recognises that no earlier pattern has,
//! the way `tailspin` layers its highlighters. A pattern late in the order
//! fills in around the earlier ones, which is how a number inside a quoted
//! string keeps its own colour while the quotes take theirs. Cells the device
//! coloured itself are left alone by the renderer, so a firmware's own log
//! colours win over ours.
//!
//! Matching is per row and cached by the row's text, so a screen that is not
//! changing costs nothing to keep coloured.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use regex::{Regex, RegexSet};

use crate::theme::InterfaceTheme;

/// What a stretch of a line is, as far as colour is concerned. The roles
/// are finer than a reader would name them — an address is its digits, its
/// letters and its separators — because that is where the colour goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Role {
    // How serious a line is.
    Error,
    Warning,
    Info,
    Debug,
    Trace,
    Success,
    // Literals that mean yes and no.
    True,
    Null,
    // When.
    Date,
    Time,
    Zone,
    TimeSeparator,
    /// A time since boot: the kernel's `[    1.234567]`, ESP-IDF's `(1234)`.
    Uptime,
    // How much.
    Number,
    Percent,
    Duration,
    DurationUnit,
    Size,
    SizeUnit,
    /// A physical reading: `3.3V`, `-67dBm`, `25°C`, `115200 baud`.
    Measure,
    MeasureUnit,
    Version,
    // Addresses, by the shape of their parts.
    HexPrefix,
    HexDigit,
    HexLetter,
    /// A byte in a dump: `DE AD BE EF`.
    HexByte,
    IpDigit,
    IpLetter,
    IpSeparator,
    MacDigit,
    MacLetter,
    MacSeparator,
    UuidDigit,
    UuidLetter,
    UuidSeparator,
    // Where.
    UrlScheme,
    UrlHost,
    UrlPath,
    QueryKey,
    QueryValue,
    UrlSymbol,
    EmailName,
    EmailSymbol,
    EmailDomain,
    PathSegment,
    PathSeparator,
    /// `main.c` in `main.c:42`.
    SourceFile,
    LineNumber,
    // Who is talking, and how the line is put together.
    ProcessName,
    ProcessId,
    ProcessBracket,
    /// A module or component name: `wifi:` in ESP-IDF, `[main]` in a log.
    Tag,
    TagBracket,
    Key,
    KeySeparator,
    JsonKey,
    Punctuation,
    Quote,
    // The device's own vocabulary.
    /// An AT command or its unsolicited reply: `AT+CWJAP`, `+CWJAP:`.
    Command,
    /// A peripheral or pin: `GPIO12`, `UART1`, `PA0`.
    Peripheral,
    Checksum,
    /// A name in capitals: an error code, a state, a define.
    Constant,
    // HTTP methods, each on a ground of its own, as `tailspin` sets them.
    HttpGet,
    HttpPost,
    HttpPut,
    HttpDelete,
    HttpOther,
    // A shell's listings: the columns of `ls -l`, and the prompt above them.
    /// A directory, as its type letter and as its name.
    Directory,
    Symlink,
    Executable,
    FileName,
    /// A block or character device, a pipe, a socket.
    Special,
    PermRead,
    PermWrite,
    PermExec,
    User,
    Group,
    Host,
    /// The `$` or `#` a shell waits with.
    Prompt,
    /// A command-line switch: `-la`, `--color`.
    Flag,
}

/// How a role is drawn in one theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoleStyle {
    pub(crate) color: u32,
    /// A ground of its own, for the few roles set as a pill.
    pub(crate) background: Option<u32>,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
}

/// A role's colour in both themes, with the weight it is set in.
#[derive(Clone, Copy)]
struct Ink {
    dark: u32,
    light: u32,
    /// A ground in both themes, for a pill.
    background: Option<(u32, u32)>,
    bold: bool,
    italic: bool,
    underline: bool,
}

const fn ink(dark: u32, light: u32) -> Ink {
    Ink {
        dark,
        light,
        background: None,
        bold: false,
        italic: false,
        underline: false,
    }
}

impl Ink {
    const fn bold(self) -> Self {
        Self { bold: true, ..self }
    }

    const fn italic(self) -> Self {
        Self {
            italic: true,
            ..self
        }
    }

    const fn underline(self) -> Self {
        Self {
            underline: true,
            ..self
        }
    }

    /// Set on a ground of `ground`'s colour: a pill.
    const fn on(self, ground: Ink) -> Self {
        Self {
            background: Some((ground.dark, ground.light)),
            ..self
        }
    }
}

// The wheel the roles are painted from, a dark and a light reading of each
// hue: the dark ones sit on the terminal's near-black, the light ones on
// white, and each pair reads as the same colour across the switch. Hues
// step around the circle finely enough that neighbours in a line — a value
// and its unit, a host and its path — are never the same family.
const RED: Ink = ink(0xff6b6b, 0xc62828);
const CORAL: Ink = ink(0xff8a80, 0xd84a3a);
const ORANGE: Ink = ink(0xff9f5a, 0xc65a10);
const PEACH: Ink = ink(0xffb08a, 0xc66a2b);
const AMBER: Ink = ink(0xffb454, 0xb45f06);
const SAFFRON: Ink = ink(0xffd166, 0xa37b00);
const GOLD: Ink = ink(0xf0c674, 0xa8730f);
const SAND: Ink = ink(0xe0b070, 0x9c6f1d);
const LIME: Ink = ink(0xb8e05a, 0x6a8f0e);
const CHARTREUSE: Ink = ink(0xc3e88d, 0x5c8a12);
const GREEN: Ink = ink(0x5be49b, 0x1f8a4c);
const MINT: Ink = ink(0x8ee6b0, 0x2e9e6a);
const TEAL: Ink = ink(0x4fd1c5, 0x0f8a8a);
const AQUA: Ink = ink(0x6fdccf, 0x0f8f83);
const CYAN: Ink = ink(0x5fd1d8, 0x1a8a96);
const SKY: Ink = ink(0x7dc4ff, 0x1a6fc0);
const BLUE: Ink = ink(0x7aa2f7, 0x2f6fd6);
const STEEL: Ink = ink(0x82aaff, 0x3b6fd1);
const PERIWINKLE: Ink = ink(0x9aa5ff, 0x4b5ccf);
const INDIGO: Ink = ink(0x8b87ff, 0x5b57d8);
const VIOLET: Ink = ink(0xb48cff, 0x6f42c1);
const LILAC: Ink = ink(0xc8a2ff, 0x7e57c2);
const ORCHID: Ink = ink(0xd18aff, 0x8e24aa);
const MAGENTA: Ink = ink(0xe08ae8, 0xa03cb8);
const PINK: Ink = ink(0xff8ad0, 0xc2409c);
const ROSE: Ink = ink(0xff8fa3, 0xc2185b);
const GREY: Ink = ink(0x6b7385, 0x8a8f9c);
const DIM: Ink = ink(0x5d6475, 0xa0a4ad);
const SILVER: Ink = ink(0x9aa3b5, 0x6b7280);
const WHITE: Ink = ink(0xc8cdd8, 0x3b3b42);
/// The page itself, for text set on a coloured ground.
const PAGE: Ink = ink(0x0b0d11, 0xffffff);

impl Role {
    /// How the role is drawn in the given theme.
    pub(crate) fn style(self, theme: InterfaceTheme) -> RoleStyle {
        let ink = self.ink();
        let pick = |dark: u32, light: u32| match theme {
            InterfaceTheme::Dark => dark,
            InterfaceTheme::Light => light,
        };
        RoleStyle {
            color: pick(ink.dark, ink.light),
            background: ink.background.map(|(dark, light)| pick(dark, light)),
            bold: ink.bold,
            italic: ink.italic,
            underline: ink.underline,
        }
    }

    const fn ink(self) -> Ink {
        match self {
            // Severity keeps the colours every log reader has learnt, and
            // the two that matter most are set in bold.
            Self::Error => RED.bold(),
            Self::Warning => AMBER.bold(),
            Self::Info => SKY,
            Self::Debug => VIOLET,
            Self::Trace => GREY,
            Self::Success => GREEN.bold(),
            // Yes and no lean, so a value reads apart from a word.
            Self::True => MINT.italic(),
            Self::Null => ROSE.italic(),
            // Time is the blue-to-purple quarter: the date warm, the clock
            // cool, the zone a flag on the end.
            Self::Date => ORCHID,
            Self::Time => BLUE,
            Self::Zone => CORAL,
            Self::TimeSeparator => DIM,
            Self::Uptime => PERIWINKLE,
            // Quantities: a value in a cool colour, its unit a warm one and
            // leaning, so `350ms` reads as two things.
            Self::Number => CYAN,
            Self::Percent => SAFFRON,
            Self::Duration => BLUE,
            Self::DurationUnit => MAGENTA.italic(),
            Self::Size => AQUA,
            Self::SizeUnit => PEACH.italic(),
            Self::Measure => CHARTREUSE,
            Self::MeasureUnit => SAND.italic(),
            Self::Version => LIME,
            // Addresses: digits and letters in two colours so a hex word
            // shows its shape, separators in a third so the grouping does.
            Self::HexPrefix => CORAL,
            Self::HexDigit => STEEL,
            Self::HexLetter => LILAC,
            Self::HexByte => TEAL,
            Self::IpDigit => STEEL,
            Self::IpLetter => ORCHID,
            Self::IpSeparator => CORAL,
            Self::MacDigit => AQUA,
            Self::MacLetter => LILAC,
            Self::MacSeparator => DIM,
            Self::UuidDigit => PERIWINKLE.italic(),
            Self::UuidLetter => MAGENTA.italic(),
            Self::UuidSeparator => DIM,
            // Places.
            Self::UrlScheme => TEAL,
            Self::UrlHost => STEEL.underline(),
            Self::UrlPath => PERIWINKLE,
            Self::QueryKey => ORCHID,
            Self::QueryValue => CYAN,
            Self::UrlSymbol => SILVER,
            Self::EmailName => GREEN.underline(),
            Self::EmailSymbol => CORAL,
            Self::EmailDomain => GREEN.underline(),
            Self::PathSegment => GREEN,
            Self::PathSeparator => GOLD,
            Self::SourceFile => GOLD,
            Self::LineNumber => CYAN,
            // Structure is quiet — keys, brackets and punctuation step back
            // so the values they frame step forward.
            Self::ProcessName => PEACH,
            Self::ProcessId => CYAN,
            Self::ProcessBracket => CORAL,
            Self::Tag => TEAL,
            Self::TagBracket => DIM,
            Self::Key => SILVER,
            Self::KeySeparator => WHITE,
            Self::JsonKey => STEEL,
            Self::Punctuation => DIM,
            Self::Quote => SAND,
            // The device's own words.
            Self::Command => ORANGE.bold(),
            Self::Peripheral => PINK,
            Self::Checksum => INDIGO,
            Self::Constant => GOLD,
            // HTTP methods are pills: the page's ink on a ground that says
            // what the request does, green to read and red to remove.
            Self::HttpGet => PAGE.on(GREEN).bold(),
            Self::HttpPost => PAGE.on(AMBER).bold(),
            Self::HttpPut => PAGE.on(VIOLET).bold(),
            Self::HttpDelete => PAGE.on(RED).bold(),
            Self::HttpOther => PAGE.on(BLUE).bold(),
            // A listing keeps the colours `ls` itself would use, so a
            // directory is the blue it has always been; the mode string is
            // read a letter at a time, as `eza` sets it.
            Self::Directory => BLUE.bold(),
            Self::Symlink => TEAL.bold(),
            Self::Executable => GREEN.bold(),
            Self::FileName => WHITE,
            Self::Special => MAGENTA.bold(),
            Self::PermRead => SAFFRON,
            Self::PermWrite => CORAL,
            Self::PermExec => GREEN,
            Self::User => GOLD.bold(),
            Self::Group => SAND,
            Self::Host => STEEL,
            Self::Prompt => ORANGE.bold(),
            Self::Flag => PINK,
        }
    }
}

/// How a matched stretch is coloured.
#[derive(Clone, Copy)]
enum Fill {
    /// One role throughout.
    Solid(Role),
    /// Digits, letters and everything else each their own role: how an
    /// address is read, by the shape of its parts.
    Split {
        digit: Role,
        letter: Role,
        other: Role,
    },
    /// Text broken by delimiter characters, the two in their own roles.
    Delimited {
        text: Role,
        delimiter: Role,
        delimiters: &'static str,
    },
    /// A URL query: `key=value&key=value`.
    Query,
    /// A severity word, read to find which level it names.
    Level,
    /// A mode string, `drwxr-xr-x`: the type letter by what it is, then
    /// read, write and execute each in their own colour.
    Permissions,
}

impl Fill {
    /// Gives the unclaimed characters of `segment`, which begins at
    /// character `start` of the line, the roles this fill hands out.
    fn apply(self, segment: &str, start: usize, owner: &mut [Option<Role>]) {
        let level = matches!(self, Self::Level).then(|| level_role(segment));
        let mut in_value = false;
        for (offset, ch) in segment.chars().enumerate() {
            let role = match self {
                Self::Solid(role) => role,
                Self::Split {
                    digit,
                    letter,
                    other,
                } => {
                    if ch.is_ascii_digit() {
                        digit
                    } else if ch.is_ascii_alphabetic() {
                        letter
                    } else {
                        other
                    }
                }
                Self::Delimited {
                    text,
                    delimiter,
                    delimiters,
                } => {
                    if delimiters.contains(ch) {
                        delimiter
                    } else {
                        text
                    }
                }
                Self::Query => match ch {
                    '&' => {
                        in_value = false;
                        Role::UrlSymbol
                    }
                    '=' if !in_value => {
                        in_value = true;
                        Role::UrlSymbol
                    }
                    _ if in_value => Role::QueryValue,
                    _ => Role::QueryKey,
                },
                Self::Level => level.unwrap_or(Role::Info),
                Self::Permissions => match (offset, ch) {
                    (0, 'd') => Role::Directory,
                    (0, 'l') => Role::Symlink,
                    (0, '-') => Role::Punctuation,
                    (0, _) => Role::Special,
                    (1..=9, 'r') => Role::PermRead,
                    (1..=9, 'w') => Role::PermWrite,
                    (1..=9, 'x' | 's' | 'S' | 't' | 'T') => Role::PermExec,
                    _ => Role::Punctuation,
                },
            };
            let slot = &mut owner[start + offset];
            if slot.is_none() {
                *slot = Some(role);
            }
        }
    }
}

/// The level a severity word names, whichever log format spelt it.
fn level_role(word: &str) -> Role {
    match word.to_ascii_lowercase().as_str() {
        "e" | "err" | "error" | "fatal" | "critical" | "crit" | "panic" | "fail" | "failed"
        | "failure" | "assert" | "exception" | "traceback" | "emerg" | "alert" => Role::Error,
        "w" | "wrn" | "warn" | "warning" => Role::Warning,
        "i" | "inf" | "info" | "notice" => Role::Info,
        "d" | "dbg" | "debug" => Role::Debug,
        "v" | "trace" | "verbose" => Role::Trace,
        _ => Role::Success,
    }
}

/// One thing to look for, and how to colour what is found. `None` names
/// the whole match, a string one of its named groups; fills are applied in
/// order, so a group listed before the whole match takes its characters
/// first.
struct Pattern {
    regex: Regex,
    fills: Vec<(Option<&'static str>, Fill)>,
}

fn pattern(regex: &str, fills: Vec<(Option<&'static str>, Fill)>) -> Pattern {
    Pattern {
        regex: Regex::new(regex).expect("a built-in highlight pattern compiles"),
        fills,
    }
}

/// The whole match in one fill.
fn whole(regex: &str, fill: Fill) -> Pattern {
    pattern(regex, vec![(None, fill)])
}

/// Each named group in its own fill; what lies between them is left alone.
fn parts(regex: &str, fills: &[(&'static str, Fill)]) -> Pattern {
    pattern(
        regex,
        fills
            .iter()
            .map(|(name, fill)| (Some(*name), *fill))
            .collect(),
    )
}

/// The patterns, in the order they get to claim characters. Places come
/// first so a word inside a path or URL stays part of it; then the device's
/// own vocabulary and the keys of `key=value` pairs, which are what they are
/// whatever word they use; then the words that say how a line went;
/// addresses before times, because the tail of a MAC address looks like a
/// clock; then quantities; and last the numbers, punctuation and quotes
/// that fill in around everything else.
fn patterns() -> Vec<Pattern> {
    use Fill::{Delimited, Level, Query, Solid, Split};
    use Role::*;

    let time = Delimited {
        text: Time,
        delimiter: TimeSeparator,
        delimiters: ":.,",
    };
    let date = |delimiters| Delimited {
        text: Date,
        delimiter: TimeSeparator,
        delimiters,
    };
    let path = |delimiters| Delimited {
        text: PathSegment,
        delimiter: PathSeparator,
        delimiters,
    };
    let ip = Split {
        digit: IpDigit,
        letter: IpLetter,
        other: IpSeparator,
    };

    // The columns of `ls -l` after the mode: link count, owner, group, size
    // (or a device's major and minor), then the date as GNU, BusyBox, macOS
    // or a localised `ls` prints it, with a clock or a year after it.
    let ls_columns = concat!(
        r"\s+(?P<links>\d+)\s+(?P<user>[\w.$-]+)\s+(?P<group>[\w.$-]+)\s+",
        r"(?P<size>\d+,\s+\d+|\d[\d,.]*[KMGTPE]?i?B?)\s+",
        r"(?P<date>[^\s\d]{1,9}\.? {1,2}\d{1,2}|\d{1,2}月 {1,2}\d{1,2}日?|\d{4}-\d{2}-\d{2})\s+",
        r"(?:(?P<time>\d{1,2}:\d{2}(?::\d{2})?(?:\.\d+)?)|(?P<year>\d{4}))(?: (?P<zone>[+-]\d{4}))?\s+",
    );
    let ls_fills = [
        ("perm", Fill::Permissions),
        ("links", Solid(Number)),
        ("user", Solid(User)),
        ("group", Solid(Group)),
        ("size", Solid(Size)),
        ("date", Solid(Date)),
        ("time", time),
        ("year", Solid(Date)),
        ("zone", Solid(Zone)),
    ];
    // One pattern per kind of entry, so the name takes the colour of what
    // it is; the last catches whatever the first three did not.
    let listing = |mode: &str, name: &str, fills: &[(&'static str, Fill)]| {
        parts(
            &format!("^(?P<perm>{mode}[.+@]?){ls_columns}{name}$"),
            &[ls_fills.as_slice(), fills].concat(),
        )
    };
    let prompt_fills = [
        ("user", Solid(User)),
        ("at", Solid(EmailSymbol)),
        ("host", Solid(Host)),
        ("colon", Solid(Punctuation)),
        ("path", path("/")),
        ("sigil", Solid(Prompt)),
    ];

    vec![
        // ---- A shell's listings ---------------------------------------------
        listing(
            r"l[-rwxsStT]{9}",
            r"(?P<name>.+?) (?P<arrow>->) (?P<target>.+)",
            &[
                ("name", Solid(Symlink)),
                ("arrow", Solid(Punctuation)),
                ("target", path("/")),
            ],
        ),
        listing(
            r"d[-rwxsStT]{9}",
            r"(?P<name>.+)",
            &[("name", Solid(Directory))],
        ),
        listing(
            r"-[-rw]{2}[xs][-rwxsStT]{6}",
            r"(?P<name>.+)",
            &[("name", Solid(Executable))],
        ),
        listing(
            r"[-dlbcps][-rwxsStT]{9}",
            r"(?P<name>.+)",
            &[("name", Solid(FileName))],
        ),
        // A mode string on its own, from `find -ls`, `stat` or `tar tv`.
        parts(
            r"(?:^|\s)(?P<perm>[-dlbcps][-rwxsStT]{9}[.+@]?)(?:\s|$)",
            &[("perm", Fill::Permissions)],
        ),
        // The prompt a shell waits with: `user@host:~/dir$` or `[user@host dir]#`.
        parts(
            r"^(?P<user>[\w.-]+)(?P<at>@)(?P<host>[\w.-]+)(?P<colon>:)(?P<path>[~/][^\s$#]*)?\s?(?P<sigil>[$#])(?:\s|$)",
            &prompt_fills,
        ),
        parts(
            r"^\[(?P<user>[\w.-]+)(?P<at>@)(?P<host>[\w.-]+) (?P<path>[^\]]+)\](?P<sigil>[$#])(?:\s|$)",
            &prompt_fills,
        ),
        // ---- Places ------------------------------------------------------
        parts(
            r#"\b(?P<scheme>[a-zA-Z][a-zA-Z0-9+.-]*)(?P<colon>://)(?P<host>[^\s/?#"'<>()\[\]]+)(?P<path>/[^\s?#"'<>()\[\]]*)?(?:(?P<question>\?)(?P<query>[^\s#"'<>()\[\]]*))?"#,
            &[
                ("scheme", Solid(UrlScheme)),
                ("colon", Solid(UrlSymbol)),
                (
                    "host",
                    Delimited {
                        text: UrlHost,
                        delimiter: UrlSymbol,
                        delimiters: ":",
                    },
                ),
                (
                    "path",
                    Delimited {
                        text: UrlPath,
                        delimiter: UrlSymbol,
                        delimiters: "/",
                    },
                ),
                ("question", Solid(UrlSymbol)),
                ("query", Query),
            ],
        ),
        parts(
            r"\b(?P<name>[\w.+-]+)(?P<at>@)(?P<domain>[\w-]+(?:\.[\w-]+)+)\b",
            &[
                ("name", Solid(EmailName)),
                ("at", Solid(EmailSymbol)),
                (
                    "domain",
                    Delimited {
                        text: EmailDomain,
                        delimiter: EmailSymbol,
                        delimiters: ".",
                    },
                ),
            ],
        ),
        // `main.c:42` — where an assert or a panic says it happened.
        parts(
            r"\b(?P<file>[\w-]+\.(?:c|h|cc|cpp|cxx|hpp|hh|rs|py|js|ts|go|java|kt|swift|m|mm|lua|sh|ino|s|S|asm|ld|v|sv))(?P<colon>:)(?P<line>\d+)\b",
            &[
                ("file", Solid(SourceFile)),
                ("colon", Solid(Punctuation)),
                ("line", Solid(LineNumber)),
            ],
        ),
        whole(
            r#"\b[A-Za-z]:\\(?:[^\\\s"'<>|*?]+\\)*[^\\\s"'<>|*?]*"#,
            path(":\\"),
        ),
        // A Unix path has to start a word: `ERROR/WARN` is not one.
        parts(
            r#"(?:^|[\s"'`(\[<=,;:])(?P<path>(?:~|\.{1,2})?(?:/[\w.@+~%-]+)+/?)"#,
            &[("path", path("/"))],
        ),
        // ---- The device's own words -------------------------------------------
        whole(
            r"\bAT(?:[+#$&%][A-Za-z0-9_]*|[A-Z][A-Z0-9]{0,4})?\b",
            Solid(Command),
        ),
        whole(r"^\+[A-Z][A-Z0-9_]*:?", Solid(Command)),
        whole(
            r"\b(?:GPIO|IO|PIN|EXTI|P[A-K]|D|A)\d{1,2}\b|\b(?:UART|USART|LPUART|SPI|QSPI|I2C|I2S|TIM|TIMER|ADC|DAC|DMA|CAN|FDCAN|USB|OTG|PWM|RTC|WDT|IWDG|WWDG|SDIO|SDMMC|FMC|ETH|MAC|PHY|CORE|CPU|IRQ|NVIC|FLASH|SRAM|PSRAM|EEPROM|NVS|SPIFFS|LittleFS|FATFS)\d{0,2}\b",
            Solid(Peripheral),
        ),
        whole(
            r"\b(?:CRC|CRC8|CRC16|CRC32|crc|crc8|crc16|crc32|[Cc]hecksum|CHECKSUM|chksum|cksum|LRC|FCS|MD5|md5|SHA1|SHA256|SHA-256|sha256|[Hh]ash|HASH|XOR)\b",
            Solid(Checksum),
        ),
        whole(r"\bGET\b", Solid(HttpGet)),
        whole(r"\bPOST\b", Solid(HttpPost)),
        whole(r"\b(?:PUT|PATCH)\b", Solid(HttpPut)),
        whole(r"\bDELETE\b", Solid(HttpDelete)),
        whole(r"\b(?:HEAD|OPTIONS|CONNECT)\b", Solid(HttpOther)),
        // A switch on a command line: `-la`, `--color`. Before keys, so
        // `--color=auto` is a switch with a value rather than a key.
        parts(
            r"(?:^|\s)(?P<flag>--?[A-Za-z][\w-]*)",
            &[("flag", Solid(Flag))],
        ),
        // A key is a key whatever it is called: `err=null` and `"err": null`
        // both name a field.
        parts(
            r"\b(?P<key>[A-Za-z_][\w.-]*)(?P<eq>=)",
            &[("key", Solid(Key)), ("eq", Solid(KeySeparator))],
        ),
        parts(
            r#"(?P<open>")(?P<key>[^"]+)(?P<close>")\s*(?P<colon>:)"#,
            &[
                ("open", Solid(Punctuation)),
                ("key", Solid(JsonKey)),
                ("close", Solid(Punctuation)),
                ("colon", Solid(Punctuation)),
            ],
        ),
        // A name in capitals with an underscore in it is a code, a state,
        // a define: `ESP_ERR_INVALID_STATE`, `WL_CONNECTED`.
        whole(r"\b[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+\b", Solid(Constant)),
        // ---- How the line went ----------------------------------------------
        // ESP-IDF: `I (1234) wifi: connected`.
        parts(
            r"^(?P<level>[EWIDV]) \((?P<uptime>\d+)\)(?: (?P<tag>[\w.:/-]+):)?",
            &[
                ("level", Level),
                ("uptime", Solid(Uptime)),
                ("tag", Solid(Tag)),
            ],
        ),
        // Zephyr: `[00:00:01.234,567] <inf> main: Hello`.
        parts(
            r"^(?:(?P<open>\[)(?P<uptime>\d{2}:\d{2}:\d{2}\.\d{3}(?:,\d{3})?)(?P<close>\]) )?(?P<lt><)(?P<level>err|wrn|inf|dbg)(?P<gt>>)(?: (?P<tag>[\w.:/-]+):)?",
            &[
                ("open", Solid(TagBracket)),
                ("close", Solid(TagBracket)),
                ("lt", Solid(TagBracket)),
                ("gt", Solid(TagBracket)),
                (
                    "uptime",
                    Delimited {
                        text: Uptime,
                        delimiter: TimeSeparator,
                        delimiters: ":.,",
                    },
                ),
                ("level", Level),
                ("tag", Solid(Tag)),
            ],
        ),
        parts(
            r"(?i)\b(?P<level>error|err|fatal|critical|crit|panic|fail|failed|failure|assert|exception|traceback|emerg|alert|warning|warn|info|notice|debug|dbg|verbose|trace)\b",
            &[("level", Level)],
        ),
        whole(
            r"\b(?:Guru Meditation Error|Backtrace|Segmentation fault|[Ss]tack overflow|Assertion failed|assert failed|[Cc]ore dump|Hard[Ff]ault|BusFault|UsageFault|MemManage|Rebooting|No such file or directory|Permission denied|command not found|Connection refused|Operation not permitted|Input/output error|Device or resource busy|not found)\b|abort\(\)",
            Solid(Error),
        ),
        whole(
            r"\b(?:[Ww]atchdog|WDT|[Bb]rownout|[Rr]etry(?:ing)?|[Tt]imeout|[Tt]imed out|[Dd]isconnected|[Ll]ost|[Dd]eprecated)\b",
            Solid(Warning),
        ),
        whole(
            r"\b(?:OK|SUCCESS|SUCCEEDED|PASS|PASSED|DONE|READY|CONNECTED|ONLINE|Success|Succeeded|Passed|Done|Ready|Connected|Online|success|succeeded|connected|ready|done)\b",
            Solid(Success),
        ),
        whole(
            r"\b(?:true|TRUE|True|yes|YES|Yes|enabled|ENABLED|Enabled)\b",
            Solid(True),
        ),
        whole(
            r"\b(?:false|FALSE|False|null|NULL|Null|nil|NIL|None|none|NONE|NaN|undefined|nullptr|disabled|DISABLED|Disabled)\b",
            Solid(Null),
        ),
        // ---- Addresses --------------------------------------------------------
        // Before times: the tail of a MAC address looks like a clock.
        whole(
            r"\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b",
            Split {
                digit: MacDigit,
                letter: MacLetter,
                other: MacSeparator,
            },
        ),
        whole(
            r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b|\b(?:[0-9a-f]{1,4}:){1,6}:[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,5}\b|::[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,6}\b",
            ip,
        ),
        whole(r"\b(?:\d{1,3}\.){3}\d{1,3}(?::\d{1,5})?\b", ip),
        whole(
            r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
            Split {
                digit: UuidDigit,
                letter: UuidLetter,
                other: UuidSeparator,
            },
        ),
        // `0x` after its digits, so the prefix is what is left for it.
        pattern(
            r"\b0[xX](?P<digits>[0-9a-fA-F]+)\b",
            vec![
                (
                    Some("digits"),
                    Split {
                        digit: HexDigit,
                        letter: HexLetter,
                        other: HexLetter,
                    },
                ),
                (None, Solid(HexPrefix)),
            ],
        ),
        // ---- When -----------------------------------------------------------
        parts(
            r"\b(?P<date>\d{4}[-/]\d{2}[-/]\d{2})(?:(?P<t>[T ])(?P<time>\d{2}:\d{2}(?::\d{2})?(?:[.,]\d{1,9})?)(?P<zone>Z|[+-]\d{2}:?\d{2})?)?\b",
            &[
                ("date", date("-/")),
                ("t", Solid(TimeSeparator)),
                ("time", time),
                ("zone", Solid(Zone)),
            ],
        ),
        parts(
            r"\b(?P<date>\d{1,2}[/.-]\d{1,2}[/.-]\d{4})\b",
            &[("date", date("/.-"))],
        ),
        whole(
            r"\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]* {1,2}\d{1,2}(?:,? \d{4})?\b",
            Solid(Date),
        ),
        // The kernel's seconds since boot: `[    1.234567] usb 1-1: ...`.
        parts(
            r"^(?P<open>\[)(?P<uptime>\s*\d+\.\d{3,6})(?P<close>\])",
            &[
                ("open", Solid(TagBracket)),
                ("uptime", Solid(Uptime)),
                ("close", Solid(TagBracket)),
            ],
        ),
        parts(
            r"\b(?P<time>\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d{1,9})?)(?P<zone>Z|[+-]\d{2}:?\d{2}| ?(?:UTC|GMT))?\b",
            &[("time", time), ("zone", Solid(Zone))],
        ),
        // A dump of bytes: after times, since `10:20:30` is also three
        // pairs of hex digits.
        // Separated by spaces or by colons, never a mixture: `19 17:15` in
        // a listing is a date and a clock, not three bytes.
        whole(
            r"\b(?:[0-9A-Fa-f]{2} ){2,}[0-9A-Fa-f]{2}\b|\b(?:[0-9A-Fa-f]{2}[:-]){2,}[0-9A-Fa-f]{2}\b",
            Split {
                digit: HexByte,
                letter: HexLetter,
                other: Punctuation,
            },
        ),
        // ---- Who is talking ---------------------------------------------------
        parts(
            r"\b(?P<name>[A-Za-z_][\w.-]*)(?P<open>\[)(?P<pid>\d+)(?P<close>\])",
            &[
                ("name", Solid(ProcessName)),
                ("open", Solid(ProcessBracket)),
                ("pid", Solid(ProcessId)),
                ("close", Solid(ProcessBracket)),
            ],
        ),
        parts(
            r"(?P<open>\[)(?P<tag>[A-Za-z_][\w:./ -]{0,31}?)(?P<close>\])",
            &[
                ("open", Solid(TagBracket)),
                ("tag", Solid(Tag)),
                ("close", Solid(TagBracket)),
            ],
        ),
        parts(
            r"^(?P<tag>[A-Za-z_][\w./-]{0,31}):(?:\s|$)",
            &[("tag", Solid(Tag))],
        ),
        // ---- How much -----------------------------------------------------------
        parts(
            r"\b(?P<value>\d+(?:\.\d+)?)(?P<unit>ns|us|µs|μs|ms|s|sec|secs|m|min|mins|h|hr|hrs|d|days?)\b",
            &[("value", Solid(Duration)), ("unit", Solid(DurationUnit))],
        ),
        parts(
            r"\b(?P<value>\d+(?:\.\d+)?) ?(?P<unit>B|KB|kB|KiB|MB|MiB|GB|GiB|TB|TiB|[Bb]ytes?|kb|mb|gb|Kb|Mb)\b",
            &[("value", Solid(Size)), ("unit", Solid(SizeUnit))],
        ),
        parts(
            r"(?P<value>-?\b\d+(?:\.\d+)?) ?(?P<unit>Hz|kHz|KHz|MHz|GHz|mV|uV|V|uA|mA|A|mW|W|kW|dBm|dB|°C|℃|°F|C|F|K|bps|kbps|Mbps|Gbps|baud|Ω|ohm|kΩ|mAh|Wh|mWh|lux|hPa|kPa|Pa|bar|rpm|RPM|%RH|RH|ppm|ppb|deg|rad|mm|cm|km|kg|mg)\b",
            &[("value", Solid(Measure)), ("unit", Solid(MeasureUnit))],
        ),
        whole(r"-?\b\d+(?:\.\d+)?%", Solid(Percent)),
        whole(
            r"\b[vV]?\d+\.\d+\.\d+(?:\.\d+)?(?:[-+][\w.]+)?\b",
            Solid(Version),
        ),
        // ---- Structure ----------------------------------------------------------
        whole(r"-?\b\d+(?:\.\d+)?\b", Solid(Number)),
        whole(r"[{}\[\]]", Solid(Punctuation)),
        // Quotes come last and fill in around what is already coloured, so
        // the number inside a string keeps its colour and the string its own.
        whole(r#""[^"]*""#, Solid(Quote)),
        whole(r"`[^`]+`", Solid(Quote)),
    ]
}

/// The patterns compiled, with a memory of the rows it has already read.
pub(crate) struct Highlighter {
    patterns: Vec<Pattern>,
    /// Which patterns match a line at all, in one pass, so only those are
    /// run for their captures.
    any: RegexSet,
    cache: Mutex<HashMap<String, Arc<[Option<Role>]>>>,
}

/// Rows remembered before the memory is emptied and started again: more
/// than a screen holds, fewer than a scrollback does.
const CACHE_ROWS: usize = 4096;

impl Highlighter {
    /// The one highlighter, built the first time it is asked for. Compiling
    /// the patterns is the expensive part and there is no reason to do it
    /// per tab.
    pub(crate) fn shared() -> &'static Highlighter {
        static SHARED: OnceLock<Highlighter> = OnceLock::new();
        SHARED.get_or_init(Highlighter::new)
    }

    fn new() -> Self {
        let patterns = patterns();
        let any = RegexSet::new(patterns.iter().map(|pattern| pattern.regex.as_str()))
            .expect("the highlight patterns compile as a set");
        Self {
            patterns,
            any,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The role of each character of `text`, `None` where it is plain.
    pub(crate) fn roles(&self, text: &str) -> Arc<[Option<Role>]> {
        if let Some(roles) = self.cache.lock().unwrap().get(text) {
            return roles.clone();
        }
        let roles: Arc<[Option<Role>]> = self.compute(text).into();
        let mut cache = self.cache.lock().unwrap();
        if cache.len() >= CACHE_ROWS {
            cache.clear();
        }
        cache.insert(text.to_owned(), roles.clone());
        roles
    }

    fn compute(&self, text: &str) -> Vec<Option<Role>> {
        let mut owner = vec![None; text.chars().count()];
        if text.is_empty() {
            return owner;
        }
        // The pattern engine speaks in bytes; the grid, in characters.
        let mut char_at = vec![0usize; text.len() + 1];
        for (index, (byte, ch)) in text.char_indices().enumerate() {
            char_at[byte..byte + ch.len_utf8()].fill(index);
        }
        char_at[text.len()] = owner.len();

        for index in self.any.matches(text) {
            let pattern = &self.patterns[index];
            for captures in pattern.regex.captures_iter(text) {
                for (target, fill) in &pattern.fills {
                    let matched = match target {
                        None => captures.get(0),
                        Some(name) => captures.name(name),
                    };
                    if let Some(matched) = matched {
                        fill.apply(matched.as_str(), char_at[matched.start()], &mut owner);
                    }
                }
            }
        }
        owner
    }
}

#[cfg(test)]
mod tests {
    use super::{Highlighter, Role, Role::*};
    use crate::theme::InterfaceTheme;

    /// The line as it would be coloured: each stretch with its role, or
    /// `None` where it is plain.
    fn painted(text: &str) -> Vec<(&str, Option<Role>)> {
        let roles = Highlighter::shared().roles(text);
        let mut out: Vec<(usize, usize, Option<Role>)> = Vec::new();
        for (index, (byte, ch)) in text.char_indices().enumerate() {
            let end = byte + ch.len_utf8();
            match out.last_mut() {
                Some((_, last_end, role)) if *role == roles[index] => *last_end = end,
                _ => out.push((byte, end, roles[index])),
            }
        }
        out.into_iter()
            .map(|(start, end, role)| (&text[start..end], role))
            .collect()
    }

    /// Only the coloured stretches, for lines where the plain gaps are noise.
    fn coloured(text: &str) -> Vec<(&str, Role)> {
        painted(text)
            .into_iter()
            .filter_map(|(text, role)| role.map(|role| (text, role)))
            .collect()
    }

    #[test]
    fn an_esp_idf_line_is_read_by_its_parts() {
        assert_eq!(
            coloured("I (1234) wifi: connected to 192.168.1.20 in 350ms"),
            vec![
                ("I", Info),
                ("1234", Uptime),
                ("wifi", Tag),
                ("connected", Success),
                ("192", IpDigit),
                (".", IpSeparator),
                ("168", IpDigit),
                (".", IpSeparator),
                ("1", IpDigit),
                (".", IpSeparator),
                ("20", IpDigit),
                ("350", Duration),
                ("ms", DurationUnit),
            ]
        );
        assert_eq!(coloured("E (99) main: failed")[0], ("E", Error));
        assert_eq!(coloured("W (99) x: y")[0], ("W", Warning));
        assert_eq!(coloured("D (99) x: y")[0], ("D", Debug));
        assert_eq!(coloured("V (99) x: y")[0], ("V", Trace));
    }

    #[test]
    fn a_zephyr_line_is_read_by_its_parts() {
        let line = coloured("[00:00:01.234,567] <wrn> bt_hci: timeout");
        assert_eq!(line[0], ("[", TagBracket));
        assert_eq!(line[1], ("00", Uptime));
        assert_eq!(line[2], (":", TimeSeparator));
        assert!(line.contains(&("wrn", Warning)));
        assert!(line.contains(&("bt_hci", Tag)));
        assert!(line.contains(&("timeout", Warning)));
    }

    #[test]
    fn severity_words_are_read_in_any_case() {
        assert_eq!(coloured("Error: it broke")[0], ("Error", Error));
        assert_eq!(coloured("[WARN] careful")[1], ("WARN", Warning));
        assert_eq!(coloured("debug: x")[0], ("debug", Debug));
        assert_eq!(
            coloured("Guru Meditation Error: Core 0 panic'ed")[0].1,
            Error
        );
        // Not the middle of another word.
        assert_eq!(coloured("errors"), vec![]);
    }

    #[test]
    fn a_time_is_split_into_its_parts() {
        assert_eq!(
            coloured("2024-03-05T10:20:30.123Z"),
            vec![
                ("2024", Date),
                ("-", TimeSeparator),
                ("03", Date),
                ("-", TimeSeparator),
                ("05", Date),
                ("T", TimeSeparator),
                ("10", Time),
                (":", TimeSeparator),
                ("20", Time),
                (":", TimeSeparator),
                ("30", Time),
                (".", TimeSeparator),
                ("123", Time),
                ("Z", Zone),
            ]
        );
        assert_eq!(coloured("at 10:20:30 UTC")[0], ("10", Time));
        assert_eq!(coloured("at 10:20:30 UTC").last(), Some(&(" UTC", Zone)));
        assert_eq!(
            coloured("[    1.234567] usb 1-1")[1],
            ("    1.234567", Uptime)
        );
    }

    #[test]
    fn a_url_is_split_into_its_parts() {
        assert_eq!(
            coloured("see https://example.com:8080/api/v1?key=abc&n=2 now"),
            vec![
                ("https", UrlScheme),
                ("://", UrlSymbol),
                ("example.com", UrlHost),
                (":", UrlSymbol),
                ("8080", UrlHost),
                ("/", UrlSymbol),
                ("api", UrlPath),
                ("/", UrlSymbol),
                ("v1", UrlPath),
                ("?", UrlSymbol),
                ("key", QueryKey),
                ("=", UrlSymbol),
                ("abc", QueryValue),
                ("&", UrlSymbol),
                ("n", QueryKey),
                ("=", UrlSymbol),
                ("2", QueryValue),
            ]
        );
        assert_eq!(
            coloured("mail ops@example.org"),
            vec![
                ("ops", EmailName),
                ("@", EmailSymbol),
                ("example", EmailDomain),
                (".", EmailSymbol),
                ("org", EmailDomain),
            ]
        );
    }

    #[test]
    fn addresses_show_their_shape() {
        assert_eq!(
            coloured("mac aa:bb:cc:00:11:22"),
            vec![
                ("aa", MacLetter),
                (":", MacSeparator),
                ("bb", MacLetter),
                (":", MacSeparator),
                ("cc", MacLetter),
                (":", MacSeparator),
                ("00", MacDigit),
                (":", MacSeparator),
                ("11", MacDigit),
                (":", MacSeparator),
                ("22", MacDigit),
            ]
        );
        assert_eq!(
            coloured("at 0x3ffb1a2c"),
            vec![
                ("0x", HexPrefix),
                ("3", HexDigit),
                ("ffb", HexLetter),
                ("1", HexDigit),
                ("a", HexLetter),
                ("2", HexDigit),
                ("c", HexLetter),
            ]
        );
        assert_eq!(coloured("fe80::1")[0], ("fe", IpLetter));
        assert_eq!(
            coloured("id 123e4567-e89b-12d3-a456-426614174000")[0],
            ("123", UuidDigit)
        );
        assert_eq!(
            coloured("id 123e4567-e89b-12d3-a456-426614174000")[2],
            ("4567", UuidDigit)
        );
        assert_eq!(
            coloured("rx: DE AD BE EF"),
            vec![
                ("rx", Tag),
                ("DE", HexLetter),
                (" ", Punctuation),
                ("AD", HexLetter),
                (" ", Punctuation),
                ("BE", HexLetter),
                (" ", Punctuation),
                ("EF", HexLetter),
            ]
        );
        // A Rust path is not an IPv6 address.
        assert_eq!(coloured("core::fmt::write"), vec![]);
    }

    #[test]
    fn places_keep_the_words_inside_them() {
        assert_eq!(
            coloured("open /dev/tty.usbserial-1410 failed"),
            vec![
                ("/", PathSeparator),
                ("dev", PathSegment),
                ("/", PathSeparator),
                ("tty.usbserial-1410", PathSegment),
                ("failed", Error),
            ]
        );
        assert_eq!(
            coloured("assert at main.c:42"),
            vec![
                ("assert", Error),
                ("main.c", SourceFile),
                (":", Punctuation),
                ("42", LineNumber),
            ]
        );
        // The `error` in a path is part of the path.
        assert_eq!(
            coloured("/var/log/error.log")[5],
            ("error.log", PathSegment)
        );
        assert_eq!(
            coloured("ERROR/WARN"),
            vec![("ERROR", Error), ("WARN", Warning)]
        );
    }

    #[test]
    fn structure_steps_back_and_values_step_forward() {
        assert_eq!(
            coloured(r#"count is "value 42 here" end"#),
            vec![(r#""value "#, Quote), ("42", Number), (r#" here""#, Quote),]
        );
        assert_eq!(
            coloured("temp=25 ok=true err=null"),
            vec![
                ("temp", Key),
                ("=", KeySeparator),
                ("25", Number),
                ("ok", Key),
                ("=", KeySeparator),
                ("true", True),
                ("err", Key),
                ("=", KeySeparator),
                ("null", Null),
            ]
        );
        assert_eq!(
            coloured(r#"{"rssi": -67}"#),
            vec![
                (r#"{""#, Punctuation),
                ("rssi", JsonKey),
                (r#"":"#, Punctuation),
                ("-67", Number),
                ("}", Punctuation),
            ]
        );
        assert_eq!(
            coloured("kernel[123]: [main] up"),
            vec![
                ("kernel", ProcessName),
                ("[", ProcessBracket),
                ("123", ProcessId),
                ("]", ProcessBracket),
                ("[", TagBracket),
                ("main", Tag),
                ("]", TagBracket),
            ]
        );
    }

    #[test]
    fn quantities_split_value_from_unit() {
        assert_eq!(
            coloured("vbat 3.3V rssi -67dBm temp 25°C"),
            vec![
                ("3.3", Measure),
                ("V", MeasureUnit),
                ("-67", Measure),
                ("dBm", MeasureUnit),
                ("25", Measure),
                ("°C", MeasureUnit),
            ]
        );
        assert_eq!(
            coloured("free 12 KB of 4MiB, 75% used, fw v1.2.3-rc1"),
            vec![
                ("12", Size),
                ("KB", SizeUnit),
                ("4", Size),
                ("MiB", SizeUnit),
                ("75%", Percent),
                ("v1.2.3-rc1", Version),
            ]
        );
        assert_eq!(coloured("took 5s")[0], ("5", Duration));
        assert_eq!(coloured("took 5s")[1], ("s", DurationUnit));
    }

    #[test]
    fn a_devices_own_words_are_known() {
        assert_eq!(
            coloured("AT+CWJAP=\"ssid\",\"pw\"")[0],
            ("AT+CWJAP", Command)
        );
        assert_eq!(coloured("+CWJAP:1")[0], ("+CWJAP:", Command));
        assert_eq!(
            coloured("GPIO12 high, UART1 open")[0],
            ("GPIO12", Peripheral)
        );
        assert_eq!(
            coloured("GPIO12 high, UART1 open")[1],
            ("UART1", Peripheral)
        );
        assert_eq!(coloured("CRC32 mismatch")[0], ("CRC32", Checksum));
        assert_eq!(coloured("OK"), vec![("OK", Success)]);
        assert_eq!(coloured("GET /index.html 200")[0], ("GET", HttpGet));
        assert_eq!(coloured("DELETE /item")[0], ("DELETE", HttpDelete));
        assert_eq!(
            coloured("err=0x103 (ESP_ERR_INVALID_STATE)")[4],
            ("ESP_ERR_INVALID_STATE", Constant)
        );
        // A key named like a level is still a key.
        assert_eq!(coloured(r#"{"err": null}"#)[1], ("err", JsonKey));
    }

    #[test]
    fn a_listing_is_read_column_by_column() {
        assert_eq!(
            coloured("drwxr-xr-x  3 root root 4096 Aug 19 17:15 .."),
            vec![
                ("d", Directory),
                ("r", PermRead),
                ("w", PermWrite),
                ("x", PermExec),
                ("r", PermRead),
                ("-", Punctuation),
                ("x", PermExec),
                ("r", PermRead),
                ("-", Punctuation),
                ("x", PermExec),
                ("3", Number),
                ("root", User),
                ("root", Group),
                ("4096", Size),
                ("Aug 19", Date),
                ("17", Time),
                (":", TimeSeparator),
                ("15", Time),
                ("..", Directory),
            ]
        );
        let file = coloured("-rw-r--r-- 1 dietpi dietpi  220 May  9 10:58 .bash_logout");
        assert_eq!(file[0], ("-", Punctuation));
        assert!(file.contains(&("dietpi", User)));
        assert!(file.contains(&("dietpi", Group)));
        assert!(file.contains(&("May  9", Date)));
        assert_eq!(file.last(), Some(&(".bash_logout", FileName)));
        assert_eq!(
            coloured("-rwxr-xr-x 1 root root 220 May  9 10:58 run.sh").last(),
            Some(&("run.sh", Executable))
        );
        let link = coloured("lrwxrwxrwx 1 root root 7 Aug 19  2023 bin -> usr/bin");
        assert_eq!(link[0], ("l", Symlink));
        assert!(link.contains(&("2023", Date)));
        assert!(link.contains(&("bin", Symlink)));
        assert!(link.contains(&("->", Punctuation)));
        assert_eq!(link.last(), Some(&("bin", PathSegment)));
        // BusyBox pads its columns wide and a device has a major and minor;
        // a mode string on its own is still read as one.
        let device = coloured("crw-rw-rw-    1 root     root        1,   3 Aug 19 17:15 null");
        assert_eq!(device[0], ("c", Special));
        assert!(device.contains(&("1,   3", Size)));
        assert_eq!(device.last(), Some(&("null", FileName)));
        assert_eq!(coloured("mode is -rw-r--r-- now")[1], ("r", PermRead));
    }

    #[test]
    fn a_prompt_and_its_command_line_are_read() {
        assert_eq!(
            coloured("dietpi@DietPi:~$ ls -la --color=auto"),
            vec![
                ("dietpi", User),
                ("@", EmailSymbol),
                ("DietPi", Host),
                (":", Punctuation),
                ("~", PathSegment),
                ("$", Prompt),
                ("-la", Flag),
                ("--color", Flag),
                ("=", KeySeparator),
            ]
        );
        let prompt = coloured("[root@board ~]# cat /etc/os-release");
        assert!(prompt.contains(&("root", User)));
        assert!(prompt.contains(&("board", Host)));
        assert!(prompt.contains(&("#", Prompt)));
        assert_eq!(
            coloured("sh: foo: command not found").last(),
            Some(&("command not found", Error))
        );
        // A clock without seconds is a clock; a minus before a digit is not a switch.
        assert_eq!(
            coloured("at 17:35 today"),
            vec![("17", Time), (":", TimeSeparator), ("35", Time)]
        );
        assert_eq!(coloured("rssi -67")[0], ("-67", Number));
    }

    #[test]
    fn a_row_read_once_is_remembered() {
        let highlighter = Highlighter::shared();
        let first = highlighter.roles("temp=25");
        let second = highlighter.roles("temp=25");
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(highlighter.roles("").len(), 0);
    }

    #[test]
    fn every_role_has_a_colour_in_both_themes() {
        for role in [Error, Number, Quote, Peripheral] {
            let dark = role.style(InterfaceTheme::Dark);
            let light = role.style(InterfaceTheme::Light);
            assert_ne!(dark.color, light.color);
        }
        assert!(Error.style(InterfaceTheme::Dark).bold);
        assert!(DurationUnit.style(InterfaceTheme::Light).italic);
        assert!(UrlHost.style(InterfaceTheme::Dark).underline);
        // A pill has a ground in each theme; everything else has none.
        assert_eq!(
            HttpGet.style(InterfaceTheme::Dark).background,
            Some(0x5be49b)
        );
        assert_eq!(
            HttpGet.style(InterfaceTheme::Light).background,
            Some(0x1f8a4c)
        );
        assert_eq!(Number.style(InterfaceTheme::Dark).background, None);
    }
}
