//! Where clutch may not hold the keyboards, GNOME can be given the
//! bindings as shortcuts of its own, which needs no more than you have.
//! GNOME does the listening then: chords only, and only where no
//! shortcut of GNOME's is on the same keys.

use crate::config::Binding;
use crate::keys::{parse_sequence, Chord, ALT, CTRL, SHIFT, SUPER};
use evdev::KeyCode;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

const LIST: &str = "org.gnome.settings-daemon.plugins.media-keys";
const ENTRY: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
/// Our shortcuts are the ones under this, numbered. The rest are yours.
const OURS: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/clutch";
/// Where GNOME keeps the shortcuts that would be in the way of ours.
const THEIRS: &[&str] = &[
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.shell.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
    LIST,
];

pub struct Shortcut {
    keys: String,
    accelerator: String,
    command: String,
}

/// Keys whose name in GNOME is not the kernel's.
const NAMES: &[(&str, &str)] = &[
    ("ENTER", "Return"),
    ("ESC", "Escape"),
    ("TAB", "Tab"),
    ("BACKSPACE", "BackSpace"),
    ("DELETE", "Delete"),
    ("INSERT", "Insert"),
    ("HOME", "Home"),
    ("END", "End"),
    ("PAGEUP", "Page_Up"),
    ("PAGEDOWN", "Page_Down"),
    ("UP", "Up"),
    ("DOWN", "Down"),
    ("LEFT", "Left"),
    ("RIGHT", "Right"),
    ("SYSRQ", "Print"),
    ("DOT", "period"),
    ("LEFTBRACE", "bracketleft"),
    ("RIGHTBRACE", "bracketright"),
    ("COMPOSE", "Menu"),
    ("PAUSE", "Pause"),
    ("SCROLLLOCK", "Scroll_Lock"),
    ("CAPSLOCK", "Caps_Lock"),
];
/// And those whose name is the kernel's, in small letters.
const SAME: &[&str] = &[
    "SPACE", "MINUS", "EQUAL", "COMMA", "SLASH", "BACKSLASH", "SEMICOLON", "APOSTROPHE", "GRAVE",
];

/// A chord as GNOME writes one: "<Control><Shift>space".
pub fn accelerator(chord: Chord) -> Option<String> {
    let name = format!("{:?}", KeyCode::new(chord.code));
    let name = name.strip_prefix("KEY_")?;
    let letter_or_digit = name.len() == 1;
    let function = name.starts_with('F') && name[1..].parse::<u8>().is_ok();
    let key = if letter_or_digit || SAME.contains(&name) {
        name.to_ascii_lowercase()
    } else if function {
        name.to_string()
    } else {
        NAMES.iter().find(|(kernel, _)| *kernel == name)?.1.to_string()
    };
    let mut text = String::new();
    for (modifier, written) in [(CTRL, "<Control>"), (SHIFT, "<Shift>"), (ALT, "<Alt>"), (SUPER, "<Super>")] {
        if chord.mods & modifier != 0 {
            text += written;
        }
    }
    Some(text + &key)
}

/// The bindings GNOME can take, and a line for each that it cannot.
pub fn shortcuts(bindings: &[Binding]) -> (Vec<Shortcut>, Vec<String>) {
    let mut shortcuts = Vec::new();
    let mut notes = Vec::new();
    for binding in bindings {
        let keys = binding.keys.clone();
        let sequence = parse_sequence(&keys).unwrap_or_default();
        if sequence.len() != 1 {
            notes.push(format!("{keys}: not bound here, GNOME has no sequences"));
        } else if let Some(accelerator) = accelerator(sequence[0]) {
            // GNOME runs a program with its arguments; a binding is a
            // line of shell.
            let command = format!("sh -c {}", shell_quoted(&binding.command));
            shortcuts.push(Shortcut { keys, accelerator, command });
        } else {
            notes.push(format!("{keys}: not bound here, GNOME has no shortcut on that key"));
        }
    }
    (shortcuts, notes)
}

fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A string as gsettings reads one.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// The strings in what gsettings prints: ['a', 'b'] gives a and b.
fn strings(printed: &str) -> Vec<String> {
    printed.split('\'').skip(1).step_by(2).map(str::to_string).collect()
}

/// An accelerator as its modifiers and key, however it was written.
fn normal(accelerator: &str) -> (u8, String) {
    let mut mods = 0;
    let mut rest = accelerator;
    while let Some((modifier, after)) = rest.strip_prefix('<').and_then(|r| r.split_once('>')) {
        mods |= match modifier.to_ascii_lowercase().as_str() {
            "control" | "primary" | "ctrl" => CTRL,
            "shift" => SHIFT,
            "alt" | "mod1" => ALT,
            "super" | "mod4" | "meta" => SUPER,
            _ => 0,
        };
        rest = after;
    }
    (mods, rest.to_ascii_lowercase())
}

