//! The running clutch: it holds the keyboards, reads every key, and
//! writes the ones it does not keep to a keyboard of its own.

use crate::config::{self, Binding, Config};
use crate::engine::{Engine, Out};
use crate::gnome;
use crate::setup::RULE_PATH;
use std::sync::{Arc, Mutex};
use crate::keys::{KEY_BACKSPACE, KEY_ENTER, KEY_ESC};
use evdev::uinput::VirtualDevice;
use evdev::{AbsoluteAxisCode, AttributeSet, Device, EventType, InputEvent, KeyCode};
use evdev::RelativeAxisCode;
use std::collections::HashSet;
use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram, UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

/// The name of clutch's own keyboard. A keyboard whose name starts with
/// it is never taken, or clutch would be reading what it wrote.
const NAME: &str = "clutch";
const UINPUT: &str = "/dev/uinput";
const INPUT: &str = "/dev/input";

pub struct Options {
    pub config: PathBuf,
    /// Take these and no others, whatever they are. For the tests.
    pub devices: Vec<PathBuf>,
}

struct Keyboard {
    path: PathBuf,
    name: String,
    device: Device,
    /// Ours alone: no program sees its keys but through clutch.
    held: bool,
    down: HashSet<u16>,
}

struct Daemon {
    options: Options,
    /// The settings file as last read, to tell when it has changed.
    text: Option<String>,
    problem: Option<String>,
    bindings: Vec<Binding>,
    runs: Vec<u32>,
    engine: Engine,
    keyboards: Vec<Keyboard>,
    /// What turned out not to be a keyboard, so it is not opened again.
    passed_over: HashSet<PathBuf>,
    out: Option<VirtualDevice>,
    out_down: HashSet<u16>,
    paused: bool,
    /// The bindings are GNOME's shortcuts, as the keyboards cannot be held.
    gnome: bool,
    /// How the bindings stand with GNOME, once it has been given them.
    given: Arc<Mutex<gnome::Given>>,
    under_systemd: bool,
    started: Instant,
}

fn log(message: impl AsRef<str>) {
    eprintln!("clutch: {}", message.as_ref());
}

fn fail(message: impl AsRef<str>) -> ! {
    log(message);
    process::exit(1);
}

pub fn run(options: Options) -> ! {
    let socket = config::socket();
    if UnixStream::connect(&socket).is_ok() {
        fail("already running");
    }
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)
        .unwrap_or_else(|e| fail(format!("{}: {e}", socket.display())));
    listener.set_nonblocking(true).ok();

    if let Some(dir) = options.config.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let watch = Watch::new(&[
        (Path::new(INPUT), libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB),
        (
            options.config.parent().unwrap_or(Path::new(".")),
            libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE | libc::IN_DELETE,
        ),
    ]);

    let (settings, tree) = Config::parse("").unwrap();
    let mut daemon = Daemon {
        options,
        text: None,
        problem: None,
        bindings: Vec::new(),
        runs: Vec::new(),
        engine: Engine::new(tree, settings.timeout),
        keyboards: Vec::new(),
        passed_over: HashSet::new(),
        out: None,
        out_down: HashSet::new(),
        paused: false,
        gnome: false,
        given: Arc::default(),
        under_systemd: env::var_os("NOTIFY_SOCKET").is_some(),
        started: Instant::now(),
    };
    daemon.reload();
    notify("READY=1");

    let mut looked = None::<Instant>;
    let mut asked = None::<Instant>;
    loop {
        // The seat is ours while we may write to uinput: with the udev
        // rule that is while we are logged in at the machine itself.
        let ours = may_write(UINPUT);
        if !ours && daemon.out.is_some() {
            log("no longer at the keyboard: letting go");
            daemon.let_go();
            daemon.keyboards.clear();
            daemon.out = None;
        }
        if ours && daemon.out.is_none() {
            daemon.out = keyboard_of_our_own();
        }
        if daemon.out.is_some() && daemon.gnome {
            log("the keyboards can be held after all: taking the bindings back from GNOME");
            daemon.gnome = false;
            gnome::sync_later(Vec::new(), Vec::new(), daemon.given.clone());
        }
        // With no udev rule the keyboards will never be ours. GNOME can
        // have the bindings in their place; it is asked for every so
        // often because before anyone logs in there is no desktop.
        let ask = daemon.out.is_none() && !daemon.gnome && !Path::new(RULE_PATH).exists();
        if ask && asked.is_none_or(|at| at.elapsed() >= Duration::from_secs(2)) {
            asked = Some(Instant::now());
            if daemon.desktop().contains("GNOME") {
                log("no way to hold the keyboards here: giving the bindings to GNOME");
                daemon.gnome = true;
                daemon.give_to_gnome();
            }
        }
        if daemon.out.is_some() && looked.is_none_or(|at| at.elapsed() >= Duration::from_secs(2)) {
            daemon.look_for_keyboards();
            looked = Some(Instant::now());
        }
        if !daemon.paused {
            for keyboard in daemon.keyboards.iter_mut().filter(|k| !k.held) {
                take(keyboard);
            }
        }

        let mut fds: Vec<libc::pollfd> = [watch.fd, listener.as_raw_fd()]
            .into_iter()
            .chain(daemon.keyboards.iter().map(|k| k.device.as_raw_fd()))
            .map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 })
            .collect();
        let mut wait = Duration::from_secs(1);
        if let Some(deadline) = daemon.engine.deadline() {
            wait = wait.min(deadline.saturating_duration_since(Instant::now()));
        }
        if daemon.keyboards.iter().any(|k| !k.held) && !daemon.paused {
            // One is waiting for its keys to come up before it is taken.
            wait = wait.min(Duration::from_millis(250));
        }
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, wait.as_millis() as i32) };
        notify("WATCHDOG=1");

        // From the last, so that one unplugged does not move the rest.
        for at in (0..daemon.keyboards.len()).rev() {
            if fds[at + 2].revents != 0 {
                daemon.read(at);
            }
        }
        daemon.engine.tick(Instant::now());

        if fds[0].revents != 0 {
            watch.drain();
            daemon.passed_over.clear();
            looked = None;
            daemon.reload();
        }
        if fds[1].revents != 0 {
            while let Ok((stream, _)) = listener.accept() {
                daemon.answer(stream);
            }
        }
    }
}

