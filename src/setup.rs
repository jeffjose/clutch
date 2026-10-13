//! Putting clutch on a machine, and taking it off again.

use crate::{config, gnome};
use std::env;
use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

fn may_write(path: &str) -> bool {
    let path = CString::new(path).unwrap();
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

pub fn setup() -> Result<(), String> {
    if fs::read_to_string(RULE_PATH).ok().as_deref() != Some(RULE) {
        println!("Letting you at the keyboards: one udev rule, which needs sudo.");
        if let Err(problem) = ran(Command::new("sudo").args(["sh", "-c", AS_ROOT, RULE_PATH, RULE])) {
            println!(
                "{problem}\n\
                 Without the rule clutch cannot hold the keyboards. On GNOME it gives\n\
                 your bindings to GNOME as shortcuts instead; anywhere else it waits."
            );
        }
    }
    if Path::new(RULE_PATH).exists() && !may_write("/dev/uinput") {
        println!(
            "The rule is in, but the keyboards are not yours yet. They are while you\n\
             are logged in at this machine itself; clutch starts by itself then."
        );
    }

    let settings = config::path();
    if !settings.exists() {
        write(&settings, config::EXAMPLE)?;
        println!("{}: written, with nothing bound yet", settings.display());
    }

    let exe = env::current_exe().map_err(|e| e.to_string())?;
    write(&unit_path(), &UNIT.replace("@EXE@", &exe.to_string_lossy()))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--quiet", "clutch.service"])?;
    systemctl(&["restart", "clutch.service"])?;

    // A moment for it to find the keyboards before it is asked.
    sleep(Duration::from_millis(1500));
    print!("{}", crate::ask("status")?);
    Ok(())
}

pub fn remove() -> Result<(), String> {
    let _ = systemctl(&["disable", "--quiet", "--now", "clutch.service"]);
    let _ = fs::remove_file(unit_path());
    let _ = systemctl(&["daemon-reload"]);
    // Any shortcuts of ours that GNOME was given.
    if env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.contains("GNOME")) {
        gnome::clear();
    }
    if Path::new(RULE_PATH).exists() {
        println!("Taking the udev rule out, which needs sudo.");
        ran(Command::new("sudo").args(["sh", "-c", AS_ROOT, RULE_PATH, ""]))?;
    }
    println!("Removed. {} is left as it is.", config::path().display());
    Ok(())
}
