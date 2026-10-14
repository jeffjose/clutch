//! The box that everything is found from: type, and the windows,
//! applications and commands that match are listed; Enter goes to the
//! one picked out. What is typed first says what is being looked for.
//!
//! It is a program run for as long as the box is up, and has nothing to
//! do with the clutch that holds the keyboards: any key, bound anywhere,
//! can open it.

mod apps;
pub mod canvas;
mod controls;
mod dress;
mod calc;
mod emoji;
mod timers;
mod field;
mod looks;
mod matching;
mod projects;

use crate::config::{self, Config};
use crate::desk::Desk;
use crate::extension;
use canvas::{Canvas, Icon, Text};
use controls::{Controls, Deed};
use field::TextField;
use looks::Look;
use std::collections::BTreeMap;
use std::sync::mpsc;
use projects::Project;
use softbuffer::{Context, Surface};
use std::collections::HashMap;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{self, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, fs, thread};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::platform::x11::{WindowAttributesExtX11, WindowType};
use winit::window::{CursorIcon, Icon as WindowIcon, Window, WindowId, WindowLevel};

const TEXT_COLOR: u32 = 0xEEEEEE;
const DIM_TEXT_COLOR: u32 = 0x9A9A9A;
const PANEL_COLOR: u32 = 0x1E1E1E;
const EDGE_COLOR: u32 = 0x555555;
const RULE_COLOR: u32 = 0x333333;
const ACCENT: u32 = 0x3E63DD;
const ON_ACCENT: u32 = 0xFFFFFF;
/// The lesser words of the row picked out: gray on the accent is mud.
const DIM_ON_ACCENT: u32 = 0xC9D3F6;
/// A control's tile, and under the pointer anything that can be clicked.
const WELL_COLOR: u32 = 0x2A2A2A;
/// The key drawn on a project's tile, and the count of its changed files.
const KEY_COLOR: u32 = 0x3A3A3A;
const CHANGED_COLOR: u32 = 0xFFB020;
/// What each control is lit in when it is on: Wi-Fi, sound, the
/// microphone, Bluetooth, quiet, the night light.
const CONTROL_COLORS: [u32; controls::COUNT] = [0x2F8BFF, 0x1FBF83, 0xFF6652, 0x8A6CFF, 0xE85D9E, 0xFFA51F, 0x12A594, 0x12A594, 0x12A594];

/// How many matches are in view at once.
const ROWS: usize = 9;
/// The projects are tiles, this many across, and at most this many bands
/// of them in view: more are scrolled to.
const COLUMNS: usize = 4;
const BANDS: usize = 3;
/// How many of the projects that are not open are suggested, as tiles.
const SUGGESTED: usize = 4;

/// The icons, as letters of the icon font.
mod glyph {
    pub const SEARCH: &str = "\u{e154}";
    pub const WINDOW: &str = "\u{e0a2}";
    pub const APP: &str = "\u{e104}";
    pub const RUN: &str = "\u{e073}";
    pub const TERMINAL: &str = "\u{e184}";
    pub const KEYBOARD: &str = "\u{e283}";
    pub const WORKSPACE: &str = "\u{e342}";
    pub const MAXIMIZE: &str = "\u{e115}";
    pub const FULLSCREEN: &str = "\u{e538}";
    pub const CLOSE: &str = "\u{e1b1}";
    pub const RIGHT: &str = "\u{e04d}";
    pub const LEFT: &str = "\u{e04c}";
    pub const PAUSE: &str = "\u{e131}";
    pub const PLAY: &str = "\u{e13f}";
    pub const REFRESH: &str = "\u{e148}";
    pub const PENCIL: &str = "\u{e1f8}";
    pub const CLIPBOARD: &str = "\u{e089}";
    pub const CLOCK: &str = "\u{e08b}";
    pub const STAR: &str = "\u{e179}";
    pub const CHECK: &str = "\u{e070}";
}

/// What is being looked for, which the first letter typed says.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Nothing in front: windows, then applications.
    Find,
    /// `>` things to do: to the window in front, the bindings, clutch itself.
    Run,
    /// `@` windows only.
    Windows,
    /// `#` workspaces.
    Workspaces,
    /// `!` a line of shell.
    Shell,
    /// `"` what has been copied, to copy again.
    Clipboard,
    /// `:` emoji, to copy.
    Emoji,
}

impl Mode {
    /// The mode, and what was typed after the letter that chose it.
    fn of(typed: &str) -> (Mode, &str) {
        let mode = match typed.chars().next() {
            Some('>') => Mode::Run,
            Some('@') => Mode::Windows,
            Some('#') => Mode::Workspaces,
            Some('!') => Mode::Shell,
            Some('"') => Mode::Clipboard,
            Some(':') => Mode::Emoji,
            _ => return (Mode::Find, typed),
        };
        (mode, &typed[1..])
    }

    fn label(self) -> (&'static str, &'static str) {
        match self {
            Mode::Find => (glyph::SEARCH, "Find"),
            Mode::Run => (glyph::RUN, "Do"),
            Mode::Windows => (glyph::WINDOW, "Window"),
            Mode::Workspaces => (glyph::WORKSPACE, "Workspace"),
            Mode::Shell => (glyph::TERMINAL, "Shell"),
            Mode::Clipboard => (glyph::CLIPBOARD, "Clipboard"),
            Mode::Emoji => (glyph::STAR, "Emoji"),
        }
    }
}

/// What the list holds: what was found, or what is behind a control.
#[derive(Clone, Copy, PartialEq, Debug)]
enum View {
    Find,
    Wifi,
    Sound,
    /// A project's look, being chosen.
    Look,
}

/// What the pointer is over.
#[derive(Clone, Copy, PartialEq)]
enum Hit {
    /// One of what was found, a tile or a row, by its place among them.
    Item(usize),
    Control(usize),
    /// The bar of a device's level, and the level at that point along it.
    Level(usize, u32),
    /// A line of the menu, and which color on it where it is the colors.
    Menu(usize, usize),
    /// One of the things a look is chosen from: its part, and its place in that.
    Cell(usize, usize),
    /// The button back, out of whatever has been gone into.
    Back,
    /// The way into a control: its networks, its devices.
    Open(usize),
    /// The Sound tile's buttons, and its bar at a level.
    Mute,
    Nudge(i32),
    Volume(u32),
    /// A widget's tile, by its place among them.
    Widget(usize),
    /// The time this opening took, at the foot: a click copies it.
    Took,
}

/// A measure drawn on a row: how strong a network is, out of 100, or how
/// loud a device, and whether it is muted.
#[derive(Clone, Copy)]
enum Gauge {
    None,
    Bars(u32),
    Level(u32, bool),
    /// What is left of a timer that is done at this time: it counts down as it is shown.
    Clock(u64),
    /// What it is known by, drawn where its picture would be: a copy's place.
    Number(usize),
    /// A sum and its answer: the sum is drawn first.
    Sum,
    /// A project that is not open: the editor's icon, by the word that opens it.
    Editor,
}

/// The picture at the front of a row, where it has one of its own.
#[derive(Clone)]
enum Picture {
    None,
    Window(u32),
    Named(String),
    /// A coding project's own, by its name and where it is kept.
    Project(String, Option<PathBuf>),
    Emoji(String),
}

#[derive(Clone)]
enum Action {
    Activate(u32),
    Launch { exec: String, terminal: bool },
    Shell(String),
    Workspace(u32),
    SendTo { window: u32, workspace: u32 },
    MoveToMonitor { window: u32, back: bool },
    Maximize(u32),
    Fullscreen(u32),
    Close(u32),
    /// Something said to the clutch that holds the keyboards.
    Clutch(&'static str),
    EditSettings,
    /// Throw away the projects' pictures, to be made again from the folders.
    ForgetPictures,
    /// Show what is behind a control. The box stays up.
    Open(View),
    /// Do something to a control. The box stays up.
    Deed(Deed),
    /// Put this text on the clipboard.
    Copy(String),
    /// Put what was copied, kept in this file, back on the clipboard.
    Restore(PathBuf),
    /// Start a timer of so many seconds, for what it says; or stop one.
    Timer(u64, String),
    StopTimer(timers::Timer),
    /// Put a copy back on the clipboard and paste it into the window before.
    Paste(PathBuf),
    /// A line you typed, for your own shell to run.
    Typed(String),
    /// Type this into the box, in place of what is there.
    Type(String),
}

#[derive(Clone)]
struct Item {
    glyph: &'static str,
    picture: Picture,
    title: String,
    /// A lesser word after the title: the program a window belongs to.
    detail: String,
    /// At the right: a binding's keys, or what kind of thing this is.
    hint: String,
    /// What it is found by.
    find: String,
    action: Action,
    /// The coding project this is a window of, if it is an editor's.
    project: Option<Project>,
    gauge: Gauge,
}

impl Item {
    fn new(glyph: &'static str, title: &str, detail: &str, hint: &str, action: Action) -> Item {
        let find = format!("{title} {detail}");
        let (picture, gauge) = (Picture::None, Gauge::None);
        Item { glyph, picture, title: title.into(), detail: detail.into(), hint: hint.into(), find, action, project: None, gauge }
    }
}

/// Everything there is to find, gathered once as the box opens.
#[derive(Default)]
struct Stock {
    windows: Vec<Item>,
    apps: Vec<Item>,
    commands: Vec<Item>,
    workspaces: Vec<Item>,
    /// The controls, which are found by name like anything else.
    controls: Vec<Item>,
    /// The projects that are not open and were opened last, with what
    /// starts each: suggested, as tiles.
    closed: Vec<Item>,
    /// And those opened longer ago, which are found by typing.
    shelved: Vec<Item>,
    /// What has been copied, the newest first.
    copies: Vec<Item>,
    /// Words that open a web address, and the address.
    links: Vec<(String, String)>,
    /// The widgets in the settings.
    widgets: Vec<crate::widgets::Widget>,
    /// The icon of the editor projects open in.
    editor_icon: Option<String>,
    /// The window used before the one in front: where Enter alone goes.
    recent: Option<u32>,
}

impl Stock {
    fn gather(desk: Option<&Desk>) -> Stock {
        let mut stock = Stock::default();
        let mut sends = Vec::new();
        let mut folders = Vec::new();
        let active = desk.and_then(Desk::active);
        let settings = fs::read_to_string(config::path()).unwrap_or_default();
        // (Read once: the widgets are the box's too.)
        let (bindings, links, widgets) = Config::parse(&settings).map_or_else(|_| Default::default(), |(settings, _)| (settings.bindings, settings.links, settings.widgets));
        (stock.links, stock.widgets) = (links, widgets);
        crate::timing::mark("settings");
        // A letter that is bound with Alt or Super, as the key that opens
        // the box may be, is clutch's before it is the box's: no project
        // can be gone to by it.
        let bound = |binding: &config::Binding| -> Option<char> {
            let keys = binding.keys.to_lowercase();
            let mut parts: Vec<&str> = keys.split('+').map(str::trim).collect();
            let last = parts.pop().filter(|last| last.len() == 1 && !keys.contains(' '))?;
            parts.iter().any(|part| matches!(*part, "alt" | "super")).then(|| last.chars().next()).flatten()
        };
        // And the controls' letters are theirs.
        let kept: Vec<char> = bindings.iter().filter_map(bound).chain((0..controls::COUNT).filter_map(controls::key_of)).collect();
        // The windows of projects that are open, by what their windows are called.
        let mut open: Vec<String> = Vec::new();
        let apps = apps::all();
        crate::timing::mark("applications");

        if let Some(desk) = desk {
            let (names, current) = desk.workspaces();
            let mut windows = desk.windows();
            // The window in front goes last: the one to go to is rarely
            // the one already there, and with it out of the way Enter
            // alone goes back to the one before.
            windows.sort_by_key(|window| Some(window.id) == active);
            for window in &windows {
                let place = window.workspace.filter(|at| *at != current).and_then(|at| names.get(at as usize));
                let mut item = Item::new(glyph::WINDOW, &window.title, &window.class, place.map_or("", |name| name), Action::Activate(window.id));
                // Its own icon where it brings one; else its program's, by name.
                item.picture = if window.icon.is_empty() { Picture::Window(window.id) } else { Picture::Named(window.icon.clone()) };
                // An editor's window is a project.
                if let Some(name) = projects::name_of(&window.title) {
                    let folder = projects::folder_of(&name, &window.class);
                    folders.push(folder.clone());
                    open.push(window.class.to_lowercase());
                    item.picture = Picture::Project(name.clone(), folder.clone());
                    item.project = Some(Project { name, folder, open: true, ..Project::default() });
                }
                stock.windows.push(item);
            }
            stock.recent = windows.first().map(|window| window.id);
            crate::timing::mark("windows");
            for (at, name) in names.iter().enumerate() {
                let hint = if at as u32 == current { "here" } else { "" };
                stock.workspaces.push(Item::new(glyph::WORKSPACE, name, "", hint, Action::Workspace(at as u32)));
            }
            // What can be done to the window that was in front.
            if let Some(window) = active.filter(|id| windows.iter().any(|window| window.id == *id)) {
                let to = |glyph, name: &str, action| Item::new(glyph, name, "", "window", action);
                if desk.monitor_count() > 1 {
                    stock.commands.push(to(glyph::RIGHT, "Next monitor", Action::MoveToMonitor { window, back: false }));
                    stock.commands.push(to(glyph::LEFT, "Previous monitor", Action::MoveToMonitor { window, back: true }));
                }
                stock.commands.push(to(glyph::MAXIMIZE, "Maximize", Action::Maximize(window)));
                stock.commands.push(to(glyph::FULLSCREEN, "Fullscreen", Action::Fullscreen(window)));
                stock.commands.push(to(glyph::CLOSE, "Close window", Action::Close(window)));
                // One for each workspace: last, where they are out of the way.
                for (at, name) in names.iter().enumerate().filter(|(at, _)| *at as u32 != current) {
                    sends.push(to(glyph::WORKSPACE, &format!("Send to {name}"), Action::SendTo { window, workspace: at as u32 }));
                }
            }
        }

        // A launcher made for one folder, whose windows are that folder's
        // editor (`code-home-you-scripts-jog`), is a project too: one
        // that is not open, and that it starts. Where the project is
        // open, its window stands for it.
        let (launchers, apps): (Vec<apps::App>, Vec<apps::App>) = apps.into_iter().partition(|app| app.class.to_lowercase().starts_with("code-"));
        // The editor's own icon, for what opens in it.
        stock.editor_icon = apps.iter().find(|app| app.name == "Visual Studio Code" || app.exec.split_whitespace().next().is_some_and(|program| program.ends_with("/code") || program == "code")).map(|app| app.icon.clone());
        let mut launchers: Vec<apps::App> = launchers.into_iter().filter(|app| !open.contains(&app.class.to_lowercase())).collect();
        // The ones opened last first: a few of those are suggested as
        // tiles, after the open ones, and the rest are found by typing.
        launchers.sort_by_key(|app| std::cmp::Reverse(app.written));
        for (at, app) in launchers.into_iter().enumerate() {
            let name = app.name.strip_suffix(" (VS Code)").unwrap_or(&app.name).to_string();
            let folder = projects::folder_of(&name, &app.class);
            let mut item = Item::new(glyph::APP, &name, "", "Open", Action::Launch { exec: app.exec, terminal: app.terminal });
            item.gauge = Gauge::Editor;
            item.picture = Picture::Project(name.clone(), folder.clone());
            item.project = Some(Project { name, folder: folder.clone(), letter: ' ', ..Project::default() });
            if at < SUGGESTED {
                folders.push(folder);
                stock.closed.push(item);
            } else {
                stock.shelved.push(item);
            }
        }
        // And every folder where projects are kept, with no launcher yet:
        // "drift" is `cd ~/scripts/drift && code .`.
        let known: Vec<String> = stock.windows.iter().chain(&stock.closed).chain(&stock.shelved).filter_map(|item| Some(item.project.as_ref()?.name.to_lowercase())).collect();
        for folder in projects::all_folders() {
            let Some(name) = folder.file_name().map(|name| name.to_string_lossy().into_owned()) else { continue };
            if known.contains(&name.to_lowercase()) {
                continue;
            }
            let line = format!("code {}", shell_quoted(&folder.to_string_lossy()));
            let mut item = Item::new(glyph::APP, &name, &config::short(&folder), "Open", Action::Launch { exec: line, terminal: false });
            item.gauge = Gauge::Editor;
            item.picture = Picture::Project(name.clone(), Some(folder.clone()));
            item.project = Some(Project { name, folder: Some(folder), letter: ' ', ..Project::default() });
            stock.shelved.push(item);
        }

        crate::timing::mark("project folders");
        // What the folders say of the projects, and a letter for each.
        let mut found: Vec<&mut Project> = stock.windows.iter_mut().chain(stock.closed.iter_mut()).filter_map(|item| item.project.as_mut()).collect();
        let mut looked: Vec<Project> = found.iter().map(|project| (**project).clone()).collect();
        projects::look_into(&mut looked, &folders);
        // The open ones choose their letters first: a suggestion does not
        // take the letter a project that is open has always had.
        let called = |open: bool| -> Vec<String> { looked.iter().filter(|project| project.open == open).map(|project| project.name.clone()).collect() };
        let open_letters = projects::letters(&called(true), &kept);
        let kept: Vec<char> = kept.iter().copied().chain(open_letters.iter().copied()).collect();
        let (mut open_letters, mut other_letters) = (open_letters.into_iter(), projects::letters(&called(false), &kept).into_iter());
        for (project, looked) in found.iter_mut().zip(looked) {
            let letter = if looked.open { open_letters.next() } else { other_letters.next() };
            **project = Project { letter: letter.unwrap_or(' '), ..looked };
        }
        for item in stock.windows.iter_mut().chain(stock.closed.iter_mut()).chain(stock.shelved.iter_mut()) {
            let Some(project) = &item.project else { continue };
            // Found by its name and what it is written in.
            item.find = format!("{} {} {}", project.name, item.find, project.kinds.join(" "));
            // The project is what it is called; the file that is open
            // in it is the lesser word.
            if project.open {
                let file = item.title.rsplit_once(&format!(" - {}", project.name)).map_or("", |(file, _)| file);
                (item.detail, item.title) = (file.to_string(), project.name.clone());
            }
        }

        for app in apps {
            let action = Action::Launch { exec: app.exec, terminal: app.terminal };
            let mut item = Item::new(glyph::APP, &app.name, &app.kind, "", action);
            item.find = format!("{} {} {}", app.name, app.kind, app.keywords);
            item.picture = Picture::Named(app.icon);
            stock.apps.push(item);
        }

        // The bindings, which can be run from here as well as by their keys.
        for binding in bindings {
            stock.commands.push(Item::new(glyph::KEYBOARD, &binding.command, "", &binding.keys, Action::Shell(binding.command.clone())));
        }

        crate::timing::mark("projects and git");
        // What has been copied: its first line, and how long ago.
        let now = timers::now();
        for (number, copy) in crate::clip::history().into_iter().enumerate() {
            let ago = match now.saturating_sub(copy.at) {
                ..60 => "just now".to_string(),
                ..3600 => format!("{} min ago", (now - copy.at) / 60),
                ..86400 => format!("{} h ago", (now - copy.at) / 3600),
                _ => format!("{} days ago", (now - copy.at) / 86400),
            };
            let mut item = match &copy.text {
                // An emoji copied is said by its name and shown as its picture: the font the box is set in has none.
                // (Only what could be one is looked up: the emoji, and their font, are not read for every copy.)
                Some(text) if text.trim().len() <= 8 && !text.trim().is_ascii() && emoji::all().iter().any(|emoji| emoji.text == text.trim()) => {
                    let name = emoji::all().iter().find(|emoji| emoji.text == text.trim()).map_or(String::new(), |emoji| emoji.name.clone());
                    let mut item = Item::new(glyph::CLIPBOARD, &name, &format!("emoji \u{b7} {ago}"), "", Action::Restore(copy.path.clone()));
                    item.picture = Picture::Emoji(text.trim().to_string());
                    item.find = format!("{name} emoji");
                    item
                }
                Some(text) => {
                    let line = text.trim().lines().next().unwrap_or("").chars().take(200).collect::<String>();
                    let more = if text.trim().lines().count() > 1 { format!("{} lines · {ago}", text.trim().lines().count()) } else { ago };
                    let mut item = Item::new(glyph::CLIPBOARD, &line, &more, "", Action::Restore(copy.path.clone()));
                    item.find = text.clone();
                    item
                }
                None => {
                    // How large, as a PNG's head says.
                    let head = fs::read(&copy.path).ok().filter(|bytes| bytes.len() > 24).map(|bytes| {
                        let at = |from: usize| u32::from_be_bytes([bytes[from], bytes[from + 1], bytes[from + 2], bytes[from + 3]]);
                        (at(16), at(20))
                    });
                    let title = head.map_or("A picture".to_string(), |(wide, tall)| format!("A picture, {wide}\u{d7}{tall}"));
                    let mut item = Item::new(glyph::CLIPBOARD, &title, &ago, "", Action::Restore(copy.path.clone()));
                    item.find = "picture image screenshot".to_string();
                    item
                }
            };
            // Each is known by its place, the newest 1: "3 is the third last copied.
            item.gauge = Gauge::Number(number + 1);
            stock.copies.push(item);
        }
        // Found as the controls are, by name, they type what stands for them.
        let mut find_as = |name: &str, find: &str, typed: &str| {
            let mut item = Item::new(glyph::RUN, name, "", typed, Action::Type(typed.to_string()));
            item.find = format!("{name} {find}");
            stock.controls.push(item);
        };
        find_as("Clipboard", "copied paste history", "\"");
        find_as("Emoji", "smiley", ":");
        let mut clutch = |glyph, name: &str, action| stock.commands.push(Item::new(glyph, name, "", "clutch", action));
        clutch(glyph::PENCIL, "Edit settings", Action::EditSettings);
        clutch(glyph::REFRESH, "Reload settings", Action::Clutch("reload"));
        clutch(glyph::PAUSE, "Pause clutch", Action::Clutch("pause"));
        clutch(glyph::PLAY, "Resume clutch", Action::Clutch("resume"));
        clutch(glyph::REFRESH, "Refresh project icons", Action::ForgetPictures);
        // Other words each control is known by.
        let also = ["wifi network internet", "volume audio speakers output", "microphone mute", "", "do not disturb notifications", "redshift warm", "pause music media", "skip track media", "back track media"];
        for (at, name) in controls::NAMES.iter().enumerate() {
            let action = match at {
                controls::WIFI => Action::Open(View::Wifi),
                controls::SOUND => Action::Open(View::Sound),
                _ => Action::Deed(Deed::Flip(at)),
            };
            let mut item = Item::new(glyph::RUN, name, "", "control", action);
            item.find = format!("{name} {}", also[at]);
            stock.controls.push(item);
        }
        stock.commands.extend(sends);
        crate::timing::mark("clipboard and commands");
        stock
    }

