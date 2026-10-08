# WinDict

A tiny always-on dictionary for Windows. Select a word in any app, press
**Ctrl+Alt+D**, and a frosted-glass card with the definition slides in next to it.

- **Works offline.** A full English dictionary (Open English WordNet, ~150k
  words and phrases) is built in; inflections like "ran" or "geese" resolve to
  their base form. The online dictionary only fills gaps: pronunciations, and
  words or parts of speech WordNet doesn't have.
- **Native and light.** Rust + Win32 + Windows.UI.Composition. No WebView, no
  runtime, no async framework. While idle it's a tray icon and a hotkey; the
  GPU resources are released a few minutes after the last lookup.
- **Fluent look.** Acrylic blur from the desktop behind the card, Windows 11
  fonts (falls back to Segoe UI on Windows 10), follows light/dark mode and
  your accent color, soft shadow, hairline border.
- **Fluid motion.** Every animation (fade/scale-in, height changes when the
  definition arrives, smooth scrolling, the loading dots) runs on the system
  compositor, so it stays smooth regardless of what the app is doing.
- **Stays out of the way.** The card never takes focus from the app you're
  reading in. It closes with Esc, a click elsewhere, or switching apps.

## Using it

1. Run `windict.exe`. An "Aa" icon appears in the tray.
2. Select a word (or a short phrase) anywhere and press **Ctrl+Alt+D**.
3. Scroll with the mouse wheel if the entry is long.

Click the tray icon for **Start with Windows**, **Edit settings…**,
**Reload settings** and **Quit**.

### Settings

`%APPDATA%\WinDict\config.json` (created on first run):

```json
{
  "hotkey": "Ctrl+Alt+D",
  "theme": "system",
  "max_definitions": 4,
  "max_synonyms": 6,
  "use_ui_automation": true,
  "online_lookup": true
}
```

| Key | Meaning |
| --- | --- |
| `hotkey` | Any combination of `Ctrl`, `Alt`, `Shift`, `Win` plus a key: letters, digits, `F1`–`F24`, `Space`, `Insert`, arrows, punctuation… |
| `theme` | `system`, `dark` or `light`. |
| `max_definitions` | Definitions shown per part of speech. |
| `max_synonyms` | Synonyms shown per part of speech (`0` hides them). |
| `use_ui_automation` | Read the selection via UI Automation first (see below). |
| `online_lookup` | Use dictionaryapi.dev for pronunciations and words missing offline. `false` = fully offline. |

Use **Reload settings** in the tray menu after editing.

## Download

Grab `WinDict-<version>.exe` from the
[latest release](https://github.com/BosioPietro/WinDict/releases/latest) and
run it. No installer; it's a single file.

Windows SmartScreen may warn about an unrecognised app the first time, because
the executable isn't code-signed: choose **More info → Run anyway**.

## Building

Requires Windows 10 1903+ and the Rust MSVC toolchain
(`rustup default stable-msvc`, with the Visual Studio C++ Build Tools).

```powershell
cargo build --release
.\target\release\windict.exe
```

Every push is also built by GitHub Actions; the `windict` artifact on each run
is the ready-to-use `.exe`.

### Publishing a release

Bump `version` in `Cargo.toml`, commit, then tag and push:

```sh
git tag v0.2.0
git push origin v0.2.0
```

The `release` workflow builds the exe and publishes it as a GitHub Release.

## How it works

| File | What it does |
| --- | --- |
| `src/main.rs` | Single-instance guard, DPI awareness, WinRT + dispatcher queue setup. |
| `src/app.rs` | The controller: hotkey → capture → lookup → card; tray; light-dismiss hooks; idle teardown. |
| `src/selection.rs` | Gets the selected text: UI Automation `TextPattern` first, then a simulated Ctrl+C that saves and restores your clipboard. |
| `src/offline.rs` | Reads the built-in dictionary straight from the executable (binary search + one small compressed block per lookup, ~0.3 ms) and resolves inflections. |
| `src/dictionary.rs` | Entry types; the [Free Dictionary API](https://dictionaryapi.dev) (Wiktionary data) client; merging online data into offline entries. |
| `tools/build_dict.py` | Converts WordNet into `data/english.wdict` (see below). |
| `src/http.rs` | A ~100-line HTTPS GET on WinHTTP (system TLS and proxy settings, no extra crates). |
| `src/popup.rs` | The card: composition visual tree, placement next to the selection, all animations. |
| `src/render.rs` | DirectWrite layout + Direct2D drawing of the text into composition surfaces; the shadow. |
| `src/theme.rs` | Light/dark, transparency setting and accent color. |
| `src/tray.rs`, `src/autostart.rs`, `src/hotkey.rs`, `src/config.rs` | Tray icon/menu, Run-key autostart, hotkey parsing, settings. |

### Lookup flow

1. The built-in dictionary is checked first. If it knows the word, the card
   appears immediately, with no loading state.
2. In the background the online dictionary is asked for what's missing. If it
   adds something (usually the pronunciation), it fades into the card in place;
   if it fails or you're offline, nothing changes.
3. If the word isn't in the built-in dictionary at all, the card shows a
   loading animation until the online answer (or "not found") arrives.

Online answers are cached for the session.

### The built-in dictionary

`data/english.wdict` (~6.5 MB) is generated from
[Open English WordNet](https://en-word.net/) and embedded in the exe. It is
never loaded as a whole: Windows pages in only the few KB a lookup touches,
and drops them again when idle. To rebuild it (e.g. for a newer WordNet):

```sh
curl -LO https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/corpora/english_wordnet.zip
unzip english_wordnet.zip
python tools/build_dict.py english_wordnet data/english.wdict
```

### Selection capture notes

- UI Automation works without touching the clipboard and tells WinDict where
  the selection is, so the card appears right under the word. Most modern apps
  (Edge/Chrome, Word, Notepad, VS Code, Windows Terminal…) support it.
- Elsewhere it falls back to Ctrl+C. The previous clipboard contents are put
  back afterwards, but the looked-up word may show up in clipboard history
  (Win+V). In terminals the fallback is skipped (Ctrl+C would interrupt the
  running program).
- Windows doesn't let a normal app send keystrokes to an app running as
  administrator, so the fallback can't read selections there.

## Limitations / ideas

- English only. Other languages could be added as more `.wdict` files.
- No pronunciation audio yet (the API provides it).

## Credits

Dictionary data: [Open English WordNet](https://en-word.net/) (CC BY 4.0),
derived from Princeton WordNet 3.1 (see `data/LICENSE-WordNet.txt`), and
[Wiktionary](https://www.wiktionary.org/) via [dictionaryapi.dev](https://dictionaryapi.dev).
