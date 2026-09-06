//! Software updates from GitHub Releases, applied in place.
//!
//! A check reads the latest release and picks the package built for this
//! system. Installing downloads that package under the system temporary
//! directory, verifies it against the SHA-256 the release publishes, and
//! then replaces the running copy where it stands: the macOS bundle is
//! swapped for the one inside the disk image, the Linux executable is
//! overwritten from the tarball or upgraded by `dpkg`, and on Windows the
//! installer or archive is applied by a helper once serialX has exited. The
//! new version runs the next time serialX starts — right away, when the user
//! asks to relaunch.

use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::Sender,
    thread,
    time::Duration,
};

use reqwest::{StatusCode, blocking::Client};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/miskin-lee/serialX/releases/latest";
const GITHUB_API_VERSION: &str = "2022-11-28";

type BoxError = Box<dyn std::error::Error>;

#[derive(Clone, Debug)]
pub(crate) struct UpdateInfo {
    pub version: String,
    pub asset_name: String,
    pub download_url: String,
    pub checksum: Option<String>,
    pub checksum_url: Option<String>,
}

#[derive(Debug)]
pub(crate) enum CheckResult {
    UpToDate { version: String },
    Available(UpdateInfo),
}

/// A newer serialX that is in place (macOS, Linux) or staged (Windows) and
/// runs the next time serialX starts.
#[derive(Clone, Debug)]
pub(crate) struct ReadyUpdate {
    pub version: String,
    relaunch: Relaunch,
}

impl ReadyUpdate {
    /// Where the update stands and what a relaunch does, for the dialog.
    pub fn summary(&self) -> String {
        if cfg!(windows) {
            format!(
                "serialX v{} is downloaded and verified. Relaunching installs it and starts the new version.",
                self.version
            )
        } else {
            format!(
                "serialX v{} is installed and runs the next time serialX opens.",
                self.version
            )
        }
    }
}

/// How to start the new version once this process has exited.
#[derive(Clone, Debug)]
enum Relaunch {
    #[cfg(target_os = "macos")]
    Bundle(PathBuf),
    #[cfg(target_os = "linux")]
    Executable(PathBuf),
    #[cfg(windows)]
    Installer {
        package: PathBuf,
        install_dir: PathBuf,
    },
    #[cfg(windows)]
    Archive {
        package: PathBuf,
        install_dir: PathBuf,
    },
}

/// Why an update could not be installed.
#[derive(Debug)]
pub(crate) struct InstallError {
    pub message: String,
    /// The downloaded and verified package, when the failure came after the
    /// download, so that the user can finish the update by hand.
    pub package: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) enum UpdateEvent {
    CheckCompleted(Result<CheckResult, String>),
    DownloadProgress {
        downloaded: u64,
    },
    /// The package is verified and is being applied.
    Installing,
    InstallCompleted(Result<ReadyUpdate, InstallError>),
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

pub(crate) fn spawn_update_check(events: Sender<UpdateEvent>) {
    thread::spawn(move || {
        let result = check_for_update().map_err(|error| error.to_string());
        let _ = events.send(UpdateEvent::CheckCompleted(result));
    });
}

pub(crate) fn spawn_update_install(info: UpdateInfo, events: Sender<UpdateEvent>) {
    thread::spawn(move || {
        let package = match download_package(&info, |downloaded| {
            let _ = events.send(UpdateEvent::DownloadProgress { downloaded });
        }) {
            Ok(package) => package,
            Err(error) => {
                let _ = events.send(UpdateEvent::InstallCompleted(Err(InstallError {
                    message: error.to_string(),
                    package: None,
                })));
                return;
            }
        };
        let _ = events.send(UpdateEvent::Installing);
        let result = install(&info, &package).map_err(|error| InstallError {
            message: error.to_string(),
            package: Some(package.clone()),
        });
        let _ = events.send(UpdateEvent::InstallCompleted(result));
    });
}

fn http_client() -> Result<Client, BoxError> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(10 * 60))
        .user_agent(format!("serialX/{}", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn check_for_update() -> Result<CheckResult, BoxError> {
    let client = http_client()?;
    let response = client
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .send()?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(CheckResult::UpToDate {
            version: env!("CARGO_PKG_VERSION").to_string(),
        });
    }
    let release: GitHubRelease = response.error_for_status()?.json()?;

    let latest = release.tag_name.trim_start_matches('v');
    if !is_newer_version(latest, env!("CARGO_PKG_VERSION"))? {
        return Ok(CheckResult::UpToDate {
            version: latest.to_string(),
        });
    }

    let asset_name = expected_asset_name(
        latest,
        env::consts::OS,
        env::consts::ARCH,
        is_portable_install(),
    )?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| format!("Release v{latest} does not include {asset_name}"))?;
    let checksum_url = release
        .assets
        .iter()
        .find(|asset| asset.name == "SHA256SUMS.txt")
        .map(|asset| asset.browser_download_url.clone());

    Ok(CheckResult::Available(UpdateInfo {
        version: latest.to_string(),
        asset_name,
        download_url: asset.browser_download_url.clone(),
        checksum: asset
            .digest
            .as_deref()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .map(str::to_owned),
        checksum_url,
    }))
}

