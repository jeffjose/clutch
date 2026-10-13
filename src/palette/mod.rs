//! The box that everything is found from: type, and the windows,
//! applications and commands that match are listed; Enter goes to the
//! one picked out. What is typed first says what is being looked for.
//!
//! It is a program run for as long as the box is up, and has nothing to
//! do with the clutch that holds the keyboards: any key, bound anywhere,
//! can open it.

mod apps;
pub mod canvas;
mod field;
mod matching;

use crate::config::{self, Config};
use crate::desk::Desk;
use crate::extension;
use canvas::{Canvas, Icon, Text};
use field::TextField;
use softbuffer::{Context, Surface};
use std::collections::HashMap;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{self, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, fs, thread};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::platform::x11::{WindowAttributesExtX11, WindowType};
use winit::window::{Window, WindowId, WindowLevel};

const TEXT_COLOR: u32 = 0xEEEEEE;
const DIM_TEXT_COLOR: u32 = 0x9A9A9A;
const PANEL_COLOR: u32 = 0x1E1E1E;
const EDGE_COLOR: u32 = 0x555555;
const RULE_COLOR: u32 = 0x333333;
const ACCENT: u32 = 0x3E63DD;
const ON_ACCENT: u32 = 0xFFFFFF;
/// The lesser words of the row picked out: grey on the accent is mud.
const DIM_ON_ACCENT: u32 = 0xC9D3F6;

/// How many matches are in view at once.
const ROWS: usize = 8;

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
}

impl Mode {
    /// The mode, and what was typed after the letter that chose it.
    fn of(typed: &str) -> (Mode, &str) {
        let mode = match typed.chars().next() {
            Some('>') => Mode::Run,
            Some('@') => Mode::Windows,
            Some('#') => Mode::Workspaces,
            Some('!') => Mode::Shell,
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
        }
    }
}

/// The picture at the front of a row, where it has one of its own.
#[derive(Clone)]
enum Picture {
    None,
    Window(u32),
    Named(String),
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
}

impl Item {
    fn new(glyph: &'static str, title: &str, detail: &str, hint: &str, action: Action) -> Item {
        let find = format!("{title} {detail}");
        Item { glyph, picture: Picture::None, title: title.into(), detail: detail.into(), hint: hint.into(), find, action }
    }
}

/// Everything there is to find, gathered once as the box opens.
#[derive(Default)]
struct Stock {
    windows: Vec<Item>,
    apps: Vec<Item>,
    commands: Vec<Item>,
    workspaces: Vec<Item>,
}

