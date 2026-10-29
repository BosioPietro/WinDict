//! The pictures of a word looked up, a phrase translated, and the
//! dictionaries, drawn in the language WinDict's shown in. Each is a
//! screenshot or a render with its text wiped: the page behind the card and
//! the cards' content are set here, as they'd be in that language,
//! definitions and a translation baked in for the dictionaries that aren't
//! downloaded yet.

use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT;
use windows::UI::Composition::CompositionDrawingSurface;

use crate::dictionary::{self, Definition, Meaning, Sense};
use crate::i18n::Language;
use crate::lang::LangCode;
use crate::popup::{self, Still};
use crate::render::{
    capitalize, Body, Font, Layout, Line, Over, Patch, Renderer, Scene, Translated, TranslationCard,
};
use crate::theme::Theme;
use crate::translate::{self, Translation};

/// The screenshots' pixels to a DIP: they were taken at 225%.
const SHOT: f32 = 2.25;
/// How far apart a page's lines are, in DIPs: 19px type, 1.7 apart.
const LINE_H: f32 = 32.3;
/// How far in a paragraph after the first starts, in DIPs: 1.6em.
const INDENT: f32 = 30.4;
/// The card's corners', in the screenshots' pixels.
const CARD_RADIUS: f32 = 8.0 * SHOT;
/// The dictionaries' render's pixels to a DIP of the intro's picture pane:
/// it was drawn at 1235 by 1566, for the pane's 328 by 416.
const PANE: f32 = 1235.0 / 328.0;
/// The middle of the pane, where the dictionaries' cards stand, in its DIPs.
const PANE_MIDDLE: f32 = 328.0 / 2.0;
/// The dictionaries' card in front: its top, in the pane's DIPs, and how
/// big it's drawn.
const FRONT_TOP: f32 = 118.0;
const FRONT: f32 = 0.66;
/// Each card behind is this much higher than the one in front of it, in
/// the pane's DIPs, and this much smaller, and fades and blurs this much
/// more, the blur in the pane's DIPs.
const STEP: f32 = 40.0;
const BACK: f32 = 0.84;
const DIM: f32 = 0.2;
const BLUR: f32 = 0.5;
/// The cards' height, in DIPs: the Lookup screenshot's card's.
const CARD_H: f32 = (1258.0 - 341.0) / SHOT;

/// The pictures drawn here.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shot {
    /// A book, a word in it looked up.
    Lookup,
    /// A mail, a phrase in it translated.
    Translate,
    /// The same word in three dictionaries, their cards one behind another.
    Dictionaries,
}

impl Shot {
    /// The screenshot, its text wiped.
    fn picture(self, dark: bool) -> &'static [u8] {
        match (self, dark) {
            (Shot::Lookup, false) => include_bytes!("../../assets/intro/lookup-light.jpg"),
            (Shot::Lookup, true) => include_bytes!("../../assets/intro/lookup-dark.jpg"),
            (Shot::Translate, false) => include_bytes!("../../assets/intro/translate-light.jpg"),
            (Shot::Translate, true) => include_bytes!("../../assets/intro/translate-dark.jpg"),
            (Shot::Dictionaries, false) => {
                include_bytes!("../../assets/intro/dictionaries-light.jpg")
            }
            (Shot::Dictionaries, true) => {
                include_bytes!("../../assets/intro/dictionaries-dark.jpg")
            }
        }
    }

    /// The card's frame in the screenshot, in its pixels: the dictionaries'
    /// is the one in front.
    fn card(self) -> D2D_RECT_F {
        match self {
            Shot::Lookup => rect(182.0, 341.0, 1045.0, 1258.0),
            Shot::Translate => rect(191.0, 600.0, 1053.0, 1038.0),
            Shot::Dictionaries => stacked(0),
        }
    }

    /// The screenshot's pixels to a DIP of the card's content.
    fn scale(self) -> f32 {
        match self {
            Shot::Lookup | Shot::Translate => SHOT,
            Shot::Dictionaries => FRONT * PANE,
        }
    }
}

