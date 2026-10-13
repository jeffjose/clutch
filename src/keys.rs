//! The names of keys, and the chords and sequences written with them.

use evdev::KeyCode;
use std::str::FromStr;

pub const CTRL: u8 = 1;
pub const SHIFT: u8 = 2;
pub const ALT: u8 = 4;
pub const SUPER: u8 = 8;

pub const KEY_ESC: u16 = 1;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_ENTER: u16 = 28;
pub const KEY_LEFTCTRL: u16 = 29;

/// A key with the modifiers held as it goes down.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Chord {
    pub mods: u8,
    pub code: u16,
}

/// Which modifier a key is, or 0. The left and right of each are one.
pub fn modifier_of(code: u16) -> u8 {
    match code {
        29 | 97 => CTRL,
        42 | 54 => SHIFT,
        56 | 100 => ALT,
        125 | 126 => SUPER,
        _ => 0,
    }
}

fn modifier_named(name: &str) -> Option<u8> {
    Some(match name {
        "ctrl" | "control" => CTRL,
        "shift" => SHIFT,
        "alt" => ALT,
        "super" | "meta" | "win" | "cmd" => SUPER,
        _ => return None,
    })
}

/// Other names for keys, beside the kernel's own (KEY_SPACE is "space").
const ALIASES: &[(&str, &str)] = &[
    ("escape", "esc"),
    ("return", "enter"),
    ("print", "sysrq"),
    ("printscreen", "sysrq"),
    ("del", "delete"),
    ("ins", "insert"),
    ("pgup", "pageup"),
    ("pgdn", "pagedown"),
    ("period", "dot"),
    ("menu", "compose"),
    ("-", "minus"),
    ("=", "equal"),
    (",", "comma"),
    (".", "dot"),
    ("/", "slash"),
    ("\\", "backslash"),
    (";", "semicolon"),
    ("'", "apostrophe"),
    ("`", "grave"),
    ("[", "leftbrace"),
    ("]", "rightbrace"),
];

pub fn key_named(name: &str) -> Option<u16> {
    let name = ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name, |(_, key)| key);
    let code = KeyCode::from_str(&format!("KEY_{}", name.to_ascii_uppercase())).ok()?;
    Some(code.code())
}

/// "ctrl+shift+space": any modifiers, then one key that is not one.
pub fn parse_chord(text: &str) -> Result<Chord, String> {
    let mut parts: Vec<&str> = text.split('+').collect();
    let key = parts.pop().unwrap_or("").to_ascii_lowercase();
    let mut mods = 0;
    for part in parts {
        let part = part.to_ascii_lowercase();
        mods |= modifier_named(&part).ok_or(format!("\"{part}\" is not a modifier"))?;
    }
    if modifier_named(&key).is_some() {
        return Err(format!("\"{text}\" needs a key after its modifiers"));
    }
    let code = key_named(&key).ok_or(format!("\"{key}\" is not a key"))?;
    Ok(Chord { mods, code })
}

/// "super+space t": chords pressed one after another.
pub fn parse_sequence(text: &str) -> Result<Vec<Chord>, String> {
    let chords: Vec<Chord> = text
        .split_whitespace()
        .map(parse_chord)
        .collect::<Result<_, _>>()?;
    if chords.is_empty() {
        return Err("no keys".into());
    }
    Ok(chords)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords() {
        assert_eq!(parse_chord("ctrl+space"), Ok(Chord { mods: CTRL, code: 57 }));
        assert_eq!(parse_chord("Ctrl+Shift+A"), Ok(Chord { mods: CTRL | SHIFT, code: 30 }));
        assert_eq!(parse_chord("super+return"), Ok(Chord { mods: SUPER, code: 28 }));
        assert_eq!(parse_chord("print"), Ok(Chord { mods: 0, code: 99 }));
        assert_eq!(parse_chord("alt+/"), Ok(Chord { mods: ALT, code: 53 }));
        assert_eq!(parse_chord("f13"), Ok(Chord { mods: 0, code: 183 }));
        assert!(parse_chord("ctrl+alt").is_err());
        assert!(parse_chord("hyper+a").is_err());
        assert!(parse_chord("ctrl+nosuchkey").is_err());
    }

    #[test]
    fn sequences() {
        let seq = parse_sequence("super+space t").unwrap();
        assert_eq!(seq, vec![Chord { mods: SUPER, code: 57 }, Chord { mods: 0, code: 20 }]);
        assert!(parse_sequence("  ").is_err());
    }
}
