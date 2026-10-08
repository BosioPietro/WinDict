# WinDict

A tiny always-on dictionary for Windows. Select a word in any app, press
**Ctrl+Alt+D**, and a frosted-glass card with the definition slides in next to it.

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

Right-click the tray icon for **Start with Windows**, **Edit settings…**,
**Reload settings** and **Quit**. Left-click it to check it's alive.

### Settings

`%APPDATA%\WinDict\config.json` (created on first run):

```json
{
  "hotkey": "Ctrl+Alt+D",
  "theme": "system",
  "max_definitions": 4,
  "max_synonyms": 6,
  "use_ui_automation": true
}
```

| Key | Meaning |
| --- | --- |
| `hotkey` | Any combination of `Ctrl`, `Alt`, `Shift`, `Win` plus a key: letters, digits, `F1`–`F24`, `Space`, `Insert`, arrows, punctuation… |
| `theme` | `system`, `dark` or `light`. |
| `max_definitions` | Definitions shown per part of speech. |
| `max_synonyms` | Synonyms shown per part of speech (`0` hides them). |
| `use_ui_automation` | Read the selection via UI Automation first (see below). |

Use **Reload settings** in the tray menu after editing.

## Building

Requires Windows 10 1903+ and the Rust MSVC toolchain
(`rustup default stable-msvc`, with the Visual Studio C++ Build Tools).

```powershell
cargo build --release
.\target\release\windict.exe
```

Every push is also built by GitHub Actions; the `windict` artifact on each run
is the ready-to-use `.exe`.

## How it works

| File | What it does |
| --- | --- |
| `src/main.rs` | Single-instance guard, DPI awareness, WinRT + dispatcher queue setup. |
| `src/app.rs` | The controller: hotkey → capture → lookup → card; tray; light-dismiss hooks; idle teardown. |
| `src/selection.rs` | Gets the selected text: UI Automation `TextPattern` first, then a simulated Ctrl+C that saves and restores your clipboard. |
| `src/dictionary.rs` | Calls [Free Dictionary API](https://dictionaryapi.dev) (Wiktionary data) and merges the entries. |
| `src/http.rs` | A ~100-line HTTPS GET on WinHTTP (system TLS and proxy settings, no extra crates). |
| `src/popup.rs` | The card: composition visual tree, placement next to the selection, all animations. |
| `src/render.rs` | DirectWrite layout + Direct2D drawing of the text into composition surfaces; the shadow. |
| `src/theme.rs` | Light/dark, transparency setting and accent color. |
| `src/tray.rs`, `src/autostart.rs`, `src/hotkey.rs`, `src/config.rs` | Tray icon/menu, Run-key autostart, hotkey parsing, settings. |

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

- English only, and needs an internet connection (definitions are cached for
  the session). An offline dictionary (e.g. WordNet) or other languages could
  be added behind the same `Lookup` type.
- No pronunciation audio yet (the API provides it).