/// How big the dictionaries' card `depth` cards behind the front one is
/// drawn, the front one's 0.
fn stacked_scale(depth: usize) -> f32 {
    FRONT * BACK.powi(depth as i32)
}

/// The frame, in the render's pixels, of the dictionaries' card `depth`
/// cards behind the front one: on the pane's middle, a step higher each.
fn stacked(depth: usize) -> D2D_RECT_F {
    let k = stacked_scale(depth);
    let left = PANE_MIDDLE - popup::CARD_W / 2.0 * k;
    let top = FRONT_TOP - depth as f32 * STEP;
    rect(
        left * PANE,
        top * PANE,
        (left + popup::CARD_W * k) * PANE,
        (top + CARD_H * k) * PANE,
    )
}

/// The dictionaries behind `language`'s, nearest first: those to download,
/// as English's comes with WinDict.
fn behind(language: Language) -> [Language; 2] {
    match language {
        Language::English | Language::French => [Language::Italian, Language::Spanish],
        Language::Italian => [Language::Spanish, Language::French],
        Language::Spanish => [Language::French, Language::Italian],
    }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

/// `shot` in `language`, light or dark as `theme` is, filling a `w` by `h`
/// pixel surface.
pub fn draw(
    r: &Renderer,
    shot: Shot,
    language: Language,
    theme: &Theme,
    w: f32,
    h: f32,
) -> Result<CompositionDrawingSurface> {
    let parts = Parts::new(r, shot, language, theme.dark)?;
    r.draw_scene(
        shot.picture(theme.dark),
        w,
        h,
        Some((&parts.scene(), theme)),
    )
}

/// What's drawn over a screenshot: the page, or the cards behind, nearest
/// first, and the card's content.
struct Parts {
    shot: Shot,
    page: Option<Layout>,
    behind: Vec<Vec<(Layout, f32, f32)>>,
    card: Vec<(Layout, f32, f32)>,
    /// The card's content's height, in DIPs, which the tests check.
    #[cfg_attr(not(test), allow(dead_code))]
    card_h: f32,
}

impl Parts {
    fn new(r: &Renderer, shot: Shot, language: Language, dark: bool) -> Result<Parts> {
        let mut behind = Vec::new();
        let (page, (card, card_h)) = match shot {
            Shot::Lookup => {
                let book = Book::of(language);
                (Some(book.layout(r, dark)?), definition_card(r, book)?)
            }
            Shot::Translate => {
                let (mail, to) = Mail::of(language);
                (Some(mail.layout(r, dark)?), translation_card(r, mail, to)?)
            }
            Shot::Dictionaries => {
                for other in self::behind(language) {
                    behind.push(definition_card(r, Book::of(other))?.0);
                }
                (None, definition_card(r, Book::of(language))?)
            }
        };
        Ok(Parts {
            shot,
            page,
            behind,
            card,
            card_h,
        })
    }

    fn scene(&self) -> Scene<'_> {
        let frame = self.shot.card();
        let scale = self.shot.scale();
        let over = |layout, x, y| Over {
            layout,
            x,
            y,
            scale,
            clip: None,
            opacity: 1.0,
            blur: 0.0,
        };
        let mut under: Vec<Over> = (self.page.iter())
            .map(|page| over(page, 0.0, 0.0))
            .collect();
        // Each card behind shows only above the one in front of it, fainter
        // and softer the further back it is.
        let mut above = frame.top;
        for (i, card) in self.behind.iter().enumerate() {
            let depth = i + 1;
            let at = stacked(depth);
            let k = stacked_scale(depth) * PANE;
            let clip = rect(at.left, at.top, at.right, above);
            above = at.top;
            under.extend(card.iter().map(|(layout, x, y)| Over {
                layout,
                x: at.left + x * k,
                y: at.top + y * k,
                scale: k,
                clip: Some(clip),
                opacity: 1.0 - DIM * depth as f32,
                blur: BLUR * depth as f32 * PANE,
            }));
        }
        Scene {
            under,
            // The page's covered where the screenshot's card is.
            card: self.page.as_ref().map(|_| D2D1_ROUNDED_RECT {
                rect: frame,
                radiusX: CARD_RADIUS,
                radiusY: CARD_RADIUS,
            }),
            on: (self.card.iter())
                .map(|(layout, x, y)| over(layout, frame.left + x * scale, frame.top + y * scale))
                .collect(),
        }
    }
}

