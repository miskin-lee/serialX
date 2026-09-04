use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    process::Command,
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

#[derive(Debug)]
pub(crate) enum UpdateEvent {
    CheckCompleted(Result<CheckResult, String>),
    DownloadProgress { downloaded: u64 },
    InstallerLaunched(Result<String, String>),
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
        let version = info.version.clone();
        let result = download_and_launch(&info, |downloaded| {
            let _ = events.send(UpdateEvent::DownloadProgress { downloaded });
        })
        .map(|()| version)
        .map_err(|error| error.to_string());
        let _ = events.send(UpdateEvent::InstallerLaunched(result));
    });
}

fn http_client() -> Result<Client, Box<dyn std::error::Error>> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(10 * 60))
        .user_agent(format!("serialX/{}", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn check_for_update() -> Result<CheckResult, Box<dyn std::error::Error>> {
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

    let asset_name = expected_asset_name(latest, std::env::consts::OS, std::env::consts::ARCH)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| format!("Release v{latest} 缺少当前系统的安装包 {asset_name}"))?;
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

fn is_newer_version(latest: &str, current: &str) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(Version::parse(latest)? > Version::parse(current)?)
}

fn expected_asset_name(
    version: &str,
    os: &str,
    arch: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let platform_suffix = match (os, arch) {
        ("macos", "aarch64") => "macos_aarch64.dmg",
        ("windows", "x86_64") => "windows_x86_64-setup.exe",
        ("linux", "x86_64") => "linux_amd64.deb",
        ("linux", "aarch64") => "linux_arm64.deb",
        _ => return Err(format!("暂不支持 {os}/{arch} 的自动更新").into()),
    };
    Ok(format!("serialX_{version}_{platform_suffix}"))
}

fn download_and_launch(
    info: &UpdateInfo,
    mut progress: impl FnMut(u64),
) -> Result<(), Box<dyn std::error::Error>> {
    let client = http_client()?;
    let expected_checksum = match &info.checksum {
        Some(checksum) if is_sha256(checksum) => checksum.to_ascii_lowercase(),
        _ => {
            let checksum_url = info
                .checksum_url
                .as_deref()
                .ok_or("Release 缺少 SHA256SUMS.txt，已拒绝安装")?;
            let manifest = client
                .get(checksum_url)
                .send()?
                .error_for_status()?
                .text()?;
            checksum_from_manifest(&manifest, &info.asset_name)
                .ok_or("校验清单中没有当前安装包，已拒绝安装")?
        }
    };

    let update_dir = std::env::temp_dir()
        .join("serialx-updates")
        .join(&info.version);
    fs::create_dir_all(&update_dir)?;
    let installer_path = update_dir.join(&info.asset_name);
    let partial_path = update_dir.join(format!("{}.part", info.asset_name));

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

    let actual_checksum = format!("{:x}", hasher.finalize());
    if actual_checksum != expected_checksum {
        let _ = fs::remove_file(&partial_path);
        return Err(format!(
            "安装包 SHA-256 校验失败（期望 {expected_checksum}，实际 {actual_checksum}）"
        )
        .into());
    }

    if installer_path.exists() {
        fs::remove_file(&installer_path)?;
    }
    fs::rename(partial_path, &installer_path)?;
    launch_installer(&installer_path)
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

#[cfg(target_os = "windows")]
fn launch_installer(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Command::new(path).spawn()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_installer(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Command::new("open").arg(path).spawn()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn launch_installer(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    match Command::new("xdg-open").arg(path).spawn() {
        Ok(_) => Ok(()),
        Err(first_error) => Command::new("gio")
            .arg("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|_| first_error.into()),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn launch_installer(_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Err("当前系统不支持自动打开安装包".into())
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
            expected_asset_name("1.2.3", "macos", "aarch64").unwrap(),
            "serialX_1.2.3_macos_aarch64.dmg"
        );
        assert_eq!(
            expected_asset_name("1.2.3", "windows", "x86_64").unwrap(),
            "serialX_1.2.3_windows_x86_64-setup.exe"
        );
        assert_eq!(
            expected_asset_name("1.2.3", "linux", "x86_64").unwrap(),
            "serialX_1.2.3_linux_amd64.deb"
        );
        assert!(expected_asset_name("1.2.3", "macos", "x86_64").is_err());
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
}