impl Daemon {
    /// Read the settings file if it is not as it was.
    fn reload(&mut self) -> String {
        let text = fs::read_to_string(&self.options.config).ok();
        if text == self.text {
            return "unchanged".into();
        }
        let parsed = Config::parse(text.as_deref().unwrap_or(""));
        self.text = text;
        match parsed {
            Ok((settings, tree)) => {
                self.engine.rebind(tree, settings.timeout);
                self.runs = vec![0; settings.bindings.len()];
                self.bindings = settings.bindings;
                self.problem = None;
                if self.gnome {
                    self.give_to_gnome();
                }
                let said = format!("{} bound", plural(self.bindings.len(), "key"));
                log(format!("{}: {said}", self.options.config.display()));
                said
            }
            Err(problem) => {
                // The bindings stay as they were before the mistake.
                log(format!("{}: {problem}", self.options.config.display()));
                self.problem = Some(problem.clone());
                problem
            }
        }
    }

    fn look_for_keyboards(&mut self) {
        let paths: Vec<PathBuf> = if !self.options.devices.is_empty() {
            self.options.devices.clone()
        } else {
            let Ok(dir) = fs::read_dir(INPUT) else { return };
            dir.flatten()
                .filter(|entry| entry.file_name().as_bytes().starts_with(b"event"))
                .map(|entry| entry.path())
                .collect()
        };
        for path in paths {
            if self.passed_over.contains(&path) || self.keyboards.iter().any(|k| k.path == path) {
                continue;
            }
            // One we may not open yet is tried again the next time.
            let Ok(device) = Device::open(&path) else { continue };
            let name = device.name().unwrap_or("").to_string();
            let wanted = !self.options.devices.is_empty()
                || (is_keyboard(&device) && !name.starts_with(NAME));
            if !wanted || device.set_nonblocking(true).is_err() {
                self.passed_over.insert(path);
                continue;
            }
            log(format!("keyboard: {name} ({})", path.display()));
            self.keyboards.push(Keyboard { path, name, device, held: false, down: HashSet::new() });
        }
    }

    /// The keys that have arrived from one keyboard.
    fn read(&mut self, at: usize) {
        let fetched: io::Result<Vec<(u16, i32)>> =
            self.keyboards[at].device.fetch_events().map(|events| {
                events
                    .filter(|event| event.event_type() == EventType::KEY)
                    .map(|event| (event.code(), event.value()))
                    .collect()
            });
        let keys = match fetched {
            Ok(keys) => keys,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
            Err(_) => {
                // Unplugged. The keys it had down come up.
                let gone = self.keyboards.remove(at);
                log(format!("keyboard gone: {}", gone.name));
                if gone.held {
                    for code in gone.down {
                        self.key(code, 0);
                    }
                }
                return;
            }
        };
        // Before a keyboard is ours the programs have its keys already.
        if !self.keyboards[at].held {
            return;
        }
        for (code, value) in keys {
            let keyboard = &mut self.keyboards[at];
            match value {
                0 => keyboard.down.remove(&code),
                1 => keyboard.down.insert(code),
                _ => false,
            };
            // The way out that does not depend on anything else working.
            if [KEY_BACKSPACE, KEY_ESC, KEY_ENTER].iter().all(|key| keyboard.down.contains(key)) {
                log("Backspace, Escape and Enter together: stopping");
                process::exit(0);
            }
            self.key(code, value);
        }
    }