/// A line for each of ours that a shortcut of GNOME's is already on,
/// from what `gsettings list-recursively` printed.
fn clashes(shortcuts: &[Shortcut], listed: &str) -> Vec<String> {
    let mut notes = Vec::new();
    for line in listed.lines() {
        let mut parts = line.splitn(3, ' ');
        let (Some(schema), Some(key), Some(value)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        for theirs in strings(value) {
            for ours in shortcuts.iter().filter(|s| normal(&s.accelerator) == normal(&theirs)) {
                notes.push(format!(
                    "{}: GNOME has these keys for {key}, and wins. To give them up:\n      gsettings set {schema} {key} \"[]\"",
                    ours.keys
                ));
            }
        }
    }
    notes
}

fn gsettings(arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("gsettings")
        .args(arguments)
        .output()
        .map_err(|e| format!("gsettings: {e}"))?;
    if !output.status.success() {
        return Err(format!("gsettings: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Make GNOME's shortcuts of ours exactly these, and leave every other
/// one alone. Returns what there is to say about it.
pub fn sync(shortcuts: &[Shortcut]) -> Vec<String> {
    let listed = match gsettings(&["get", LIST, "custom-keybindings"]) {
        Ok(listed) => strings(&listed),
        Err(problem) => return vec![problem],
    };
    let (old, mut paths): (Vec<String>, Vec<String>) =
        listed.into_iter().partition(|path| path.starts_with(OURS));

    let mut notes = Vec::new();
    for (number, shortcut) in shortcuts.iter().enumerate() {
        let path = format!("{OURS}{number}/");
        let entry = format!("{ENTRY}:{path}");
        for (key, value) in [
            ("name", format!("clutch: {}", shortcut.keys)),
            ("command", shortcut.command.clone()),
            ("binding", shortcut.accelerator.clone()),
        ] {
            if let Err(problem) = gsettings(&["set", &entry, key, &quoted(&value)]) {
                notes.push(format!("{}: {problem}", shortcut.keys));
            }
        }
        paths.push(path);
    }

    let list = match paths.is_empty() {
        true => "@as []".to_string(),
        false => format!("[{}]", paths.iter().map(|p| quoted(p)).collect::<Vec<_>>().join(", ")),
    };
    if let Err(problem) = gsettings(&["set", LIST, "custom-keybindings", &list]) {
        notes.push(problem);
    }
    for path in old.iter().filter(|path| !paths.contains(path)) {
        let _ = gsettings(&["reset-recursively", &format!("{ENTRY}:{path}")]);
    }

    for schema in THEIRS {
        // Not every GNOME has every one of these.
        if let Ok(listed) = gsettings(&["list-recursively", schema]) {
            notes.extend(clashes(shortcuts, &listed));
        }
    }
    notes
}

/// Take ours out of GNOME's shortcuts.
pub fn clear() {
    sync(&[]);
}

static LATEST: AtomicU64 = AtomicU64::new(0);
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// `sync` without waiting for it: gsettings is a program run a few times
/// for each binding. What there is to say goes after `notes`.
pub fn sync_later(shortcuts: Vec<Shortcut>, said: Vec<String>, notes: Arc<Mutex<Vec<String>>>) {
    let turn = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
    *notes.lock().unwrap() = said.clone();
    thread::spawn(move || {
        let _one = ONE_AT_A_TIME.lock().unwrap();
        // Saved again while we waited: the later one is the one to do.
        if LATEST.load(Ordering::SeqCst) != turn {
            return;
        }
        let mut said = said;
        said.extend(sync(&shortcuts));
        if LATEST.load(Ordering::SeqCst) == turn {
            *notes.lock().unwrap() = said;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::parse_chord;

    fn written(keys: &str) -> Option<String> {
        accelerator(parse_chord(keys).unwrap())
    }

    #[test]
    fn chords_as_gnome_writes_them() {
        assert_eq!(written("ctrl+space").as_deref(), Some("<Control>space"));
        assert_eq!(written("ctrl+shift+a").as_deref(), Some("<Control><Shift>a"));
        assert_eq!(written("super+return").as_deref(), Some("<Super>Return"));
        assert_eq!(written("alt+f4").as_deref(), Some("<Alt>F4"));
        assert_eq!(written("print").as_deref(), Some("Print"));
        assert_eq!(written("super+1").as_deref(), Some("<Super>1"));
        assert_eq!(written("ctrl+.").as_deref(), Some("<Control>period"));
        assert_eq!(written("ctrl+kp5"), None);
    }

    fn binding(keys: &str, command: &str) -> Binding {
        Binding { keys: keys.into(), command: command.into() }
    }

    #[test]
    fn what_gnome_can_take() {
        let (shortcuts, notes) = shortcuts(&[
            binding("ctrl+space", "echo it's"),
            binding("super+space t", "xterm"),
            binding("ctrl+kp5", "xterm"),
        ]);
        assert_eq!(shortcuts.len(), 1);
        assert_eq!(shortcuts[0].command, r"sh -c 'echo it'\''s'");
        assert_eq!(notes.len(), 2);
        assert!(notes[0].starts_with("super+space t: ") && notes[0].contains("sequences"));
    }

    #[test]
    fn what_gsettings_prints_and_reads() {
        assert_eq!(strings("['/a/', '/b/']\n"), ["/a/", "/b/"]);
        assert!(strings("@as []\n").is_empty());
        assert_eq!(quoted(r"sh -c 'a\b'"), r"'sh -c \'a\\b\''");
    }

    #[test]
    fn a_shortcut_of_gnomes_on_the_same_keys() {
        assert_eq!(normal("<Primary><Shift>Space"), normal("<Shift><Control>space"));
        assert_eq!(normal("<Super>space"), (SUPER, "space".into()));
        let (shortcuts, _) = shortcuts(&[binding("super+space", "x"), binding("ctrl+space", "y")]);
        let listed = "org.gnome.desktop.wm.keybindings switch-input-source ['<Super>space', 'XF86Keyboard']\n\
                      org.gnome.desktop.wm.keybindings close ['<Alt>F4']\n";
        let notes = clashes(&shortcuts, listed);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("super+space: "));
        assert!(notes[0].contains("gsettings set org.gnome.desktop.wm.keybindings switch-input-source"));
    }
}
