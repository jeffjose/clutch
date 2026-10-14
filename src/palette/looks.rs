//! How a project's tile looks, where you have said: what is on it (one of
//! Solar's drawings, letters, a picture of your choosing), its color, what
//! it is filled with and the shape it is cut to. As envelope's places are.
//!
//! What is not said is as it was before any of this: the project's own
//! icon where its folder has one, and otherwise a drawing, a color and a
//! shape that its name picks.

use super::canvas::{mix, Icon, GEIST};
use super::emoji;
use super::projects::{fitted_to, number_of};
use crate::config;
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};
use std::fs;

#[derive(Clone, Default, PartialEq, Debug, serde::Deserialize)]
pub struct Look {
    /// "solar:rocket", "letters:JG", or "file:/a/picture.png".
    #[serde(default)]
    pub icon: Option<String>,
    /// One of `HUES`, by name.
    #[serde(default)]
    pub hue: Option<String>,
    /// One of `FILLS`.
    #[serde(default)]
    pub fill: Option<String>,
    /// One of `SHAPES`.
    #[serde(default)]
    pub shape: Option<String>,
}

impl Look {
    /// Whether nothing has been said of it.
    pub fn is_plain(&self) -> bool {
        *self == Look::default()
    }

    /// All of it in a line: what a picture drawn from it is kept under.
    pub fn said(&self) -> String {
        let part = |part: &Option<String>| part.clone().unwrap_or_default();
        format!("{}|{}|{}|{}", part(&self.icon), part(&self.hue), part(&self.fill), part(&self.shape))
    }
}

/// Radix's bright hues, round the wheel, each at its solid step.
pub const HUES: [(&str, u32); 21] = [
    ("tomato", 0xE54D2E),
    ("orange", 0xF76B15),
    ("amber", 0xFFC53D),
    ("yellow", 0xFFE629),
    ("lime", 0xBDEE63),
    ("grass", 0x46A758),
    ("green", 0x30A46C),
    ("jade", 0x29A383),
    ("mint", 0x86EAD4),
    ("teal", 0x12A594),
    ("cyan", 0x00A2C7),
    ("sky", 0x7CE2FE),
    ("blue", 0x0090FF),
    ("indigo", 0x3E63DD),
    ("iris", 0x5B5BD6),
    ("violet", 0x6E56CF),
    ("purple", 0x8E4EC6),
    ("plum", 0xAB4ABA),
    ("pink", 0xD6409F),
    ("crimson", 0xE93D82),
    ("ruby", 0xE54666),
];
/// Tailwind's colors, each at its 500.
pub const TAILWIND: [(&str, u32); 22] = [
    ("slate", 0x64748B),
    ("gray", 0x6B7280),
    ("zinc", 0x71717A),
    ("stone", 0x78716C),
    ("red", 0xEF4444),
    ("orange", 0xF97316),
    ("amber", 0xF59E0B),
    ("yellow", 0xEAB308),
    ("lime", 0x84CC16),
    ("green", 0x22C55E),
    ("emerald", 0x10B981),
    ("teal", 0x14B8A6),
    ("cyan", 0x06B6D4),
    ("sky", 0x0EA5E9),
    ("blue", 0x3B82F6),
    ("indigo", 0x6366F1),
    ("violet", 0x8B5CF6),
    ("purple", 0xA855F7),
    ("fuchsia", 0xD946EF),
    ("pink", 0xEC4899),
    ("rose", 0xF43F5E),
    ("neutral", 0x737373),
];

/// The sets of colors there are to choose from, by name. A color of
/// Radix's is kept by its name alone, as it was first; another's as
/// "tailwind:blue".
pub const PALETTES: [(&str, &[(&str, u32)]); 2] = [("Radix", &HUES), ("Tailwind", &TAILWIND)];

/// A color, by how it is kept: which palette, and where in it.
pub fn color_named(hue: &str) -> Option<(usize, usize)> {
    let (palette, name) = match hue.split_once(':') {
        Some((palette, name)) => (PALETTES.iter().position(|(known, _)| known.eq_ignore_ascii_case(palette))?, name),
        None => (0, hue),
    };
    Some((palette, PALETTES[palette].1.iter().position(|(known, _)| *known == name)?))
}

/// How a color is kept.
pub fn color_kept(palette: usize, at: usize) -> String {
    let name = PALETTES[palette].1[at].0;
    if palette == 0 { name.to_string() } else { format!("{}:{name}", PALETTES[palette].0.to_lowercase()) }
}

