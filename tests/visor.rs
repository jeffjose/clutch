//! visor on X11, for real: clutch between a keyboard that is not real
//! and a reader, a Ghostty that comes and goes on the screen, and a
//! second window to click away to. The terminal runs `sleep`, not screen,
//! so no session of yours is taken.
//!
//! It needs /dev/uinput, an X11 display and Ghostty:
//!
//!     cargo test --test visor -- --ignored

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant};
use std::{env, fs};

const CTRL: u16 = 29;
const SUPER: u16 = 125;
const LEFT: u16 = 105;
const UP: u16 = 103;

const CLASS: &str = "io.github.jeffjose.visortest";

fn wait_for<T>(what: &str, mut look: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(found) = look() {
            return found;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "waited too long for {what}");
        sleep(Duration::from_millis(50));
    }
}

struct Bench {
    dir: PathBuf,
    keyboard: VirtualDevice,
    seen: Device,
    clutch: Child,
}

impl Bench {
    fn ask(&self, what: &str) -> String {
        let mut stream = UnixStream::connect(self.dir.join("socket")).unwrap();
        stream.write_all(what.as_bytes()).unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply
    }

    fn press(&mut self, keys: &[(u16, i32)]) {
        for (code, value) in keys {
            self.keyboard.emit(&[InputEvent::new(EventType::KEY.0, *code, *value)]).unwrap();
            sleep(Duration::from_millis(5));
        }
    }

    fn seen(&mut self) -> Vec<(u16, i32)> {
        let mut keys = Vec::new();
        let mut quiet = Instant::now();
        while quiet.elapsed() < Duration::from_millis(400) {
            if let Ok(events) = self.seen.fetch_events() {
                for event in events.filter(|e| e.event_type() == EventType::KEY) {
                    keys.push((event.code(), event.value()));
                    quiet = Instant::now();
                }
            }
            sleep(Duration::from_millis(5));
        }
        keys
    }

    /// `clutch visor …`, as a key would run it, with the bench's settings.
    fn visor(&self, words: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_clutch"))
            .arg("visor")
            .args(words)
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .env("XDG_STATE_HOME", self.dir.join("state"))
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr)
    }

    fn status(&self) -> String {
        self.visor(&["status"])
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = self.clutch.kill();
        let _ = self.clutch.wait();
        let _ = Command::new("pkill").args(["-f", &format!("class={CLASS}")]).status();
    }
}

#[test]
#[ignore = "needs /dev/uinput, an X11 display and Ghostty"]
fn visor_on_x11() {
    let dir = env::temp_dir().join(format!("clutch-visor-test-{}", std::process::id()));
    let settings_dir = dir.join("config/clutch");
    fs::create_dir_all(&settings_dir).unwrap();
    let ghostty = format!("ghostty --class={CLASS} --gtk-single-instance=false --window-decoration=false -e sleep 120");
    let settings = format!(
        "[bind]\n\"ctrl+space\" = \"clutch visor\"\n\n[visor]\nclass = \"{CLASS}\"\nrun = \"{ghostty}\"\nhide_on_click_away = true\n"
    );
    let config = settings_dir.join("config.toml");
    fs::write(&config, settings).unwrap();

    let keys: AttributeSet<KeyCode> = (1..130).map(KeyCode::new).collect();
    let mut keyboard = VirtualDevice::builder().expect("/dev/uinput").name("clutch visor test keyboard").with_keys(&keys).unwrap().build().unwrap();
    let node = wait_for("the keyboard's node", || {
        let node = keyboard.enumerate_dev_nodes_blocking().ok()?.next()?.ok()?;
        Device::open(&node).is_ok().then_some(node)
    });
    let clutch = Command::new(env!("CARGO_BIN_EXE_clutch"))
        .args(["run", "--config"])
        .arg(&config)
        .arg("--device")
        .arg(&node)
        .env("CLUTCH_SOCKET", dir.join("socket"))
        .env("CLUTCH_NAME", "clutch visor test output")
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env_remove("NOTIFY_SOCKET")
        .current_dir(&dir)
        .spawn()
        .unwrap();
    let mut seen = wait_for("clutch's own keyboard", || {
        evdev::enumerate().map(|(_, device)| device).find(|device| device.name() == Some("clutch visor test output"))
    });
    seen.grab().unwrap();
    seen.set_nonblocking(true).unwrap();
    let mut bench = Bench { dir, keyboard, seen, clutch };
    wait_for("clutch to hold the keyboard", || bench.ask("state").starts_with("holding").then_some(()));

    let super_left = [(SUPER, 1), (LEFT, 1), (LEFT, 0), (SUPER, 0)];
    let passed = vec![(SUPER, 1), (LEFT, 1), (LEFT, 0), (SUPER, 0)];
    let kept = vec![(SUPER, 1), (CTRL, 1), (CTRL, 0), (SUPER, 0)];

    // No terminal: Super+Left is the desktop's, as ever.
    assert!(bench.status().contains("not running"), "{}", bench.status());
    bench.press(&super_left);
    assert_eq!(bench.seen(), passed);

    // Ctrl+Space starts it, at the top.
    bench.press(&[(CTRL, 1), (57, 1), (57, 0), (CTRL, 0)]);
    assert_eq!(bench.seen(), vec![(CTRL, 1), (CTRL, 0)]);
    wait_for("the terminal to come in", || bench.status().contains("shown").then_some(()));
    assert!(bench.status().contains("shown, top, 100% wide and 40% high"), "{}", bench.status());

    // In, Super+Left is visor's: it goes to the left, and the desktop
    // sees only Super and the tap of Ctrl that stands in for the key.
    bench.press(&super_left);
    assert_eq!(bench.seen(), kept);
    wait_for("it to go left", || bench.status().contains("shown, left").then_some(()));

    // Super+Up takes it back to the top.
    bench.press(&[(SUPER, 1), (UP, 1), (UP, 0), (SUPER, 0)]);
    assert_eq!(bench.seen(), vec![(SUPER, 1), (CTRL, 1), (CTRL, 0), (SUPER, 0)]);
    wait_for("it to go up", || bench.status().contains("shown, top").then_some(()));

    // `clutch status` says where it is, and which keys are visor's.
    let status = bench.ask("status");
    assert!(status.contains("Visor") && status.contains("shown, top"), "{status}");
    assert!(status.contains("In visor") && status.contains("super+left"), "{status}");

    // Away, Super+Left is the desktop's again.
    bench.visor(&["hide"]);
    wait_for("it to go", || bench.status().contains("hidden").then_some(()));
    bench.press(&super_left);
    assert_eq!(bench.seen(), passed);

    // Another window taking the keyboard sends it away.
    bench.visor(&["show"]);
    wait_for("it to come back", || bench.status().contains("shown").then_some(()));
    let other = format!("ghostty --class={CLASS}.other --gtk-single-instance=false -e sleep 60");
    Command::new("sh").arg("-c").arg(&other).spawn().unwrap();
    let window = wait_for("the other window", || {
        let found = Command::new("xdotool").args(["search", "--class", &format!("{CLASS}.other")]).output().ok()?;
        String::from_utf8_lossy(&found.stdout).lines().last().map(str::to_string)
    });
    sleep(Duration::from_millis(500));
    Command::new("xdotool").args(["windowactivate", &window]).status().unwrap();
    wait_for("a click away to send it away", || bench.status().contains("hidden").then_some(()));
}
