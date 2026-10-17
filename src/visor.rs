//! visor: a terminal that comes in from an edge of the screen at a key,
//! and goes again at the same key. It is a Ghostty window of a class of
//! its own, running screen, which clutch puts where it is asked and
//! hides. On X11 clutch asks the window manager; on Wayland only GNOME's
//! shell may move a window, so there the extension does it.

use crate::config;
use crate::desk::X11;
use crate::extension;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

pub const HELP: &str = "  clutch visor                  bring the terminal in, or send it away
  clutch visor show | hide
  clutch visor top | bottom | left | right | center
                                move it to that edge of the screen
  clutch visor bigger | smaller
  clutch visor grow             its next size, and round again after the last
  clutch visor full             full screen, and back
  clutch visor [shape] NAME     top-centered, left-half, right-half,
                                right-third, corner, floating, peek, and
                                with `shape`: top, bottom, full
  clutch visor reset            back to the shape in the settings
  clutch visor status           where it is, in a line
";

/// The class the terminal is given, which is how its window is known.
pub const CLASS: &str = "io.github.jeffjose.visor";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    /// In the middle, from no edge.
    Center,
}

const EDGES: [(&str, Edge); 5] =
    [("top", Edge::Top), ("bottom", Edge::Bottom), ("left", Edge::Left), ("right", Edge::Right), ("center", Edge::Center)];

impl Edge {
    fn parse(word: &str) -> Option<Edge> {
        EDGES.iter().find(|(name, _)| *name == word).map(|(_, edge)| *edge)
    }

    fn name(self) -> &'static str {
        EDGES.iter().find(|(_, edge)| *edge == self).map_or("top", |(name, _)| name)
    }

    /// Whether it comes in across the screen, so that how far it comes
    /// in is its width.
    fn across(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

/// Which end of its edge it keeps to, when it does not cover all of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Start,
    Middle,
    End,
}

const ALIGNS: [(&str, Align); 5] =
    [("start", Align::Start), ("center", Align::Middle), ("end", Align::End), ("left", Align::Start), ("right", Align::End)];

impl Align {
    fn parse(word: &str) -> Option<Align> {
        match word {
            "top" => Some(Align::Start),
            "bottom" => Some(Align::End),
            "middle" => Some(Align::Middle),
            _ => ALIGNS.iter().find(|(name, _)| *name == word).map(|(_, align)| *align),
        }
    }

    fn name(self) -> &'static str {
        ALIGNS.iter().find(|(_, align)| *align == self).map_or("center", |(name, _)| name)
    }
}

/// Where the terminal is: the edge it comes in from, how far it comes
/// in and how much of the edge it covers, in percent of the screen, and
/// which end of the edge it keeps to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shape {
    pub edge: Edge,
    pub depth: u32,
    pub span: u32,
    pub align: Align,
}

const fn shape(edge: Edge, depth: u32, span: u32, align: Align) -> Shape {
    Shape { edge, depth, span, align }
}

const FULL: Shape = shape(Edge::Top, 100, 100, Align::Middle);

/// The shapes that have names: what is written, and what it is called.
pub const SHAPES: &[(&str, &str, Shape)] = &[
    ("top", "Top", shape(Edge::Top, 40, 100, Align::Middle)),
    ("top-centered", "Top, centered", shape(Edge::Top, 40, 60, Align::Middle)),
    ("bottom", "Bottom", shape(Edge::Bottom, 40, 100, Align::Middle)),
    ("left-half", "Left half", shape(Edge::Left, 50, 100, Align::Middle)),
    ("right-half", "Right half", shape(Edge::Right, 50, 100, Align::Middle)),
    ("right-third", "Right third", shape(Edge::Right, 34, 100, Align::Middle)),
    ("corner", "Top right corner", shape(Edge::Top, 50, 50, Align::End)),
    ("floating", "Floating, in the middle", shape(Edge::Center, 60, 64, Align::Middle)),
    ("full", "Full screen", FULL),
    ("peek", "Peek strip", shape(Edge::Top, 12, 100, Align::Middle)),
];

pub fn named(name: &str) -> Option<Shape> {
    SHAPES.iter().find(|(word, _, _)| *word == name).map(|(_, _, shape)| *shape)
}

/// The letter that picks a shape in the box, with Alt.
pub fn letter_of(name: &str) -> Option<char> {
    const LETTERS: [(&str, char); 10] = [
        ("top", 't'),
        ("top-centered", 'c'),
        ("bottom", 'b'),
        ("left-half", 'l'),
        ("right-half", 'r'),
        ("right-third", 'i'),
        ("corner", 'k'),
        ("floating", 'f'),
        ("full", 'u'),
        ("peek", 'p'),
    ];
    LETTERS.iter().find(|(word, _)| *word == name).map(|(_, letter)| *letter)
}