/// What a tile can be filled with; "none" is no tile, the drawing alone.
pub const FILLS: [&str; 18] = [
    "none", "tint", "pastel", "haze", "solid", "gradient", "glow", "dusk", "aurora", "holo", "split", "stripes", "rays", "rings", "dots",
    "check", "night", "ink",
];
pub const SHAPES: [&str; 21] = [
    "rounded", "squircle", "circle", "cookie", "clover", "burst", "hexagon", "drop", "diamond", "triangle", "pentagon", "octagon", "star",
    "sun", "flower", "stamp", "arch", "leaf", "oval", "heart", "blob",
];

/// Solar's drawings, in the order the font has them: tools/project_icons.py.
pub const GLYPHS: [&str; 144] = [
    "rocket", "planet", "atom", "bolt", "flame", "leaf", "ghost", "cat", "paw", "crown", "star", "compass", "telescope", "satellite", "ufo",
    "asteroid", "black-hole", "magnet", "lightbulb", "key", "gamepad", "gameboy", "dumbbell", "cup-hot", "donut", "balloon", "bomb", "bone",
    "confetti", "magic-stick-3", "palette", "paint-roller", "cassette", "vinyl", "boombox", "headphones-round", "camera", "clapperboard",
    "reel", "tv", "radio", "map", "globus", "signpost", "umbrella", "snowflake", "sun", "moon-stars", "cloud-bolt", "tornado", "waterdrop",
    "fire", "ferris-wheel", "skateboard", "football", "basketball", "bowling", "golf", "tennis", "volleyball", "target", "medal-star",
    "cup-star", "gift", "box", "suitcase", "backpack", "t-shirt", "glasses", "wineglass", "tea-cup", "chef-hat", "whisk", "bottle",
    "test-tube", "dna", "bacteria", "virus", "pill", "stethoscope", "cpu", "database", "server", "flash-drive", "sd-card", "usb", "socket",
    "wheel", "bus", "tram", "scooter", "plain", "delivery", "garage", "home-smile", "buildings", "city", "shop", "sofa", "armchair", "lamp",
    "chandelier", "bath", "bell", "flag", "bookmark", "book", "notebook", "diploma", "square-academic-cap", "calculator", "ruler-pen",
    "scissors", "eraser", "pen", "paperclip", "pin", "binoculars", "flashlight", "hourglass", "stopwatch", "alarm", "wallet", "money-bag",
    "ticket", "tag", "cart", "masks", "incognito", "bot", "bug", "code", "layers", "widget", "structure", "heart", "shield", "lock",
    "sledgehammer", "toolbox", "broom", "corkscrew", "ladle", "perfume",
];

/// A drawing's two letters in the font of them: the full tone, and the
/// one Solar draws at half strength.
pub fn glyph_letters(at: usize) -> (char, char) {
    let letter = |tone: u32| char::from_u32(0xE000 + 2 * at as u32 + tone).unwrap_or(' ');
    (letter(0), letter(1))
}

/// Where the looks are kept: beside the settings.
fn path() -> std::path::PathBuf {
    config::path().with_file_name("looks.toml")
}

/// Every look that has been set, by the project's folder.
pub fn load() -> BTreeMap<String, Look> {
    toml::from_str(&fs::read_to_string(path()).unwrap_or_default()).unwrap_or_default()
}

pub fn save(looks: &BTreeMap<String, Look>) {
    let quoted = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    let mut text = String::from("# How the projects in clutch's box look. Written by the box: right-click a project there.\n");
    for (project, look) in looks.iter().filter(|(_, look)| !look.is_plain()) {
        text += &format!("\n[{}]\n", quoted(project));
        for (name, part) in [("icon", &look.icon), ("hue", &look.hue), ("fill", &look.fill), ("shape", &look.shape)] {
            if let Some(part) = part {
                text += &format!("{name} = {}\n", quoted(part));
            }
        }
    }
    let _ = fs::write(path(), text);
}

/// A project's color: the one chosen, or the one its name comes out as.
pub fn hue_of(look: &Look, name: &str) -> (String, u32) {
    let chosen = look.hue.as_deref().and_then(color_named).map(|(palette, at)| PALETTES[palette].1[at]);
    let chosen = chosen.map(|(named, color)| (look.hue.clone().unwrap_or(named.to_string()).replace(':', " "), color));
    let hash = name.trim().to_lowercase().bytes().fold(2166136261u32, |hash, byte| (hash ^ u32::from(byte)).wrapping_mul(16777619));
    chosen.unwrap_or_else(|| (HUES[hash as usize % HUES.len()].0.to_string(), HUES[hash as usize % HUES.len()].1))
}