    fn key(&mut self, code: u16, value: i32) {
        let mut out = Vec::new();
        self.engine.key(code, value, Instant::now(), &mut out);
        for out in out {
            match out {
                Out::Key(code, value) => self.write(code, value),
                Out::Run(binding) => {
                    self.runs[binding] += 1;
                    launch(&self.bindings[binding], self.under_systemd);
                }
            }
        }
    }

    fn write(&mut self, code: u16, value: i32) {
        let Some(out) = &mut self.out else { return };
        match value {
            0 => self.out_down.remove(&code),
            _ => self.out_down.insert(code),
        };
        // If the keys cannot be passed on, nobody should be holding them.
        if let Err(e) = out.emit(&[InputEvent::new(EventType::KEY.0, code, value)]) {
            fail(format!("writing a key: {e}"));
        }
    }

    fn desktop(&self) -> String {
        if !self.under_systemd {
            return env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        }
        session_environment()
            .into_iter()
            .find(|(name, _)| name == "XDG_CURRENT_DESKTOP")
            .map_or(String::new(), |(_, value)| value)
    }

    /// Make GNOME's shortcuts of ours the bindings, or none while paused.
    fn give_to_gnome(&mut self) {
        let (shortcuts, said) = match self.paused {
            true => (Vec::new(), Vec::new()),
            false => gnome::shortcuts(&self.bindings),
        };
        gnome::sync_later(shortcuts, said, self.given.clone());
    }

    /// Give the keyboards back to the programs, with no key left down.
    fn let_go(&mut self) {
        for keyboard in self.keyboards.iter_mut().filter(|k| k.held) {
            let _ = keyboard.device.ungrab();
            keyboard.held = false;
            keyboard.down.clear();
        }
        for code in std::mem::take(&mut self.out_down) {
            self.write(code, 0);
        }
        self.out_down.clear();
        self.engine.reset();
    }

    fn answer(&mut self, mut stream: UnixStream) {
        stream.set_nonblocking(false).ok();
        stream.set_read_timeout(Some(Duration::from_millis(200))).ok();
        let mut buffer = [0; 64];
        let length = stream.read(&mut buffer).unwrap_or(0);
        let asked = String::from_utf8_lossy(&buffer[..length]).trim().to_string();
        let (what, with) = asked.split_once(' ').unwrap_or((&asked, ""));
        let reply = match what {
            "status" => {
                let (columns, colour) = with.split_once(' ').unwrap_or((with, ""));
                self.status(columns.parse().unwrap_or(0), colour == "colour")
            }
            "fix" => self.fix(),
            "state" => format!("{}\n", self.state()),
            "pause" => {
                self.paused = true;
                self.let_go();
                if self.gnome {
                    self.give_to_gnome();
                }
                "paused: the keys are as they were without clutch\n".into()
            }
            "resume" => {
                self.paused = false;
                if self.gnome {
                    self.give_to_gnome();
                }
                "resumed\n".into()
            }
            "reload" => {
                let said = self.reload();
                // Asked for, GNOME is given the bindings again even if
                // the file is as it was: its side may be what changed.
                if self.gnome && said == "unchanged" {
                    self.give_to_gnome();
                }
                format!("{said}\n")
            }
            other => format!("not something clutch does: {other}\n"),
        };
        let _ = stream.write_all(reply.as_bytes());
    }

    /// How things stand, in a few words.
    fn state(&self) -> String {
        let held = self.keyboards.iter().filter(|k| k.held).count();
        if self.paused {
            "paused".into()
        } else if self.gnome {
            "GNOME shortcuts (the keyboard cannot be held here)".into()
        } else if self.out.is_none() {
            "waiting for keyboard access".into()
        } else if held == 0 {
            "no keyboard found".into()
        } else {
            format!("holding {}", plural(held, "keyboard"))
        }
    }

