//! What the right button brings up, and where a project's look is
//! chosen: the drawing, letters or picture on its tile, its color, what
//! the tile is filled with and the shape it is cut to.

use super::canvas::{duo, mix, Canvas, Text};
use super::controls::{self, Deed};
use super::looks::{self, Look, FILLS, GLYPHS, HUES, SHAPES};
use super::{colors, emoji, glyph, iconify, matching, projects, Action, Hit, Layout, Palette, View};
use super::{ACCENT, DIM_TEXT_COLOR, EDGE_COLOR, KEY_COLOR, ON_ACCENT, PANEL_COLOR, RULE_COLOR, TEXT_COLOR, WELL_COLOR};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};

/// The project a look is of.
#[derive(Clone, PartialEq)]
pub struct Target {
    pub name: String,
    pub folder: Option<PathBuf>,
}

impl Target {
    /// What its look is kept under: where it is, or failing that its name.
    pub fn key(&self) -> String {
        self.folder.as_ref().map_or(self.name.clone(), |folder| folder.to_string_lossy().into_owned())
    }
}

/// What a line of the menu does.
#[derive(Clone)]
pub enum Do {
    /// What Enter would have: the thing at this place among what was found.
    Go(usize),
    Act(Action),
    Open(View),
    Edit(Target),
    Shuffle(Target),
    Reset(Target),
    Rescan(Target),
    /// Take a copy out of the clipboard's history.
    Forget(PathBuf),
    /// Forget the color last picked from a palette, by its place among them.
    ForgetPick(usize),
}

pub enum Line {
    /// What it does, and the keys that do it without the menu, if any: "Ctrl+I".
    Entry { glyph: &'static str, label: String, act: Do, keys: &'static str },
    Rule,
    /// The name of the lines under it.
    Group(&'static str),
    /// The colors, to give the project one where it stands.
    Swatches(Target),
}

pub struct Menu {
    /// Where the button was pressed.
    at: (i32, i32),
    lines: Vec<Line>,
    picked: usize,
    /// Which color, while the keys are on those.
    swatch: usize,
    /// Whether it is the panel of actions, which Ctrl+K opens at the
    /// corner and which is typed into, rather than the menu at the pointer.
    panel: bool,
    typed: String,
}

impl Menu {
    /// The lines that are shown: all of them, or in the panel with
    /// something typed, those whose words have it, and none of the lines between.
    fn shown(&self) -> Vec<usize> {
        let typed = self.typed.trim().to_lowercase();
        if !self.panel || typed.is_empty() {
            return (0..self.lines.len()).collect();
        }
        let has = |words: &str| typed.split_whitespace().all(|word| words.to_lowercase().contains(word));
        (0..self.lines.len())
            .filter(|at| match &self.lines[*at] {
                Line::Entry { label, .. } => has(label),
                Line::Swatches(_) => has("color colour"),
                _ => false,
            })
            .collect()
    }

    fn can(&self, at: usize) -> bool {
        matches!(self.lines.get(at), Some(Line::Entry { .. } | Line::Swatches(_)))
    }

    /// Picked out: the first that can be, among those shown.
    fn settle(&mut self) {
        let shown = self.shown();
        if !shown.contains(&self.picked) || !self.can(self.picked) {
            self.picked = shown.into_iter().find(|at| self.can(*at)).unwrap_or(0);
        }
    }
}

/// Where a copied link would be found in it, if it is one.
fn link_in(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    ((text.starts_with("http://") || text.starts_with("https://")) && !text.contains(char::is_whitespace)).then(|| text.to_string())
}

/// The project's page on GitHub, from where git says it is kept.
fn github_of(folder: &std::path::Path) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(folder).args(["remote", "get-url", "origin"]).stderr(Stdio::null()).output().ok()?;
    let remote = String::from_utf8(output.stdout).ok()?;
    let remote = remote.trim().trim_end_matches(".git");
    let path = remote.strip_prefix("git@github.com:").or_else(|| remote.strip_prefix("https://github.com/"))?;
    Some(format!("https://github.com/{path}"))
}

/// The parts of a look, each a row of things to choose from.
const FACE: usize = 0;
const COLOR: usize = 1;
const FILL: usize = 2;
const SHAPE: usize = 3;
/// Not a part, but where the kinds of thing to put on the tile are
/// chosen between: what a click on one of them is.
const TAB: usize = 4;
const TABS: [&str; 4] = ["Drawings", "Emoji", "Letters", "Picture"];
const DRAWINGS: usize = 0;
const EMOJI: usize = 1;
const LETTERS: usize = 2;
/// How many drawings across, and how many rows of them in view.
const ACROSS: usize = 18;
const DOWN: usize = 3;
/// How many across in the rows under them: the name of the palette, as
/// wide as three, then none, then the colors of the longest palette.
const STRIP: usize = 3 + 1 + 22;
/// The colors in the menu: none, and Radix's.
const SWATCHES: usize = HUES.len() + 1;

pub struct Editing {
    pub target: Target,
    part: usize,
    /// Where the keys are in each part.
    at: [usize; 4],
    /// The first row of drawings in view.
    first: usize,
    /// Which kind of thing is being chosen for the tile, and from which palette its color.
    tab: usize,
    palette: usize,
}

/// What can be on a tile.
#[derive(Clone, PartialEq)]
enum Face {
    /// As it comes: the folder's icon, or the drawing its name picks.
    Own,
    Letters(String),
    /// A picture, to be pointed at.
    File,
    Drawing(usize),
    /// By its place in the list of them.
    Emoji(usize),
}

/// Where the parts of the editor are, in the list's side of the box.
struct Plan {
    left: i32,
    top: i32,
    /// The side of a drawing's cell, and of one in the rows under them.
    cell: i32,
    dot: i32,
    grid: i32,
    strips: [i32; 3],
}

impl Palette {
    pub fn look_of(&self, target: &Target) -> Look {
        self.looks.get(&target.key()).cloned().unwrap_or_default()
    }

    /// Give a project a look, and keep it.
    fn dress(&mut self, target: &Target, change: impl FnOnce(&mut Look)) {
        let mut look = self.look_of(target);
        change(&mut look);
        match look.is_plain() {
            true => drop(self.looks.remove(&target.key())),
            false => drop(self.looks.insert(target.key(), look)),
        }
        looks::save(&self.looks);
    }

    // ── The menu ────────────────────────────────────────────────────────

    /// The menu of what the pointer is over, where there is something to say of it.
    pub fn open_menu(&mut self, hit: Option<Hit>) {
        self.open_menu_as(hit, false);
    }