    /// What matches what was typed, the best first.
    /// The timers that are running, each a row that stops it.
    fn timers(&self) -> Vec<Item> {
        timers::running()
            .into_iter()
            .map(|timer| {
                let title = if timer.label.is_empty() { "Timer".to_string() } else { timer.label.clone() };
                let mut item = Item::new(glyph::CLOCK, &title, "", "", Action::StopTimer(timer.clone()));
                item.gauge = Gauge::Clock(timer.end);
                item
            })
            .collect()
    }

    /// The projects that are tiles: those open, by name so that each is
    /// always in the same place; then those suggested, the last opened first.
    fn projects(&self) -> Vec<Item> {
        let mut open: Vec<Item> = self.windows.iter().filter(|item| item.project.is_some()).cloned().collect();
        open.sort_by_key(|item| item.project.as_ref().map(|project| project.name.to_lowercase()));
        open.into_iter().chain(self.closed.iter().cloned()).collect()
    }

    fn matching(&self, typed: &str) -> Vec<Item> {
        let (mode, typed) = Mode::of(typed);
        let pick = |items: &[Item]| -> Vec<Item> {
            let finds = items.iter().map(|item| item.find.as_str());
            matching::ranked(typed, finds).into_iter().map(|at| items[at].clone()).collect()
        };
        let shell = |line: &str| {
            let line = line.trim();
            Item::new(glyph::TERMINAL, line, "", "shell", Action::Typed(line.into()))
        };
        match mode {
            Mode::Windows => pick(&self.windows),
            Mode::Workspaces => pick(&self.workspaces),
            Mode::Run => pick(&self.commands),
            Mode::Shell if typed.trim().is_empty() => Vec::new(),
            Mode::Shell => vec![shell(typed)],
            // "3 is the third: it alone, to be copied with Enter.
            Mode::Clipboard => match typed.trim().parse::<usize>() {
                Ok(number) => self.copies.get(number.wrapping_sub(1)).cloned().into_iter().collect(),
                Err(_) => pick(&self.copies),
            },
            // Of all the emoji, those whose names match: Enter copies one.
            Mode::Emoji => {
                let all = emoji::all();
                let finds: Vec<String> = all.iter().map(|emoji| format!("{} {}", emoji.name, emoji.group)).collect();
                // One called just what was typed comes first: "fire" before "fire engine".
                let mut found = matching::ranked(typed, finds.iter().map(String::as_str));
                found.sort_by_key(|at| all[*at].name != typed.trim().to_lowercase());
                let found = found.into_iter().take(200);
                found
                    .map(|at| {
                        let emoji = &all[at];
                        let mut item = Item::new(glyph::STAR, &emoji.name, &emoji.group, "", Action::Copy(emoji.text.clone()));
                        item.picture = Picture::Emoji(emoji.text.clone());
                        item
                    })
                    .collect()
            }
            // With nothing typed: the windows, to go straight to one. Where
            // they cannot be seen, the applications.
            // The projects come first, by name, so that each is always in
            // the same place; then the other windows, the last used first.
            Mode::Find if typed.trim().is_empty() => match self.windows.is_empty() && self.closed.is_empty() {
                true => self.apps.clone(),
                // Timers that are running come before the windows, counting down.
                false => {
                    let others = self.windows.iter().filter(|item| item.project.is_none()).cloned();
                    self.projects().into_iter().chain(self.timers()).chain(others).collect()
                }
            },
            Mode::Find => {
                // The closest first, a window before an application that
                // matches as well: what is open is more likely wanted than
                // another of it. Scattered letters count only when neither
                // has the letters together.
                const OPEN: i32 = 40;
                let found = |loose: bool| -> Vec<Item> {
                    let of = |items: &[Item], more: i32| -> Vec<(i32, Item)> {
                        let scored = items.iter().filter_map(|item| Some((matching::score(typed, &item.find, loose)? + more, item.clone())));
                        scored.collect()
                    };
                    let mut scored = of(&self.windows, OPEN);
                    scored.extend(of(&self.closed, OPEN / 2));
                    scored.extend(of(&self.shelved, OPEN / 2));
                    scored.extend(of(&self.apps, 0));
                    scored.extend(of(&self.controls, 0));
                    scored.sort_by_key(|(score, _)| -score);
                    scored.into_iter().map(|(_, item)| item).collect()
                };
                let mut found = match found(false) {
                    together if together.is_empty() => found(true),
                    together => together,
                };
                // A web address is opened in the browser that is used, in
                // a new tab of the window it has open: first, where it is
                // plainly one, and otherwise after what else it matches.
                if let Some((address, plainly)) = web_address(typed) {
                    let line = format!("xdg-open {}", shell_quoted(&address));
                    let item = Item::new(glyph::RIGHT, &format!("Open {}", typed.trim()), "", "", Action::Shell(line));
                    found.insert(if plainly { 0 } else { found.len() }, item);
                }
                // What is plainly one of these comes before all of it: a
                // word of the links with what to look for, a sum or a
                // conversion, a timer to start, the timers to stop.
                let mut first = Vec::new();
                // A link is its word, a slash, and what to look for: "gh/clutch".
                let (word, rest) = typed.trim().split_once('/').unwrap_or((typed.trim(), ""));
                if let Some((_, address)) = self.links.iter().find(|(known, _)| *known == word.to_lowercase()) {
                    let rest = rest.trim();
                    if !rest.is_empty() || !address.contains("{}") {
                        let address = address.replace("{}", &url_encoded(rest));
                        let shown = address.trim_start_matches("https://").trim_start_matches("http://").to_string();
                        let line = format!("xdg-open {}", shell_quoted(&address));
                        first.push(Item::new(glyph::RIGHT, &format!("Open {shown}"), "", word, Action::Shell(line)));
                    }
                }
                if let Some(answer) = calc::answer(typed) {
                    let mut item = Item::new(glyph::CHECK, &format!("= {answer}"), typed.trim(), "", Action::Copy(answer));
                    item.gauge = Gauge::Sum;
                    first.push(item);
                }
                if let Some((seconds, label)) = timers::asked(typed) {
                    let title = match label.as_str() {
                        "" => format!("Timer {}", timers::said(seconds)),
                        label => format!("Timer {} \u{b7} {label}", timers::said(seconds)),
                    };
                    first.push(Item::new(glyph::CLOCK, &title, "", "", Action::Timer(seconds, label)));
                }
                if typed.trim().starts_with("timer") {
                    first.extend(self.timers());
                }
                found.splice(0..0, first);
                // Whatever else it is, it can be run.
                if found.is_empty() {
                    // What matches nothing can be looked up on the web, with the `g` link.
                    if let Some((_, address)) = self.links.iter().find(|(word, _)| word == "g") {
                        let line = format!("xdg-open {}", shell_quoted(&address.replace("{}", &url_encoded(typed.trim()))));
                        found.push(Item::new(glyph::SEARCH, &format!("Search {}", typed.trim()), "", "", Action::Shell(line)));
                    }
                    found.push(shell(typed));
                    // A word that matches nothing can be a new project too: made where projects are, and opened.
                    let word = typed.trim();
                    let fits = word.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.') && !word.starts_with('.');
                    if let (true, Some(home)) = (fits, projects::all_folders_in().first()) {
                        let folder = home.join(word);
                        let line = format!("mkdir -p {0} && code {0}", shell_quoted(&folder.to_string_lossy()));
                        found.push(Item::new(glyph::PENCIL, &format!("New project {word}"), &config::short(&folder), "", Action::Shell(line)));
                    }
                }
                found
            }
        }
    }
}

/// Start something and leave it running after the box has gone.
///
/// It is started with your shell's environment, not clutch's: clutch is
/// started by systemd, whose PATH is short, and what is started from the
/// box (an editor, a terminal) would otherwise go without what your
/// shell sets up, mise's tools say.
fn spawn(line: &str) {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(config::with_clutch(line));
    start(command);
}

/// A line you typed, run by your own shell, so that its aliases are there.
fn spawn_typed(line: &str) {
    let mut command = Command::new(login_shell());
    command.arg("-c").arg(line);
    start(command);
}

fn start(mut command: Command) {
    command.envs(login_env().iter().cloned()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    if let Some(home) = env::var_os("HOME") {
        command.current_dir(home);
    }
    let _ = command.spawn();
}

/// The shell you log in with, as the system's list of users has it.
fn login_shell() -> String {
    let entry = unsafe { libc::getpwuid(libc::getuid()) };
    let shell = (!entry.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr((*entry).pw_shell) }.to_string_lossy().into_owned());
    shell.filter(|shell| !shell.is_empty()).unwrap_or_else(|| "/bin/sh".to_string())
}

/// The environment your shell sets up, asked of it once: what it prints
/// for `env`. Nothing, where it does not answer.
fn login_env() -> &'static [(String, String)] {
    static ENV: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    ENV.get_or_init(|| {
        let output = Command::new(login_shell()).args(["-c", "env"]).stdin(Stdio::null()).stderr(Stdio::null()).output();
        let printed = output.ok().filter(|output| output.status.success()).map(|output| String::from_utf8_lossy(&output.stdout).into_owned()).unwrap_or_default();
        printed.lines().filter_map(|line| line.split_once('=')).filter(|(name, _)| !name.is_empty() && !name.contains(' ')).map(|(name, value)| (name.to_string(), value.to_string())).collect()
    })
}

