//! The coding projects that are open: an editor's window is one, and
//! from its name comes the folder, what the project is written in and
//! how its git stands. None of it is set anywhere.

use super::apps;
use super::canvas::Icon;
use super::looks::{self, Look};
use std::cmp::Reverse;
use std::collections::{BTreeSet, VecDeque};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    /// Where it is kept, if that could be worked out.
    pub folder: Option<PathBuf>,
    /// Whether it has a window; one that has none can be started.
    pub open: bool,
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

/// The folders projects are kept in, those of them that there are:
/// ~/scripts first, where a new one is made.
pub fn all_folders_in() -> Vec<PathBuf> {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    ["scripts", "code", "src", "projects", "work", "dev", "repos", "git"].iter().map(|place| home.join(place)).filter(|folder| folder.is_dir()).collect()
}

/// Every project's folder: those in the folders projects are kept in,
/// but not hidden ones.
pub fn all_folders() -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = all_folders_in()
        .iter()
        .flat_map(|place| fs::read_dir(place).into_iter().flatten().flatten())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()) && !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .collect();
    all.sort();
    all
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

/// A number from a name that is the same every day, well stirred.
pub fn number_of(name: &str) -> u32 {
    let mut hash = name.bytes().fold(2166136261u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(16777619));
    hash = (hash ^ hash >> 16).wrapping_mul(0x85EB_CA6B);
    hash = (hash ^ hash >> 13).wrapping_mul(0xC2B2_AE35);
    hash ^ hash >> 16
}

/// The side of a project's picture as it is kept, whatever it was made from.
const SIDE: usize = 128;
/// How long what was found in a folder is taken as still so.
const KEPT_FOR: u64 = 24 * 60 * 60;

/// Where the pictures are kept between one opening of the box and the next.
pub fn kept_in() -> PathBuf {
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    env::var_os("XDG_CACHE_HOME").map_or(home.join(".cache"), PathBuf::from).join("clutch/projects")
}

