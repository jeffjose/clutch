//! Putting clutch on a machine, and taking it off again.

use crate::style::{Style, GREEN, YELLOW};
use crate::{config, extension, gnome};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

const RULE: &str = include_str!("../assets/70-clutch.rules");
pub const RULE_PATH: &str = "/etc/udev/rules.d/70-clutch.rules";
const UNIT: &str = include_str!("../assets/clutch.service");
const ICON: &str = include_str!("../assets/clutch.svg");
const ENTRY: &str = include_str!("../assets/clutch.desktop");

/// Where the icon and the menu entry go, among your own.
fn menu_paths() -> (PathBuf, PathBuf) {
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    let data = env::var_os("XDG_DATA_HOME").map_or(home.join(".local/share"), PathBuf::from);
    (data.join("icons/hicolor/scalable/apps/clutch.svg"), data.join("applications/clutch.desktop"))
}

/// The udev rule goes in, or with nothing to put, comes out; then udev
/// looks at the keyboards again. The one thing here that needs root.
const AS_ROOT: &str = r#"
if [ -n "$1" ]; then printf "%s" "$1" > "$0" && chmod 644 "$0"; else rm -f "$0"; fi
udevadm control --reload
udevadm trigger --subsystem-match=input --subsystem-match=misc --sysname-match='event*' --sysname-match=uinput
udevadm settle
"#;

fn unit_path() -> PathBuf {
    config::config_home().join("systemd/user/clutch.service")
}

fn ran(command: &mut Command) -> Result<(), String> {
    let name = command.get_program().to_string_lossy().into_owned();
    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{name}: {status}")),
        Err(e) => Err(format!("{name}: {e}")),
    }
}

fn systemctl(arguments: &[&str]) -> Result<(), String> {
    ran(Command::new("systemctl").arg("--user").args(arguments))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Put the rule in, or with no text take it out. sudo asks for the
/// password on the terminal itself; what else it has to say is said by us.
fn as_root(rule: &str, why: &str) -> bool {
    Command::new("sudo")
        .args(["-p", &format!("password, to {why} the udev rule: ")])
        .args(["sh", "-c", AS_ROOT, RULE_PATH, rule])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn setup() -> Result<(), String> {
    let style = Style::of_terminal();
    if fs::read_to_string(RULE_PATH).ok().as_deref() == Some(RULE) {
        print!("{}", style.line(GREEN, "Unchanged", "udev rule"));
    } else if as_root(RULE, "install") {
        print!("{}", style.line(GREEN, "Installed", "udev rule"));
    } else {
        print!("{}", style.warning("udev rule not installed: sudo refused, so the keyboard cannot be held here"));
    }
    // On GNOME, the part of clutch that sees the windows for it.
    if let Some((done, what)) = extension::install() {
        print!("{}", style.line(GREEN, done, what));
    }

    // The icon, and the entry that ties it to the box's window and puts
    // clutch in the list of programs.
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let entry = ENTRY.replace("Exec=clutch", &format!("Exec={}", exe.display()));
    let (icon_at, entry_at) = menu_paths();
    let same = fs::read_to_string(&entry_at).ok().as_deref() == Some(&entry) && fs::read_to_string(&icon_at).ok().as_deref() == Some(ICON);
    write(&icon_at, ICON)?;
    write(&entry_at, &entry)?;
    print!("{}", style.line(GREEN, if same { "Unchanged" } else { "Installed" }, "icon and menu entry"));

    let settings = config::path();
    if settings.exists() {
        print!("{}", style.line(GREEN, "Unchanged", &config::short(&settings)));
    } else {
        write(&settings, config::EXAMPLE)?;
        print!("{}", style.line(GREEN, "Created", &format!("{} (nothing bound yet)", config::short(&settings))));
    }

    write(&unit_path(), &UNIT.replace("@EXE@", &exe.to_string_lossy()))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--quiet", "clutch.service"])?;
    systemctl(&["restart", "clutch.service"])?;
    print!("{}", style.line(GREEN, "Started", "clutch.service"));

    // It takes a moment to find the keyboards, or to find it cannot.
    let mut state = Err("not started".to_string());
    for _ in 0..30 {
        sleep(Duration::from_millis(100));
        state = crate::ask("state");
        if state.as_ref().is_ok_and(|state| !state.starts_with("waiting")) {
            break;
        }
    }
    let state = state?;
    match state.trim() {
        waiting if waiting.starts_with("waiting") => print!("{}", style.line(YELLOW, "Waiting", "for keyboard access")),
        state => print!("{}", style.line(GREEN, "Running", state)),
    }
    Ok(())
}

pub fn remove() -> Result<(), String> {
    let style = Style::of_terminal();
    let _ = systemctl(&["disable", "--quiet", "--now", "clutch.service"]);
    let _ = fs::remove_file(unit_path());
    let _ = systemctl(&["daemon-reload"]);
    print!("{}", style.line(GREEN, "Removed", "clutch.service"));
    // Any shortcuts of ours that GNOME was given.
    if env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.contains("GNOME")) {
        gnome::clear();
        print!("{}", style.line(GREEN, "Removed", "GNOME shortcuts"));
        // And GNOME's own, which `clutch fix` took, go back to it.
        match gnome::restore() {
            0 => {}
            given => print!("{}", style.line(GREEN, "Restored", &format!("{given} of GNOME's own shortcuts"))),
        }
        extension::remove();
        print!("{}", style.line(GREEN, "Removed", "GNOME extension"));
    }
    let (icon_at, entry_at) = menu_paths();
    if fs::remove_file(entry_at).is_ok() | fs::remove_file(icon_at).is_ok() {
        print!("{}", style.line(GREEN, "Removed", "icon and menu entry"));
    }
    if Path::new(RULE_PATH).exists() {
        match as_root("", "remove") {
            true => print!("{}", style.line(GREEN, "Removed", "udev rule")),
            false => print!("{}", style.warning("udev rule left in place: sudo refused")),
        }
    }
    print!("{}", style.line(GREEN, "Kept", &config::short(&config::path())));
    Ok(())
}