fn is_newer_version(latest: &str, current: &str) -> Result<bool, BoxError> {
    Ok(Version::parse(latest)? > Version::parse(current)?)
}

/// The release package that updates this copy: the installer for a copy the
/// installer laid down, the portable archive for one that stands on its own.
fn expected_asset_name(
    version: &str,
    os: &str,
    arch: &str,
    portable: bool,
) -> Result<String, BoxError> {
    let platform_suffix = match (os, arch, portable) {
        ("macos", "aarch64", _) => "macos_aarch64.dmg",
        ("windows", "x86_64", false) => "windows_x86_64-setup.exe",
        ("windows", "x86_64", true) => "windows_x86_64.zip",
        ("linux", "x86_64", false) => "linux_amd64.deb",
        ("linux", "x86_64", true) => "linux_x86_64.tar.gz",
        ("linux", "aarch64", false) => "linux_arm64.deb",
        ("linux", "aarch64", true) => "linux_aarch64.tar.gz",
        _ => return Err(format!("Automatic updates are not supported on {os}/{arch}").into()),
    };
    Ok(format!("serialX_{version}_{platform_suffix}"))
}

/// Whether this copy of serialX stands on its own — a portable archive, a
/// local build — rather than having been laid down by the platform installer.
/// The installer leaves `Uninstall.exe` beside the executable on Windows; the
/// Debian package puts the executable at `/usr/bin/serialx`.
fn is_portable_install() -> bool {
    let Ok(executable) = env::current_exe() else {
        return true;
    };
    #[cfg(windows)]
    {
        !executable.with_file_name("Uninstall.exe").exists()
    }
    #[cfg(target_os = "linux")]
    {
        executable != Path::new("/usr/bin/serialx")
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = executable;
        false
    }
}

fn update_directory(version: &str) -> PathBuf {
    env::temp_dir().join("serialx-updates").join(version)
}

/// Downloads the package and verifies it, reusing an earlier download when
/// it is intact. Returns the verified package.
fn download_package(info: &UpdateInfo, mut progress: impl FnMut(u64)) -> Result<PathBuf, BoxError> {
    let client = http_client()?;
    let expected_checksum = expected_checksum(&client, info)?;

    let update_dir = update_directory(&info.version);
    fs::create_dir_all(&update_dir)?;
    let package_path = update_dir.join(&info.asset_name);
    let partial_path = update_dir.join(format!("{}.part", info.asset_name));

    if package_path.is_file() && file_sha256(&package_path)? == expected_checksum {
        progress(fs::metadata(&package_path)?.len());
        return Ok(package_path);
    }

    let mut response = client.get(&info.download_url).send()?.error_for_status()?;
    let mut output = File::create(&partial_path)?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        downloaded += count as u64;
        progress(downloaded);
    }
    output.sync_all()?;
    drop(output);

    let actual_checksum = format!("{:x}", hasher.finalize());
    if actual_checksum != expected_checksum {
        let _ = fs::remove_file(&partial_path);
        return Err(format!(
            "Package SHA-256 verification failed (expected {expected_checksum}, got {actual_checksum})"
        )
        .into());
    }

    if package_path.exists() {
        fs::remove_file(&package_path)?;
    }
    fs::rename(partial_path, &package_path)?;
    Ok(package_path)
}

fn expected_checksum(client: &Client, info: &UpdateInfo) -> Result<String, BoxError> {
    match &info.checksum {
        Some(checksum) if is_sha256(checksum) => Ok(checksum.to_ascii_lowercase()),
        _ => {
            let checksum_url = info
                .checksum_url
                .as_deref()
                .ok_or("The release has no SHA256SUMS.txt; installation was refused")?;
            let manifest = client
                .get(checksum_url)
                .send()?
                .error_for_status()?
                .text()?;
            Ok(checksum_from_manifest(&manifest, &info.asset_name).ok_or(
                "The checksum manifest does not list this package; installation was refused",
            )?)
        }
    }
}

