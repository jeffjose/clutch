//! The settings file: which keys are taken, and what each runs.

use crate::engine::Bindings;
use crate::keys::parse_sequence;
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const EXAMPLE: &str = include_str!("../assets/config.toml");

pub struct Binding {
    /// The keys as they are written in the file.
    pub keys: String,
    pub command: String,
}

pub struct Config {
    pub bindings: Vec<Binding>,
    pub timeout: Duration,
    /// Words that, typed in the box, open a web address: "gh" and
    /// "https://github.com/{}", where {} is what is typed after the word.
    pub links: Vec<(String, String)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    timeout: Option<u64>,
    #[serde(default)]
    bind: toml::Table,
    #[serde(default)]
    links: toml::Table,
}

impl Config {
    /// The settings and the tree of their keys, or what is wrong with them.
    pub fn parse(text: &str) -> Result<(Config, Bindings), String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut bindings = Vec::new();
        let mut tree = Bindings::default();
        for (keys, command) in file.bind {
            let command = command
                .as_str()
                .ok_or(format!("\"{keys}\": what it runs must be in quotes"))?
                .to_string();
            let sequence = parse_sequence(&keys).map_err(|e| format!("\"{keys}\": {e}"))?;
            tree.add(&sequence, bindings.len()).map_err(|e| format!("\"{keys}\" {e}"))?;
            bindings.push(Binding { keys, command });
        }
        let timeout = Duration::from_millis(file.timeout.unwrap_or(2000));
        let mut links = Vec::new();
        for (word, address) in file.links {
            let address = address.as_str().ok_or(format!("links.{word}: the address must be in quotes"))?;
            if word.contains(char::is_whitespace) {
                return Err(format!("links.\"{word}\": one word, with no spaces"));
            }
            links.push((word.to_lowercase(), address.to_string()));
        }
        Ok((Config { bindings, timeout, links }, tree))
    }
}

fn home() -> PathBuf {
    PathBuf::from(env::var_os("HOME").unwrap_or_default())
}

/// Where what clutch has to remember is kept.
pub fn state_home() -> PathBuf {
    match env::var_os("XDG_STATE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home().join(".local/state"),
    }
}

pub fn config_home() -> PathBuf {
    match env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home().join(".config"),
    }
}

/// A line of shell, with clutch itself where the line can find it. What
/// starts a binding, the desktop or systemd, may not look where clutch
/// was installed, and a binding may well run `clutch palette`.
pub fn with_clutch(line: &str) -> String {
    let dir = env::current_exe().ok().and_then(|exe| Some(exe.parent()?.to_string_lossy().into_owned()));
    match dir {
        Some(dir) => format!("PATH='{}':\"$PATH\"; {line}", dir.replace('\'', "'\\''")),
        None => line.to_string(),
    }
}

/// A path as it is typed, with ~ for home.
pub fn short(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) if env::var_os("HOME").is_some() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

pub fn path() -> PathBuf {
    config_home().join("clutch/config.toml")
}

/// Where the running clutch is spoken to.
pub fn socket() -> PathBuf {
    if let Some(path) = env::var_os("CLUTCH_SOCKET") {
        return PathBuf::from(path);
    }
    match env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("clutch.sock"),
        _ => PathBuf::from(format!("/tmp/clutch-{}.sock", unsafe { libc::getuid() })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_is_a_settings_file() {
        let (config, _) = Config::parse(EXAMPLE).unwrap();
        // It binds the box, and nothing else.
        let bound: Vec<(&str, &str)> = config.bindings.iter().map(|binding| (binding.keys.as_str(), binding.command.as_str())).collect();
        assert_eq!(bound, [("alt+k", "clutch palette")]);
        assert!(config.links.iter().any(|(word, address)| word == "gh" && address.contains("{}")));
    }

    #[test]
    fn bindings_in_the_order_written() {
        let text = "timeout = 500\n[bind]\n\"ctrl+space\" = \"one\"\n\"alt+a b\" = \"two\"\n";
        let (config, _) = Config::parse(text).unwrap();
        assert_eq!(config.timeout, Duration::from_millis(500));
        let keys: Vec<&str> = config.bindings.iter().map(|b| b.keys.as_str()).collect();
        assert_eq!(keys, ["ctrl+space", "alt+a b"]);
        assert_eq!(config.bindings[1].command, "two");
    }

    #[test]
    fn what_is_wrong_is_said() {
        let wrong = |text| Config::parse(text).err().unwrap();
        assert!(wrong("[bind]\n\"ctrl+nokey\" = \"x\"\n").contains("nokey"));
        assert!(wrong("[bind]\n\"ctrl+a\" = 3\n").contains("quotes"));
        assert!(wrong("[bind]\n\"ctrl+a\" = \"x\"\n\"ctrl+a b\" = \"y\"\n").contains("ctrl+a b"));
        assert!(Config::parse("binds = 1\n").is_err());
    }
}