/// A drawing of a shape: a screen, `size` across, with the part the
/// terminal takes lit.
pub fn picture(shape: Shape, size: usize) -> crate::palette::canvas::Icon {
    const CLEAR: u32 = 0x0000_0000;
    const BEZEL: u32 = 0xFF1B_2029;
    const SCREEN: u32 = 0xFF3A_4554;
    const LIT: u32 = 0xFF3C_C4B7;
    let size = size.max(8);
    let mut pixels = vec![CLEAR; size * size];
    // A screen sixteen by ten, in the middle of the square.
    let (width, height) = (size, size * 10 / 16);
    let top = (size - height) / 2;
    let border = (size / 24).max(1);
    let (inner_w, inner_h) = (width - 2 * border, height - 2 * border);
    let [left, up, across, down] = shape.rect();
    let lit = |x: usize, y: usize| {
        let (fx, fy) = ((x as f64 + 0.5) / inner_w as f64, (y as f64 + 0.5) / inner_h as f64);
        fx >= left && fx < left + across && fy >= up && fy < up + down
    };
    for y in 0..height {
        for x in 0..width {
            let inside = x >= border && x < width - border && y >= border && y < height - border;
            let color = match inside {
                false => BEZEL,
                true if lit(x - border, y - border) => LIT,
                true => SCREEN,
            };
            pixels[(top + y) * size + x] = color;
        }
    }
    crate::palette::canvas::Icon { width: size, height: size, pixels }
}

impl Shape {
    /// Its width and its height, in percent.
    fn width_height(self) -> (u32, u32) {
        match self.edge.across() {
            true => (self.depth, self.span),
            false => (self.span, self.depth),
        }
    }

    fn with_width_height(self, width: u32, height: u32) -> Shape {
        match self.edge.across() {
            true => Shape { depth: width, span: height, ..self },
            false => Shape { depth: height, span: width, ..self },
        }
    }

    /// Where it is, in parts of the screen: left, top, width, height.
    pub fn rect(self) -> [f64; 4] {
        let (depth, span) = (f64::from(self.depth) / 100.0, f64::from(self.span) / 100.0);
        let along = match self.align {
            Align::Start => 0.0,
            Align::Middle => (1.0 - span) / 2.0,
            Align::End => 1.0 - span,
        };
        match self.edge {
            Edge::Top => [along, 0.0, span, depth],
            Edge::Bottom => [along, 1.0 - depth, span, depth],
            Edge::Left => [0.0, along, depth, span],
            Edge::Right => [1.0 - depth, along, depth, span],
            Edge::Center => [(1.0 - span) / 2.0, (1.0 - depth) / 2.0, span, depth],
        }
    }

    fn said(self) -> String {
        if self.depth == 100 && self.span == 100 {
            return "full screen".into();
        }
        let (width, height) = self.width_height();
        let at = match (self.edge.across(), self.align) {
            _ if self.edge == Edge::Center || self.span == 100 => "",
            (_, Align::Middle) => "",
            (false, Align::Start) => ", at the left",
            (false, Align::End) => ", at the right",
            (true, Align::Start) => ", at the top",
            (true, Align::End) => ", at the bottom",
        };
        format!("{}, {width}% wide and {height}% high{at}", self.edge.name())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Monitor {
    /// The one the pointer is on.
    InUse,
    Main,
}

/// The `[visor]` part of the settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// What starts the terminal, when there is none.
    pub run: String,
    pub class: String,
    pub shape: Shape,
    /// Pixels between the terminal and the edges of the screen.
    pub gap: u32,
    /// The sizes `grow` goes through, in percent of how far it comes in.
    pub grow: Vec<u32>,
    pub monitor: Monitor,
    pub slide: bool,
    pub hide_on_click_away: bool,
    /// The keys that move it while it is in, and what each does: the
    /// keys as written, and the word after `clutch visor`.
    pub keys: Vec<(String, String)>,
}

/// The keys it has while it is in, unless the settings say others.
const KEYS: [(&str, &str); 10] = [
    ("super+left", "left"),
    ("super+right", "right"),
    ("super+up", "top"),
    ("super+down", "bottom"),
    ("super+c", "center"),
    ("super+equal", "bigger"),
    ("super+minus", "smaller"),
    ("super+g", "grow"),
    ("super+f", "full"),
    ("super+escape", "hide"),
];

/// The words a key may say, after `clutch visor`.
fn is_word(word: &str) -> bool {
    let plain = ["toggle", "show", "hide", "bigger", "smaller", "grow", "full", "reset"];
    plain.contains(&word) || Edge::parse(word).is_some() || named(word).is_some()
}

fn run_for(class: &str) -> String {
    format!("ghostty --class={class} --gtk-single-instance=false --window-decoration=false -e screen -dRR")
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            run: run_for(CLASS),
            class: CLASS.into(),
            shape: SHAPES[0].2,
            gap: 0,
            grow: Vec::new(),
            monitor: Monitor::InUse,
            slide: true,
            hide_on_click_away: false,
            keys: KEYS.iter().map(|(keys, word)| (keys.to_string(), word.to_string())).collect(),
        }
    }
}

