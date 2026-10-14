//! What has been copied: kept by the running clutch as it is copied, so
//! that the box can list it and put any of it back.
//!
//! Each copy is a file in the state folder, named by when it was made: a
//! .txt for text and a .png for a picture. The last `KEPT` are kept. What
//! a password manager copies, which it marks as such, is not.

use crate::config;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use x11rb::connection::Connection;
use x11rb::protocol::xfixes::{self, ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::Event;

const KEPT: usize = 100;
/// Text longer than this is not kept: it is a file's worth, not a copy's.
const LONGEST: usize = 64 * 1024;

pub fn folder() -> PathBuf {
    config::state_home().join("clutch/clipboard")
}

/// A copy, as the box lists them: the newest first.
pub struct Copy {
    pub path: PathBuf,
    /// When, in seconds.
    pub at: u64,
    /// Its text, or for a picture none.
    pub text: Option<String>,
}

pub fn history() -> Vec<Copy> {
    let mut copies: Vec<Copy> = fs::read_dir(folder())
        .map(|files| {
            files
                .flatten()
                .filter_map(|file| {
                    let path = file.path();
                    let at = path.file_stem()?.to_str()?.parse().ok()?;
                    let text = match path.extension()?.to_str()? {
                        "txt" => Some(fs::read_to_string(&path).ok()?),
                        "png" => None,
                        _ => return None,
                    };
                    Some(Copy { path, at, text })
                })
                .collect()
        })
        .unwrap_or_default();
    copies.sort_by_key(|copy| std::cmp::Reverse(copy.at));
    copies
}

/// Put a copy back on the clipboard, as it was.
pub fn restore(copy: &std::path::Path) {
    let picture = copy.extension().is_some_and(|ending| ending == "png");
    let Ok(file) = fs::File::open(copy) else { return };
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let (program, kind): (&str, &[&str]) = match (wayland, picture) {
        (true, true) => ("wl-copy", &["--type", "image/png"]),
        (true, false) => ("wl-copy", &[]),
        (false, true) => ("xclip", &["-selection", "clipboard", "-t", "image/png"]),
        (false, false) => ("xclip", &["-selection", "clipboard"]),
    };
    // It stays behind, to hand the clipboard over when asked, after the box has gone.
    let _ = Command::new(program).args(kind).stdin(file).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Put text on the clipboard.
pub fn copy_text(text: &str) {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let (program, kind): (&str, &[&str]) = if wayland { ("wl-copy", &[]) } else { ("xclip", &["-selection", "clipboard"]) };
    let child = Command::new(program).args(kind).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    if let Ok(mut child) = child {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// Keep what was copied, unless it is what was copied last.
fn keep(text: Option<&str>, picture: Option<&[u8]>) {
    let dir = folder();
    let _ = fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir);
    let last = history().into_iter().next();
    if let (Some(text), Some(Copy { text: Some(was), .. })) = (text, &last) {
        if text == was {
            return;
        }
    }
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis()) as u64 / 1000;
    let (path, bytes) = match (text, picture) {
        (_, Some(picture)) => (dir.join(format!("{at}.png")), picture),
        (Some(text), None) => (dir.join(format!("{at}.txt")), text.as_bytes()),
        _ => return,
    };
    let written = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path);
    if let Ok(mut file) = written {
        let _ = file.write_all(bytes);
    }
    for old in history().into_iter().skip(KEPT) {
        let _ = fs::remove_file(old.path);
    }
}

/// What the clipboard has now, asked of xclip: a picture where it offers
/// one, else its text. Nothing, where a password manager says it is a secret.
fn read_x() -> (Option<String>, Option<Vec<u8>>) {
    let ask = |kind: &str| {
        let output = Command::new("xclip").args(["-selection", "clipboard", "-o", "-t", kind]).stderr(Stdio::null()).output().ok()?;
        output.status.success().then_some(output.stdout)
    };
    let kinds = String::from_utf8(ask("TARGETS").unwrap_or_default()).unwrap_or_default();
    if kinds.lines().any(|kind| kind == "x-kde-passwordManagerHint") {
        return (None, None);
    }
    if kinds.lines().any(|kind| kind == "image/png") {
        return (None, ask("image/png").filter(|picture| !picture.is_empty()));
    }
    let text = ask("UTF8_STRING").or_else(|| ask("STRING")).and_then(|bytes| String::from_utf8(bytes).ok());
    (text.filter(|text| !text.trim().is_empty() && text.len() <= LONGEST), None)
}

/// Keep each copy as it is made, for as long as clutch runs. On X11 the
/// server says when the clipboard changes hands; on Wayland it is asked
/// every second or two, where wl-paste can.
pub fn watch() {
    thread::spawn(|| {
        if watch_x().is_none() && std::env::var_os("WAYLAND_DISPLAY").is_some() {
            let mut last = None;
            loop {
                thread::sleep(Duration::from_millis(1500));
                let output = Command::new("wl-paste").args(["-n"]).stderr(Stdio::null()).output();
                let text = output.ok().filter(|output| output.status.success()).and_then(|output| String::from_utf8(output.stdout).ok());
                if text.is_some() && text != last {
                    keep(text.as_deref().filter(|text| !text.trim().is_empty() && text.len() <= LONGEST), None);
                    last = text;
                }
            }
        }
    });
}

fn watch_x() -> Option<()> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen].root;
    conn.xfixes_query_version(5, 0).ok()?.reply().ok()?;
    let clipboard = conn.intern_atom(false, b"CLIPBOARD").ok()?.reply().ok()?.atom;
    conn.xfixes_select_selection_input(root, clipboard, SelectionEventMask::SET_SELECTION_OWNER).ok()?;
    conn.flush().ok()?;
    loop {
        if let Event::XfixesSelectionNotify(xfixes::SelectionNotifyEvent { .. }) = conn.wait_for_event().ok()? {
            // A moment for the program that copied to be ready to say what.
            thread::sleep(Duration::from_millis(80));
            let (text, picture) = read_x();
            keep(text.as_deref(), picture.as_deref());
        }
    }
}
