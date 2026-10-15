//! Icons from Iconify, looked up as they are asked for rather than
//! carried: a search names them, and each set gives the drawings of those
//! of its icons that were found, in one answer. A drawing once fetched is
//! kept in the cache, so that it is there again without the network.

use super::canvas::Icon;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

const API: &str = "https://api.iconify.design";
/// How many a search asks for.
const LIMIT: usize = 96;
/// How long typing must stop for before what is typed is looked for.
const PAUSE: Duration = Duration::from_millis(180);
/// How long a search's answer is kept.
const SEARCH_KEPT: Duration = Duration::from_secs(24 * 60 * 60);
/// A copied icon: this many pixels square, in black, on nothing.
const COPIED_SIDE: usize = 256;

/// A set of icons, as the search says of it.
#[derive(Clone, Default)]
pub struct Set {
    pub name: String,
    pub license: String,
}

/// What a search found: the icons, by their full names ("lucide:house"),
/// and the sets they are of, by their prefixes.
#[derive(Clone, Default)]
pub struct Found {
    pub names: Vec<String>,
    pub sets: HashMap<String, Set>,
}

fn kept_in() -> PathBuf {
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    env::var_os("XDG_CACHE_HOME").map_or(home.join(".cache"), PathBuf::from).join("clutch/icons")
}

/// Where an icon's drawing is kept: a folder for each set.
fn kept(name: &str) -> Option<PathBuf> {
    let (set, icon) = name.split_once(':')?;
    let plain = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    (plain(set) && plain(icon)).then(|| kept_in().join(set).join(format!("{icon}.svg")))
}

/// An icon's drawing, as an SVG, where it has been fetched.
pub fn drawing(name: &str) -> Option<String> {
    fs::read_to_string(kept(name)?).ok()
}