/// A book's page, a word on it selected: its lines, set by hand so the word
/// ends the second, where the card opens under it.
struct Book {
    paragraphs: &'static [&'static [&'static str]],
    /// The word, ending the first paragraph's second line.
    word: &'static str,
    language: &'static str,
    part_of_speech: &'static str,
    /// Its senses, and how each is used.
    senses: &'static [(&'static str, Option<&'static str>)],
    synonyms: &'static [&'static str],
    /// The other part of speech it is, as English's word is a verb too.
    also: Option<(
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
    )>,
    source: &'static str,
}

/// The book's face.
const GEORGIA: Font = Font {
    family: "Georgia",
    size: 19.0,
    bold: false,
};
/// Where the book's lines start and end, in DIPs, the column 560 wide.
const BOOK_RIGHT: f32 = 153.33;
const BOOK_LEFT: f32 = BOOK_RIGHT - 560.0;
/// The first line's top, in DIPs.
const BOOK_TOP: f32 = 83.26;

impl Book {
    fn of(language: Language) -> &'static Book {
        match language {
            Language::English => &BOOK_EN,
            Language::Italian => &BOOK_IT,
            Language::Spanish => &BOOK_ES,
            Language::French => &BOOK_FR,
        }
    }

    fn layout(&self, r: &Renderer, dark: bool) -> Result<Layout> {
        let color = if dark { 0xE4DAC8 } else { 0x2B2620 };
        let mut lines = Vec::new();
        let mut y = BOOK_TOP;
        for (p, paragraph) in self.paragraphs.iter().enumerate() {
            for (i, &text) in paragraph.iter().enumerate() {
                let indent = if p > 0 && i == 0 { INDENT } else { 0.0 };
                // The word ends the first paragraph's second line.
                let (text, picked) = match (p, i) {
                    (0, 1) => (text.strip_suffix(self.word).unwrap_or(text), self.word),
                    _ => (text, ""),
                };
                lines.push(Line {
                    x: BOOK_LEFT + indent,
                    y,
                    width: BOOK_RIGHT - BOOK_LEFT - indent,
                    height: LINE_H,
                    font: &GEORGIA,
                    color,
                    // Every line but a paragraph's last fills the column.
                    justify: i + 1 < paragraph.len(),
                    text: text.trim_end(),
                    picked,
                });
                y += LINE_H;
            }
        }
        r.layout_page(&lines, &[], BOOK_RIGHT, y)
    }
}

/// The card for the book's word, looked up.
fn definition_card(r: &Renderer, book: &Book) -> Result<Still> {
    let strings = |list: &[&str]| list.iter().map(|s| s.to_string()).collect();
    let mut meanings = vec![Meaning {
        part_of_speech: book.part_of_speech.to_owned(),
        senses: (book.senses.iter())
            .map(|(text, example)| Sense {
                text: text.to_string(),
                example: example.map(str::to_owned),
            })
            .collect(),
        synonyms: strings(book.synonyms),
        antonyms: Vec::new(),
    }];
    if let Some((part_of_speech, senses, synonyms)) = book.also {
        meanings.push(Meaning {
            part_of_speech: part_of_speech.to_owned(),
            senses: (senses.iter())
                .map(|text| Sense {
                    text: text.to_string(),
                    example: None,
                })
                .collect(),
            synonyms: strings(synonyms),
            antonyms: Vec::new(),
        });
    }
    let definition = Definition {
        word: book.word.to_owned(),
        language: LangCode::new(book.language),
        phonetic: None,
        meanings,
        note: None,
        sources: vec![book.source],
        others: Vec::new(),
    };
    let body = Body::Definition(&definition);
    popup::still(r, &capitalize(book.word), &body, true, true)
}

