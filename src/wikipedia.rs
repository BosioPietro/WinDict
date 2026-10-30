//! Wikipedia's summary of a page (<https://en.wikipedia.org/api/rest_v1/>):
//! what the dictionaries skip, people, places and brands, is answered
//! from it, and a definition links to its word's article.

use std::thread;

use serde::Deserialize;

use crate::dictionary::percent_encode;
use crate::http;
use crate::lang::LangCode;

/// The most of an article's opening the card shows, in characters: it's
/// cut at the end of the last sentence that fits.
const MAX_EXTRACT: usize = 420;
/// Where the pictures are, the only hosts they're fetched from: the
/// summaries give the thumbnails' host, the files' before it.
const PICTURES: [&str; 2] = ["thumb.wikimedia.org", "upload.wikimedia.org"];

/// The opening of a page, as the card shows it.
#[derive(Clone, Debug)]
pub struct Article {
    pub title: String,
    /// What it's about, in a few words ("Capital of Italy"), if the page
    /// says.
    pub description: Option<String>,
    /// Its first few sentences: empty when it's `ambiguous`.
    pub extract: String,
    pub url: String,
    pub thumbnail: Option<Thumbnail>,
    /// The title is several things', and the page lists them.
    pub ambiguous: bool,
}

/// The page's picture, made small.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumbnail {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// The article for the first of `candidates` (a title, in the language
/// whose Wikipedia it's looked for in) that has one, all asked at once;
/// failing that, the first that names several. Each title is tried as
/// it's written, and then each of its words capitalized ("new york" →
/// "New York"). An error, when none could be asked.
pub fn find(candidates: &[(LangCode, String)]) -> Result<Option<Article>, String> {
    let asked: Vec<_> = candidates
        .iter()
        .cloned()
        .map(|(language, title)| thread::spawn(move || summary(&language, &title)))
        .collect();
    let mut found = Vec::new();
    let mut failure = None;
    for answer in asked {
        match answer.join().unwrap_or(Ok(None)) {
            Ok(Some(article)) => found.push(article),
            Ok(None) => {}
            Err(e) => failure = Some(e),
        }
    }
    match (found.iter().position(|a| !a.ambiguous), failure) {
        (Some(i), _) => Ok(Some(found.swap_remove(i))),
        (None, Some(e)) if found.is_empty() => Err(e),
        _ => Ok(found.into_iter().next()),
    }
}

/// The page called `title`, or a variant of it, in `language`'s Wikipedia.
fn summary(language: &str, title: &str) -> Result<Option<Article>, String> {
    let host = format!("{}.wikipedia.org", edition(language));
    for title in variants(title) {
        let path = format!(
            "/api/rest_v1/page/summary/{}",
            percent_encode(&title.replace(' ', "_"))
        );
        match http::get(&host, &path)? {
            (200, body) => return Ok(parse(&body)),
            (404, _) => continue,
            (code, _) => return Err(format!("Wikipedia: HTTP {code}")),
        }
    }
    Ok(None)
}

/// The Wikipedia for `language`: "pt" for "pt-BR".
fn edition(language: &str) -> String {
    let code = language.split(['-', '_']).next().unwrap_or(language);
    code.to_ascii_lowercase()
}

/// `title`, then with each of its words capitalized, if that's another.
/// Wikipedia capitalizes a title's first letter itself.
fn variants(title: &str) -> Vec<String> {
    let capitalized = title
        .split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ");
    let mut out = vec![title.to_string()];
    // Only past the first word: Wikipedia capitalizes that one.
    if capitalized.get(1..) != title.get(1..) && title.contains(' ') {
        out.push(capitalized);
    }
    out
}

#[derive(Deserialize)]
struct Summary {
    #[serde(rename = "type")]
    kind: String,
    title: String,
    #[serde(default)]
    titles: Option<Titles>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    extract: String,
    #[serde(default)]
    thumbnail: Option<Picture>,
    content_urls: Urls,
}

#[derive(Deserialize)]
struct Titles {
    normalized: String,
}

