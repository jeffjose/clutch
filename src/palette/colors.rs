//! The palettes of design systems, to copy a color from: Tailwind's,
//! Radix's, Material's and Chakra's. Each is its hues, and of each hue a
//! color for every shade, from light to dark. They are carried, in
//! `assets/palettes.json`, which `tools/palettes.py` makes.

use serde::Deserialize;
use std::sync::OnceLock;

pub struct Palette {
    pub id: String,
    pub name: String,
    pub shades: Vec<String>,
    /// The shade the palette means by a hue alone: Tailwind's 500, Radix's 9.
    pub main: usize,
    pub hues: Vec<(String, Vec<u32>)>,
}

#[derive(Deserialize)]
struct Written {
    id: String,
    name: String,
    shades: Vec<String>,
    main: usize,
    hues: Vec<(String, Vec<String>)>,
}

pub fn all() -> &'static [Palette] {
    static ALL: OnceLock<Vec<Palette>> = OnceLock::new();
    ALL.get_or_init(|| {
        let written: Vec<Written> = serde_json::from_str(include_str!("../../assets/palettes.json")).unwrap_or_default();
        written
            .into_iter()
            .map(|palette| {
                let hues = palette.hues.into_iter().map(|(name, colors)| (name, colors.iter().filter_map(|color| parsed(color)).collect())).collect();
                Palette { id: palette.id, name: palette.name, shades: palette.shades, main: palette.main, hues }
            })
            .collect()
    })
}

/// A color, by its palette, hue and shade.
pub fn of(palette: usize, hue: usize, shade: usize) -> u32 {
    all()[palette].hues[hue].1[shade]
}

/// What it is called: "blue-500".
pub fn name_of(palette: usize, hue: usize, shade: usize) -> String {
    let palette = &all()[palette];
    format!("{}-{}", palette.hues[hue].0, palette.shades[shade])
}

/// "#3b82f6", or "3b82f6", as a color.
pub fn parsed(text: &str) -> Option<u32> {
    let digits = text.trim().trim_start_matches('#');
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit())).then(|| u32::from_str_radix(digits, 16).ok()).flatten()
}

pub fn hex(color: u32) -> String {
    format!("#{color:06x}")
}

/// The color as OKLCH: how light, from 0 to 1, how much color, and its hue in degrees.
pub fn oklch(color: u32) -> (f64, f64, f64) {
    let linear = |channel: u32| {
        let v = f64::from(channel & 0xFF) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (linear(color >> 16), linear(color >> 8), linear(color));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let (lightness, a, b) = (
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    );
    let chroma = (a * a + b * b).sqrt();
    let hue = if chroma < 1e-4 { 0.0 } else { b.atan2(a).to_degrees().rem_euclid(360.0) };
    (lightness, chroma, hue)
}

/// What a shade is for, where its palette says: Radix gives each of its
/// twelve steps a use, from the app's background to high-contrast text.
pub fn use_of(palette: usize, shade: usize) -> Option<&'static str> {
    const RADIX: [&str; 12] = [
        "App background",
        "Subtle background",
        "Element background",
        "Hovered element",
        "Active element",
        "Subtle border",
        "Element border",
        "Hovered border",
        "Solid",
        "Hovered solid",
        "Low-contrast text",
        "High-contrast text",
    ];
    (all()[palette].id == "radix").then(|| RADIX.get(shade).copied()).flatten()
}

/// The ways a color can be copied, each with what it is called: the
/// hex first, which is what Enter copies.
pub fn formats(palette: usize, hue: usize, shade: usize) -> Vec<(&'static str, String)> {
    let color = of(palette, hue, shade);
    let (l, c, h) = oklch(color);
    let (name, shade_name) = (&all()[palette].hues[hue].0, &all()[palette].shades[shade]);
    let mut formats = vec![
        ("hex", hex(color)),
        ("RGB", format!("rgb({} {} {})", color >> 16 & 0xFF, color >> 8 & 0xFF, color & 0xFF)),
        ("OKLCH", format!("oklch({:.1}% {:.3} {:.1})", l * 100.0, c, h)),
    ];
    match all()[palette].id.as_str() {
        "tailwind" => formats.extend([("variable", format!("var(--color-{name}-{shade_name})")), ("class", format!("bg-{name}-{shade_name}"))]),
        "radix" => formats.push(("variable", format!("var(--{name}-{shade_name})"))),
        "chakra" => formats.extend([("variable", format!("var(--chakra-colors-{name}-{shade_name})")), ("token", format!("{name}.{shade_name}"))]),
        _ => {}
    }
    formats
}

/// Where the color last copied from each palette is kept, by name, so
/// that its palette opens on it the next time: "tailwind" = ["red", "300"].
fn picks_kept() -> std::path::PathBuf {
    crate::config::state_home().join("clutch/colors.json")
}

