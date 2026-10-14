//! The things that are glanced at and flipped, not found: whether Wi-Fi
//! is on and what it has joined, where sound plays and how loud, the
//! microphone, Bluetooth, notifications, the night light.
//!
//! Each is asked of the system's own tool, nmcli or pactl say, which
//! takes longer than the box may wait to open: they are asked on threads
//! of their own, and what they say comes in when it is ready.

use std::collections::HashSet;
use std::env;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub const WIFI: usize = 0;
pub const SOUND: usize = 1;
pub const MIC: usize = 2;
pub const BLUETOOTH: usize = 3;
pub const QUIET: usize = 4;
pub const NIGHT: usize = 5;
/// What is playing, to pause or go on with; and to go to the next of,
/// or the one before. These three are not tiles but the buttons of one
/// bar across the foot, as a player has them.
pub const PLAY: usize = 6;
pub const NEXT: usize = 7;
pub const PREVIOUS: usize = 8;
pub const COUNT: usize = 9;
/// Those that are tiles, two across.
pub const TILES: usize = 6;
pub const NAMES: [&str; COUNT] = ["Wi-Fi", "Sound", "Mic", "Bluetooth", "Quiet", "Night light", "Play", "Next", "Previous"];
/// The key each is flipped by, with Alt, as a project is gone to. The
/// track before has none: it is wanted least, and letters are few.
pub const KEYS: [char; COUNT] = ['w', 's', 'm', 'b', 'q', 'l', 'p', 'n', ' '];

/// Whether this is a laptop: it has a battery of its own, and not only a
/// mouse's or a keyboard's, which say they are a device's.
pub fn laptop() -> bool {
    let read = |path: std::path::PathBuf| std::fs::read_to_string(path).unwrap_or_default().trim().to_string();
    std::fs::read_dir("/sys/class/power_supply").into_iter().flatten().flatten().any(|supply| {
        read(supply.path().join("type")) == "Battery" && read(supply.path().join("scope")) != "Device"
    })
}

/// The controls that are tiles, as they stand: sound, wide, then the
/// microphone and Bluetooth, and Wi-Fi where it is a laptop's. Quiet and
/// the night light are kept, but not shown.
pub fn shown() -> Vec<usize> {
    let mut shown = vec![SOUND];
    if laptop() {
        shown.push(WIFI);
    }
    shown.extend([MIC, BLUETOOTH]);
    shown
}

/// The key a control is flipped by, if it is one that is shown.
pub fn key_of(at: usize) -> Option<char> {
    let shown = at >= TILES || shown().contains(&at);
    Some(KEYS[at]).filter(|key| shown && *key != ' ')
}

/// How one of them stands.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Tile {
    /// In a few words: "HomeNet · strong".
    pub what: String,
    /// Whether it is on.
    pub lit: bool,
    /// Whether the system has answered yet.
    pub known: bool,
    /// Whether this machine has such a thing at all.
    pub there: bool,
}

impl Tile {
    fn of(what: impl Into<String>, lit: bool) -> Tile {
        Tile { what: what.into(), lit, known: true, there: true }
    }

