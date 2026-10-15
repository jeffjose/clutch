//! Drawing into the window's pixels: boxes with rounded corners, text,
//! icons. The same primitives, font and icons as jog.

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::mpsc;

/// Drawing primitives over a window buffer. Everything is clipped.
pub struct Canvas<'a> {
    pixels: &'a mut [u32],
    pub width: i32,
    pub height: i32,
}

/// A small picture with its own transparency: an application's icon.
pub struct Icon {
    pub width: usize,
    pub height: usize,
    /// One 0xAARRGGBB to a pixel.
    pub pixels: Vec<u32>,
}

impl<'a> Canvas<'a> {
    pub fn new(pixels: &'a mut [u32], width: u32, height: u32) -> Self {
        Canvas { pixels, width: width as i32, height: height as i32 }
    }

    /// Rows of the part of a rectangle that is on the canvas.
    fn rows(&mut self, x: i32, y: i32, w: i32, h: i32) -> impl Iterator<Item = &mut [u32]> {
        let (x0, x1) = (x.clamp(0, self.width) as usize, (x + w).clamp(0, self.width) as usize);
        let (y0, y1) = (y.clamp(0, self.height) as usize, (y + h).clamp(0, self.height) as usize);
        let rows = self.pixels.chunks_exact_mut(self.width.max(1) as usize);
        rows.skip(y0).take(y1.saturating_sub(y0)).map(move |row| &mut row[x0..x1.max(x0)])
    }

    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        self.rows(x, y, w, h).for_each(|row| row.fill(color));
    }

    /// A filled rectangle with rounded corners, `amount`/256 opaque.
    #[allow(clippy::too_many_arguments)]
    pub fn round_fill(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, color: u32, amount: u32) {
        let radius = radius.clamp(0, w.min(h) / 2);
        let solid = amount >= 256;
        for py in y..y + h {
            // Only the ends of the rows at the top and bottom curve; the
            // rest of every row is plain.
            let curved = py < y + radius || py >= y + h - radius;
            let inset = if curved { radius } else { 0 };
            for px in (x..x + inset).chain(x + w - inset..x + w) {
                let cover = rounded_cover(px, py, x, y, w, h, radius, 0.0);
                self.cover(px, py, color, (cover * f64::from(amount)) as u32);
            }
            // (Solid, the row is set rather than mixed: the panel is the
            // whole window, and would otherwise be a million mixes a frame.)
            match solid {
                true => self.fill(x + inset, py, w - 2 * inset, 1, color),
                false => self.rows(x + inset, py, w - 2 * inset, 1).for_each(|row| row.iter_mut().for_each(|pixel| *pixel = mix(*pixel, color, amount))),
            }
        }
    }

    /// Makes what is drawn a window with rounded corners, where the
    /// window's pixels can be seen through: every pixel solid, but those
    /// outside the curve of a corner clear, and those on it part-way.
    /// The panel must have been drawn rounded on a ground of nothing, so
    /// that the colors along the curve are already thinned to match.
    pub fn round_window(&mut self, radius: i32) {
        self.pixels.iter_mut().for_each(|pixel| *pixel |= 0xFF00_0000);
        let (width, height) = (self.width, self.height);
        for (corner_x, corner_y) in [(0, 0), (width - radius, 0), (0, height - radius), (width - radius, height - radius)] {
            for y in corner_y.max(0)..(corner_y + radius).min(height) {
                for x in corner_x.max(0)..(corner_x + radius).min(width) {
                    let solid = (rounded_cover(x, y, 0, 0, width, height, radius, 0.0) * 255.0).round() as u32;
                    let pixel = &mut self.pixels[(y * width + x) as usize];
                    *pixel = if solid == 0 { 0 } else { *pixel & 0x00FF_FFFF | solid << 24 };
                }
            }
        }
    }

    fn cover(&mut self, x: i32, y: i32, color: u32, amount: u32) {
        if amount > 0 && x >= 0 && y >= 0 && x < self.width && y < self.height {
            let pixel = &mut self.pixels[(y * self.width + x) as usize];
            *pixel = mix(*pixel, color, amount.min(256));
        }
    }

    fn blend(&mut self, x: i32, y: i32, color: u32, coverage: u8) {
        self.cover(x, y, color, u32::from(coverage) + u32::from(coverage >> 7));
    }

    /// Draws an icon to fill a square of side `size`, over what is there.
    /// Each pixel drawn is the average of the part of the icon it covers,
    /// which is what keeps a large icon made small from looking torn.
    pub fn icon(&mut self, icon: &Icon, x: i32, y: i32, size: i32) {
        if size <= 0 || icon.width == 0 || icon.height == 0 || icon.pixels.len() < icon.width * icon.height {
            return;
        }
        let size = size as usize;
        for row in 0..size {
            let (top, bottom) = (row * icon.height / size, ((row + 1) * icon.height).div_ceil(size).max(row * icon.height / size + 1));
            for column in 0..size {
                let (left, right) = (column * icon.width / size, ((column + 1) * icon.width).div_ceil(size).max(column * icon.width / size + 1));
                // Colors are weighed by how opaque they are, or the
                // unseen color of a clear pixel would bleed in.
                let (mut a, mut r, mut g, mut b, mut count) = (0u64, 0u64, 0u64, 0u64, 0u64);
                for sy in top..bottom.min(icon.height) {
                    for pixel in &icon.pixels[sy * icon.width + left..sy * icon.width + right.min(icon.width)] {
                        let alpha = u64::from(pixel >> 24);
                        a += alpha;
                        r += u64::from(pixel >> 16 & 0xFF) * alpha;
                        g += u64::from(pixel >> 8 & 0xFF) * alpha;
                        b += u64::from(pixel & 0xFF) * alpha;
                        count += 1;
                    }
                }
                if a == 0 || count == 0 {
                    continue;
                }
                let color = ((r / a) << 16 | (g / a) << 8 | (b / a)) as u32;
                let alpha = (a / count) as u32;
                self.cover(x + column as i32, y + row as i32, color, alpha + (alpha >> 7));
            }
        }
    }
}