    /// The same, as the panel of actions that Ctrl+K opens.
    pub fn open_panel(&mut self) {
        let hit = if self.beside { Hit::Control(self.control) } else { Hit::Item(self.selected) };
        self.open_menu_as(Some(hit), true);
    }

    fn open_menu_as(&mut self, hit: Option<Hit>, panel: bool) {
        let Some(lines) = self.lines_for(hit) else { return };
        let at = self.pointer.unwrap_or((self.layout.pad * 2, self.layout.head));
        let mut menu = Menu { at, lines, picked: 0, swatch: 0, panel, typed: String::new() };
        menu.settle();
        self.menu = Some(menu);
    }

    /// Do what the keys pressed do to what is picked out, as its actions
    /// say: "Ctrl+I" opens its look. Returns whether they did anything.
    pub fn shortcut(&mut self, keys: &str, event_loop: &ActiveEventLoop) -> bool {
        let hit = if self.beside { Hit::Control(self.control) } else { Hit::Item(self.selected) };
        let Some(lines) = self.lines_for(Some(hit)) else { return false };
        let Some(at) = lines.iter().position(|line| matches!(line, Line::Entry { keys: known, .. } if *known == keys)) else { return false };
        self.menu = Some(Menu { at: (0, 0), lines, picked: at, swatch: 0, panel: true, typed: String::new() });
        self.menu_do(at, 0, event_loop);
        true
    }

    /// What can be done to what is at a place: by the kind of thing it is.
    fn lines_for(&self, hit: Option<Hit>) -> Option<Vec<Line>> {
        let keyed = |glyph, label: &str, act, keys| Line::Entry { glyph, label: label.to_string(), act, keys };
        let entry = |glyph, label: &str, act| keyed(glyph, label, act, "");
        Some(match hit? {
            Hit::Item(at) | Hit::Level(at, _) => {
                let item = self.found.get(at)?;
                let target = item.project.as_ref().map(|project| Target { name: project.name.clone(), folder: project.folder.clone() });
                match (&item.action, target) {
                    (action, Some(target)) => {
                        let go = match action {
                            Action::Activate(_) => keyed(glyph::WINDOW, &format!("Go to {}", target.name), Do::Go(at), "Enter"),
                            _ => keyed(glyph::APP, &format!("Open {}", target.name), Do::Go(at), "Enter"),
                        };
                        let mut lines = vec![
                            go,
                            Line::Group("Look"),
                            keyed(glyph::PENCIL, "Icon and color\u{2026}", Do::Edit(target.clone()), "Ctrl+I"),
                            Line::Swatches(target.clone()),
                            entry(glyph::REFRESH, "Shuffle", Do::Shuffle(target.clone())),
                        ];
                        if !self.look_of(&target).is_plain() {
                            lines.push(entry(glyph::LEFT, "Reset look", Do::Reset(target.clone())));
                        }
                        lines.push(entry(glyph::SEARCH, "Rescan icon", Do::Rescan(target.clone())));
                        if let Some(folder) = &target.folder {
                            lines.push(Line::Group("Folder"));
                            let path = folder.to_string_lossy().into_owned();
                            lines.push(keyed(glyph::CLIPBOARD, "Copy path", Do::Act(Action::Copy(path)), "Ctrl+Shift+C"));
                            if let Some(page) = github_of(folder) {
                                let line = format!("xdg-open {}", super::shell_quoted(&page));
                                lines.push(keyed(glyph::RIGHT, "Open on GitHub", Do::Act(Action::Shell(line)), "Ctrl+G"));
                            }
                        }
                        if let Action::Activate(window) = action {
                            lines.extend([Line::Group("Window"), keyed(glyph::CLOSE, "Close window", Do::Act(Action::Close(*window)), "Ctrl+Shift+W")]);
                        }
                        lines
                    }
                    (Action::Activate(window), None) => vec![
                        keyed(glyph::WINDOW, "Go to it", Do::Go(at), "Enter"),
                        entry(glyph::MAXIMIZE, "Maximize", Do::Act(Action::Maximize(*window))),
                        entry(glyph::FULLSCREEN, "Fullscreen", Do::Act(Action::Fullscreen(*window))),
                        Line::Rule,
                        keyed(glyph::CLOSE, "Close window", Do::Act(Action::Close(*window)), "Ctrl+Shift+W"),
                    ],
                    // What was copied: again, into the window before, opened, or forgotten.
                    (Action::Restore(path), _) => {
                        let mut lines = vec![
                            keyed(glyph::CLIPBOARD, "Copy", Do::Go(at), "Enter"),
                            keyed(glyph::RIGHT, "Paste", Do::Act(Action::Paste(path.clone())), "Ctrl+Enter"),
                        ];
                        if let Some(link) = link_in(path) {
                            let line = format!("xdg-open {}", super::shell_quoted(&link));
                            lines.push(keyed(glyph::APP, "Open link", Do::Act(Action::Shell(line)), "Ctrl+O"));
                        }
                        lines.extend([Line::Rule, keyed(glyph::CLOSE, "Forget", Do::Forget(path.clone()), "Ctrl+D")]);
                        lines
                    }
                    // A color, in each of the ways it is written.
                    (Action::CopyColor(_, picked), _) => {
                        let (palette, hue, shade) = *picked;
                        let formats = colors::formats(palette, hue, shade).into_iter().skip(1);
                        let copy = formats.map(|(name, said)| entry(glyph::CLIPBOARD, &format!("Copy {name}"), Do::Act(Action::CopyColor(said, *picked))));
                        let mut lines: Vec<Line> = std::iter::once(keyed(glyph::CLIPBOARD, "Copy hex", Do::Go(at), "Enter")).chain(copy).collect();
                        // The palette opens on the one last picked; this lets it open as it comes.
                        if colors::last_picked(palette).is_some() {
                            lines.extend([Line::Rule, entry(glyph::CLOSE, "Forget last pick", Do::ForgetPick(palette))]);
                        }
                        lines
                    }
                    (Action::CopyIcon(name), _) => {
                        let mut lines = vec![keyed(glyph::CLIPBOARD, "Copy PNG", Do::Go(at), "Enter")];
                        if let Some(svg) = self.lookup.drawings.get(name) {
                            lines.push(keyed(glyph::CLIPBOARD, "Copy SVG", Do::Act(Action::Copy(svg.clone())), "Ctrl+Shift+C"));
                        }
                        let page = format!("xdg-open {}", super::shell_quoted(&iconify::page_of(name)));
                        lines.extend([entry(glyph::CLIPBOARD, "Copy name", Do::Act(Action::Copy(name.clone()))), keyed(glyph::RIGHT, "Open on Iconify", Do::Act(Action::Shell(page)), "Ctrl+O")]);
                        lines
                    }
                    (Action::Copy(_), _) => vec![keyed(glyph::CLIPBOARD, "Copy", Do::Go(at), "Enter")],
                    (Action::Deed(Deed::Tell(player, _)), _) => vec![
                        keyed(glyph::PLAY, "Play or pause", Do::Go(at), "Enter"),
                        entry(glyph::RIGHT, "Next", Do::Act(Action::Deed(Deed::Tell(player.clone(), "Next")))),
                        entry(glyph::LEFT, "Previous", Do::Act(Action::Deed(Deed::Tell(player.clone(), "Previous")))),
                    ],
                    (Action::Deed(Deed::Use(device)), _) => vec![
                        keyed(glyph::RUN, "Use", Do::Go(at), "Enter"),
                        keyed(glyph::CLOSE, if device.muted { "Unmute" } else { "Mute" }, Do::Act(Action::Deed(Deed::Mute(device.clone()))), "Ctrl+M"),
                    ],
                    (Action::Deed(Deed::Join(_)), _) => vec![keyed(glyph::RUN, "Join", Do::Go(at), "Enter")],
                    (Action::Deed(Deed::Leave(_)), _) => vec![keyed(glyph::CLOSE, "Leave", Do::Go(at), "Enter")],
                    (Action::Launch { .. }, _) => vec![keyed(glyph::APP, "Open", Do::Go(at), "Enter")],
                    _ => vec![keyed(glyph::RUN, &item.title, Do::Go(at), "Enter")],
                }
            }
            Hit::Control(at) => {
                let lit = self.controls.tiles[at].lit;
                let flip = Do::Act(Action::Deed(Deed::Flip(at)));
                match at {
                    controls::WIFI => vec![keyed(glyph::RUN, "Networks\u{2026}", Do::Open(View::Wifi), "Enter"), entry(glyph::REFRESH, if lit { "Turn off" } else { "Turn on" }, flip)],
                    controls::SOUND => vec![keyed(glyph::RUN, "Devices\u{2026}", Do::Open(View::Sound), "Enter"), entry(glyph::CLOSE, if lit { "Mute" } else { "Unmute" }, flip)],
                    _ => vec![entry(glyph::REFRESH, if lit { "Turn off" } else { "Turn on" }, flip)],
                }
            }
            _ => return None,
        })
    }

