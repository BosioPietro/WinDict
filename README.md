# WinDict

**Select a word. Press Ctrl+Alt+D. Read the definition.**

WinDict is a dictionary that lives in your Windows tray and appears exactly
where you are reading: next to the word you selected, in any app. No browser
tab, no copy and paste, no waiting.

## Light enough to forget about

- **2 MB in the background, 4 MB while showing a definition.** Most apps use
  more than that to draw their title bar.
- **Instant answers.** A dictionary of 150,000 English words and phrases is
  built in and looked up in well under a millisecond. The card appears before
  your finger leaves the keys.
- **Works offline.** The internet is only used to add pronunciations and the
  rare word the built-in dictionary doesn't know.
- **One small file.** Native Rust, no browser engine, no runtime, no installer.

## Feels like part of Windows

A frosted-glass card that follows your light or dark theme and accent color,
with smooth, GPU-driven animations. It never takes focus away from what you are
doing, understands inflections ("ran" shows "run"), and closes with Esc or a
click anywhere else.

## Get started

Download `WinDict-<version>.exe` from the
[latest release](https://github.com/BosioPietro/WinDict/releases/latest) and
run it. Select a word anywhere, press **Ctrl+Alt+D**, and that's it.

Click the tray icon for **Start with Windows**, settings, and **Quit**.
The first time, Windows SmartScreen may ask for confirmation because the app
isn't commercially signed: choose **More info**, then **Run anyway**.

## Settings

Stored in `%APPDATA%\WinDict\config.json`; use **Reload settings** in the tray
menu after editing.

| Setting | Default | |
| --- | --- | --- |
| `hotkey` | `"Ctrl+Alt+D"` | Any combination of Ctrl, Alt, Shift, Win and a key. |
| `theme` | `"system"` | `"system"`, `"dark"` or `"light"`. |
| `max_definitions` | `4` | Definitions per part of speech. |
| `max_synonyms` | `6` | Synonyms per part of speech; `0` hides them. |
| `online_lookup` | `true` | `false` keeps WinDict fully offline. |
| `use_ui_automation` | `true` | Read selections without touching the clipboard where apps allow it. |

## Showing the card above the Start menu

Windows keeps Start and Search above every ordinary window, so by default
WinDict closes them before showing a definition. To have the card appear on
top of them instead, install the UI access edition. Windows grants that
permission only to signed apps in a protected folder, and the included script
handles all of it. From an administrator PowerShell, in the release folder or
in a clone of this repository:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install-uiaccess.ps1
```

The script creates a certificate that is trusted on this machine only, signs
WinDict with it, installs it to `C:\Program Files\WinDict` and starts it. The
certificate's private key is deleted right after signing, so it can never sign
anything else. Run it again to update, or with `-Uninstall` to remove it.

## Credits

Definitions from [Open English WordNet](https://en-word.net/) (CC BY 4.0,
derived from Princeton WordNet; see `data/LICENSE-WordNet.txt`) and
[Wiktionary](https://www.wiktionary.org/) via
[dictionaryapi.dev](https://dictionaryapi.dev). WinDict is MIT licensed.