/// How much of the pixel at `(px, py)`, from 0 to 1, lies within a
/// rectangle with corners of `radius`, shrunk by `inset` on every side.
/// Fractions along the edge are what make the curve smooth.
#[allow(clippy::too_many_arguments)]
fn rounded_cover(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32, radius: i32, inset: f64) -> f64 {
    let (half_w, half_h) = (f64::from(w) / 2.0 - inset, f64::from(h) / 2.0 - inset);
    let radius = (f64::from(radius) - inset).clamp(0.0, half_w.min(half_h).max(0.0));
    // Distance from the middle, less the straight part of each side.
    let across = (f64::from(px) + 0.5 - f64::from(x) - f64::from(w) / 2.0).abs() - (half_w - radius);
    let down = (f64::from(py) + 0.5 - f64::from(y) - f64::from(h) / 2.0).abs() - (half_h - radius);
    let outside = across.max(0.0).hypot(down.max(0.0)) + across.max(down).min(0.0) - radius;
    (0.5 - outside).clamp(0.0, 1.0)
}

/// Blends `amount`/256 of `b` into `a`, two channels at a time.
pub fn mix(a: u32, b: u32, amount: u32) -> u32 {
    let lerp = |mask: u32| (((a & mask) * (256 - amount) + (b & mask) * amount) >> 8) & mask;
    lerp(0xFF00FF) | lerp(0x00FF00)
}

/// The font the box is set in.
pub const GEIST: &[u8] = include_bytes!("../../assets/Geist-Regular.ttf");

/// Which font the icons are, where the others are counted from 0.
const ICONS: usize = usize::MAX;
/// And the drawings in two tones, which have a font and a part of Unicode
/// of their own: `duo` says which letters a drawing is.
const DRAWINGS: usize = usize::MAX - 1;
const DRAWINGS_FROM: u32 = 0xF0000;

/// The two letters that draw one of Solar's two-tone drawings: the full
/// tone, and the one drawn at half strength.
pub fn duo(drawing: usize) -> (char, char) {
    let letter = |tone: u32| char::from_u32(DRAWINGS_FROM + 2 * drawing as u32 + tone).unwrap_or(' ');
    (letter(0), letter(1))
}