    /// How wide the menu is, and each line shown and how tall it is.
    fn menu_sizes(&self) -> (i32, Vec<(usize, i32)>) {
        let Layout { px, pad, row, .. } = self.layout;
        let Some(menu) = &self.menu else { return (0, Vec::new()) };
        let wide = if menu.panel { pad * 24 } else { pad * 17 };
        let dot = (wide - pad) / 11;
        let tall = |line: &Line| match line {
            Line::Entry { .. } => row - pad / 4,
            Line::Rule => pad / 2,
            Line::Group(_) => Text::line_height(px * 3 / 4) + pad / 3,
            Line::Swatches(_) => 2 * dot + pad / 2,
        };
        (wide, menu.shown().into_iter().map(|at| (at, tall(&menu.lines[at]))).collect())
    }

    /// The panel's field to type into, at its foot: how tall.
    fn search_height(&self) -> i32 {
        if self.menu.as_ref().is_some_and(|menu| menu.panel) { self.layout.row + self.layout.pad / 4 } else { 0 }
    }

    /// Where the menu is: at the pointer, but all of it within the box.
    /// The panel stands in the corner of the list, over its foot.
    fn menu_area(&self) -> (i32, i32, i32, i32) {
        let (wide, lines) = self.menu_sizes();
        let tall = lines.iter().map(|(_, tall)| tall).sum::<i32>() + self.layout.pad / 2 + self.search_height();
        let Some(menu) = &self.menu else { return (0, 0, 0, 0) };
        if menu.panel {
            let Layout { pad, width, foot, .. } = self.layout;
            return (width - pad / 2 - wide, self.layout.height() - foot - pad / 2 - tall, wide, tall);
        }
        let at = menu.at;
        (at.0.clamp(2, (self.layout.full() - wide - 2).max(2)), at.1.clamp(2, (self.layout.height() - tall - 2).max(2)), wide, tall)
    }

    /// Which line of the menu a point is on, and which color of them where it is on those.
    pub fn menu_hit(&self, x: i32, y: i32) -> Option<Hit> {
        let (left, top, wide, _) = self.menu_area();
        let (pad, menu) = (self.layout.pad, self.menu.as_ref()?);
        let mut line_top = top + pad / 4;
        for (at, tall) in self.menu_sizes().1 {
            if (line_top..line_top + tall).contains(&y) && (left..left + wide).contains(&x) {
                let dot = (wide - pad) / 11;
                return match menu.lines[at] {
                    Line::Entry { .. } => Some(Hit::Menu(at, 0)),
                    Line::Swatches(_) => {
                        let (across, down) = ((x - left - pad / 2) / dot, (y - line_top - pad / 4) / dot);
                        ((0..11).contains(&across) && (0..2).contains(&down) && x >= left + pad / 2).then_some(Hit::Menu(at, (down * 11 + across) as usize))
                    }
                    Line::Rule | Line::Group(_) => None,
                };
            }
            line_top += tall;
        }
        None
    }

    /// Keys, drawn as keys, ending at `right`: "Ctrl+Shift+C" is three.
    pub fn keys_at(&mut self, canvas: &mut Canvas, right: i32, top: i32, keys: &str, on: bool) -> i32 {
        let Layout { px, pad, line, .. } = self.layout;
        let small = px * 3 / 4;
        let tall = line * 4 / 5;
        let mut right = right;
        for key in keys.split('+').rev() {
            let key = match key {
                "Enter" => "\u{21b5}",
                "Shift" => "\u{21e7}",
                "Backspace" => "\u{232b}",
                other => other,
            };
            let wide = (self.text.width(key, small) + pad / 2).max(tall);
            let left = right - wide;
            let (edge, under, ink) = if on { (0xB4C2F2, ACCENT, ON_ACCENT) } else { (0x5A5A5A, 0x2E2E2E, 0xD8D8D8) };
            canvas.round_fill(left, top, wide, tall, tall / 4, edge, 256);
            canvas.round_fill(left + 1, top + 1, wide - 2, tall - 3, tall / 4 - 1, under, 256);
            let across = self.text.width(key, small);
            self.text.draw(canvas, left + (wide - across) / 2, top + (tall - 1 - Text::line_height(small)) / 2, small, ink, key);
            right = left - pad / 4;
        }
        // Where they begin, for what is said before them.
        right + pad / 4
    }