    /// Everything about the running clutch: how it stands, then a line
    /// for each binding with a mark for whether it works. For a terminal
    /// `columns` wide, or 0 for something that is not one, and in colour
    /// if asked.
    fn status(&self, columns: usize, colour: bool) -> String {
        let paint = |code: &str, text: &str| match colour {
            true => format!("\x1b[{code}m{text}\x1b[0m"),
            false => text.to_string(),
        };
        let (bold, dim, green, red, yellow) = ("1", "2", "32", "31", "33");
        let given = self.given.lock().unwrap();
        let working = !self.paused && (self.gnome || self.keyboards.iter().any(|k| k.held));

        let version = format!("clutch {}", env!("CARGO_PKG_VERSION"));
        let up = format!(" · up {}", ago(self.started.elapsed()));
        let mut text = format!("{}{}\n{}\n", paint(bold, &version), paint(dim, &up), self.state());
        for keyboard in &self.keyboards {
            let waiting = if keyboard.held { "" } else { " (not held yet)" };
            text += &paint(dim, &format!("  {}{waiting}", keyboard.name));
            text += "\n";
        }
        text += &paint(dim, &config::short(&self.options.config));
        text += "\n";
        if let Some(problem) = &self.problem {
            text += &paint(red, &format!("  {problem}"));
            text += &paint(dim, " (so the bindings are as they were)");
            text += "\n";
        }
        if self.gnome {
            for note in given.notes.iter().filter(|note| note.keys.is_empty()) {
                text += &paint(red, &format!("  {}", note.what));
                text += "\n";
            }
        }
        text += "\n";

        let width = |text: &str| text.chars().count();
        let keys = self.bindings.iter().map(|b| width(&b.keys)).max().unwrap_or(0);
        let widest = self.bindings.iter().map(|b| width(&b.command)).max().unwrap_or(0);
        // What is in front of a command, and the count that may follow it.
        let (before, after) = (2 + 1 + 2 + keys + 2, 2 + 6);
        let command = if columns > 0 { widest.min(columns.saturating_sub(before + after).max(12)) } else { widest };
        let failed = self.gnome && given.notes.iter().any(|note| note.keys.is_empty());
        let mut broken = 0;
        for (binding, runs) in self.bindings.iter().zip(&self.runs) {
            let note = given.notes.iter().find(|note| self.gnome && note.keys == binding.keys);
            // The mark: working, not working, or not in force.
            let mark = match note {
                _ if !working => paint(dim, "–"),
                Some(note) if note.fix.is_none() && note.what.starts_with("skipped") => paint(yellow, "–"),
                Some(_) => paint(red, "✗"),
                None if failed => paint(red, "✗"),
                None if self.gnome && !given.done => paint(dim, "…"),
                None => paint(green, "✓"),
            };
            let pressed = if !self.gnome && *runs > 0 { format!("{runs}×") } else { String::new() };
            let shown = format!("{:command$}", cut(&binding.command, command));
            let row = format!("  {mark}  {:keys$}  {}  {}", binding.keys, paint(dim, &shown), pressed);
            text += row.trim_end();
            text += "\n";
            if let Some(note) = note.filter(|_| working) {
                broken += usize::from(note.fix.is_some());
                let colour = if note.fix.is_some() { red } else { yellow };
                text += &format!("{:before$}{}\n", "", paint(colour, &note.what));
            }
        }
        if self.bindings.is_empty() {
            text += &paint(dim, "  nothing bound");
            text += "\n";
        }
        if broken > 0 {
            let them = if broken == 1 { "its keys" } else { "their keys" };
            text += &format!("\n{} not working. {} takes {them} from GNOME.\n", plural(broken, "binding"), paint(bold, "clutch fix"));
        }
        text
    }

    /// Take from GNOME the shortcuts that are on our bindings' keys.
    fn fix(&mut self) -> String {
        if !self.gnome {
            return "nothing to fix: the keys clutch holds are never the desktop's\n".into();
        }
        let notes = self.given.lock().unwrap().notes.clone();
        let said = gnome::fix(&notes);
        if said.is_empty() {
            return "nothing to fix\n".into();
        }
        self.give_to_gnome();
        said.join("\n") + "\n"
    }
}

