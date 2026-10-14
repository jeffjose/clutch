//! How long the box takes to come up, kept so that a slow one can be
//! looked into after the fact: one line for each opening, in
//! ~/.local/state/clutch/timings.jsonl, the last `KEPT` of them.
//!
//! Each says when the key was pressed (the running clutch passes that on
//! as CLUTCH_PRESSED), how long until the box was started, and then how
//! long each step of its coming up took, to the first frame on screen.
//! `clutch timings` reads them.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const KEPT: usize = 500;

/// When the box started, by both clocks, and each step since, in order.
struct Marks {
    started: Instant,
    started_at: u128,
    steps: Vec<(&'static str, Instant)>,
    done: bool,
}

static MARKS: Mutex<Option<Marks>> = Mutex::new(None);

fn now_nanos() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos())
}

/// What the running clutch sets, for what a key starts: when it was pressed.
pub fn pressed_now() -> String {
    now_nanos().to_string()
}

/// The box has started: steps are counted from here.
pub fn begin() {
    if let Ok(mut marks) = MARKS.lock() {
        *marks = Some(Marks { started: Instant::now(), started_at: now_nanos(), steps: Vec::new(), done: false });
    }
}

/// A step is done.
pub fn mark(step: &'static str) {
    if let Ok(mut marks) = MARKS.lock() {
        if let Some(marks) = marks.as_mut().filter(|marks| !marks.done) {
            marks.steps.push((step, Instant::now()));
        }
    }
}

fn path() -> PathBuf {
    crate::config::state_home().join("clutch/timings.jsonl")
}

/// The box is on the screen: keep what it took, once.
pub fn finish() {
    let Ok(mut marks) = MARKS.lock() else { return };
    let Some(marks) = marks.as_mut().filter(|marks| !marks.done) else { return };
    marks.done = true;
    let ms = |from: Instant, to: Instant| (to - from).as_secs_f64() * 1000.0;
    let mut steps = Vec::new();
    let mut last = marks.started;
    for (step, at) in &marks.steps {
        steps.push(format!("[\"{step}\",{:.1}]", ms(last, *at)));
        last = *at;
    }
    let total = ms(marks.started, last);
    // From the key to the box starting: what the running clutch and the
    // shell it starts the box with took.
    let pressed = std::env::var("CLUTCH_PRESSED").ok().and_then(|nanos| nanos.parse::<u128>().ok());
    let waited = pressed.map(|pressed| marks.started_at.saturating_sub(pressed) as f64 / 1e6);
    let line = format!(
        "{{\"at\":{},\"to_start\":{},\"total\":{:.1},\"steps\":[{}]}}\n",
        marks.started_at / 1_000_000_000,
        waited.map_or("null".to_string(), |waited| format!("{waited:.1}")),
        total + waited.unwrap_or(0.0),
        steps.join(",")
    );
    let path = path();
    let _ = fs::create_dir_all(path.parent().unwrap_or(&path));
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
    // A ring: once it is half as long again as it keeps, the oldest go.
    if let Ok(text) = fs::read_to_string(&path) {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > KEPT * 3 / 2 {
            let kept = lines[lines.len() - KEPT..].join("\n") + "\n";
            let fresh = path.with_extension("jsonl.new");
            if fs::write(&fresh, kept).is_ok() {
                let _ = fs::rename(fresh, &path);
            }
        }
    }
}

/// One opening, as kept.
struct Opening {
    at: u64,
    to_start: Option<f64>,
    total: f64,
    steps: Vec<(String, f64)>,
}

fn read() -> Vec<Opening> {
    let text = fs::read_to_string(path()).unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let said: serde_json::Value = serde_json::from_str(line).ok()?;
            let steps = said["steps"].as_array()?.iter().filter_map(|step| Some((step[0].as_str()?.to_string(), step[1].as_f64()?))).collect();
            Some(Opening { at: said["at"].as_u64()?, to_start: said["to_start"].as_f64(), total: said["total"].as_f64()?, steps })
        })
        .collect()
}

/// `clutch timings`: the last few openings and how long each took, the
/// slowest of what is kept with the steps that made them slow, and the
/// usual time of each step.
///
/// With a number, the last so many lines as they are kept, to copy.
pub fn show(count: Option<usize>) -> Result<(), String> {
    if let Some(count) = count {
        let text = fs::read_to_string(path()).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        for line in &lines[lines.len().saturating_sub(count)..] {
            println!("{line}");
        }
        return Ok(());
    }
    let openings = read();
    if openings.is_empty() {
        return Err(format!("nothing kept yet: open the box, then look again ({})", crate::config::short(&path())));
    }
    let clock = |at: u64| {
        let output = std::process::Command::new("date").args(["-d", &format!("@{at}"), "+%a %H:%M:%S"]).output();
        output.ok().map_or(at.to_string(), |output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let slowest_steps = |opening: &Opening| {
        let mut steps: Vec<(String, f64)> = opening.to_start.map(|ms| ("to start".to_string(), ms)).into_iter().chain(opening.steps.iter().cloned()).collect();
        steps.sort_by(|a, b| b.1.total_cmp(&a.1));
        steps.iter().take(3).map(|(step, ms)| format!("{step} {ms:.0}")).collect::<Vec<_>>().join(", ")
    };
    println!("last opened (ms, key to first frame; the slowest steps)");
    for opening in openings.iter().rev().take(10) {
        println!("  {}  {:>6.0}   {}", clock(opening.at), opening.total, slowest_steps(opening));
    }
    let mut slow: Vec<&Opening> = openings.iter().collect();
    slow.sort_by(|a, b| b.total.total_cmp(&a.total));
    println!("\nslowest of the {} kept", openings.len());
    for opening in slow.iter().take(5) {
        println!("  {}  {:>6.0}   {}", clock(opening.at), opening.total, slowest_steps(opening));
    }
    // Each step's middle time, and its worst.
    let mut names: Vec<String> = vec!["to start".to_string()];
    for opening in &openings {
        for (step, _) in &opening.steps {
            if !names.contains(step) {
                names.push(step.clone());
            }
        }
    }
    println!("\neach step (ms)               middle    worst");
    for name in names {
        let mut times: Vec<f64> = openings
            .iter()
            .filter_map(|opening| if name == "to start" { opening.to_start } else { opening.steps.iter().find(|(step, _)| *step == name).map(|(_, ms)| *ms) })
            .collect();
        if times.is_empty() {
            continue;
        }
        times.sort_by(f64::total_cmp);
        println!("  {name:<24} {:>8.1} {:>8.1}", times[times.len() / 2], times[times.len() - 1]);
    }
    Ok(())
}
