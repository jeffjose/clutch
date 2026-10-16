//! The windows, monitors and workspaces of the desktop, and what can be
//! done to them. On X11 any program may ask the window manager. On
//! Wayland none may: there GNOME's shell is asked, through the part of
//! clutch that `clutch setup` puts in it.

use crate::extension;
use crate::palette::canvas::Icon;
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ClipOrdering, ConnectionExt, EventMask, Rectangle};
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

pub struct Window {
    pub id: u32,
    pub title: String,
    /// The program it belongs to, as it calls itself: "firefox".
    pub class: String,
    /// Which workspace it is on, counted from 0; none if on them all.
    pub workspace: Option<u32>,
    /// The name of its program's icon, where the desktop says: on X11 a
    /// window brings its icon itself.
    pub icon: String,
}

/// The desktop, whichever way it is reached.
pub enum Desk {
    X11(X11),
    Gnome(Gnome),
}

impl Desk {
    pub fn open() -> Option<Desk> {
        match env::var_os("WAYLAND_DISPLAY") {
            Some(_) => Gnome::open().map(Desk::Gnome),
            None => X11::open().map(Desk::X11),
        }
    }

    /// The window that has the keyboard.
    pub fn active(&self) -> Option<u32> {
        match self {
            Desk::X11(desk) => desk.active(),
            Desk::Gnome(desk) => desk.state.windows.iter().find(|window| window.front).map(|window| window.id),
        }
    }

    /// The windows a taskbar would show, the one used last first.
    pub fn windows(&self) -> Vec<Window> {
        match self {
            Desk::X11(desk) => desk.windows(),
            Desk::Gnome(desk) => desk.windows(),
        }
    }

    /// A window's own icon, where it brings one.
    pub fn icon(&self, window: u32, size: usize) -> Option<Icon> {
        match self {
            Desk::X11(desk) => desk.icon(window, size),
            Desk::Gnome(_) => None,
        }
    }

    /// Cut a window of ours to a rectangle with rounded corners, where
    /// the desktop lets a window be cut. On Wayland it is drawn so.
    pub fn round(&self, window: u32, width: i32, height: i32, radius: i32) {
        if let Desk::X11(desk) = self {
            desk.round(window, width, height, radius);
        }
    }

    pub fn monitor_count(&self) -> usize {
        match self {
            Desk::X11(desk) => desk.monitors().len(),
            Desk::Gnome(desk) => desk.state.monitors,
        }
    }

    /// The monitor the pointer is on, where a window may say where it
    /// goes. GNOME's shell places clutch's box itself.
    pub fn monitor_in_use(&self) -> Option<Area> {
        match self {
            Desk::X11(desk) => desk.monitor_in_use(),
            Desk::Gnome(_) => None,
        }
    }

    /// How much larger than plain the desktop draws its letters, where
    /// that can be asked here: on Wayland the window is told.
    pub fn text_scale(&self) -> Option<f64> {
        match self {
            Desk::X11(desk) => Some(desk.text_scale()),
            Desk::Gnome(_) => None,
        }
    }

    /// The names of the workspaces, and which one is in view.
    pub fn workspaces(&self) -> (Vec<String>, u32) {
        match self {
            Desk::X11(desk) => desk.workspaces(),
            Desk::Gnome(desk) => (desk.state.workspaces.clone(), desk.state.workspace),
        }
    }

    pub fn activate(&self, window: u32) {
        match self {
            Desk::X11(desk) => desk.activate(window),
            Desk::Gnome(desk) => desk.ask("Activate", &[window.to_string()]),
        }
    }

    pub fn close(&self, window: u32) {
        match self {
            Desk::X11(desk) => desk.close(window),
            Desk::Gnome(desk) => desk.ask("Close", &[window.to_string()]),
        }
    }

    pub fn toggle_maximized(&self, window: u32) {
        match self {
            Desk::X11(desk) => desk.toggle_maximized(window),
            Desk::Gnome(desk) => desk.ask("Maximize", &[window.to_string()]),
        }
    }

