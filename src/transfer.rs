//! Sending and receiving files: the dialog a transfer starts from, the
//! strip over the log while one runs, and what a tab keeps of it.
//!
//! `Send File…` and `Receive File…` in the Session menu open one dialog
//! with the direction set: the protocol, a segmented switch of the four
//! in [`crate::modem`], with a line under it saying what the choice
//! means; then the file to send, or the folder to receive into — and,
//! under XMODEM, which carries no name, the name the file is to get.
//! The protocol and the folder chosen last are kept with the settings, so
//! the second transfer opens on the first one's choices, and the folder
//! can be set in Settings as well; until a folder is chosen, received
//! files go to the account's downloads, and a name that is taken gets a
//! number after it.
//!
//! While a transfer runs a strip lies along the top of the log: which way
//! the file is going and its name, the protocol, a bar of how far it has
//! got with the bytes and the rate beside it, and `Cancel`. The port is
//! the transfer's meanwhile — keys and the composer go nowhere — and when
//! it is over the log says how: the files with their sizes and how long
//! they took, or why it stopped. ZMODEM needs no dialog when the device
//! begins: `sz` on the device starts a receive on its own, and `rz`
//! opens the dialog with ZMODEM chosen.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use gpui_kit::component::{
    Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::controls::{Choice, ChoiceText, dialog_footer, eyebrow, segmented, tag};
use crate::icons::{Glyph, icon_chip};
use crate::modem::{self, Direction, Link, Protocol, TransferEvent, TransferJob, Wire};
use crate::remote::{
    RemoteContext, RemoteLink, WriteOptions,
    browser::{Pick, open_remote_browser},
    remote_file_name, remote_join,
};
use crate::serial::{SerialCommand, SerialEvent, SerialTabState};
use crate::theme::{
    CAPTION, InterfaceTheme, LABEL, MICRO, MONO_SMALL, TITLE, Typography, WorkbenchPalette, tint,
};
use crate::{SerialWorkspace, format_bytes};

/// Width of the dialog: the four protocol names on one rail at the label
/// size, with air.
const DIALOG_WIDTH: f32 = 480.;
const FIELD_HEIGHT: f32 = 32.;
const SECTION_GAP: f32 = 16.;
/// Height of the strip over the log while a transfer runs.
const STRIP_HEIGHT: f32 = 30.;
/// The widest the file's name grows in the strip before it truncates.
const STRIP_NAME_MAX_WIDTH: f32 = 280.;
/// What the file field says with nothing chosen.
const NO_FILE: &str = "No file chosen";
/// The name an XMODEM file gets when none is typed.
const DEFAULT_NAME: &str = "received.bin";
/// The protocol the dialog opens on before one has been chosen: the one
/// that starts by itself, and the one a shell's `sz` and `rz` speak.
const DEFAULT_PROTOCOL: Protocol = Protocol::ZModem;
/// How long a file has to have been going before a rate is worth saying.
const RATE_AFTER: Duration = Duration::from_millis(500);

/// A transfer under way on a tab: what it is, how far it has got, and
/// the flag that stops it.
#[derive(Clone)]
pub(crate) struct Transfer {
    pub(crate) protocol: Protocol,
    pub(crate) direction: Direction,
    /// The file going now, once the protocol has said which.
    file: Option<String>,
    size: Option<u64>,
    done: u64,
    file_started: Instant,
    /// What the port had carried for the transfer at the last report, so
    /// the title bar's counters can be moved by the difference.
    wire: Wire,
    cancel: Arc<AtomicBool>,
    /// The folder on this machine a transfer on a host goes through.
    staging: Option<Staging>,
}

/// How a transfer on a host is staged through this machine: the protocol
/// engines read and write files here, and the connection carries them
/// the rest of the way.
#[derive(Clone)]
pub(crate) enum Staging {
    /// The files were fetched from the host into this folder, which goes
    /// with the transfer.
    Fetched { folder: PathBuf },
    /// Received files land in this folder and go up to `destination` on
    /// the host once the transfer is over.
    Landing {
        folder: PathBuf,
        destination: String,
        link: RemoteLink,
    },
}

/// A staged transfer that is over: what to do with the staging folder,
/// and the files that arrived in it.
pub(crate) struct StagingOutcome {
    staging: Staging,
    files: Vec<(String, u64)>,
}