impl Stock {
    fn gather(desk: Option<&Desk>) -> Stock {
        let mut stock = Stock::default();
        let mut sends = Vec::new();
        let active = desk.and_then(Desk::active);

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
                stock.windows.push(item);
            }
            for (at, name) in names.iter().enumerate() {
                let hint = if at as u32 == current { "here" } else { "" };
                stock.workspaces.push(Item::new(glyph::WORKSPACE, name, "", hint, Action::Workspace(at as u32)));
            }
            // What can be done to the window that was in front.
            if let Some(window) = active.filter(|id| windows.iter().any(|window| window.id == *id)) {
                let to = |glyph, name: &str, action| Item::new(glyph, name, "", "window", action);
                if desk.monitor_count() > 1 {
                    stock.commands.push(to(glyph::RIGHT, "Move window to next monitor", Action::MoveToMonitor { window, back: false }));
                    stock.commands.push(to(glyph::LEFT, "Move window to previous monitor", Action::MoveToMonitor { window, back: true }));
                }
                stock.commands.push(to(glyph::MAXIMIZE, "Maximize or restore window", Action::Maximize(window)));
                stock.commands.push(to(glyph::FULLSCREEN, "Fullscreen on or off", Action::Fullscreen(window)));
                stock.commands.push(to(glyph::CLOSE, "Close window", Action::Close(window)));
                // One for each workspace: last, where they are out of the way.
                for (at, name) in names.iter().enumerate().filter(|(at, _)| *at as u32 != current) {
                    sends.push(to(glyph::WORKSPACE, &format!("Send window to {name}"), Action::SendTo { window, workspace: at as u32 }));
                }
            }
        }

        for app in apps::all() {
            let action = Action::Launch { exec: app.exec, terminal: app.terminal };
            let mut item = Item::new(glyph::APP, &app.name, &app.kind, "", action);
            item.find = format!("{} {} {}", app.name, app.kind, app.keywords);
            item.picture = Picture::Named(app.icon);
            stock.apps.push(item);
        }

        // The bindings, which can be run from here as well as by their keys.
        let settings = fs::read_to_string(config::path()).unwrap_or_default();
        if let Ok((settings, _)) = Config::parse(&settings) {
            for binding in settings.bindings {
                stock.commands.push(Item::new(glyph::KEYBOARD, &binding.command, "", &binding.keys, Action::Shell(binding.command.clone())));
            }
        }
        let mut clutch = |glyph, name: &str, action| stock.commands.push(Item::new(glyph, name, "", "clutch", action));
        clutch(glyph::PENCIL, "Edit settings", Action::EditSettings);
        clutch(glyph::REFRESH, "Reload settings", Action::Clutch("reload"));
        clutch(glyph::PAUSE, "Pause clutch", Action::Clutch("pause"));
        clutch(glyph::PLAY, "Resume clutch", Action::Clutch("resume"));
        stock.commands.extend(sends);
        stock
    }

    /// What matches what was typed, the best first.
    fn matching(&self, typed: &str) -> Vec<Item> {
        let (mode, typed) = Mode::of(typed);
        let pick = |items: &[Item]| -> Vec<Item> {
            let finds = items.iter().map(|item| item.find.as_str());
            matching::ranked(typed, finds).into_iter().map(|at| items[at].clone()).collect()
        };
        let shell = |line: &str| {
            let line = line.trim();
            Item::new(glyph::TERMINAL, line, "", "run in a shell", Action::Shell(line.into()))
        };
        match mode {
            Mode::Windows => pick(&self.windows),
            Mode::Workspaces => pick(&self.workspaces),
            Mode::Run => pick(&self.commands),
            Mode::Shell if typed.trim().is_empty() => Vec::new(),
            Mode::Shell => vec![shell(typed)],
            // With nothing typed: the windows, to go straight to one. Where
            // they cannot be seen, the applications.
            Mode::Find if typed.trim().is_empty() => match self.windows.is_empty() {
                true => self.apps.clone(),
                false => self.windows.clone(),
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
                    scored.extend(of(&self.apps, 0));
                    scored.sort_by_key(|(score, _)| -score);
                    scored.into_iter().map(|(_, item)| item).collect()
                };
                let mut found = match found(false) {
                    together if together.is_empty() => found(true),
                    together => together,
                };
                // Whatever else it is, it can be run.
                if found.is_empty() {
                    found.push(shell(typed));
                }
                found
            }
        }
    }
}