/// Text rendering with the font clutch carries, and the icons, which are
/// letters of a font of their own.
pub struct Text {
    font: Option<fontdue::Font>,
    icons: Option<fontdue::Font>,
    /// The two-tone drawings, read the first time one is drawn.
    drawings: Option<fontdue::Font>,
    /// Fonts for the letters the main one lacks, Korean say, each found
    /// through fontconfig the first time such a letter comes up. A font
    /// for a large script takes a while to load, so that happens on
    /// another thread; until it is in, the letter is not drawn.
    others: Vec<fontdue::Font>,
    loading: Option<mpsc::Receiver<Option<fontdue::Font>>>,
    /// Letters a font has been looked for, so each is only asked about once.
    sought: HashSet<char>,
    glyphs: HashMap<(char, u32), (fontdue::Metrics, Vec<u8>)>,
}

impl Text {
    pub fn load() -> Self {
        let load = |bytes: &[u8]| fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok();
        let font = load(GEIST);
        let icons = load(include_bytes!("../../assets/solar.ttf"));
        Text { font, icons, ..Text::empty() }
    }

    /// No fonts at all, which draws nothing: for a box whose fonts are
    /// still being read.
    pub fn empty() -> Self {
        Text { font: None, icons: None, drawings: None, others: Vec::new(), loading: None, sought: HashSet::new(), glyphs: HashMap::new() }
    }

    /// Draw the letters the box is set in ready, at the size of its text
    /// and of its lesser words, so that the first frame finds them drawn.
    pub fn warm(&mut self, px: u32) {
        let letters: Vec<char> = (' '..='~').chain("\u{2026}\u{b7}\u{203a}\u{2190}\u{2192}".chars()).collect();
        for px in [px, px * 5 / 6] {
            for c in &letters {
                self.glyph(*c, px);
            }
        }
    }

    /// The letter drawn at a size, once: drawn the first time it is wanted.
    fn glyph(&mut self, c: char, px: u32) -> Option<&(fontdue::Metrics, Vec<u8>)> {
        if !self.glyphs.contains_key(&(c, px)) {
            let which = self.font_with(c);
            let font = self.font(which)?;
            let glyph = font.rasterize(Self::letter(which, c), Self::size(which, px));
            self.glyphs.insert((c, px), glyph);
        }
        self.glyphs.get(&(c, px))
    }

    pub fn line_height(px: u32) -> i32 {
        (px * 4 / 3) as i32
    }

    /// Whether a font is still on its way, after which text drawn in the
    /// meantime will want drawing again.
    pub fn waiting(&self) -> bool {
        self.loading.is_some()
    }

    /// Which font has letter `c`: 0 for the main one, otherwise one of the
    /// others, counted from 1. Looks for a font that has it if none does.
    fn font_with(&mut self, c: char) -> usize {
        // The icons have a font, and a part of Unicode, to themselves.
        if ('\u{e000}'..='\u{f8ff}').contains(&c) {
            return ICONS;
        }
        if c as u32 >= DRAWINGS_FROM {
            if self.drawings.is_none() {
                let font = include_bytes!("../../assets/solar-duotone.ttf") as &[u8];
                self.drawings = fontdue::Font::from_bytes(font, fontdue::FontSettings::default()).ok();
            }
            return DRAWINGS;
        }
        let has = |font: &fontdue::Font| font.lookup_glyph_index(c) != 0;
        if self.font.as_ref().is_none_or(has) || c.is_control() {
            return 0;
        }
        if let Some(Ok(found)) = self.loading.as_ref().map(mpsc::Receiver::try_recv) {
            self.loading = None;
            self.others.extend(found);
            // What was left out can now be drawn.
            self.glyphs.clear();
        }
        if let Some(at) = self.others.iter().position(has) {
            return at + 1;
        }
        if self.loading.is_none() && self.sought.insert(c) {
            let (sender, receiver) = mpsc::channel();
            self.loading = Some(receiver);
            std::thread::spawn(move || drop(sender.send(font_for(c))));
        }
        0
    }

    fn font(&self, which: usize) -> Option<&fontdue::Font> {
        match which {
            0 => self.font.as_ref(),
            ICONS => self.icons.as_ref(),
            DRAWINGS => self.drawings.as_ref(),
            other => self.others.get(other - 1),
        }
    }