/// A size in percent: "40%", or 40.
fn percent(key: &str, value: &toml::Value) -> Result<u32, String> {
    let number = match value {
        toml::Value::Integer(number) => Some(*number),
        toml::Value::String(text) => text.trim().trim_end_matches('%').trim().parse().ok(),
        _ => None,
    };
    match number {
        Some(number @ 5..=100) => Ok(number as u32),
        _ => Err(format!("visor.{key}: a size from 5% to 100%, as \"40%\"")),
    }
}

impl Settings {
    pub fn from_table(table: &toml::Table) -> Result<Settings, String> {
        let mut settings = Settings::default();
        let text = |key: &str| -> Result<Option<String>, String> {
            match table.get(key) {
                None => Ok(None),
                Some(toml::Value::String(text)) => Ok(Some(text.clone())),
                Some(_) => Err(format!("visor.{key}: it must be in quotes")),
            }
        };
        let yes = |key: &str, otherwise: bool| -> Result<bool, String> {
            match table.get(key) {
                None => Ok(otherwise),
                Some(toml::Value::Boolean(yes)) => Ok(*yes),
                Some(_) => Err(format!("visor.{key}: true or false")),
            }
        };
        const KNOWN: [&str; 13] =
            ["run", "class", "shape", "edge", "width", "height", "align", "gap", "grow", "monitor", "slide", "hide_on_click_away", "keys"];
        if let Some(key) = table.keys().find(|key| !KNOWN.contains(&key.as_str())) {
            return Err(format!("visor.{key}: there is no such setting (it has {})", KNOWN.join(", ")));
        }

        if let Some(class) = text("class")? {
            let fits = class.contains('.') && class.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
            if !fits {
                return Err("visor.class: a name such as \"io.github.jeffjose.visor\", in letters, digits, dots and dashes".into());
            }
            settings.run = run_for(&class);
            settings.class = class;
        }
        if let Some(run) = text("run")? {
            settings.run = run;
        }
        if let Some(name) = text("shape")? {
            let names: Vec<&str> = SHAPES.iter().map(|(name, _, _)| *name).collect();
            settings.shape = named(&name).ok_or(format!("visor.shape: one of {}", names.join(", ")))?;
        }
        if let Some(edge) = text("edge")? {
            settings.shape.edge = Edge::parse(&edge).ok_or("visor.edge: top, bottom, left, right or center")?;
        }
        let (mut width, mut height) = settings.shape.width_height();
        if let Some(value) = table.get("width") {
            width = percent("width", value)?;
        }
        if let Some(value) = table.get("height") {
            height = percent("height", value)?;
        }
        settings.shape = settings.shape.with_width_height(width, height);
        if let Some(align) = text("align")? {
            settings.shape.align = Align::parse(&align).ok_or("visor.align: start, center or end (or left, right, top, bottom)")?;
        }
        match table.get("gap") {
            None => {}
            Some(toml::Value::Integer(gap @ 0..=200)) => settings.gap = *gap as u32,
            Some(_) => return Err("visor.gap: pixels, from 0 to 200".into()),
        }
        match table.get("grow") {
            None => {}
            Some(toml::Value::Array(sizes)) if !sizes.is_empty() => {
                settings.grow = sizes.iter().map(|size| percent("grow", size)).collect::<Result<_, _>>()?;
            }
            Some(_) => return Err("visor.grow: a list of sizes, as [\"40%\", \"70%\", \"100%\"]".into()),
        }
        if let Some(monitor) = text("monitor")? {
            settings.monitor = match monitor.as_str() {
                "in use" | "in-use" => Monitor::InUse,
                "main" => Monitor::Main,
                _ => return Err("visor.monitor: \"in use\" or \"main\"".into()),
            };
        }
        settings.slide = yes("slide", true)?;
        settings.hide_on_click_away = yes("hide_on_click_away", false)?;
        // Keys of its own take the place of all those it has otherwise.
        match table.get("keys") {
            None => {}
            Some(toml::Value::Table(keys)) => {
                settings.keys = Vec::new();
                for (keys, word) in keys {
                    let word = word.as_str().ok_or(format!("visor.keys.\"{keys}\": what it does must be in quotes"))?;
                    if !is_word(word) {
                        return Err(format!("visor.keys.\"{keys}\": \"{word}\" is not something visor does (clutch visor help)"));
                    }
                    match crate::keys::parse_sequence(keys) {
                        Ok(sequence) if sequence.len() == 1 => {}
                        Ok(_) => return Err(format!("visor.keys.\"{keys}\": one chord, not keys one after another")),
                        Err(e) => return Err(format!("visor.keys.\"{keys}\": {e}")),
                    }
                    settings.keys.push((keys.clone(), word.to_string()));
                }
            }
            Some(_) => return Err("visor.keys: a table of keys, as [visor.keys] \"super+left\" = \"left\"".into()),
        }
        Ok(settings)
    }