/// What a row is, in a word, for the foot of the box.
fn kind_said(item: &Item) -> &'static str {
    match (&item.action, &item.picture, &item.gauge) {
        (_, Picture::Project(..), _) => "project",
        (Action::Activate(_), ..) => "window",
        (Action::Launch { .. }, ..) => "application",
        (Action::Restore(_), ..) => "copied",
        (Action::Copy(_), Picture::Emoji(_), _) => "emoji",
        (Action::Copy(_), _, Gauge::Sum) => "sum",
        (Action::Timer(..) | Action::StopTimer(_), ..) => "timer",
        (Action::Shell(line), ..) if line.starts_with("xdg-open ") => "link",
        (Action::Shell(_) | Action::Typed(_), ..) => "command",
        (Action::Deed(_) | Action::Open(_), ..) => "control",
        (Action::Workspace(_), ..) => "workspace",
        _ => "action",
    }
}

/// What Enter does to a row, in a few words.
fn main_said(item: &Item) -> &'static str {
    match &item.action {
        Action::Activate(_) => "Go to it",
        Action::Launch { .. } | Action::Open(_) => "Open",
        Action::Shell(line) if line.starts_with("xdg-open ") => "Open",
        Action::Shell(_) | Action::Typed(_) => "Run",
        Action::Copy(_) | Action::Restore(_) => "Copy",
        Action::Timer(..) => "Start",
        Action::StopTimer(_) => "Stop",
        Action::Type(_) => "Show",
        Action::Deed(Deed::Join(_)) => "Join",
        Action::Deed(Deed::Leave(_)) => "Leave",
        Action::Deed(Deed::Use(_)) => "Use",
        Action::Deed(Deed::Tell(..)) => "Play or pause",
        Action::Deed(_) => "Turn on or off",
        Action::Workspace(_) => "Go to it",
        _ => "Do it",
    }
}

/// Text as it goes in a web address, where all but letters, digits and a
/// few marks are written as their bytes.
fn url_encoded(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            // A slash is left as it is: "gh jeffjose/dotfiles" is a path as much as words.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (byte as char).to_string(),
            b' ' => "+".to_string(),
            byte => format!("%{byte:02X}"),
        })
        .collect()
}

/// What was typed as a web address, if it looks like one, and whether it
/// plainly is: it says so, or ends in a name the web is mostly under.
/// "main.rs" looks like one too, but is not plainly one.
fn web_address(typed: &str) -> Option<(String, bool)> {
    let typed = typed.trim();
    if typed.is_empty() || typed.contains(char::is_whitespace) {
        return None;
    }
    if typed.starts_with("http://") || typed.starts_with("https://") {
        return Some((typed.to_string(), true));
    }
    let host = typed.split(['/', '?', '#']).next()?;
    if host.starts_with("localhost:") {
        return Some((format!("http://{typed}"), true));
    }
    let (name, end) = host.rsplit_once('.')?;
    if name.is_empty() || end.len() < 2 || !end.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    const COMMON: &[&str] = &["com", "org", "net", "io", "dev", "app", "ai", "co", "gov", "edu", "me", "tv", "us", "uk", "in", "de", "gg", "xyz", "news"];
    Some((format!("https://{typed}"), COMMON.contains(&end.to_lowercase().as_str())))
}

fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn perform(action: &Action, desk: Option<&Desk>) {
    match (action, desk) {
        (Action::Launch { exec, terminal: false }, _) | (Action::Shell(exec), _) => spawn(exec),
        (Action::Typed(line), _) => spawn_typed(line),
        (Action::Launch { exec, terminal: true }, _) => spawn(&format!("x-terminal-emulator -e sh -c {}", shell_quoted(exec))),
        (Action::EditSettings, _) => spawn(&format!("xdg-open {}", shell_quoted(&config::path().to_string_lossy()))),
        (Action::Clutch(what), _) => drop(crate::ask(what)),
        (Action::ForgetPictures, _) => drop(fs::remove_dir_all(projects::kept_in())),
        (Action::Copy(text), _) => crate::clip::copy_text(text),
        (Action::Restore(copy), _) => crate::clip::restore(copy),
        (Action::Timer(seconds, label), _) => timers::start(*seconds, label),
        (Action::StopTimer(timer), _) => timers::stop(timer),
        // Copied again, then pasted into the window that is in front once the box has gone.
        (Action::Paste(copy), _) => {
            crate::clip::restore(copy);
            spawn("sleep 0.25; xdotool key --clearmodifiers ctrl+v");
        }
        (Action::Activate(window), Some(desk)) => desk.activate(*window),
        (Action::Workspace(workspace), Some(desk)) => desk.go_to_workspace(*workspace),
        (Action::SendTo { window, workspace }, Some(desk)) => desk.send_to_workspace(*window, *workspace),
        (Action::MoveToMonitor { window, back }, Some(desk)) => drop(desk.move_to_monitor(*window, *back)),
        (Action::Maximize(window), Some(desk)) => desk.toggle_maximized(*window),
        (Action::Fullscreen(window), Some(desk)) => desk.toggle_fullscreen(*window),
        (Action::Close(window), Some(desk)) => desk.close(*window),
        // Those the box stays up for are its own to do.
        (Action::Open(_) | Action::Deed(_) | Action::Type(_), _) | (_, None) => {}
    }
}

/// The sizes of the box's parts, all from the size of its letters.
#[derive(Clone, Copy)]
struct Layout {
    px: u32,
    pad: i32,
    line: i32,
    /// How wide the list is, and the controls beside it.
    width: i32,
    side: i32,
    head: i32,
    row: i32,
    foot: i32,
}

impl Layout {
    /// For a monitor this tall, letters as large as jog's on it; but not
    /// much larger than the desktop's own, which are `scale` times plain.
    /// A monitor that is vast and not scaled is one that is sat close to.
    fn for_monitor(height: i32, scale: f64) -> Layout {
        let px = (height / 90).min((16.0 * scale) as i32).clamp(14, 30) as u32;
        let (pad, line) = (px as i32, Text::line_height(px));
        let foot = Text::line_height(px * 5 / 6) + pad;
        Layout { px, pad, line, width: pad * 46, side: pad * 22, head: line + 2 * pad, row: line + pad, foot }
    }

    /// The whole box across: the list, and the controls beside it.
    fn full(&self) -> i32 {
        self.width + self.side
    }

    fn height(&self) -> i32 {
        self.head + ROWS as i32 * self.row + self.pad / 2 + self.foot
    }

    /// How round the box's corners are.
    fn corner(&self) -> i32 {
        self.pad * 2 / 3
    }

    /// The side of a project's picture on its tile.
    fn art(&self) -> i32 {
        self.row - self.pad / 3
    }

    /// How tall a project's tile is: a line, a little taller than a row.
    fn tile(&self) -> i32 {
        self.row + self.pad / 4
    }
}

struct Palette {
    desk: Option<Desk>,
    /// Why there is no desk, where there is none.
    no_desk: String,
    stock: Stock,
    field: TextField,
    found: Vec<Item>,
    selected: usize,
    /// The first match in view.
    first: usize,
    /// How many of the matches, the first ones, are drawn as tiles, and
    /// the first band of those in view.
    tiles: usize,
    band: usize,
    modifiers: ModifiersState,
    layout: Layout,
    /// Where on the screen the box goes, if that can be said.
    place: Option<(i32, i32)>,
    text: Text,
    /// The pictures of the rows that have come into view, by what they are of.
    icons: HashMap<String, Option<Icon>>,
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    focused: bool,
    controls: Controls,
    view: View,
    /// Whether the keys are among the controls' tiles, and which of them.
    beside: bool,
    control: usize,
    /// Where the pointer is in the box, and what it is over there.
    pointer: Option<(i32, i32)>,
    hover: Option<Hit>,
    /// How the projects look, where you have said.
    looks: BTreeMap<String, Look>,
    /// What the right button brought up.
    menu: Option<dress::Menu>,
    /// The project whose look is being chosen.
    editing: Option<dress::Editing>,
    /// Whether another window has been put in front for a moment: the
    /// chooser of a picture. The box does not go because of it.
    away: bool,
    /// What the chooser says, when it does.
    chosen: Option<mpsc::Receiver<Option<String>>>,
    /// Wakes the box from another thread.
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The controls that are tiles, here.
    shown: Vec<usize>,
    /// How long this opening took, from the key to the first frame, and
    /// the line kept of it: shown at the foot, to be copied.
    took: Option<(f64, String)>,
    /// The widgets in the settings, and what each last showed, if anything.
    widgets: Vec<(crate::widgets::Widget, Option<crate::widgets::Tile>)>,
    /// What git said of each project's folder, once asked: its last commit and what has changed.
    git: std::cell::RefCell<HashMap<String, (Option<String>, Vec<String>)>>,
}

impl Palette {
    /// What is behind a control, as rows: those that match what is typed.
    fn behind(&self, view: View, typed: &str) -> Vec<Item> {
        let mut items = Vec::new();
        let deed = |title: &str, hint: &str, deed: Deed| Item::new(glyph::RUN, title, "", hint, Action::Deed(deed));
        if view == View::Wifi {
            let tile = &self.controls.tiles[controls::WIFI];
            for network in &self.controls.networks {
                let detail = match (network.joined, network.saved, network.open) {
                    (true, ..) => format!("connected · {}", network.band),
                    (_, true, _) => "saved".to_string(),
                    (.., true) => "open".to_string(),
                    _ => "locked".to_string(),
                };
                let (hint, action) = match network.joined {
                    true => ("", Deed::Leave(network.name.clone())),
                    false => ("", Deed::Join(network.clone())),
                };
                let mut item = Item::new(glyph::RUN, &network.name, &detail, hint, Action::Deed(action));
                item.gauge = Gauge::Bars(network.signal);
                items.push(item);
            }
            if tile.known && tile.there {
                if tile.lit {
                    items.push(deed("Scan again", "Ctrl+R", Deed::Scan));
                }
                items.push(deed(if tile.lit { "Turn off" } else { "Turn on" }, "", Deed::Flip(controls::WIFI)));
            }
        } else {
            // What is playing, to pause it or let it go on.
            for player in &self.controls.players {
                let title = if player.title.is_empty() { &player.called } else { &player.title };
                let by: Vec<&str> = [player.artist.as_str(), player.called.as_str()].into_iter().filter(|word| !word.is_empty() && word != title).collect();
                let (glyph, hint) = if player.playing { (glyph::PLAY, "playing") } else { (glyph::PAUSE, "paused") };
                items.push(Item::new(glyph, title, &by.join(" · "), hint, Action::Deed(Deed::Tell(player.clone(), "PlayPause"))));
            }
            // What plays before what listens, and of each the one in use first.
            let mut devices: Vec<&controls::Device> = self.controls.devices.iter().collect();
            devices.sort_by_key(|device| (device.input, !device.used));
            for device in devices {
                let detail = match (device.input, device.used) {
                    (_, true) => "in use",
                    (false, false) => "output",
                    (true, false) => "input",
                };
                let hint = if device.muted { "muted".to_string() } else { format!("{}%", device.level) };
                let mut item = Item::new(glyph::RUN, &device.called, detail, &hint, Action::Deed(Deed::Use(device.clone())));
                item.gauge = Gauge::Level(device.level, device.muted);
                items.push(item);
            }
        }
        let finds = items.iter().map(|item| item.find.as_str());
        matching::ranked(typed, finds).into_iter().map(|at| items[at].clone()).collect()
    }

    /// Show what is behind a control, or go back to what was found.
    fn open(&mut self, view: View) {
        (self.view, self.beside, self.field) = (view, false, TextField::new(""));
        self.controls.said.clear();
        if view != View::Look {
            self.editing = None;
        }
        self.typed_changed();
    }

    /// A control's tile was pressed: one with more behind it opens, and
    /// the others flip.
    /// A click on a tile flips it, as does Space; Enter goes into the
    /// two with more behind them, and flips the rest.
    fn press(&mut self, at: usize, enter: bool) {
        let tile = &self.controls.tiles[at];
        if !tile.known || !tile.there {
            return;
        }
        match at {
            controls::WIFI if enter => self.open(View::Wifi),
            // Sound is gone into, by a click too: its levels are what is
            // wanted of it, and muting is Space, Alt+S or its actions.
            controls::SOUND if enter || self.hover == Some(Hit::Control(at)) => self.open(View::Sound),
            _ => self.controls.act(Deed::Flip(at)),
        }
    }

    /// The controls have said something new: the rows that are made of
    /// them are made again, with the same one picked out.
    fn heard(&mut self) {
        if self.view == View::Find {
            return;
        }
        let (picked, at) = (self.found.get(self.selected).map(|item| item.title.clone()), self.selected);
        self.typed_changed();
        let same = self.found.iter().position(|item| Some(&item.title) == picked.as_ref());
        self.select(same.unwrap_or(at));
    }

    /// Where the button back is, while something has been gone into: round
    /// the arrow and the "Find" beside it.
    fn back_area(&mut self) -> Option<(i32, i32, i32, i32)> {
        if self.view == View::Find {
            return None;
        }
        let Layout { px, pad, line, .. } = self.layout;
        let across = self.text.width(glyph::LEFT, px) + pad / 2 + self.text.width(Mode::Find.label().1, px);
        Some((pad * 3 / 2 - pad / 2, pad - pad / 3, across + pad, line + pad * 2 / 3))
    }

    /// Back to what was found, from whatever was gone into.
    fn back(&mut self) {
        self.open(View::Find);
    }

    /// Where a control's tile is: left, top, width, height. Two across
    /// and three down, beside the list.
    /// What is playing has a bar across the foot, wide and low, as a
    /// player has: the button before, the one to play or pause, the next.
    fn control_area(&self, at: usize) -> (i32, i32, i32, i32) {
        let Layout { pad, head, width, side, foot, line, .. } = self.layout;
        let gap = pad / 2;
        let bar = 2 * line + pad;
        let bar_top = self.layout.height() - foot - gap - bar;
        let button = line + pad / 2;
        // The buttons from the right: next, play, before.
        let button_at = |from_right: i32| (width + side - gap - pad / 2 - (from_right + 1) * button - from_right * pad / 4, bar_top + (bar - button) / 2, button, button);
        match at {
            controls::PLAY => return button_at(1),
            controls::NEXT => return button_at(0),
            controls::PREVIOUS => return button_at(2),
            _ => {}
        }
        // The small ones, two across, stand over the player; sound has
        // all there is above them, being the one most used.
        let small: Vec<usize> = self.shown.iter().copied().filter(|shown| *shown != controls::SOUND).collect();
        // As many as three to a row: a laptop's Wi-Fi, the microphone and
        // Bluetooth stand side by side, and leave sound its room. The last
        // row's few share it between them.
        let across = small.len().clamp(1, 3);
        let tile_height = 2 * line + pad * 3 / 2;
        let small_top = bar_top - small.len().div_ceil(across) as i32 * (tile_height + gap);
        // The widgets, each across the column, between sound and those.
        let widgets_top = small_top - self.widgets.len() as i32 * (self.widget_height() + gap);
        if at == controls::SOUND {
            return (width + gap, head + gap, side - 2 * gap, widgets_top - gap - head - gap);
        }
        let Some(place) = small.iter().position(|shown| *shown == at) else { return (0, 0, 0, 0) };
        let in_row = (small.len() - place / across * across).min(across) as i32;
        let tile_width = (side - (in_row + 1) * gap) / in_row;
        let column = (place % across) as i32;
        (width + gap + column * (tile_width + gap), small_top + (place / across) as i32 * (tile_height + gap), tile_width, tile_height)
    }

