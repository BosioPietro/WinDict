//! Saying a word out loud, in its own language, with the voices Windows has
//! (Windows.Media.SpeechSynthesis). The speech is made and played on a
//! thread of its own, so the card never waits on it.

use std::sync::mpsc::{channel, Sender};

use windows::core::{Result, HSTRING};
use windows::Media::Core::MediaSource;
use windows::Media::Playback::{MediaPlayer, MediaPlayerAudioCategory};
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

/// The voices installed, as their language ("en-US") and id.
pub struct Voices(Vec<(String, HSTRING)>);

impl Voices {
    /// The voices installed now, the default one first, so that it's the
    /// one picked for its language. None, if they can't be listed.
    pub fn load() -> Voices {
        let list = || -> Result<Vec<(String, HSTRING)>> {
            let default = SpeechSynthesizer::DefaultVoice().and_then(|v| v.Id()).ok();
            let mut voices = Vec::new();
            for voice in SpeechSynthesizer::AllVoices()? {
                voices.push((voice.Language()?.to_string(), voice.Id()?));
            }
            voices.sort_by_key(|(_, id)| Some(id) != default.as_ref());
            Ok(voices)
        };
        Voices(list().unwrap_or_default())
    }

    /// The id of a voice speaking the language with code `language` ("en",
    /// "zh-CN"), if there's one.
    pub fn for_language(&self, language: &str) -> Option<&HSTRING> {
        self.0
            .iter()
            .find(|(voice, _)| speaks(voice, language))
            .map(|(_, id)| id)
    }
}

/// Whether a voice in `voice` ("en-US") speaks the language with code
/// `language`: "en" is any English, "zh-CN" only that Chinese.
fn speaks(voice: &str, language: &str) -> bool {
    let (voice, language) = (voice.to_ascii_lowercase(), language.to_ascii_lowercase());
    voice == language
        || voice
            .strip_prefix(&language)
            .is_some_and(|rest| rest.starts_with('-'))
}

/// Says words, one at a time: a new one cuts off the one being said.
#[derive(Default)]
pub struct Speaker {
    /// To the thread that speaks, once there's been something to say.
    tx: Option<Sender<(String, HSTRING)>>,
}

impl Speaker {
    /// Says `text` with the voice with id `voice`.
    pub fn say(&mut self, text: &str, voice: &HSTRING) {
        let mut request = (text.to_string(), voice.clone());
        // A thread that's gone, having failed to start speaking, is started again.
        if let Some(tx) = &self.tx {
            match tx.send(request) {
                Ok(()) => return,
                Err(failed) => request = failed.0,
            }
        }
        let (tx, rx) = channel::<(String, HSTRING)>();
        let _ = tx.send(request);
        self.tx = Some(tx);
        std::thread::spawn(move || {
            let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
            let Ok((synthesizer, player)) = voice_and_player() else {
                return;
            };
            // Only the latest of those asked meanwhile is worth saying.
            while let Ok(mut request) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }
                let (text, voice) = request;
                let _ = say(&synthesizer, &player, &text, &voice);
            }
        });
    }
}

fn voice_and_player() -> Result<(SpeechSynthesizer, MediaPlayer)> {
    let player = MediaPlayer::new()?;
    player.SetAudioCategory(MediaPlayerAudioCategory::Speech)?;
    Ok((SpeechSynthesizer::new()?, player))
}

fn say(
    synthesizer: &SpeechSynthesizer,
    player: &MediaPlayer,
    text: &str,
    voice: &HSTRING,
) -> Result<()> {
    if let Some(voice) = find_voice(voice)? {
        synthesizer.SetVoice(&voice)?;
    }
    let stream = synthesizer
        .SynthesizeTextToStreamAsync(&HSTRING::from(text))?
        .join()?;
    let source = MediaSource::CreateFromStream(&stream, &stream.ContentType()?)?;
    // The player keeps playing on its own; the thread waits for the next word.
    player.SetSource(&source)?;
    player.Play()
}

fn find_voice(id: &HSTRING) -> Result<Option<VoiceInformation>> {
    for voice in SpeechSynthesizer::AllVoices()? {
        if voice.Id()? == *id {
            return Ok(Some(voice));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_voices_to_languages() {
        assert!(speaks("en-US", "en"));
        assert!(speaks("en-GB", "EN"));
        assert!(speaks("it-IT", "it"));
        assert!(speaks("zh-CN", "zh-CN"));
        assert!(!speaks("zh-TW", "zh-CN"));
        assert!(!speaks("es-ES", "en"));
        // "e" is no language's prefix of another's.
        assert!(!speaks("en-US", "e"));
    }
}
