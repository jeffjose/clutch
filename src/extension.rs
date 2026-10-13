//! The part of clutch that lives in GNOME's shell, where on Wayland the
//! windows can be seen and moved from: putting it in, and asking it.

use crate::gnome;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const UUID: &str = "clutch@jeffjose.github.io";
const PATH: &str = "/io/github/jeffjose/Clutch";
const NAME: &str = "io.github.jeffjose.Clutch";

/// Which writing of the extension this is. The shell keeps running the
/// one it started with, and says which that was.
const VERSION: u32 = 2;

/// What it answers to.
const INTERFACE: &str = r#"const VERSION = 2;
const INTERFACE = `<node><interface name="io.github.jeffjose.Clutch">
  <method name="State"><arg type="s" direction="out"/></method>
  <method name="Activate"><arg type="u" direction="in"/></method>
  <method name="Close"><arg type="u" direction="in"/></method>
  <method name="Maximize"><arg type="u" direction="in"/></method>
  <method name="Fullscreen"><arg type="u" direction="in"/></method>
  <method name="MoveToMonitor"><arg type="u" direction="in"/><arg type="b" direction="in"/></method>
  <method name="SendTo"><arg type="u" direction="in"/><arg type="u" direction="in"/></method>
  <method name="GoTo"><arg type="u" direction="in"/></method>
</interface></node>`;
"#;

/// The same extension is written two ways: the shell took to modules
/// with version 45, and reads nothing else since.
const MODULES: &str = "import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
";
const BEFORE_MODULES: &str = "const {Gio, Meta, Shell} = imports.gi;
const Main = imports.ui.main;
";
const BODY: &str = include_str!("../assets/extension.js");

fn dir() -> PathBuf {
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    let data = env::var_os("XDG_DATA_HOME").map_or(home.join(".local/share"), PathBuf::from);
    data.join("gnome-shell/extensions").join(UUID)
}

/// The shell's version, its first number: 46.
fn shell_version() -> Option<u32> {
    let output = Command::new("gnome-shell").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace().find_map(|word| word.split('.').next()?.parse().ok())
}

fn source(version: u32) -> String {
    match version >= 45 {
        true => format!("{MODULES}\n{INTERFACE}\nexport default class Clutch extends Extension {{\n{BODY}}}\n"),
        false => format!("{BEFORE_MODULES}\n{INTERFACE}\nclass Clutch {{\n{BODY}}}\n\nfunction init() {{\n    return new Clutch();\n}}\n"),
    }
}

/// Put it in and turn it on. A shell on Wayland only finds a new
/// extension when it starts, which is at the next login. Returns what
/// was done and to what, or `None` where there is no GNOME shell.
pub fn install() -> Option<(&'static str, &'static str)> {
    let version = shell_version()?;
    let dir = dir();
    fs::create_dir_all(&dir).ok()?;
    let metadata = format!(
        "{{\n  \"uuid\": \"{UUID}\",\n  \"name\": \"clutch\",\n  \"description\": \"Lets clutch see and move windows.\",\n  \
         \"url\": \"https://github.com/jeffjose/clutch\",\n  \"shell-version\": [\"{version}\"]\n}}\n"
    );
    fs::write(dir.join("metadata.json"), metadata).ok()?;
    fs::write(dir.join("extension.js"), source(version)).ok()?;
    enable(true);
    // A shell that already knew of it takes a moment to start it.
    let mut running = None;
    for _ in 0..10 {
        running = call("State", &[]);
        if running.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Some(match running {
        Some(state) if state.contains(&format!("\"version\":{VERSION},")) => ("Loaded", "GNOME extension"),
        Some(_) => ("Updated", "GNOME extension (log out and in to load the new one)"),
        None => ("Installed", "GNOME extension (log out and in once to load it)"),
    })
}

pub fn remove() {
    enable(false);
    let _ = fs::remove_dir_all(dir());
}

/// Put it on the shell's list of extensions to run, or take it off. The
/// list is written to directly: `gnome-extensions enable` asks the shell,
/// which has not heard of an extension put in since it started and so
/// leaves it off, to stay off at the next login as well.
fn enable(on: bool) {
    const LIST: [&str; 2] = ["org.gnome.shell", "enabled-extensions"];
    let Ok(printed) = gnome::gsettings(&["get", LIST[0], LIST[1]]) else { return };
    let mut enabled = gnome::strings(&printed);
    if enabled.iter().any(|uuid| uuid == UUID) == on {
        return;
    }
    enabled.retain(|uuid| uuid != UUID);
    if on {
        enabled.push(UUID.to_string());
    }
    let _ = gnome::gsettings(&["set", LIST[0], LIST[1], &gnome::list(&enabled)]);
}

/// Ask it for something. What it says back is as gdbus prints it.
pub fn call(method: &str, arguments: &[String]) -> Option<String> {
    let output = Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.gnome.Shell", "--object-path", PATH, "--method"])
        .arg(format!("{NAME}.{method}"))
        .args(arguments)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The one string gdbus printed, as it was sent: `('…',)` without its
/// wrapping, and its backslashes single again. What the extension sends
/// has nothing else that gdbus would have changed.
pub fn unwrap(printed: &str) -> Option<String> {
    let inner = printed.trim().strip_prefix("('")?.strip_suffix("',)")?;
    Some(inner.replace("\\\\", "\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_both_ways() {
        let new = source(46);
        assert!(new.starts_with("import Gio from 'gi://Gio';"));
        assert!(new.contains("export default class Clutch extends Extension {"));
        assert!(!new.contains("function init()"));
        let old = source(42);
        assert!(old.starts_with("const {Gio, Meta, Shell} = imports.gi;"));
        assert!(old.contains("function init()") && !old.contains("import "));
        for source in [&new, &old] {
            assert!(source.contains("<method name=\"State\">") && source.contains("    State() {"));
        }
        // Written out where asked, for a JavaScript engine to check.
        if let Some(dir) = env::var_os("CLUTCH_EXTENSION_TO") {
            let dir = PathBuf::from(dir);
            fs::write(dir.join("new.mjs"), new).unwrap();
            fs::write(dir.join("old.js"), old).unwrap();
        }
    }

    #[test]
    fn what_gdbus_printed() {
        let printed = "('{\"title\":\"a \\\\u0027b\\\\u0027 \\\\\\\\ c\"}',)\n";
        assert_eq!(unwrap(printed).as_deref(), Some("{\"title\":\"a \\u0027b\\u0027 \\\\ c\"}"));
        assert_eq!(unwrap("()"), None);
    }
}