/// A mail, a phrase in it selected.
struct Mail {
    subject: &'static str,
    /// Where the mail is, on a chip after the subject.
    folder: &'static str,
    sender: &'static str,
    role: &'static str,
    /// Its paragraphs, each line set by hand.
    paragraphs: &'static [&'static [&'static str]],
    /// The phrase, ending the second paragraph's last line.
    phrase: &'static str,
    language: &'static str,
    /// The phrase, translated into each language.
    translations: &'static [(&'static str, &'static str)],
    /// How the mail's signed, under it.
    sign: &'static [&'static str],
}

const SEGOE: &str = "Segoe UI Variable Text";
const MAIL_FONT: Font = Font {
    family: SEGOE,
    size: 19.0,
    bold: false,
};
const SUBJECT_FONT: Font = Font {
    family: SEGOE,
    size: 22.0,
    bold: false,
};
const SENDER_FONT: Font = Font {
    family: SEGOE,
    size: 14.0,
    bold: true,
};
const ROLE_FONT: Font = Font {
    family: SEGOE,
    size: 14.0,
    bold: false,
};
const CHIP_FONT: Font = Font {
    family: SEGOE,
    size: 12.0,
    bold: false,
};
/// Where the selected phrase starts, in DIPs: the card's text lines up
/// with it.
const PHRASE_X: f32 = 106.67;
/// The tops of the subject's line, the sender's, and the mail's first.
const SUBJECT_TOP: f32 = -4.0;
const SUBJECT_H: f32 = 32.0;
const SENDER_TOP: f32 = 47.4;
const SENDER_H: f32 = 20.0;
const MAIL_TOP: f32 = 112.6;
/// Between a mail's paragraphs, in DIPs.
const PARAGRAPH_GAP: f32 = 18.0;
/// The avatar's, and the gap after it, before the sender's name.
const AVATAR: f32 = 40.0 + 12.0;

impl Mail {
    /// The mail, and the language its phrase is translated into: English,
    /// but for English, where it's Italian translated into English.
    fn of(language: Language) -> (&'static Mail, &'static str) {
        match language {
            Language::English => (&MAIL_IT, "en"),
            Language::Italian => (&MAIL_EN, "it"),
            Language::Spanish => (&MAIL_EN, "es"),
            Language::French => (&MAIL_EN, "fr"),
        }
    }

    fn layout(&self, r: &Renderer, dark: bool) -> Result<Layout> {
        let (ink, muted, chip) = if dark {
            (0xE3E3E3, 0xA8ABAF, 0x3A3A3A)
        } else {
            (0x1F1F1F, 0x5F6368, 0xE8EAED)
        };
        // The page starts where it puts the phrase over the card's text.
        let before = self.paragraphs[1].last().copied().unwrap_or_default();
        let before = before.strip_suffix(self.phrase).unwrap_or(before);
        let left = PHRASE_X - r.page_width(&MAIL_FONT, before)?;
        let line = |x, y, height, font, color, text| Line {
            x,
            y,
            width: 0.0,
            height,
            font,
            color,
            justify: false,
            text,
            picked: "",
        };
        let mut lines = Vec::new();
        lines.push(line(
            left,
            SUBJECT_TOP,
            SUBJECT_H,
            &SUBJECT_FONT,
            ink,
            self.subject,
        ));
        let chip_x = left + r.page_width(&SUBJECT_FONT, self.subject)? + 12.0;
        let chip_w = r.page_width(&CHIP_FONT, self.folder)? + 12.0;
        let chip_h = 20.0;
        let chip_top = SUBJECT_TOP + (SUBJECT_H - chip_h) / 2.0;
        lines.push(line(
            chip_x + 6.0,
            chip_top,
            chip_h,
            &CHIP_FONT,
            muted,
            self.folder,
        ));
        let patches = [Patch {
            x: chip_x,
            y: chip_top,
            width: chip_w,
            height: chip_h,
            radius: 4.0,
            color: chip,
        }];
        let name_x = left + AVATAR;
        lines.push(line(
            name_x,
            SENDER_TOP,
            SENDER_H,
            &SENDER_FONT,
            ink,
            self.sender,
        ));
        let role_x = name_x + r.page_width(&SENDER_FONT, self.sender)?;
        lines.push(line(
            role_x, SENDER_TOP, SENDER_H, &ROLE_FONT, muted, self.role,
        ));
        let mut y = MAIL_TOP;
        let signed = [self.sign];
        let paragraphs = self.paragraphs.iter().chain(&signed);
        for (p, paragraph) in paragraphs.enumerate() {
            let color = if p == self.paragraphs.len() {
                muted
            } else {
                ink
            };
            for (i, &text) in paragraph.iter().enumerate() {
                let picked = p == 1 && i + 1 == paragraph.len();
                let mut l = line(left, y, LINE_H, &MAIL_FONT, color, text);
                if picked {
                    l.text = text.strip_suffix(self.phrase).unwrap_or(text);
                    l.picked = self.phrase;
                }
                lines.push(l);
                y += LINE_H;
            }
            y += PARAGRAPH_GAP;
        }
        r.layout_page(&lines, &patches, PHRASE_X + 560.0, y)
    }
}

