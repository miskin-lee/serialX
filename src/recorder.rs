//! Recording: the file a session keeps of what its device said.
//!
//! A session made with `Record` ticked opens a file of its own every time
//! its port opens, and writes what the device sends into it as it arrives.
//! One connection is one file: reconnecting after a board reset leaves the
//! last run where it was rather than growing one file all week, so a run
//! can be sent on by itself.
//!
//! What goes in is the bytes as the port read them, untouched — the escape
//! sequences and the colour the device sent along with the text. The log on
//! screen is the emulation's reading of those bytes; the file is the bytes
//! themselves, so a recording can be replayed, diffed or run through any
//! tool that expects what came off the wire. Nothing the workbench prints
//! is in it, and neither is what was typed at the device: a recording is
//! what the device said, and only that.
//!
//! Where they go is one folder, set in Settings and pointing at the
//! account's documents until it is set to something else. Under it the
//! recordings keep their own tree — `serialX`, then the day — so a folder
//! full of them is still a folder you can find yesterday's run in:
//!
//! ```text
//! ~/Documents/serialX/2026-09-09/Motor board-20260909-142312.log
//! ```

use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

/// The folder the recordings make for themselves inside the one the
/// settings name, so pointing the setting at the desktop leaves a single
/// `serialX` folder there rather than a year of loose days.
pub(crate) const RECORDINGS_FOLDER: &str = "serialX";
/// The day's folder, and the time in a recording's name. Neither carries a
/// character a file system objects to, and both sort as they read.
const DAY_FORMAT: &str = "%Y-%m-%d";
const TIME_FORMAT: &str = "%Y%m%d-%H%M%S";
/// What a recording is called when the session's name is nothing a file can
/// be named after.
const FALLBACK_NAME: &str = "session";
/// The most of a session's name a file name carries. A name longer than
/// this is cut rather than the whole path being refused by the platform.
const MAX_NAME: usize = 60;

/// The folder the recordings are written into, given what the settings say:
/// the folder that was chosen, or the account's documents until one is.
/// Nothing at all only on a machine that will not say where either is.
pub(crate) fn recordings_folder(chosen: Option<&Path>) -> Option<PathBuf> {
    let root = match chosen {
        Some(root) => root.to_path_buf(),
        None => default_root()?,
    };
    Some(root.join(RECORDINGS_FOLDER))
}

/// Where the recordings go when nothing has been chosen: the documents
/// folder the platform gives every account.
fn default_root() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Documents"))
    }

    #[cfg(windows)]
    {
        env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|home| home.join("Documents"))
    }

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        if let Some(documents) = env::var_os("XDG_DOCUMENTS_DIR") {
            return Some(PathBuf::from(documents));
        }
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Documents"))
    }
}

/// The file one connection writes into, open for as long as the port is.
pub(crate) struct Recorder {
    path: PathBuf,
    file: BufWriter<File>,
}