    /// The letter as its font knows it: the drawings are kept, in theirs,
    /// where the icons are in the icons'.
    fn letter(which: usize, c: char) -> char {
        if which == DRAWINGS { char::from_u32(c as u32 - DRAWINGS_FROM + 0xE000).unwrap_or(c) } else { c }
    }

    /// The size a letter is drawn at: icons a little larger than the text
    /// they sit in, which they would otherwise look small beside.
    fn size(which: usize, px: u32) -> f32 {
        if which == ICONS { px as f32 * 1.15 } else { px as f32 }
    }

    pub fn width(&mut self, text: &str, px: u32) -> i32 {
        let mut width = 0.0;
        for c in text.chars() {
            let which = self.font_with(c);
            width += self.font(which).map_or(0.0, |font| font.metrics(Self::letter(which, c), Self::size(which, px)).advance_width);
        }
        width.ceil() as i32
    }

    /// `line`, or as much of it as fits in `width` with a mark that it
    /// was cut.
    pub fn clipped(&mut self, line: &str, width: i32, px: u32) -> String {
        if self.width(line, px) <= width {
            return line.to_string();
        }
        // Each letter measured once: the mark fits after the longest
        // beginning it leaves room for.
        let mark = self.width("\u{2026}", px) as f32;
        let mut across = 0.0;
        let mut kept = String::new();
        for c in line.chars() {
            let which = self.font_with(c);
            across += self.font(which).map_or(0.0, |font| font.metrics(Self::letter(which, c), Self::size(which, px)).advance_width);
            if (across + mark).ceil() as i32 > width {
                break;
            }
            kept.push(c);
        }
        kept.push('\u{2026}');
        kept
    }

    /// Draws one line of text whose line box has its top-left at `(x, y)`.
    pub fn draw(&mut self, canvas: &mut Canvas, x: i32, y: i32, px: u32, color: u32, text: &str) {
        let Some(font) = &self.font else { return };
        let (ascent, descent) = font
            .horizontal_line_metrics(px as f32)
            .map_or((px as f32 * 0.8, px as f32 * -0.2), |m| (m.ascent, m.descent));
        let baseline = y + ((Self::line_height(px) as f32 + ascent + descent) / 2.0).round() as i32;

        let mut pen = x as f32;
        for c in text.chars() {
            let Some((metrics, coverage)) = self.glyph(c, px) else { continue };
            let left = pen.round() as i32 + metrics.xmin;
            // An icon sits in the middle of the line, whatever its font
            // takes for a baseline; letters stand on the text's.
            let icon = ('\u{e000}'..='\u{f8ff}').contains(&c);
            let top = match icon {
                // A drawing's two tones are placed by the square both are
                // drawn in, or one would sit askew of the other.
                _ if c as u32 >= DRAWINGS_FROM => y + (Self::line_height(px) + px as i32) / 2 - metrics.ymin - metrics.height as i32,
                true => y + (Self::line_height(px) - metrics.height as i32) / 2,
                false => baseline - metrics.ymin - metrics.height as i32,
            };
            for (row, line) in coverage.chunks_exact(metrics.width.max(1)).enumerate() {
                for (column, &amount) in line.iter().enumerate() {
                    if amount > 0 {
                        canvas.blend(left + column as i32, top + row as i32, color, amount);
                    }
                }
            }
            pen += metrics.advance_width;
        }
    }
}

/// A font that has letter `c`, as fontconfig picks one. The file may hold
/// several fonts, in which case fontconfig says which.
fn font_for(c: char) -> Option<fontdue::Font> {
    let pattern = format!("sans:charset={:x}", c as u32);
    let output = Command::new("fc-match").args(["-f", "%{file}\n%{index}", &pattern]).output().ok()?;
    let answer = String::from_utf8(output.stdout).ok()?;
    let (file, index) = answer.split_once('\n')?;
    let settings = fontdue::FontSettings { collection_index: index.trim().parse().unwrap_or(0), ..Default::default() };
    let font = fontdue::Font::from_bytes(std::fs::read(file).ok()?, settings).ok()?;
    (font.lookup_glyph_index(c) != 0).then_some(font)
}