fn file_sha256(path: &Path) -> Result<String, BoxError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn checksum_from_manifest(manifest: &str, asset_name: &str) -> Option<String> {
    manifest.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let checksum = fields.next()?;
        let filename = fields.next()?.trim_start_matches('*');
        (filename == asset_name && is_sha256(checksum)).then(|| checksum.to_ascii_lowercase())
    })
}

/// Runs a system tool to completion and returns what it printed, or what it
/// complained about.
fn run<I, S>(program: &str, args: I) -> Result<String, BoxError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Unable to run {program}: {error}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    Err(if detail.is_empty() {
        format!("{program} failed ({})", output.status).into()
    } else {
        format!("{program} failed ({}): {detail}", output.status).into()
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn remove_existing(path: &Path) -> Result<(), BoxError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

// ---------------------------------------------------------------- macOS ---

#[cfg(target_os = "macos")]
fn install(info: &UpdateInfo, package: &Path) -> Result<ReadyUpdate, BoxError> {
    let bundle = running_bundle()?;
    replace_bundle(package, &bundle)?;
    Ok(ReadyUpdate {
        version: info.version.clone(),
        relaunch: Relaunch::Bundle(bundle),
    })
}

#[cfg(target_os = "macos")]
fn running_bundle() -> Result<PathBuf, BoxError> {
    let executable = env::current_exe()?;
    bundle_containing(&executable).ok_or_else(|| {
        format!(
            "serialX is running from {} rather than an application bundle, so it cannot replace itself",
            executable.display()
        )
        .into()
    })
}

/// The `.app` that `executable` is the main executable of — it sits at
/// `<name>.app/Contents/MacOS/<executable>` — or `None` for a bare binary.
#[cfg(any(target_os = "macos", test))]
fn bundle_containing(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

/// Puts the application inside `disk_image` where `bundle` stands. The copy
/// is made beside the bundle first, so that the swap itself is two renames
/// on one volume, and the old bundle is removed once the new one is in
/// place. The running process keeps its mapped executable either way.
#[cfg(target_os = "macos")]
fn replace_bundle(disk_image: &Path, bundle: &Path) -> Result<(), BoxError> {
    let parent = bundle
        .parent()
        .ok_or("The application bundle has no parent directory")?;
    let name = bundle
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or("The application bundle has no name")?;
    let staged = parent.join(format!(".{name}.update"));
    let retired = parent.join(format!("{name}.previous"));

    let image = MountedImage::attach(disk_image)?;
    let source = image.application()?;
    remove_existing(&staged)?;
    run("ditto", [source.as_os_str(), staged.as_os_str()]).map_err(|error| {
        format!(
            "Unable to copy the new version into {}: {error}",
            parent.display()
        )
    })?;
    drop(image);

    // serialX downloaded the image itself, so nothing is quarantined; make
    // sure of it, or Gatekeeper would refuse the unnotarised bundle.
    let _ = run(
        "xattr",
        [
            OsStr::new("-dr"),
            OsStr::new("com.apple.quarantine"),
            staged.as_os_str(),
        ],
    );

    remove_existing(&retired)?;
    fs::rename(bundle, &retired).map_err(|error| {
        let _ = fs::remove_dir_all(&staged);
        format!("Unable to move {} aside: {error}", bundle.display())
    })?;
    if let Err(error) = fs::rename(&staged, bundle) {
        let _ = fs::rename(&retired, bundle);
        let _ = fs::remove_dir_all(&staged);
        return Err(format!(
            "Unable to move the new version into {}: {error}",
            bundle.display()
        )
        .into());
    }
    let _ = fs::remove_dir_all(&retired);
    Ok(())
}

/// A disk image attached for the duration; dropping it detaches the image.
#[cfg(target_os = "macos")]
struct MountedImage {
    mount_point: PathBuf,
}

#[cfg(target_os = "macos")]
impl MountedImage {
    fn attach(image: &Path) -> Result<Self, BoxError> {
        let report = run(
            "hdiutil",
            [
                OsStr::new("attach"),
                OsStr::new("-nobrowse"),
                OsStr::new("-readonly"),
                OsStr::new("-noverify"),
                OsStr::new("-noautoopen"),
                OsStr::new("-plist"),
                image.as_os_str(),
            ],
        )?;
        let mount_point = mount_point_from_plist(&report)
            .ok_or("hdiutil did not report where the disk image is mounted")?;
        Ok(Self { mount_point })
    }

    /// The application bundle at the top of the image.
    fn application(&self) -> Result<PathBuf, BoxError> {
        let mut bundles: Vec<PathBuf> = fs::read_dir(&self.mount_point)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension() == Some(OsStr::new("app"))
                    && path.join("Contents/Info.plist").is_file()
            })
            .collect();
        bundles.sort();
        bundles
            .into_iter()
            .next()
            .ok_or_else(|| "The disk image does not contain an application".into())
    }
}

#[cfg(target_os = "macos")]
impl Drop for MountedImage {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet"])
            .arg(&self.mount_point)
            .stdin(Stdio::null())
            .output();
    }
}

