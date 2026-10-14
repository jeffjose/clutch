//! Widgets: a program's output, shown as a tile in the box. The settings
//! say what to run and how often, and fill a tile's few places from what
//! it prints, so every widget has the same tile and none can make it ugly:
//!
//! ```toml
//! [widgets.claude]
//! run = "ccpace -j"
//! every = 60
//! title = "Claude"
//! value = "{used:.0}%"
//! bar = "used"            # how full, out of `max`
//! max = 100
//! mark = "expected"       # where it should be, as a tick across the bar
//! line = "{delta:+.1} vs plan · {room_today:.0}% left today"
//! warn = "delta > 5"
//! bad = "week_over"
//! ```
//!
//! The running clutch runs each widget every `every` seconds and keeps
//! what it printed; the box only reads that, so it opens at once.

use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A widget that takes longer than this is stopped.
const LONGEST: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Widget {
    pub name: String,
    pub run: String,
    pub every: u64,
    pub title: String,
    pub value: String,
    pub bar: String,
    pub max: f64,
    pub mark: String,
    pub line: String,
    pub warn: String,
    pub bad: String,
    /// What a click on it runs, if anything.
    pub open: String,
}

/// The widgets in the settings, in the order written.
pub fn from_settings(table: &toml::Table) -> Result<Vec<Widget>, String> {
    let mut widgets = Vec::new();
    for (name, said) in table {
        let said = said.as_table().ok_or(format!("widgets.{name}: a table, as [widgets.{name}]"))?;
        let text = |key: &str| said.get(key).and_then(|value| value.as_str()).unwrap_or("").to_string();
        let number = |key: &str, or: f64| said.get(key).and_then(|value| value.as_float().or(value.as_integer().map(|n| n as f64))).unwrap_or(or);
        for key in said.keys() {
            if !["run", "every", "title", "value", "bar", "max", "mark", "line", "warn", "bad", "open"].contains(&key.as_str()) {
                return Err(format!("widgets.{name}: no setting {key}"));
            }
        }
        let run = text("run");
        if run.is_empty() {
            return Err(format!("widgets.{name}: run, the command that prints it"));
        }
        widgets.push(Widget {
            name: name.clone(),
            run,
            every: number("every", 60.0).max(5.0) as u64,
            title: said.get("title").and_then(|value| value.as_str()).unwrap_or(name).to_string(),
            value: text("value"),
            bar: text("bar"),
            max: number("max", 100.0),
            mark: text("mark"),
            line: text("line"),
            warn: text("warn"),
            bad: text("bad"),
            open: text("open"),
        });
    }
    Ok(widgets)
}

fn folder() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    std::env::var_os("XDG_CACHE_HOME").map_or(home.join(".cache"), PathBuf::from).join("clutch/widgets")
}

fn kept(widget: &Widget) -> PathBuf {
    folder().join(format!("{}.out", widget.name))
}

/// What a widget last printed, and how many seconds ago.
pub fn last(widget: &Widget) -> Option<(String, u64)> {
    let path = kept(widget);
    let age = fs::metadata(&path).ok()?.modified().ok()?.elapsed().map_or(0, |since| since.as_secs());
    Some((fs::read_to_string(path).ok()?, age))
}

/// Whether it is time it was run again.
pub fn due(widget: &Widget) -> bool {
    last(widget).is_none_or(|(_, age)| age >= widget.every)
}

/// Run it, and keep what it prints if it says it went well. Not for
/// longer than `LONGEST`: then it is stopped, and what was kept stays.
pub fn refresh(widget: &Widget) -> bool {
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(crate::config::with_clutch(&widget.run))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return false };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return false,
            Ok(None) if started.elapsed() > LONGEST => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut printed = String::new();
    if child.stdout.take().is_none_or(|mut out| out.read_to_string(&mut printed).is_err()) || printed.trim().is_empty() {
        return false;
    }
    let _ = fs::create_dir_all(folder());
    // Written whole, then put in place, so the box never reads half of it.
    let path = kept(widget);
    let fresh = path.with_extension(format!("{}", SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos())));
    fs::write(&fresh, printed).is_ok() && fs::rename(&fresh, &path).is_ok()
}

/// For the running clutch: run every widget when it is due, for as long
/// as clutch runs. The settings are read each time, so a widget added is
/// run without clutch being started again.
pub fn keep_fresh() {
    thread::spawn(|| loop {
        let settings = fs::read_to_string(crate::config::path()).unwrap_or_default();
        if let Ok((settings, _)) = crate::config::Config::parse(&settings) {
            for widget in settings.widgets.iter().filter(|widget| due(widget)) {
                refresh(widget);
            }
        }
        thread::sleep(Duration::from_secs(5));
    });
}

/// A tile, as a widget's places come out of what it printed.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Tile {
    pub title: String,
    pub value: String,
    /// How full the bar is, and where its tick is, from 0 to 1.
    pub bar: Option<f64>,
    pub mark: Option<f64>,
    pub line: String,
    /// 0 as it should be, 1 to be watched, 2 wrong.
    pub tone: u8,
}

