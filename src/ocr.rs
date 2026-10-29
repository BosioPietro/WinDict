//! Reading text off a picture of the screen, with the recognizers Windows
//! has for the languages installed (Windows.Media.Ocr), and telling what
//! language it's in. It runs on a thread of its own: recognizing takes a
//! moment, longer with each recognizer tried.

use windows::core::Result;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::{OcrEngine, OcrResult};
use windows::Storage::Streams::DataWriter;
use windows::Win32::Globalization::*;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::lang::LangCode;
use crate::{offline, translate};

/// At most this many recognizers are tried: each takes a moment.
const MAX_ENGINES: usize = 4;
/// Pictures smaller than this, in pixels each way, are read at twice their
/// size: Windows reads small text poorly.
const ENLARGE_BELOW: u32 = 1200;

/// A picture: rows of blue, green, red and alpha bytes, from the top.
pub struct Picture {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The text read off a picture, and the language it's likeliest in.
pub struct Scanned {
    pub text: String,
    pub language: Option<LangCode>,
}

/// Why nothing could be read.
pub enum ScanError {
    /// Windows has no recognizer for any language installed.
    NoRecognizer,
    /// It read the picture, and found no text.
    NoText,
}

/// Whether Windows can read text in any language: it reads in those whose
/// optical character recognition is installed, as most languages' packs
/// install it.
pub fn available() -> bool {
    OcrEngine::AvailableRecognizerLanguages().is_ok_and(|l| l.Size().is_ok_and(|n| n > 0))
}

/// Reads the text in `picture`, in one line. With several recognizers,
/// it's the one whose language the text it reads turns out to be in, if
/// any, else the one for the user's languages, first.
pub fn read(picture: &Picture) -> std::result::Result<Scanned, ScanError> {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let engines = engines();
    if engines.is_empty() {
        return Err(ScanError::NoRecognizer);
    }
    let Ok(bitmap) = bitmap(picture) else {
        return Err(ScanError::NoText);
    };
    let mut first = None;
    for engine in &engines {
        let Some(text) = recognize(engine, &bitmap).filter(|t| !t.is_empty()) else {
            continue;
        };
        let language = detect(&text);
        let read_in = engine
            .RecognizerLanguage()
            .and_then(|l| l.LanguageTag())
            .map(|t| primary(&t.to_string()))
            .unwrap_or_default();
        if language.as_deref().is_some_and(|l| primary(l) == read_in) {
            return Ok(Scanned { text, language });
        }
        first.get_or_insert(Scanned { text, language });
    }
    first.ok_or(ScanError::NoText)
}

/// The recognizers to try, the one for the user's languages first.
fn engines() -> Vec<OcrEngine> {
    let mut engines: Vec<OcrEngine> = OcrEngine::TryCreateFromUserProfileLanguages()
        .ok()
        .into_iter()
        .collect();
    let tag = |e: &OcrEngine| e.RecognizerLanguage().and_then(|l| l.LanguageTag()).ok();
    let first = engines.first().and_then(tag);
    if let Ok(languages) = OcrEngine::AvailableRecognizerLanguages() {
        for language in languages {
            if engines.len() >= MAX_ENGINES {
                break;
            }
            if language.LanguageTag().ok() == first {
                continue;
            }
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
                engines.push(engine);
            }
        }
    }
    engines
}

/// `picture` as Windows takes it to read, enlarged when it's small and cut
/// down to what it reads at most.
fn bitmap(picture: &Picture) -> Result<SoftwareBitmap> {
    let most = OcrEngine::MaxImageDimension().unwrap_or(4096).max(1);
    let (w, h) = (picture.width, picture.height);
    let k = if w.max(h) < ENLARGE_BELOW && w.max(h) * 2 <= most {
        2
    } else {
        1
    };
    let (out_w, out_h) = ((w * k).min(most), (h * k).min(most));
    let mut pixels = Vec::with_capacity((out_w * out_h * 4) as usize);
    for y in 0..out_h {
        let row = &picture.pixels[((y / k) * w * 4) as usize..];
        for x in 0..out_w {
            let at = ((x / k) * 4) as usize;
            pixels.extend_from_slice(&row[at..at + 3]);
            // Opaque: what the screen showed has no alpha to speak of.
            pixels.push(255);
        }
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&pixels)?;
    let buffer = writer.DetachBuffer()?;
    SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        out_w as i32,
        out_h as i32,
    )
}