/// The `mount-point` that `hdiutil attach -plist` reports for the mounted
/// file system.
#[cfg(any(target_os = "macos", test))]
fn mount_point_from_plist(plist: &str) -> Option<PathBuf> {
    let (_, rest) = plist.split_once("<key>mount-point</key>")?;
    let (_, rest) = rest.split_once("<string>")?;
    let (path, _) = rest.split_once("</string>")?;
    let path = path
        .trim()
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    Some(PathBuf::from(path))
}

#[cfg(target_os = "macos")]
pub(crate) fn relaunch(update: &ReadyUpdate) -> Result<(), String> {
    let Relaunch::Bundle(bundle) = &update.relaunch;
    spawn_unix_helper([OsStr::new("open"), OsStr::new("-n"), bundle.as_os_str()])
}

#[cfg(target_os = "macos")]
pub(crate) fn open_package(package: &Path) -> Result<(), String> {
    spawn_detached(Command::new("open").arg(package))
}

// ---------------------------------------------------------------- Linux ---

#[cfg(target_os = "linux")]
fn install(info: &UpdateInfo, package: &Path) -> Result<ReadyUpdate, BoxError> {
    let executable = env::current_exe()?;
    if is_portable_install() {
        replace_executable(package, &executable, &info.version)?;
    } else {
        // dpkg unpacks beside the old file and renames it into place, so the
        // running process keeps its executable; pkexec asks for the password.
        run(
            "pkexec",
            [OsStr::new("dpkg"), OsStr::new("-i"), package.as_os_str()],
        )?;
    }
    Ok(ReadyUpdate {
        version: info.version.clone(),
        relaunch: Relaunch::Executable(executable),
    })
}

/// Overwrites `executable` with the one inside `tarball`. The copy is made
/// beside it first, so that the swap is one rename, which Linux allows over
/// a running program.
#[cfg(target_os = "linux")]
fn replace_executable(tarball: &Path, executable: &Path, version: &str) -> Result<(), BoxError> {
    use std::os::unix::fs::PermissionsExt;

    let unpack_dir = tarball
        .parent()
        .ok_or("The package has no parent directory")?
        .join("unpacked");
    remove_existing(&unpack_dir)?;
    fs::create_dir_all(&unpack_dir)?;
    run(
        "tar",
        [
            OsStr::new("-xzf"),
            tarball.as_os_str(),
            OsStr::new("-C"),
            unpack_dir.as_os_str(),
        ],
    )?;
    let source = unpack_dir
        .join(format!("serialX-{version}-linux-{}", env::consts::ARCH))
        .join("bin/serialx");
    if !source.is_file() {
        return Err(format!("The archive does not contain {}", source.display()).into());
    }

    let directory = executable
        .parent()
        .ok_or("The executable has no parent directory")?;
    let staged = directory.join(".serialx.update");
    fs::copy(&source, &staged).map_err(|error| {
        format!(
            "Unable to copy the new version into {}: {error}",
            directory.display()
        )
    })?;
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
    fs::rename(&staged, executable).map_err(|error| {
        let _ = fs::remove_file(&staged);
        format!("Unable to replace {}: {error}", executable.display())
    })?;
    let _ = fs::remove_dir_all(&unpack_dir);
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn relaunch(update: &ReadyUpdate) -> Result<(), String> {
    let Relaunch::Executable(executable) = &update.relaunch;
    spawn_unix_helper([executable.as_os_str()])
}

#[cfg(target_os = "linux")]
pub(crate) fn open_package(package: &Path) -> Result<(), String> {
    spawn_detached(Command::new("xdg-open").arg(package))
}

// ----------------------------------------------------------------- Unix ---

/// Starts a shell that outlives this process, waits for it to exit (a minute
/// at most) and then runs `launch`. Its own process group keeps it clear of
/// anything that signals ours.
#[cfg(unix)]
fn spawn_unix_helper<'a>(launch: impl IntoIterator<Item = &'a OsStr>) -> Result<(), String> {
    const SCRIPT: &str = "\
i=0
while kill -0 \"$1\" 2>/dev/null && [ \"$i\" -lt 300 ]; do sleep 0.2; i=$((i + 1)); done
shift
exec \"$@\"";
    spawn_detached(
        Command::new("/bin/sh")
            .arg("-c")
            .arg(SCRIPT)
            .arg("serialx-relaunch")
            .arg(std::process::id().to_string())
            .args(launch),
    )
}

