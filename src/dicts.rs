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
        file: "it-8dfb3b1735d4.wdict",
        size: 10_280_756,
        sha256: "8dfb3b1735d4b56a3e0644ba8cbd83a330b5400ee2cacbdb6d77de80ce82aa9a",
    },
    Download {
        language: "es",
        name: "Espa\u{F1}ol",
        file: "es-ba542ddaabb8.wdict",
        size: 19_585_095,
        sha256: "ba542ddaabb82b34795ff49318944eee2b657f2daf98d5c281b2ea152bee841a",
    },
    Download {
        language: "fr",
        name: "Fran\u{E7}ais",
        file: "fr-2a2278120dfb.wdict",
        size: 103_622_036,
        sha256: "2a2278120dfb3effeecc05ce7608c4fbd52003d2d0e5ecb375642bd690619cb9",
    },
    Download {
        language: "de",
        name: "Deutsch",
        file: "de-34bd1ebb2742.wdict",
        size: 58_673_276,
        sha256: "34bd1ebb27422789ad25eed97c8c877483f4293856ee0b40b4e5e2498732a842",
    },
    Download {
        language: "pt",
        name: "Portugu\u{EA}s",
        file: "pt-9afbb4d12647.wdict",
        size: 11_320_225,
        sha256: "9afbb4d12647259ec5cff0af0a9efc9c3d36e6bc32d9aec3a26b70e67634df2f",
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
/// it's downloaded, telling `progress` how far it's got, in percent.
/// Returns whether it had to be. Errors are short, to fit under the
/// setting.
pub fn install(download: &Download, mut progress: impl FnMut(u32)) -> Result<bool, String> {
    let dir = offline::dicts_dir();
    let path = dir.join(format!("{}.wdict", download.language));
    let file = download.file();
    if in_place(&file, &path) {
        return Ok(false);
    }
    std::fs::create_dir_all(&dir).map_err(unsaved)?;
    save(&file, &path, &mut progress)?;
    // It was checked as it was fetched.
    let _ = std::fs::write(checked(&path), file.sha256);
    Ok(true)
}

/// Whether the file at `path` is `file`. It's read through and hashed
/// only once: that it was is noted beside it, and from then on it's told
/// by its size and the note, which takes no time at each start. A file
/// of another size, as another version's is, isn't read at all.
fn in_place(file: &File, path: &Path) -> bool {
    if !std::fs::metadata(path).is_ok_and(|m| m.len() == file.size) {
        return false;
    }
    let note = checked(path);
    if std::fs::read_to_string(&note).is_ok_and(|hash| hash.trim() == file.sha256) {
        return true;
    }
    let right = std::fs::read(path).is_ok_and(|bytes| file.matches(&bytes));
    if right {
        let _ = std::fs::write(&note, file.sha256);
    }
    right
}

/// Where it's noted which file the one at `path` was checked to be: its
/// hash, in a file beside it ("it.wdict.sha256").
fn checked(path: &Path) -> PathBuf {
    let mut note = path.as_os_str().to_owned();
    note.push(".sha256");
    PathBuf::from(note)
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
    .map_err(|_| if oversized { damaged() } else { unreachable() })?;
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

fn unreachable() -> String {
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

    /// The file it's downloaded from, as it must be.
    fn file(&self) -> File<'_> {
        File {
            release: DICTIONARIES,
            name: self.file,
            size: self.size,
            sha256: self.sha256,
        }
    }
}

impl File<'_> {
    /// Whether `bytes` are the file, whole and as released.
    fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() as u64 == self.size && sha256(bytes).is_some_and(|h| hex(&h) == self.sha256)
    }
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
        assert!(find("de").is_some());
        assert!(find("pt").is_some());
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
        assert!(d.file().matches(b"abc"));
        assert!(!d.file().matches(b"abd"));
        assert!(!d.file().matches(b"abcd"));
    }

    #[test]
    fn checks_a_file_in_place_once() {
        let dir = std::env::temp_dir().join(format!("windict-dicts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("xx.wdict");
        let file = File {
            release: DICTIONARIES,
            name: "xx.wdict",
            size: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        // Missing, of another size or with something else in it, it isn't
        // the file, and nothing is noted.
        assert!(!in_place(&file, &path));
        std::fs::write(&path, b"abcd").unwrap();
        assert!(!in_place(&file, &path));
        std::fs::write(&path, b"abd").unwrap();
        assert!(!in_place(&file, &path));
        assert!(!checked(&path).exists());
        // The file is, and that's noted, not to read it through again.
        std::fs::write(&path, b"abc").unwrap();
        assert!(in_place(&file, &path));
        assert_eq!(
            std::fs::read_to_string(checked(&path)).unwrap(),
            file.sha256
        );
        assert!(in_place(&file, &path));
        // A note of another file, as an older version leaves, says nothing
        // of this one: it's read through again.
        std::fs::write(checked(&path), "0000").unwrap();
        assert!(in_place(&file, &path));
        std::fs::write(checked(&path), "0000").unwrap();
        std::fs::write(&path, b"abd").unwrap();
        assert!(!in_place(&file, &path));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
