//! Putting clutch on a machine, and taking it off again.

use crate::{config, gnome};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

const RULE: &str = include_str!("../assets/70-clutch.rules");
pub const RULE_PATH: &str = "/etc/udev/rules.d/70-clutch.rules";
const UNIT: &str = include_str!("../assets/clutch.service");

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

fn say(what: &str, how: &str) {
    println!("{what:9} {how}");
}

pub fn setup() -> Result<(), String> {
    let rule = if fs::read_to_string(RULE_PATH).ok().as_deref() == Some(RULE) {
        "installed already"
    } else if as_root(RULE, "install") {
        "installed"
    } else {
        "not installed: sudo refused"
    };
    say("rule", rule);

    let settings = config::path();
    let new = !settings.exists();
    if new {
        write(&settings, config::EXAMPLE)?;
    }
    say("settings", &format!("{}{}", config::short(&settings), if new { " (new, nothing bound)" } else { "" }));

    let exe = env::current_exe().map_err(|e| e.to_string())?;
    write(&unit_path(), &UNIT.replace("@EXE@", &exe.to_string_lossy()))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--quiet", "clutch.service"])?;
    systemctl(&["restart", "clutch.service"])?;

    // It takes a moment to find the keyboards, or to find it cannot.
    let mut state = Err("not started".to_string());
    for _ in 0..30 {
        sleep(Duration::from_millis(100));
        state = crate::ask("state");
        if state.as_ref().is_ok_and(|state| !state.starts_with("waiting")) {
            break;
        }
    }
    say("service", state?.trim());
    Ok(())
}

pub fn remove() -> Result<(), String> {
    let _ = systemctl(&["disable", "--quiet", "--now", "clutch.service"]);
    let _ = fs::remove_file(unit_path());
    let _ = systemctl(&["daemon-reload"]);
    // Any shortcuts of ours that GNOME was given.
    if env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.contains("GNOME")) {
        gnome::clear();
        // And GNOME's own, which `clutch fix` took, go back to it.
        gnome::restore();
    }
    say("service", "removed");
    if Path::new(RULE_PATH).exists() {
        say("rule", if as_root("", "remove") { "removed" } else { "left: sudo refused" });
    }
    say("settings", &format!("{} (kept)", config::short(&config::path())));
    Ok(())
}