/// The text `engine` reads in `bitmap`, its lines run together.
fn recognize(engine: &OcrEngine, bitmap: &SoftwareBitmap) -> Option<String> {
    let result: OcrResult = engine.RecognizeAsync(bitmap).ok()?.join().ok()?;
    let lines: Vec<String> = result
        .Lines()
        .ok()?
        .into_iter()
        .filter_map(|line| line.Text().ok())
        .map(|text| tidy(&text.to_string()))
        .filter(|text| !text.is_empty())
        .collect();
    Some(join_lines(&lines))
}

/// Lines run together as one: by a space, unless one ends and the next
/// starts in writing without spaces between words, or a word was broken
/// over them with a hyphen.
fn join_lines(lines: &[String]) -> String {
    let mut text = String::new();
    for line in lines {
        let (Some(last), Some(next)) = (text.chars().last(), line.chars().next()) else {
            text.push_str(line);
            continue;
        };
        if last == '-' && next.is_lowercase() {
            text.pop();
        } else if !(unspaced(last) && unspaced(next)) {
            text.push(' ');
        }
        text.push_str(line);
    }
    text
}

/// `line` without the spaces Windows reads between characters of writing
/// that doesn't space its words (Chinese, Japanese), and with no more than
/// one space anywhere else.
fn tidy(line: &str) -> String {
    let chars: Vec<char> = line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let mut out = String::with_capacity(line.len());
    for (i, &c) in chars.iter().enumerate() {
        let between = |before: usize| {
            let prev = chars.get(before).copied();
            let next = chars.get(i + 1).copied();
            prev.is_some_and(unspaced) && next.is_some_and(unspaced)
        };
        if c == ' ' && i > 0 && between(i - 1) {
            continue;
        }
        out.push(c);
    }
    out
}

/// Whether `c` is written without spaces around words: Han and kana.
fn unspaced(c: char) -> bool {
    matches!(script(c), Some(Script::Han | Script::Kana))
        || matches!(c, '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FFEF}')
}

#[derive(Clone, Copy, PartialEq)]
enum Script {
    Kana,
    Hangul,
    Han,
    Cyrillic,
    Greek,
}

fn script(c: char) -> Option<Script> {
    Some(match c {
        '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}' => Script::Kana,
        '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{AC00}'..='\u{D7AF}' => {
            Script::Hangul
        }
        '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' => Script::Han,
        '\u{0400}'..='\u{04FF}' => Script::Cyrillic,
        '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}' => Script::Greek,
        _ => return None,
    })
}

/// The language `text` is likeliest in, among those it can be translated
/// from: told by its writing, when that's one language's, else by the
/// dictionaries in use and what Windows makes of it.
pub fn detect(text: &str) -> Option<LangCode> {
    if let Some(language) = by_script(text) {
        return Some(language.into());
    }
    let words = text.split_whitespace().count();
    // Windows guesses short text poorly: the dictionaries go first there.
    let guess = if words >= 3 {
        by_windows(text).or_else(|| offline::detect(text))
    } else {
        offline::detect(text).or_else(|| by_windows(text))
    };
    guess.filter(|l| translate::supported(l))
}

/// The language `text`'s writing is, if it's one of a language of its
/// own: kana is Japanese, even among Han; hangul Korean; Cyrillic Russian,
/// unless it has Ukrainian's own letters.
fn by_script(text: &str) -> Option<&'static str> {
    let mut counts = [0usize; 5];
    let mut letters = 0;
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        letters += 1;
        if let Some(s) = script(c) {
            counts[s as usize] += 1;
        }
    }
    let count = |s: Script| counts[s as usize];
    // Most of the letters, so a name in Latin letters doesn't sway it.
    let most = |n: usize| n * 2 > letters;
    if count(Script::Kana) > 0 && most(count(Script::Kana) + count(Script::Han)) {
        Some("ja")
    } else if most(count(Script::Hangul) + count(Script::Han)) && count(Script::Hangul) > 0 {
        Some("ko")
    } else if most(count(Script::Han)) {
        Some("zh-CN")
    } else if most(count(Script::Cyrillic)) {
        let ukrainian = text.chars().any(|c| "іїєґІЇЄҐ".contains(c));
        Some(if ukrainian { "uk" } else { "ru" })
    } else if most(count(Script::Greek)) {
        Some("el")
    } else {
        None
    }
}

