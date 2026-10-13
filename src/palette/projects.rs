//! The coding projects that are open: an editor's window is one, and
//! from its name comes the folder, what the project is written in and
//! how its git stands. None of it is set anywhere.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct Project {
    pub name: String,
    /// What it is written in: "rust", "python", "svelte". The most telling first.
    pub kinds: Vec<&'static str>,
    /// The branch, unless it is the usual one.
    pub branch: String,
    /// How many files differ from what is committed, where git said in time.
    pub changed: Option<usize>,
    /// The key that goes to it, with Alt.
    pub letter: char,
    pub colour: u32,
}

/// The editors whose windows are projects, by how their titles end.
const EDITORS: &[&str] = &[" - Visual Studio Code", " - Code - OSS", " - VSCodium", " - Cursor", " - Antigravity"];

/// The project a window is of, if it is an editor's: "README.md - jog -
/// Visual Studio Code" is jog.
pub fn name_of(title: &str) -> Option<String> {
    let rest = EDITORS.iter().find_map(|editor| title.strip_suffix(editor))?;
    let name = rest.rsplit(" - ").next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Where a project is kept. On X11 the editor names its window after the
/// folder, "code-home-you-scripts-jog", which leaves only the question of
/// which dashes were slashes; elsewhere the usual places are looked in.
pub fn folder_of(name: &str, class: &str) -> Option<PathBuf> {
    let named = class.strip_prefix("code-").and_then(|path| {
        let parts: Vec<&str> = path.split('-').collect();
        unspell(Path::new("/"), &parts)
    });
    named.or_else(|| {
        let home = PathBuf::from(env::var_os("HOME")?);
        let places = ["scripts", "code", "src", "projects", "work", "dev", "repos", "git", ""];
        places.iter().map(|place| home.join(place).join(name)).find(|folder| folder.is_dir())
    })
}

/// The folder that `parts`, joined by slashes or dashes, spell under `base`.
fn unspell(base: &Path, parts: &[&str]) -> Option<PathBuf> {
    if parts.is_empty() {
        return Some(base.to_path_buf());
    }
    // The longest name first: "git-cal" before "git" with "cal" inside it.
    (1..=parts.len()).rev().find_map(|taken| {
        let folder = base.join(parts[..taken].join("-"));
        folder.is_dir().then(|| unspell(&folder, &parts[taken..])).flatten()
    })
}

/// What a project is written in, from the files at its top.
pub fn kinds_of(folder: &Path) -> Vec<&'static str> {
    let names: BTreeSet<String> = fs::read_dir(folder)
        .map(|files| files.flatten().map(|file| file.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    let has = |name: &str| names.contains(name);
    let ends = |end: &str| names.iter().any(|name| name.ends_with(end));
    let mut kinds = Vec::new();

    if has("Cargo.toml") {
        kinds.push("rust");
    }
    if has("go.mod") {
        kinds.push("go");
    }
    if has("package.json") {
        // The framework says more than "node" does.
        let package = fs::read_to_string(folder.join("package.json")).unwrap_or_default();
        let uses = |library: &str| package.contains(&format!("\"{library}\""));
        kinds.push(if uses("svelte") || uses("@sveltejs/kit") {
            "svelte"
        } else if uses("next") {
            "next"
        } else if uses("astro") {
            "astro"
        } else if uses("vue") {
            "vue"
        } else if uses("react") {
            "react"
        } else if has("tsconfig.json") {
            "ts"
        } else {
            "node"
        });
    }
    if ["pyproject.toml", "requirements.txt", "setup.py", "uv.lock", "Pipfile"].iter().any(|file| has(file)) {
        kinds.push("python");
    }
    if has("Android.bp") || has("AndroidManifest.xml") || has("build.gradle") || has("build.gradle.kts") {
        kinds.push("android");
    }
    if has("CMakeLists.txt") {
        kinds.push("c++");
    }
    if ends(".tf") {
        kinds.push("terraform");
    }
    if has("Dockerfile") || has("docker-compose.yml") || has("compose.yaml") {
        kinds.push("docker");
    }
    // With nothing that says, the files themselves do.
    if kinds.is_empty() {
        for (end, kind) in [(".py", "python"), (".rs", "rust"), (".go", "go"), (".ts", "ts"), (".js", "js"), (".html", "html"), (".sh", "shell")] {
            if ends(end) {
                kinds.push(kind);
            }
        }
    }
    kinds.truncate(3);
    kinds
}

/// The colour a kind is known by.
pub fn kind_colour(kind: &str) -> u32 {
    match kind {
        "rust" => 0xDEA584,
        "go" => 0x00ADD8,
        "python" => 0x5B9BD5,
        "svelte" => 0xFF5A2B,
        "next" | "node" => 0x8CC84B,
        "astro" => 0xFF7E33,
        "vue" => 0x41B883,
        "react" => 0x61DAFB,
        "ts" => 0x4A90E2,
        "js" => 0xF1E05A,
        "android" => 0x3DDC84,
        "c++" => 0xF34B7D,
        "terraform" => 0x9B7BF2,
        "docker" => 0x4AA8F0,
        "html" => 0xE8734A,
        _ => 0x9A9A9A,
    }
}

/// A colour for a project that is the same every day: its name decides
/// where round the wheel, and every one is as bright as the others.
pub fn colour_of(name: &str) -> u32 {
    let hash = name.bytes().fold(2166136261u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(16777619));
    let hue = f64::from(hash % 360);
    let (saturation, lightness) = (0.62, 0.68);
    let chroma = (1.0 - f64::abs(2.0 * lightness - 1.0)) * saturation;
    let second = chroma * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 / 60 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let channel = |part: f64| ((part + lightness - chroma / 2.0) * 255.0).round() as u32;
    channel(r) << 16 | channel(g) << 8 | channel(b)
}

/// A letter for each project: the first in its name that none before it
/// has. Taken in the order of the names, so that the same projects have
/// the same letters whichever was used last.
pub fn letters(names: &[String]) -> Vec<char> {
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by_key(|at| names[*at].to_lowercase());
    let mut letters = vec![' '; names.len()];
    let mut taken = Vec::new();
    for at in order {
        let free = |letter: &char| letter.is_ascii_alphanumeric() && !taken.contains(letter);
        let letter = names[at].to_lowercase().chars().find(free).or_else(|| ('a'..='z').chain('0'..='9').find(free));
        letters[at] = letter.unwrap_or(' ');
        taken.push(letters[at]);
    }
    letters
}

/// The branch, unless it is main or master, and how many files have
/// changed: as `git status --porcelain -b` prints them.
fn git_said(printed: &str) -> (String, usize) {
    let mut lines = printed.lines();
    let head = lines.next().unwrap_or("").trim_start_matches("## ");
    let branch = head.split("...").next().unwrap_or("").split(' ').next().unwrap_or("");
    let branch = if matches!(branch, "main" | "master" | "HEAD" | "No") { "" } else { branch };
    (branch.to_string(), lines.count())
}

/// How long git is waited for, all the projects together. One that is
/// vast is drawn without what git would have said.
const GIT_WAIT: Duration = Duration::from_millis(120);

/// Fill in the folders' side of each project: what it is written in, and,
/// asked of all at once, how its git stands.
pub fn look_into(projects: &mut [Project], folders: &[Option<PathBuf>]) {
    let (sender, receiver) = mpsc::channel();
    for (at, folder) in folders.iter().enumerate() {
        let Some(folder) = folder.clone() else { continue };
        projects[at].kinds = kinds_of(&folder);
        let sender = sender.clone();
        thread::spawn(move || {
            let output = Command::new("git")
                .arg("-C")
                .arg(&folder)
                .args(["status", "--porcelain", "-b"])
                .stderr(Stdio::null())
                .output();
            if let Ok(output) = output.as_ref().map(|output| String::from_utf8_lossy(&output.stdout)) {
                let _ = sender.send((at, git_said(&output)));
            }
        });
    }
    drop(sender);
    let until = Instant::now() + GIT_WAIT;
    while let Ok((at, (branch, changed))) = receiver.recv_timeout(until.saturating_duration_since(Instant::now())) {
        (projects[at].branch, projects[at].changed) = (branch, Some(changed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_in_a_title() {
        assert_eq!(name_of("README.md - jog - Visual Studio Code").as_deref(), Some("jog"));
        assert_eq!(name_of("jog - Visual Studio Code").as_deref(), Some("jog"));
        assert_eq!(name_of("a - b.rs - git-cal - Cursor").as_deref(), Some("git-cal"));
        assert_eq!(name_of("jog - Mozilla Firefox"), None);
    }

    #[test]
    fn a_letter_each_and_always_the_same() {
        let names = |list: &[&str]| -> Vec<String> { list.iter().map(|name| name.to_string()).collect() };
        assert_eq!(letters(&names(&["clutch", "homelab", "jog", "hotshot", "magicfs"])), ['c', 'h', 'j', 'o', 'm']);
        // In another order they keep their letters.
        assert_eq!(letters(&names(&["hotshot", "jog", "homelab"])), ['o', 'j', 'h']);
        assert_eq!(letters(&names(&["aa", "a"])), ['b', 'a']);
    }

    #[test]
    fn what_git_said() {
        assert_eq!(git_said("## main...origin/main [ahead 1]\n M a\n?? b\n"), (String::new(), 2));
        assert_eq!(git_said("## record-audio\n"), ("record-audio".to_string(), 0));
        assert_eq!(git_said("## No commits yet on main\n?? a\n"), (String::new(), 1));
    }

    #[test]
    fn a_folder_spelt_with_dashes() {
        let base = env::temp_dir().join(format!("clutch-unspell-{}", std::process::id()));
        fs::create_dir_all(base.join("scripts/git-cal")).unwrap();
        fs::create_dir_all(base.join("scripts/git")).unwrap();
        assert_eq!(unspell(&base, &["scripts", "git", "cal"]), Some(base.join("scripts/git-cal")));
        assert_eq!(unspell(&base, &["scripts", "git"]), Some(base.join("scripts/git")));
        assert_eq!(unspell(&base, &["scripts", "nothing"]), None);
        fs::write(base.join("scripts/git-cal/Cargo.toml"), "").unwrap();
        fs::write(base.join("scripts/git-cal/package.json"), "{\"devDependencies\":{\"svelte\":\"5\"}}").unwrap();
        assert_eq!(kinds_of(&base.join("scripts/git-cal")), ["rust", "svelte"]);
        fs::write(base.join("scripts/git/run.sh"), "").unwrap();
        assert_eq!(kinds_of(&base.join("scripts/git")), ["shell"]);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_colour_from_the_name() {
        assert_eq!(colour_of("jog"), colour_of("jog"));
        assert_ne!(colour_of("jog"), colour_of("clutch"));
    }
}
