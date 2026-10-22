# WinDict

A small Windows tray app for looking up words. Select a word in any app, press
**Ctrl+Alt+D**, and its definition shows up in a card next to the selection.
Press Esc or click elsewhere to close it.

With nothing selected, the shortcut opens a small box to type a word into
instead, for text that can't be selected: in an image, a protected PDF or a
game.

Each part of speech lists similar words and, where there are any, opposite
ones (“good”: “bad”). Click one to open its definition in the same card; the arrow next to
the word, Backspace, Alt+← or the mouse's back button returns to the
previous one. Tab and Shift+Tab move between the synonyms and the source
links in order, and Enter opens the one that's ringed.

With online lookup on, the translate button at the top right of the card (or
**Ctrl+T**, while the card is open) turns it into a translation.

## Footprint

- About 3 MB of memory while idle and ~20 MB while a card is open.
- An English dictionary (Open English WordNet, about 150,000 words and
  phrases) is built into the executable, so most lookups never touch the
  network and take under a millisecond. Italian (from Wikizionario, about
  75,000 words) is downloaded, once, when it's chosen in the settings.
- The online dictionary is only used for pronunciations and for words the
  built-in one doesn't have, and translations only when asked for. Both can be
  turned off.
- A single executable written in Rust against the native Windows APIs. No
  installer and no runtime to install.

Inflected forms are resolved to their base word ("ran" shows "run"), and the
card follows the system light/dark theme and accent color.

## Get started

Download `WinDict-<version>.exe` from the
[latest release](https://github.com/BosioPietro/WinDict/releases/latest) and
run it. Select a word anywhere, press **Ctrl+Alt+D**, and that's it.

Click the tray icon for **Start with Windows**, settings, and **Quit**.
The first time, Windows SmartScreen may ask for confirmation because the app
isn't commercially signed: choose **More info**, then **Run anyway**.

## Settings

Click the tray icon, then **Settings…**. Changes apply as you make them. To
change the shortcut, click it and press the new one; Esc keeps the old one.

The settings are stored in `%APPDATA%\WinDict\config.json`. To edit the file
by hand, use **Open config.json** at the bottom of the settings window, then
**Reload settings** in the tray menu.

## Showing the card above the Start menu

Windows keeps Start and Search above every ordinary window, so by default
WinDict closes them before showing a definition. To have the card appear on
top of them instead, install the UI access edition. Windows grants that
permission only to signed apps in a protected folder, and the included script
handles all of it. Download `install-uiaccess.ps1` from the
[latest release](https://github.com/BosioPietro/WinDict/releases/latest) and
run it from an administrator PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File install-uiaccess.ps1
```

It downloads the latest UI access build, so no Rust toolchain is needed. To
install your own build from a clone of this repository instead, run
`scripts\install-uiaccess-local.ps1`, which builds it with Cargo.

The script creates a certificate that is trusted on this machine only, signs
WinDict with it, installs it to `C:\Program Files\WinDict` and starts it. The
certificate's private key is deleted right after signing, so it can never sign
anything else. Run it again to update, or with `-Uninstall` to remove it.

## Building from source

WinDict needs Windows and a Rust toolchain (1.88 or newer).

```powershell
cargo build --release        # target\release\windict.exe
cargo test                   # unit tests, including the bundled dictionary
cargo clippy --all-targets -- -D warnings
```

## Credits

Definitions from [Open English WordNet](https://en-word.net/) (CC BY 4.0,
derived from Princeton WordNet; see `data/LICENSE-WordNet.txt`) and
[Wiktionary](https://www.wiktionary.org/), via
[dictionaryapi.dev](https://dictionaryapi.dev) with a direct Wiktionary
fallback. The Italian dictionary, and Italian lookups online, are from
[Wikizionario](https://it.wiktionary.org/) through
[wiktextract](https://kaikki.org/itwiktionary/), with Wikizionario's own
API as a fallback (CC BY-SA 4.0; see
`data/LICENSE-Wiktionary.txt`). Translations are from Google Translate's
public web endpoint, which is unofficial and may change, with
[MyMemory](https://mymemory.translated.net/) as a fallback. WinDict is MIT
licensed.
