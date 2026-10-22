//! Emoji: the list of them, by group and name, as Unicode gives it, and
//! the picture of each, as the color emoji font has it. A color font's
//! letters are pictures, which the font renderer cannot draw: Noto Color
//! Emoji keeps each as a PNG, which is read out of the font here.
//!
//! The list, and the font for a system that has none of its own, come
//! with clutch: both are of Emoji 16.0.

use super::apps;
use super::canvas::Icon;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::sync::OnceLock;

/// Unicode's list of them, emoji-test.txt.
const LIST: &str = include_str!("../../assets/emoji-test.txt");
/// The system's own font, which may be newer than the one clutch has.
const FONTS: &[&str] = &["/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf", "/usr/share/fonts/noto/NotoColorEmoji.ttf", "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf"];
const BUNDLED: &[u8] = include_bytes!("../../assets/NotoColorEmoji.ttf");

pub struct Emoji {
    pub text: String,
    pub name: String,
    pub group: String,
}

/// Every emoji that is one letter and the font has a picture of, in
/// Unicode's order: the smileys first.
pub fn all() -> &'static [Emoji] {
    static ALL: OnceLock<Vec<Emoji>> = OnceLock::new();
    ALL.get_or_init(|| {
        let Some(font) = font() else { return Vec::new() };
        let has = |emoji: &&Emoji| emoji.text.chars().next().is_some_and(|c| font.glyphs.contains_key(&(c as u32)));
        listed().iter().filter(has).map(|emoji| Emoji { text: emoji.text.clone(), name: emoji.name.clone(), group: emoji.group.clone() }).collect()
    })
}

/// The name of an emoji, from the list alone: the font, which is large,
/// is not read to say what a copy was.
pub fn name_of(text: &str) -> Option<&'static str> {
    listed().iter().find(|emoji| emoji.text == text).map(|emoji| emoji.name.as_str())
}

/// Every emoji that is one letter, as Unicode lists them, whether or
/// not the font has a picture of it.
fn listed() -> &'static [Emoji] {
    static LISTED: OnceLock<Vec<Emoji>> = OnceLock::new();
    LISTED.get_or_init(|| {
        let text = LIST;
        let mut group = String::new();
        let mut all = Vec::new();
        for line in text.lines() {
            if let Some(named) = line.strip_prefix("# group: ") {
                group = named.trim().to_string();
                continue;
            }
            // "1F600 ; fully-qualified # 😀 E1.0 grinning face"
            let Some((points, rest)) = line.split_once(';') else { continue };
            if !rest.trim_start().starts_with("fully-qualified") {
                continue;
            }
            let points: Vec<u32> = points.split_whitespace().filter_map(|point| u32::from_str_radix(point, 16).ok()).filter(|point| *point != 0xFE0F).collect();
            let [point] = points[..] else { continue };
            let name = rest.split_once('#').map_or("", |(_, said)| said).split_whitespace().skip(2).collect::<Vec<_>>().join(" ");
            let text = char::from_u32(point).map(String::from).unwrap_or_default();
            all.push(Emoji { text, name, group: group.clone() });
        }
        all
    })
}

/// The picture of an emoji, as the font has it.
pub fn picture(text: &str) -> Option<Icon> {
    let font = font()?;
    let point = text.chars().find(|c| *c != '\u{fe0f}')? as u32;
    let (start, end) = *font.glyphs.get(&point)?;
    apps::decode(std::io::Cursor::new(font.bytes.get(start..end)?))
}

/// The font, read once: where in it each emoji's PNG is.
struct Font {
    bytes: Cow<'static, [u8]>,
    glyphs: HashMap<u32, (usize, usize)>,
}

/// The system's font, or failing that clutch's own.
fn font() -> Option<&'static Font> {
    static FONT: OnceLock<Option<Font>> = OnceLock::new();
    FONT.get_or_init(|| FONTS.iter().find_map(|path| read(Cow::Owned(fs::read(path).ok()?))).or_else(|| read(Cow::Borrowed(BUNDLED)))).as_ref()
}