/// What Windows makes of `text`'s language (Extended Linguistic
/// Services), the likeliest it can be translated from.
fn by_windows(text: &str) -> Option<LangCode> {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut guid = ELS_GUID_LANGUAGE_DETECTION;
    let options = MAPPING_ENUM_OPTIONS {
        Size: std::mem::size_of::<MAPPING_ENUM_OPTIONS>(),
        pGuid: &mut guid,
        ..Default::default()
    };
    let mut services: *mut MAPPING_SERVICE_INFO = std::ptr::null_mut();
    let mut count = 0u32;
    unsafe {
        MappingGetServices(Some(&options), &mut services, &mut count).ok()?;
        if services.is_null() {
            return None;
        }
        let mut bag = MAPPING_PROPERTY_BAG {
            Size: std::mem::size_of::<MAPPING_PROPERTY_BAG>(),
            ..Default::default()
        };
        let mut found = None;
        if count > 0 && MappingRecognizeText(services, &wide, 0, None, &mut bag).is_ok() {
            if bag.dwRangesCount > 0 && !bag.prgResultRanges.is_null() {
                let range = &*bag.prgResultRanges;
                let data = range.pData as *const u16;
                let len = range.dwDataSize as usize / 2;
                if !data.is_null() {
                    let tags = std::slice::from_raw_parts(data, len);
                    found = tags
                        .split(|&u| u == 0)
                        .map(String::from_utf16_lossy)
                        .filter(|t| !t.is_empty())
                        .find_map(|t| supported_code(&t));
                }
            }
            let _ = MappingFreePropertyBag(&bag);
        }
        let _ = MappingFreeServices(services);
        found.map(LangCode::from)
    }
}

/// The code of the language with tag `tag` ("en-US", "zh-Hans") it can
/// be translated from, if it's one.
fn supported_code(tag: &str) -> Option<&'static str> {
    let code = match primary(tag).as_str() {
        "zh" => "zh-CN",
        "nb" | "nn" => "no",
        other => {
            return translate::LANGUAGES
                .iter()
                .map(|(c, _)| *c)
                .find(|c| *c == other)
        }
    };
    Some(code)
}

/// The language a tag ("en-US", "zh-Hans-CN") is for, without where.
fn primary(tag: &str) -> String {
    let language = tag
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match language.as_str() {
        "nb" | "nn" => "no".into(),
        _ => language,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_writing() {
        assert_eq!(by_script("こんにちは世界"), Some("ja"));
        assert_eq!(by_script("日本語の本"), Some("ja"));
        assert_eq!(by_script("中文的书"), Some("zh-CN"));
        assert_eq!(by_script("한국어 책"), Some("ko"));
        assert_eq!(by_script("Привет, мир"), Some("ru"));
        assert_eq!(by_script("Привіт, світ"), Some("uk"));
        assert_eq!(by_script("Γειά σου κόσμε"), Some("el"));
        assert_eq!(by_script("Hello world"), None);
        // A name in another writing doesn't sway it.
        assert_eq!(
            by_script("Tolstoy wrote Война и мир in many long years"),
            None
        );
    }

    #[test]
    fn runs_lines_together() {
        let lines = |l: &[&str]| join_lines(&l.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(lines(&["Hello", "world"]), "Hello world");
        assert_eq!(
            lines(&["a long hy-", "phenated word"]),
            "a long hyphenated word"
        );
        assert_eq!(lines(&["日本語の", "本"]), "日本語の本");
    }

    #[test]
    fn closes_up_unspaced_writing() {
        assert_eq!(tidy("日 本 語 の 本"), "日本語の本");
        assert_eq!(tidy("Hello   world"), "Hello world");
        assert_eq!(tidy("한국어 책"), "한국어 책");
    }

    #[test]
    fn maps_tags() {
        assert_eq!(supported_code("en-US"), Some("en"));
        assert_eq!(supported_code("zh-Hans"), Some("zh-CN"));
        assert_eq!(supported_code("nb-NO"), Some("no"));
        assert_eq!(supported_code("xx"), None);
    }
}