#[cfg(unix)]
fn spawn_detached(command: &mut Command) -> Result<(), String> {
    use std::os::unix::process::CommandExt;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
        .map_err(|error| format!("Unable to start the helper: {error}"))
}

// -------------------------------------------------------------- Windows ---

/// Nothing can replace a running executable on Windows, so the package is
/// only checked here; `relaunch` applies it once serialX has exited.
#[cfg(windows)]
fn install(info: &UpdateInfo, package: &Path) -> Result<ReadyUpdate, BoxError> {
    let executable = env::current_exe()?;
    let install_dir = executable
        .parent()
        .ok_or("The executable has no parent directory")?
        .to_path_buf();
    let package = package.to_path_buf();
    let relaunch = if is_portable_install() {
        Relaunch::Archive {
            package,
            install_dir,
        }
    } else {
        Relaunch::Installer {
            package,
            install_dir,
        }
    };
    Ok(ReadyUpdate {
        version: info.version.clone(),
        relaunch,
    })
}

/// Starts a hidden PowerShell that waits for this process to exit, applies
/// the package — the installer silently into the same directory, or the
/// archive's files over it — and starts serialX again.
#[cfg(windows)]
pub(crate) fn relaunch(update: &ReadyUpdate) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let (package, install_dir, apply) = match &update.relaunch {
        Relaunch::Installer {
            package,
            install_dir,
        } => (
            package,
            install_dir,
            format!(
                "Start-Process -FilePath {} -ArgumentList @('/S', {}) -Wait",
                powershell_literal(package),
                powershell_literal_string(&format!("/D={}", install_dir.display())),
            ),
        ),
        Relaunch::Archive {
            package,
            install_dir,
        } => (
            package,
            install_dir,
            format!(
                "$stage = {stage}\n\
                 if (Test-Path -LiteralPath $stage) {{ Remove-Item -LiteralPath $stage -Recurse -Force }}\n\
                 Expand-Archive -LiteralPath {package} -DestinationPath $stage -Force\n\
                 Copy-Item -Path (Join-Path $stage 'serialX\\*') -Destination {install_dir} -Recurse -Force\n\
                 Remove-Item -LiteralPath $stage -Recurse -Force",
                stage = powershell_literal(&package.with_file_name("unpacked")),
                package = powershell_literal(package),
                install_dir = powershell_literal(install_dir),
            ),
        ),
    };
    let script = format!(
        "$ErrorActionPreference = 'Stop'\n\
         Wait-Process -Id {pid} -ErrorAction SilentlyContinue\n\
         Start-Sleep -Milliseconds 500\n\
         {apply}\n\
         Start-Process -FilePath {executable} -WorkingDirectory {install_dir}\n",
        pid = std::process::id(),
        executable = powershell_literal(&install_dir.join("serialX.exe")),
        install_dir = powershell_literal(install_dir),
    );
    let script_path = package.with_file_name("relaunch.ps1");
    fs::write(&script_path, script)
        .map_err(|error| format!("Unable to write the relaunch script: {error}"))?;

    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(drop)
        .map_err(|error| format!("Unable to start the helper: {error}"))
}

#[cfg(windows)]
fn powershell_literal(path: &Path) -> String {
    powershell_literal_string(&path.display().to_string())
}

#[cfg(windows)]
fn powershell_literal_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
pub(crate) fn open_package(package: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", package.display()))
        .spawn()
        .map(drop)
        .map_err(|error| format!("Unable to open the package: {error}"))
}