    pub fn draw_menu(&mut self, canvas: &mut Canvas) {
        if self.menu.is_none() {
            return;
        }
        let Layout { px, pad, line, .. } = self.layout;
        let (left, top, wide, tall) = self.menu_area();
        let (_, sizes) = self.menu_sizes();
        let (panel, typed) = self.menu.as_ref().map_or((false, String::new()), |menu| (menu.panel, menu.typed.clone()));
        // The panel is over the box, which sits back a little behind it.
        if panel {
            canvas.round_fill(0, 0, canvas.width, canvas.height, 0, 0x000000, 46);
        }
        canvas.round_fill(left - 1, top - 1, wide + 2, tall + 2, pad / 2 + 1, EDGE_COLOR, 256);
        canvas.round_fill(left, top, wide, tall, pad / 2, if panel { 0x252525 } else { mix(PANEL_COLOR, WELL_COLOR, 128) }, 256);
        let dot = (wide - pad) / 11;
        let mut line_top = top + pad / 4;
        for (at, height) in sizes {
            let Some(menu) = &self.menu else { return };
            let (picked, swatch) = (menu.picked == at, menu.swatch);
            match &menu.lines[at] {
                Line::Entry { glyph, label, keys, .. } => {
                    let (glyph, label, keys) = (*glyph, label.clone(), *keys);
                    let pointed = matches!(self.hover, Some(Hit::Menu(over, _)) if over == at);
                    if picked || pointed {
                        canvas.round_fill(left + pad / 4, line_top, wide - pad / 2, height, pad / 3, if picked { ACCENT } else { WELL_COLOR }, 256);
                    }
                    let (main, dim) = if picked { (ON_ACCENT, 0xC9D3F6) } else { (TEXT_COLOR, DIM_TEXT_COLOR) };
                    let text_top = line_top + (height - line) / 2;
                    self.text.draw(canvas, left + pad * 3 / 4, text_top, px, dim, glyph);
                    let keys_room = if keys.is_empty() { 0 } else { pad * 5 };
                    let label = self.text.clipped(&label, wide - pad * 7 / 2 - keys_room, px);
                    let label_left = left + pad * 5 / 2;
                    self.text.draw(canvas, label_left, text_top, px, main, &label);
                    // What is typed, marked where it is in the words.
                    let found = (!typed.trim().is_empty()).then(|| label.to_lowercase().find(&typed.trim().to_lowercase())).flatten();
                    if let Some(from) = found {
                        let before = self.text.width(&label[..from], px);
                        let across = self.text.width(&label[from..from + typed.trim().len()], px);
                        canvas.fill(label_left + before, text_top + line - 2, across, 2, super::CHANGED_COLOR);
                    }
                    if !keys.is_empty() {
                        self.keys_at(canvas, left + wide - pad * 3 / 4, line_top + (height - line * 4 / 5) / 2, keys, picked);
                    }
                }
                Line::Group(name) => {
                    let small = px * 3 / 4;
                    self.text.draw(canvas, left + pad * 3 / 4, line_top + pad / 3, small, DIM_TEXT_COLOR, &name.to_uppercase());
                }
                Line::Rule => canvas.fill(left + pad / 2, line_top + height / 2, wide - pad, 1, RULE_COLOR),
                Line::Swatches(target) => {
                    // None of them, which is the color its name comes out as; then each.
                    let now = self.look_of(target).hue;
                    for which in 0..SWATCHES {
                        let (x, y) = (left + pad / 2 + (which % 11) as i32 * dot, line_top + pad / 4 + (which / 11) as i32 * dot);
                        let on = if which == 0 { now.is_none() } else { now.as_deref() == Some(HUES[which - 1].0) };
                        let pointed = self.hover == Some(Hit::Menu(at, which));
                        if (picked && swatch == which) || pointed {
                            canvas.round_fill(x, y, dot, dot, dot / 2, if pointed { TEXT_COLOR } else { ACCENT }, 256);
                        }
                        let inset = (dot / 7).max(2);
                        let color = if which == 0 { KEY_COLOR } else { HUES[which - 1].1 };
                        canvas.round_fill(x + inset, y + inset, dot - 2 * inset, dot - 2 * inset, dot / 2, color, 256);
                        if on {
                            let mark = dot / 3;
                            canvas.round_fill(x + (dot - mark) / 2, y + (dot - mark) / 2, mark, mark, mark / 2, if which == 0 { TEXT_COLOR } else { 0xFFFFFF }, 256);
                        }
                    }
                }
            }
            line_top += height;
        }
        // The panel's field, at its foot: what is typed narrows the actions.
        if panel {
            let search_top = top + tall - self.search_height();
            canvas.fill(left + 1, search_top, wide - 2, 1, RULE_COLOR);
            let text_top = search_top + (self.search_height() - line) / 2;
            self.text.draw(canvas, left + pad * 3 / 4, text_top, px, DIM_TEXT_COLOR, glyph::SEARCH);
            let typed_left = left + pad * 5 / 2;
            if typed.is_empty() {
                canvas.fill(typed_left, text_top, 2, line, TEXT_COLOR);
                self.text.draw(canvas, typed_left + pad / 2, text_top, px, DIM_TEXT_COLOR, "Search actions");
            } else {
                self.text.draw(canvas, typed_left, text_top, px, TEXT_COLOR, &typed);
                let across = self.text.width(&typed, px);
                canvas.fill(typed_left + across + 1, text_top, 2, line, TEXT_COLOR);
            }
        }
    }