fn changed_at(path: &Path) -> Option<u64> {
    Some(fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

/// A project's own icon, where its folder has one. Looking through a
/// folder takes a while, so what was found is kept, and looked for again
/// after a day or when the file it came from has changed.
fn found(name: &str, folder: Option<&Path>) -> Option<Icon> {
    let key = folder.map_or(name.to_string(), |folder| folder.to_string_lossy().into_owned());
    let note = kept_in().join(format!("{:08x}", number_of(&key)));
    let kept = note.with_extension("png");
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());

    // The note says when the folder was looked through, and what the
    // picture was made from and when that had last changed.
    let said = fs::read_to_string(&note).unwrap_or_default();
    let mut said = said.lines();
    let mut number = || said.next().and_then(|line| line.parse::<u64>().ok());
    let (looked, stamp) = (number(), number());
    let source = said.next().unwrap_or("");
    let still = looked.is_some_and(|looked| now.saturating_sub(looked) < KEPT_FOR)
        && (source.is_empty() || changed_at(Path::new(source)) == stamp);
    if still && source.is_empty() {
        return None;
    }
    if let Some(icon) = still.then(|| apps::read(&kept)).flatten() {
        return Some(icon);
    }

    let own = folder.into_iter().flat_map(icons_in).find_map(|path| Some((fitted(&load(&path)?), path)));
    let _ = fs::create_dir_all(kept_in());
    let (icon, source) = own.map_or((None, None), |(icon, path)| (Some(icon), Some(path)));
    let stamp = source.as_deref().and_then(changed_at).unwrap_or(0);
    let source = source.map_or(String::new(), |path| path.to_string_lossy().into_owned());
    if icon.as_ref().is_none_or(|icon| save(icon, &kept).is_some()) {
        let _ = fs::write(&note, format!("{now}\n{stamp}\n{source}\n"));
    }
    icon
}

/// Look through the folder again the next time, whatever is kept of it.
pub fn forget(name: &str, folder: Option<&Path>) {
    let key = folder.map_or(name.to_string(), |folder| folder.to_string_lossy().into_owned());
    let _ = fs::remove_file(kept_in().join(format!("{:08x}", number_of(&key))));
}

fn pointed_at(look: &Look) -> Option<&Path> {
    look.icon.as_deref().and_then(|icon| icon.strip_prefix("file:")).map(Path::new)
}

/// The picture that goes on a project's tile, where it is one and not a
/// drawing or letters: the one its look points at, or its folder's own.
pub fn own_of(name: &str, folder: Option<&Path>, look: &Look) -> Option<Icon> {
    match (pointed_at(look), &look.icon) {
        (Some(path), _) => load(path).map(|icon| fitted(&icon)),
        (None, None) => found(name, folder),
        _ => None,
    }
}

/// The picture a project is known by. As it comes, that is the icon its
/// folder has, or a drawing its name picks; a look you gave it says what
/// else: another drawing, letters, a picture, a color, a tile under it.
/// What is drawn is kept, since drawing it takes a while.
pub fn picture(name: &str, folder: Option<&Path>, look: &Look) -> Option<Icon> {
    // Kept under what it was drawn from, so that a change to any of it is
    // a new drawing. A picture you pointed at is told by where it is and
    // when it last changed, and is not read unless it must be.
    let (own, of) = match pointed_at(look) {
        Some(path) => (None, format!("{} {:?}", path.display(), changed_at(path))),
        None => {
            let own = own_of(name, folder, look);
            let of = own.as_ref().map_or(0, |own| own.pixels.iter().fold(2166136261u32, |hash, pixel| (hash ^ pixel).wrapping_mul(16777619)));
            (own, of.to_string())
        }
    };
    if look.is_plain() && own.is_some() {
        return own;
    }
    let key = folder.map_or(name.to_string(), |folder| folder.to_string_lossy().into_owned());
    let kept = kept_in().join(format!("{:08x}-{:08x}.png", number_of(&key), number_of(&format!("{name} {} {of}", look.said()))));
    apps::read(&kept).or_else(|| {
        let own = own.or_else(|| own_of(name, folder, look));
        let icon = looks::render(look, name, own.as_ref(), SIDE);
        let _ = fs::create_dir_all(kept_in());
        let _ = save(&icon, &kept);
        Some(icon)
    })
}

/// Folders that hold what was fetched or built, not what the project is.
const NOT_ITS_OWN: &[&str] = &["node_modules", "target", "dist", "build", "out", "vendor", "venv", "third_party", "__pycache__"];

/// How good a file's name is as the icon of project `name`, the lower the
/// better: a favicon, then one named for the project or plainly an icon.
fn rank_of(stem: &str, name: &str) -> Option<u32> {
    let sized = |word: &str| {
        let rest = stem.strip_prefix(word).and_then(|rest| rest.strip_prefix(['-', '_']));
        rest.is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
    };
    match stem {
        _ if stem == "favicon" || sized("favicon") => Some(0),
        "icon" | "app-icon" | "app_icon" | "appicon" | "apple-touch-icon" => Some(1),
        _ if stem == name || stem.strip_prefix(name).is_some_and(|rest| matches!(rest, "-icon" | "_icon" | "-logo" | "_logo")) => Some(1),
        _ if sized("icon") => Some(2),
        "logo" | "ic_launcher" => Some(3),
        _ => None,
    }
}

/// The files in a project that look to be its icon, the likeliest first.
/// Not far down, and not through more than a few thousand files.
fn icons_in(folder: &Path) -> Vec<PathBuf> {
    let name = folder.file_name().map_or(String::new(), |name| name.to_string_lossy().to_lowercase());
    let mut found = Vec::new();
    let mut folders = VecDeque::from([(folder.to_path_buf(), 0u32)]);
    let mut seen = 0;
    while let Some((folder, depth)) = folders.pop_front().filter(|_| seen < 4000) {
        let Ok(files) = fs::read_dir(&folder) else { continue };
        for file in files.flatten() {
            seen += 1;
            let (path, called) = (file.path(), file.file_name().to_string_lossy().to_lowercase());
            let Ok(kind) = file.file_type() else { continue };
            if kind.is_dir() {
                if depth < 4 && !called.starts_with('.') && !NOT_ITS_OWN.contains(&called.as_str()) {
                    folders.push_back((path, depth + 1));
                }
                continue;
            }
            let Some((stem, ending)) = called.rsplit_once('.') else { continue };
            let Some(kind) = ["png", "svg", "ico", "webp", "jpg"].iter().position(|kind| *kind == ending) else { continue };
            if let Some(rank) = rank_of(stem, &name) {
                // Of several sizes of the same thing, the largest.
                let size = file.metadata().map_or(0, |data| data.len());
                found.push(((rank, depth, kind, Reverse(size)), path));
            }
        }
    }
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

/// A picture file of any kind. A PNG is read here; the others are made
/// into one by ImageMagick, where that is installed.
fn load(path: &Path) -> Option<Icon> {
    let ending = path.extension()?.to_string_lossy().to_lowercase();
    if ending == "png" {
        return apps::read(path);
    }
    let mut from = path.as_os_str().to_owned();
    if ending == "ico" {
        // An .ico holds the icon at several sizes: the largest of them.
        let bytes = fs::read(path).ok()?;
        let count = usize::from(u16::from_le_bytes([*bytes.get(4)?, *bytes.get(5)?]));
        let side = |at: &usize| bytes.get(6 + at * 16).map(|side| (usize::from(*side) + 255) % 256);
        from.push(format!("[{}]", (0..count).max_by_key(side)?));
    }
    let side = format!("{SIDE}x{SIDE}");
    let made = Command::new("convert")
        .args(["-background", "none", "-density", "384"])
        .arg(from)
        .args(["-resize", &side, "png:-"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    apps::decode(std::io::Cursor::new(made.stdout))
}

/// A picture made the size they are kept at: whole, in the middle of the
/// square, each pixel the average of what it covers.
fn fitted(icon: &Icon) -> Icon {
    fitted_to(icon, SIDE)
}

/// The same, in a square of any side.
pub fn fitted_to(icon: &Icon, side: usize) -> Icon {
    let longer = icon.width.max(icon.height).max(1);
    let (width, height) = ((icon.width * side / longer).max(1), (icon.height * side / longer).max(1));
    let (left, top) = ((side - width) / 2, (side - height) / 2);
    let mut pixels = vec![0u32; side * side];
    for y in 0..height {
        let rows = y * icon.height / height..((y + 1) * icon.height).div_ceil(height).min(icon.height);
        for x in 0..width {
            let columns = x * icon.width / width..((x + 1) * icon.width).div_ceil(width).min(icon.width);
            // Colors are weighed by how opaque they are.
            let (mut a, mut r, mut g, mut b, mut count) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for pixel in rows.clone().flat_map(|row| &icon.pixels[row * icon.width..][columns.clone()]) {
                let alpha = u64::from(pixel >> 24);
                a += alpha;
                r += u64::from(pixel >> 16 & 0xFF) * alpha;
                g += u64::from(pixel >> 8 & 0xFF) * alpha;
                b += u64::from(pixel & 0xFF) * alpha;
                count += 1;
            }
            if a > 0 {
                pixels[(top + y) * side + left + x] = ((a / count) << 24 | (r / a) << 16 | (g / a) << 8 | (b / a)) as u32;
            }
        }
    }
    Icon { width: side, height: side, pixels }
}

fn save(icon: &Icon, path: &Path) -> Option<()> {
    let file = std::io::BufWriter::new(fs::File::create(path).ok()?);
    let mut encoder = png::Encoder::new(file, icon.width as u32, icon.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    let bytes: Vec<u8> = icon.pixels.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]).collect();
    encoder.write_header().ok()?.write_image_data(&bytes).ok()
}

/// A letter for each project: the first in its name that none before it
/// has. Taken in the order of the names, so that the same projects have
/// the same letters whichever was used last. None is given a letter in
/// `kept`: those are keys that never reach the box.
pub fn letters(names: &[String], kept: &[char]) -> Vec<char> {
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by_key(|at| names[*at].to_lowercase());
    let mut letters = vec![' '; names.len()];
    let mut taken = kept.to_vec();
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

/// How git stands in a folder: the branch, unless it is the usual one,
/// and how many files have changed.
pub type GitSaid = (PathBuf, (String, usize));

/// How long git is waited for, all the projects together, where the box
/// must have everything before it goes on: listed or pictured rather
/// than shown. On the screen it is not waited for at all.
pub const GIT_WAIT: Duration = Duration::from_millis(120);

/// Ask git how each folder stands, all at once and each on a thread of
/// its own. What each says comes back on the receiver as it does, and
/// `wake` is called each time, so the box is drawn without waiting, and
/// again as they answer: a project that is vast is drawn without what
/// git would have said, until it has. (Asked once the box is on the
/// screen: twenty of them starting at once slow the desktop, and the
/// window, as it comes up.)
pub fn ask_git(folders: &[PathBuf], wake: impl Fn() + Send + Sync + 'static) -> mpsc::Receiver<GitSaid> {
    let (sender, receiver) = mpsc::channel();
    let wake = std::sync::Arc::new(wake);
    for folder in folders.iter().cloned() {
        let (sender, wake) = (sender.clone(), wake.clone());
        thread::spawn(move || {
            let output = Command::new("git")
                .arg("-C")
                .arg(&folder)
                .args(["status", "--porcelain", "-b"])
                .stderr(Stdio::null())
                .output();
            if let Ok(output) = output.as_ref().map(|output| String::from_utf8_lossy(&output.stdout)) {
                if sender.send((folder, git_said(&output))).is_ok() {
                    wake();
                }
            }
        });
    }
    receiver
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
        assert_eq!(letters(&names(&["clutch", "homelab", "jog", "hotshot", "magicfs"]), &[]), ['c', 'h', 'j', 'o', 'm']);
        // In another order they keep their letters.
        assert_eq!(letters(&names(&["hotshot", "jog", "homelab"]), &[]), ['o', 'j', 'h']);
        assert_eq!(letters(&names(&["aa", "a"]), &[]), ['b', 'a']);
        // A key that is bound, Alt+K to the box say, is no project's.
        assert_eq!(letters(&names(&["kite", "jog"]), &['k']), ['i', 'j']);
    }

    #[test]
    fn what_git_said() {
        assert_eq!(git_said("## main...origin/main [ahead 1]\n M a\n?? b\n"), (String::new(), 2));
        assert_eq!(git_said("## record-audio\n"), ("record-audio".to_string(), 0));
        assert_eq!(git_said("## No commits yet on main\n?? a\n"), (String::new(), 1));
    }

    #[test]
    fn a_folder_spelled_with_dashes() {
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
    fn the_likeliest_icon_first() {
        let base = env::temp_dir().join(format!("clutch-icons-{}", std::process::id())).join("jog");
        for file in ["static/favicon.svg", "assets/jog.png", "assets/logo.png", "node_modules/x/favicon.png", "docs/shot.png", "web/icon-512.png"] {
            fs::create_dir_all(base.join(file).parent().unwrap()).unwrap();
            fs::write(base.join(file), "").unwrap();
        }
        let found: Vec<PathBuf> = icons_in(&base).into_iter().map(|path| path.strip_prefix(&base).unwrap().to_path_buf()).collect();
        let named: Vec<&str> = found.iter().filter_map(|path| path.to_str()).collect();
        assert_eq!(named, ["static/favicon.svg", "assets/jog.png", "web/icon-512.png", "assets/logo.png"]);
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }

}