fn fetched(url: &str) -> Result<String, String> {
    let output = Command::new("curl").args(["-sf", "-m", "8", url]).stdin(Stdio::null()).stderr(Stdio::null()).output().map_err(|e| format!("curl: {e}"))?;
    if !output.status.success() {
        return Err("offline".to_string());
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

/// What is typed, as a search: "lucide:arrow" is "arrow" among Lucide's alone.
fn search_url(typed: &str) -> Option<String> {
    let typed = typed.trim();
    let (set, words) = match typed.split_once(':') {
        Some((set, words)) if !set.is_empty() && !set.contains(' ') => (Some(set), words.trim()),
        _ => (None, typed),
    };
    if words.is_empty() {
        return None;
    }
    let mut url = format!("{API}/search?query={}&limit={LIMIT}", super::url_encoded(words));
    if let Some(set) = set {
        url += &format!("&prefix={}", super::url_encoded(&set.to_lowercase()));
    }
    Some(url)
}

/// What a search found, as Iconify answered it: kept for a day, so that
/// a box opened again and again, as it is while clutch is worked on,
/// asks nothing of Iconify's servers for what it has asked before.
pub fn search(typed: &str) -> Result<Found, String> {
    let Some(url) = search_url(typed) else { return Ok(Found::default()) };
    let path = kept_in().join("searches").join(format!("{:08x}.json", super::projects::number_of(&url)));
    let fresh = fs::metadata(&path).ok().and_then(|kept| kept.modified().ok()?.elapsed().ok()).is_some_and(|age| age < SEARCH_KEPT);
    let text = match fs::read_to_string(&path).ok().filter(|_| fresh) {
        Some(text) => text,
        None => {
            let text = fetched(&url)?;
            let _ = fs::create_dir_all(kept_in().join("searches"));
            let _ = fs::write(&path, &text);
            text
        }
    };
    let answer: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(found_in(&answer))
}

fn found_in(answer: &serde_json::Value) -> Found {
    let names = answer["icons"].as_array().into_iter().flatten().filter_map(|name| name.as_str().map(String::from)).collect();
    let sets = answer["collections"].as_object().into_iter().flatten();
    let sets = sets.map(|(prefix, set)| (prefix.clone(), Set { name: set["name"].as_str().unwrap_or(prefix).to_string(), license: set["license"]["title"].as_str().unwrap_or("").to_string() }));
    Found { names, sets: sets.collect() }
}

/// The drawings of these icons: those kept, and the rest fetched, a set
/// at a time and the sets all at once. Each fetched is kept.
pub fn drawings(names: &[String]) -> HashMap<String, String> {
    let mut found = HashMap::new();
    let mut missing: HashMap<String, Vec<String>> = HashMap::new();
    for name in names {
        match drawing(name) {
            Some(svg) => drop(found.insert(name.clone(), svg)),
            None => {
                if let (Some((set, icon)), Some(_)) = (name.split_once(':'), kept(name)) {
                    missing.entry(set.to_string()).or_default().push(icon.to_string());
                }
            }
        }
    }
    let asked: Vec<_> = missing.into_iter().map(|(set, icons)| thread::spawn(move || of_set(&set, &icons))).collect();
    for asked in asked {
        for (name, svg) in asked.join().unwrap_or_default() {
            if let Some(path) = kept(&name) {
                let _ = fs::create_dir_all(path.parent().unwrap_or(&kept_in()));
                let _ = fs::write(path, &svg);
            }
            found.insert(name, svg);
        }
    }
    found
}

fn of_set(set: &str, icons: &[String]) -> Vec<(String, String)> {
    let Ok(text) = fetched(&format!("{API}/{set}.json?icons={}", icons.join(","))) else { return Vec::new() };
    serde_json::from_str(&text).map(|answer| drawings_in(set, &answer)).unwrap_or_default()
}

/// Each icon of a set's answer as an SVG of its own. An alias is drawn
/// as the icon it is another name for.
fn drawings_in(set: &str, answer: &serde_json::Value) -> Vec<(String, String)> {
    let number = |of: &serde_json::Value, key: &str, or: f64| of[key].as_f64().unwrap_or(or);
    // Iconify's own: an icon that says nothing of its size is 16 square.
    let (width, height) = (number(answer, "width", 16.0), number(answer, "height", 16.0));
    let svg = |icon: &serde_json::Value| {
        let (wide, tall) = (number(icon, "width", width), number(icon, "height", height));
        let (left, top) = (number(icon, "left", 0.0), number(icon, "top", 0.0));
        let body = icon["body"].as_str()?;
        Some(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{left} {top} {wide} {tall}\" width=\"{wide}\" height=\"{tall}\">{body}</svg>"))
    };
    let Some(icons) = answer["icons"].as_object() else { return Vec::new() };
    let mut drawn: Vec<(String, String)> = icons.iter().filter_map(|(name, icon)| Some((format!("{set}:{name}"), svg(icon)?))).collect();
    for (alias, of) in answer["aliases"].as_object().into_iter().flatten() {
        if let Some(svg) = of["parent"].as_str().and_then(|parent| icons.get(parent)).and_then(svg) {
            drawn.push((format!("{set}:{alias}"), svg));
        }
    }
    drawn
}

/// An icon drawn, a square of `side`: in `color` where it is drawn in
/// the color of the text around it, as most are, and in its own elsewhere.
pub fn render(svg: &str, color: u32, side: usize) -> Option<Icon> {
    use resvg::{tiny_skia, usvg};
    let svg = svg.replace("currentColor", &format!("#{color:06x}"));
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(side as u32, side as u32)?;
    let size = tree.size();
    let scale = side as f32 / size.width().max(size.height());
    let (x, y) = ((side as f32 - size.width() * scale) / 2.0, (side as f32 - size.height() * scale) / 2.0);
    resvg::render(&tree, tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, x, y), &mut pixmap.as_mut());
    let pixels = pixmap
        .pixels()
        .iter()
        .map(|pixel| {
            let pixel = pixel.demultiply();
            u32::from(pixel.alpha()) << 24 | u32::from(pixel.red()) << 16 | u32::from(pixel.green()) << 8 | u32::from(pixel.blue())
        })
        .collect();
    Some(Icon { width: side, height: side, pixels })
}

/// Put an icon on the clipboard as a picture: black, on nothing, large
/// enough to be made smaller wherever it is pasted.
pub fn copy_png(name: &str) {
    let Some(icon) = drawing(name).and_then(|svg| render(&svg, 0x000000, COPIED_SIDE)) else { return };
    let path = kept_in().join("copied.png");
    let _ = fs::create_dir_all(kept_in());
    let Ok(file) = fs::File::create(&path) else { return };
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), icon.width as u32, icon.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    let bytes: Vec<u8> = icon.pixels.iter().flat_map(|pixel| [(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8, (pixel >> 24) as u8]).collect();
    let written = encoder.write_header().and_then(|mut writer| writer.write_image_data(&bytes));
    if written.is_ok() {
        crate::clip::restore(&path);
    }
}

/// Its page on Iconify's site.
pub fn page_of(name: &str) -> String {
    let (set, icon) = name.split_once(':').unwrap_or(("", name));
    format!("https://icon-sets.iconify.design/{set}/{icon}/")
}

type Answer = (String, Result<Found, String>, HashMap<String, String>);
type Wake = Arc<dyn Fn() + Send + Sync>;

/// The searches made as the box is typed into, each on a thread of its
/// own, and only once the typing stops: an answer to what is no longer
/// typed is kept, but not shown.
pub struct Lookup {
    asked: Arc<AtomicUsize>,
    sender: mpsc::Sender<Answer>,
    answers: mpsc::Receiver<Answer>,
    wake: Arc<Mutex<Option<Wake>>>,
    /// What was last looked for, and whether its answer has come.
    typed: String,
    answered: bool,
    /// What is shown: the last answer to come for what is typed.
    pub shown: Option<Result<Found, String>>,
    pub drawings: HashMap<String, String>,
    /// Every answer while the box is up, by what was typed: going back to
    /// what was typed before needs nothing fetched.
    known: HashMap<String, Result<Found, String>>,
}

