//! Parsing of hotkey strings such as "Ctrl+Alt+D", naming them back, and
//! reading the keys held as one is pressed.

use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: HOT_KEY_MODIFIERS,
    pub vk: u32,
}

/// Keys with names, each under the name it's shown with, then other
/// names it's known by.
const NAMED_KEYS: &[(&[&str], VIRTUAL_KEY)] = &[
    (&["Space"], VK_SPACE),
    (&["Enter", "Return"], VK_RETURN),
    (&["Tab"], VK_TAB),
    (&["Insert", "Ins"], VK_INSERT),
    (&["Delete", "Del"], VK_DELETE),
    (&["Home"], VK_HOME),
    (&["End"], VK_END),
    (&["PageUp", "PgUp"], VK_PRIOR),
    (&["PageDown", "PgDn"], VK_NEXT),
    (&["Up"], VK_UP),
    (&["Down"], VK_DOWN),
    (&["Left"], VK_LEFT),
    (&["Right"], VK_RIGHT),
    (&["Pause"], VK_PAUSE),
    (&["`", "Backquote", "Grave"], VK_OEM_3),
    (&["-", "Minus"], VK_OEM_MINUS),
    (&["=", "Equals", "Plus"], VK_OEM_PLUS),
    (&[",", "Comma"], VK_OEM_COMMA),
    (&[".", "Period"], VK_OEM_PERIOD),
    (&["/", "Slash"], VK_OEM_2),
    (&[";", "Semicolon"], VK_OEM_1),
    (&["'", "Quote"], VK_OEM_7),
    (&["["], VK_OEM_4),
    (&["]"], VK_OEM_6),
    (&["\\", "Backslash"], VK_OEM_5),
];

/// The modifiers in the order they're written.
const MODIFIERS: [(HOT_KEY_MODIFIERS, &str); 4] = [
    (MOD_WIN, "Win"),
    (MOD_CONTROL, "Ctrl"),
    (MOD_ALT, "Alt"),
    (MOD_SHIFT, "Shift"),
];

pub fn parse(text: &str) -> Result<Hotkey, String> {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    let mut vk = None;
    for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "alt" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "windows" | "super" | "meta" => modifiers |= MOD_WIN,
            key => {
                if vk.is_some() {
                    return Err(format!("\"{text}\" has more than one non-modifier key"));
                }
                vk = Some(
                    key_code(key).ok_or_else(|| format!("Unknown key \"{part}\" in \"{text}\""))?,
                );
            }
        }
    }
    let vk = vk.ok_or_else(|| format!("\"{text}\" has no key besides modifiers"))?;
    if modifiers.0 == 0 && !is_function_key(vk) {
        return Err(format!(
            "\"{text}\" needs at least one modifier (Ctrl, Alt, Shift, Win)"
        ));
    }
    Ok(Hotkey { modifiers, vk })
}

impl Hotkey {
    /// The names of its keys, modifiers first: `["Ctrl", "Alt", "D"]`.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = modifier_names(self.modifiers)
            .into_iter()
            .map(String::from)
            .collect();
        names.extend(key_name(self.vk));
        names
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.names().join("+"))
    }
}

/// The names of `modifiers`, in the order they're written.
pub fn modifier_names(modifiers: HOT_KEY_MODIFIERS) -> Vec<&'static str> {
    MODIFIERS
        .iter()
        .filter(|(m, _)| modifiers.0 & m.0 != 0)
        .map(|&(_, name)| name)
        .collect()
}

/// F1 to F24, which make shortcuts on their own.
pub fn is_function_key(vk: u32) -> bool {
    (VK_F1.0 as u32..=VK_F24.0 as u32).contains(&vk)
}

/// The name `parse` reads a key by, if it can be part of a shortcut.
pub fn key_name(vk: u32) -> Option<String> {
    let c = char::from_u32(vk)?;
    if c.is_ascii_uppercase() || c.is_ascii_digit() {
        return Some(c.to_string());
    }
    if is_function_key(vk) {
        return Some(format!("F{}", vk - VK_F1.0 as u32 + 1));
    }
    NAMED_KEYS
        .iter()
        .find(|(_, key)| key.0 as u32 == vk)
        .map(|(names, _)| names[0].to_owned())
}

fn key_code(key: &str) -> Option<u32> {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        let c = c.to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return Some(c as u32);
        }
    }
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(VK_F1.0 as u32 + n - 1);
        }
    }
    NAMED_KEYS
        .iter()
        .find(|(names, _)| names.iter().any(|n| n.eq_ignore_ascii_case(key)))
        .map(|(_, vk)| vk.0 as u32)
}

/// Whether `vk` is held down, as of the message being handled.
pub fn is_held(vk: VIRTUAL_KEY) -> bool {
    // The high bit (a negative state) means the key is down.
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

/// The modifiers held down, as of the message being handled.
pub fn held_modifiers() -> HOT_KEY_MODIFIERS {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    for (vks, modifier) in [
        (&[VK_CONTROL][..], MOD_CONTROL),
        (&[VK_MENU], MOD_ALT),
        (&[VK_SHIFT], MOD_SHIFT),
        (&[VK_LWIN, VK_RWIN], MOD_WIN),
    ] {
        if vks.iter().any(|&vk| is_held(vk)) {
            modifiers |= modifier;
        }
    }
    modifiers
}

/// Whether `vk` is Ctrl, Alt, Shift or Win, on either side.
pub fn is_modifier(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk,
        VK_CONTROL
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_MENU
            | VK_LMENU
            | VK_RMENU
            | VK_SHIFT
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_LWIN
            | VK_RWIN
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkeys() {
        let h = parse("Ctrl+Alt+D").unwrap();
        assert_eq!(h.modifiers, MOD_CONTROL | MOD_ALT);
        assert_eq!(h.vk, 'D' as u32);
        assert_eq!(parse("win + shift + f1").unwrap().vk, VK_F1.0 as u32);
        assert_eq!(parse("F9").unwrap().modifiers.0, 0);
        assert_eq!(parse("Ctrl+PgDn").unwrap().vk, VK_NEXT.0 as u32);
        assert!(parse("Ctrl+Alt").is_err());
        assert!(parse("D").is_err());
        assert!(parse("Ctrl+Foo").is_err());
    }

    #[test]
    fn names_hotkeys() {
        let named = |text| parse(text).unwrap().to_string();
        assert_eq!(named("alt+ctrl+d"), "Ctrl+Alt+D");
        assert_eq!(named("shift+win+f12"), "Win+Shift+F12");
        assert_eq!(named("ctrl+pgup"), "Ctrl+PageUp");
        assert_eq!(named("Ctrl+Plus"), "Ctrl+=");
        // What's written is read back as the same shortcut.
        for text in [
            "Ctrl+Alt+D",
            "Win+Shift+F1",
            "Ctrl+\\",
            "Alt+Space",
            "Ctrl+7",
        ] {
            let hotkey = parse(text).unwrap();
            assert_eq!(parse(&hotkey.to_string()).unwrap(), hotkey, "{text}");
        }
        assert_eq!(key_name(VK_LWIN.0 as u32), None);
    }
}