/// The card for the mail's phrase, translated `to`.
fn translation_card(r: &Renderer, mail: &Mail, to: &str) -> Result<Still> {
    let text = (mail.translations.iter())
        .find(|(code, _)| *code == to)
        .map_or(mail.phrase, |(_, text)| text);
    let translation = Translation {
        text: text.to_owned(),
        context: None,
        groups: Vec::new(),
        source: translate::GOOGLE,
    };
    let body = Body::Translation(TranslationCard {
        word: mail.phrase,
        from: mail.language,
        to,
        state: Translated::Done(&translation),
        picker: None,
    });
    popup::still(r, mail.phrase, &body, false, false)
}

static BOOK_EN: Book = Book {
    paragraphs: &[
        &[
            "By the time Mara reached the harbour wall, the",
            "sun had slipped behind the headland, and the last gleam",
            "of it lay on the water like a thread pulled loose from",
            "a coat. She sat on the cold stones and waited. Below",
            "her, the boats knocked against one another in the",
            "swell, patient as old dogs.",
        ],
        &[
            "Her father had told her once that the sea kept every",
            "promise made beside it, though never in the way you",
            "meant. She had not believed him then. She was not",
            "sure she believed him now, but she had come anyway,",
            "and that, she supposed, was a kind of answer.",
        ],
        &[
            "A light came on in the window of the customs house.",
            "Somebody was still awake down there, bent over a",
            "ledger or a letter, and the thought of it made the",
            "evening seem a little less enormous.",
        ],
    ],
    word: "gleam",
    language: "en",
    part_of_speech: "verb",
    senses: &[
        ("Be shiny, as if wet.", Some("His eyes were glistening")),
        ("Shine brightly, like a star or a light.", None),
        (
            "To shine with faint or brief light.",
            Some("A terrible thought gleamed in her mind"),
        ),
    ],
    synonyms: &["glitter", "glisten", "glint", "shine", "glimmer"],
    also: Some((
        "noun",
        &[
            "An appearance of reflected light.",
            "A flash of light (especially reflected light)",
        ],
        &["gleaming", "glow", "lambency", "glimmer"],
    )),
    source: "Open English WordNet",
};

static BOOK_IT: Book = Book {
    paragraphs: &[
        &[
            "Quando Mara raggiunse il muro del porto, il sole era",
            "scivolato dietro il promontorio, e l'ultimo bagliore",
            "si stendeva sull'acqua come un filo sfilato da un",
            "cappotto. Si sedette sulle pietre fredde e aspettò. Sotto",
            "di lei, le barche cozzavano l'una contro l'altra",
            "nell'onda lunga, pazienti come vecchi cani.",
        ],
        &[
            "Suo padre le aveva detto una volta che il mare",
            "manteneva ogni promessa fatta sulla sua riva, anche se",
            "mai nel modo che intendevi. Allora non gli aveva",
            "creduto. Non era sicura di credergli nemmeno adesso,",
            "ma era venuta lo stesso, e questa, pensò, era già",
            "una specie di risposta.",
        ],
        &[
            "Si accese una luce alla finestra della dogana.",
            "Qualcuno era ancora sveglio laggiù, chino su un",
            "registro o su una lettera, e il pensiero rese la sera",
            "un po' meno sconfinata.",
        ],
    ],
    word: "bagliore",
    language: "it",
    part_of_speech: "noun",
    senses: &[
        (
            "Luce viva e improvvisa, che abbaglia per un istante.",
            Some("Il bagliore di un lampo"),
        ),
        (
            "Luce fioca e tremolante, che si intravede appena.",
            Some("L'ultimo bagliore del tramonto"),
        ),
        (
            "Riflesso luminoso su una superficie.",
            Some("Il bagliore della luna sul mare"),
        ),
        (
            "Manifestazione breve e improvvisa di un sentimento.",
            Some("Un bagliore di speranza"),
        ),
    ],
    synonyms: &["lampo", "luccichio", "chiarore", "barlume", "riflesso"],
    also: None,
    source: dictionary::WIKIZIONARIO,
};

