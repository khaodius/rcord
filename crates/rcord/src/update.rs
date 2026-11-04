//! Updates: at each start rcord asks GitHub for the latest release, and when it is newer and
//! the person agrees, downloads the package for this install, checks it against the
//! release's SHA256SUMS.txt, installs it without any installer screens and restarts.
//!
//! How it installs depends on how rcord was installed:
//!
//! - **Windows, installed**: the setup program, run with `/VERYSILENT`. It closes rcord,
//!   upgrades in place and starts rcord again.
//! - **Windows, portable** and **Linux, per-user** (`~/.local/bin`): the archive is unpacked
//!   and its binaries replace the running ones by rename, which works while they run.
//! - **Linux, .deb or .rpm**: the package through apt, dnf, zypper or rpm, via `pkexec`, so the
//!   system asks for the password the way any package update does.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const REPO: &str = "khaodius/rcord";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub url: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// What the person sees after agreeing to update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// The new version is in place; start `exe` and quit.
    Restart(PathBuf),
    /// The Windows setup program is running and starts rcord itself once done; quit now.
    Quit,
}

fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(format!("rcord/{CURRENT}"))
        .connect_timeout(Duration::from_secs(10))
        .build()?)
}

/// `a` is newer than `b`, comparing dotted numbers (`0.1.10` is newer than `0.1.9`).
pub fn is_newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['.', '-', '+'])
            .map_while(|p| p.parse().ok())
            .collect()
    };
    parse(a) > parse(b)
}

/// The latest release, if it is newer than this build.
pub async fn check() -> anyhow::Result<Option<Release>> {
    let res = client()?
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        // No public release yet.
        return Ok(None);
    }
    let release: ApiRelease = res.error_for_status()?.json().await?;
    let version = release.tag_name.trim_start_matches('v').to_owned();
    if release.draft || release.prerelease || !is_newer(&version, CURRENT) {
        return Ok(None);
    }
    Ok(Some(Release {
        version,
        url: release.html_url,
        assets: release.assets,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Install {
    WindowsSetup,
    WindowsPortable,
    Deb,
    Rpm,
    LinuxUser,
}

fn detect(exe: &Path) -> Install {
    if cfg!(windows) {
        // The setup program writes unins000.exe next to rcord.exe.
        let installed = exe
            .parent()
            .is_some_and(|dir| dir.join("unins000.exe").exists());
        return if installed {
            Install::WindowsSetup
        } else {
            Install::WindowsPortable
        };
    }
    if exe.starts_with("/usr") {
        let owned = |cmd: &str, args: &[&str]| {
            Command::new(cmd)
                .args(args)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if owned("dpkg-query", &["-W", "rcord"]) {
            return Install::Deb;
        }
        if owned("rpm", &["-q", "rcord"]) {
            return Install::Rpm;
        }
    }
    Install::LinuxUser
}

fn asset_name(install: Install, version: &str) -> String {
    match install {
        Install::WindowsSetup => format!("rcord-{version}-x64-setup.exe"),
        Install::WindowsPortable => format!("rcord-{version}-x64-windows.zip"),
        Install::Deb => format!("rcord_{version}_amd64.deb"),
        Install::Rpm => format!("rcord-{version}-1.x86_64.rpm"),
        Install::LinuxUser => format!("rcord-{version}-x86_64-linux.tar.gz"),
    }
}

async fn download(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    Ok(client
        .get(url)
        .timeout(Duration::from_secs(600))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec())
}

fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        (file == name).then(|| hash.to_ascii_lowercase())
    })
}

/// Downloads and installs `release`. Blocking steps run on Tokio's blocking pool.
pub async fn apply(release: Release) -> anyhow::Result<Applied> {
    let exe = std::env::current_exe().context("could not find the running rcord")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let install = tokio::task::spawn_blocking({
        let exe = exe.clone();
        move || detect(&exe)
    })
    .await?;
    let name = asset_name(install, &release.version);
    let find = |n: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == n)
            .map(|a| a.browser_download_url.clone())
    };
    let url = find(&name).with_context(|| format!("the release has no {name}"))?;
    let sums_url = find("SHA256SUMS.txt").context("the release has no SHA256SUMS.txt")?;

    let client = client()?;
    let sums = String::from_utf8_lossy(&download(&client, &sums_url).await?).into_owned();
    let expected =
        expected_hash(&sums, &name).with_context(|| format!("no checksum for {name}"))?;
    let data = download(&client, &url).await?;
    let actual = format!("{:x}", Sha256::digest(&data));
    if actual != expected {
        bail!("the download of {name} did not match its checksum");
    }

    let dir = std::env::temp_dir().join(format!("rcord-update-{}", release.version));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(&name);
    std::fs::write(&file, &data)?;

    tokio::task::spawn_blocking(move || install_package(install, &file, &dir, &exe)).await?
}