/// Start something and leave it running after the box has gone.
fn spawn(line: &str) {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(config::with_clutch(line)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    if let Some(home) = env::var_os("HOME") {
        command.current_dir(home);
    }
    let _ = command.spawn();
}

fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn perform(action: &Action, desk: Option<&Desk>) {
    match (action, desk) {
        (Action::Launch { exec, terminal: false }, _) | (Action::Shell(exec), _) => spawn(exec),
        (Action::Launch { exec, terminal: true }, _) => spawn(&format!("x-terminal-emulator -e sh -c {}", shell_quoted(exec))),
        (Action::EditSettings, _) => spawn(&format!("xdg-open {}", shell_quoted(&config::path().to_string_lossy()))),
        (Action::Clutch(what), _) => drop(crate::ask(what)),
        (Action::Activate(window), Some(desk)) => desk.activate(*window),
        (Action::Workspace(workspace), Some(desk)) => desk.go_to_workspace(*workspace),
        (Action::SendTo { window, workspace }, Some(desk)) => desk.send_to_workspace(*window, *workspace),
        (Action::MoveToMonitor { window, back }, Some(desk)) => drop(desk.move_to_monitor(*window, *back)),
        (Action::Maximize(window), Some(desk)) => desk.toggle_maximized(*window),
        (Action::Fullscreen(window), Some(desk)) => desk.toggle_fullscreen(*window),
        (Action::Close(window), Some(desk)) => desk.close(*window),
        (_, None) => {}
    }
}

/// The sizes of the box's parts, all from the size of its letters.
#[derive(Clone, Copy)]
struct Layout {
    px: u32,
    pad: i32,
    line: i32,
    width: i32,
    head: i32,
    row: i32,
    foot: i32,
}

impl Layout {
    /// For a monitor this tall, letters as large as jog's on it.
    fn for_monitor(height: i32) -> Layout {
        let px = (height / 90).clamp(14, 30) as u32;
        let (pad, line) = (px as i32, Text::line_height(px));
        Layout { px, pad, line, width: pad * 46, head: line + 2 * pad, row: line + pad, foot: Text::line_height(px * 5 / 6) + pad }
    }

    fn height(&self) -> i32 {
        self.head + ROWS as i32 * self.row + self.pad / 2 + self.foot
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
}

impl Palette {
    fn typed_changed(&mut self) {
        self.found = self.stock.matching(&self.field.string());
        (self.selected, self.first) = (0, 0);
    }

    fn select(&mut self, to: usize) {
        self.selected = to.min(self.found.len().saturating_sub(1));
        self.first = self.first.clamp((self.selected + 1).saturating_sub(ROWS), self.selected);
    }

    fn step(&mut self, by: isize) {
        let count = self.found.len() as isize;
        if count > 0 {
            self.select((self.selected as isize + by).rem_euclid(count) as usize);
        }
    }

    /// The picture for a row, fetched the first time the row is drawn.
    fn icon(&mut self, item: &Item, size: usize) -> Option<&Icon> {
        let key = match &item.picture {
            Picture::None => return None,
            Picture::Window(id) => format!("window {id}"),
            Picture::Named(name) => name.clone(),
        };
        let desk = self.desk.as_ref();
        let fetch = || match &item.picture {
            // A window that brings no icon may still be of a program
            // that has one.
            Picture::Window(id) => desk.and_then(|desk| desk.icon(*id, size)).or_else(|| apps::icon(&item.detail.to_lowercase())),
            Picture::Named(name) => apps::icon(name),
            Picture::None => None,
        };
        self.icons.entry(key).or_insert_with(fetch).as_ref()
    }

    fn draw(&mut self, pixels: &mut [u32], width: u32, height: u32) {
        let mut canvas = Canvas::new(pixels, width, height);
        let Layout { px, pad, line, head, row, .. } = self.layout;
        let (width, height) = (canvas.width, canvas.height);
        canvas.fill(0, 0, width, height, EDGE_COLOR);
        canvas.fill(1, 1, width - 2, height - 2, PANEL_COLOR);

        // What is being looked for, and what has been typed.
        let typed = self.field.string();
        let (mode, _) = Mode::of(&typed);
        let (icon, label) = mode.label();
        let (left, top) = (pad * 3 / 2, pad);
        self.text.draw(&mut canvas, left, top, px, DIM_TEXT_COLOR, icon);
        let label_left = left + self.text.width(icon, px) + pad / 2;
        self.text.draw(&mut canvas, label_left, top, px, DIM_TEXT_COLOR, label);
        let typed_left = label_left + self.text.width(label, px) + pad;
        if let Some((from, to)) = self.field.selection() {
            let mut upto = |at: usize| self.text.width(&self.field.text[..at].iter().collect::<String>(), px);
            let (from, to) = (upto(from), upto(to));
            canvas.fill(typed_left + from, top, to - from, line, ACCENT);
        }
        self.text.draw(&mut canvas, typed_left, top, px, TEXT_COLOR, &typed);
        let before_cursor: String = self.field.text[..self.field.cursor].iter().collect();
        canvas.fill(typed_left + self.text.width(&before_cursor, px), top, (pad / 10).max(2), line, TEXT_COLOR);
        let note = match self.found.len() {
            0 => String::new(),
            count => format!("{} of {count}", self.selected + 1),
        };
        let note_left = width - left - self.text.width(&note, px);
        self.text.draw(&mut canvas, note_left, top, px, DIM_TEXT_COLOR, &note);
        canvas.fill(1, head - 1, width - 2, 1, RULE_COLOR);

        // The matches.
        let picture = row - pad / 2;
        for at in self.first..(self.first + ROWS).min(self.found.len()) {
            let item = self.found[at].clone();
            let row_top = head + pad / 4 + (at - self.first) as i32 * row;
            let picked = at == self.selected;
            if picked {
                canvas.round_fill(pad / 2, row_top, width - pad, row, pad / 2, ACCENT, 256);
            }
            let (main, dim) = if picked { (ON_ACCENT, DIM_ON_ACCENT) } else { (TEXT_COLOR, DIM_TEXT_COLOR) };
            let text_top = row_top + (row - line) / 2;
            match self.icon(&item, picture as usize) {
                Some(icon) => canvas.icon(icon, left, row_top + (row - picture) / 2, picture),
                None => {
                    let across = self.text.width(item.glyph, px);
                    self.text.draw(&mut canvas, left + (picture - across) / 2, text_top, px, dim, item.glyph);
                }
            }
            let hint_left = width - left - self.text.width(&item.hint, px);
            self.text.draw(&mut canvas, hint_left, text_top, px, dim, &item.hint);
            let title_left = left + picture + pad * 3 / 4;
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
            let nothing = match (mode, &self.desk) {
                (Mode::Shell, _) => "Type a command, and Enter runs it".to_string(),
                (Mode::Windows | Mode::Workspaces, None) => format!("Windows cannot be seen: {}", self.no_desk),
                _ => "Nothing matches".to_string(),
            };
            let nothing = self.text.clipped(&nothing, width - 2 * left, px);
            self.text.draw(&mut canvas, left, head + pad / 4 + (row - line) / 2, px, DIM_TEXT_COLOR, &nothing);
        }

        // What the first letter can be.
        let small = px * 5 / 6;
        let foot_top = height - self.layout.foot;
        canvas.fill(1, foot_top, width - 2, 1, RULE_COLOR);
        let mut at = left;
        for (key, what) in [(">", "do"), ("@", "windows"), ("#", "workspaces"), ("!", "shell"), ("Esc", "close")] {
            let text_top = foot_top + (self.layout.foot - Text::line_height(small)) / 2;
            self.text.draw(&mut canvas, at, text_top, small, TEXT_COLOR, key);
            at += self.text.width(key, small) + pad / 3;
            self.text.draw(&mut canvas, at, text_top, small, DIM_TEXT_COLOR, what);
            at += self.text.width(what, small) + pad * 3 / 2;
        }
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
        match key {
            Key::Named(NamedKey::Escape) => return event_loop.exit(),
            Key::Named(NamedKey::Enter) => {
                if let Some(item) = self.found.get(self.selected) {
                    // Out of the way first, so that the window gone to is
                    // the last thing to be given the keyboard.
                    if let Some(window) = &self.window {
                        window.set_visible(false);
                    }
                    perform(&item.action, self.desk.as_ref());
                }
                return event_loop.exit();
            }
            Key::Named(NamedKey::ArrowDown) => self.step(1),
            Key::Named(NamedKey::ArrowUp) => self.step(-1),
            Key::Named(NamedKey::Tab) if self.modifiers.shift_key() => self.step(-1),
            Key::Named(NamedKey::Tab) => self.step(1),
            Key::Character("n") if ctrl => self.step(1),
            Key::Character("p") if ctrl => self.step(-1),
            Key::Named(NamedKey::PageDown) => self.select(self.selected + ROWS),
            Key::Named(NamedKey::PageUp) => self.select(self.selected.saturating_sub(ROWS)),
            key => {
                if self.field.key(key, self.modifiers) {
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
                self.layout = Layout::for_monitor(monitor.size().height as i32);
            }
        }
        let size = PhysicalSize::new(self.layout.width as u32, self.layout.height() as u32);
        // The name is what GNOME's shell knows the box by, to place it.
        let attributes = Window::default_attributes().with_title("clutch");
        let attributes = WindowAttributesExtX11::with_name(attributes, "clutch", "clutch");
        let attributes = WindowAttributesExtWayland::with_name(attributes, "clutch", "clutch");
        let mut attributes = attributes
            .with_x11_window_type(vec![WindowType::Dialog])
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(size);
        if let Some((x, y)) = self.place {
            attributes = attributes.with_position(PhysicalPosition::new(x, y));
        }
        let Ok(window) = event_loop.create_window(attributes) else { return event_loop.exit() };
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
                // A font for letters ours lacks is on its way: draw again
                // when it should be in.
                if self.text.waiting() {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(60)));
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key(event.logical_key.as_ref(), event_loop);
            }
            // Clicked away from: it was not wanted after all.
            WindowEvent::Focused(focused) => {
                if self.focused && !focused {
                    event_loop.exit();
                }
                self.focused = focused;
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.text.waiting() {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

impl Palette {
    /// The box with `typed` already in it, not yet on the screen.
    fn new(typed: &str) -> Palette {
        let desk = Desk::open();
        // Why not, to be said where the windows would have been listed.
        let no_desk = match (&desk, env::var_os("WAYLAND_DISPLAY")) {
            (Some(_), _) => String::new(),
            (None, Some(_)) => format!("GNOME extension {}", extension::health().err().unwrap_or("gave no windows".into())),
            (None, None) => "no X display".to_string(),
        };
        // On the monitor in use, across its middle and a fifth of the way
        // down. Where that cannot be asked, the desktop places the box.
        let monitor = desk.as_ref().and_then(Desk::monitor_in_use);
        let layout = Layout::for_monitor(monitor.map_or(1080, |(_, _, _, height)| height));
        let place = monitor.map(|(x, y, width, height)| (x + (width - layout.width) / 2, y + height / 5));
        let stock = Stock::gather(desk.as_ref());
        let mut palette = Palette {
            desk,
            no_desk,
            stock,
            field: TextField::new(typed),
            found: Vec::new(),
            selected: 0,
            first: 0,
            modifiers: ModifiersState::empty(),
            layout,
            place,
            text: Text::load(),
            icons: HashMap::new(),
            window: None,
            surface: None,
            focused: false,
        };
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
    let run = event_loop.run_app(&mut Palette::new(typed)).map_err(|e| e.to_string());
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
/// put on the screen: with `row`, that row picked out.
pub fn picture(typed: &str, row: usize, path: &str) -> Result<(), String> {
    let mut palette = Palette::new(typed);
    palette.select(row);
    let (width, height) = (palette.layout.width as u32, palette.layout.height() as u32);
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
    fn what_matches_nothing_can_still_be_run() {
        let found = stock().matching("htop");
        assert_eq!(found.len(), 1);
        assert!(matches!(&found[0].action, Action::Shell(line) if line == "htop"));
    }

    #[test]
    fn scattered_letters_only_when_nothing_has_them_together() {
        // "fls" is in neither a window nor an application as typed.
        assert_eq!(titles("fls"), ["Files"]);
        assert_eq!(titles("files"), ["Files"]);
    }
}