    /// Do what a line of the menu says: the one picked out, or `line`.
    pub fn menu_do(&mut self, line: usize, swatch: usize, event_loop: &ActiveEventLoop) {
        let Some(menu) = self.menu.take() else { return };
        match menu.lines.get(line) {
            Some(Line::Swatches(target)) => {
                let hue = swatch.checked_sub(1).and_then(|at| HUES.get(at)).map(|(name, _)| name.to_string());
                self.dress(target, |look| look.hue = hue);
            }
            Some(Line::Entry { act, .. }) => match act.clone() {
                Do::Go(at) => self.go(at, event_loop),
                Do::Open(view) => self.open(view),
                Do::Edit(target) => self.edit(target),
                Do::Shuffle(target) => {
                    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |since| since.subsec_nanos());
                    self.dress(&target, |look| *look = looks::shuffled(projects::number_of(&seed.to_string())));
                }
                Do::Reset(target) => self.dress(&target, |look| *look = Look::default()),
                Do::Rescan(target) => {
                    projects::forget(&target.name, target.folder.as_deref());
                    self.icons.clear();
                }
                Do::Act(Action::Deed(deed)) => self.controls.act(deed),
                Do::ForgetPick(palette) => colors::forget_picked(palette),
                Do::Forget(path) => {
                    let _ = std::fs::remove_file(&path);
                    self.forget_copy(&path);
                }
                // What is done to a window is done with the box out of the way.
                Do::Act(action) => {
                    if let Some(window) = &self.window {
                        window.set_visible(false);
                    }
                    super::perform(&action, self.desk.as_ref());
                    event_loop.exit();
                }
            },
            _ => {}
        }
    }

    /// A key, while the menu is up.
    pub fn menu_key(&mut self, key: Key<&str>, arrow: Option<(i32, i32)>, event_loop: &ActiveEventLoop) {
        let ctrl = self.modifiers.control_key();
        // The keys of an action do it, from the panel as from the list.
        if let Some(keys) = self.keys_pressed(&key) {
            if keys != "Ctrl+K" {
                if let Some(at) = self.menu.as_ref().and_then(|menu| menu.lines.iter().position(|line| matches!(line, Line::Entry { keys: known, .. } if *known == keys))) {
                    return self.menu_do(at, 0, event_loop);
                }
            }
        }
        let Some(menu) = &mut self.menu else { return };
        let on_colors = matches!(menu.lines.get(menu.picked), Some(Line::Swatches(_)));
        match (key, arrow) {
            // Esc takes back what was typed, then closes it; Ctrl+K closes it as it opened it.
            (Key::Named(NamedKey::Escape), _) if !menu.typed.is_empty() => menu.typed.clear(),
            (Key::Named(NamedKey::Escape), _) => self.menu = None,
            (Key::Character("k"), _) if ctrl => self.menu = None,
            (Key::Named(NamedKey::Enter), _) => {
                let (line, swatch) = (menu.picked, menu.swatch);
                self.menu_do(line, swatch, event_loop);
            }
            // Among the colors the arrows go across, and down their two lines before off them.
            (_, Some((across, 0))) if on_colors => menu.swatch = (menu.swatch as i32 + across).clamp(0, SWATCHES as i32 - 1) as usize,
            (_, Some((0, 1))) if on_colors && menu.swatch < 11 => menu.swatch += 11,
            (_, Some((0, -1))) if on_colors && menu.swatch >= 11 => menu.swatch -= 11,
            (_, Some((0, down))) => {
                let shown = menu.shown();
                let can: Vec<usize> = shown.into_iter().filter(|at| menu.can(*at)).collect();
                if let Some(now) = can.iter().position(|at| *at == menu.picked) {
                    menu.picked = can[(now as i32 + down).rem_euclid(can.len() as i32) as usize];
                }
            }
            // Typed into, in the panel: the actions whose words have it.
            (Key::Named(NamedKey::Backspace), _) if menu.panel => {
                menu.typed.pop();
                menu.settle();
            }
            (Key::Named(NamedKey::Space), _) if menu.panel => {
                menu.typed.push(' ');
                menu.settle();
            }
            (Key::Character(letters), _) if menu.panel && !ctrl && !self.modifiers.alt_key() => {
                menu.typed.push_str(letters);
                menu.settle();
            }
            _ => {}
        }
    }

    /// The keys pressed, as an action's keys are written: "Ctrl+Shift+C".
    /// Only with Ctrl, as only those can be an action's.
    pub fn keys_pressed(&self, key: &Key<&str>) -> Option<String> {
        if !self.modifiers.control_key() {
            return None;
        }
        let name = match key {
            Key::Character(letter) => letter.to_uppercase(),
            Key::Named(NamedKey::Enter) => "Enter".to_string(),
            Key::Named(NamedKey::Backspace) => "Backspace".to_string(),
            _ => return None,
        };
        let shift = if self.modifiers.shift_key() { "Shift+" } else { "" };
        Some(format!("Ctrl+{shift}{name}"))
    }

    // ── The look, chosen ────────────────────────────────────────────────

    /// Open a project's look to be chosen, in place of the list.
    pub fn edit(&mut self, target: Target) {
        let look = self.look_of(&target);
        // Open where what it has now is: its emoji among the emoji, its color's palette.
        let tab = match look.icon.as_deref() {
            Some(icon) if icon.starts_with("emoji:") => EMOJI,
            Some(icon) if icon.starts_with("letters:") => LETTERS,
            Some(icon) if icon.starts_with("file:") => TABS.len() - 1,
            _ => DRAWINGS,
        };
        let palette = look.hue.as_deref().and_then(looks::color_named).map_or(0, |(palette, _)| palette);
        self.editing = Some(Editing { target, part: FACE, at: [0; 4], first: 0, tab, palette });
        self.open(View::Look);
        self.settle_editing();
    }

    /// What can go on the tile, of the kind being chosen: drawings and
    /// emoji are those whose names match what is typed.
    fn faces(&self) -> Vec<Face> {
        let Some(editing) = &self.editing else { return Vec::new() };
        let typed = self.field.string();
        let typed = typed.trim();
        match editing.tab {
            DRAWINGS => {
                let names: Vec<String> = GLYPHS.iter().map(|name| name.replace('-', " ")).collect();
                let drawings = matching::ranked(typed, names.iter().map(String::as_str)).into_iter().map(Face::Drawing);
                std::iter::once(Face::Own).chain(drawings).collect()
            }
            EMOJI => {
                let all = emoji::all();
                let finds: Vec<String> = all.iter().map(|emoji| format!("{} {}", emoji.name, emoji.group)).collect();
                matching::ranked(typed, finds.iter().map(String::as_str)).into_iter().map(Face::Emoji).collect()
            }
            // What is typed is the letters; with nothing typed, its initials.
            LETTERS => {
                let initials: String = editing.target.name.split(|c: char| !c.is_alphanumeric()).filter_map(|word| word.chars().next()).take(2).collect();
                let letters: String = if typed.is_empty() { initials } else { typed.chars().take(2).collect() };
                vec![Face::Letters(letters.to_uppercase())]
            }
            _ => vec![Face::Own, Face::File],
        }
    }

    /// How many there are to choose from in a part.
    fn count(&self, part: usize) -> usize {
        match part {
            FACE => self.faces().len(),
            COLOR => 2 + looks::PALETTES[self.editing.as_ref().map_or(0, |editing| editing.palette)].1.len(),
            FILL => FILLS.len(),
            SHAPE => SHAPES.len(),
            _ => TABS.len(),
        }
    }

    /// Put the keys, in each part, on what the look has now.
    fn settle_editing(&mut self) {
        let now = self.now_at();
        // Where it has none of these, the keys start at the first.
        let at = now.map(|at| if at == usize::MAX { 0 } else { at });
        if let Some(editing) = &mut self.editing {
            editing.at = at;
            editing.first = (at[FACE] / ACROSS).saturating_sub(DOWN - 1);
        }
    }

    /// Where, in each part, what the look has now is.
    fn now_at(&self) -> [usize; 4] {
        let Some(target) = self.editing.as_ref().map(|editing| editing.target.clone()) else { return [0; 4] };
        let look = self.look_of(&target);
        let faces = self.faces();
        let face = match look.icon.as_deref() {
            None => Some(Face::Own),
            Some(icon) if icon.starts_with("letters:") => faces.first().cloned(),
            Some(icon) if icon.starts_with("file:") => Some(Face::File),
            Some(icon) if icon.starts_with("emoji:") => emoji::all().iter().position(|emoji| Some(emoji.text.as_str()) == icon.strip_prefix("emoji:")).map(Face::Emoji),
            Some(_) => Some(Face::Drawing(looks::glyph_of(&look, &target.name))),
        };
        let palette = self.editing.as_ref().map_or(0, |editing| editing.palette);
        // A color of another palette than the one shown is none of these.
        let color = match look.hue.as_deref().map(looks::color_named) {
            None => 1,
            Some(Some((of, at))) if of == palette => at + 2,
            Some(_) => usize::MAX,
        };
        let place = |from: &[&str], of: &str| from.iter().position(|known| *known == of).unwrap_or(0);
        [
            face.and_then(|face| faces.iter().position(|known| *known == face)).unwrap_or(usize::MAX),
            color,
            place(&FILLS, looks::fill_of(&look)),
            place(&SHAPES, looks::shape_of(&look, &target.name)),
        ]
    }

    /// What was typed changed: the drawings are others now.
    pub fn editing_typed(&mut self) {
        // Something typed is looked for among the drawings, past the one that is not.
        let past = usize::from(!self.field.string().trim().is_empty() && self.editing.as_ref().is_some_and(|editing| editing.tab == DRAWINGS) && self.faces().len() > 1);
        if let Some(editing) = &mut self.editing {
            (editing.part, editing.at[FACE], editing.first) = (FACE, past, 0);
        }
    }

    /// Go to another kind of thing to put on the tile.
    fn to_tab(&mut self, tab: usize) {
        if let Some(editing) = &mut self.editing {
            (editing.tab, editing.part) = (tab % TABS.len(), FACE);
        }
        self.field = super::field::TextField::new("");
        self.settle_editing();
    }

    fn plan(&mut self) -> Plan {
        let Layout { pad, head, row, width, .. } = self.layout;
        let inside = width - pad;
        let (cell, dot) = (inside / ACROSS as i32, inside / STRIP as i32);
        let top = head + pad / 4;
        let grid = top + row * 3 / 2 + pad / 4;
        let strips = [0, 1, 2].map(|strip| grid + DOWN as i32 * cell + pad / 2 + strip * (dot + pad / 4));
        Plan { left: pad / 2, top, cell, dot, grid, strips }
    }

    /// Where one of the things to choose from is drawn: left, top, how wide and how tall.
    fn cell(&mut self, part: usize, at: usize) -> Option<(i32, i32, i32, i32)> {
        let plan = self.plan();
        let first = self.editing.as_ref()?.first;
        match part {
            FACE => {
                let down = (at / ACROSS).checked_sub(first).filter(|down| *down < DOWN)?;
                Some((plan.left + (at % ACROSS) as i32 * plan.cell, plan.grid + down as i32 * plan.cell, plan.cell, plan.cell))
            }
            // The palette's name, as wide as three, and then the rest.
            COLOR if at == 0 => Some((plan.left, plan.strips[0], 3 * plan.dot, plan.dot)),
            COLOR => Some((plan.left + (at as i32 + 2) * plan.dot, plan.strips[0], plan.dot, plan.dot)),
            FILL | SHAPE => Some((plan.left + at as i32 * plan.dot, plan.strips[part - 1], plan.dot, plan.dot)),
            // The kinds, as words across the top, at the right.
            _ => {
                let Layout { px, pad, line, width, .. } = self.layout;
                let wide = |text: &mut Text, at: usize| text.width(TABS[at], px) + pad;
                let from = width - pad - (0..TABS.len()).map(|at| wide(&mut self.text, at)).sum::<i32>();
                let left = from + (0..at).map(|at| wide(&mut self.text, at)).sum::<i32>();
                Some((left, plan.top + pad / 4, wide(&mut self.text, at), line + pad / 2))
            }
        }
    }

    pub fn editor_hit(&mut self, x: i32, y: i32) -> Option<Hit> {
        (FACE..=TAB).find_map(|part| {
            (0..self.count(part)).find_map(|at| {
                let (left, top, wide, tall) = self.cell(part, at)?;
                ((left..left + wide).contains(&x) && (top..top + tall).contains(&y)).then_some(Hit::Cell(part, at))
            })
        })
    }

    /// Choose what is at a place in a part, and keep it.
    pub fn choose(&mut self, part: usize, at: usize) {
        let Some((target, palette)) = self.editing.as_ref().map(|editing| (editing.target.clone(), editing.palette)) else { return };
        if part == TAB {
            return self.to_tab(at);
        }
        if let Some(editing) = &mut self.editing {
            (editing.part, editing.at[part]) = (part, at);
        }
        match part {
            FACE => match self.faces().get(at).cloned() {
                Some(Face::Own) => self.dress(&target, |look| look.icon = None),
                Some(Face::Letters(letters)) => self.dress(&target, |look| look.icon = Some(format!("letters:{letters}"))),
                Some(Face::Drawing(drawing)) => self.dress(&target, |look| look.icon = Some(format!("solar:{}", GLYPHS[drawing]))),
                Some(Face::Emoji(at)) => self.dress(&target, |look| look.icon = Some(format!("emoji:{}", emoji::all()[at].text))),
                Some(Face::File) => self.point_at_a_picture(),
                None => {}
            },
            // The palette's name goes to the next palette.
            COLOR if at == 0 => {
                if let Some(editing) = &mut self.editing {
                    editing.palette = (palette + 1) % looks::PALETTES.len();
                }
            }
            COLOR => self.dress(&target, |look| look.hue = (at >= 2).then(|| looks::color_kept(palette, at - 2))),
            FILL => self.dress(&target, |look| look.fill = Some(FILLS[at].to_string())),
            _ => self.dress(&target, |look| look.shape = Some(SHAPES[at].to_string())),
        }
    }

    /// Ask, with the desktop's own chooser, for a picture. The box is out
    /// of its way meanwhile, and is not gone because it is not in front.
    fn point_at_a_picture(&mut self) {
        let (sender, chosen) = mpsc::channel();
        (self.chosen, self.away) = (Some(chosen), true);
        if let Some(window) = &self.window {
            window.set_visible(false);
        }
        let wake = self.wake.clone();
        thread::spawn(move || {
            let filter = "--file-filter=Pictures | *.png *.svg *.jpg *.jpeg *.webp *.ico";
            let asked = Command::new("zenity").args(["--file-selection", "--title=A picture for the project", filter]).stderr(Stdio::null()).output();
            let path = asked.ok().filter(|asked| asked.status.success()).map(|asked| String::from_utf8_lossy(&asked.stdout).trim().to_string());
            let _ = sender.send(path.filter(|path| !path.is_empty()));
            if let Some(wake) = wake {
                wake();
            }
        });
    }

    /// The chooser has answered, if it has.
    pub fn picture_chosen(&mut self) {
        let Some(answer) = self.chosen.as_ref().and_then(|chosen| chosen.try_recv().ok()) else { return };
        (self.chosen, self.away) = (None, false);
        if let (Some(path), Some(target)) = (answer, self.editing.as_ref().map(|editing| editing.target.clone())) {
            self.dress(&target, |look| look.icon = Some(format!("file:{path}")));
        }
        if let Some(window) = &self.window {
            window.set_visible(true);
            window.focus_window();
            window.request_redraw();
        }
    }

    /// A key, while a look is being chosen. Returns whether it was one of the editor's.
    pub fn editor_key(&mut self, key: Key<&str>, arrow: Option<(i32, i32)>) -> bool {
        let counts = [FACE, COLOR, FILL, SHAPE].map(|part| self.count(part));
        let ctrl = self.modifiers.control_key();
        let Some(editing) = &mut self.editing else { return false };
        let (part, at, tab) = (editing.part, editing.at[editing.part], editing.tab);
        match (key, arrow) {
            (Key::Named(NamedKey::Enter), _) => self.choose(part, at),
            // With Ctrl, Tab goes between the kinds: drawings, emoji, letters, a picture.
            (Key::Named(NamedKey::Tab), _) if ctrl => self.to_tab(if self.modifiers.shift_key() { tab + TABS.len() - 1 } else { tab + 1 }),
            (Key::Named(NamedKey::Tab), _) => editing.part = (part + 1) % 4,
            (_, Some((across, 0))) => editing.at[part] = (at as i32 + across).clamp(0, counts[part] as i32 - 1) as usize,
            // Up and down through the rows of drawings, and off them to the next part.
            (_, Some((0, down))) if part == FACE && (0..counts[FACE] as i32).contains(&(at as i32 + down * ACROSS as i32)) => {
                editing.at[FACE] = (at as i32 + down * ACROSS as i32) as usize;
            }
            (_, Some((0, down))) => editing.part = (part as i32 + down).clamp(0, 3) as usize,
            _ => return false,
        }
        if let Some(editing) = &mut self.editing {
            let down = editing.at[FACE] / ACROSS;
            editing.first = editing.first.clamp((down + 1).saturating_sub(DOWN), down);
        }
        true
    }

    /// What the keys are on, in words, for the foot of the box: a cell is
    /// too small to say it.
    pub fn editor_named(&self) -> String {
        let Some(editing) = &self.editing else { return String::new() };
        let at = editing.at[editing.part];
        match editing.part {
            FACE => match self.faces().get(at) {
                Some(Face::Own) => "Own icon".to_string(),
                Some(Face::Letters(letters)) => letters.clone(),
                Some(Face::File) => "Picture\u{2026}".to_string(),
                Some(Face::Drawing(drawing)) => GLYPHS[*drawing].replace('-', " "),
                Some(Face::Emoji(at)) => emoji::all().get(*at).map_or(String::new(), |emoji| format!("{} \u{b7} {}", emoji.name, emoji.group)),
                None => "Nothing matches".to_string(),
            },
            COLOR => match at {
                0 => format!("Palette: {}", looks::PALETTES[editing.palette].0),
                1 => "Auto".to_string(),
                at => format!("{} {}", looks::PALETTES[editing.palette].0, looks::PALETTES[editing.palette].1[at - 2].0),
            },
            FILL => format!("Background: {}", FILLS[at]),
            _ => format!("Shape: {}", SHAPES[at]),
        }
    }

    /// The wheel, over the drawings: a row a notch.
    pub fn editor_wheel(&mut self, down: bool) {
        let rows = self.count(FACE).div_ceil(ACROSS);
        if let Some(editing) = &mut self.editing {
            editing.first = if down { (editing.first + 1).min(rows.saturating_sub(DOWN)) } else { editing.first.saturating_sub(1) };
        }
    }

    /// A small tile of a look, drawn once and kept while the box is up.
    fn sample(&mut self, canvas: &mut Canvas, target: &Target, look: &Look, own: bool, (left, top, side): (i32, i32, i32)) {
        let key = format!("sample {} {} {own} {side}", target.key(), look.said());
        let draw = || {
            let own = own.then(|| projects::own_of(&target.name, target.folder.as_deref(), look)).flatten();
            Some(looks::render(look, &target.name, own.as_ref(), side.max(1) as usize))
        };
        if let Some(icon) = self.icons.entry(key).or_insert_with(draw) {
            canvas.icon(icon, left, top, side);
        }
    }

    pub fn draw_editor(&mut self, canvas: &mut Canvas) {
        let Some((target, part, at, tab, palette)) = self.editing.as_ref().map(|editing| (editing.target.clone(), editing.part, editing.at, editing.tab, editing.palette)) else { return };
        let Layout { px, pad, line, row, .. } = self.layout;
        let plan = self.plan();
        let look = self.look_of(&target);
        let (hue, color) = looks::hue_of(&look, &target.name);
        let small = px * 5 / 6;
        let faces = self.faces();

        // The tile as it is now, and in words what it is made of.
        let big = row * 3 / 2 - pad / 4;
        self.sample(canvas, &target, &look, true, (plan.left + pad / 2, plan.top, big));
        let words_left = plan.left + big + pad * 3 / 2;
        let tabs_left = self.cell(TAB, 0).map_or(self.layout.width, |(left, ..)| left);
        self.text.draw(canvas, words_left, plan.top + pad / 4, px, TEXT_COLOR, &target.name);
        let on_it = match look.icon.as_deref() {
            None => "its own icon".to_string(),
            Some(icon) => icon.split_once(':').map_or(icon.to_string(), |(kind, what)| match kind {
                "file" => "a picture".to_string(),
                "letters" => format!("the letters {what}"),
                "emoji" => emoji::all().iter().find(|emoji| emoji.text == what).map_or("an emoji".to_string(), |emoji| emoji.name.clone()),
                _ => what.replace('-', " "),
            }),
        };
        let made_of = format!("{on_it} · {}{} · {} · {}", hue, if look.hue.is_none() { " (auto)" } else { "" }, looks::fill_of(&look), looks::shape_of(&look, &target.name));
        let made_of = self.text.clipped(&made_of, tabs_left - words_left - pad, small);
        self.text.draw(canvas, words_left, plan.top + pad / 4 + line, small, DIM_TEXT_COLOR, &made_of);

        // The kinds of thing that can go on it, as a control of segments.
        for which in 0..TABS.len() {
            let Some((left, top, wide, tall)) = self.cell(TAB, which) else { continue };
            let pointed = self.hover == Some(Hit::Cell(TAB, which));
            if which == tab || pointed {
                canvas.round_fill(left, top, wide, tall, tall / 2, if which == tab { KEY_COLOR } else { WELL_COLOR }, 256);
            }
            let across = self.text.width(TABS[which], px);
            self.text.draw(canvas, left + (wide - across) / 2, top + (tall - line) / 2, px, if which == tab { TEXT_COLOR } else { DIM_TEXT_COLOR }, TABS[which]);
        }

        // Where there are few to choose from, what to do is said beside them.
        let beside = match tab {
            LETTERS => Some("Type letters"),
            DRAWINGS | EMOJI if faces.is_empty() => Some("No match"),
            _ => None,
        };
        if let Some(beside) = beside {
            let left = plan.left + faces.len() as i32 * plan.cell + pad / 2;
            self.text.draw(canvas, left, plan.grid + (plan.cell - line) / 2, px, DIM_TEXT_COLOR, beside);
        }

        // Each thing to choose from, in its cell: under the keys it has a
        // line round it, and what the look has now stands on a ground.
        let settled = self.now_at();
        for which in [FACE, COLOR, FILL, SHAPE] {
            for place in 0..self.count(which) {
                let Some((left, top, wide, side)) = self.cell(which, place) else { continue };
                let (under_keys, is_now) = (part == which && at[which] == place, settled[which] == place);
                let pointed = self.hover == Some(Hit::Cell(which, place));
                if under_keys {
                    canvas.round_fill(left, top, wide, side, side / 5, ACCENT, 256);
                    canvas.round_fill(left + 2, top + 2, wide - 4, side - 4, side / 5 - 1, if is_now { WELL_COLOR } else { PANEL_COLOR }, 256);
                } else if is_now || pointed {
                    canvas.round_fill(left, top, wide, side, side / 5, if is_now { KEY_COLOR } else { WELL_COLOR }, 256);
                }
                let inset = (side / 7).max(3);
                let inner = (left + inset, top + inset, side - 2 * inset);
                match which {
                    FACE if matches!(faces[place], Face::Emoji(_)) => {
                        let Face::Emoji(at) = faces[place] else { continue };
                        let text = &emoji::all()[at].text;
                        let side = inner.2.max(1) as usize;
                        let drawn = self.icons.entry(format!("emoji {text} {side}")).or_insert_with(|| emoji::picture(text).map(|picture| projects::fitted_to(&picture, side)));
                        if let Some(icon) = drawn {
                            canvas.icon(icon, inner.0, inner.1, inner.2);
                        }
                    }
                    // The palette, by name: pressed, the next.
                    COLOR if place == 0 => {
                        let name = looks::PALETTES[palette].0;
                        let across = self.text.width(name, small);
                        self.text.draw(canvas, left + (wide - across) / 2, top + (side - Text::line_height(small)) / 2, small, TEXT_COLOR, name);
                    }
                    FACE => match &faces[place] {
                        Face::Own => self.sample(canvas, &target, &Look { icon: None, ..look.clone() }, true, inner),
                        Face::Letters(letters) => self.sample(canvas, &target, &Look { icon: Some(format!("letters:{letters}")), ..look.clone() }, false, inner),
                        Face::File => {
                            let across = self.text.width(glyph::APP, px);
                            self.text.draw(canvas, left + (side - across) / 2, top + (side - line) / 2, px, DIM_TEXT_COLOR, glyph::APP);
                        }
                        Face::Drawing(drawing) => {
                            let size = (side * 5 / 8) as u32;
                            let (full, faint) = duo(*drawing);
                            let at = (left + (side - size as i32) / 2, top + (side - Text::line_height(size)) / 2);
                            self.text.draw(canvas, at.0, at.1, size, mix(PANEL_COLOR, color, 130), &faint.to_string());
                            self.text.draw(canvas, at.0, at.1, size, color, &full.to_string());
                        }
                        Face::Emoji(_) => {}
                    },
                    COLOR => match place.checked_sub(2) {
                        Some(hue) => canvas.round_fill(inner.0, inner.1, inner.2, inner.2, inner.2 / 2, looks::PALETTES[palette].1[hue].1, 256),
                        // None: a ring, with nothing in it.
                        None => {
                            canvas.round_fill(inner.0, inner.1, inner.2, inner.2, inner.2 / 2, DIM_TEXT_COLOR, 256);
                            canvas.round_fill(inner.0 + 2, inner.1 + 2, inner.2 - 4, inner.2 - 4, inner.2 / 2 - 2, PANEL_COLOR, 256);
                        }
                    },
                    FILL => self.sample(canvas, &target, &Look { fill: Some(FILLS[place].to_string()), icon: look.icon.clone().or(Some(format!("solar:{}", GLYPHS[looks::glyph_of(&look, &target.name)]))), ..look.clone() }, false, inner),
                    // The shape bare, in no color: it is the outline that is chosen.
                    _ => {
                        let bare = Look { icon: Some("letters:".to_string()), hue: None, fill: Some("ink".to_string()), shape: Some(SHAPES[place].to_string()) };
                        self.sample(canvas, &Target { name: String::new(), folder: None }, &bare, false, inner);
                    }
                }
            }
        }
    }
}