    /// The sizes `grow` goes through: those in the settings, or else
    /// the one set, then halfway to full, then full.
    fn sizes(&self) -> Vec<u32> {
        if !self.grow.is_empty() {
            return self.grow.clone();
        }
        let depth = self.shape.depth;
        let mut sizes = vec![depth, (depth + 100) / 2, 100];
        sizes.dedup();
        sizes
    }
}

/// Where the terminal has been moved to since the settings said, kept
/// from one key to the next. Saving the settings starts it over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Now {
    shape: Shape,
    /// Which of the sizes `grow` is at.
    step: usize,
    full: bool,
}

fn state_file() -> PathBuf {
    config::state_home().join("clutch/visor")
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

impl Now {
    fn of(settings: &Settings) -> Now {
        Now { shape: settings.shape, step: 0, full: false }
    }

    fn written(self) -> String {
        let s = self.shape;
        format!("{} {} {} {} {} {}\n", s.edge.name(), s.depth, s.span, s.align.name(), self.step, u8::from(self.full))
    }

    fn read(text: &str) -> Option<Now> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let [edge, depth, span, align, step, full] = words.as_slice() else { return None };
        let size = |word: &str| word.parse().ok().filter(|size| (5..=100).contains(size));
        let shape = Shape { edge: Edge::parse(edge)?, depth: size(depth)?, span: size(span)?, align: Align::parse(align)? };
        Some(Now { shape, step: step.parse().ok()?, full: *full == "1" })
    }

    /// As it was left, unless the settings were saved since.
    fn load(settings: &Settings) -> Now {
        let file = state_file();
        let newer = matches!((modified(&config::path()), modified(&file)), (Some(settings), Some(kept)) if settings > kept);
        let kept = (!newer).then(|| fs::read_to_string(&file).ok()).flatten().and_then(|text| Now::read(&text));
        kept.unwrap_or(Now::of(settings))
    }

    fn save(self) {
        let file = state_file();
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(file, self.written());
    }

    fn shape(self) -> Shape {
        if self.full {
            FULL
        } else {
            self.shape
        }
    }
}

/// What a word on the command line does to where the terminal is, or
/// `None` for a word that is not one of those.
fn moved(now: Now, word: &str, name: Option<&str>, settings: &Settings) -> Option<Result<Now, String>> {
    let mut next = now;
    match word {
        "bigger" | "smaller" => {
            let by: i32 = if word == "bigger" { 10 } else { -10 };
            next.shape.depth = (now.shape.depth as i32 + by).clamp(10, 100) as u32;
            next.full = false;
        }
        "grow" => {
            let sizes = settings.sizes();
            next.step = (now.step + 1) % sizes.len();
            next.shape.depth = sizes[next.step];
            next.full = false;
        }
        "full" => next.full = !now.full,
        "reset" => next = Now::of(settings),
        "shape" => {
            let names: Vec<&str> = SHAPES.iter().map(|(name, _, _)| *name).collect();
            let Some(shape) = name.and_then(named) else {
                return Some(Err(format!("visor shape: one of {}", names.join(", "))));
            };
            next = Now { shape, step: 0, full: false };
        }
        // An edge keeps the size; a shape's name, which may be said
        // without `shape`, is a size too.
        _ => match (Edge::parse(word), named(word)) {
            (Some(edge), _) => {
                next.shape.edge = edge;
                next.full = false;
            }
            (None, Some(shape)) => next = Now { shape, step: 0, full: false },
            (None, None) => return None,
        },
    }
    Some(Ok(next))
}

