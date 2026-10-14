//! The applications installed, as their .desktop files describe them,
//! and their icons.

use super::canvas::Icon;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub struct App {
    pub name: String,
    /// What kind of thing it is: "Web Browser".
    pub kind: String,
    /// Other words it should be found by.
    pub keywords: String,
    /// The command, with the places for files and addresses taken out.
    pub exec: String,
    pub terminal: bool,
    pub icon: String,
    /// What its windows are known by, where it says.
    pub class: String,
    /// When its file was last written, in seconds: for a launcher made
    /// each time a folder is opened, when that was.
    pub written: u64,
}

/// Where applications and icons are kept: your own first, then the system's.
fn data_dirs() -> Vec<PathBuf> {
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    let own = env::var_os("XDG_DATA_HOME").map_or(home.join(".local/share"), PathBuf::from);
    let shared = env::var("XDG_DATA_DIRS").unwrap_or_default();
    let shared = if shared.is_empty() { "/usr/local/share:/usr/share".to_string() } else { shared };
    std::iter::once(own).chain(shared.split(':').map(PathBuf::from)).collect()
}

/// One .desktop file, if it is an application that is meant to be shown.
fn parse(text: &str) -> Option<App> {
    let mut app = App { name: String::new(), kind: String::new(), keywords: String::new(), exec: String::new(), terminal: false, icon: String::new(), class: String::new(), written: 0 };
    let mut main = false;
    for line in text.lines() {
        if line.starts_with('[') {
            // The actions further down are entries of their own.
            main = line == "[Desktop Entry]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| main) else { continue };
        let value = value.trim();
        match key.trim() {
            "Name" => app.name = value.into(),
            "GenericName" => app.kind = value.into(),
            "Keywords" => app.keywords = value.replace(';', " "),
            "Exec" => app.exec = command(value),
            "Icon" => app.icon = value.into(),
            "StartupWMClass" => app.class = value.into(),
            "Terminal" => app.terminal = value == "true",
            "Type" if value != "Application" => return None,
            "NoDisplay" | "Hidden" if value == "true" => return None,
            _ => {}
        }
    }
    (!app.name.is_empty() && !app.exec.is_empty()).then_some(app)
}

/// An Exec line without its %f, %U and the like, which stand for files
/// that there are none of here.
fn command(exec: &str) -> String {
    let mut command = String::new();
    let mut letters = exec.chars();
    while let Some(letter) = letters.next() {
        match letter {
            '%' => match letters.next() {
                Some('%') => command.push('%'),
                _ => {}
            },
            _ => command.push(letter),
        }
    }
    command.trim().to_string()
}

/// Every application, by name. One of your own stands in for the
/// system's of the same file name.
pub fn all() -> Vec<App> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in data_dirs() {
        let Ok(files) = fs::read_dir(dir.join("applications")) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().is_none_or(|kind| kind != "desktop") || !seen.insert(file.file_name()) {
                continue;
            }
            let written = file.metadata().ok().and_then(|data| data.modified().ok()).and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok());
            let app = fs::read_to_string(&path).ok().as_deref().and_then(parse);
            apps.extend(app.map(|app| App { written: written.map_or(0, |since| since.as_secs()), ..app }));
        }
    }
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

/// The sizes an icon is looked for in, the nearest above what a row
/// needs first.
const SIZES: &[&str] = &["64x64", "96x96", "128x128", "48x48", "256x256", "32x32", "512x512"];
const THEMES: &[&str] = &["hicolor", "Papirus", "elementary-xfce", "Adwaita", "gnome", "Humanity"];

/// The picture for an icon's name, where it is a PNG that can be found:
/// the name is a file itself, or one in the usual folders.
pub fn icon(name: &str) -> Option<Icon> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        return read(Path::new(name));
    }
    let dirs = data_dirs();
    for dir in &dirs {
        for theme in THEMES {
            for size in SIZES {
                let found = read(&dir.join(format!("icons/{theme}/{size}/apps/{name}.png")));
                if found.is_some() {
                    return found;
                }
            }
        }
    }
    dirs.iter().find_map(|dir| read(&dir.join(format!("pixmaps/{name}.png"))))
}

pub fn read(path: &Path) -> Option<Icon> {
    decode(std::io::BufReader::new(fs::File::open(path).ok()?))
}

/// clutch's own icon, which it carries.
pub fn own() -> Option<Icon> {
    decode(std::io::Cursor::new(include_bytes!("../../assets/clutch.png")))
}

pub fn decode(png: impl std::io::BufRead + std::io::Seek) -> Option<Icon> {
    let mut decoder = png::Decoder::new(png);
    // Every kind of PNG comes out as eight bits each of what it has.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut bytes = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut bytes).ok()?;
    let pixel = |p: &[u8]| -> u32 {
        let (r, g, b, a) = match info.color_type {
            png::ColorType::Rgba => (p[0], p[1], p[2], p[3]),
            png::ColorType::Rgb => (p[0], p[1], p[2], 255),
            png::ColorType::GrayscaleAlpha => (p[0], p[0], p[0], p[1]),
            _ => (p[0], p[0], p[0], 255),
        };
        u32::from(a) << 24 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
    };
    let pixels = bytes[..info.buffer_size()].chunks_exact(info.color_type.samples()).map(pixel).collect();
    Some(Icon { width: info.width as usize, height: info.height as usize, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_file() {
        let app = parse(
            "[Desktop Entry]\nType=Application\nName=Firefox\nGenericName=Web Browser\n\
             Exec=firefox %u\nIcon=firefox\nKeywords=web;internet;\n\n\
             [Desktop Action new-window]\nName=New Window\nExec=firefox --new-window\n",
        )
        .unwrap();
        assert_eq!((app.name.as_str(), app.kind.as_str()), ("Firefox", "Web Browser"));
        assert_eq!(app.exec, "firefox");
        assert_eq!(app.keywords, "web internet ");
        assert!(!app.terminal);
    }

    #[test]
    fn what_is_not_shown() {
        assert!(parse("[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n").is_none());
        assert!(parse("[Desktop Entry]\nType=Link\nName=X\nExec=x\n").is_none());
        assert!(parse("[Desktop Entry]\nType=Application\nName=X\n").is_none());
    }

    #[test]
    fn places_for_files_are_taken_out() {
        assert_eq!(command("code --new-window %F"), "code --new-window");
        assert_eq!(command("sh -c 'echo 100%%' %u"), "sh -c 'echo 100%'");
    }
}