/// A folder of this machine's for one transfer to go through.
fn staging_folder() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    env::temp_dir().join(format!(
        "serialX-transfer-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

impl Transfer {
    fn new(job: &TransferJob, cancel: Arc<AtomicBool>, staging: Option<Staging>) -> Self {
        Self {
            protocol: job.protocol(),
            direction: job.direction(),
            file: None,
            size: None,
            done: 0,
            file_started: Instant::now(),
            wire: Wire::default(),
            cancel,
            staging,
        }
    }

    /// Asks the transfer's thread to stop; it says so when it has.
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The share of the current file done, when its size is known.
    fn fraction(&self) -> Option<f32> {
        let size = self.size?;
        if size == 0 {
            return Some(1.);
        }
        Some((self.done as f64 / size as f64).min(1.) as f32)
    }

    fn rate(&self) -> Option<f64> {
        let elapsed = self.file_started.elapsed();
        (elapsed >= RATE_AFTER && self.done > 0).then(|| self.done as f64 / elapsed.as_secs_f64())
    }

    /// The first words of the strip: which way, and what.
    fn heading(&self) -> String {
        match (self.direction, &self.file) {
            (Direction::Send, Some(file)) => format!("Sending {file}"),
            (Direction::Receive, Some(file)) => format!("Receiving {file}"),
            (Direction::Send, None) => "Sending".into(),
            (Direction::Receive, None) => "Waiting for the device to send".into(),
        }
    }

    /// The figures: what is done of what, and how fast.
    fn figures(&self) -> String {
        let mut parts = vec![match self.size {
            Some(size) => format!("{} of {}", format_bytes(self.done), format_bytes(size)),
            None => format_bytes(self.done),
        }];
        if let Some(fraction) = self.fraction() {
            parts.push(format!("{:.0}%", (fraction * 100.).floor()));
        }
        if let Some(rate) = self.rate() {
            parts.push(format!("{}/s", format_bytes(rate as u64)));
        }
        parts.join(" · ")
    }
}

/// How long a transfer took, said the way a person would.
fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds == 0 {
        "under a second".into()
    } else if seconds < 60 {
        format!("{seconds} s")
    } else {
        format!("{} min {} s", seconds / 60, seconds % 60)
    }
}

impl SerialTabState {
    /// Takes what the transfer's thread reported: moves the strip along,
    /// keeps the title bar's counters honest about what the port carried,
    /// and when the run is over says so in the log and lets the port go.
    /// A staged transfer that is over hands back its staging, for the
    /// workbench to finish.
    pub(crate) fn transfer_event(&mut self, event: TransferEvent) -> Option<StagingOutcome> {
        // A transfer ended from this end — a disconnect — has nothing
        // left to hear.
        let Some(mut transfer) = self.transfer.take() else {
            return None;
        };
        let wire = match &event {
            TransferEvent::Progress { wire, .. }
            | TransferEvent::Finished { wire, .. }
            | TransferEvent::Failed { wire, .. } => Some(*wire),
            TransferEvent::File { .. } => None,
        };
        if let Some(wire) = wire {
            self.rx_bytes = self
                .rx_bytes
                .saturating_add(wire.read.saturating_sub(transfer.wire.read));
            self.tx_bytes = self
                .tx_bytes
                .saturating_add(wire.written.saturating_sub(transfer.wire.written));
            transfer.wire = wire;
        }
        let protocol = transfer.protocol.label();
        match event {
            TransferEvent::File { name, size } => {
                transfer.file = Some(name);
                transfer.size = size;
                transfer.done = 0;
                transfer.file_started = Instant::now();
                self.transfer = Some(transfer);
                None
            }
            TransferEvent::Progress { done, .. } => {
                transfer.done = done;
                self.transfer = Some(transfer);
                None
            }
            TransferEvent::Finished { files, elapsed, .. } => {
                let verb = match transfer.direction {
                    Direction::Send => "Sent",
                    Direction::Receive => "Received",
                };
                let what = if files.is_empty() {
                    "nothing".to_string()
                } else {
                    files
                        .iter()
                        .map(|(name, size)| format!("{name} ({})", format_bytes(*size)))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                self.note(format!(
                    "{verb} {what} over {protocol} in {}",
                    format_elapsed(elapsed)
                ));
                transfer
                    .staging
                    .map(|staging| StagingOutcome { staging, files })
            }
            TransferEvent::Failed {
                reason, cancelled, ..
            } => {
                let verb = match transfer.direction {
                    Direction::Send => "send",
                    Direction::Receive => "receive",
                };
                if cancelled {
                    self.note(format!("{protocol} {verb} cancelled"));
                } else {
                    self.note(format!("{protocol} {verb} failed: {reason}"));
                }
                transfer.staging.map(|staging| StagingOutcome {
                    staging,
                    files: Vec::new(),
                })
            }
        }
    }
}

/// Where received files go when no folder has been chosen: the downloads
/// folder the platform gives every account.
pub(crate) fn default_transfer_folder() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = env::var_os("HOME");

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    if let Some(downloads) = env::var_os("XDG_DOWNLOAD_DIR") {
        return Some(PathBuf::from(downloads));
    }

    home.map(PathBuf::from).map(|home| home.join("Downloads"))
}

impl SerialWorkspace {
    /// `Send File…` and `Receive File…`: the dialog, for the session in
    /// front. With nothing open there is nowhere for a file to go.
    pub(crate) fn open_transfer_dialog(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.active_tab().map(|tab| tab.id) else {
            return;
        };
        self.open_transfer_dialog_for(tab_id, direction, None, window, cx);
    }

    fn open_transfer_dialog_for(
        &mut self,
        tab_id: usize,
        direction: Direction,
        preset: Option<Protocol>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tab(tab_id) else {
            return;
        };
        let title = tab.title().to_string();
        let ready = tab.connected && tab.transfer.is_none();
        if !ready {
            let complaint = if tab.connected {
                "A transfer is already running on this session."
            } else {
                "Connect the serial port before sending or receiving a file."
            };
            if let Some(tab) = self.tab_mut(tab_id) {
                tab.note(complaint);
            }
            cx.notify();
            return;
        }

        let theme = self.interface_theme;
        let palette = theme.palette();
        let protocol = preset
            .or(self.presets.settings.transfer_protocol)
            .unwrap_or(DEFAULT_PROTOCOL);
        let folder = self.transfer_folder();
        let remote = self.remote_context();
        let on_host = remote.as_ref().map(|remote| remote.title.clone());
        let editor = cx.new(|cx| {
            TransferEditor::new(theme, direction, protocol, folder, remote, window, cx)
        });
        let workspace = cx.weak_entity();

        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let editor = editor.clone();
            let (glyph, heading, blurb, confirm) = match direction {
                Direction::Send => (
                    Glyph::Upload,
                    "Send a file",
                    format!(
                        "To {title}{}. The device has to be waiting for it: rz at a shell, or a bootloader's load command.",
                        on_host
                            .as_ref()
                            .map(|host| format!(", from the files on {host}"))
                            .unwrap_or_default()
                    ),
                    "Send",
                ),
                Direction::Receive => (
                    Glyph::Download,
                    "Receive a file",
                    format!(
                        "From {title}{}. The device has to be sending it: sz at a shell, or a dump command.",
                        on_host
                            .as_ref()
                            .map(|host| format!(", into a folder on {host}"))
                            .unwrap_or_default()
                    ),
                    "Receive",
                ),
            };
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
                .child(editor.clone())
                .footer(dialog_footer(palette, confirm, glyph, None))
                .on_ok(move |_, _, cx| {
                    // A dialog with nothing to send stays open and says
                    // so under the field.
                    let Ok(job) = editor.update(cx, |editor, cx| editor.job(cx)) else {
                        return false;
                    };
                    let _ = workspace.update(cx, |workspace, cx| {
                        workspace.start_transfer(tab_id, job, cx);
                    });
                    true
                })
        });
    }

    /// Where received files go: the folder chosen in Settings or in the
    /// last receive, else the default.
    fn transfer_folder(&self) -> PathBuf {
        self.presets
            .settings
            .transfer_folder
            .clone()
            .unwrap_or_else(|| self.default_receive_folder())
    }

    /// Where received files go when no folder has been chosen: the
    /// account's downloads, or — on a machine with none — the temporary
    /// folder. On a host, the host's downloads.
    fn default_receive_folder(&self) -> PathBuf {
        if let Some(info) = self.remote_info() {
            return PathBuf::from(&info.downloads);
        }
        default_transfer_folder().unwrap_or_else(env::temp_dir)
    }

    /// Runs a job on a tab's port. The choices are kept with the settings,
    /// so the next dialog opens on them. On a host the files are the
    /// host's: a send fetches them here first, a receive lands them here
    /// and sends them up after.
    pub(crate) fn start_transfer(&mut self, tab_id: usize, job: TransferJob, cx: &mut Context<Self>) {
        let mut settings = self.presets.settings.clone();
        settings.transfer_protocol = Some(job.protocol());
        // The default stays the default, so Settings does not show it as
        // a folder someone chose.
        if let TransferJob::Receive { folder, .. } = &job {
            settings.transfer_folder =
                (*folder != self.default_receive_folder()).then(|| folder.clone());
        }
        self.presets.set_settings(settings);

        let Some(tab) = self.tab_mut(tab_id) else {
            return;
        };
        if !tab.connected {
            tab.note("Connect the serial port before sending or receiving a file.");
            cx.notify();
            return;
        }
        if tab.transfer.is_some() {
            tab.note("A transfer is already running on this session.");
            cx.notify();
            return;
        }

        let Some(remote) = self.remote_context() else {
            self.launch_transfer(tab_id, job, None, cx);
            return;
        };
        match job {
            TransferJob::Send { protocol, paths } => {
                self.fetch_then_send(tab_id, protocol, paths, remote, cx);
            }
            TransferJob::Receive {
                protocol,
                folder,
                name,
            } => {
                let landing = staging_folder();
                if let Err(error) = fs::create_dir_all(&landing) {
                    if let Some(tab) = self.tab_mut(tab_id) {
                        tab.note(format!("Could not make a folder to receive into: {error}"));
                    }
                    cx.notify();
                    return;
                }
                let destination = folder.to_string_lossy().into_owned();
                self.launch_transfer(
                    tab_id,
                    TransferJob::Receive {
                        protocol,
                        folder: landing.clone(),
                        name,
                    },
                    Some(Staging::Landing {
                        folder: landing,
                        destination,
                        link: remote.link,
                    }),
                    cx,
                );
            }
        }
    }

    /// Takes the port's reader over through its tap, keeps the strip's
    /// state on the tab, and notes in the log that the job has begun.
    fn launch_transfer(
        &mut self,
        tab_id: usize,
        job: TransferJob,
        staging: Option<Staging>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tab_mut(tab_id) else {
            return;
        };
        if !tab.connected || tab.transfer.is_some() {
            tab.note("The transfer did not start: the session changed meanwhile.");
            cx.notify();
            return;
        }
        let Some(commands) = tab.command_tx.clone() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let events = tab.event_tx.clone();
        let leftover = events.clone();
        let link = Link::tapping(
            &tab.tap,
            move |bytes| {
                commands
                    .send(SerialCommand::Write(bytes.to_vec()))
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the port closed"))
            },
            cancel.clone(),
            move |bytes| {
                // What the device said once the transfer was over is
                // the console's again.
                if !bytes.is_empty() {
                    let _ = leftover.send_blocking(SerialEvent::Data(bytes));
                }
            },
        );
        tab.note(match (&job, &staging) {
            (TransferJob::Send { protocol, paths }, _) => format!(
                "Sending {} over {}…",
                paths
                    .iter()
                    .filter_map(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(", "),
                protocol.label()
            ),
            (
                TransferJob::Receive { protocol, .. },
                Some(Staging::Landing { destination, .. }),
            ) => format!(
                "Receiving over {} into {destination} on the host…",
                protocol.label()
            ),
            (
                TransferJob::Receive {
                    protocol, folder, ..
                },
                _,
            ) => format!(
                "Receiving over {} into {}…",
                protocol.label(),
                folder.display()
            ),
        });
        tab.transfer = Some(Transfer::new(&job, cancel, staging));
        modem::start(job, link, move |event| {
            let _ = events.send_blocking(SerialEvent::Transfer(event));
        });
        cx.notify();
    }

    /// Brings the host's files here, then sends them: the engines read
    /// files, and the host's are not files to them until they are here.
    fn fetch_then_send(
        &mut self,
        tab_id: usize,
        protocol: Protocol,
        paths: Vec<PathBuf>,
        remote: RemoteContext,
        cx: &mut Context<Self>,
    ) {
        let folder = staging_folder();
        if let Err(error) = fs::create_dir_all(&folder) {
            if let Some(tab) = self.tab_mut(tab_id) {
                tab.note(format!("Could not make a folder to fetch into: {error}"));
            }
            cx.notify();
            return;
        }
        let names: Vec<String> = paths
            .iter()
            .map(|path| remote_file_name(&path.to_string_lossy()).to_string())
            .collect();
        if let Some(tab) = self.tab_mut(tab_id) {
            tab.note(format!(
                "Fetching {} from {}…",
                names.join(", "),
                remote.title
            ));
        }
        cx.notify();

        let link = remote.link;
        cx.spawn(async move |this, cx| {
            let mut local = Vec::with_capacity(paths.len());
            let mut failure = None;
            for path in &paths {
                let on_host = path.to_string_lossy().into_owned();
                let target = folder.join(remote_file_name(&on_host));
                match link.read_file(&on_host).await {
                    Ok(bytes) => {
                        let written = {
                            let target = target.clone();
                            smol::unblock(move || fs::write(target, bytes)).await
                        };
                        if let Err(error) = written {
                            failure = Some(format!("{}: {error}", target.display()));
                            break;
                        }
                        local.push(target);
                    }
                    Err(error) => {
                        failure = Some(format!("{on_host}: {error}"));
                        break;
                    }
                }
            }
            let _ = this.update(cx, |this, cx| match failure {
                Some(reason) => {
                    let _ = fs::remove_dir_all(&folder);
                    if let Some(tab) = this.tab_mut(tab_id) {
                        tab.note(format!("Could not fetch from the host: {reason}"));
                    }
                    cx.notify();
                }
                None => this.launch_transfer(
                    tab_id,
                    TransferJob::Send {
                        protocol,
                        paths: local,
                    },
                    Some(Staging::Fetched { folder }),
                    cx,
                ),
            });
        })
        .detach();
    }

    /// A staged transfer is over: a fetched folder goes, and what landed
    /// goes up to the host and is said in the log by the name it got there.
    pub(crate) fn finish_staging(
        &mut self,
        tab_id: usize,
        outcome: StagingOutcome,
        cx: &mut Context<Self>,
    ) {
        let StagingOutcome { staging, files } = outcome;
        let (folder, destination, link) = match staging {
            Staging::Fetched { folder } => {
                thread::spawn(move || {
                    let _ = fs::remove_dir_all(folder);
                });
                return;
            }
            Staging::Landing {
                folder,
                destination,
                link,
            } => (folder, destination, link),
        };
        if files.is_empty() {
            thread::spawn(move || {
                let _ = fs::remove_dir_all(folder);
            });
            return;
        }
        if let Some(tab) = self.tab_mut(tab_id) {
            tab.note(format!(
                "Copying {} to {destination} on the host…",
                if files.len() == 1 {
                    "the file".to_string()
                } else {
                    format!("{} files", files.len())
                }
            ));
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut saved = Vec::new();
            let mut failures = Vec::new();
            for (name, _) in files {
                let local = folder.join(&name);
                let bytes = smol::unblock(move || fs::read(local)).await;
                let outcome = match bytes {
                    Ok(bytes) => {
                        link.write_file(
                            &remote_join(&destination, &name),
                            &bytes,
                            WriteOptions {
                                append: false,
                                mkdirs: true,
                                unique: true,
                            },
                        )
                        .await
                    }
                    Err(error) => Err(error.to_string()),
                };
                match outcome {
                    Ok(written) => saved.push(written.path),
                    Err(error) => failures.push(format!("{name}: {error}")),
                }
            }
            let _ = fs::remove_dir_all(&folder);
            let _ = this.update(cx, |this, cx| {
                if let Some(tab) = this.tab_mut(tab_id) {
                    if !saved.is_empty() {
                        tab.note(format!("Saved on the host: {}", saved.join(", ")));
                    }
                    for failure in failures {
                        tab.note(format!("Could not save on the host: {failure}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The strip's `Cancel`: the transfer's thread is asked to stop, and
    /// reports when it has; the strip goes then.
    pub(crate) fn cancel_transfer(&mut self, tab_id: usize, cx: &mut Context<Self>) {
        if let Some(transfer) = self.tab(tab_id).and_then(|tab| tab.transfer.as_ref()) {
            transfer.cancel();
        }
        cx.notify();
    }

    /// ZMODEM heard announcing itself on a tab's port. A device with a
    /// file to send is answered at once, into the usual folder; a device
    /// waiting for one gets the dialog, with ZMODEM chosen.
    pub(crate) fn zmodem_announced(
        &mut self,
        tab_id: usize,
        direction: Direction,
        cx: &mut Context<Self>,
    ) {
        match direction {
            Direction::Receive => {
                let folder = self.transfer_folder();
                if let Some(tab) = self.tab_mut(tab_id) {
                    tab.note("ZMODEM: the device has a file to send.");
                }
                self.start_transfer(
                    tab_id,
                    TransferJob::Receive {
                        protocol: Protocol::ZModem,
                        folder,
                        name: None,
                    },
                    cx,
                );
            }
            Direction::Send => {
                if let Some(tab) = self.tab_mut(tab_id) {
                    tab.note("ZMODEM: the device is waiting for a file.");
                }
                cx.notify();
                // The dialog needs the window, which the port's listener
                // does not have; the next turn of the loop does.
                let workspace = cx.weak_entity();
                cx.defer(move |cx| {
                    let _ = workspace.update_in(cx, |workspace, window, cx| {
                        workspace.open_transfer_dialog_for(
                            tab_id,
                            Direction::Send,
                            Some(Protocol::ZModem),
                            window,
                            cx,
                        );
                    });
                });
            }
        }
    }

    /// The strip over a log while its tab's transfer runs.
    pub(crate) fn render_transfer_strip(
        &self,
        tab_id: usize,
        transfer: &Transfer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.interface_theme.palette();
        let glyph = match transfer.direction {
            Direction::Send => Glyph::Upload,
            Direction::Receive => Glyph::Download,
        };
        let fraction = transfer.fraction();
        h_flex()
            .h(px(STRIP_HEIGHT))
            .flex_none()
            .w_full()
            .px_3()
            .gap_3()
            .items_center()
            .bg(rgb(palette.surface))
            .border_b_1()
            .border_color(rgb(palette.border_subtle))
            .child(
                Icon::new(glyph)
                    .size(px(14.))
                    .text_color(rgb(palette.accent)),
            )
            .child(
                div()
                    .min_w_0()
                    .max_w(px(STRIP_NAME_MAX_WIDTH))
                    .truncate()
                    .text_token(LABEL)
                    .text_color(rgb(palette.strong_foreground))
                    .child(transfer.heading()),
            )
            .child(tag(palette, palette.accent, MICRO, transfer.protocol.label()))
            // The bar: the file's share done, or an empty track while the
            // size is not known.
            .child(
                div()
                    .flex_1()
                    .min_w(px(40.))
                    .h(px(4.))
                    .rounded_full()
                    .bg(tint(palette.strong_foreground, 0.1))
                    .overflow_hidden()
                    .children(fraction.map(|fraction| {
                        div()
                            .h_full()
                            .rounded_full()
                            .bg(rgb(palette.accent))
                            .w(relative(fraction))
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .ui_mono_token(MONO_SMALL)
                    .text_color(rgb(palette.muted))
                    .whitespace_nowrap()
                    .child(transfer.figures()),
            )
            .child(
                Button::new(("transfer-cancel", tab_id))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .label("Cancel")
                    .tooltip("Stop the transfer")
                    .on_click(cx.listener(move |this, _, _, cx| this.cancel_transfer(tab_id, cx))),
            )
            .into_any_element()
    }
}

/// The transfer as it is being set up: the protocol, and the file or the
/// folder. Nothing runs until the dialog is confirmed.
struct TransferEditor {
    theme: InterfaceTheme,
    direction: Direction,
    protocol: Protocol,
    /// What was picked to send.
    paths: Vec<PathBuf>,
    /// Where a received file goes.
    folder: PathBuf,
    /// The host the files are on, while the workbench is on one: the
    /// picker is then the host's folders rather than the platform's.
    remote: Option<RemoteContext>,
    /// The name a received file gets, under a protocol that carries none.
    name_input: Entity<InputState>,
    _name_subscription: Subscription,
    /// Why the dialog would not confirm, after a try.
    complaint: Option<&'static str>,
}

impl TransferEditor {
    fn new(
        theme: InterfaceTheme,
        direction: Direction,
        protocol: Protocol,
        folder: PathBuf,
        remote: Option<RemoteContext>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder(DEFAULT_NAME));
        let name_subscription = cx.subscribe_in(
            &name_input,
            window,
            |editor: &mut Self, _, event: &InputEvent, _, cx: &mut Context<Self>| {
                if matches!(event, InputEvent::Change) {
                    editor.complaint = None;
                    cx.notify();
                }
            },
        );
        Self {
            theme,
            direction,
            protocol,
            paths: Vec::new(),
            folder,
            remote,
            name_input,
            _name_subscription: name_subscription,
            complaint: None,
        }
    }

    fn select_protocol(&mut self, protocol: Protocol, cx: &mut Context<Self>) {
        self.protocol = protocol;
        self.complaint = None;
        cx.notify();
    }

    /// Asks for the file, or files when the protocol takes several: the
    /// platform's picker here, the host's folders there. A cancelled pick
    /// keeps what was picked before.
    fn choose_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(remote) = self.remote.clone() {
            let start = self
                .paths
                .first()
                .and_then(|path| path.parent())
                .map(|parent| parent.to_string_lossy().into_owned())
                .filter(|parent| !parent.is_empty())
                .unwrap_or_else(|| remote.home.clone());
            let editor = cx.weak_entity();
            open_remote_browser(
                remote.link,
                Pick::Files {
                    several: self.protocol.batches(),
                },
                start,
                remote.home,
                self.theme,
                window,
                cx,
                move |paths, _, cx| {
                    let _ = editor.update(cx, |editor, cx| {
                        editor.paths = paths.into_iter().map(PathBuf::from).collect();
                        editor.complaint = None;
                        cx.notify();
                    });
                },
            );
            return;
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: self.protocol.batches(),
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |editor, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            if paths.is_empty() {
                return;
            }
            let _ = editor.update(cx, |editor, cx| {
                editor.paths = paths;
                editor.complaint = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks for the folder received files go to: the platform's picker
    /// here, the host's folders there.
    fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(remote) = self.remote.clone() {
            let start = self.folder.to_string_lossy().into_owned();
            let editor = cx.weak_entity();
            open_remote_browser(
                remote.link,
                Pick::Folder,
                start,
                remote.home,
                self.theme,
                window,
                cx,
                move |paths, _, cx| {
                    let Some(folder) = paths.into_iter().next() else {
                        return;
                    };
                    let _ = editor.update(cx, |editor, cx| {
                        editor.folder = PathBuf::from(folder);
                        cx.notify();
                    });
                },
            );
            return;
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |editor, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let _ = editor.update(cx, |editor, cx| {
                editor.folder = folder;
                cx.notify();
            });
        })
        .detach();
    }

    /// The job as set, or a complaint left under the field.
    fn job(&mut self, cx: &mut Context<Self>) -> Result<TransferJob, ()> {
        match self.direction {
            Direction::Send => {
                if self.paths.is_empty() {
                    self.complaint = Some("Choose a file to send.");
                    cx.notify();
                    return Err(());
                }
                Ok(TransferJob::Send {
                    protocol: self.protocol,
                    paths: self.paths.clone(),
                })
            }
            Direction::Receive => {
                let name = if self.protocol.carries_name() {
                    None
                } else {
                    let typed = self.name_input.read(cx).value().trim().to_string();
                    Some(if typed.is_empty() {
                        DEFAULT_NAME.to_string()
                    } else {
                        typed
                    })
                };
                Ok(TransferJob::Receive {
                    protocol: self.protocol,
                    folder: self.folder.clone(),
                    name,
                })
            }
        }
    }

    /// A section: its eyebrow, the control, and a line under it — what
    /// the control means, or why the dialog would not confirm.
    fn section(
        palette: WorkbenchPalette,
        label: &str,
        control: impl IntoElement,
        hint: Result<String, &'static str>,
    ) -> impl IntoElement {
        let (hint, color) = match hint {
            Ok(blurb) => (blurb, palette.muted),
            Err(complaint) => (complaint.to_string(), palette.danger),
        };
        v_flex()
            .gap_2()
            .child(eyebrow(palette, label))
            .child(control)
            .child(
                div()
                    .text_token(CAPTION)
                    .text_color(rgb(color))
                    .child(hint),
            )
    }

    /// A read-only field holding a path, whole in a tooltip when it is
    /// too long for the field.
    fn path_field(
        palette: WorkbenchPalette,
        id: &'static str,
        text: SharedString,
        placeholder: bool,
    ) -> impl IntoElement {
        let full = text.clone();
        h_flex()
            .id(id)
            .flex_1()
            .min_w_0()
            .h(px(FIELD_HEIGHT))
            .px_2p5()
            .items_center()
            .rounded(px(8.))
            .bg(rgb(palette.input))
            .border_1()
            .border_color(rgb(palette.input_border))
            .child(
                div()
                    .w_full()
                    .truncate()
                    .ui_mono_token(LABEL)
                    .text_color(rgb(if placeholder {
                        palette.faint
                    } else {
                        palette.strong_foreground
                    }))
                    .child(text),
            )
            .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
    }

    fn render_protocol(&self, palette: WorkbenchPalette, cx: &mut Context<Self>) -> impl IntoElement {
        let choices = Protocol::ALL
            .iter()
            .map(|&protocol| {
                Choice::new(
                    protocol.label(),
                    protocol == self.protocol,
                    cx.listener(move |editor, _, _, cx| editor.select_protocol(protocol, cx)),
                )
            })
            .collect();
        Self::section(
            palette,
            "Protocol",
            segmented("transfer-protocol", palette, ChoiceText::ui(LABEL), choices),
            Ok(self.protocol.describe().to_string()),
        )
    }

    fn render_files(&self, palette: WorkbenchPalette, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<String> = self
            .paths
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        let text: SharedString = if names.is_empty() {
            NO_FILE.into()
        } else {
            names.join(", ").into()
        };
        let several = names.len() > 1;
        let hint = match self.complaint {
            Some(complaint) => Err(complaint),
            None if several && !self.protocol.batches() => Ok(format!(
                "{} sends one file at a time; the first will go.",
                self.protocol.label()
            )),
            None if self.protocol.batches() => {
                Ok("One file, or several to go one after another.".to_string())
            }
            None => Ok("The file the device is waiting for.".to_string()),
        };
        Self::section(
            palette,
            if self.protocol.batches() { "Files" } else { "File" },
            h_flex()
                .items_center()
                .gap_2()
                .child(Self::path_field(
                    palette,
                    "transfer-files",
                    text,
                    names.is_empty(),
                ))
                .child(
                    Button::new("transfer-choose-files")
                        .outline()
                        .small()
                        .h(px(FIELD_HEIGHT))
                        .label("Choose…")
                        .when(self.remote.is_some(), |button| {
                            button.tooltip("Pick from the files on the host")
                        })
                        .on_click(cx.listener(|editor, _, window, cx| {
                            editor.choose_files(window, cx)
                        })),
                ),
            hint,
        )
    }

    fn render_folder(&self, palette: WorkbenchPalette, cx: &mut Context<Self>) -> impl IntoElement {
        Self::section(
            palette,
            if self.remote.is_some() {
                "Save to, on the host"
            } else {
                "Save to"
            },
            h_flex()
                .items_center()
                .gap_2()
                .child(Self::path_field(
                    palette,
                    "transfer-folder",
                    self.folder.display().to_string().into(),
                    false,
                ))
                .child(
                    Button::new("transfer-choose-folder")
                        .outline()
                        .small()
                        .h(px(FIELD_HEIGHT))
                        .label("Choose…")
                        .when(self.remote.is_some(), |button| {
                            button.tooltip("Pick a folder on the host")
                        })
                        .on_click(cx.listener(|editor, _, window, cx| {
                            editor.choose_folder(window, cx)
                        })),
                ),
            Ok(if self.protocol.carries_name() {
                "The file is written under the name the device sends; a name that is taken gets a number after it.".to_string()
            } else {
                "The file is written under the name below; a name that is taken gets a number after it.".to_string()
            }),
        )
    }

    fn render_name(&self, palette: WorkbenchPalette) -> impl IntoElement {
        // `Input::h` is the multi-line box's own and shadows the style, so
        // the frame is given its height through `Styled`.
        let field = Styled::h(
            Input::new(&self.name_input)
                .small()
                .ui_mono_token(LABEL)
                .bg(rgb(palette.input))
                .border_color(rgb(palette.input_border))
                .rounded(px(8.))
                .px_2p5(),
            px(FIELD_HEIGHT),
        );
        Self::section(
            palette,
            "File name",
            field,
            Ok(format!(
                "{} carries no name, so the file needs one from here. The last block is padded out with 0x1A, as the protocol does.",
                self.protocol.label()
            )),
        )
    }
}

impl Render for TransferEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        let protocol = self.render_protocol(palette, cx).into_any_element();
        let body = match self.direction {
            Direction::Send => vec![self.render_files(palette, cx).into_any_element()],
            Direction::Receive => {
                let mut body = vec![self.render_folder(palette, cx).into_any_element()];
                if !self.protocol.carries_name() {
                    body.push(self.render_name(palette).into_any_element());
                }
                body
            }
        };
        v_flex()
            .gap(px(SECTION_GAP))
            .child(protocol)
            .children(body)
    }
}

/// Whether a folder is somewhere a file can be written.
#[allow(dead_code)]
fn is_folder(path: &Path) -> bool {
    path.is_dir()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::format_elapsed;

    #[test]
    fn a_transfers_time_is_said_in_whole_units() {
        assert_eq!(format_elapsed(Duration::from_millis(400)), "under a second");
        assert_eq!(format_elapsed(Duration::from_secs(27)), "27 s");
        assert_eq!(format_elapsed(Duration::from_secs(63)), "1 min 3 s");
    }
}