pub fn run(arguments: Vec<String>) -> Result<(), String> {
    let path = config::path();
    let settings = match fs::read_to_string(&path) {
        Ok(text) => config::Config::parse(&text).map_err(|e| format!("{}: {e}", config::short(&path)))?.0.visor,
        Err(_) => Settings::default(),
    };
    let word = arguments.first().map_or("toggle", String::as_str);
    let screen = Screen::open()?;
    let now = Now::load(&settings);
    match word {
        "toggle" => screen.toggle(&settings, now.shape()),
        "show" => screen.show(&settings, now.shape()),
        "hide" => screen.hide(&settings, now.shape()),
        "status" => {
            println!("visor: {}", status_line(&settings));
            Ok(())
        }
        "help" | "-h" | "--help" => {
            print!("{HELP}");
            Ok(())
        }
        _ => {
            let next = moved(now, word, arguments.get(1).map(String::as_str), &settings)
                .ok_or(format!("visor: no command {word} (clutch visor help)"))??;
            next.save();
            screen.show(&settings, next.shape())
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Absent,
    Shown,
    Hidden,
}

enum Screen {
    X11(X11),
    Gnome,
}

impl Screen {
    fn open() -> Result<Screen, String> {
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            return Ok(Screen::Gnome);
        }
        X11::open().map(Screen::X11).ok_or("visor: there is no screen to show it on".into())
    }

    fn state(&self, settings: &Settings) -> Result<(State, &'static str), String> {
        match self {
            Screen::X11(x) => Ok((x11_state(x, settings), "X11")),
            Screen::Gnome => {
                let state = match gnome(settings, "state", FULL)?.as_str() {
                    "absent" => State::Absent,
                    "hidden" => State::Hidden,
                    _ => State::Shown,
                };
                Ok((state, "GNOME"))
            }
        }
    }

    /// Bring it in, or start it; and if it is in and has the keyboard,
    /// send it away. In and without the keyboard, it is given it.
    fn toggle(&self, settings: &Settings, shape: Shape) -> Result<(), String> {
        match self {
            Screen::Gnome => match gnome(settings, "toggle", shape)?.as_str() {
                "absent" => gnome_start(settings, shape),
                _ => Ok(()),
            },
            Screen::X11(x) => match x.find_class(&settings.class) {
                Some(window) if !x.hidden(window) && x.active() == Some(window) => Ok(x11_hide(x, window, settings, shape.edge)),
                Some(window) => Ok(x11_show(x, window, settings, shape)),
                None => x11_start(x, settings, shape),
            },
        }
    }

    fn show(&self, settings: &Settings, shape: Shape) -> Result<(), String> {
        match self {
            Screen::Gnome => match gnome(settings, "show", shape)?.as_str() {
                "absent" => gnome_start(settings, shape),
                _ => Ok(()),
            },
            Screen::X11(x) => match x.find_class(&settings.class) {
                Some(window) => Ok(x11_show(x, window, settings, shape)),
                None => x11_start(x, settings, shape),
            },
        }
    }

    fn hide(&self, settings: &Settings, shape: Shape) -> Result<(), String> {
        match self {
            Screen::Gnome => gnome(settings, "hide", shape).map(drop),
            Screen::X11(x) => {
                if let Some(window) = x.find_class(&settings.class).filter(|window| !x.hidden(*window)) {
                    x11_hide(x, window, settings, shape.edge);
                }
                Ok(())
            }
        }
    }
}

/// Start the terminal. Where it is to go is put on it as it comes up.
fn start(settings: &Settings) -> Result<(), String> {
    crate::palette::spawn(&settings.run);
    Ok(())
}

/// There is none: GNOME's shell starts it, where it can, so that it can
/// leave it out of Alt+Tab and the overview; clutch, where it cannot.
fn gnome_start(settings: &Settings, shape: Shape) -> Result<(), String> {
    match gnome(settings, "start", shape)?.as_str() {
        "starting" => Ok(()),
        _ => start(settings),
    }
}

/// For the running clutch, which takes visor's keys only while the
/// terminal is in: how to tell, as one of them is pressed.
pub fn asker(settings: Settings) -> Box<dyn FnMut() -> bool + Send> {
    Box::new(move || Screen::open().and_then(|screen| screen.state(&settings)).is_ok_and(|(state, _)| state == State::Shown))
}

/// For the running clutch on X11: send the terminal away when another
/// window takes the keyboard, where the settings say to. They are read
/// each time, so that a change to them counts at once.
pub fn watch(settings: std::sync::Arc<std::sync::Mutex<Settings>>) {
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        return;
    }
    std::thread::spawn(move || loop {
        if let Some(x) = X11::open() {
            x.each_active(|x, active| {
                let Ok(settings) = settings.lock().map(|settings| settings.clone()) else { return };
                let (Some(active), true) = (active, settings.hide_on_click_away) else { return };
                let Some(window) = x.find_class(&settings.class) else { return };
                // Its own dialogs are of its class, and the box is clutch's.
                let class = x.class_of(active);
                if active == window || x.hidden(window) || class == settings.class || class == "clutch" {
                    return;
                }
                x11_hide(x, window, &settings, Now::load(&settings).shape().edge);
            });
        }
        // The display went, or was not there yet.
        std::thread::sleep(Duration::from_secs(5));
    });
}