impl Recorder {
    /// Opens the file for a session connecting now: the day's folder under
    /// `folder`, made if it is not there, and in it the session's name and
    /// the time. Two connections within the same second write to the one
    /// file, one after the other, rather than the second wiping the first.
    pub(crate) fn start(folder: &Path, title: &str) -> io::Result<Self> {
        let now = chrono::Local::now();
        let path = recording_path(
            folder,
            title,
            &now.format(DAY_FORMAT).to_string(),
            &now.format(TIME_FORMAT).to_string(),
        );
        if let Some(day) = path.parent() {
            fs::create_dir_all(day)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file: BufWriter::new(file),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Takes what the port read. Buffered, since a device streaming is
    /// read in small pieces; the session flushes once a batch is in.
    pub(crate) fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Where a recording of `title` made on `day` at `time` is written.
fn recording_path(folder: &Path, title: &str, day: &str, time: &str) -> PathBuf {
    folder
        .join(day)
        .join(format!("{}-{time}.log", file_name(title)))
}

/// A session's name as a file can carry it: the last part of a device path,
/// since `/dev/cu.usbserial-1130` is known by its end rather than by the
/// two folders in front of it, with the characters a file system reserves
/// given up for a hyphen and a run of them read as one. Everything else a
/// name is made of is kept as it was typed — a space, a letter in Chinese
/// — since a recording is looked for by the name of the session that made
/// it.
fn file_name(title: &str) -> String {
    let tail = title
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(title);
    let mut name = String::with_capacity(tail.len());
    let mut given_up = false;
    for character in tail.chars() {
        if character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
            if !given_up && !name.is_empty() {
                name.push('-');
            }
            given_up = true;
        } else {
            name.push(character);
            given_up = false;
        }
    }
    // A name ending in a space or a full stop is one Windows will not
    // take, and one starting with a hyphen reads as a flag.
    let name = name.trim_matches(['-', '.', ' ']).to_string();
    if name.is_empty() {
        return FALLBACK_NAME.to_string();
    }
    match name.char_indices().nth(MAX_NAME) {
        Some((cut, _)) => name[..cut].trim_end_matches(['-', '.', ' ']).to_string(),
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::{RECORDINGS_FOLDER, file_name, recording_path, recordings_folder};
    use std::path::{Path, PathBuf};

    /// The chosen folder keeps the recordings' own tree inside it, so the
    /// setting can name the desktop without strewing days across it.
    #[test]
    fn the_recordings_keep_their_own_folder_under_the_one_chosen() {
        let chosen = PathBuf::from("/Users/someone/Desktop");
        assert_eq!(
            recordings_folder(Some(&chosen)),
            Some(PathBuf::from("/Users/someone/Desktop").join(RECORDINGS_FOLDER))
        );
        // With nothing chosen the folder is under the one the platform
        // gives the account for its documents.
        let default = recordings_folder(None).expect("a documents folder");
        assert!(default.ends_with(RECORDINGS_FOLDER));
        assert!(default.is_absolute());
    }

    /// A recording is filed under the day and named for its session and the
    /// time the connection opened.
    #[test]
    fn a_recording_is_filed_under_its_day_and_named_for_its_session() {
        let path = recording_path(
            Path::new("/logs/serialX"),
            "Motor board",
            "2026-09-09",
            "20260909-142312",
        );
        assert_eq!(
            path,
            PathBuf::from("/logs/serialX/2026-09-09/Motor board-20260909-142312.log")
        );
    }

    /// A device path is known by its end, what a file system reserves
    /// gives way to a hyphen, and everything else stands as it was typed.
    #[test]
    fn names_a_file_cannot_carry_are_made_into_ones_it_can() {
        assert_eq!(file_name("/dev/cu.usbserial-1130"), "cu.usbserial-1130");
        assert_eq!(file_name("COM3"), "COM3");
        assert_eq!(file_name("Motor board"), "Motor board");
        assert_eq!(file_name("电机板"), "电机板");
        assert_eq!(file_name("A:B*C?D"), "A-B-C-D");
        assert_eq!(file_name("a::b"), "a-b", "a run of them reads as one");
        assert_eq!(
            file_name("board "),
            "board",
            "Windows takes no trailing space"
        );
        assert_eq!(file_name("  ...  "), "session");
        assert_eq!(file_name(""), "session");
        assert_eq!(file_name("/dev/"), "dev");
    }

    /// A name longer than a file should carry is cut, and never left
    /// ending on the hyphen the cut fell on.
    #[test]
    fn a_long_name_is_cut_where_a_file_name_ends() {
        let long = "x".repeat(200);
        assert_eq!(file_name(&long).len(), 60);
        let cut_on_a_gap = format!("{} {}", "y".repeat(60), "z".repeat(10));
        assert_eq!(file_name(&cut_on_a_gap), "y".repeat(60));
    }
}