    /// How tall a widget's tile is: its title and value, a bar, and a line.
    fn widget_height(&self) -> i32 {
        let Layout { px, pad, line, .. } = self.layout;
        pad / 2 + line + pad / 3 + (pad / 3).max(4) + pad / 3 + Text::line_height(px * 5 / 6) + pad / 2
    }

    /// Where a widget's tile is: under Sound, the first highest.
    fn widget_area(&self, at: usize) -> (i32, i32, i32, i32) {
        let (left, top, wide, tall) = self.control_area(controls::SOUND);
        let gap = self.layout.pad / 2;
        (left, top + tall + gap + at as i32 * (self.widget_height() + gap), wide, self.widget_height())
    }

    /// A widget's tile: its title and its value across the top, a bar
    /// with the tick of where it should be, and a line. Tinted amber when
    /// its settings say to watch it, and red when it is wrong.
    fn draw_widget(&mut self, canvas: &mut Canvas, at: usize) {
        let Layout { px, pad, line, .. } = self.layout;
        let small = px * 5 / 6;
        let (left, top, wide, tall) = self.widget_area(at);
        let tile = self.widgets[at].1.clone().unwrap_or_else(|| crate::widgets::Tile { title: self.widgets[at].0.title.clone(), line: "\u{2026}".into(), ..Default::default() });
        let tone = [0x1FBF83, 0xFFB020, 0xFF6652][tile.tone.min(2) as usize];
        let ground = if tile.tone == 0 { WELL_COLOR } else { canvas::mix(WELL_COLOR, tone, 45) };
        let ground = if self.hover == Some(Hit::Widget(at)) { canvas::mix(ground, TEXT_COLOR, 30) } else { ground };
        canvas.round_fill(left, top, wide, tall, pad / 2, ground, 256);
        let inset = pad * 2 / 3;
        let value_wide = self.text.width(&tile.value, px);
        let title = self.text.clipped(&tile.title, wide - 2 * inset - value_wide - pad, px);
        self.text.draw(canvas, left + inset, top + pad / 2, px, TEXT_COLOR, &title);
        self.text.draw(canvas, left + wide - inset - value_wide, top + pad / 2, px, TEXT_COLOR, &tile.value);
        // The bar, and across it the tick of where it should be by now.
        let thick = (pad / 3).max(4);
        let bar_top = top + pad / 2 + line + pad / 3;
        let long = wide - 2 * inset;
        if let Some(full) = tile.bar {
            canvas.round_fill(left + inset, bar_top, long, thick, thick / 2, canvas::mix(ground, 0x000000, 80), 256);
            canvas.round_fill(left + inset, bar_top, (long as f64 * full) as i32, thick, thick / 2, tone, 256);
        }
        if let Some(mark) = tile.mark {
            canvas.fill(left + inset + (long as f64 * mark) as i32 - 1, bar_top - thick / 2 - 1, 2, thick * 2 + 2, TEXT_COLOR);
        }
        let said = self.text.clipped(&tile.line, long, small);
        self.text.draw(canvas, left + inset, bar_top + thick + pad / 3, small, DIM_TEXT_COLOR, &said);
    }

    /// The Sound tile's buttons, along its foot: mute, quieter, and
    /// louder at the far end, with how loud between them.
    fn sound_buttons(&self) -> [(Hit, (i32, i32, i32, i32)); 3] {
        let Layout { pad, line, .. } = self.layout;
        let (left, top, wide, tall) = self.control_area(controls::SOUND);
        let side = line + pad / 3;
        let y = top + tall - pad / 2 - side;
        let inset = pad * 2 / 3 - pad / 6;
        [
            (Hit::Mute, (left + inset, y, side, side)),
            (Hit::Nudge(-5), (left + inset + side + pad / 4, y, side, side)),
            (Hit::Nudge(5), (left + wide - inset - side, y, side, side)),
        ]
    }

    /// The bar of how loud, between the Sound tile's buttons.
    fn sound_bar(&self) -> (i32, i32, i32, i32) {
        let pad = self.layout.pad;
        let [_, (_, (quieter, y, side, _)), (_, (louder, ..))] = self.sound_buttons();
        let from = quieter + side + pad / 2;
        (from, y, louder - pad / 2 - from, side)
    }

    /// Where the way into a control is, for Wi-Fi and Sound: a button at
    /// the foot of the tile on the right. The rest of the tile flips it.
    fn way_in(&self, at: usize) -> Option<(i32, i32, i32, i32)> {
        if at != controls::WIFI {
            return None;
        }
        let Layout { pad, line, .. } = self.layout;
        let (left, top, wide, tall) = self.control_area(at);
        let side = line + pad / 2;
        Some((left + wide - side - pad / 4, top + tall - side - pad / 4, side, side))
    }

    /// The bar the buttons of what is playing are on, all of it.
    fn player_bar(&self) -> (i32, i32, i32, i32) {
        let Layout { pad, width, side, foot, line, .. } = self.layout;
        let (gap, bar) = (pad / 2, 2 * line + pad);
        (width + gap, self.layout.height() - foot - gap - bar, side - 2 * gap, bar)
    }

    /// A copy has been forgotten: out of the list, and the rest numbered again.
    fn forget_copy(&mut self, path: &std::path::Path) {
        self.stock.copies.retain(|item| !matches!(&item.action, Action::Restore(kept) if kept == path));
        for (number, item) in self.stock.copies.iter_mut().enumerate() {
            item.gauge = Gauge::Number(number + 1);
        }
        let at = self.selected;
        self.typed_changed();
        self.select(at);
    }

    /// The volume of what plays, louder or quieter by so many in a hundred.
    fn nudge_volume(&mut self, by: i32) {
        let device = self.controls.devices.iter().find(|device| device.used && !device.input).cloned();
        if let Some(device) = device {
            let to = (device.level as i32 + by).clamp(0, 150) as u32;
            // (The tile shows the level asked for at once: `act` keeps it.)
            self.controls.act(Deed::Level(device, to));
            self.heard();
        }
    }

    /// How long this opening took, as the foot says it: "70 ms", or
    /// "copied" once it has been.
    fn took_said(&self) -> String {
        match &self.took {
            Some((_, line)) if line.is_empty() => "copied".to_string(),
            Some((ms, _)) => format!("{ms:.0} ms"),
            None => String::new(),
        }
    }

    /// Where it is, in the foot's far right corner.
    fn took_area(&mut self) -> Option<(i32, i32, i32, i32)> {
        self.took.as_ref()?;
        let Layout { px, pad, foot, .. } = self.layout;
        let small = px * 5 / 6;
        let said = self.took_said();
        let wide = pad / 3 + self.text.width(&said, small) + pad / 4 + self.text.width(glyph::WINDOW, small) + pad / 3;
        let tall = Text::line_height(small) + pad / 3;
        let full = self.layout.full();
        Some((full - pad / 2 - wide, self.layout.height() - foot + (foot - tall) / 2, wide, tall))
    }

    /// Whether what was copied is being looked through, when the side
    /// beside the list shows the one picked out, or pointed at, whole.
    fn previewing(&self) -> bool {
        let typed = self.field.string();
        if self.view != View::Find || typed.is_empty() {
            return false;
        }
        // Searching, the side shows what is picked out, where there is more to say of it.
        Mode::of(&typed).0 == Mode::Clipboard || self.found.get(self.previewed()).is_some_and(|item| self.card(item).is_some())
    }

    /// The row the preview is of: the one pointed at, or else the one picked out.
    fn previewed(&self) -> usize {
        match self.hover {
            Some(Hit::Item(at)) => at,
            _ => self.selected,
        }
    }

    /// What there is to say of a row beside the list, if there is more
    /// than the row says: something large, a title, and lines of detail.
    fn card(&self, item: &Item) -> Option<(Option<String>, String, Vec<(String, String)>)> {
        let line = |name: &str, said: String| (name.to_string(), said);
        match (&item.action, &item.gauge, &item.picture) {
            (Action::Copy(answer), Gauge::Sum, _) => {
                let mut lines = vec![line("Sum", item.detail.clone())];
                // A whole number, as a programmer may want it too.
                if let Ok(whole) = answer.parse::<i64>() {
                    lines.extend([line("Hex", format!("{whole:#x}")), line("Binary", format!("{whole:#b}")), line("Octal", format!("{whole:#o}"))]);
                }
                Some((Some(answer.clone()), String::new(), lines))
            }
            (Action::Copy(text), _, Picture::Emoji(_)) => {
                let points: Vec<String> = text.chars().map(|c| format!("U+{:04X}", c as u32)).collect();
                Some((None, item.title.clone(), vec![line("Group", item.detail.clone()), line("Code", points.join(" ")), line("Enter", "copies it".to_string())]))
            }
            (Action::StopTimer(timer), Gauge::Clock(end), _) => {
                let left = timers::left(&timers::Timer { end: *end, ..timer.clone() });
                Some((Some(left), item.title.clone(), Vec::new()))
            }
            (Action::Shell(line_of_shell), ..) if line_of_shell.starts_with("xdg-open ") => {
                let address = line_of_shell.trim_start_matches("xdg-open ").trim_matches('\'').to_string();
                Some((None, item.title.clone(), vec![line("Address", address)]))
            }
            (_, _, Picture::Project(name, folder)) => {
                let project = item.project.as_ref()?;
                let mut lines = vec![line("Folder", folder.as_ref().map_or("not found".to_string(), |folder| config::short(folder)))];
                if !project.kinds.is_empty() {
                    lines.push(line("Written in", project.kinds.join(", ")));
                }
                if !project.branch.is_empty() {
                    lines.push(line("Branch", project.branch.clone()));
                }
                let (last, changed) = self.git_of(folder.as_deref());
                if let Some(last) = last {
                    lines.push(line("Last commit", last));
                }
                for file in changed.iter().take(8) {
                    lines.push(line("Changed", file.clone()));
                }
                if changed.len() > 8 {
                    lines.push(line("", format!("and {} more", changed.len() - 8)));
                }
                Some((None, name.clone(), lines))
            }
            (Action::Launch { exec, .. }, ..) => Some((None, item.title.clone(), vec![line("Kind", item.detail.clone()), line("Runs", exec.clone())])),
            _ => None,
        }
    }

    /// The last commit of a project, and the files that differ from it:
    /// asked of git the first time, and kept while the box is up.
    fn git_of(&self, folder: Option<&std::path::Path>) -> (Option<String>, Vec<String>) {
        let Some(folder) = folder else { return (None, Vec::new()) };
        let key = folder.to_string_lossy().into_owned();
        if let Some(known) = self.git.borrow().get(&key) {
            return known.clone();
        }
        let git = |arguments: &[&str]| {
            let output = Command::new("git").arg("-C").arg(folder).args(arguments).stderr(Stdio::null()).output().ok()?;
            output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
        };
        let last = git(&["log", "-1", "--format=%s · %cr"]).map(|said| said.trim().to_string()).filter(|said| !said.is_empty());
        let changed = git(&["status", "--porcelain"]).unwrap_or_default().lines().map(|line| line.get(3..).unwrap_or(line).to_string()).collect();
        self.git.borrow_mut().insert(key, (last.clone(), changed));
        self.git.borrow()[&folder.to_string_lossy().into_owned()].clone()
    }

    /// The copy pointed at, or else the one picked out: all of its text,
    /// as much as fits, or its picture as large as fits.
    fn draw_preview(&mut self, canvas: &mut Canvas) {
        let Layout { px, pad, width, side, head, foot, .. } = self.layout;
        let small = px * 5 / 6;
        let (left, top) = (width + pad, head + pad * 3 / 4);
        let (wide, tall) = (side - 2 * pad, self.layout.height() - foot - top - pad * 3 / 4);
        let at = self.previewed();
        // Anything but a copy is a card: its picture, something large, its title, and lines of what it is.
        if let Some(item) = self.found.get(at).cloned() {
            if let Some((large, title, lines)) = self.card(&item) {
                let mut y = top;
                let art = self.layout.row * 2;
                if let Some(icon) = self.icon(&item, art as usize) {
                    canvas.icon(icon, left, y, art);
                    y += art + pad / 2;
                }
                if let Some(large) = large {
                    let big = px * 2;
                    let large = self.text.clipped(&large, wide, big);
                    self.text.draw(canvas, left, y, big, TEXT_COLOR, &large);
                    y += Text::line_height(big) + pad / 4;
                }
                let title = self.text.clipped(&title, wide, px);
                self.text.draw(canvas, left, y, px, TEXT_COLOR, &title);
                y += self.layout.line + pad / 2;
                let line = Text::line_height(small);
                let names = lines.iter().map(|(name, _)| self.text.width(name, small)).max().unwrap_or(0) + pad;
                for (name, said) in lines {
                    if y + line > top + tall {
                        break;
                    }
                    self.text.draw(canvas, left, y, small, DIM_TEXT_COLOR, &name);
                    let said = self.text.clipped(&said, wide - names, small);
                    self.text.draw(canvas, left + names, y, small, TEXT_COLOR, &said);
                    y += line;
                }
                return;
            }
        }
        let Some(Action::Restore(path)) = self.found.get(at).map(|item| item.action.clone()) else {
            self.text.draw(canvas, left, top, px, DIM_TEXT_COLOR, "Nothing copied yet");
            return;
        };
        // An emoji, large, and its name.
        if let Some(Picture::Emoji(text)) = self.found.get(at).map(|item| item.picture.clone()) {
            let side = (wide.min(tall) / 2).max(1);
            let fetch = || emoji::picture(&text).map(|picture| projects::fitted_to(&picture, side as usize));
            if let Some(picture) = self.icons.entry(format!("emoji {text} {side}")).or_insert_with(fetch) {
                canvas.icon(picture, left, top, side);
            }
            let name = self.found[at].title.clone();
            self.text.draw(canvas, left, top + side + pad / 2, px, TEXT_COLOR, &name);
            let code: Vec<String> = text.chars().map(|c| format!("U+{:04X}", c as u32)).collect();
            self.text.draw(canvas, left, top + side + pad / 2 + self.layout.line, small, DIM_TEXT_COLOR, &code.join(" "));
            return;
        }
        let line = Text::line_height(small);
        let said = match path.extension().is_some_and(|ending| ending == "png") {
            // A picture, as large as fits, kept as it is drawn.
            true => {
                let room = wide.min(tall - line - pad / 2).max(1);
                let key = format!("preview {} {room}", path.display());
                let fetch = || apps::read(&path).map(|picture| projects::fitted_to(&picture, room as usize));
                if let Some(picture) = self.icons.entry(key).or_insert_with(fetch) {
                    canvas.icon(picture, left + (wide - room) / 2, top, room);
                }
                self.found[at].title.trim_start_matches("A picture, ").to_string()
            }
            // Text, each line broken where it no longer fits, until the room runs out.
            false => {
                let text = fs::read_to_string(&path).unwrap_or_default();
                let mut lines = Vec::new();
                for paragraph in text.lines() {
                    let mut now = String::new();
                    for word in paragraph.split_inclusive(' ') {
                        if !now.is_empty() && self.text.width(&format!("{now}{word}"), small) > wide {
                            lines.push(std::mem::take(&mut now));
                        }
                        // A word longer than the line is cut where it must be.
                        for letter in word.chars() {
                            if self.text.width(&format!("{now}{letter}"), small) > wide {
                                lines.push(std::mem::take(&mut now));
                            }
                            now.push(letter);
                        }
                    }
                    lines.push(now);
                }
                let room = ((tall - line - pad / 2) / line).max(1) as usize;
                for (row, shown) in lines.iter().take(room).enumerate() {
                    self.text.draw(canvas, left, top + row as i32 * line, small, TEXT_COLOR, shown.trim_end());
                }
                let more = if lines.len() > room { format!(" · {} more lines", lines.len() - room) } else { String::new() };
                format!("{} characters · {} lines{more}", text.chars().count(), text.lines().count())
            }
        };
        let said = self.text.clipped(&said, wide, small);
        self.text.draw(canvas, left, top + tall - line, small, DIM_TEXT_COLOR, &said);
    }

