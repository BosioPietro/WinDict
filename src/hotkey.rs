//! Parsing of hotkey strings such as "Ctrl+Alt+D".

use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: HOT_KEY_MODIFIERS,
    pub vk: u32,
}

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
    if modifiers.0 == 0 && !(0x70..=0x87).contains(&vk) {
        return Err(format!(
            "\"{text}\" needs at least one modifier (Ctrl, Alt, Shift, Win)"
        ));
    }
    Ok(Hotkey { modifiers, vk })
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
    let vk = match key {
        "space" => VK_SPACE,
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "insert" | "ins" => VK_INSERT,
        "delete" | "del" => VK_DELETE,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "pause" => VK_PAUSE,
        "`" | "backquote" | "grave" => VK_OEM_3,
        "-" | "minus" => VK_OEM_MINUS,
        "=" | "equals" | "plus" => VK_OEM_PLUS,
        "," | "comma" => VK_OEM_COMMA,
        "." | "period" => VK_OEM_PERIOD,
        "/" | "slash" => VK_OEM_2,
        ";" | "semicolon" => VK_OEM_1,
        "'" | "quote" => VK_OEM_7,
        "[" => VK_OEM_4,
        "]" => VK_OEM_6,
        "\\" | "backslash" => VK_OEM_5,
        _ => return None,
    };
    Some(vk.0 as u32)
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
        assert!(parse("Ctrl+Alt").is_err());
        assert!(parse("D").is_err());
        assert!(parse("Ctrl+Foo").is_err());
    }
}