impl Lookup {
    pub fn new() -> Lookup {
        let (sender, answers) = mpsc::channel();
        let wake = Arc::new(Mutex::new(None));
        Lookup { asked: Arc::default(), sender, answers, wake, typed: String::new(), answered: true, shown: None, drawings: HashMap::new(), known: HashMap::new() }
    }

    /// What wakes the box when an answer comes.
    pub fn wake_with(&self, wake: Option<Wake>) {
        if let Ok(mut now) = self.wake.lock() {
            *now = wake;
        }
    }

    /// Whether what is typed is still being looked for.
    pub fn looking(&self) -> bool {
        !self.answered
    }

    /// Look for what is typed, a moment after the typing stops.
    pub fn ask(&mut self, typed: &str) {
        let typed = typed.trim().to_string();
        if typed == self.typed {
            return;
        }
        self.typed = typed.clone();
        let number = self.asked.fetch_add(1, Ordering::SeqCst) + 1;
        if typed.is_empty() {
            (self.shown, self.answered) = (None, true);
            return;
        }
        if let Some(known) = self.known.get(&typed) {
            (self.shown, self.answered) = (Some(known.clone()), true);
            return;
        }
        self.answered = false;
        let (asked, sender, wake) = (self.asked.clone(), self.sender.clone(), self.wake.clone());
        thread::spawn(move || {
            thread::sleep(PAUSE);
            if asked.load(Ordering::SeqCst) != number {
                return;
            }
            let found = search(&typed);
            let drawings = found.as_ref().map(|found| drawings(&found.names)).unwrap_or_default();
            let _ = sender.send((typed, found, drawings));
            if let Some(wake) = wake.lock().ok().and_then(|wake| wake.clone()) {
                wake();
            }
        });
    }

    /// Take in the answers that have come. Whether one was to what is typed.
    pub fn take(&mut self) -> bool {
        let mut new = false;
        while let Ok((typed, found, drawings)) = self.answers.try_recv() {
            self.drawings.extend(drawings);
            if typed == self.typed {
                (self.shown, self.answered, new) = (Some(found.clone()), true, true);
            }
            self.known.insert(typed, found);
        }
        new
    }

    /// Wait for the answer to what is typed, for as long as a search can take.
    pub fn settle(&mut self) {
        let until = Instant::now() + Duration::from_secs(10);
        while self.looking() && Instant::now() < until {
            thread::sleep(Duration::from_millis(50));
            self.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_typed_as_a_search() {
        assert_eq!(search_url("home").as_deref(), Some("https://api.iconify.design/search?query=home&limit=96"));
        assert_eq!(search_url("arrow left").as_deref(), Some("https://api.iconify.design/search?query=arrow+left&limit=96"));
        assert_eq!(search_url("Lucide:arrow").as_deref(), Some("https://api.iconify.design/search?query=arrow&limit=96&prefix=lucide"));
        assert_eq!(search_url("  "), None);
        assert_eq!(search_url("mdi:"), None);
    }

    #[test]
    fn a_sets_answer_as_drawings() {
        let answer = serde_json::json!({
            "prefix": "lucide", "width": 24, "height": 24,
            "icons": {"house": {"body": "<path d=\"M3 10h18\"/>"}, "wide": {"body": "<g/>", "width": 32}},
            "aliases": {"home": {"parent": "house"}}
        });
        let drawn: HashMap<String, String> = drawings_in("lucide", &answer).into_iter().collect();
        assert_eq!(drawn["lucide:house"], "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" width=\"24\" height=\"24\"><path d=\"M3 10h18\"/></svg>");
        assert!(drawn["lucide:wide"].contains("viewBox=\"0 0 32 24\""));
        assert_eq!(drawn["lucide:home"], drawn["lucide:house"]);
    }

    #[test]
    fn a_search_answer() {
        let answer = serde_json::json!({"icons": ["mdi:home", "lucide:house"], "collections": {"mdi": {"name": "Material Design Icons", "license": {"title": "Apache 2.0"}}}});
        let found = found_in(&answer);
        assert_eq!(found.names, ["mdi:home", "lucide:house"]);
        assert_eq!(found.sets["mdi"].name, "Material Design Icons");
        assert_eq!(found.sets["mdi"].license, "Apache 2.0");
    }

    #[test]
    fn a_drawing_in_its_color() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\" fill=\"currentColor\"/></svg>";
        let icon = render(svg, 0x112233, 8).unwrap();
        assert_eq!((icon.width, icon.height), (8, 8));
        assert!(icon.pixels.iter().all(|pixel| *pixel == 0xFF112233));
        // Only a name of letters, digits and dashes is a place in the cache.
        assert!(kept("../x:y").is_none() && kept("mdi:home").is_some());
    }
}
