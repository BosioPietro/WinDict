//! Updates. Once a day the latest release on GitHub is looked up; when it's
//! newer, the tray icon and menu say so, and "Update app" installs it.
//!
//! The portable edition downloads the new executable next to itself, starts
//! it and quits; the new one waits for it to exit and deletes it. The
//! installed edition can't write to Program Files: it runs the scheduled
//! task its installer registered, which runs the new setup elevated without
//! asking (see installer/update.ps1).

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use windows::Win32::Foundation::{HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{WaitForSingleObject, CREATE_NO_WINDOW};

use crate::dicts;
use crate::http;
use crate::i18n::{fill, tr};
use crate::installed;

const API_HOST: &str = "api.github.com";
const LATEST: &str = "/repos/BosioPietro/WinDict/releases/latest";
/// Opened when the installed edition can't update itself.
pub const RELEASES_PAGE: &str = "https://github.com/BosioPietro/WinDict/releases/latest";
/// The scheduled task that installs the new version.
const TASK: &str = "WinDict Update";
/// What the new executable is started with: `--replace <pid> <path>`.
const REPLACE: &str = "--replace";
/// Anything bigger isn't a WinDict executable.
const MAX_SIZE: u64 = 64 * 1024 * 1024;
/// How long the new executable waits for the old one to exit.
const EXIT_WAIT_MS: u32 = 30_000;

/// A newer version than this one, and its executable.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Like "v0.21.0".
    pub tag: String,
    exe: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
struct Latest {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    /// "sha256:<hex>", which GitHub computes on upload.
    #[serde(default)]
    digest: Option<String>,
}

/// Looks up the latest release: `Some` if it's newer than this version.
pub fn check() -> Result<Option<Release>, String> {
    let (status, body) = http::get(API_HOST, LATEST)?;
    if status != 200 {
        return Err(fill(tr().download_status, &[("status", &status)]));
    }
    parse(&body, env!("CARGO_PKG_VERSION"), Edition::current())
}

/// How this copy of WinDict was installed, which decides what updates it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Edition {
    /// An executable run from wherever it was downloaded.
    Portable,
    /// By the setup, in Program Files.
    Installed,
}

impl Edition {
    fn current() -> Edition {
        if installed::dir().is_some() {
            Edition::Installed
        } else {
            Edition::Portable
        }
    }

    /// The release file that updates it.
    fn file(self, tag: &str) -> String {
        match self {
            Edition::Portable => format!("WinDict-{tag}.exe"),
            Edition::Installed => format!("WinDict-{tag}-setup.exe"),
        }
    }
}

/// Whether "Update app" runs the update task, rather than replacing this
/// executable itself.
pub fn by_task() -> bool {
    Edition::current() != Edition::Portable
}

/// The release in the GitHub API's `json`, if it's newer than `current` and
/// has the file that updates `edition`, with a digest to check it by.
fn parse(json: &[u8], current: &str, edition: Edition) -> Result<Option<Release>, String> {
    let latest: Latest =
        serde_json::from_slice(json).map_err(|_| tr().download_damaged.to_string())?;
    let (Some(latest_version), Some(current)) = (version(&latest.tag_name), version(current))
    else {
        return Ok(None);
    };
    if latest_version <= current {
        return Ok(None);
    }
    let tag = latest.tag_name;
    let exe = edition.file(&tag);
    let release = latest.assets.into_iter().find_map(|asset| {
        let sha256 = asset.digest?.strip_prefix("sha256:")?.to_ascii_lowercase();
        let hex = sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit());
        (asset.name == exe && hex && asset.size <= MAX_SIZE).then_some(Release {
            tag: tag.clone(),
            exe: asset.name,
            size: asset.size,
            sha256,
        })
    });
    Ok(release)
}