fn read(bytes: Cow<'static, [u8]>) -> Option<Font> {
    let u16_at = |at: usize| Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    // Where each of the font's tables is.
    let table = |tag: &[u8]| -> Option<usize> {
        (0..u16_at(4)? as usize).find_map(|at| {
            let record = 12 + at * 16;
            (bytes.get(record..record + 4)? == tag).then(|| u32_at(record + 8).map(|offset| offset as usize)).flatten()
        })
    };
    let (cmap, cblc, cbdt) = (table(b"cmap")?, table(b"CBLC")?, table(b"CBDT")?);

    // Which glyph each letter is: the cmap of format 12, for all of Unicode.
    let mut glyph_of = HashMap::new();
    for at in 0..u16_at(cmap + 2)? as usize {
        let sub = cmap + u32_at(cmap + 4 + at * 8 + 4)? as usize;
        if u16_at(sub)? != 12 {
            continue;
        }
        for group in 0..u32_at(sub + 12)? as usize {
            let record = sub + 16 + group * 12;
            let (first, last, glyph) = (u32_at(record)?, u32_at(record + 4)?, u32_at(record + 8)?);
            for point in first..=last {
                glyph_of.insert(point, (glyph + point - first) as u16);
            }
        }
        break;
    }

    // Where each glyph's picture is, at the first size the font has.
    let mut place = HashMap::new();
    let size = cblc + 8;
    let (array, count) = (cblc + u32_at(size)? as usize, u32_at(size + 8)? as usize);
    for at in 0..count {
        let entry = array + at * 8;
        let (first, last) = (u16_at(entry)?, u16_at(entry + 2)?);
        let sub = array + u32_at(entry + 4)? as usize;
        let (format, image, data) = (u16_at(sub)?, u16_at(sub + 2)?, cbdt + u32_at(sub + 4)? as usize);
        // Before the PNG: its size and how it sits, in a form the image's format says.
        let skip = match image {
            17 => 5 + 4,
            18 => 8 + 4,
            19 => 4,
            _ => continue,
        };
        let mut put = |glyph: u16, from: usize, to: usize| {
            if to > from + skip {
                place.insert(glyph, (data + from + skip, data + to));
            }
        };
        match format {
            1 | 3 => {
                let wide = if format == 1 { 4 } else { 2 };
                let offset = |at: usize| if wide == 4 { u32_at(sub + 8 + at * 4).map(|at| at as usize) } else { u16_at(sub + 8 + at * 2).map(usize::from) };
                for glyph in first..=last {
                    let at = (glyph - first) as usize;
                    put(glyph, offset(at)?, offset(at + 1)?);
                }
            }
            4 => {
                let pairs = u32_at(sub + 8)? as usize;
                for at in 0..pairs {
                    let pair = sub + 12 + at * 4;
                    put(u16_at(pair)?, u16_at(pair + 2)? as usize, u16_at(pair + 6)? as usize);
                }
            }
            _ => {}
        }
    }
    let glyphs = glyph_of.into_iter().filter_map(|(point, glyph)| Some((point, *place.get(&glyph)?))).collect();
    Some(Font { bytes, glyphs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_emoji_has_a_picture() {
        let smile = all().iter().find(|emoji| emoji.name == "grinning face").unwrap();
        assert_eq!(smile.group, "Smileys & Emotion");
        let picture = picture(&smile.text).unwrap();
        assert!(picture.width > 64 && picture.pixels.iter().any(|pixel| pixel >> 24 == 255));
    }

    #[test]
    fn clutch_has_a_font_of_its_own_for_every_emoji_it_lists() {
        // As on a system with no emoji font: nearly all of Unicode's
        // list is drawn by the one that comes with clutch.
        let own = read(Cow::Borrowed(BUNDLED)).expect("the font clutch has");
        let listed = listed();
        let drawn = listed.iter().filter(|emoji| emoji.text.chars().next().is_some_and(|c| own.glyphs.contains_key(&(c as u32)))).count();
        assert!(listed.len() > 1300 && drawn * 100 >= listed.len() * 99, "{drawn} of {}", listed.len());
        let (start, end) = own.glyphs[&0x1F600];
        let smile = apps::decode(std::io::Cursor::new(&own.bytes[start..end])).unwrap();
        assert!(smile.width > 64);
    }
}
