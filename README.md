# WinDict

A small Windows tray app for looking up words. Select a word in any app, press
**Ctrl+Alt+D**, and its definition shows up in a card next to the selection.
Press Esc or click elsewhere to close it.

With nothing selected, the shortcut opens a small box to type a word into
instead, for text that can't be selected: in an image, a protected PDF or a
game. It opens at the text cursor if the app reports one, and in the middle
of the screen otherwise. Words from the bundled dictionary
are suggested as you type, typos and all (“recieve” finds “receive”);
pick one with Tab, Shift+Tab, the arrow keys or
the mouse. Press Enter to look it up; Esc or the shortcut
again closes it. A word that isn't found offers the one it likely
misspells (“emobdy”: “Did you mean embody?”), a link like the synonyms.

Click a synonym to open its definition in the same card; the arrow next to
the word, Backspace, Alt+← or the mouse's back button returns to the
previous one. Tab and Shift+Tab move between the synonyms and the source
links in order, and Enter opens the one that's ringed.

## Footprint

- About 2 MB of memory while idle and 4 MB while a card is open.
- An English dictionary (Open English WordNet, about 150,000 words and
  phrases) is built into the executable, so most lookups never touch the
  network and take under a millisecond.
- The online dictionary is only used for pronunciations and for words the
  built-in one doesn't have. It can be turned off.
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
[Wiktionary](https://www.wiktionary.org/), via
[dictionaryapi.dev](https://dictionaryapi.dev) with a direct Wiktionary
fallback. WinDict is MIT licensed.