static BOOK_ES: Book = Book {
    paragraphs: &[
        &[
            "Cuando Mara llegó al muro del puerto, el sol ya se",
            "había escondido tras el cabo, y el último destello",
            "se tendía sobre el agua como un hilo suelto de un",
            "abrigo. Se sentó en las piedras frías y esperó. Abajo,",
            "las barcas chocaban unas contra otras en el oleaje,",
            "pacientes como perros viejos.",
        ],
        &[
            "Su padre le había dicho una vez que el mar cumplía",
            "toda promesa hecha a su orilla, aunque nunca como",
            "uno quería. Entonces no le había creído. Tampoco",
            "estaba segura de creerle ahora, pero había venido",
            "igual, y eso, supuso, ya era una especie de respuesta.",
        ],
        &[
            "Se encendió una luz en la ventana de la aduana.",
            "Alguien seguía despierto allí abajo, inclinado sobre",
            "un libro de cuentas o una carta, y pensarlo hizo que",
            "la noche pareciera un poco menos inmensa.",
        ],
    ],
    word: "destello",
    language: "es",
    part_of_speech: "noun",
    senses: &[
        (
            "Resplandor vivo y breve, como el de una chispa.",
            Some("El destello de un relámpago"),
        ),
        (
            "Reflejo de la luz sobre una superficie brillante.",
            Some("El último destello del sol en el agua"),
        ),
        (
            "Luz que brilla y se apaga de forma intermitente.",
            Some("El destello lejano de un faro"),
        ),
        (
            "Manifestación breve y repentina de algo.",
            Some("Un destello de esperanza"),
        ),
    ],
    synonyms: &["fulgor", "centelleo", "brillo", "resplandor", "chispazo"],
    also: None,
    source: dictionary::WIKCIONARIO,
};

static BOOK_FR: Book = Book {
    paragraphs: &[
        &[
            "Lorsque Mara atteignit le mur du port, le soleil avait",
            "glissé derrière le promontoire, et la dernière lueur",
            "s'étendait sur l'eau comme un fil tiré d'un manteau.",
            "Elle s'assit sur les pierres froides et attendit. En bas,",
            "les bateaux se cognaient les uns contre les autres",
            "dans la houle, patients comme de vieux chiens.",
        ],
        &[
            "Son père lui avait dit un jour que la mer tenait",
            "toutes les promesses faites sur ses bords, mais jamais",
            "comme on l'entendait. Elle ne l'avait pas cru alors.",
            "Elle n'était pas sûre de le croire maintenant, mais",
            "elle était venue quand même, et c'était, se dit-elle,",
            "déjà une sorte de réponse.",
        ],
        &[
            "Une lumière s'alluma à la fenêtre de la douane.",
            "Quelqu'un veillait encore là-bas, penché sur un",
            "registre ou sur une lettre, et cette pensée rendit",
            "le soir un peu moins immense.",
        ],
    ],
    word: "lueur",
    language: "fr",
    part_of_speech: "noun",
    senses: &[
        ("Clarté faible ou passagère.", Some("La lueur d'une bougie")),
        (
            "Lumière qui se reflète sur une surface.",
            Some("La dernière lueur du jour sur l'eau"),
        ),
        (
            "Éclat vif et bref dans le regard.",
            Some("Une lueur de colère dans ses yeux"),
        ),
        (
            "Manifestation passagère de quelque chose.",
            Some("Une lueur d'espoir"),
        ),
    ],
    synonyms: &["clarté", "reflet", "éclat", "scintillement", "étincelle"],
    also: None,
    source: dictionary::WIKTIONNAIRE,
};

