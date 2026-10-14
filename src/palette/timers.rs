//! Timers: "timer 25m tea" counts down from the box, and when it is done
//! the desktop says so, with a sound. The box is gone long before that,
//! so the counting is left to a shell that sleeps; what it is counting
//! is a file, for the box to show while it runs and to stop it by.

use crate::config;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const SOUND: &str = "/usr/share/sounds/freedesktop/stereo/alarm-clock-elapsed.oga";

fn folder() -> PathBuf {
    config::state_home().join("clutch/timers")
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// A timer that is running.
#[derive(Clone, PartialEq, Debug)]
pub struct Timer {
    pub path: PathBuf,
    /// When it is done, in seconds.
    pub end: u64,
    pub label: String,
    /// The shell that is counting, which is also its group.
    pub pid: i32,
}

/// What "timer …" asks for: how long, in seconds, and what it is for.
/// "25m", "1h30m", "90s" or a bare number of minutes, then any words.
pub fn asked(typed: &str) -> Option<(u64, String)> {
    let rest = typed.trim().strip_prefix("timer")?.trim_start();
    let (length, label) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let mut seconds = 0;
    let mut number = String::new();
    for c in length.chars() {
        match c {
            '0'..='9' | '.' => number.push(c),
            'h' | 'm' | 's' => {
                let size = match c {
                    'h' => 3600.0,
                    'm' => 60.0,
                    _ => 1.0,
                };
                seconds += (number.parse::<f64>().ok()? * size) as u64;
                number.clear();
            }
            _ => return None,
        }
    }
    // A number with nothing after it is minutes.
    if !number.is_empty() {
        seconds += (number.parse::<f64>().ok()? * 60.0) as u64;
    }
    (seconds > 0).then(|| (seconds, label.trim().to_string()))
}

/// How long, as a person says it: "25 min", "1 h 30 min", "90 s".
pub fn said(seconds: u64) -> String {
    let (hours, minutes, rest) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    let parts = [(hours, "h"), (minutes, "min"), (rest, "s")];
    let parts: Vec<String> = parts.iter().filter(|(count, _)| *count > 0).map(|(count, unit)| format!("{count} {unit}")).collect();
    parts.join(" ")
}

/// What is left, as a clock shows it: "24:13", or "1:02:05".
pub fn left(timer: &Timer) -> String {
    let left = timer.end.saturating_sub(now());
    match left / 3600 {
        0 => format!("{}:{:02}", left / 60, left % 60),
        hours => format!("{hours}:{:02}:{:02}", left % 3600 / 60, left % 60),
    }
}

fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Start one: a shell that sleeps, then says so and takes its file away.
pub fn start(seconds: u64, label: &str) {
    let dir = folder();
    let _ = fs::create_dir_all(&dir);
    let end = now() + seconds;
    let path = dir.join(format!("{end}-{}", std::process::id()));
    let title = if label.is_empty() { "Time is up".to_string() } else { format!("{label}: time is up") };
    let line = format!(
        "sleep {seconds}; notify-send -u critical -i alarm-clock {} {}; paplay {SOUND} 2>/dev/null; rm -f {}",
        quoted(&title),
        quoted(&format!("{} timer", said(seconds))),
        quoted(&path.to_string_lossy()),
    );
    let started = Command::new("/bin/sh").arg("-c").arg(line).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn();
    if let Ok(child) = started {
        let _ = fs::write(&path, format!("{}\n{label}\n", child.id()));
    }
}

/// The timers that are running, the soonest done first. One whose shell
/// has gone, the machine having been turned off say, is let go.
pub fn running() -> Vec<Timer> {
    let mut timers: Vec<Timer> = fs::read_dir(folder())
        .map(|files| {
            files
                .flatten()
                .filter_map(|file| {
                    let path = file.path();
                    let end = path.file_name()?.to_str()?.split('-').next()?.parse().ok()?;
                    let said = fs::read_to_string(&path).ok()?;
                    let mut lines = said.lines();
                    let pid: i32 = lines.next()?.parse().ok()?;
                    let label = lines.next().unwrap_or("").to_string();
                    // Still counting, if its shell is still there.
                    if unsafe { libc::kill(pid, 0) } != 0 {
                        let _ = fs::remove_file(&path);
                        return None;
                    }
                    Some(Timer { path, end, label, pid })
                })
                .collect()
        })
        .unwrap_or_default();
    timers.sort_by_key(|timer| timer.end);
    timers
}

/// Stop one before it is done: its shell and the sleep in it.
pub fn stop(timer: &Timer) {
    unsafe {
        libc::kill(-timer.pid, libc::SIGTERM);
    }
    let _ = fs::remove_file(&timer.path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "starts a shell that sleeps, then stops it"]
    fn a_timer_is_stopped_whole() {
        start(600, "probe");
        let timer = running().into_iter().find(|timer| timer.label == "probe").unwrap();
        let group = unsafe { libc::getpgid(timer.pid) };
        assert_eq!(group, timer.pid, "the pid kept is the shell's, which leads its group");
        stop(&timer);
        // The shell is this test's child, so it is waited for to be gone.
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(timer.pid, &mut status, 0) }, timer.pid);
        assert!(libc::WIFSIGNALED(status), "it was stopped, not let finish");
        assert!(running().iter().all(|timer| timer.label != "probe"));
    }

    #[test]
    fn how_long_is_asked() {
        assert_eq!(asked("timer 25m tea"), Some((1500, "tea".to_string())));
        assert_eq!(asked("timer 1h30m"), Some((5400, String::new())));
        assert_eq!(asked("timer 90s eggs boiled"), Some((90, "eggs boiled".to_string())));
        assert_eq!(asked("timer 10"), Some((600, String::new())));
        assert_eq!(asked("timer"), None);
        assert_eq!(asked("timer soon"), None);
        assert_eq!(asked("timers"), None);
        assert_eq!(said(5400), "1 h 30 min");
    }
}