/// The shape a tile is cut to: the one chosen, or one its name picks.
pub fn shape_of(look: &Look, name: &str) -> &'static str {
    let chosen = look.shape.as_deref().and_then(|shape| SHAPES.iter().find(|known| **known == shape));
    chosen.copied().unwrap_or(["circle", "squircle", "rounded"][number_of(name) as usize / GLYPHS.len() % 3])
}

pub fn fill_of(look: &Look) -> &'static str {
    let chosen = look.fill.as_deref().and_then(|fill| FILLS.iter().find(|known| **known == fill));
    chosen.copied().unwrap_or("gradient")
}

/// The drawing on a tile: the one chosen, or one its name picks.
pub fn glyph_of(look: &Look, name: &str) -> usize {
    let chosen = look.icon.as_deref().and_then(|icon| icon.strip_prefix("solar:")).and_then(|glyph| GLYPHS.iter().position(|known| *known == glyph));
    chosen.unwrap_or(number_of(name) as usize % GLYPHS.len())
}

/// A look that is none of your choosing: for when one is to be tried.
pub fn shuffled(seed: u32) -> Look {
    let pick = |from: &[&str], by: u32| Some(from[(seed / by) as usize % from.len()].to_string());
    // The fills that suit any drawing; the loud ones are for choosing.
    let fills = ["solid", "gradient", "glow", "dusk", "aurora", "holo", "night", "pastel", "tint"];
    Look {
        icon: Some(format!("solar:{}", GLYPHS[seed as usize % GLYPHS.len()])),
        hue: Some(HUES[(seed / 7) as usize % HUES.len()].0.to_string()),
        fill: pick(&fills, 151),
        shape: pick(&SHAPES[..11], 1009),
    }
}

/// A color as OKLCH has it: how light, how strong, and where round the wheel.
#[derive(Clone, Copy)]
struct Lch {
    l: f64,
    c: f64,
    h: f64,
}