/// Where it is, in a line, for `clutch status`.
pub fn status_line(settings: &Settings) -> String {
    let shape = Now::load(settings).shape();
    match Screen::open().and_then(|screen| screen.state(settings)) {
        Ok((State::Absent, _)) => "not running; `clutch visor` starts it".into(),
        Ok((State::Shown, on)) => format!("shown, {} ({on})", shape.said()),
        Ok((State::Hidden, on)) => format!("hidden, {} ({on})", shape.said()),
        Err(why) => why,
    }
}

/// What is asked of GNOME's shell, as the extension reads it. With it
/// go the keys the shell is to take while the terminal is in, and how
/// to start the terminal, which the shell does itself where it can so
/// as to leave it out of Alt+Tab.
fn request(settings: &Settings, what: &str, shape: Shape, keys: bool) -> String {
    let round = |part: f64| (part * 10_000.0).round() / 10_000.0;
    let keys: Vec<[String; 2]> = match keys {
        true => settings.keys.iter().filter_map(|(keys, word)| {
            let chord = *crate::keys::parse_sequence(keys).ok()?.first()?;
            Some([crate::gnome::accelerator(chord)?, word.clone()])
        }).collect(),
        false => Vec::new(),
    };
    let clutch = env::current_exe().map(|exe| exe.to_string_lossy().into_owned()).unwrap_or_else(|_| "clutch".into());
    // Your shell's PATH, which is slow to ask, only to start it.
    let path = match what {
        "start" => crate::palette::login_env().iter().find(|(name, _)| name == "PATH").map(|(_, path)| path.clone()),
        _ => None,
    };
    let mut asked = serde_json::json!({
        "do": what,
        "class": settings.class,
        "rect": shape.rect().map(round),
        "edge": shape.edge.name(),
        "gap": settings.gap,
        "main": settings.monitor == Monitor::Main,
        "slide": settings.slide,
        "blur": settings.hide_on_click_away,
        "keys": keys,
        "clutch": clutch,
    });
    if what == "start" {
        asked["run"] = serde_json::json!(["/bin/sh", "-c", format!("exec {}", settings.run)]);
        asked["path"] = serde_json::json!(path.or_else(|| env::var("PATH").ok()));
    }
    // It goes to gdbus in single quotes, so it may have none of its own.
    asked.to_string().replace('\'', "\\u0027")
}

/// Whether GNOME's shell is to take visor's keys: not where clutch
/// holds the keyboards, which takes them before the shell sees them.
fn shell_takes_keys() -> bool {
    !std::path::Path::new(crate::setup::RULE_PATH).exists()
}

/// Ask the extension, which answers with where the terminal now is:
/// shown, hidden, or absent when there is none.
fn gnome(settings: &Settings, what: &str, shape: Shape) -> Result<String, String> {
    let asked = request(settings, what, shape, shell_takes_keys());
    if let Some(answer) = extension::call("Visor", &[format!("'{asked}'")]).and_then(|printed| extension::unwrap(&printed)) {
        return Ok(answer);
    }
    Err(match extension::health() {
        Err(why) => format!("visor needs clutch's GNOME extension, which is {why}"),
        Ok(()) => "visor: the GNOME extension did not answer".into(),
    })
}

fn x11_state(x: &X11, settings: &Settings) -> State {
    match x.find_class(&settings.class) {
        None => State::Absent,
        Some(window) if x.hidden(window) => State::Hidden,
        Some(_) => State::Shown,
    }
}

fn x11_show(x: &X11, window: u32, settings: &Settings, shape: Shape) {
    let Some(area) = x.work_area(settings.monitor == Monitor::Main) else { return };
    let place = pixels(shape.rect(), area, settings.gap as i32);
    x.keep_up(window);
    // Coming back from away, it comes in from off its edge.
    let from = match settings.slide && x.hidden(window) {
        true => off_edge(place, shape.edge, area),
        false => None,
    };
    let (left, top, width, height) = place;
    let start = from.map_or(place, |(x, y)| (x, y, width, height));
    x.put(window, start);
    x.activate(window);
    // Moves asked of a window still coming back from being minimized
    // are lost, so it slides once it is back.
    let began = Instant::now();
    while x.hidden(window) && began.elapsed() < Duration::from_millis(300) {
        std::thread::sleep(Duration::from_millis(5));
    }
    // A window brought back from being minimized may be put back where
    // it was before it is moved.
    x.put(window, start);
    if let Some(from) = from {
        slide(x, window, from, (left, top));
    }
    x.put(window, place);
}