fn run(cmd: &mut Command) -> anyhow::Result<()> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd
        .status()
        .with_context(|| format!("could not run {program}"))?;
    if !status.success() {
        bail!("{program} failed ({status})");
    }
    Ok(())
}

fn have(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Moves `new` over `target`, keeping the old file aside until the next start, so a binary
/// that is running can be replaced.
fn replace(new: &Path, target: &Path) -> anyhow::Result<()> {
    let old = target.with_extension("old");
    let _ = std::fs::remove_file(&old);
    if target.exists() {
        std::fs::rename(target, &old)?;
    }
    if let Err(e) = std::fs::copy(new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn install_package(
    install: Install,
    file: &Path,
    dir: &Path,
    exe: &Path,
) -> anyhow::Result<Applied> {
    let bin_dir = exe.parent().context("rcord has no folder")?;
    match install {
        Install::WindowsSetup => {
            // The setup program closes this rcord, upgrades it and starts the new one.
            Command::new(file)
                .args([
                    "/VERYSILENT",
                    "/SUPPRESSMSGBOXES",
                    "/NORESTART",
                    "/CLOSEAPPLICATIONS",
                    "/RESTARTRCORD=1",
                ])
                .spawn()
                .context("could not start the setup program")?;
            Ok(Applied::Quit)
        }
        Install::WindowsPortable => {
            let out = dir.join("unpacked");
            run(Command::new("powershell").args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                    file.display(),
                    out.display()
                ),
            ]))?;
            let unpacked = first_dir(&out)?;
            for name in ["rcord.exe", "rcord-webview.exe"] {
                let new = unpacked.join(name);
                if new.exists() {
                    replace(&new, &bin_dir.join(name))?;
                }
            }
            Ok(Applied::Restart(exe.to_owned()))
        }
        Install::LinuxUser => {
            run(Command::new("tar").arg("-xzf").arg(file).arg("-C").arg(dir))?;
            let unpacked = first_dir(dir)?;
            for name in ["rcord", "rcord-webview"] {
                let new = unpacked.join(name);
                if new.exists() {
                    replace(&new, &bin_dir.join(name))?;
                }
            }
            Ok(Applied::Restart(exe.to_owned()))
        }
        Install::Deb => {
            let mut cmd = Command::new("pkexec");
            if have("apt-get") {
                cmd.args([
                    "env",
                    "DEBIAN_FRONTEND=noninteractive",
                    "apt-get",
                    "install",
                    "-y",
                ]);
                cmd.arg(file);
            } else {
                cmd.args(["dpkg", "-i"]).arg(file);
            }
            // apt reads local packages as its own user, so the file must be readable to it.
            make_readable(dir, file);
            run(&mut cmd)?;
            Ok(Applied::Restart(exe.to_owned()))
        }
        Install::Rpm => {
            let mut cmd = Command::new("pkexec");
            if have("dnf") {
                cmd.args(["dnf", "install", "-y"]);
            } else if have("zypper") {
                cmd.args([
                    "zypper",
                    "--non-interactive",
                    "install",
                    "--allow-unsigned-rpm",
                ]);
            } else {
                cmd.args(["rpm", "-U"]);
            }
            cmd.arg(file);
            run(&mut cmd)?;
            Ok(Applied::Restart(exe.to_owned()))
        }
    }
}

fn make_readable(dir: &Path, file: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o644));
    }
    #[cfg(not(unix))]
    let _ = (dir, file);
}

/// The `rcord-<version>-…` folder an archive unpacks into.
fn first_dir(dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.is_dir()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("rcord-"))
        })
        .context("the archive has no rcord folder")
}

/// Removes what the previous update left behind: the replaced binaries and downloads.
pub fn clean_up() {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        for name in ["rcord", "rcord-webview"] {
            let _ = std::fs::remove_file(dir.join(format!("{name}.old")));
        }
    }
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for e in entries.filter_map(Result::ok) {
            if e.file_name().to_string_lossy().starts_with("rcord-update-") {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
    }

    #[test]
    fn finds_checksums() {
        let sums = "abc123  rcord_0.1.2_amd64.deb\nDEF456 *rcord-0.1.2-x64-setup.exe\n";
        assert_eq!(
            expected_hash(sums, "rcord-0.1.2-x64-setup.exe").as_deref(),
            Some("def456")
        );
        assert_eq!(expected_hash(sums, "missing"), None);
    }

    #[test]
    fn names_assets_like_the_release() {
        assert_eq!(
            asset_name(Install::LinuxUser, "0.1.2"),
            "rcord-0.1.2-x86_64-linux.tar.gz"
        );
        assert_eq!(
            asset_name(Install::Rpm, "0.1.2"),
            "rcord-0.1.2-1.x86_64.rpm"
        );
    }
}
