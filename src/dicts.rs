//! The dictionaries other than English, which are downloaded when they're
//! chosen. They're kept in the `dictionaries` release on GitHub, named after
//! what's in them, and only the very files listed here are taken: a new
//! version of one comes with a new version of the app.
//!
//! [`save`], which fetches them, also fetches updates (`update`).

use std::io::Write;
use std::path::{Path, PathBuf};

use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};

use crate::http;
use crate::i18n::{fill, tr};
use crate::offline;

pub struct Download {
    /// As the `language` setting names it.
    pub language: &'static str,
    /// As the settings show it, in the language itself.
    pub name: &'static str,
    file: &'static str,
    size: u64,
    sha256: &'static str,
}

pub const DOWNLOADS: &[Download] = &[
    Download {
        language: "it",
        name: "Italiano",
        file: "it-caf16c6d0c49.wdict",
        size: 8_413_024,
        sha256: "caf16c6d0c49272bdcbe61861997b4c27b26c20392000cfe397a99d705aff2e2",
    },
    Download {
        language: "es",
        name: "Espa\u{F1}ol",
        file: "es-b6f25a06f010.wdict",
        size: 16_947_418,
        sha256: "b6f25a06f010fb5e55eaac4fe4a2d6cf7d74f8c162ead82eea298a49c8bb4663",
    },
    Download {
        language: "fr",
        name: "Fran\u{E7}ais",
        file: "fr-ec77c51e8b50.wdict",
        size: 89_322_553,
        sha256: "ec77c51e8b5082c51583a66449d5d589d360ccacf078d275c28f262e02fa0f3f",
    },
];

const HOST: &str = "github.com";
const RELEASES: &str = "/BosioPietro/WinDict/releases/download/";
const DICTIONARIES: &str = "dictionaries";

pub fn find(language: &str) -> Option<&'static Download> {
    DOWNLOADS.iter().find(|d| d.language == language)
}

/// Every language words can be looked up in, as code and name: English,
/// which is built in, then those that are downloaded.
pub fn languages() -> impl Iterator<Item = (&'static str, &'static str)> {
    std::iter::once(("en", "English")).chain(DOWNLOADS.iter().map(|d| (d.language, d.name)))
}

/// Where `language` comes in the order of preference: that of
/// [`languages`], then any it doesn't know.
pub fn rank(language: &str) -> usize {
    languages()
        .position(|(code, _)| code == language)
        .unwrap_or(usize::MAX)
}

/// What `language` is called, in itself ("Italiano"), or its code, in
/// capitals, if it isn't one WinDict knows.
pub fn name(language: &str) -> String {
    languages()
        .find(|(code, _)| *code == language)
        .map_or_else(|| language.to_uppercase(), |(_, name)| name.to_string())
}

/// Makes sure `download` is in its place, as it should be: if it isn't,
/// it's downloaded, telling `progress` how far it's got, in percent. Errors
/// are short, to fit under the setting.
pub fn install(download: &Download, mut progress: impl FnMut(u32)) -> Result<(), String> {
    let dir = offline::dicts_dir();
    let path = dir.join(format!("{}.wdict", download.language));
    if std::fs::read(&path).is_ok_and(|bytes| download.matches(&bytes)) {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(unsaved)?;
    let file = File {
        release: DICTIONARIES,
        name: download.file,
        size: download.size,
        sha256: download.sha256,
    };
    save(&file, &path, &mut progress)
}

/// A file attached to a release on GitHub, and what it must be.
pub struct File<'a> {
    /// The release's tag.
    pub release: &'a str,
    pub name: &'a str,
    pub size: u64,
    /// In lowercase hex.
    pub sha256: &'a str,
}

/// Downloads `file` to `path`, telling `progress` how far it's got, in
/// percent, if it's what it should be. Errors are short, to fit under a
/// setting.
pub fn save(file: &File, path: &Path, progress: &mut impl FnMut(u32)) -> Result<(), String> {
    // Fetched beside it, and moved in place once it's all there and right:
    // the one in use, if any, keeps working until then.
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let result =
        fetch(file, &part, progress).and_then(|()| std::fs::rename(&part, path).map_err(unsaved));
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn fetch(file: &File, part: &Path, progress: &mut impl FnMut(u32)) -> Result<(), String> {
    let mut out = std::fs::File::create(part).map_err(unsaved)?;
    let mut bytes = Vec::with_capacity(file.size as usize);
    let mut shown = 0;
    let path = format!("{RELEASES}{}/{}", file.release, file.name);
    // More than it said it was: what to say is picked below, once the fetch
    // has returned, so the sink needs no words.
    let mut oversized = false;
    let status = http::fetch(HOST, &path, |chunk| {
        bytes.extend_from_slice(chunk);
        if bytes.len() as u64 > file.size {
            oversized = true;
            return Err(String::new());
        }
        let percent = (bytes.len() as u64 * 100 / file.size.max(1)) as u32;
        if percent != shown {
            shown = percent;
            progress(percent);
        }
        Ok(())
    })
    .map_err(|_| if oversized { damaged() } else { offline() })?;
    if status != 200 {
        return Err(fill(tr().download_status, &[("status", &status)]));
    }
    if !file.matches(&bytes) {
        return Err(damaged());
    }
    out.write_all(&bytes)
        .and_then(|()| out.sync_all())
        .map_err(unsaved)
}

fn offline() -> String {
    tr().download_unreachable.into()
}

fn damaged() -> String {
    tr().download_damaged.into()
}

fn unsaved(e: std::io::Error) -> String {
    fill(tr().save_failed, &[("reason", &e.kind())])
}

impl Download {
    /// How big it is to download, in whole megabytes.
    pub fn megabytes(&self) -> u64 {
        (self.size + 500_000) / 1_000_000
    }

    fn matches(&self, bytes: &[u8]) -> bool {
        matches(bytes, self.size, self.sha256)
    }
}

impl File<'_> {
    fn matches(&self, bytes: &[u8]) -> bool {
        matches(bytes, self.size, self.sha256)
    }
}

fn matches(bytes: &[u8], size: u64, sha256_hex: &str) -> bool {
    bytes.len() as u64 == size && sha256(bytes).is_some_and(|h| hex(&h) == sha256_hex)
}

fn sha256(bytes: &[u8]) -> Option<[u8; 32]> {
    let mut hash = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, bytes, &mut hash) }
        .is_ok()
        .then_some(hash)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes() {
        let empty = sha256(b"").map(|h| hex(&h));
        assert_eq!(
            empty.as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        let abc = sha256(b"abc").map(|h| hex(&h));
        assert_eq!(
            abc.as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }

    #[test]
    fn lists_files_by_what_is_in_them() {
        for d in DOWNLOADS {
            assert_eq!(d.sha256.len(), 64, "{}", d.language);
            assert_eq!(d.file, format!("{}-{}.wdict", d.language, &d.sha256[..12]));
            assert_ne!(d.language, "en");
        }
        assert!(find("it").is_some());
        assert!(find("es").is_some());
        assert!(find("fr").is_some());
        assert!(find("en").is_none());
    }

    #[test]
    fn takes_only_the_file_listed() {
        let d = Download {
            language: "xx",
            name: "X",
            file: "xx.wdict",
            size: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        assert!(d.matches(b"abc"));
        assert!(!d.matches(b"abd"));
        assert!(!d.matches(b"abcd"));
    }
}