#[derive(Deserialize)]
struct Picture {
    source: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct Urls {
    desktop: Page,
}

#[derive(Deserialize)]
struct Page {
    page: String,
}

/// A summary as the card shows it: nothing for a page with nothing to say.
fn parse(body: &[u8]) -> Option<Article> {
    let s: Summary = serde_json::from_slice(body).ok()?;
    let ambiguous = s.kind == "disambiguation";
    if s.kind != "standard" && !ambiguous {
        return None;
    }
    let extract = if ambiguous {
        String::new()
    } else {
        shorten(&s.extract)
    };
    if extract.is_empty() && !ambiguous {
        return None;
    }
    Some(Article {
        title: s
            .titles
            .map_or_else(|| s.title.replace('_', " "), |t| t.normalized),
        description: s.description.filter(|d| !d.trim().is_empty()),
        extract,
        url: s.content_urls.desktop.page,
        thumbnail: s
            .thumbnail
            .filter(|p| at_pictures(&p.source).is_some())
            .filter(|p| p.width > 0 && p.height > 0)
            .map(|p| Thumbnail {
                url: p.source,
                width: p.width,
                height: p.height,
            }),
        ambiguous,
    })
}

/// `extract`'s first sentences, as many as fit in [`MAX_EXTRACT`]; at least
/// the first, cut short if it's too long on its own.
fn shorten(extract: &str) -> String {
    let extract = extract.trim();
    if extract.chars().count() <= MAX_EXTRACT {
        return extract.to_string();
    }
    let limit = extract
        .char_indices()
        .nth(MAX_EXTRACT)
        .map_or(extract.len(), |(i, _)| i);
    // A sentence ends at a full stop followed by a space; "Dr. " and
    // "U.S. " end one too early, at worst.
    let end = extract[..limit]
        .match_indices(". ")
        .map(|(i, _)| i + 1)
        .last();
    match end {
        Some(end) => extract[..end].to_string(),
        None => {
            let cut = extract[..limit].rfind(' ').unwrap_or(limit);
            format!(
                "{}\u{2026}",
                extract[..cut].trim_end_matches([',', ';', ':'])
            )
        }
    }
}

/// The host and path of `url`, if it's at one of [`PICTURES`].
fn at_pictures(url: &str) -> Option<(&'static str, &str)> {
    PICTURES.iter().find_map(|&host| {
        let path = url.strip_prefix("https://")?.strip_prefix(host)?;
        path.starts_with('/').then_some((host, path))
    })
}

/// The picture at `thumbnail`, as it's encoded (a JPEG or PNG), if it
/// could be fetched.
pub fn picture(thumbnail: &Thumbnail) -> Option<Vec<u8>> {
    let (host, path) = at_pictures(&thumbnail.url)?;
    match http::get(host, path) {
        Ok((200, body)) if !body.is_empty() => Some(body),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURIN: &str = r#"{
        "type": "standard",
        "title": "Turin",
        "titles": {"canonical": "Turin", "normalized": "Turin", "display": "Turin"},
        "description": "Italian city",
        "extract": "Turin is a city and an important business and cultural centre in northern Italy.",
        "thumbnail": {"source": "https://upload.wikimedia.org/wikipedia/commons/thumb/a/ab/Turin.jpg/320px-Turin.jpg", "width": 320, "height": 213},
        "content_urls": {"desktop": {"page": "https://en.wikipedia.org/wiki/Turin"}}
    }"#;

    #[test]
    fn reads_a_summary() {
        let article = parse(TURIN.as_bytes()).unwrap();
        assert_eq!(article.title, "Turin");
        assert_eq!(article.description.as_deref(), Some("Italian city"));
        assert!(article.extract.starts_with("Turin is a city"));
        assert_eq!(article.url, "https://en.wikipedia.org/wiki/Turin");
        assert_eq!(article.thumbnail.unwrap().height, 213);
        assert!(!article.ambiguous);
    }

    #[test]
    fn tells_a_page_of_several_articles() {
        let body = r#"{
            "type": "disambiguation",
            "title": "Mercury",
            "extract": "Mercury may refer to:",
            "content_urls": {"desktop": {"page": "https://en.wikipedia.org/wiki/Mercury"}}
        }"#;
        let article = parse(body.as_bytes()).unwrap();
        assert!(article.ambiguous);
        assert!(article.extract.is_empty());
        assert!(article.thumbnail.is_none());
    }

    #[test]
    fn skips_pages_with_nothing_to_say() {
        let main = TURIN.replace("\"standard\"", "\"mainpage\"");
        assert!(parse(main.as_bytes()).is_none());
        let empty = TURIN.replace(
            "Turin is a city and an important business and cultural centre in northern Italy.",
            "",
        );
        assert!(parse(empty.as_bytes()).is_none());
    }

    #[test]
    fn takes_pictures_only_from_wikimedia() {
        let elsewhere = TURIN.replace("upload.wikimedia.org", "example.com");
        assert!(parse(elsewhere.as_bytes()).unwrap().thumbnail.is_none());
    }

    #[test]
    fn takes_thumbnails_from_their_host() {
        let thumb = TURIN.replace("upload.wikimedia.org", "thumb.wikimedia.org");
        let picture = parse(thumb.as_bytes()).unwrap().thumbnail.unwrap();
        let (host, path) = at_pictures(&picture.url).unwrap();
        assert_eq!(host, "thumb.wikimedia.org");
        assert!(path.starts_with("/wikipedia/commons/"));
        assert!(at_pictures("https://thumb.wikimedia.org.example.com/a.jpg").is_none());
    }

    #[test]
    fn capitalizes_each_word_as_a_second_try() {
        assert_eq!(variants("paris"), ["paris"]);
        assert_eq!(variants("new york"), ["new york", "New York"]);
        assert_eq!(variants("New York"), ["New York"]);
        assert_eq!(variants("barack obama"), ["barack obama", "Barack Obama"]);
    }

    #[test]
    fn asks_the_wikipedia_of_the_language() {
        assert_eq!(edition("it"), "it");
        assert_eq!(edition("pt-BR"), "pt");
    }

    #[test]
    fn keeps_whole_sentences() {
        let short = "One. Two.";
        assert_eq!(shorten(short), short);
        let long = format!("{} Last sentence.", "A sentence of words. ".repeat(30));
        let cut = shorten(&long);
        assert!(cut.ends_with("words."));
        assert!(cut.chars().count() <= MAX_EXTRACT);
        let run_on = "word ".repeat(200);
        assert!(shorten(&run_on).ends_with("word\u{2026}"));
    }
}