fn lch(color: u32) -> Lch {
    let linear = |part: u32| {
        let part = f64::from(part & 0xFF) / 255.0;
        if part <= 0.04045 { part / 12.92 } else { ((part + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (linear(color >> 16), linear(color >> 8), linear(color));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let (light, a, b) = (
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    );
    Lch { l: light, c: a.hypot(b), h: b.atan2(a).to_degrees() }
}

fn rgb(color: Lch) -> u32 {
    let (a, b) = (color.c * color.h.to_radians().cos(), color.c * color.h.to_radians().sin());
    let l = (color.l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (color.l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (color.l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    let part = |linear: f64| {
        let linear = linear.clamp(0.0, 1.0);
        let part = if linear <= 0.0031308 { linear * 12.92 } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 };
        (part * 255.0).round() as u32
    };
    part(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s) << 16
        | part(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s) << 8
        | part(-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s)
}

/// The color, lighter or darker by `light`, `strong` times as strong, and `turn` degrees round.
fn turned(color: Lch, light: f64, strong: f64, turn: f64) -> Lch {
    Lch { l: (color.l + light).clamp(0.0, 1.0), c: color.c * strong, h: color.h + turn }
}

fn between(a: Lch, b: Lch, t: f64) -> Lch {
    let t = t.clamp(0.0, 1.0);
    Lch { l: a.l + (b.l - a.l) * t, c: a.c + (b.c - a.c) * t, h: a.h + (b.h - a.h) * t }
}

fn blend(a: u32, b: u32, t: f64) -> u32 {
    mix(a, b, (t.clamp(0.0, 1.0) * 256.0) as u32)
}

/// How far along a line at `angle`, as CSS reckons one, a point of the tile is: 0 to 1.
fn along(angle: f64, x: f64, y: f64) -> f64 {
    let (sin, cos) = angle.to_radians().sin_cos();
    ((x - 0.5) * sin - (y - 0.5) * cos) / (sin.abs() + cos.abs()) + 0.5
}

/// How far round a point is from the top, clockwise, in degrees.
fn around(x: f64, y: f64) -> f64 {
    (x - 0.5).atan2(0.5 - y).to_degrees().rem_euclid(360.0)
}

/// A glow of `color` from a point, gone at `reach` of the way to the farthest corner.
fn lit(under: u32, color: Lch, at: (f64, f64), reach: f64, x: f64, y: f64) -> u32 {
    let far = (at.0.max(1.0 - at.0)).hypot(at.1.max(1.0 - at.1));
    let gone = (x - at.0).hypot(y - at.1) / (far * reach);
    blend(under, rgb(color), 1.0 - gone)
}

/// What a tile filled with `fill` is at a point of it (0 to 1 each way),
/// and how solid: every fill is made from the one color.
fn ground(fill: &str, s: Lch, x: f64, y: f64) -> (u32, f64) {
    let deep = rgb(turned(s, -0.08, 1.0, 0.0));
    let solid = rgb(s);
    let color = match fill {
        "tint" => return (solid, 0.24),
        "pastel" => rgb(Lch { l: 0.93, c: s.c * 0.35, h: s.h }),
        "haze" => lit(rgb(turned(s, -0.05, 0.85, 0.0)), turned(s, 0.14, 0.55, 0.0), (0.5, 0.42), 1.0, x, y),
        // Lighter at the top than at the foot, as a phone's are.
        "gradient" => blend(rgb(turned(s, 0.1, 1.0, 0.0)), rgb(turned(s, -0.1, 1.0, 0.0)), along(165.0, x, y)),
        // From the color before it round the wheel to the one after.
        "dusk" => rgb(between(turned(s, 0.05, 1.0, -45.0), turned(s, -0.05, 1.0, 45.0), along(135.0, x, y))),
        // Three of its neighbors, each glowing from a corner.
        "aurora" => {
            let under = lit(solid, turned(s, -0.1, 1.0, 20.0), (0.4, 1.05), 0.6, x, y);
            let under = lit(under, turned(s, 0.0, 1.0, 60.0), (0.95, 0.3), 0.55, x, y);
            lit(under, turned(s, 0.1, 1.0, -70.0), (0.15, 0.1), 0.6, x, y)
        }
        // Lit from the top left, as a ball is.
        "glow" => lit(rgb(turned(s, -0.07, 1.0, 0.0)), turned(s, 0.16, 0.8, 0.0), (0.3, 0.22), 0.62, x, y),
        // Round the wheel a little way and back, about its middle.
        "holo" => {
            let (before, after) = (turned(s, 0.04, 1.0, -70.0), turned(s, 0.04, 1.0, 70.0));
            let stops = [before, s, after, s, before];
            let round = (around(x, y) - 210.0).rem_euclid(360.0) / 90.0;
            rgb(between(stops[round as usize % 4], stops[round as usize % 4 + 1], round.fract()))
        }
        "split" => if along(135.0, x, y) < 0.5 { rgb(turned(s, 0.05, 1.0, 0.0)) } else { deep },
        "stripes" => if (along(135.0, x, y) / 0.2).fract() < 0.5 { solid } else { deep },
        "rays" => if ((around(x, y) + 11.25) / 45.0).fract() < 0.5 { solid } else { deep },
        "rings" => if ((x - 0.5).hypot(y - 0.5) / 0.7071 / 0.2).fract() < 0.5 { solid } else { deep },
        "dots" => {
            let (across, down) = ((x / 0.25).fract() - 0.5, (y / 0.25).fract() - 0.5);
            if across.hypot(down) < 0.3 * 0.7071 { deep } else { solid }
        }
        "check" => if ((x / 0.2) as i32 + (y / 0.2) as i32) % 2 == 0 { solid } else { deep },
        // Dark, with the mark lit in the color.
        "night" => blend(rgb(Lch { l: 0.32, c: s.c * 0.5, h: s.h }), rgb(Lch { l: 0.17, c: s.c * 0.4, h: s.h }), along(165.0, x, y)),
        // No color at all: the box's own writing, turned over.
        "ink" => 0xEEEEEE,
        _ => solid,
    };
    (color, 1.0)
}

/// What is drawn on a tile of that fill is drawn in.
fn ink(fill: &str, s: Lch) -> u32 {
    match fill {
        "none" => rgb(s),
        "tint" => rgb(Lch { l: 0.84, c: s.c * 0.7, h: s.h }),
        "pastel" => rgb(Lch { l: 0.42, c: s.c, h: s.h }),
        "night" => rgb(Lch { l: 0.85, c: s.c * 0.75, h: s.h }),
        "ink" => 0x1E1E1E,
        // On a color light enough that white on it does not read: near black.
        _ if s.l > 0.76 => 0x1A1300,
        _ => 0xFFFFFF,
    }
}

/// A shape is how far its edge is from its middle, each way round.
type Edge = Box<dyn Fn(f64) -> f64>;

/// The same edge with its corners taken off: each point is the mean of those near it.
fn soft(edge: impl Fn(f64) -> f64 + 'static, over: f64) -> Edge {
    Box::new(move |a| (-4..=4).map(|step| edge(a + f64::from(step) / 4.0 * over)).sum::<f64>() / 9.0)
}

/// A figure of straight sides, a corner at the top.
fn sided(sides: f64, a: f64) -> f64 {
    let part = TAU / sides;
    (part / 2.0).cos() / ((a + PI / 2.0).rem_euclid(part) - part / 2.0).cos()
}

/// A star of so many points, as deep as `inner`.
fn star(points: f64, inner: f64, a: f64) -> f64 {
    let part = TAU / points;
    inner + (1.0 - inner) * ((a.rem_euclid(part) / part - 0.5).abs() * 2.0)
}

fn squircle(a: f64) -> f64 {
    (a.cos().abs().powi(5) + a.sin().abs().powi(5)).powf(-0.2)
}

fn square(a: f64) -> f64 {
    1.0 / a.cos().abs().max(a.sin().abs())
}

fn edge(shape: &str) -> Edge {
    match shape {
        "circle" => Box::new(|_| 1.0),
        "cookie" => Box::new(|a| 1.0 + 0.065 * (8.0 * a).cos()),
        "clover" => Box::new(|a| 1.0 + 0.2 * (4.0 * (a - PI / 4.0)).cos()),
        "burst" => soft(|a| star(12.0, 0.82, a), 0.09),
        "hexagon" => soft(|a| sided(6.0, a), 0.2),
        // A squircle with one corner left square: the corner at the foot, on the right.
        "drop" => soft(|a| if a.cos() > 0.0 && a.sin() > 0.0 { square(a) } else { squircle(a) }, 0.05),
        "diamond" => soft(|a| sided(4.0, a), 0.22),
        "triangle" => soft(|a| sided(3.0, a), 0.3),
        "pentagon" => soft(|a| sided(5.0, a), 0.2),
        "octagon" => soft(|a| sided(8.0, a + PI / 8.0), 0.1),
        "star" => soft(|a| star(5.0, 0.5, a + PI / 2.0), 0.14),
        "sun" => soft(|a| star(16.0, 0.84, a), 0.05),
        "flower" => Box::new(|a| 1.0 - 0.17 * (6.0 * a).cos()),
        // A squircle with its edge cut as a stamp's is.
        "stamp" => Box::new(|a| squircle(a) * (1.0 + 0.035 * (14.0 * a).cos())),
        // Round over the top and square at the foot.
        "arch" => soft(|a| if a.sin() > 0.0 { square(a) } else { 1.0 }, 0.05),
        // Round at two corners and square at the two across from each other.
        "leaf" => soft(|a| if a.cos() * a.sin() > 0.0 { square(a) } else { 1.0 }, 0.05),
        "oval" => Box::new(|a| 1.0 / (a.cos() / 0.76).hypot(a.sin())),
        "heart" => soft(|a| 2.0 + 2.0 * a.sin() - a.sin() * a.cos().abs().sqrt() / (1.4 - a.sin()), 0.04),
        "blob" => Box::new(|a| 1.0 + 0.09 * (2.0 * a + 0.6).sin() + 0.06 * (3.0 * a + 2.0).sin() + 0.03 * (5.0 * a).sin()),
        // A square with its corners well rounded.
        "rounded" => Box::new(|a| {
            let (x, y) = (a.cos().abs(), a.sin().abs());
            // Along a side until the corner's arc is come to, then along that.
            let (corner, far) = (0.44, square(a));
            let (px, py) = (x * far, y * far);
            if px.min(py) <= 1.0 - corner {
                return far;
            }
            let middle = 1.0 - corner;
            let toward = middle * (x + y);
            toward + (toward * toward - (2.0 * middle * middle - corner * corner)).max(0.0).sqrt()
        }),
        _ => Box::new(squircle),
    }
}

/// How much of each pixel of a square of `side` is within the shape,
/// which fills the square as far as it can without being pulled out of true.
fn cut(shape: &str, side: usize) -> Vec<f64> {
    let edge = edge(shape);
    // Where the shape lies about its own middle, to put it in the middle of the square.
    let points: Vec<(f64, f64)> = (0..96).map(|at| f64::from(at) / 96.0 * TAU).map(|a| (edge(a) * a.cos(), edge(a) * a.sin())).collect();
    let span = |of: fn(&(f64, f64)) -> f64| {
        let (low, high) = points.iter().map(of).fold((f64::MAX, f64::MIN), |(low, high), at| (low.min(at), high.max(at)));
        ((low + high) / 2.0, (high - low) / 2.0)
    };
    let ((middle_x, half_x), (middle_y, half_y)) = (span(|point| point.0), span(|point| point.1));
    let half = half_x.max(half_y);
    let parts = [1.0 / 6.0, 0.5, 5.0 / 6.0];
    let within = |x: f64, y: f64| {
        let (x, y) = (middle_x + (x / side as f64 * 2.0 - 1.0) * half, middle_y + (y / side as f64 * 2.0 - 1.0) * half);
        x.hypot(y) <= edge(y.atan2(x))
    };
    let cover = |at: usize| {
        let (x, y) = ((at % side) as f64, (at / side) as f64);
        parts.iter().flat_map(|dy| parts.map(|dx| within(x + dx, y + dy))).filter(|inside| *inside).count() as f64 / 9.0
    };
    (0..side * side).map(cover).collect()
}

/// Draws `coverage`, a letter as fontdue made it, in `color` at `strength`/256.
fn stamp(pixels: &mut [u32], side: usize, (left, top): (i32, i32), metrics: &fontdue::Metrics, coverage: &[u8], color: u32, strength: u32) {
    for (row, line) in coverage.chunks_exact(metrics.width.max(1)).enumerate() {
        for (column, &amount) in line.iter().enumerate() {
            let (x, y) = (left + column as i32, top + row as i32);
            if amount == 0 || x < 0 || y < 0 || x as usize >= side || y as usize >= side {
                continue;
            }
            let pixel = &mut pixels[y as usize * side + x as usize];
            let amount = u32::from(amount) * strength / 255;
            // On nothing, it is itself and as solid as it was drawn.
            let under = *pixel >> 24;
            let solid = under + (255 - under) * amount / 256;
            let color = if under == 0 { color } else { mix(*pixel & 0xFF_FFFF, color, amount * 255 / solid.max(1)) };
            *pixel = solid << 24 | color;
        }
    }
}

/// A project's tile, drawn: a square of `side`. `own` is a picture to go
/// on it, the folder's or one you chose, where it is to show one.
pub fn render(look: &Look, name: &str, own: Option<&Icon>, side: usize) -> Icon {
    let (_, solid) = hue_of(look, name);
    let (fill, shape, s) = (fill_of(look), shape_of(look, name), lch(solid));
    let tiled = fill != "none";
    let cover = if tiled || own.is_some() { cut(shape, side) } else { Vec::new() };
    let mut pixels = vec![0u32; side * side];

    // A picture has colors of its own: it takes only the shape.
    if let Some(own) = own {
        let own = fitted_to(own, side);
        for (at, pixel) in pixels.iter_mut().enumerate() {
            let from = own.pixels[at];
            let alpha = (f64::from(from >> 24) * cover[at]) as u32;
            *pixel = if alpha == 0 { 0 } else { alpha << 24 | from & 0xFF_FFFF };
        }
        return Icon { width: side, height: side, pixels };
    }

    if tiled {
        for (at, pixel) in pixels.iter_mut().enumerate() {
            let (color, solid) = ground(fill, s, ((at % side) as f64 + 0.5) / side as f64, ((at / side) as f64 + 0.5) / side as f64);
            let alpha = (cover[at] * solid * 255.0).round() as u32;
            *pixel = if alpha == 0 { 0 } else { alpha << 24 | color };
        }
    }

    // An emoji keeps its own colors, on the tile or on nothing.
    if let Some(picture) = look.icon.as_deref().and_then(|icon| icon.strip_prefix("emoji:")).and_then(emoji::picture) {
        let inner = (side as f64 * if tiled { 0.62 } else { 0.92 }).round() as usize;
        let small = fitted_to(&picture, inner.max(1));
        let from = (side - inner) / 2;
        for (at, color) in small.pixels.iter().enumerate() {
            let amount = color >> 24;
            if amount == 0 {
                continue;
            }
            let pixel = &mut pixels[(from + at / inner) * side + from + at % inner];
            let under = *pixel >> 24;
            let solid = under + (255 - under) * amount / 255;
            let mixed = if under == 0 { color & 0xFF_FFFF } else { mix(*pixel & 0xFF_FFFF, color & 0xFF_FFFF, amount * 256 / solid.max(1)) };
            *pixel = solid << 24 | mixed;
        }
        return Icon { width: side, height: side, pixels };
    }

    let ink = ink(fill, s);
    let settings = fontdue::FontSettings::default();
    if let Some(letters) = look.icon.as_deref().and_then(|icon| icon.strip_prefix("letters:")) {
        // Letters, heavy enough at this size to stand for it.
        let Ok(font) = fontdue::Font::from_bytes(GEIST, settings) else { return Icon { width: side, height: side, pixels } };
        let size = side as f32 * if tiled { 0.42 } else { 0.56 };
        let letters: Vec<char> = letters.to_uppercase().chars().take(2).collect();
        let across: f32 = letters.iter().map(|letter| font.metrics(*letter, size).advance_width).sum();
        let mut pen = (side as f32 - across) / 2.0;
        let foot = side as f32 / 2.0 + size * 0.36;
        for letter in letters {
            let (metrics, coverage) = font.rasterize(letter, size);
            let at = (pen.round() as i32 + metrics.xmin, foot.round() as i32 - metrics.ymin - metrics.height as i32);
            stamp(&mut pixels, side, at, &metrics, &coverage, ink, 256);
            pen += metrics.advance_width;
        }
    } else if let Ok(font) = fontdue::Font::from_bytes(include_bytes!("../../assets/solar-duotone.ttf") as &[u8], settings) {
        // The drawing fills its font's square, which is put in the middle:
        // the tone Solar draws at half strength first, the full one over it.
        let size = side as f32 * if tiled { 0.6 } else { 0.86 };
        let from = (side as f32 - size) / 2.0;
        let (full, faint) = glyph_letters(glyph_of(look, name));
        for (letter, strength) in [(faint, 140), (full, 256)] {
            let (metrics, coverage) = font.rasterize(letter, size);
            let at = (from.round() as i32 + metrics.xmin, (from + size).round() as i32 - metrics.ymin - metrics.height as i32);
            stamp(&mut pixels, side, at, &metrics, &coverage, ink, strength);
        }
    }
    Icon { width: side, height: side, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_color_there_and_back() {
        for (_, color) in HUES {
            let back = rgb(lch(color));
            let apart = |shift: u32| ((color >> shift & 0xFF) as i32 - (back >> shift & 0xFF) as i32).abs();
            assert!(apart(16) <= 1 && apart(8) <= 1 && apart(0) <= 1, "{color:06x} came back {back:06x}");
        }
    }

    #[test]
    fn every_shape_holds_its_middle_and_not_its_corner() {
        for shape in SHAPES {
            let cover = cut(shape, 32);
            assert_eq!(cover[16 * 32 + 16], 1.0, "{shape}");
            // (A leaf is square at that corner, and the one across from it.)
            assert_eq!(cover[0] == 0.0, shape != "leaf", "{shape}");
        }
    }

    #[test]
    fn a_look_that_is_drawn() {
        let look = Look { icon: Some("solar:rocket".into()), hue: Some("blue".into()), fill: Some("solid".into()), shape: Some("circle".into()) };
        let tile = render(&look, "jog", None, 64);
        // Clear at the corner; at the edge, the color itself.
        assert_eq!(tile.pixels[0], 0);
        assert_eq!(tile.pixels[32 * 64 + 2] >> 24, 255);
        assert!(tile.pixels[32 * 64 + 2] & 0xFF > 0xF0);
        assert_eq!(glyph_of(&look, "jog"), 0);
        // With nothing said, the name picks, and the same every time.
        assert_eq!(render(&Look::default(), "jog", None, 32).pixels, render(&Look::default(), "jog", None, 32).pixels);
        for fill in FILLS {
            let look = Look { fill: Some(fill.into()), ..Look::default() };
            assert_eq!(render(&look, "jog", None, 32).pixels.len(), 32 * 32, "{fill}");
        }
    }
}
