mod config;
mod daemon;
mod desk;
mod engine;
mod gnome;
mod keys;
mod palette;
mod setup;

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::exit;
use std::{env, fs};

const HELP: &str = "\
clutch: keys that are yours before any program's.

  clutch palette [TEXT]         the box: type to find a window or an application
                                  > things to do   @ windows   # workspaces   ! shell
  clutch raise NAME [COMMAND]   go to a program's window, or start it
  clutch move-monitor [prev]    the window in front, to the next monitor

These need no clutch running; any key bound anywhere can start them.

  clutch setup     put clutch on this machine and start it (asks for sudo, once)
  clutch status    what is running, and whether each binding works
  clutch fix       take from GNOME the shortcuts that are on your bindings' keys
  clutch pause     give the keyboards back, until `clutch resume`
  clutch resume
  clutch reload    read the settings again (it does when they are saved, too)
  clutch check     read the settings and say what is wrong with them
  clutch remove    take clutch off this machine
  clutch run       what the service runs: hold the keyboards, here, until stopped
      --config FILE   settings other than ~/.config/clutch/config.toml

Held together, Backspace, Escape and Enter stop clutch, whatever else is
going on.
";

/// Say something to the running clutch and return what it says back.
fn ask(what: &str) -> Result<String, String> {
    let not_running = |_| "clutch is not running (clutch setup, or systemctl --user start clutch)";
    let mut stream = UnixStream::connect(config::socket()).map_err(not_running)?;
    stream.write_all(what.as_bytes()).map_err(not_running)?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).map_err(not_running)?;
    Ok(reply)
}

/// `clutch palette [TEXT]` opens the box with TEXT typed. `--list TEXT`
/// and `--picture FILE [ROW] TEXT` show what it would hold without
/// putting it on the screen: as lines, or as a PNG with a row picked out.
fn open_palette(mut rest: Vec<String>) -> Result<(), String> {
    let mut flag = |name: &str| rest.iter().position(|argument| argument == name).map(|at| rest.remove(at)).is_some();
    let (list, picture) = (flag("--list"), flag("--picture"));
    if list {
        return Ok(palette::list(&rest.pop().unwrap_or_default()));
    }
    if !picture {
        return palette::run(&rest.join(" "));
    }
    let (Some(typed), false) = (rest.pop(), rest.is_empty()) else {
        return Err("palette --picture FILE [ROW] TEXT".into());
    };
    let row = rest.get(1).and_then(|row| row.parse().ok()).unwrap_or(0);
    palette::picture(&typed, row, &rest[0])
}

const NO_DESK: &str = "windows cannot be seen from here on Wayland yet";

/// Move the window in front to the next monitor, or the one before.
fn move_monitor(which: Option<&str>) -> Result<(), String> {
    let back = match which {
        Some("next") | None => false,
        Some("prev") => true,
        Some(other) => return Err(format!("move-monitor: next or prev, not {other}")),
    };
    let desk = desk::Desk::open().ok_or(NO_DESK)?;
    let window = desk.active().ok_or("no window is in front")?;
    desk.move_to_monitor(window, back).ok_or("there is no other monitor to move it to".into())
}

/// Go to a program's window, or start the program if it has none. Asked
/// again from one of its windows, go to its next.
fn raise(arguments: Vec<String>) -> Result<(), String> {
    let (name, command) = arguments.split_first().ok_or("raise: the name of a program, then the command that starts it")?;
    let command = if command.is_empty() { name.clone() } else { command.join(" ") };
    let start = || {
        std::process::Command::new("/bin/sh").arg("-c").arg(&command).spawn().map(drop).map_err(|e| format!("{command}: {e}"))
    };
    let Some(desk) = desk::Desk::open() else { return start() };
    let wanted = name.to_lowercase();
    let windows = desk.windows();
    let of_it: Vec<&desk::Window> = windows.iter().filter(|window| window.class.to_lowercase().contains(&wanted)).collect();
    match of_it.as_slice() {
        [] => start(),
        // The one used last, unless that is where we are: then the one
        // used longest ago, which takes each in turn.
        [first, .., last] if desk.active() == Some(first.id) => Ok(desk.activate(last.id)),
        [first, ..] => Ok(desk.activate(first.id)),
    }
}

/// How wide the terminal is, or 0 if what is written to is not one.
fn columns() -> usize {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    match unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut size) } {
        0 => size.ws_col as usize,
        _ => 0,
    }
}

fn check(path: PathBuf) -> Result<(), String> {
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (settings, _) = config::Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let width = settings.bindings.iter().map(|b| b.keys.len()).max().unwrap_or(0);
    for binding in &settings.bindings {
        println!("{:width$}  {}", binding.keys, binding.command);
    }
    if settings.bindings.is_empty() {
        println!("nothing bound");
    }
    Ok(())
}

fn run(mut arguments: impl Iterator<Item = String>) -> Result<(), String> {
    let mut options = daemon::Options { config: config::path(), devices: Vec::new() };
    while let Some(argument) = arguments.next() {
        let mut value = || arguments.next().ok_or(format!("{argument} needs a value"));
        match argument.as_str() {
            "--config" => options.config = PathBuf::from(value()?),
            "--device" => options.devices.push(PathBuf::from(value()?)),
            other => return Err(format!("run: no option {other}")),
        }
    }
    daemon::run(options)
}

fn main() {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_default();
    let done = match command.as_str() {
        "run" => run(arguments),
        "setup" => setup::setup(),
        "remove" => setup::remove(),
        "check" => check(arguments.next().map_or_else(config::path, PathBuf::from)),
        "palette" => open_palette(arguments.collect()),
        "windows" => palette::run("@"),
        "move-monitor" => move_monitor(arguments.next().as_deref()),
        "raise" => raise(arguments.collect()),
        "status" => {
            // In colour on a terminal, unless asked not to.
            let colour = columns() > 0 && env::var_os("NO_COLOR").is_none();
            ask(&format!("status {}{}", columns(), if colour { " colour" } else { "" })).map(|reply| print!("{reply}"))
        }
        "pause" | "resume" | "reload" | "fix" => ask(&command).map(|reply| print!("{reply}")),
        "" | "help" | "-h" | "--help" => {
            print!("{HELP}");
            Ok(())
        }
        "-V" | "--version" => {
            println!("clutch {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => Err(format!("no command {other} (clutch --help)")),
    };
    if let Err(problem) = done {
        eprintln!("clutch: {problem}");
        exit(1);
    }
}