    pub fn toggle_fullscreen(&self, window: u32) {
        match self {
            Desk::X11(desk) => desk.toggle_fullscreen(window),
            Desk::Gnome(desk) => desk.ask("Fullscreen", &[window.to_string()]),
        }
    }

    /// Move a window to the next monitor, or with `back` the one before.
    /// `None` if there is no other.
    pub fn move_to_monitor(&self, window: u32, back: bool) -> Option<()> {
        match self {
            Desk::X11(desk) => desk.move_to_monitor(window, back),
            Desk::Gnome(desk) => (desk.state.monitors > 1).then(|| desk.ask("MoveToMonitor", &[window.to_string(), back.to_string()])),
        }
    }

    pub fn go_to_workspace(&self, workspace: u32) {
        match self {
            Desk::X11(desk) => desk.go_to_workspace(workspace),
            Desk::Gnome(desk) => desk.ask("GoTo", &[workspace.to_string()]),
        }
    }

    /// Send a window to another workspace, without following it there.
    pub fn send_to_workspace(&self, window: u32, workspace: u32) {
        match self {
            Desk::X11(desk) => desk.send_to_workspace(window, workspace),
            Desk::Gnome(desk) => desk.ask("SendTo", &[window.to_string(), workspace.to_string()]),
        }
    }
}

/// GNOME's shell, asked through clutch's extension. Everything there is
/// to list comes in one answer, as the box opens.
pub struct Gnome {
    state: State,
}

#[derive(Deserialize)]
struct State {
    windows: Vec<GnomeWindow>,
    workspaces: Vec<String>,
    workspace: u32,
    monitors: usize,
}

#[derive(Deserialize)]
struct GnomeWindow {
    id: u32,
    title: String,
    program: String,
    icon: String,
    /// Below 0 for a window that is on every workspace.
    workspace: i64,
    front: bool,
}

impl Gnome {
    fn open() -> Option<Gnome> {
        let printed = extension::call("State", &[])?;
        let state = serde_json::from_str(&extension::unwrap(&printed)?).ok()?;
        Some(Gnome { state })
    }

    fn windows(&self) -> Vec<Window> {
        let window = |w: &GnomeWindow| Window {
            id: w.id,
            title: w.title.clone(),
            class: w.program.clone(),
            workspace: u32::try_from(w.workspace).ok(),
            icon: w.icon.clone(),
        };
        self.state.windows.iter().map(window).collect()
    }

    fn ask(&self, method: &str, arguments: &[String]) {
        extension::call(method, arguments);
    }
}

pub struct X11 {
    conn: RustConnection,
    root: u32,
    atoms: RefCell<HashMap<&'static str, u32>>,
}

/// A rectangle of the screen: left, top, width, height.
type Area = (i32, i32, i32, i32);

/// To a pager's request the window manager gives way, where an
/// application's own it may refuse.
const FROM_PAGER: u32 = 2;

