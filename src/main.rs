mod config;
mod daemon;
mod engine;
mod gnome;
mod keys;
mod setup;

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::exit;
use std::{env, fs};

const HELP: &str = "\
clutch: keys that are yours before any program's.

  clutch setup     put clutch on this machine and start it (asks for sudo, once)
  clutch status    what is running, the keyboards it holds, what is bound
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
        "status" => ask(&format!("status {}", columns())).map(|reply| print!("{reply}")),
        "pause" | "resume" | "reload" => ask(&command).map(|reply| print!("{reply}")),
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