/// A text no longer than this, its end given up if it must be.
fn cut(text: &str, length: usize) -> String {
    if text.chars().count() <= length {
        return text.to_string();
    }
    let kept: String = text.chars().take(length.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

fn ago(elapsed: Duration) -> String {
    match elapsed.as_secs() {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

fn may_write(path: &str) -> bool {
    let path = CString::new(path).unwrap();
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// Something typed on, and not also a mouse or a touchpad: what such a
/// device points with would be lost on the way through.
fn is_keyboard(device: &Device) -> bool {
    let Some(keys) = device.supported_keys() else { return false };
    let typed_on = [KeyCode::KEY_A, KeyCode::KEY_Z, KeyCode::KEY_SPACE, KeyCode::KEY_ENTER]
        .iter()
        .all(|key| keys.contains(*key));
    let points = device
        .supported_relative_axes()
        .is_some_and(|axes| axes.contains(RelativeAxisCode::REL_X))
        || device
            .supported_absolute_axes()
            .is_some_and(|axes| axes.contains(AbsoluteAxisCode::ABS_X));
    typed_on && !points
}

/// Make a keyboard ours, but only between keys: taken with a key down,
/// the programs would never see it come up.
fn take(keyboard: &mut Keyboard) {
    let idle = |device: &Device| device.get_key_state().is_ok_and(|keys| keys.iter().next().is_none());
    // What is waiting to be read has reached the programs already.
    while keyboard.device.fetch_events().is_ok_and(|events| events.count() > 0) {}
    if !idle(&keyboard.device) || keyboard.device.grab().is_err() {
        return;
    }
    if !idle(&keyboard.device) {
        let _ = keyboard.device.ungrab();
        return;
    }
    keyboard.held = true;
    keyboard.down.clear();
}

/// The keyboard the programs see in place of the real ones. It has every
/// key but the buttons of mice and gamepads, which would make it one.
fn keyboard_of_our_own() -> Option<VirtualDevice> {
    let name = env::var("CLUTCH_NAME").unwrap_or(NAME.into());
    let keys: AttributeSet<KeyCode> = (1..0x100)
        .chain(0x160..0x220)
        .chain(0x230..0x2c0)
        .map(KeyCode::new)
        .collect();
    let built = VirtualDevice::builder()
        .and_then(|builder| builder.name(&name).with_keys(&keys))
        .and_then(|builder| builder.build());
    match built {
        Ok(device) => Some(device),
        Err(e) => {
            log(format!("{UINPUT}: {e}"));
            None
        }
    }
}

/// Run what a key is bound to, as the desktop would have.
fn launch(binding: &Binding, under_systemd: bool) {
    let keys = binding.keys.clone();
    let text = binding.command.clone();
    thread::spawn(move || {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(config::with_clutch(&text)).stdin(Stdio::null()).process_group(0);
        if let Some(home) = env::var_os("HOME") {
            command.current_dir(home);
        }
        if under_systemd {
            // Started before anyone logged in, clutch knows nothing of
            // the display. systemd is told of it when the session starts.
            command.envs(session_environment());
            for ours in ["NOTIFY_SOCKET", "WATCHDOG_USEC", "WATCHDOG_PID", "INVOCATION_ID"] {
                command.env_remove(ours);
            }
        }
        match command.status() {
            Ok(status) if status.success() => {}
            Ok(status) => log(format!("{keys}: {text}: {status}")),
            Err(e) => log(format!("{keys}: {text}: {e}")),
        }
    });
}

fn session_environment() -> Vec<(String, String)> {
    let Ok(output) = Command::new("systemctl").args(["--user", "show-environment"]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once('='))
        // A value with anything odd in it is written as $'...'; none of
        // the ones a program needs to find the display are.
        .filter(|(_, value)| !value.starts_with("$'"))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

/// Tell systemd we are up, and again each time round that we still are.
fn notify(state: &str) {
    let Some(path) = env::var_os("NOTIFY_SOCKET") else { return };
    let Ok(socket) = UnixDatagram::unbound() else { return };
    let _ = match path.as_bytes().strip_prefix(b"@") {
        Some(name) => SocketAddr::from_abstract_name(name)
            .and_then(|address| socket.send_to_addr(state.as_bytes(), &address)),
        None => socket.send_to(state.as_bytes(), Path::new(&path)),
    };
}

/// Told when a keyboard is plugged in or the settings file is saved.
struct Watch {
    fd: RawFd,
}

impl Watch {
    fn new(dirs: &[(&Path, u32)]) -> Watch {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        for (dir, events) in dirs {
            let dir = CString::new(dir.as_os_str().as_bytes()).unwrap();
            unsafe { libc::inotify_add_watch(fd, dir.as_ptr(), *events) };
        }
        Watch { fd }
    }

    fn drain(&self) {
        let mut buffer = [0u8; 4096];
        while unsafe { libc::read(self.fd, buffer.as_mut_ptr().cast(), buffer.len()) } > 0 {}
    }
}