    /// A key, drawn as a key on a keyboard is: an outline a little deeper
    /// at its foot, and the letter, in no color but what it stands on.
    /// It is the same on everything, so that it reads as a key and not as
    /// a mark of the thing.
    fn key_cap(&mut self, canvas: &mut Canvas, (left, top, side): (i32, i32, i32), ground: u32, letter: char) {
        let small = self.layout.px * 5 / 6;
        let (edge, letter_color) = (canvas::mix(ground, 0xFFFFFF, 70), canvas::mix(ground, 0xFFFFFF, 200));
        canvas.round_fill(left, top, side, side, side / 4, edge, 256);
        canvas.round_fill(left + 1, top + 1, side - 2, side - 3, side / 4 - 1, ground, 256);
        let letter = letter.to_uppercase().to_string();
        let across = self.text.width(&letter, small);
        self.text.draw(canvas, left + (side - across) / 2, top + (side - 1 - Text::line_height(small)) / 2, small, letter_color, &letter);
    }

    /// What is playing, as a bar: its title and who, and the buttons, each
    /// with the letter that presses it with Alt. With nothing playing, it says so.
    fn draw_player(&mut self, canvas: &mut Canvas, lit_up: Option<usize>) {
        let Layout { px, pad, line, .. } = self.layout;
        let small = px * 5 / 6;
        let (left, top, wide, tall) = self.player_bar();
        let player = self.controls.players.first().cloned();
        let playing = player.as_ref().is_some_and(|player| player.playing);
        let color = CONTROL_COLORS[controls::PLAY];
        let ground = if playing { canvas::mix(PANEL_COLOR, color, 160) } else { WELL_COLOR };
        let pointed = matches!(self.hover, Some(Hit::Control(at)) if at >= controls::TILES);
        canvas.round_fill(left, top, wide, tall, pad / 2, if pointed { canvas::mix(ground, TEXT_COLOR, 20) } else { ground }, 256);
        let (main, dim) = if playing { (ON_ACCENT, canvas::mix(color, ON_ACCENT, 170)) } else { (TEXT_COLOR, DIM_TEXT_COLOR) };

        let buttons = [(controls::PREVIOUS, "\u{e162}"), (controls::PLAY, if playing { glyph::PAUSE } else { glyph::PLAY }), (controls::NEXT, "\u{e163}")];
        let words_right = self.control_area(controls::PREVIOUS).0 - pad / 2;
        let inset = pad * 2 / 3;
        let (title, by) = match &player {
            Some(player) => {
                let title = if player.title.is_empty() { player.called.clone() } else { player.title.clone() };
                let by: Vec<&str> = [player.artist.as_str(), player.called.as_str()].into_iter().filter(|word| !word.is_empty() && *word != title).collect();
                (title, by.join(" · "))
            }
            None if self.controls.tiles[controls::PLAY].known => ("Nothing is playing".to_string(), String::new()),
            None => (String::new(), String::new()),
        };
        let title = self.text.clipped(&title, words_right - left - inset, px);
        self.text.draw(canvas, left + inset, top + pad / 2, px, if player.is_some() { main } else { DIM_TEXT_COLOR }, &title);
        let by = self.text.clipped(&by, words_right - left - inset, small);
        self.text.draw(canvas, left + inset, top + pad / 2 + line, small, dim, &by);

        for (at, drawn) in buttons {
            let (x, y, side, _) = self.control_area(at);
            let picked = lit_up == Some(at);
            let pointed = self.hover == Some(Hit::Control(at));
            if picked || pointed {
                canvas.round_fill(x, y, side, side, side / 2, if picked { ACCENT } else { canvas::mix(ground, TEXT_COLOR, 50) }, 256);
            }
            let across = self.text.width(drawn, px);
            let shade = if player.is_some() { main } else { DIM_TEXT_COLOR };
            self.text.draw(canvas, x + (side - across) / 2, y + (side - line) / 2, px, shade, drawn);
            // The letter, small, at the foot of its button.
            let key = controls::KEYS[at];
            if key != ' ' {
                let letter = key.to_uppercase().to_string();
                let tiny = px * 2 / 3;
                let half = self.text.width(&letter, tiny) / 2;
                self.text.draw(canvas, x + side - half - 1, y + side - Text::line_height(tiny) * 2 / 3, tiny, dim, &letter);
            }
        }
    }

    /// Where a device's level is drawn on its row: from, and how long.
    fn level_bar(&mut self) -> (i32, i32) {
        let Layout { px, pad, width, .. } = self.layout;
        let long = pad * 6;
        // The numbers kept in a column, so that the bars line up.
        (width - pad * 3 / 2 - self.text.width("muted", px) - pad * 3 / 4 - long, long)
    }

    /// What is drawn at a point of the box, of the things that can be clicked.
    fn hit(&mut self, x: i32, y: i32) -> Option<Hit> {
        let Layout { pad, head, row, width, .. } = self.layout;
        let within = |(left, top, wide, tall): (i32, i32, i32, i32)| (left..left + wide).contains(&x) && (top..top + tall).contains(&y);
        // With the menu up, nothing else is to be clicked.
        if self.menu.is_some() {
            return self.menu_hit(x, y);
        }
        if self.back_area().is_some_and(within) {
            return Some(Hit::Back);
        }
        if self.took_area().is_some_and(within) {
            return Some(Hit::Took);
        }
        if self.view == View::Look && x < width {
            return self.editor_hit(x, y);
        }
        // Anywhere else on the player's bar is its button to play or pause.
        let on_bar = within(self.player_bar()).then_some(controls::PLAY);
        // A preview in their place has nothing to click.
        if !self.previewing() {
            if let Some(at) = (0..self.widgets.len()).find(|at| within(self.widget_area(*at))) {
                return Some(Hit::Widget(at));
            }
            if let Some((hit, _)) = self.sound_buttons().into_iter().find(|(_, area)| within(*area)) {
                return Some(hit);
            }
            let (from, top, long, tall) = self.sound_bar();
            if (from - pad / 2..from + long + pad / 2).contains(&x) && (top..top + tall).contains(&y) {
                return Some(Hit::Volume(((x - from).clamp(0, long) * 20 / long.max(1) * 5) as u32));
            }
            if let Some(at) = [controls::WIFI, controls::SOUND].into_iter().find(|at| self.way_in(*at).is_some_and(within)) {
                return Some(Hit::Open(at));
            }
            if let Some(at) = (0..controls::COUNT).find(|at| within(self.control_area(*at))).or(on_bar) {
                let tile = &self.controls.tiles[at];
                return (tile.known && tile.there).then_some(Hit::Control(at));
            }
        }
        if x >= width - pad / 2 || x < pad / 2 || y < head + pad / 4 {
            return None;
        }
        // The projects' tiles, then the rows under them: as they are drawn.
        let (bands, rows) = self.room();
        let (tile_width, tile_height) = ((width - pad) / COLUMNS as i32, self.layout.tile());
        let (across, down) = ((x - pad / 2) / tile_width, (y - head - pad / 4) / tile_height);
        if (down as usize) < bands {
            let at = (self.band + down as usize) * COLUMNS + across as usize;
            return ((across as usize) < COLUMNS && at < self.tiles).then_some(Hit::Item(at));
        }
        let under_tiles = if bands > 0 { bands as i32 * tile_height + pad / 2 } else { 0 };
        let down = y - head - pad / 4 - under_tiles;
        let at = self.first.max(self.tiles) + (down / row) as usize;
        if down < 0 || (down / row) as usize >= rows || at >= self.found.len() {
            return None;
        }
        // On a device's bar, a click is a level: a little past its end is all of it.
        let (from, long) = self.level_bar();
        match self.found[at].gauge {
            Gauge::Level(..) if (from - pad / 2..from + long + pad / 2).contains(&x) => {
                Some(Hit::Level(at, ((x - from).clamp(0, long) * 20 / long * 5) as u32))
            }
            _ => Some(Hit::Item(at)),
        }
    }

    /// The pointer moved, or what is under it may have.
    fn pointed(&mut self) {
        self.hover = self.pointer.and_then(|(x, y)| self.hit(x, y));
        if let Some(window) = &self.window {
            window.set_cursor(if self.hover.is_some() { CursorIcon::Pointer } else { CursorIcon::Default });
            window.request_redraw();
        }
    }

    /// A click on what the pointer is over.
    fn click(&mut self, event_loop: &ActiveEventLoop) {
        // Away from the menu, it goes and nothing else happens.
        if self.menu.is_some() {
            match self.hover {
                Some(Hit::Menu(line, swatch)) => self.menu_do(line, swatch, event_loop),
                _ => self.menu = None,
            }
            return self.pointed();
        }
        match self.hover {
            Some(Hit::Back) => self.back(),
            // The whole of what was kept of this opening, to be pasted somewhere.
            Some(Hit::Took) => {
                if let Some((ms, line)) = self.took.take() {
                    crate::clip::copy_text(&line);
                    self.took = Some((ms, String::new()));
                }
            }
            Some(Hit::Mute) => self.controls.act(Deed::Flip(controls::SOUND)),
            // A widget runs what its settings say a click on it should, and the box goes.
            Some(Hit::Widget(at)) => {
                let open = self.widgets[at].0.open.clone();
                if !open.is_empty() {
                    if let Some(window) = &self.window {
                        window.set_visible(false);
                    }
                    spawn(&open);
                    return event_loop.exit();
                }
            }
            Some(Hit::Nudge(by)) => self.nudge_volume(by),
            Some(Hit::Volume(level)) => {
                let device = self.controls.devices.iter().find(|device| device.used && !device.input).cloned();
                if let Some(device) = device {
                    self.nudge_volume(level as i32 - device.level as i32);
                }
            }
            Some(Hit::Open(at)) => {
                self.control = at;
                self.open(if at == controls::WIFI { View::Wifi } else { View::Sound });
            }
            Some(Hit::Cell(part, at)) => self.choose(part, at),
            Some(Hit::Menu(..)) => {}
            Some(Hit::Item(at)) => {
                self.beside = false;
                self.select(at);
                self.go(at, event_loop);
            }
            Some(Hit::Control(at)) => {
                self.control = at;
                self.press(at, false);
            }
            Some(Hit::Level(at, level)) => {
                self.beside = false;
                self.select(at);
                if let Some(device) = self.device() {
                    self.controls.act(Deed::Level(device, level));
                    self.heard();
                }
            }
            None => {}
        }
        self.pointed();
    }

    /// The device picked out, in the list of them.
    fn device(&self) -> Option<controls::Device> {
        match self.found.get(self.selected).map(|item| &item.action) {
            Some(Action::Deed(Deed::Use(device))) if self.view == View::Sound => Some(device.clone()),
            _ => None,
        }
    }

    /// The player picked out, likewise.
    fn player(&self) -> Option<controls::Player> {
        match self.found.get(self.selected).map(|item| &item.action) {
            Some(Action::Deed(Deed::Tell(player, _))) if self.view == View::Sound => Some(player.clone()),
            _ => None,
        }
    }

    fn typed_changed(&mut self) {
        let typed = self.field.string();
        self.found = match self.view {
            View::Find => self.stock.matching(&typed),
            // A look being chosen has no rows: what is typed finds drawings.
            View::Look => {
                self.editing_typed();
                Vec::new()
            }
            view => self.behind(view, &typed),
        };
        (self.selected, self.first, self.band, self.tiles) = (0, 0, 0, 0);
        // With nothing typed the projects are tiles, and what is picked
        // out is the window used before this one, wherever it is.
        if typed.is_empty() && self.view == View::Find {
            self.tiles = self.found.iter().take_while(|item| item.project.is_some()).count();
            let recent = |item: &Item| matches!(item.action, Action::Activate(id) if Some(id) == self.stock.recent);
            self.select(self.found.iter().position(recent).unwrap_or(0));
        }
    }

    /// How many bands of tiles are in view, and how many rows fit under them.
    fn room(&self) -> (usize, usize) {
        let bands = self.tiles.div_ceil(COLUMNS).min(BANDS);
        let Layout { row, pad, .. } = self.layout;
        let taken = if bands > 0 { bands as i32 * self.layout.tile() + pad / 2 } else { 0 };
        (bands, ((ROWS as i32 * row - taken) / row).max(1) as usize)
    }

    fn select(&mut self, to: usize) {
        self.selected = to.min(self.found.len().saturating_sub(1));
        let (bands, rows) = self.room();
        if self.selected < self.tiles {
            // Its band of tiles into view.
            let band = self.selected / COLUMNS;
            self.band = self.band.clamp((band + 1).saturating_sub(bands), band);
        } else {
            self.first = self.first.max(self.tiles).clamp((self.selected + 1).saturating_sub(rows), self.selected);
        }
    }

    fn step(&mut self, by: isize) {
        let count = self.found.len() as isize;
        if count > 0 {
            self.select((self.selected as isize + by).rem_euclid(count) as usize);
        }
    }

    /// Up or down by one: among the tiles that is a band, and from their
    /// last band down is the first row under them.
    fn step_down(&mut self, down: bool) {
        let (at, tiles, count) = (self.selected, self.tiles, self.found.len());
        let to = match (at < tiles, down) {
            (true, true) if at + COLUMNS < tiles => at + COLUMNS,
            // Onto the last band where it is short, or off the tiles.
            (true, true) if at / COLUMNS < (tiles - 1) / COLUMNS => tiles - 1,
            (true, true) if tiles < count => tiles,
            (true, true) => at,
            (true, false) => at.checked_sub(COLUMNS).unwrap_or(at),
            (false, false) if at == tiles && tiles > 0 => tiles - 1,
            (false, false) => at.saturating_sub(1).max(tiles),
            (false, true) => (at + 1).min(count.saturating_sub(1)),
        };
        self.select(to);
    }

    /// Go to a project, by its place among all of them, and be done.
    fn go_to_project(&mut self, at: usize, event_loop: &ActiveEventLoop) {
        if let Some(item) = self.stock.projects().get(at) {
            if let Some(window) = &self.window {
                window.set_visible(false);
            }
            perform(&item.action, self.desk.as_ref());
        }
        event_loop.exit();
    }

    /// Go to what is picked out, or to `item`, and be done.
    fn go(&mut self, at: usize, event_loop: &ActiveEventLoop) {
        // A control is flipped, or looked into, with the box still up.
        match self.found.get(at).map(|item| item.action.clone()) {
            Some(Action::Open(view)) => return self.open(view),
            Some(Action::Deed(deed)) => return self.controls.act(deed),
            // What it stands for is typed: the clipboard's history, the emoji.
            Some(Action::Type(typed)) => {
                self.field = TextField::new(&typed);
                return self.typed_changed();
            }
            _ => {}
        }
        if let Some(item) = self.found.get(at) {
            // Out of the way first, so that the window gone to is the
            // last thing to be given the keyboard.
            if let Some(window) = &self.window {
                window.set_visible(false);
            }
            perform(&item.action, self.desk.as_ref());
        }
        event_loop.exit();
    }

    /// How a project looks, by its name and where it is kept.
    fn project_look(&self, name: &str, folder: &Option<PathBuf>) -> Look {
        self.look_of(&dress::Target { name: name.to_string(), folder: folder.clone() })
    }