// ---------------------------------------------------------------- Other ---

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn install(_info: &UpdateInfo, _package: &Path) -> Result<ReadyUpdate, BoxError> {
    Err("Automatic updates are not supported on this system".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub(crate) fn relaunch(_update: &ReadyUpdate) -> Result<(), String> {
    Err("Automatic updates are not supported on this system".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub(crate) fn open_package(_package: &Path) -> Result<(), String> {
    Err("This system cannot open the update package automatically".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semantic_versions() {
        assert!(is_newer_version("1.2.0", "1.1.9").unwrap());
        assert!(!is_newer_version("1.2.0", "1.2.0").unwrap());
        assert!(!is_newer_version("1.2.0-beta.1", "1.2.0").unwrap());
    }

    #[test]
    fn selects_release_assets() {
        assert_eq!(
            expected_asset_name("1.2.3", "macos", "aarch64", false).unwrap(),
            "serialX_1.2.3_macos_aarch64.dmg"
        );
        assert_eq!(
            expected_asset_name("1.2.3", "windows", "x86_64", false).unwrap(),
            "serialX_1.2.3_windows_x86_64-setup.exe"
        );
        assert_eq!(
            expected_asset_name("1.2.3", "windows", "x86_64", true).unwrap(),
            "serialX_1.2.3_windows_x86_64.zip"
        );
        assert_eq!(
            expected_asset_name("1.2.3", "linux", "x86_64", false).unwrap(),
            "serialX_1.2.3_linux_amd64.deb"
        );
        assert_eq!(
            expected_asset_name("1.2.3", "linux", "aarch64", true).unwrap(),
            "serialX_1.2.3_linux_aarch64.tar.gz"
        );
        assert!(expected_asset_name("1.2.3", "macos", "x86_64", false).is_err());
    }

    #[test]
    fn parses_gnu_checksum_manifest() {
        let checksum = "a".repeat(64);
        let manifest = format!("{checksum}  other.zip\n{checksum} *serialX_1.2.3.dmg\n");
        assert_eq!(
            checksum_from_manifest(&manifest, "serialX_1.2.3.dmg"),
            Some(checksum)
        );
        assert_eq!(checksum_from_manifest(&manifest, "missing.dmg"), None);
    }

    #[test]
    fn finds_the_bundle_around_its_executable() {
        assert_eq!(
            bundle_containing(Path::new(
                "/Applications/serialX.app/Contents/MacOS/serialx"
            )),
            Some(PathBuf::from("/Applications/serialX.app"))
        );
        assert_eq!(
            bundle_containing(Path::new("/Users/me/serialX/target/debug/serialx")),
            None
        );
        assert_eq!(
            bundle_containing(Path::new("/Volumes/Image/serialX.app/serialx")),
            None
        );
    }

    #[test]
    fn reads_the_mount_point_hdiutil_reports() {
        let plist = "<plist version=\"1.0\"><dict><key>system-entities</key><array>\
            <dict><key>content-hint</key><string>GUID_partition_scheme</string></dict>\
            <dict><key>content-hint</key><string>Apple_APFS</string>\
            <key>mount-point</key><string>/Volumes/serialX 0.1.0 &amp; more</string></dict>\
            </array></dict></plist>";
        assert_eq!(
            mount_point_from_plist(plist),
            Some(PathBuf::from("/Volumes/serialX 0.1.0 & more"))
        );
        assert_eq!(mount_point_from_plist("<plist/>"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn swaps_the_bundle_for_the_one_in_the_disk_image() {
        let root = env::temp_dir().join(format!("serialx-updater-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let installed = root.join("Applications/serialX.app");
        write_bundle(&installed, "old");
        let image_source = root.join("image");
        write_bundle(&image_source.join("serialX.app"), "new");
        let image = root.join("serialX.dmg");
        let volume_name = format!("serialX updater test {}", std::process::id());
        run(
            "hdiutil",
            [
                OsStr::new("create"),
                OsStr::new("-quiet"),
                OsStr::new("-srcfolder"),
                image_source.as_os_str(),
                OsStr::new("-volname"),
                OsStr::new(&volume_name),
                OsStr::new("-format"),
                OsStr::new("UDZO"),
                image.as_os_str(),
            ],
        )
        .unwrap();

        replace_bundle(&image, &installed).unwrap();

        assert_eq!(
            fs::read_to_string(installed.join("Contents/MacOS/serialx")).unwrap(),
            "new"
        );
        assert!(!root.join("Applications/.serialX.app.update").exists());
        assert!(!root.join("Applications/serialX.app.previous").exists());
        assert!(!Path::new("/Volumes").join(&volume_name).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "macos")]
    fn write_bundle(bundle: &Path, marker: &str) {
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(bundle.join("Contents/Info.plist"), "<plist/>").unwrap();
        fs::write(bundle.join("Contents/MacOS/serialx"), marker).unwrap();
    }
}