/// "v1.2.3" or "1.2.3", as numbers. Nothing else is a version, so a tag
/// that is one is safe in a URL.
fn version(tag: &str) -> Option<(u32, u32, u32)> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let mut next = || -> Option<u32> {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Installs `release`. The portable edition downloads it, starts it and
/// returns, for this one to quit; the others start the task that installs
/// it, which closes this one.
pub fn install(release: &Release) -> Result<(), String> {
    if by_task() {
        return run_task();
    }
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = current
        .parent()
        .ok_or_else(|| tr().update_failed.to_string())?;
    let path = dir.join(&release.exe);
    let file = dicts::File {
        release: &release.tag,
        name: &release.exe,
        size: release.size,
        sha256: &release.sha256,
    };
    dicts::save(&file, &path, &mut |_| {})?;
    std::process::Command::new(&path)
        .arg(REPLACE)
        .arg(std::process::id().to_string())
        .arg(&current)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

fn run_task() -> Result<(), String> {
    let schtasks = crate::win::windows_dir().join("System32\\schtasks.exe");
    let ran = std::process::Command::new(schtasks)
        .args(["/Run", "/TN", TASK])
        .creation_flags(CREATE_NO_WINDOW.0)
        .status()
        .is_ok_and(|s| s.success());
    if ran {
        Ok(())
    } else {
        Err(tr().update_task_missing.into())
    }
}

/// This executable was started by the version it replaces, to take its place.
pub struct Replacing {
    old: PathBuf,
}

impl Replacing {
    /// From the command line, if it says so.
    pub fn from_args() -> Option<Replacing> {
        let mut args = std::env::args_os().skip(1);
        if args.next()? != REPLACE {
            return None;
        }
        // Its process id, which can be another's by the time it's read:
        // the instance mutex is waited on instead.
        let _pid: u32 = args.next()?.to_str()?.parse().ok()?;
        let old = PathBuf::from(args.next()?);
        Some(Replacing { old })
    }

    /// Waits, for a while, for the old version to exit and let go of
    /// `instance`, the mutex the running WinDict holds. Returns whether it
    /// did, which makes this one the running WinDict.
    pub fn wait(&self, instance: HANDLE) -> bool {
        let waited = unsafe { WaitForSingleObject(instance, EXIT_WAIT_MS) };
        waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED
    }

    /// Deletes the old executable, if it's one beside this one. Windows may
    /// hold on to it for a moment after it exits.
    pub fn remove_old(&self) {
        let Ok(current) = std::env::current_exe() else {
            return;
        };
        let exe = self
            .old
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
        if !exe || self.old == current || self.old.parent() != current.parent() {
            return;
        }
        for _ in 0..20 {
            if std::fs::remove_file(&self.old).is_ok() || !self.old.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "880316e20297068fd9a2a449ed89c4cd5d53eb4a658c506ce63f0144c6571572";

    fn latest(tag: &str) -> String {
        format!(
            r#"{{"tag_name": "{tag}", "assets": [
                {{"name": "WinDict-{tag}.exe", "size": 8441344, "digest": "sha256:{DIGEST}"}},
                {{"name": "WinDict-{tag}-setup.exe", "size": 8441344, "digest": "sha256:{DIGEST}"}},
                {{"name": "notes.txt", "size": 2404}}
            ]}}"#
        )
    }

    #[test]
    fn reads_versions() {
        assert_eq!(version("v0.20.3"), Some((0, 20, 3)));
        assert_eq!(version("1.2.10"), Some((1, 2, 10)));
        for tag in [
            "v1.2",
            "v1.2.3.4",
            "v1.2.x",
            "v1..3",
            "dictionaries",
            "v1.2.3-rc1",
            "",
        ] {
            assert_eq!(version(tag), None, "{tag}");
        }
        assert!(version("v0.20.10") > version("v0.20.9"));
    }

    #[test]
    fn finds_newer_releases() {
        let json = latest("v0.21.0");
        let release = parse(json.as_bytes(), "0.20.3", Edition::Portable)
            .unwrap()
            .unwrap();
        assert_eq!(release.tag, "v0.21.0");
        assert_eq!(release.exe, "WinDict-v0.21.0.exe");
        assert_eq!(release.size, 8441344);
        assert_eq!(release.sha256, DIGEST);
        let release = parse(json.as_bytes(), "0.20.3", Edition::Installed)
            .unwrap()
            .unwrap();
        assert_eq!(release.exe, "WinDict-v0.21.0-setup.exe");
        // Not newer.
        assert_eq!(
            parse(json.as_bytes(), "0.21.0", Edition::Portable),
            Ok(None)
        );
        assert_eq!(
            parse(json.as_bytes(), "0.22.0", Edition::Portable),
            Ok(None)
        );
    }

    #[test]
    fn needs_an_executable_to_check() {
        let without_digest =
            r#"{"tag_name": "v1.0.0", "assets": [{"name": "WinDict-v1.0.0.exe", "size": 5}]}"#;
        assert_eq!(
            parse(without_digest.as_bytes(), "0.1.0", Edition::Portable),
            Ok(None)
        );
        let bad_digest = r#"{"tag_name": "v1.0.0", "assets": [
            {"name": "WinDict-v1.0.0.exe", "size": 5, "digest": "md5:abc"}]}"#;
        assert_eq!(
            parse(bad_digest.as_bytes(), "0.1.0", Edition::Portable),
            Ok(None)
        );
        let no_assets = r#"{"tag_name": "v1.0.0"}"#;
        assert_eq!(
            parse(no_assets.as_bytes(), "0.1.0", Edition::Portable),
            Ok(None)
        );
        assert!(parse(b"not json", "0.1.0", Edition::Portable).is_err());
    }
}