    /// The picture for a row, fetched the first time the row is drawn.
    fn icon(&mut self, item: &Item, size: usize) -> Option<&Icon> {
        let look = match &item.picture {
            Picture::Project(name, folder) => self.project_look(name, folder),
            _ => Look::default(),
        };
        let key = match &item.picture {
            Picture::None => return None,
            Picture::Window(id) => format!("window {id}"),
            Picture::Named(name) => name.clone(),
            Picture::Project(name, folder) => format!("project {name} {folder:?} {}", look.said()),
            Picture::Emoji(text) => format!("emoji {text} {size}"),
        };
        let desk = self.desk.as_ref();
        let fetch = || match &item.picture {
            // A window that brings no icon may still be of a program
            // that has one.
            Picture::Window(id) => desk.and_then(|desk| desk.icon(*id, size)).or_else(|| apps::icon(&item.detail.to_lowercase())),
            Picture::Named(name) => apps::icon(name),
            Picture::Project(name, folder) => projects::picture(name, folder.as_deref(), &look),
            // A copied picture, made small enough to sit in a row.
            Picture::Emoji(text) => emoji::picture(text).map(|picture| projects::fitted_to(&picture, size.max(1))),
            Picture::None => None,
        };
        self.icons.entry(key).or_insert_with(fetch).as_ref()
    }

    fn draw(&mut self, pixels: &mut [u32], width: u32, height: u32) {
        let mut canvas = Canvas::new(pixels, width, height);
        let Layout { px, pad, line, head, row, .. } = self.layout;
        // The list is `width` across, and the box with the controls `full`.
        let (full, height, width) = (canvas.width, canvas.height, self.layout.width);
        // A panel with rounded corners and a thin edge, on nothing.
        let corner = self.layout.corner();
        canvas.fill(0, 0, full, height, 0);
        canvas.round_fill(0, 0, full, height, corner, EDGE_COLOR, 256);
        canvas.round_fill(1, 1, full - 2, height - 2, corner - 1, PANEL_COLOR, 256);

        // What is being looked for, and what has been typed.
        let typed = self.field.string();
        let (mode, _) = match self.view {
            View::Find => Mode::of(&typed),
            _ => (Mode::Find, ""),
        };
        let (icon, label) = mode.label();
        let (left, top) = (pad * 3 / 2, pad);
        // Gone into something, the way back is a button where the search
        // was: an arrow and "Find", which is where it goes back to.
        let (icon, color) = match self.back_area() {
            Some((x, y, wide, tall)) => {
                let pointed = self.hover == Some(Hit::Back);
                canvas.round_fill(x, y, wide, tall, tall / 2, if pointed { KEY_COLOR } else { WELL_COLOR }, 256);
                (glyph::LEFT, TEXT_COLOR)
            }
            None => (icon, DIM_TEXT_COLOR),
        };
        self.text.draw(&mut canvas, left, top, px, color, icon);
        let label_left = left + self.text.width(icon, px) + pad / 2;
        self.text.draw(&mut canvas, label_left, top, px, color, label);
        let mut typed_left = label_left + self.text.width(label, px) + pad;
        // And the control that has been gone into.
        if self.view != View::Find {
            let within = match (self.view, &self.editing) {
                (View::Wifi, _) => controls::NAMES[controls::WIFI].to_string(),
                (View::Look, Some(editing)) => editing.target.name.clone(),
                _ => controls::NAMES[controls::SOUND].to_string(),
            };
            let within = format!("{within} \u{203a}");
            self.text.draw(&mut canvas, typed_left, top, px, TEXT_COLOR, &within);
            typed_left += self.text.width(&within, px) + pad * 2 / 3;
        }
        if let Some((from, to)) = self.field.selection() {
            let mut upto = |at: usize| self.text.width(&self.field.text[..at].iter().collect::<String>(), px);
            let (from, to) = (upto(from), upto(to));
            canvas.fill(typed_left + from, top, to - from, line, ACCENT);
        }
        self.text.draw(&mut canvas, typed_left, top, px, TEXT_COLOR, &typed);
        let before_cursor: String = self.field.text[..self.field.cursor].iter().collect();
        canvas.fill(typed_left + self.text.width(&before_cursor, px), top, (pad / 10).max(2), line, TEXT_COLOR);
        // How something being done to a control is going, if anything is.
        let note = match (self.controls.said.as_str(), self.found.len()) {
            ("", 0) => String::new(),
            ("", count) => format!("{} of {count}", self.selected + 1),
            (said, _) => self.text.clipped(said, full / 2, px),
        };
        let note_left = full - left - self.text.width(&note, px);
        self.text.draw(&mut canvas, note_left, top, px, DIM_TEXT_COLOR, &note);
        canvas.fill(1, head - 1, full - 2, 1, RULE_COLOR);

        // The projects, a tile each: its picture, its name, how its git
        // stands where that is worth saying, and its key.
        let (bands, rows) = self.room();
        let (tile_width, tile_height) = ((width - pad) / COLUMNS as i32, self.layout.tile());
        let small = px * 5 / 6;
        for at in (self.band * COLUMNS..self.tiles).take(bands * COLUMNS) {
            let Some(project) = self.found[at].project.clone() else { continue };
            let (column, band) = ((at % COLUMNS) as i32, (at / COLUMNS - self.band) as i32);
            let (tile_left, tile_top) = (pad / 2 + column * tile_width, head + pad / 4 + band * tile_height);
            let picked = at == self.selected && !self.beside;
            if self.hover == Some(Hit::Item(at)) && !picked {
                canvas.round_fill(tile_left, tile_top, tile_width, tile_height, pad / 2, WELL_COLOR, 256);
            }
            if picked {
                canvas.round_fill(tile_left, tile_top, tile_width, tile_height, pad / 2, ACCENT, 256);
            }
            let (main, dim) = if picked { (ON_ACCENT, DIM_ON_ACCENT) } else { (TEXT_COLOR, DIM_TEXT_COLOR) };
            // Its picture first, as a row has one.
            let item = self.found[at].clone();
            let art = self.layout.art();
            if let Some(icon) = self.icon(&item, art as usize) {
                canvas.icon(icon, tile_left + pad / 2, tile_top + (tile_height - art) / 2, art);
            }
            let text_left = tile_left + pad / 2 + art + pad * 2 / 3;
            // At the end, the key that goes to it, drawn as a key.
            let cap = line;
            let (cap_left, cap_top) = (tile_left + tile_width - pad / 2 - cap, tile_top + (tile_height - cap) / 2);
            let pointed = self.hover == Some(Hit::Item(at));
            let ground = if picked { ACCENT } else if pointed { WELL_COLOR } else { PANEL_COLOR };
            self.key_cap(&mut canvas, (cap_left, cap_top, cap), ground, project.letter);
            // Before it, what is not as it should be: a branch that is
            // not the usual one, and how many files have changed.
            let changed = project.changed.filter(|changed| *changed > 0).map_or(String::new(), |changed| changed.to_string());
            let mut right = cap_left - pad / 2;
            for (words, color) in [(changed.as_str(), CHANGED_COLOR), (project.branch.as_str(), dim)] {
                let across = self.text.width(words, small);
                // Only where it leaves the name room.
                if !words.is_empty() && right - across > text_left + 4 * pad {
                    right -= across;
                    self.text.draw(&mut canvas, right, tile_top + (tile_height - Text::line_height(small)) / 2, small, if picked { DIM_ON_ACCENT } else { color }, words);
                    right -= pad / 2;
                }
            }
            // One that is not open is said more quietly: Enter starts it.
            let name = self.text.clipped(&project.name, right - text_left, px);
            self.text.draw(&mut canvas, text_left, tile_top + (tile_height - line) / 2, px, if project.open { main } else { dim }, &name);
        }
        let under_tiles = if bands > 0 { bands as i32 * tile_height + pad / 2 } else { 0 };
        if bands > 0 && self.found.len() > self.tiles {
            canvas.fill(1, head + pad / 4 + under_tiles - pad / 4, width - 1, 1, RULE_COLOR);
        }
        // While the keys are among the controls, what is picked out in
        // the list is only where they will come back to.
        let pick = if self.beside { WELL_COLOR } else { ACCENT };

        // The matches.
        let picture = row - pad / 2;
        let first = self.first.max(self.tiles);
        for at in first..(first + rows).min(self.found.len()) {
            let item = self.found[at].clone();
            let row_top = head + pad / 4 + under_tiles + (at - first) as i32 * row;
            let picked = at == self.selected;
            if picked {
                canvas.round_fill(pad / 2, row_top, width - pad, row, pad / 2, pick, 256);
            } else if matches!(self.hover, Some(Hit::Item(over) | Hit::Level(over, _)) if over == at) {
                canvas.round_fill(pad / 2, row_top, width - pad, row, pad / 2, WELL_COLOR, 256);
            }
            let (main, dim) = if picked && !self.beside { (ON_ACCENT, DIM_ON_ACCENT) } else { (TEXT_COLOR, DIM_TEXT_COLOR) };
            let text_top = row_top + (row - line) / 2;
            match (item.gauge, self.icon(&item, picture as usize)) {
                // How strong a network is: four bars, as many lit as it has.
                (Gauge::Number(number), _) => {
                    let number = number.to_string();
                    let across = self.text.width(&number, px);
                    self.text.draw(&mut canvas, left + (picture - across) / 2, text_top, px, if picked && !self.beside { ON_ACCENT } else { DIM_TEXT_COLOR }, &number);
                }
                (Gauge::Bars(signal), _) => {
                    let (bar, lit) = ((picture / 7).max(2), (signal as i32 + 24) / 25);
                    for at in 0..4 {
                        let tall = picture * (at + 1) / 4 * 3 / 4;
                        let color = if at < lit { main } else { canvas::mix(PANEL_COLOR, dim, 90) };
                        let color = if picked && at >= lit && !self.beside { canvas::mix(ACCENT, ON_ACCENT, 70) } else { color };
                        canvas.fill(left + at * (bar + bar / 2 + 1), row_top + (row + picture * 3 / 4) / 2 - tall, bar, tall, color);
                    }
                }
                (_, Some(icon)) => canvas.icon(icon, left, row_top + (row - picture) / 2, picture),
                (_, None) => {
                    let across = self.text.width(item.glyph, px);
                    self.text.draw(&mut canvas, left + (picture - across) / 2, text_top, px, dim, item.glyph);
                }
            }
            // A timer's is what is left of it, as it is drawn.
            let hint = match item.gauge {
                Gauge::Clock(end) => format!("{} left", timers::left(&timers::Timer { path: PathBuf::new(), end, label: String::new(), pid: 0 })),
                _ => item.hint.clone(),
            };
            let mut hint_left = width - left - self.text.width(&hint, px);
            self.text.draw(&mut canvas, hint_left, text_top, px, if matches!(item.gauge, Gauge::Clock(_)) { main } else { dim }, &hint);
            // "Open", with the icon of what it opens in.
            if let (Gauge::Editor, Some(editor)) = (item.gauge, self.stock.editor_icon.clone()) {
                let side = line;
                let mut opens = Item::new(glyph::APP, "", "", "", Action::Type(String::new()));
                opens.picture = Picture::Named(editor);
                if let Some(icon) = self.icon(&opens, side as usize) {
                    hint_left -= side + pad / 3;
                    canvas.icon(icon, hint_left, row_top + (row - side) / 2, side);
                }
            }
            // How loud: a bar, filled as far as the level, before the number.
            if let Gauge::Level(level, muted) = item.gauge {
                let thick = (pad / 3).max(4);
                let (from, long) = self.level_bar();
                hint_left = from;
                let trough = if picked && !self.beside { canvas::mix(ACCENT, ON_ACCENT, 70) } else { WELL_COLOR };
                let trough = if picked && self.beside { canvas::mix(WELL_COLOR, TEXT_COLOR, 40) } else { trough };
                canvas.round_fill(hint_left, row_top + (row - thick) / 2, long, thick, thick / 2, trough, 256);
                let filled = long * level.min(100) as i32 / 100;
                canvas.round_fill(hint_left, row_top + (row - thick) / 2, filled, thick, thick / 2, if muted { dim } else { main }, 256);
            }
            let mut title_left = left + picture + pad * 3 / 4;
            // A sum reads as one is written: the sum, quietly, then what it comes to.
            if let Gauge::Sum = item.gauge {
                let sum = self.text.clipped(&item.detail, (hint_left - title_left) / 2, px);
                self.text.draw(&mut canvas, title_left, text_top, px, dim, &sum);
                title_left += self.text.width(&sum, px) + pad / 2;
                let title = self.text.clipped(&item.title, hint_left - pad - title_left, px);
                self.text.draw(&mut canvas, title_left, text_top, px, main, &title);
                continue;
            }
            let room = hint_left - pad - title_left;
            let title = self.text.clipped(&item.title, room, px);
            self.text.draw(&mut canvas, title_left, text_top, px, main, &title);
            let detail_left = title_left + self.text.width(&title, px) + pad * 3 / 4;
            if hint_left - pad - detail_left > 4 * pad {
                let detail = self.text.clipped(&item.detail, hint_left - pad - detail_left, px);
                self.text.draw(&mut canvas, detail_left, text_top, px, dim, &detail);
            }
        }
        if self.found.is_empty() {
            let asked = self.controls.tiles[if self.view == View::Wifi { controls::WIFI } else { controls::SOUND }].known;
            let nothing = match (mode, &self.desk) {
                _ if self.view != View::Find && !asked => "Looking\u{2026}".to_string(),
                (Mode::Shell, _) => "Type a command".to_string(),
                (Mode::Windows | Mode::Workspaces, None) => format!("Windows cannot be seen: {}", self.no_desk),
                _ => "Nothing matches".to_string(),
            };
            let nothing = self.text.clipped(&nothing, width - 2 * left, px);
            self.text.draw(&mut canvas, left, head + pad / 4 + (row - line) / 2, px, DIM_TEXT_COLOR, &nothing);
        }

        // What the first letter can be.
        let small = px * 5 / 6;
        let foot_top = height - self.layout.foot;
        canvas.fill(1, foot_top, full - 2, 1, RULE_COLOR);

        // The controls, a tile each: its name, the number that flips it,
        // and how it stands.
        canvas.fill(width, head, 1, foot_top - head, RULE_COLOR);
        // The one the keys are on; or, with the keys in the list, the
        // one whose insides the list is showing.
        let lit_up = match (self.beside, self.view) {
            (true, _) => Some(self.control),
            (_, View::Wifi) => Some(controls::WIFI),
            (_, View::Sound) => Some(controls::SOUND),
            _ => None,
        };
        // Looking through what was copied, that side shows it whole instead.
        let previewing = self.previewing();
        let shown = if previewing { Vec::new() } else { self.shown.clone() };
        for at in shown {
            let tile = self.controls.tiles[at].clone();
            let (tile_left, tile_top, tile_width, tile_height) = self.control_area(at);
            let (picked, pointed) = (lit_up == Some(at), self.hover == Some(Hit::Control(at)));
            // One that is on is lit in its own color, and the one the
            // keys are on has a line round it.
            let color = CONTROL_COLORS[at];
            let ground = if tile.lit { canvas::mix(PANEL_COLOR, color, if picked { 205 } else { 160 }) } else { WELL_COLOR };
            let ground = if pointed { canvas::mix(ground, TEXT_COLOR, 30) } else { ground };
            if picked {
                canvas.round_fill(tile_left - 2, tile_top - 2, tile_width + 4, tile_height + 4, pad / 2 + 2, if tile.lit { canvas::mix(color, ON_ACCENT, 110) } else { ACCENT }, 256);
            }
            canvas.round_fill(tile_left, tile_top, tile_width, tile_height, pad / 2, ground, 256);
            let (main, dim) = match tile.lit {
                true => (ON_ACCENT, canvas::mix(color, ON_ACCENT, 170)),
                false => (if tile.there || !tile.known { TEXT_COLOR } else { DIM_TEXT_COLOR }, DIM_TEXT_COLOR),
            };
            let inset = pad * 2 / 3;
            // As much of its name as leaves its key clear.
            let name = self.text.clipped(controls::NAMES[at], tile_width - 2 * inset - line, px);
            self.text.draw(&mut canvas, tile_left + inset, tile_top + pad / 2, px, main, &name);
            // Its key, as a project's is drawn: Alt and it flips it.
            let cap = line;
            self.key_cap(&mut canvas, (tile_left + tile_width - inset / 2 - cap, tile_top + pad / 2, cap), ground, controls::key_of(at).unwrap_or(' '));
            // Those with more behind them have a way in, at the foot on the right.
            let way_in = self.way_in(at);
            if let Some((x, y, side, _)) = way_in {
                if self.hover == Some(Hit::Open(at)) {
                    canvas.round_fill(x, y, side, side, side / 2, canvas::mix(ground, 0xFFFFFF, 50), 256);
                }
                let across = self.text.width(glyph::RIGHT, px);
                self.text.draw(&mut canvas, x + (side - across) / 2, y + (side - line) / 2, px, main, glyph::RIGHT);
            }
            let room = tile_width - 2 * inset - way_in.map_or(0, |(_, _, side, _)| side);
            let what = self.text.clipped(&tile.what, room, small);
            // Sound says what it plays through under its name, and has its buttons at its foot.
            let what_top = match at {
                controls::SOUND => tile_top + pad / 2 + line + pad / 6,
                _ => tile_top + tile_height - pad / 2 - Text::line_height(small),
            };
            // (Where sound has too little room for it over its buttons, it goes without.)
            let room = at != controls::SOUND || what_top + Text::line_height(small) <= self.sound_bar().1 - pad / 6;
            if room {
                self.text.draw(&mut canvas, tile_left + inset, what_top, small, dim, &what);
            }
            if at == controls::SOUND {
                let device = self.controls.devices.iter().find(|device| device.used && !device.input).cloned();
                let muted = device.as_ref().is_some_and(|device| device.muted);
                // Mute (or, muted, the sound back), quieter, louder.
                let drawn = [if muted { "\u{e1a9}" } else { "\u{e1ab}" }, "\u{e1a9}", "\u{e1aa}"];
                for ((hit, (x, y, side, _)), drawn) in self.sound_buttons().into_iter().zip(drawn) {
                    let pointed = self.hover == Some(hit);
                    canvas.round_fill(x, y, side, side, side / 2, canvas::mix(ground, if pointed { 0xFFFFFF } else { 0x000000 }, if pointed { 60 } else { 40 }), 256);
                    let across = self.text.width(drawn, px);
                    self.text.draw(&mut canvas, x + (side - across) / 2, y + (side - line) / 2, px, main, drawn);
                }
                // How loud, large, where there is room for it.
                let level_said = match &device {
                    Some(device) if device.muted => "Muted".to_string(),
                    Some(device) => format!("{}%", device.level),
                    None => String::new(),
                };
                let big = px * 9 / 4;
                let big_top = what_top + Text::line_height(small) + pad / 3;
                if big_top + Text::line_height(big) < self.sound_bar().1 - pad / 3 {
                    self.text.draw(&mut canvas, tile_left + inset, big_top, big, main, &level_said);
                }
                let (x, y, long, tall) = self.sound_bar();
                let thick = (pad / 4).max(4);
                let bar_top = y + (tall - thick) / 2;
                canvas.round_fill(x, bar_top, long, thick, thick / 2, canvas::mix(ground, 0x000000, 70), 256);
                let level = device.as_ref().map_or(0, |device| device.level.min(100) as i32);
                canvas.round_fill(x, bar_top, long * level / 100, thick, thick / 2, if muted { dim } else { main }, 256);
                // A knob where it is, to be grabbed by the eye.
                let knob = thick * 3;
                canvas.round_fill(x + long * level / 100 - knob / 2, bar_top + thick / 2 - knob / 2, knob, knob, knob / 2, if muted { dim } else { main }, 256);
            }
        }
        match previewing {
            true => self.draw_preview(&mut canvas),
            false => {
                self.draw_player(&mut canvas, lit_up);
                for at in 0..self.widgets.len() {
                    self.draw_widget(&mut canvas, at);
                }
            }
        }

        let mut at = left;
        let project = Some(("Alt+letter", "go"));
        let named = if self.view == View::Look { self.editor_named() } else { String::new() };
        let found_said = self.found.get(self.selected).map_or(String::new(), |item| {
            let title = self.text.clipped(&item.title, width / 2, small);
            format!("{title} \u{b7} {}", kind_said(item))
        });
        let keys: Vec<(&str, &str)> = match (self.beside, self.view) {
            (true, _) => vec![("Space", "flip"), ("Enter", "open Wi-Fi or Sound"), ("Tab", "back to the list"), ("Esc", if self.view == View::Find { "close" } else { "back" })],
            (_, View::Wifi) => vec![("Enter", "join or leave"), ("Ctrl+R", "scan"), ("Tab", "controls"), ("Esc", "back")],
            // What the keys are on, said first, as an action bar does.
            (_, View::Look) => vec![(named.as_str(), ""), ("Enter", "use"), ("Tab", "next row"), ("Ctrl+Tab", "drawings, emoji\u{2026}"), ("Esc", "back")],
            (_, View::Sound) if self.player().is_some() => vec![("Enter", "play or pause"), ("\u{2190} \u{2192}", "previous, next"), ("Tab", "controls"), ("Esc", "back")],
            (_, View::Sound) => vec![("\u{2190} \u{2192}", "quieter, louder"), ("Enter", "use this"), ("Ctrl+M", "mute"), ("Tab", "controls"), ("Esc", "back")],
            // Searching, what is picked out is said, as an action bar does; with nothing typed, how to look for things.
            _ if !self.field.string().is_empty() => vec![(found_said.as_str(), "")],
            _ => project.into_iter().chain([(">", "do"), ("@", "windows"), ("\"", "clipboard"), (":", "emoji"), ("!", "shell")]).collect(),
        };
        // At the right: what Enter does, and the way to the rest of what can be done.
        let text_top = foot_top + (self.layout.foot - Text::line_height(small)) / 2;
        let caps_top = foot_top + (self.layout.foot - line * 4 / 5) / 2;
        // In the far corner, how long this opening took; a click copies it all.
        if let Some((x, y, wide, tall)) = self.took_area() {
            if self.hover == Some(Hit::Took) {
                canvas.round_fill(x, y, wide, tall, tall / 2, WELL_COLOR, 256);
            }
            let said = self.took_said();
            self.text.draw(&mut canvas, x + pad / 3, text_top, small, DIM_TEXT_COLOR, &said);
            let icon_left = x + pad / 3 + self.text.width(&said, small) + pad / 4;
            self.text.draw(&mut canvas, icon_left, text_top, small, DIM_TEXT_COLOR, glyph::WINDOW);
        }
        let mut right = width - pad;
        if self.view != View::Look && !self.found.is_empty() {
            let from = self.keys_at(&mut canvas, right, caps_top, "Ctrl+K", false);
            let words = "Actions";
            right = from - pad / 3 - self.text.width(words, small);
            self.text.draw(&mut canvas, right, text_top, small, DIM_TEXT_COLOR, words);
            if !self.beside {
                if let Some(main) = self.found.get(self.selected).map(main_said) {
                    canvas.fill(right - pad * 3 / 4, foot_top + self.layout.foot / 4, 1, self.layout.foot / 2, RULE_COLOR);
                    let from = self.keys_at(&mut canvas, right - pad * 3 / 2, caps_top, "Enter", false);
                    right = from - pad / 3 - self.text.width(main, small);
                    self.text.draw(&mut canvas, right, text_top, small, TEXT_COLOR, main);
                }
            }
        }
        for (key, what) in keys {
            if at + self.text.width(key, small) + pad / 3 + self.text.width(what, small) > right - pad {
                break;
            }
            let text_top = foot_top + (self.layout.foot - Text::line_height(small)) / 2;
            self.text.draw(&mut canvas, at, text_top, small, TEXT_COLOR, key);
            at += self.text.width(key, small) + pad / 3;
            self.text.draw(&mut canvas, at, text_top, small, DIM_TEXT_COLOR, what);
            at += self.text.width(what, small) + pad * 3 / 2;
        }
        // Over the list, a look being chosen; over everything, the menu.
        if self.view == View::Look {
            canvas.fill(1, head, width - 1, foot_top - head, PANEL_COLOR);
            self.draw_editor(&mut canvas);
        }
        self.draw_menu(&mut canvas);
        canvas.round_window(corner);
    }