static MAIL_EN: Mail = Mail {
    subject: "Milan showroom: revised floor plans",
    folder: "Inbox",
    sender: "Sarah Whitfield",
    role: " · Account lead",
    paragraphs: &[
        &["Hi Luca,"],
        &[
            "Thanks for the call this morning. I've shared the revised",
            "floor plans with the contractor in Milan and asked for",
            "an updated quote by Friday. I'll keep you in the loop",
        ],
        &[
            "In the meantime, could you send me the latest budget?",
            "I'd like to go through it with Giulia before Thursday.",
        ],
    ],
    phrase: "I'll keep you in the loop",
    language: "en",
    translations: &[
        ("it", "Vi terrò aggiornati"),
        ("es", "Te mantendré al tanto"),
        ("fr", "Je vous tiendrai au courant"),
    ],
    sign: &["Best regards,", "Sarah Whitfield"],
};

static MAIL_IT: Mail = Mail {
    subject: "Showroom di Milano: planimetrie riviste",
    folder: "Posta in arrivo",
    sender: "Chiara Bassi",
    role: " · Responsabile clienti",
    paragraphs: &[
        &["Ciao Luca,"],
        &[
            "grazie per la telefonata di stamattina. Ho mandato le",
            "planimetrie riviste all'impresa di Milano e ho chiesto",
            "un nuovo preventivo entro venerdì. Ti terrò aggiornato",
        ],
        &[
            "Nel frattempo, potresti mandarmi l'ultimo budget?",
            "Vorrei rivederlo con Giulia prima di giovedì.",
        ],
    ],
    phrase: "Ti terrò aggiornato",
    language: "it",
    translations: &[("en", "I'll keep you posted")],
    sign: &["Un saluto,", "Chiara Bassi"],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeMode;

    fn renderer() -> Renderer {
        let (_queue, compositor) = unsafe { crate::start_ui() }.unwrap();
        std::mem::forget(_queue);
        Renderer::new(&compositor).unwrap()
    }

    /// The card's content fits its frame, nearly filling it.
    #[test]
    fn cards_fit_their_frames() {
        let r = renderer();
        for shot in [Shot::Lookup, Shot::Translate, Shot::Dictionaries] {
            let frame = shot.card();
            let frame_h = (frame.bottom - frame.top) / shot.scale();
            for &language in Language::ALL {
                let parts = Parts::new(&r, shot, language, false).unwrap();
                println!("{shot:?} {language:?}: {:.1} of {frame_h:.1}", parts.card_h);
                assert!(parts.card_h <= frame_h + 1.0, "{shot:?} {language:?}");
            }
        }
    }

    /// Each picture, in each language, light and dark, as raw BGRA for
    /// looking at: `WINDICT_SCENES=dir cargo test scenes -- --ignored`.
    #[test]
    #[ignore]
    fn scenes() {
        let dir = std::env::var("WINDICT_SCENES").unwrap();
        let r = renderer();
        for shot in [Shot::Lookup, Shot::Translate, Shot::Dictionaries] {
            for &language in Language::ALL {
                for dark in [false, true] {
                    let mode = if dark {
                        ThemeMode::Dark
                    } else {
                        ThemeMode::Light
                    };
                    let theme = Theme::current(mode);
                    let parts = Parts::new(&r, shot, language, dark).unwrap();
                    let (w, h) = (1235, 1565);
                    let pixels =
                        (r.scene_pixels(shot.picture(dark), w, h, Some((&parts.scene(), &theme))))
                            .unwrap();
                    let name = format!("{dir}/{shot:?}-{}-{dark}.bgra", language.code());
                    std::fs::write(name, pixels).unwrap();
                }
            }
        }
    }
}