/// Send it away: out past its edge, where it slides, and then minimized.
fn x11_hide(x: &X11, window: u32, settings: &Settings, edge: Edge) {
    let area = x.work_area(settings.monitor == Monitor::Main);
    let frame = x.frame_area(window);
    if let (true, Some(area), Some(frame)) = (settings.slide, area, frame) {
        if let Some(to) = off_edge(frame, edge, area) {
            slide(x, window, (frame.0, frame.1), to);
        }
    }
    x.minimize(window);
}

/// Where a window of this place is just past an edge of the area, to
/// slide in from or out to; none from the middle, which is no edge.
fn off_edge((left, top, width, height): (i32, i32, i32, i32), edge: Edge, (x, y, w, h): (i32, i32, i32, i32)) -> Option<(i32, i32)> {
    match edge {
        Edge::Top => Some((left, y - height)),
        Edge::Bottom => Some((left, y + h)),
        Edge::Left => Some((x - width, top)),
        Edge::Right => Some((x + w, top)),
        Edge::Center => None,
    }
}

/// Move a window from one corner to the other in steps, quick at first
/// and slowing to a stop, in about a sixth of a second.
fn slide(x: &X11, window: u32, (from_x, from_y): (i32, i32), (to_x, to_y): (i32, i32)) {
    const STEPS: u32 = 10;
    for step in 1..=STEPS {
        let done = f64::from(step) / f64::from(STEPS);
        let eased = 1.0 - (1.0 - done).powi(3);
        let along = |from: i32, to: i32| from + (f64::from(to - from) * eased).round() as i32;
        x.move_to(window, along(from_x, to_x), along(from_y, to_y));
        x.sync();
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Start the terminal and wait for its window, to put it in place.
fn x11_start(x: &X11, settings: &Settings, shape: Shape) -> Result<(), String> {
    start(settings)?;
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(10) {
        if let Some(window) = x.find_class(&settings.class) {
            x11_show(x, window, settings, shape);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!("visor: no window of class {} came up from: {}", settings.class, settings.run))
}

/// A rectangle in parts of a screen, as pixels of an area of it, kept
/// `gap` pixels in from each side.
fn pixels([left, top, width, height]: [f64; 4], (x, y, w, h): (i32, i32, i32, i32), gap: i32) -> (i32, i32, i32, i32) {
    let along = |part: f64, size: i32| (part * f64::from(size)).round() as i32;
    (
        x + along(left, w) + gap,
        y + along(top, h) + gap,
        (along(width, w) - 2 * gap).max(1),
        (along(height, h) - 2 * gap).max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn shapes_cover_their_part_of_the_screen() {
        assert_eq!(named("top").unwrap().rect(), [0.0, 0.0, 1.0, 0.4]);
        assert_eq!(named("bottom").unwrap().rect(), [0.0, 0.6, 1.0, 0.4]);
        assert_eq!(named("left-half").unwrap().rect(), [0.0, 0.0, 0.5, 1.0]);
        assert_eq!(named("right-half").unwrap().rect(), [0.5, 0.0, 0.5, 1.0]);
        assert_eq!(named("corner").unwrap().rect(), [0.5, 0.0, 0.5, 0.5]);
        let [left, top, width, height] = named("top-centered").unwrap().rect();
        assert!((left - 0.2).abs() < 1e-9 && top == 0.0 && (width - 0.6).abs() < 1e-9 && (height - 0.4).abs() < 1e-9);
        let [left, top, ..] = named("floating").unwrap().rect();
        assert!((left - 0.18).abs() < 1e-9 && (top - 0.2).abs() < 1e-9);
    }

    #[test]
    fn as_pixels_with_a_gap() {
        // A 1920 by 1080 screen with a 30 pixel panel at the top.
        let area = (0, 30, 1920, 1050);
        assert_eq!(pixels(named("top").unwrap().rect(), area, 0), (0, 30, 1920, 420));
        assert_eq!(pixels(named("right-half").unwrap().rect(), area, 8), (968, 38, 944, 1034));
        // On a second monitor, to the right.
        assert_eq!(pixels(named("left-half").unwrap().rect(), (1920, 0, 2560, 1440), 0), (1920, 0, 1280, 1440));
    }

    #[test]
    fn settings_say_width_and_height_whichever_the_edge() {
        let settings = Settings::from_table(&table("edge = \"left\"\nwidth = \"30%\"\n")).unwrap();
        assert_eq!(settings.shape, shape(Edge::Left, 30, 100, Align::Middle));
        let settings = Settings::from_table(&table("shape = \"top-centered\"\nheight = 50\nalign = \"right\"\n")).unwrap();
        assert_eq!(settings.shape, shape(Edge::Top, 50, 60, Align::End));
        // From the top to the left, it comes in as far as before.
        let settings = Settings::from_table(&table("edge = \"left\"\n")).unwrap();
        assert_eq!(settings.shape.width_height(), (40, 100));
    }

    #[test]
    fn settings_by_default_and_wrong() {
        let settings = Settings::from_table(&table("")).unwrap();
        assert_eq!(settings, Settings::default());
        assert!(settings.run.contains("--class=io.github.jeffjose.visor") && settings.run.ends_with("-e screen -dRR"));
        let renamed = Settings::from_table(&table("class = \"org.example.drop\"\n")).unwrap();
        assert!(renamed.run.contains("--class=org.example.drop"));
        let wrong = |text: &str| Settings::from_table(&table(text)).unwrap_err();
        assert!(wrong("hieght = 40\n").contains("hieght"));
        assert!(wrong("height = \"140%\"\n").contains("height"));
        assert!(wrong("edge = \"up\"\n").contains("edge"));
        assert!(wrong("class = \"it's\"\n").contains("class"));
        assert!(wrong("slide = \"yes\"\n").contains("slide"));
        assert!(wrong("grow = []\n").contains("grow"));
    }

    #[test]
    fn grows_through_its_sizes_and_round_again() {
        let settings = Settings::default();
        assert_eq!(settings.sizes(), [40, 70, 100]);
        let mut now = Now::of(&settings);
        let mut depths = Vec::new();
        for _ in 0..4 {
            now = moved(now, "grow", None, &settings).unwrap().unwrap();
            depths.push(now.shape.depth);
        }
        assert_eq!(depths, [70, 100, 40, 70]);
        let set = Settings::from_table(&table("grow = [\"30%\", 90]\n")).unwrap();
        assert_eq!(set.sizes(), [30, 90]);
    }

    #[test]
    fn moved_about() {
        let settings = Settings::default();
        let now = Now::of(&settings);
        let left = moved(now, "left", None, &settings).unwrap().unwrap();
        assert_eq!(left.shape, shape(Edge::Left, 40, 100, Align::Middle));
        let full = moved(left, "full", None, &settings).unwrap().unwrap();
        assert_eq!(full.shape(), FULL);
        assert_eq!(moved(full, "full", None, &settings).unwrap().unwrap().shape(), left.shape);
        assert_eq!(moved(now, "smaller", None, &settings).unwrap().unwrap().shape.depth, 30);
        assert_eq!(moved(full, "bigger", None, &settings).unwrap().unwrap().full, false);
        let third = moved(now, "shape", Some("right-third"), &settings).unwrap().unwrap();
        assert_eq!(third.shape, named("right-third").unwrap());
        assert!(moved(now, "shape", Some("nope"), &settings).unwrap().is_err());
        assert_eq!(moved(now, "right-third", None, &settings).unwrap().unwrap(), third);
        assert!(moved(now, "sideways", None, &settings).is_none());
    }

    #[test]
    fn kept_as_written() {
        let now = Now { shape: named("corner").unwrap(), step: 2, full: true };
        assert_eq!(now.written(), "top 50 50 end 2 1\n");
        assert_eq!(Now::read(&now.written()), Some(now));
        assert_eq!(Now::read("top 50"), None);
        assert_eq!(Now::read("top 500 50 end 0 0"), None);
    }

    #[test]
    fn what_the_shell_is_asked() {
        let asked = request(&Settings::default(), "toggle", named("right-third").unwrap(), true);
        // Sent as a string in single quotes, it has none of its own.
        assert!(!asked.contains('\''));
        let asked: serde_json::Value = serde_json::from_str(&asked).unwrap();
        assert_eq!(asked["do"], "toggle");
        assert_eq!(asked["class"], "io.github.jeffjose.visor");
        assert_eq!(asked["rect"], serde_json::json!([0.66, 0.0, 0.34, 1.0]));
        assert_eq!((&asked["edge"], &asked["gap"], &asked["slide"], &asked["blur"]), (&"right".into(), &0.into(), &true.into(), &false.into()));
        // The keys, as GNOME writes them, and what each says to do.
        assert_eq!(asked["keys"][0], serde_json::json!(["<Super>Left", "left"]));
        assert_eq!(asked["keys"][5], serde_json::json!(["<Super>equal", "bigger"]));
        assert!(asked.get("run").is_none());
        let start: serde_json::Value = serde_json::from_str(&request(&Settings::default(), "start", FULL, false)).unwrap();
        assert_eq!(start["run"][2], "exec ghostty --class=io.github.jeffjose.visor --gtk-single-instance=false --window-decoration=false -e screen -dRR");
        assert_eq!(start["keys"], serde_json::json!([]));
    }

    #[test]
    fn said_in_a_line() {
        assert_eq!(named("top").unwrap().said(), "top, 100% wide and 40% high");
        assert_eq!(named("corner").unwrap().said(), "top, 50% wide and 50% high, at the right");
        assert_eq!(FULL.said(), "full screen");
    }
}