/// Fill a widget's places from what it printed: JSON, whose fields the
/// places name, or plain text, which `{text}` and `{line1}`… are.
pub fn tile(widget: &Widget, printed: &str) -> Tile {
    let data = serde_json::from_str::<Value>(printed).unwrap_or_else(|_| {
        let mut lines = serde_json::Map::new();
        lines.insert("text".into(), Value::String(printed.trim().to_string()));
        for (at, line) in printed.lines().enumerate() {
            lines.insert(format!("line{}", at + 1), Value::String(line.to_string()));
        }
        Value::Object(lines)
    });
    let fraction = |path: &str| (!path.is_empty()).then(|| number(&data, path)).flatten().map(|n| (n / widget.max.max(f64::MIN_POSITIVE)).clamp(0.0, 1.0));
    let tone = if truth(&data, &widget.bad) { 2 } else if truth(&data, &widget.warn) { 1 } else { 0 };
    Tile { title: fill(&widget.title, &data), value: fill(&widget.value, &data), bar: fraction(&widget.bar), mark: fraction(&widget.mark), line: fill(&widget.line, &data), tone }
}

/// The field a path names: "used", "speed.0.speed".
fn field<'a>(data: &'a Value, path: &str) -> Option<&'a Value> {
    path.trim().split('.').try_fold(data, |at, key| match key.parse::<usize>() {
        Ok(index) if at.is_array() => at.get(index),
        _ => at.get(key),
    })
}

fn number(data: &Value, path: &str) -> Option<f64> {
    match field(data, path)? {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => text.trim().trim_end_matches('%').parse().ok(),
        Value::Bool(yes) => Some(if *yes { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Whether a condition holds: "week_over", "delta > 5", "speed.0.speed <= 1".
fn truth(data: &Value, said: &str) -> bool {
    let said = said.trim();
    if said.is_empty() {
        return false;
    }
    for sign in [">=", "<=", "==", "!=", ">", "<"] {
        if let Some((path, than)) = said.split_once(sign) {
            let (Some(value), Ok(than)) = (number(data, path), than.trim().parse::<f64>()) else { return false };
            return match sign {
                ">=" => value >= than,
                "<=" => value <= than,
                "==" => value == than,
                "!=" => value != than,
                ">" => value > than,
                _ => value < than,
            };
        }
    }
    match field(data, said) {
        Some(Value::Bool(yes)) => *yes,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty() && text != "false",
        _ => false,
    }
}

/// A place's words, with each `{path}` or `{path:format}` put in: the
/// format is `.1` for so many places, and `+` for a sign always.
pub fn fill(template: &str, data: &Value) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let inside = &rest[open + 1..open + close];
        let (path, format) = inside.split_once(':').unwrap_or((inside, ""));
        out.push_str(&said(field(data, path), format));
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

fn said(value: Option<&Value>, format: &str) -> String {
    let sign = format.contains('+');
    let places = format.split('.').nth(1).and_then(|places| places.trim_matches(|c: char| !c.is_ascii_digit()).parse::<usize>().ok());
    match value {
        Some(Value::Number(n)) => {
            let n = n.as_f64().unwrap_or(0.0);
            let text = match places {
                Some(places) => format!("{n:.places$}"),
                // As it would be written: whole where it is whole, else no long tail.
                None if n.fract() == 0.0 => format!("{}", n as i64),
                None => format!("{n:.2}").trim_end_matches('0').to_string(),
            };
            if sign && n >= 0.0 { format!("+{text}") } else { text }
        }
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(yes)) => yes.to_string(),
        Some(other) => other.to_string(),
        None => "\u{b7}".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> Widget {
        let settings: toml::Table = toml::from_str(
            r#"
            [claude]
            run = "ccpace -j"
            title = "Claude"
            value = "{used:.0}%"
            bar = "used"
            mark = "expected"
            line = "{delta:+.1} vs plan · {room_today:.0}% left"
            warn = "delta > 5"
            bad = "week_over"
            "#,
        )
        .unwrap();
        from_settings(&settings).unwrap().remove(0)
    }

    #[test]
    fn a_tile_from_json() {
        let printed = r#"{"used": 55.0, "expected": 63.27, "delta": -8.27, "room_today": 11.25, "week_over": false}"#;
        let tile = tile(&claude(), printed);
        assert_eq!(tile.value, "55%");
        assert_eq!(tile.line, "-8.3 vs plan · 11% left");
        assert_eq!((tile.bar, tile.tone), (Some(0.55), 0));
        assert!((tile.mark.unwrap() - 0.6327).abs() < 1e-9);
        let over = super::tile(&claude(), r#"{"used": 99, "delta": 7, "week_over": true}"#);
        assert_eq!((over.tone, over.line.as_str()), (2, "+7.0 vs plan · \u{b7}% left"));
    }

    #[test]
    fn a_tile_from_text_and_paths() {
        let widget = Widget { value: "{line1}".into(), line: "{line2}".into(), max: 100.0, ..Widget::default() };
        let tile = tile(&widget, "42 rpm\nfans quiet\n");
        assert_eq!((tile.value.as_str(), tile.line.as_str()), ("42 rpm", "fans quiet"));
        let data: Value = serde_json::from_str(r#"{"speed": [{"speed": 0.2}]}"#).unwrap();
        assert_eq!(fill("{speed.0.speed:.1}×", &data), "0.2×");
        assert!(truth(&data, "speed.0.speed < 1"));
    }

    #[test]
    fn what_is_wrong_with_one_is_said() {
        let settings: toml::Table = toml::from_str("[x]\nrun = \"date\"\ncolour = \"red\"").unwrap();
        assert_eq!(from_settings(&settings).unwrap_err(), "widgets.x: no setting colour");
        let settings: toml::Table = toml::from_str("[x]\ntitle = \"X\"").unwrap();
        assert!(from_settings(&settings).is_err());
    }
}