    fn redraw(&mut self) {
        let (Some(window), Some(mut surface)) = (self.window.clone(), self.surface.take()) else { return };
        let size = window.inner_size();
        if let (Some(width), Some(height)) = (std::num::NonZeroU32::new(size.width), std::num::NonZeroU32::new(size.height)) {
            if surface.resize(width, height).is_ok() {
                if let Ok(mut buffer) = surface.buffer_mut() {
                    self.draw(&mut buffer, size.width, size.height);
                    let _ = buffer.present();
                }
            }
        }
        self.surface = Some(surface);
    }

    fn key(&mut self, key: Key<&str>, event_loop: &ActiveEventLoop) {
        let ctrl = self.modifiers.control_key();
        let among_tiles = self.selected < self.tiles;
        // The arrows, and with Ctrl the keys under the fingers that stand for them.
        let arrow = match key {
            Key::Named(NamedKey::ArrowLeft) => Some((-1, 0)),
            Key::Named(NamedKey::ArrowRight) => Some((1, 0)),
            Key::Named(NamedKey::ArrowUp) => Some((0, -1)),
            Key::Named(NamedKey::ArrowDown) => Some((0, 1)),
            // (Ctrl+K is the actions, and Ctrl+P up.)
            Key::Character(letter) if ctrl => [("h", (-1, 0)), ("l", (1, 0)), ("p", (0, -1)), ("j", (0, 1)), ("n", (0, 1))].into_iter().find(|(key, _)| *key == letter).map(|(_, way)| way),
            _ => None,
        };
        // The menu, while it is up, has the keys; then the look being chosen.
        if self.menu.is_some() {
            return self.menu_key(key, arrow, event_loop);
        }
        let menu_key = matches!(key, Key::Named(NamedKey::ContextMenu)) || (matches!(key, Key::Named(NamedKey::F10)) && self.modifiers.shift_key());
        if menu_key {
            let hit = if self.beside { Hit::Control(self.control) } else { Hit::Item(self.selected) };
            // Where the menu comes up, with no pointer to say: by what it is of.
            self.pointer = None;
            return self.open_menu(Some(hit));
        }
        // Ctrl+K: everything that can be done to what is picked out.
        if key == Key::Character("k") && ctrl && self.view != View::Look {
            return self.open_panel();
        }
        // An action's own keys do it without the panel.
        if let Some(keys) = self.keys_pressed(&key) {
            if self.view != View::Look && self.shortcut(&keys, event_loop) {
                return;
            }
        }
        // Alt and up or down: louder or quieter, with the box still up to press again.
        if self.modifiers.alt_key() && matches!(key, Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown)) {
            return self.nudge_volume(if key == Key::Named(NamedKey::ArrowUp) { 5 } else { -5 });
        }
        if self.view == View::Look && self.editor_key(key.clone(), arrow) {
            return;
        }
        let level = self.device().filter(|_| !self.beside);
        let player = self.player().filter(|_| !self.beside);
        match key {
            // Back a step, and out only from where the box opened.
            Key::Named(NamedKey::Escape) if self.view != View::Find => self.back(),
            // Backspace with nothing left to take out goes back too, as a path is gone up.
            Key::Named(NamedKey::Backspace) if self.view != View::Find && self.field.text.is_empty() => self.back(),
            Key::Named(NamedKey::Escape) => return event_loop.exit(),
            Key::Named(NamedKey::Tab) => self.beside = !self.beside,
            // Among the controls: two across and three down. Off their
            // left is the list again.
            Key::Named(NamedKey::Enter) if self.beside => self.press(self.control, true),
            Key::Named(NamedKey::Space) if self.beside => self.press(self.control, false),
            _ if self.beside && arrow.is_some() => {
                let Some((across, down)) = arrow else { return };
                // On Sound, across is how loud.
                if self.control == controls::SOUND && across != 0 {
                    return self.nudge_volume(5 * across);
                }
                // Otherwise they are gone through in the order they stand,
                // the player's buttons last; off the first is the list again.
                let order: Vec<usize> = self.shown.iter().copied().chain([controls::PREVIOUS, controls::PLAY, controls::NEXT]).collect();
                let now = order.iter().position(|at| *at == self.control).unwrap_or(0) as i32;
                let next = now + if down != 0 { down } else { across };
                let to = (next >= 0).then(|| order[(next as usize).min(order.len() - 1)]);
                // Off the left of them is the list again.
                match to {
                    Some(to) => self.control = to,
                    None => self.beside = false,
                }
            }
            Key::Named(NamedKey::Enter) => {
                self.go(self.selected, event_loop);
            }
            // A project's letter with Alt goes straight to it; or with
            // Super, which may still be down from the key that opened
            // the box, where the desktop has no use of its own for the two.
            // A control's letter flips it, and the box goes: Alt+K then
            // Alt+P is a pause, wherever you are.
            Key::Character(letter) if (self.modifiers.alt_key() || self.modifiers.super_key()) && !ctrl => {
                let letter = letter.to_lowercase().chars().next();
                // Among what was copied, Alt and a digit copies that one at once.
                let digit = letter.and_then(|letter| letter.to_digit(10)).filter(|digit| *digit > 0);
                if let (Some(digit), (Mode::Clipboard, _)) = (digit, Mode::of(&self.field.string())) {
                    let copy = self.stock.copies.get(digit as usize - 1).map(|item| item.action.clone());
                    if let Some(action) = copy {
                        if let Some(window) = &self.window {
                            window.set_visible(false);
                        }
                        perform(&action, self.desk.as_ref());
                        return event_loop.exit();
                    }
                }
                let of_it = |item: &Item| item.project.as_ref().is_some_and(|project| Some(project.letter) == letter);
                if let Some(at) = self.stock.projects().iter().position(of_it) {
                    return self.go_to_project(at, event_loop);
                }
                let there = |at: &usize| !self.controls.tiles[*at].known || self.controls.tiles[*at].there;
                // The box stays up, for whatever is wanted next.
                if let Some(at) = (0..controls::COUNT).find(|at| controls::key_of(*at).is_some() && controls::key_of(*at) == letter).filter(there) {
                    self.controls.act_when_known(Deed::Flip(at));
                    return self.heard();
                }
            }
            Key::Character("r") if ctrl && self.view == View::Wifi => self.controls.act(Deed::Scan),
            // On what is playing, the arrows across are the one before and the next.
            _ if player.is_some() && matches!(arrow, Some((-1 | 1, 0))) => {
                self.controls.act(Deed::Tell(player.unwrap(), if arrow == Some((1, 0)) { "Next" } else { "Previous" }));
            }
            // A device's level, by the arrows across; and whether it is muted.
            Key::Character("m") if ctrl && level.is_some() => self.controls.act(Deed::Mute(level.unwrap())),
            _ if level.is_some() && matches!(arrow, Some((-1 | 1, 0))) => {
                let device = level.unwrap();
                let to = (device.level as i32 / 5 * 5 + 5 * arrow.unwrap().0).clamp(0, 150) as u32;
                self.controls.act(Deed::Level(device, to));
                self.heard();
            }
            Key::Named(NamedKey::ArrowDown) => self.step_down(true),
            Key::Named(NamedKey::ArrowUp) => self.step_down(false),
            // Across the tiles, while there is nothing typed to move in.
            Key::Named(NamedKey::ArrowRight) if among_tiles => self.select((self.selected + 1).min(self.tiles - 1)),
            Key::Named(NamedKey::ArrowLeft) if among_tiles => self.select(self.selected.saturating_sub(1)),
            Key::Character("n") if ctrl => self.step(1),
            Key::Character("p") if ctrl => self.step(-1),
            // The keys under the fingers, with Ctrl since plain they are
            // letters to type: J and K down and up, H and L across.
            Key::Character("j") if ctrl => self.step_down(true),
            Key::Character("l") if ctrl && among_tiles => self.select((self.selected + 1).min(self.tiles - 1)),
            Key::Character("h") if ctrl && among_tiles => self.select(self.selected.saturating_sub(1)),
            Key::Named(NamedKey::PageDown) => self.select(self.selected + ROWS),
            Key::Named(NamedKey::PageUp) => self.select(self.selected.saturating_sub(ROWS)),
            key => {
                if self.field.key(key, self.modifiers) {
                    // What is typed is typed into the list.
                    self.beside = false;
                    self.typed_changed();
                }
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler for Palette {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // Where the desktop could not be asked which monitor is in use,
        // the letters are sized for the first one there is.
        if self.place.is_none() {
            let monitor = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next());
            if let Some(monitor) = monitor {
                self.layout = Layout::for_monitor(monitor.size().height as i32, monitor.scale_factor());
            }
        }
        let size = PhysicalSize::new(self.layout.full() as u32, self.layout.height() as u32);
        // The name is what GNOME's shell knows the box by, to place it.
        // Its own icon, for the desktops that show a window's.
        let icon = apps::own().and_then(|icon| {
            let rgba = icon.pixels.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]).collect();
            WindowIcon::from_rgba(rgba, icon.width as u32, icon.height as u32).ok()
        });
        let attributes = Window::default_attributes().with_title("clutch").with_window_icon(icon);
        let attributes = WindowAttributesExtX11::with_name(attributes, "clutch", "clutch");
        let attributes = WindowAttributesExtWayland::with_name(attributes, "clutch", "clutch");
        let mut attributes = attributes
            .with_x11_window_type(vec![WindowType::Dialog])
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            // On Wayland the corners are rounded by leaving them clear.
            .with_transparent(env::var_os("WAYLAND_DISPLAY").is_some())
            .with_inner_size(size);
        if let Some((x, y)) = self.place {
            attributes = attributes.with_position(PhysicalPosition::new(x, y));
        }
        let Ok(window) = event_loop.create_window(attributes) else { return event_loop.exit() };
        crate::timing::mark("window");
        // On X11 the window itself is cut to the shape, which needs
        // nothing of the desktop: not all of them can see through one.
        let x11 = match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Xlib(handle)) => Some(handle.window as u32),
            Ok(RawWindowHandle::Xcb(handle)) => Some(handle.window.get()),
            _ => None,
        };
        if let (Some(id), Some(desk)) = (x11, &self.desk) {
            desk.round(id, size.width as i32, size.height as i32, self.layout.corner());
        }
        let window = Arc::new(window);
        let surface = Context::new(window.clone()).and_then(|context| Surface::new(&context, window.clone()));
        let Ok(surface) = surface else { return event_loop.exit() };
        window.focus_window();
        (self.window, self.surface) = (Some(window), Some(surface));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                self.redraw();
                // The first frame is on the screen: what it took is kept.
                crate::timing::mark("first frame");
                if let Some(took) = crate::timing::finish() {
                    // Said at the foot, drawn again to show it.
                    self.took = Some(took);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                // A font for letters ours lacks is on its way: draw again
                // when it should be in.
                if self.text.waiting() {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(60)));
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            // Not those the toolkit makes up for keys that were down as the
            // box was given the keyboard: Enter, still held as the box went
            // out of the way, would otherwise do what it did twice.
            WindowEvent::KeyboardInput { event, is_synthetic: false, .. } if event.state == ElementState::Pressed => {
                self.key(event.logical_key.as_ref(), event_loop);
                // What is under the pointer may be something else now.
                self.pointed();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = Some((position.x as i32, position.y as i32));
                self.pointed();
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer = None;
                self.pointed();
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.click(event_loop),
            // The right button: what can be done to what it is over.
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                self.menu = None;
                self.pointed();
                let hit = self.hover;
                self.open_menu(hit);
                self.pointed();
            }
            // The wheel moves what is picked out, a step a notch.
            WindowEvent::MouseWheel { delta, .. } => {
                let down = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines < 0.0,
                    MouseScrollDelta::PixelDelta(by) => by.y < 0.0,
                };
                if self.view == View::Look {
                    self.editor_wheel(down);
                } else if self.hover == Some(Hit::Control(controls::SOUND)) {
                    // Over the Sound tile, the wheel is its volume.
                    self.nudge_volume(if down { -5 } else { 5 });
                } else if self.menu.is_none() {
                    self.beside = false;
                    self.step_down(down);
                }
                self.pointed();
            }
            // Clicked away from: it was not wanted after all.
            WindowEvent::Focused(focused) => {
                if self.focused && !focused && !self.away {
                    event_loop.exit();
                }
                self.focused = focused;
            }
            _ => {}
        }
    }

    /// The controls have something new to say.
    fn user_event(&mut self, _: &ActiveEventLoop, _: ()) {
        self.picture_chosen();
        // A widget that was run may have something new to show.
        for (widget, tile) in self.widgets.iter_mut() {
            let fresh = crate::widgets::last(widget).map(|(printed, _)| crate::widgets::tile(widget, &printed));
            if fresh.is_some() && fresh != *tile {
                *tile = fresh;
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        if self.controls.take() {
            self.heard();
            self.pointed();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.text.waiting() {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        } else if self.found.iter().any(|item| matches!(item.gauge, Gauge::Clock(_))) {
            // A timer in view counts down: drawn again each second.
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(1)));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn new_events(&mut self, _: &ActiveEventLoop, cause: winit::event::StartCause) {
        if let winit::event::StartCause::ResumeTimeReached { .. } = cause {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

impl Palette {
    /// The box with `typed` already in it, not yet on the screen.
    fn new(typed: &str) -> Palette {
        let desk = Desk::open();
        crate::timing::mark("desktop");
        // Why not, to be said where the windows would have been listed.
        let no_desk = match (&desk, env::var_os("WAYLAND_DISPLAY")) {
            (Some(_), _) => String::new(),
            (None, Some(_)) => format!("GNOME extension {}", extension::health().err().unwrap_or("gave no windows".into())),
            (None, None) => "no X display".to_string(),
        };
        // In the middle of the monitor in use, each way. Where that cannot
        // be asked, the desktop places the box.
        let monitor = desk.as_ref().and_then(Desk::monitor_in_use);
        let scale = desk.as_ref().and_then(Desk::text_scale).unwrap_or(1.0);
        let layout = Layout::for_monitor(monitor.map_or(1080, |(_, _, _, height)| height), scale);
        let place = monitor.map(|(x, y, width, height)| (x + (width - layout.full()) / 2, y + ((height - layout.height()) / 2).max(0)));
        let stock = Stock::gather(desk.as_ref());
        let mut palette = Palette {
            desk,
            no_desk,
            stock,
            field: TextField::new(typed),
            found: Vec::new(),
            selected: 0,
            first: 0,
            tiles: 0,
            band: 0,
            modifiers: ModifiersState::empty(),
            layout,
            place,
            text: Text::load(),
            icons: HashMap::new(),
            window: None,
            surface: None,
            focused: false,
            controls: Controls::new(),
            view: View::Find,
            beside: false,
            control: 0,
            pointer: None,
            hover: None,
            looks: looks::load(),
            menu: None,
            editing: None,
            away: false,
            chosen: None,
            wake: None,
            git: Default::default(),
            shown: controls::shown(),
            widgets: Vec::new(),
            took: None,
        };
        // The widgets, as they last were: what the running clutch keeps.
        {
            let widgets = std::mem::take(&mut palette.stock.widgets);
            palette.widgets = widgets.into_iter().map(|widget| {
                let tile = crate::widgets::last(&widget).map(|(printed, _)| crate::widgets::tile(&widget, &printed));
                (widget, tile)
            }).collect();
            crate::timing::mark("widgets");
        }
        palette.typed_changed();
        palette
    }
}

/// Open the box with `typed` already in it, and stay until something is
/// picked or it is dismissed.
///
/// Asked for while it is already up, it is taken down instead: the key
/// that opens the box also closes it.
pub fn run(typed: &str) -> Result<(), String> {
    crate::timing::begin();
    // The box that is up listens here, and goes when anything calls.
    let door = config::socket().with_file_name("clutch-palette.sock");
    if UnixStream::connect(&door).is_ok() {
        return Ok(());
    }
    let _ = fs::remove_file(&door);
    if let Ok(listener) = UnixListener::bind(&door) {
        let door = door.clone();
        thread::spawn(move || {
            let _ = listener.accept();
            let _ = fs::remove_file(&door);
            process::exit(0);
        });
    }
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    crate::timing::mark("event loop");
    let mut palette = Palette::new(typed);
    // The box is up before the controls have answered, and is woken as
    // each does.
    let proxy = event_loop.create_proxy();
    let wake = event_loop.create_proxy();
    palette.wake = Some(Arc::new(move || {
        let _ = wake.send_event(());
    }));
    // A widget whose output is old, the running clutch not having run it,
    // is run now, without waiting for it: it is drawn again when it answers.
    for (widget, _) in palette.widgets.iter().filter(|(widget, _)| crate::widgets::due(widget)) {
        let (widget, wake) = (widget.clone(), palette.wake.clone());
        thread::spawn(move || {
            if crate::widgets::refresh(&widget) {
                if let Some(wake) = wake {
                    wake();
                }
            }
        });
    }
    palette.controls.start(move || {
        let _ = proxy.send_event(());
    });
    crate::timing::mark("controls asked");
    let run = event_loop.run_app(&mut palette).map_err(|e| e.to_string());
    let _ = fs::remove_file(&door);
    run
}

/// What the box would list for `typed`, a line for each.
pub fn list(typed: &str) {
    for item in &Palette::new(typed).found {
        println!("{}\t{}\t{}", item.title, item.detail, item.hint);
    }
}

/// The box as it would be drawn for `typed`, written to a PNG and never
/// put on the screen: with `row`, that row picked out. A `typed` of
/// "wifi:" or "sound:" is what is behind that control.
pub fn picture(typed: &str, row: usize, path: &str) -> Result<(), String> {
    let behind = [("wifi:", View::Wifi), ("sound:", View::Sound)].into_iter().find_map(|(word, view)| Some((typed.strip_prefix(word)?, view)));
    // "look:" is the look of what is at `row` being chosen, and "menu:" its menu.
    let (look, menu) = (typed.strip_prefix("look:"), typed.strip_prefix("menu:"));
    let typed = look.or(menu).unwrap_or(behind.map_or(typed, |(typed, _)| typed));
    let mut palette = Palette::new(if look.is_some() { "" } else { typed });
    palette.controls.start(|| {});
    palette.controls.settle();
    if let Some((_, view)) = behind {
        palette.view = view;
        palette.typed_changed();
    }
    palette.select(row);
    if let Some(project) = look.and_then(|_| palette.found.get(row)?.project.clone()) {
        palette.edit(dress::Target { name: project.name, folder: project.folder });
        palette.field = TextField::new(typed);
        palette.typed_changed();
    }
    if menu.is_some() {
        palette.pointer = Some((palette.layout.pad * 8, palette.layout.head + palette.layout.pad));
        palette.open_menu(Some(Hit::Item(row)));
    }
    let (width, height) = (palette.layout.full() as u32, palette.layout.height() as u32);
    let mut pixels = vec![0u32; (width * height) as usize];
    palette.draw(&mut pixels, width, height);
    // Once more when the fonts for letters ours lacks have come in.
    while palette.text.waiting() {
        std::thread::sleep(Duration::from_millis(50));
        palette.draw(&mut pixels, width, height);
    }
    let file = fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    let bytes: Vec<u8> = pixels.iter().flat_map(|pixel| [(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8]).collect();
    let written = encoder.write_header().and_then(|mut writer| writer.write_image_data(&bytes));
    written.map_err(|e| format!("{path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stock() -> Stock {
        let item = |title: &str, detail: &str| Item::new(glyph::APP, title, detail, "", Action::Clutch("status"));
        Stock {
            windows: vec![item("clutch — Mozilla Firefox", "firefox"), item("notes.md", "code")],
            apps: vec![item("Firefox", "Web Browser"), item("Files", "File Manager")],
            commands: vec![item("Close", "notes.md"), item("tilix --quake", "")],
            workspaces: vec![item("Workspace 1", ""), item("Workspace 2", "")],
            controls: Vec::new(),
            closed: Vec::new(),
            shelved: Vec::new(),
            copies: Vec::new(),
            links: vec![("gh".into(), "https://github.com/search?q={}".into())],
            editor_icon: None,
            widgets: Vec::new(),
            recent: None,
        }
    }

    fn titles(typed: &str) -> Vec<String> {
        stock().matching(typed).into_iter().map(|item| item.title).collect()
    }

    #[test]
    fn the_first_letter_says_what_is_looked_for() {
        assert_eq!(titles(""), ["clutch — Mozilla Firefox", "notes.md"]);
        assert_eq!(titles("fire"), ["clutch — Mozilla Firefox", "Firefox"]);
        assert_eq!(titles("@fire"), ["clutch — Mozilla Firefox"]);
        assert_eq!(titles(">clo"), ["Close"]);
        assert_eq!(titles("#2"), ["Workspace 2"]);
        assert_eq!(titles("!ls -la"), ["ls -la"]);
        assert!(titles("!").is_empty());
    }

    #[test]
    fn a_copy_by_its_number() {
        let mut stock = stock();
        stock.copies = ["newest", "older", "oldest"].iter().map(|text| Item::new(glyph::CLIPBOARD, text, "", "", Action::Copy(text.to_string()))).collect();
        let titles = |typed: &str| stock.matching(typed).into_iter().map(|item| item.title).collect::<Vec<_>>();
        assert_eq!(titles("\"2"), ["older"]);
        assert_eq!(titles("\"1"), ["newest"]);
        assert!(titles("\"9").is_empty());
        assert_eq!(titles("\"old"), ["older", "oldest"]);
    }

    #[test]
    fn sums_links_and_timers_come_first() {
        let first = |typed: &str| stock().matching(typed).into_iter().next().map(|item| item.title).unwrap_or_default();
        assert_eq!(first("12*7"), "= 84");
        assert_eq!(first("gh/clutch"), "Open github.com/search?q=clutch");
        assert_eq!(first("gh/two words"), "Open github.com/search?q=two+words");
        assert_eq!(first("gh/jeffjose/dotfiles"), "Open github.com/search?q=jeffjose/dotfiles");
        assert_ne!(first("gh clutch"), "Open github.com/search?q=clutch");
        assert_eq!(first("timer 25m tea"), "Timer 25 min \u{b7} tea");
    }

    #[test]
    fn a_web_address() {
        assert_eq!(web_address("youtube.com"), Some(("https://youtube.com".to_string(), true)));
        assert_eq!(web_address("github.com/jeffjose/clutch"), Some(("https://github.com/jeffjose/clutch".to_string(), true)));
        assert_eq!(web_address("main.rs"), Some(("https://main.rs".to_string(), false)));
        assert_eq!(web_address("localhost:5173"), Some(("http://localhost:5173".to_string(), true)));
        assert_eq!(web_address("hello world.com"), None);
        assert_eq!(web_address("firefox"), None);
        let found = stock().matching("youtube.com");
        assert!(matches!(&found[0].action, Action::Shell(line) if line == "xdg-open 'https://youtube.com'"));
    }

    #[test]
    fn what_matches_nothing_can_still_be_run() {
        let found = stock().matching("htop");
        assert!(matches!(&found[0].action, Action::Typed(line) if line == "htop"));
        // And, where there is somewhere projects are kept, a new project of that name.
        assert!(found.iter().skip(1).all(|item| item.title == "New project htop"));
    }

    #[test]
    fn scattered_letters_only_when_nothing_has_them_together() {
        // "fls" is in neither a window nor an application as typed.
        assert_eq!(titles("fls"), ["Files"]);
        assert_eq!(titles("files"), ["Files"]);
    }
}