    fn absent() -> Tile {
        Tile { what: "not here".into(), lit: false, known: true, there: false }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Network {
    pub name: String,
    /// Out of 100.
    pub signal: u32,
    pub joined: bool,
    /// Whether it has been joined before, so that its password is known.
    pub saved: bool,
    pub open: bool,
    pub band: &'static str,
}

/// Something sound plays through, or is listened with.
#[derive(Clone, PartialEq, Debug)]
pub struct Device {
    /// What pactl knows it by.
    pub name: String,
    pub called: String,
    pub input: bool,
    /// A percentage, which may be over 100.
    pub level: u32,
    pub muted: bool,
    /// Whether it is the one in use.
    pub used: bool,
}

/// A program that is playing something, or has it paused.
#[derive(Clone, PartialEq, Debug)]
pub struct Player {
    /// What it is known by on the session's bus.
    pub name: String,
    /// The program: "Chrome".
    pub called: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

/// Something done to one of them.
#[derive(Clone)]
pub enum Deed {
    /// Turn the one at this place on, or off.
    Flip(usize),
    Join(Network),
    Leave(String),
    Scan,
    Level(Device, u32),
    Mute(Device),
    Use(Device),
    /// Tell a player something: "PlayPause", "Next", "Previous".
    Tell(Player, &'static str),
}

enum Report {
    Tile(usize, Tile),
    Wifi(Tile, Vec<Network>),
    Sound(Tile, Tile, Vec<Device>, Vec<Player>),
    /// How something being done is going, or why it could not be.
    Said(String),
}

type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct Controls {
    pub tiles: [Tile; COUNT],
    pub networks: Vec<Network>,
    pub devices: Vec<Device>,
    pub players: Vec<Player>,
    pub said: String,
    reports: Receiver<Report>,
    sender: Sender<Report>,
    /// To the thread that does things, one after another. With each deed,
    /// whether the tile it is of was lit when it was asked for.
    deeds: Option<Sender<(Deed, bool)>>,
}

impl Controls {
    /// Nothing known yet, and nothing asked.
    pub fn new() -> Controls {
        let (sender, reports) = mpsc::channel();
        let (networks, devices, players) = (Vec::new(), Vec::new(), Vec::new());
        Controls { tiles: Default::default(), networks, devices, players, said: String::new(), reports, sender, deeds: None }
    }

    /// Ask how everything stands. `wake` is called, from another thread,
    /// each time there is something for `take`.
    pub fn start(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        let wake: Wake = Arc::new(wake);
        // Those not shown are not asked about, and stand as answered.
        let shown = shown();
        for at in [WIFI, QUIET, NIGHT].into_iter().filter(|at| !shown.contains(at)) {
            self.tiles[at] = Tile::absent();
        }
        for at in [WIFI, SOUND, BLUETOOTH, QUIET, NIGHT].into_iter().filter(|at| shown.contains(at)) {
            let (sender, wake) = (self.sender.clone(), wake.clone());
            thread::spawn(move || {
                let _ = sender.send(read(at));
                wake();
            });
        }
        let (deeds, asked) = mpsc::channel::<(Deed, bool)>();
        self.deeds = Some(deeds);
        let sender = self.sender.clone();
        thread::spawn(move || {
            let say = |report: Report| {
                let _ = sender.send(report);
                wake();
            };
            while let Ok(first) = asked.recv() {
                // Those asked for meanwhile are done before anything is
                // read again, or a level held down would be seen to jump
                // back and forth.
                let mut touched = [false; COUNT];
                let mut next = Some(first);
                while let Some((deed, lit)) = next {
                    touched[perform(&deed, lit, &say)] = true;
                    next = asked.try_recv().ok();
                }
                for at in (0..COUNT).filter(|at| touched[*at]) {
                    say(read(at));
                }
            }
        });
    }

    /// Take in what has been said since the last time. Returns whether
    /// anything was.
    pub fn take(&mut self) -> bool {
        let mut any = false;
        while let Ok(report) = self.reports.try_recv() {
            self.heard(report);
            any = true;
        }
        any
    }

    /// Wait until everything has answered, or for as long as is sensible.
    pub fn settle(&mut self) {
        while self.tiles.iter().any(|tile| !tile.known) {
            let Ok(report) = self.reports.recv_timeout(Duration::from_secs(4)) else { return };
            self.heard(report);
        }
    }

    fn heard(&mut self, report: Report) {
        match report {
            Report::Tile(at, tile) => self.tiles[at] = tile,
            Report::Wifi(tile, networks) => (self.tiles[WIFI], self.networks) = (tile, networks),
            Report::Sound(sound, mic, devices, players) => {
                // What is playing is the one to pause; with nothing playing, the one there is to go on with.
                let player = players.first();
                self.tiles[PLAY] = player.map_or(Tile::absent(), |player| {
                    let what = if player.title.is_empty() { &player.called } else { &player.title };
                    Tile::of(what.clone(), player.playing)
                });
                self.tiles[NEXT] = player.map_or(Tile::absent(), |player| Tile::of(player.artist.clone(), false));
                self.tiles[PREVIOUS] = player.map_or(Tile::absent(), |player| Tile::of(player.called.clone(), false));
                (self.tiles[SOUND], self.tiles[MIC], self.devices, self.players) = (sound, mic, devices, players);
            }
            Report::Said(said) => self.said = said,
        }
    }

    /// Do it as soon as what it is of has said how it stands: asked for as
    /// the box opens, it may not have yet, and flipping it blind would be a guess.
    pub fn act_when_known(&mut self, deed: Deed) {
        if let Deed::Flip(at) = deed {
            while !self.tiles[at].known {
                let Ok(report) = self.reports.recv_timeout(Duration::from_secs(3)) else { break };
                self.heard(report);
            }
        }
        self.act(deed);
    }

    /// What is asked of the two that are of a player, asked of the player.
    fn as_said(&self, deed: Deed) -> Deed {
        match (deed, self.players.first()) {
            (Deed::Flip(PLAY), Some(player)) => Deed::Tell(player.clone(), "PlayPause"),
            (Deed::Flip(NEXT), Some(player)) => Deed::Tell(player.clone(), "Next"),
            (Deed::Flip(PREVIOUS), Some(player)) => Deed::Tell(player.clone(), "Previous"),
            (deed, _) => deed,
        }
    }

    pub fn act(&mut self, deed: Deed) {
        let deed = self.as_said(deed);
        // A level is shown as asked for at once: it is held down, and
        // the answer is a moment behind.
        if let Deed::Level(device, level) = &deed {
            for known in self.devices.iter_mut().filter(|known| known.name == device.name) {
                known.level = *level;
            }
        }
        let lit = match deed {
            Deed::Flip(at) => self.tiles[at].lit,
            _ => false,
        };
        if let Some(deeds) = &self.deeds {
            let _ = deeds.send((deed, lit));
        }
    }
}

/// What a program prints, if it ran and was content.
fn printed(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a program, and if it was not content, the first thing it said.
fn tried(program: &str, arguments: &[&str]) -> Result<(), String> {
    let output = Command::new(program).args(arguments).stdin(Stdio::null()).output().map_err(|e| format!("{program}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    Err(said.lines().next().unwrap_or("it did not work").trim_start_matches("Error: ").to_string())
}

/// The desktop, where what is asked depends on it.
fn desktop() -> &'static str {
    let named = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_uppercase();
    ["GNOME", "XFCE"].into_iter().find(|desktop| named.contains(desktop)).unwrap_or("")
}

const BANNERS: [&str; 2] = ["org.gnome.desktop.notifications", "show-banners"];
const NIGHT_LIGHT: [&str; 2] = ["org.gnome.settings-daemon.plugins.color", "night-light-enabled"];
const DISTURB: [&str; 4] = ["-c", "xfce4-notifyd", "-p", "/do-not-disturb"];

/// Ask how the control at this place stands.
fn read(at: usize) -> Report {
    match at {
        WIFI => read_wifi(),
        SOUND | MIC | PLAY | NEXT | PREVIOUS => read_sound(),
        BLUETOOTH => Report::Tile(at, read_bluetooth()),
        QUIET => {
            let quiet = match desktop() {
                "GNOME" => printed("gsettings", &["get", BANNERS[0], BANNERS[1]]).map(|shown| shown == "false"),
                // Until it is first set, xfce has nothing to say of it.
                "XFCE" => printed("xfconf-query", &["-c", "xfce4-notifyd", "-l"]).map(|_| printed("xfconf-query", &DISTURB).as_deref() == Some("true")),
                _ => None,
            };
            let tile = |quiet: bool| Tile::of(if quiet { "notifications off" } else { "notifications on" }, quiet);
            Report::Tile(at, quiet.map_or(Tile::absent(), tile))
        }
        _ => {
            let on = printed("gsettings", &["get", NIGHT_LIGHT[0], NIGHT_LIGHT[1]]).filter(|_| desktop() == "GNOME");
            Report::Tile(at, on.map_or(Tile::absent(), |on| Tile::of(if on == "true" { "on" } else { "off" }, on == "true")))
        }
    }
}

/// Do it. Returns which control should be asked about again.
fn perform(deed: &Deed, lit: bool, say: &impl Fn(Report)) -> usize {
    let to = if lit { "off" } else { "on" };
    let pactl = |what: &str, device: &Device, value: &str| {
        let what = format!("set-{}-{what}", if device.input { "source" } else { "sink" });
        tried("pactl", &[&what, &device.name, value])
    };
    let (at, done) = match deed {
        Deed::Flip(WIFI) => {
            let done = tried("nmcli", &["radio", "wifi", to]);
            // Turned on, it is a while before it has seen anything.
            if !lit && done.is_ok() {
                say(read_wifi());
                thread::sleep(Duration::from_secs(3));
            }
            (WIFI, done)
        }
        Deed::Flip(SOUND) => (SOUND, tried("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "toggle"])),
        Deed::Flip(MIC) => (MIC, tried("pactl", &["set-source-mute", "@DEFAULT_SOURCE@", "toggle"])),
        Deed::Flip(BLUETOOTH) => (BLUETOOTH, tried("timeout", &["5", "bluetoothctl", "power", to])),
        Deed::Flip(QUIET) => {
            let done = match desktop() {
                "GNOME" => tried("gsettings", &["set", BANNERS[0], BANNERS[1], if lit { "true" } else { "false" }]),
                _ => tried("xfconf-query", &[&DISTURB[..], &["-n", "-t", "bool", "-s", if lit { "false" } else { "true" }]].concat()),
            };
            (QUIET, done)
        }
        Deed::Flip(PLAY | NEXT | PREVIOUS) => (SOUND, Err("nothing is playing".to_string())),
        Deed::Flip(at) => (*at, tried("gsettings", &["set", NIGHT_LIGHT[0], NIGHT_LIGHT[1], if lit { "false" } else { "true" }])),
        Deed::Join(network) => {
            say(Report::Said(format!("joining {}…", network.name)));
            let done = match network.saved {
                true => tried("nmcli", &["--wait", "20", "connection", "up", "id", &network.name]),
                false => tried("nmcli", &["--wait", "20", "device", "wifi", "connect", &network.name]),
            };
            (WIFI, done)
        }
        Deed::Leave(name) => (WIFI, tried("nmcli", &["connection", "down", "id", name])),
        Deed::Scan => {
            say(Report::Said("scanning…".into()));
            (WIFI, tried("nmcli", &["device", "wifi", "list", "--rescan", "yes"]))
        }
        Deed::Level(device, level) => (SOUND, pactl("volume", device, &format!("{level}%"))),
        Deed::Mute(device) => (SOUND, pactl("mute", device, "toggle")),
        Deed::Use(device) => (SOUND, tried("pactl", &[if device.input { "set-default-source" } else { "set-default-sink" }, &device.name])),
        Deed::Tell(player, what) => {
            let done = tried("busctl", &["--user", "call", &player.name, MPRIS_PATH, MPRIS_PLAYER, what]);
            // It is a moment before the player says it has done so.
            thread::sleep(Duration::from_millis(150));
            (SOUND, done)
        }
    };
    say(Report::Said(done.err().unwrap_or_default()));
    at
}

/// A line of nmcli's terse printing, which has a colon between one thing
/// and the next and a backslash before any that is part of a name.
fn fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut letters = line.chars();
    while let Some(letter) = letters.next() {
        match letter {
            '\\' => fields.last_mut().unwrap().extend(letters.next()),
            ':' => fields.push(String::new()),
            _ => fields.last_mut().unwrap().push(letter),
        }
    }
    fields
}

/// The networks in nmcli's list of them, each once however many of its
/// masts are in reach: the one joined first, then those joined before,
/// then the rest, the strongest first.
fn networks_from(listing: &str, saved: &HashSet<String>) -> Vec<Network> {
    let mut networks: Vec<Network> = Vec::new();
    for line in listing.lines() {
        let [joined, name, signal, security, band] = &fields(line)[..] else { continue };
        if name.is_empty() {
            continue;
        }
        let band = match band.split(' ').next().and_then(|mhz| mhz.parse::<u32>().ok()) {
            Some(..3000) => "2.4 GHz",
            Some(..5925) => "5 GHz",
            Some(_) => "6 GHz",
            None => "",
        };
        let network = Network {
            name: name.clone(),
            signal: signal.parse().unwrap_or(0),
            joined: joined.trim() == "*",
            saved: saved.contains(name),
            open: security.is_empty() || security == "--",
            band,
        };
        match networks.iter_mut().find(|known| known.name == network.name) {
            Some(known) if (network.joined, network.signal) > (known.joined, known.signal) => *known = network,
            Some(_) => {}
            None => networks.push(network),
        }
    }
    networks.sort_by_key(|network| (!network.joined, !network.saved, std::cmp::Reverse(network.signal)));
    networks
}

fn read_wifi() -> Report {
    let has = printed("nmcli", &["-t", "-f", "TYPE", "device"]).is_some_and(|kinds| kinds.lines().any(|kind| kind == "wifi"));
    let Some(on) = printed("nmcli", &["-t", "-f", "WIFI", "general"]).filter(|_| has) else {
        return Report::Wifi(Tile::absent(), Vec::new());
    };
    if on != "enabled" {
        return Report::Wifi(Tile::of("off", false), Vec::new());
    }
    let saved = printed("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"]).unwrap_or_default();
    let saved = saved.lines().map(fields).filter(|known| known.get(1).is_some_and(|kind| kind.contains("wireless")));
    let saved: HashSet<String> = saved.map(|mut known| known.swap_remove(0)).collect();
    let listing = printed("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY,FREQ", "device", "wifi", "list", "--rescan", "no"]);
    let networks = networks_from(&listing.unwrap_or_default(), &saved);
    let what = match networks.iter().find(|network| network.joined) {
        Some(joined) => format!("{} · {}", joined.name, strength(joined.signal)),
        None => "not joined".to_string(),
    };
    Report::Wifi(Tile::of(what, true), networks)
}

pub fn strength(signal: u32) -> &'static str {
    match signal {
        67.. => "strong",
        34.. => "fair",
        _ => "weak",
    }
}

/// The devices in pactl's list of them, as JSON. A sink's monitor is a
/// source in name only, and is left out.
fn devices_from(listing: &str, input: bool, used: &str) -> Vec<Device> {
    let listed: Vec<serde_json::Value> = serde_json::from_str(listing).unwrap_or_default();
    let device = |device: &serde_json::Value| -> Option<Device> {
        let name = device["name"].as_str()?;
        if device["properties"]["device.class"] == "monitor" || name.ends_with(".monitor") {
            return None;
        }
        // The level of the first of its channels, which are near enough alike.
        let level = device["volume"].as_object().and_then(|channels| channels.values().next());
        let level = level.and_then(|channel| channel["value_percent"].as_str()?.trim_end_matches('%').trim().parse().ok());
        Some(Device {
            name: name.to_string(),
            called: device["description"].as_str().unwrap_or(name).to_string(),
            input,
            level: level.unwrap_or(0),
            muted: device["mute"] == true,
            used: name == used,
        })
    };
    listed.iter().filter_map(device).collect()
}

const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// What a player says is on, from its Metadata as busctl prints it in
/// JSON: the title, and the artists.
fn track_from(metadata: &str) -> (String, String) {
    let metadata: serde_json::Value = serde_json::from_str(metadata).unwrap_or_default();
    let of = |key: &str| &metadata["data"][key]["data"];
    let artists: Vec<&str> = of("xesam:artist").as_array().map_or(Vec::new(), |artists| artists.iter().filter_map(|artist| artist.as_str()).collect());
    (of("xesam:title").as_str().unwrap_or("").to_string(), artists.join(", "))
}

/// The programs that say, on the session's bus, that they play things:
/// the ones playing first.
fn read_players() -> Vec<Player> {
    let names = printed("busctl", &["--user", "list", "--no-legend"]).unwrap_or_default();
    let names = names.lines().filter_map(|line| line.split_whitespace().next()).filter(|name| name.starts_with("org.mpris.MediaPlayer2."));
    let mut players: Vec<Player> = names
        .filter_map(|name| {
            let property = |of: &str, property: &str| printed("busctl", &["--user", "--json=short", "get-property", name, MPRIS_PATH, of, property]);
            let word = |printed: String| serde_json::from_str::<serde_json::Value>(&printed).ok()?["data"].as_str().map(str::to_string);
            let playing = property(MPRIS_PLAYER, "PlaybackStatus").and_then(word)? == "Playing";
            let (title, artist) = track_from(&property(MPRIS_PLAYER, "Metadata").unwrap_or_default());
            let called = property("org.mpris.MediaPlayer2", "Identity").and_then(word).unwrap_or_default();
            Some(Player { name: name.to_string(), called, title, artist, playing })
        })
        .collect();
    players.sort_by_key(|player| !player.playing);
    players
}

fn read_sound() -> Report {
    let list = |kind: &str, input: bool, used: &str| -> Option<Vec<Device>> {
        let used = printed("pactl", &[used])?;
        Some(devices_from(&printed("pactl", &["-f", "json", "list", kind])?, input, &used))
    };
    let players = read_players();
    let Some(mut devices) = list("sinks", false, "get-default-sink") else {
        return Report::Sound(Tile::absent(), Tile::absent(), Vec::new(), players);
    };
    devices.extend(list("sources", true, "get-default-source").unwrap_or_default());
    let used = |input: bool| devices.iter().find(|device| device.used && device.input == input);
    let sound = match used(false) {
        Some(device) if device.muted => Tile::of("muted", false),
        Some(device) => Tile::of(device.called.clone(), true),
        None => Tile::absent(),
    };
    let mic = used(true).map_or(Tile::absent(), |device| Tile::of(if device.muted { "muted" } else { "on" }, !device.muted));
    Report::Sound(sound, mic, devices, players)
}

fn read_bluetooth() -> Tile {
    let ask = |arguments: &[&str]| printed("timeout", &[&["3", "bluetoothctl"], arguments].concat());
    let Some(powered) = ask(&["show"]).and_then(|shown| Some(shown.lines().find(|line| line.trim().starts_with("Powered:"))?.contains("yes"))) else {
        return Tile::absent();
    };
    if !powered {
        return Tile::of("off", false);
    }
    // The first of the devices it knows that is connected.
    let devices = ask(&["devices"]).unwrap_or_default();
    let connected = devices.lines().take(12).find_map(|line| {
        let mut words = line.splitn(3, ' ');
        let (address, name) = (words.nth(1)?, words.next()?);
        ask(&["info", address])?.contains("Connected: yes").then(|| name.to_string())
    });
    Tile::of(connected.unwrap_or("on".into()), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colon_in_a_name() {
        assert_eq!(fields(r"*:Home\:Net:80:WPA2:5180 MHz"), ["*", "Home:Net", "80", "WPA2", "5180 MHz"]);
        assert_eq!(fields(" :open::"), [" ", "open", "", ""]);
    }

    #[test]
    fn each_network_once_and_the_joined_first() {
        let listing = " :Far:30:WPA2:2412 MHz\n :Home:60:WPA2:2412 MHz\n*:Home:55:WPA2:5180 MHz\n ::90:WPA2:5180 MHz\n :Cafe:80::2437 MHz\n :Old:40:WPA2:5180 MHz\n";
        let saved = HashSet::from(["Home".to_string(), "Old".to_string()]);
        let networks = networks_from(listing, &saved);
        let names: Vec<&str> = networks.iter().map(|network| network.name.as_str()).collect();
        assert_eq!(names, ["Home", "Old", "Cafe", "Far"]);
        assert!(networks[0].joined && networks[0].saved && networks[0].band == "5 GHz");
        assert!(networks[2].open && !networks[2].saved);
    }

    #[test]
    fn the_devices_pactl_lists() {
        let listing = r#"[
            {"name":"out.usb","description":"USB Audio","mute":false,"volume":{"front-left":{"value":40000,"value_percent":"62%"}},"properties":{"device.class":"sound"}},
            {"name":"out.usb.monitor","description":"Monitor of USB Audio","mute":false,"volume":{},"properties":{"device.class":"monitor"}},
            {"name":"out.hdmi","description":"HDMI","mute":true,"volume":{"front-left":{"value":65536,"value_percent":"100%"}},"properties":{}}
        ]"#;
        let devices = devices_from(listing, false, "out.usb");
        assert_eq!(devices.len(), 2);
        assert_eq!((devices[0].called.as_str(), devices[0].level, devices[0].used, devices[0].muted), ("USB Audio", 62, true, false));
        assert!(devices[1].muted && !devices[1].used);
        assert!(devices_from("not json", false, "").is_empty());
    }

    #[test]
    fn what_a_player_has_on() {
        let metadata = r#"{"type":"a{sv}","data":{"xesam:artist":{"type":"as","data":["A","B"]},"xesam:title":{"type":"s","data":"Song"}}}"#;
        assert_eq!(track_from(metadata), ("Song".to_string(), "A, B".to_string()));
        assert_eq!(track_from(""), (String::new(), String::new()));
    }
}