impl X11 {
    pub fn open() -> Option<X11> {
        // Under Wayland there is still an X display, for old programs,
        // and it knows only of their windows.
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            return None;
        }
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        Some(X11 { conn, root, atoms: RefCell::default() })
    }

    fn atom(&self, name: &'static str) -> u32 {
        if let Some(atom) = self.atoms.borrow().get(name) {
            return *atom;
        }
        let asked = self.conn.intern_atom(false, name.as_bytes()).ok().and_then(|cookie| cookie.reply().ok());
        let atom = asked.map_or(0, |reply| reply.atom);
        self.atoms.borrow_mut().insert(name, atom);
        atom
    }

    fn numbers(&self, window: u32, property: &'static str, most: u32) -> Vec<u32> {
        let asked = self.conn.get_property(false, window, self.atom(property), AtomEnum::ANY, 0, most);
        asked.ok().and_then(|cookie| cookie.reply().ok()).and_then(|reply| reply.value32().map(Iterator::collect)).unwrap_or_default()
    }

    fn bytes(&self, window: u32, property: &'static str) -> Vec<u8> {
        let asked = self.conn.get_property(false, window, self.atom(property), AtomEnum::ANY, 0, 4096);
        asked.ok().and_then(|cookie| cookie.reply().ok()).map_or(Vec::new(), |reply| reply.value)
    }

    /// Ask the window manager for something, as a pager would.
    fn ask(&self, window: u32, what: &'static str, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, window, self.atom(what), data);
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        let _ = self.conn.send_event(false, self.root, mask, event);
        let _ = self.conn.flush();
    }

    /// The window that has the keyboard.
    pub fn active(&self) -> Option<u32> {
        self.numbers(self.root, "_NET_ACTIVE_WINDOW", 1).first().copied().filter(|id| *id != 0)
    }

    /// Cut a window to a rectangle with rounded corners: a strip for
    /// each row of the curves, and one piece for all between.
    fn round(&self, window: u32, width: i32, height: i32, radius: i32) {
        let radius = radius.clamp(0, width.min(height) / 2);
        let strip = |x: i32, y: i32, w: i32, h: i32| Rectangle { x: x as i16, y: y as i16, width: w as u16, height: h as u16 };
        let mut shape = vec![strip(0, radius, width, height - 2 * radius)];
        for row in 0..radius {
            // How far the curve cuts into this row.
            let from_middle = f64::from(radius) - f64::from(row) - 0.5;
            let inset = (f64::from(radius) - (f64::from(radius * radius) - from_middle * from_middle).max(0.0).sqrt()).round() as i32;
            shape.push(strip(inset, row, width - 2 * inset, 1));
            shape.push(strip(inset, height - 1 - row, width - 2 * inset, 1));
        }
        let _ = self.conn.shape_rectangles(SO::SET, SK::BOUNDING, ClipOrdering::UNSORTED, window, 0, 0, &shape);
        let _ = self.conn.flush();
    }

    /// The windows a taskbar would show, the one used last first.
    pub fn windows(&self) -> Vec<Window> {
        // The window manager keeps them from the bottom of the pile up,
        // which is near enough to the order they were last used in.
        let pile = self.numbers(self.root, "_NET_CLIENT_LIST_STACKING", 4096);
        let (skip, dock, desktop) =
            (self.atom("_NET_WM_STATE_SKIP_TASKBAR"), self.atom("_NET_WM_WINDOW_TYPE_DOCK"), self.atom("_NET_WM_WINDOW_TYPE_DESKTOP"));
        let mut windows = Vec::new();
        for id in pile.into_iter().rev() {
            let kinds = self.numbers(id, "_NET_WM_WINDOW_TYPE", 8);
            let states = self.numbers(id, "_NET_WM_STATE", 16);
            if states.contains(&skip) || kinds.contains(&dock) || kinds.contains(&desktop) {
                continue;
            }
            let mut title = String::from_utf8_lossy(&self.bytes(id, "_NET_WM_NAME")).into_owned();
            if title.is_empty() {
                title = String::from_utf8_lossy(&self.bytes(id, "WM_NAME")).into_owned();
            }
            // Two names, each ended by a zero: the second is the program's.
            let class = self.bytes(id, "WM_CLASS");
            let class = class.split(|byte| *byte == 0).filter(|part| !part.is_empty()).next_back();
            let class = class.map_or(String::new(), |class| String::from_utf8_lossy(class).into_owned());
            let workspace = self.numbers(id, "_NET_WM_DESKTOP", 1).first().copied().filter(|at| *at != u32::MAX);
            windows.push(Window { id, title, class, workspace, icon: String::new() });
        }
        windows
    }

    /// A window's own icon, the smallest it has that is at least `size`
    /// across, or else its largest.
    pub fn icon(&self, window: u32, size: usize) -> Option<Icon> {
        // Each size it comes in: the width, the height, then the pixels.
        let data = self.numbers(window, "_NET_WM_ICON", u32::MAX / 4);
        let mut best: Option<(usize, usize, &[u32])> = None;
        let mut rest = &data[..];
        while let [width, height, after @ ..] = rest {
            let (width, height) = (*width as usize, *height as usize);
            let length = width.checked_mul(height).filter(|length| *length > 0 && *length <= after.len())?;
            let better = best.is_none_or(|(had, _, _)| if had >= size { width >= size && width < had } else { width > had });
            if better {
                best = Some((width, height, &after[..length]));
            }
            rest = &after[length..];
        }
        best.map(|(width, height, pixels)| Icon { width, height, pixels: pixels.to_vec() })
    }

    /// Bring a window to the front and give it the keyboard, going to
    /// its workspace if it is on another.
    pub fn activate(&self, window: u32) {
        self.ask(window, "_NET_ACTIVE_WINDOW", [FROM_PAGER, CURRENT_TIME, 0, 0, 0]);
    }

    pub fn close(&self, window: u32) {
        self.ask(window, "_NET_CLOSE_WINDOW", [CURRENT_TIME, FROM_PAGER, 0, 0, 0]);
    }

    fn maximized(&self, window: u32) -> bool {
        let states = self.numbers(window, "_NET_WM_STATE", 16);
        states.contains(&self.atom("_NET_WM_STATE_MAXIMIZED_VERT")) && states.contains(&self.atom("_NET_WM_STATE_MAXIMIZED_HORZ"))
    }

    fn set_maximized(&self, window: u32, on: bool) {
        let (vert, horz) = (self.atom("_NET_WM_STATE_MAXIMIZED_VERT"), self.atom("_NET_WM_STATE_MAXIMIZED_HORZ"));
        self.ask(window, "_NET_WM_STATE", [u32::from(on), vert, horz, FROM_PAGER, 0]);
    }

    pub fn toggle_maximized(&self, window: u32) {
        self.set_maximized(window, !self.maximized(window));
    }

    pub fn toggle_fullscreen(&self, window: u32) {
        const TOGGLE: u32 = 2;
        self.ask(window, "_NET_WM_STATE", [TOGGLE, self.atom("_NET_WM_STATE_FULLSCREEN"), 0, FROM_PAGER, 0]);
    }

    /// The monitors, from the left.
    pub fn monitors(&self) -> Vec<Area> {
        let asked = self.conn.randr_get_monitors(self.root, true).ok().and_then(|cookie| cookie.reply().ok());
        let mut monitors: Vec<Area> = asked.map_or(Vec::new(), |reply| {
            let area = |m: &x11rb::protocol::randr::MonitorInfo| (i32::from(m.x), i32::from(m.y), i32::from(m.width), i32::from(m.height));
            reply.monitors.iter().map(area).collect()
        });
        monitors.sort();
        monitors
    }

    /// What the desktop has set Xft.dpi to, over the 96 that is plain.
    fn text_scale(&self) -> f64 {
        let resources = String::from_utf8_lossy(&self.bytes(self.root, "RESOURCE_MANAGER")).into_owned();
        let dpi = resources.lines().find_map(|line| line.strip_prefix("Xft.dpi:")?.trim().parse::<f64>().ok());
        dpi.map_or(1.0, |dpi| (dpi / 96.0).clamp(0.5, 4.0))
    }

    /// The monitor the pointer is on, which is where one is looking.
    pub fn monitor_in_use(&self) -> Option<Area> {
        let pointer = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        let (x, y) = (i32::from(pointer.root_x), i32::from(pointer.root_y));
        let monitors = self.monitors();
        let holds = |(left, top, width, height): &Area| (*left..left + width).contains(&x) && (*top..top + height).contains(&y);
        monitors.iter().find(|area| holds(area)).or(monitors.first()).copied()
    }

    /// Where a window is with its frame, and how much of that is frame:
    /// left, right, top, bottom.
    fn frame(&self, window: u32) -> Option<(Area, [i32; 4])> {
        let size = self.conn.get_geometry(window).ok()?.reply().ok()?;
        let at = self.conn.translate_coordinates(window, self.root, 0, 0).ok()?.reply().ok()?;
        let edges = self.numbers(window, "_NET_FRAME_EXTENTS", 4);
        let edge = |which: usize| edges.get(which).copied().unwrap_or(0) as i32;
        let edges = [edge(0), edge(1), edge(2), edge(3)];
        let area = (
            i32::from(at.dst_x) - edges[0],
            i32::from(at.dst_y) - edges[2],
            i32::from(size.width) + edges[0] + edges[1],
            i32::from(size.height) + edges[2] + edges[3],
        );
        Some((area, edges))
    }

    /// Move a window to the monitor after the one it is on, or with
    /// `back` the one before, to the same place on it; a window that
    /// filled its monitor fills the new one.
    pub fn move_to_monitor(&self, window: u32, back: bool) -> Option<()> {
        let monitors = self.monitors();
        let ((x, y, width, height), edges) = self.frame(window)?;
        let middle = (x + width / 2, y + height / 2);
        let holds = |(left, top, w, h): &Area| (*left..left + w).contains(&middle.0) && (*top..top + h).contains(&middle.1);
        let from = monitors.iter().position(holds)?;
        let step = if back { monitors.len() - 1 } else { 1 };
        let (now, next) = (monitors[from], monitors[(from + step) % monitors.len()]);
        if now == next {
            return None;
        }
        // As far across and down the new monitor as it was the old, and
        // no larger than the new one.
        let (width, height) = (width.min(next.2), height.min(next.3));
        let place = |at: i32, size: i32, old: (i32, i32), new: (i32, i32)| {
            let room = (old.1 - size).max(1);
            new.0 + i64::from(at - old.0).clamp(0, i64::from(room)) as i32 * (new.1 - size).max(0) / room
        };
        let left = place(x, width, (now.0, now.2), (next.0, next.2));
        let top = place(y, height, (now.1, now.3), (next.1, next.3));

        let maximized = self.maximized(window);
        if maximized {
            self.set_maximized(window, false);
        }
        self.move_resize(window, (left, top, width, height), edges);
        if maximized {
            self.set_maximized(window, true);
        }
        Some(())
    }

    /// Move a window's frame to a rectangle of the screen, and make the
    /// window in it as large as the frame leaves room for.
    fn move_resize(&self, window: u32, (left, top, width, height): Area, edges: [i32; 4]) {
        const X_Y_WIDTH_HEIGHT: u32 = 0xF << 8;
        const NORTH_WEST: u32 = 1;
        let flags = NORTH_WEST | X_Y_WIDTH_HEIGHT | FROM_PAGER << 12;
        let inner = ((width - edges[0] - edges[1]).max(1) as u32, (height - edges[2] - edges[3]).max(1) as u32);
        self.ask(window, "_NET_MOVERESIZE_WINDOW", [flags, left as u32, top as u32, inner.0, inner.1]);
    }

    /// Put a window on a rectangle of the screen, frame and all.
    pub fn put(&self, window: u32, area: Area) {
        if self.maximized(window) {
            self.set_maximized(window, false);
        }
        let edges = self.frame(window).map_or([0; 4], |(_, edges)| edges);
        self.move_resize(window, area, edges);
    }

    /// The window of a program by its class, as WM_CLASS has it, among
    /// all the window manager keeps: those off the taskbar as well.
    pub fn find_class(&self, class: &str) -> Option<u32> {
        let all = self.numbers(self.root, "_NET_CLIENT_LIST", 4096);
        all.into_iter().find(|id| self.bytes(*id, "WM_CLASS").split(|byte| *byte == 0).any(|name| name == class.as_bytes()))
    }

    /// Whether a window is minimized.
    pub fn hidden(&self, window: u32) -> bool {
        self.numbers(window, "_NET_WM_STATE", 16).contains(&self.atom("_NET_WM_STATE_HIDDEN"))
    }

    pub fn minimize(&self, window: u32) {
        const ICONIC: u32 = 3;
        self.ask(window, "WM_CHANGE_STATE", [ICONIC, 0, 0, 0, 0]);
    }

    /// Keep a window over the others and on every workspace, and leave
    /// it off the taskbar and the pager.
    pub fn keep_up(&self, window: u32) {
        const ADD: u32 = 1;
        for [one, two] in [["_NET_WM_STATE_ABOVE", "_NET_WM_STATE_STICKY"], ["_NET_WM_STATE_SKIP_TASKBAR", "_NET_WM_STATE_SKIP_PAGER"]] {
            self.ask(window, "_NET_WM_STATE", [ADD, self.atom(one), self.atom(two), FROM_PAGER, 0]);
        }
        self.ask(window, "_NET_WM_DESKTOP", [u32::MAX, FROM_PAGER, 0, 0, 0]);
    }

    /// The main monitor, or the first where none is.
    fn main_monitor(&self) -> Option<Area> {
        let reply = self.conn.randr_get_monitors(self.root, true).ok()?.reply().ok()?;
        let monitor = reply.monitors.iter().find(|monitor| monitor.primary).or(reply.monitors.first())?;
        Some((i32::from(monitor.x), i32::from(monitor.y), i32::from(monitor.width), i32::from(monitor.height)))
    }

    /// Where windows may go on the main monitor, or the one in use: all
    /// of it but what panels keep for themselves.
    pub fn work_area(&self, main: bool) -> Option<Area> {
        let monitor = if main { self.main_monitor() } else { self.monitor_in_use() }?;
        let desktop = self.numbers(self.root, "_NET_CURRENT_DESKTOP", 1).first().copied().unwrap_or(0) as usize;
        let areas = self.numbers(self.root, "_NET_WORKAREA", 4096);
        // The window manager says it for all the monitors together, so
        // only where that cuts into this one counts.
        let area = areas.chunks_exact(4).nth(desktop).and_then(|area| <[u32; 4]>::try_from(area).ok());
        let Some([x, y, width, height]) = area.map(|area| area.map(|n| n as i32)) else {
            return Some(monitor);
        };
        let (left, top) = (monitor.0.max(x), monitor.1.max(y));
        let (right, bottom) = ((monitor.0 + monitor.2).min(x + width), (monitor.1 + monitor.3).min(y + height));
        match right > left && bottom > top {
            true => Some((left, top, right - left, bottom - top)),
            false => Some(monitor),
        }
    }

    /// The names of the workspaces, and which one is in view.
    pub fn workspaces(&self) -> (Vec<String>, u32) {
        let count = self.numbers(self.root, "_NET_NUMBER_OF_DESKTOPS", 1).first().copied().unwrap_or(0) as usize;
        let named = self.bytes(self.root, "_NET_DESKTOP_NAMES");
        let mut names: Vec<String> = named.split(|byte| *byte == 0).map(|name| String::from_utf8_lossy(name).into_owned()).collect();
        names.resize(count, String::new());
        for (at, name) in names.iter_mut().enumerate().filter(|(_, name)| name.is_empty()) {
            *name = format!("Workspace {}", at + 1);
        }
        (names, self.numbers(self.root, "_NET_CURRENT_DESKTOP", 1).first().copied().unwrap_or(0))
    }

    pub fn go_to_workspace(&self, workspace: u32) {
        self.ask(self.root, "_NET_CURRENT_DESKTOP", [workspace, CURRENT_TIME, 0, 0, 0]);
    }

    /// Send a window to another workspace, without following it there.
    pub fn send_to_workspace(&self, window: u32, workspace: u32) {
        self.ask(window, "_NET_WM_DESKTOP", [workspace, FROM_PAGER, 0, 0, 0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_shell_answers() {
        // As gdbus printed it, from the extension run against a made-up shell.
        let printed = r#"('{"version":2,"windows":[{"id":7,"title":"It\\u0027s a \\u201ctest\\u201d \\\\ ok \\u2014 \\ud55c\\uad6d","program":"Firefox","icon":"firefox","workspace":0,"front":true},{"id":12,"title":"everywhere","program":"x","icon":"","workspace":-1,"front":false}],"workspaces":["Workspace 1","Workspace 2"],"workspace":1,"monitors":2}',)"#;
        let state: State = serde_json::from_str(&extension::unwrap(printed).unwrap()).unwrap();
        let desk = Desk::Gnome(Gnome { state });
        let windows = desk.windows();
        assert_eq!(windows[0].title, "It's a “test” \\ ok — 한국");
        assert_eq!((windows[0].class.as_str(), windows[0].icon.as_str(), windows[0].workspace), ("Firefox", "firefox", Some(0)));
        assert_eq!(windows[1].workspace, None);
        assert_eq!(desk.active(), Some(7));
        assert_eq!(desk.workspaces(), (vec!["Workspace 1".to_string(), "Workspace 2".to_string()], 1));
        assert_eq!(desk.monitor_count(), 2);
    }
}