fn picks() -> std::collections::BTreeMap<String, (String, String)> {
    std::fs::read_to_string(picks_kept()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn keep_picks(picks: &std::collections::BTreeMap<String, (String, String)>) {
    let path = picks_kept();
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    if let Ok(text) = serde_json::to_string(picks) {
        let _ = std::fs::write(path, text);
    }
}

/// The hue and shade last copied from a palette, if one was and it still has them.
pub fn last_picked(palette: usize) -> Option<(usize, usize)> {
    let palette = &all()[palette];
    let (hue, shade) = picks().remove(&palette.id)?;
    Some((palette.hues.iter().position(|(name, _)| *name == hue)?, palette.shades.iter().position(|name| *name == shade)?))
}

pub fn keep_picked(palette: usize, hue: usize, shade: usize) {
    let mut kept = picks();
    let palette = &all()[palette];
    kept.insert(palette.id.clone(), (palette.hues[hue].0.clone(), palette.shades[shade].clone()));
    keep_picks(&kept);
}

pub fn forget_picked(palette: usize) {
    let mut kept = picks();
    if kept.remove(&all()[palette].id).is_some() {
        keep_picks(&kept);
    }
}

/// The colors called what was typed, in every palette that has one so
/// called: "blue-500" is Tailwind's, Material's and Chakra's.
pub fn named(typed: &str) -> Vec<(usize, usize, usize)> {
    let typed = typed.trim().to_lowercase();
    let Some((hue, shade)) = typed.rsplit_once(['-', ' ']) else { return Vec::new() };
    let mut found = Vec::new();
    for (at, palette) in all().iter().enumerate() {
        let hue_at = palette.hues.iter().position(|(name, _)| *name == hue);
        let shade_at = palette.shades.iter().position(|name| *name == shade);
        if let (Some(hue_at), Some(shade_at)) = (hue_at, shade_at) {
            found.push((at, hue_at, shade_at));
        }
    }
    found
}

/// Of each palette, the color nearest to this one, by how far apart they
/// are in OKLab, where distance is as the eye sees it.
pub fn nearest(color: u32) -> Vec<(usize, usize, usize)> {
    let lab = |color: u32| {
        let (l, c, h) = oklch(color);
        (l, c * h.to_radians().cos(), c * h.to_radians().sin())
    };
    let (l, a, b) = lab(color);
    let apart = |other: u32| {
        let (l2, a2, b2) = lab(other);
        (l - l2).powi(2) + (a - a2).powi(2) + (b - b2).powi(2)
    };
    let mut found = Vec::new();
    for (at, palette) in all().iter().enumerate() {
        let places = palette.hues.iter().enumerate().flat_map(|(hue, (_, colors))| colors.iter().enumerate().map(move |(shade, color)| (hue, shade, *color)));
        if let Some((hue, shade, _)) = places.min_by(|x, y| apart(x.2).total_cmp(&apart(y.2))) {
            found.push((at, hue, shade));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palettes_are_whole() {
        let ids: Vec<&str> = all().iter().map(|palette| palette.id.as_str()).collect();
        assert_eq!(ids, ["tailwind", "radix", "material", "chakra"]);
        for palette in all() {
            assert!(palette.hues.iter().all(|(_, colors)| colors.len() == palette.shades.len()), "{}", palette.id);
        }
    }

    #[test]
    fn a_color_by_its_name() {
        let found = named("blue-500");
        assert_eq!(found.iter().map(|(palette, ..)| all()[*palette].id.as_str()).collect::<Vec<_>>(), ["tailwind", "material", "chakra"]);
        let (palette, hue, shade) = found[0];
        assert_eq!(name_of(palette, hue, shade), "blue-500");
        assert_eq!(named("blue 9").len(), 1);
        assert!(named("blue").is_empty());
        // Radix's steps each have a use; Tailwind's shades none.
        let (radix, _, nine) = named("blue-9")[0];
        assert_eq!(use_of(radix, nine), Some("Solid"));
        assert_eq!(use_of(0, 5), None);
    }

    #[test]
    fn its_formats() {
        let (palette, hue, shade) = named("blue-500")[0];
        let formats = formats(palette, hue, shade);
        assert_eq!(formats[0], ("hex", "#2b7fff".to_string()));
        assert_eq!(formats[1].1, "rgb(43 127 255)");
        assert!(formats.iter().any(|format| format.1 == "bg-blue-500"));
        // White and black have no hue.
        assert!(oklch(0xFFFFFF).1 < 1e-3);
        assert!((oklch(0xFFFFFF).0 - 1.0).abs() < 1e-3);
    }

    #[test]
    fn the_nearest_of_each() {
        let tailwind_blue = of(0, all()[0].hues.iter().position(|(name, _)| name == "blue").unwrap(), 5);
        assert_eq!(name_of(0, nearest(tailwind_blue)[0].1, nearest(tailwind_blue)[0].2), "blue-500");
        assert_eq!(nearest(tailwind_blue).len(), 4);
        assert_eq!(parsed("#3B82F6"), Some(0x3b82f6));
        assert_eq!(parsed("3b82f"), None);
    }
}
