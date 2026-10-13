//! clutch between a keyboard that is not real and a reader that keeps
//! what clutch writes to itself, so nothing is typed into the machine
//! the test runs on.
//!
//! It needs /dev/uinput, which is there after `clutch setup` or for root:
//!
//!     cargo test -- --ignored

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant};
use std::{env, fs};

const ESC: u16 = 1;
const BACKSPACE: u16 = 14;
const ENTER: u16 = 28;
const CTRL: u16 = 29;
const A: u16 = 30;
const B: u16 = 48;
const ALT: u16 = 56;
const SPACE: u16 = 57;

fn wait_for<T>(what: &str, mut look: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(found) = look() {
            return found;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "waited too long for {what}");
        sleep(Duration::from_millis(20));
    }
}

struct Bench {
    dir: PathBuf,
    keyboard: VirtualDevice,
    node: PathBuf,
    /// What the programs would see.
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

    fn held(&self) -> bool {
        self.ask("state").starts_with("holding")
    }

    fn press(&mut self, keys: &[(u16, i32)]) {
        for (code, value) in keys {
            let event = InputEvent::new(EventType::KEY.0, *code, *value);
            self.keyboard.emit(&[event]).unwrap();
            sleep(Duration::from_millis(5));
        }
    }

    /// The keys that came out the other side, once they have stopped.
    fn seen(&mut self) -> Vec<(u16, i32)> {
        let mut keys = Vec::new();
        let mut quiet = Instant::now();
        while quiet.elapsed() < Duration::from_millis(250) {
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

    fn ran(&self, name: &str) -> bool {
        self.dir.join(name).exists()
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = self.clutch.kill();
        let _ = self.clutch.wait();
    }
}

fn bench(settings: &str) -> Bench {
    let dir = env::temp_dir().join(format!("clutch-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.toml"), settings).unwrap();

    let keys: AttributeSet<KeyCode> = (1..120).map(KeyCode::new).collect();
    let mut keyboard = VirtualDevice::builder()
        .expect("/dev/uinput")
        .name("clutch test keyboard")
        .with_keys(&keys)
        .unwrap()
        .build()
        .unwrap();
    let node = wait_for("the keyboard's node", || {
        let node = keyboard.enumerate_dev_nodes_blocking().ok()?.next()?.ok()?;
        Device::open(&node).is_ok().then_some(node)
    });

    let clutch = Command::new(env!("CARGO_BIN_EXE_clutch"))
        .args(["run", "--config"])
        .arg(dir.join("config.toml"))
        .arg("--device")
        .arg(&node)
        .env("CLUTCH_SOCKET", dir.join("socket"))
        .env("CLUTCH_NAME", "clutch test output")
        .env_remove("NOTIFY_SOCKET")
        .current_dir(&dir)
        .spawn()
        .unwrap();

    // Held by us before a key is pressed, so the desktop never has it.
    let mut seen = wait_for("clutch's own keyboard", || {
        evdev::enumerate()
            .map(|(_, device)| device)
            .find(|device| device.name() == Some("clutch test output"))
    });
    seen.grab().unwrap();
    seen.set_nonblocking(true).unwrap();

    let bench = Bench { dir, keyboard, node, seen, clutch };
    wait_for("clutch to hold the keyboard", || bench.held().then_some(()));
    bench
}

fn grabbable(node: &Path) -> bool {
    let mut device = Device::open(node).unwrap();
    let free = device.grab().is_ok();
    let _ = device.ungrab();
    free
}

#[test]
#[ignore = "needs /dev/uinput"]
fn a_keyboard_that_is_not_real() {
    let dir = env::temp_dir().join(format!("clutch-test-{}", std::process::id()));
    let touch = |name: &str| format!("touch {}", dir.join(name).display());
    let mut bench = bench(&format!(
        "[bind]\n\"ctrl+space\" = \"{}\"\n\"alt+a b\" = \"{}\"\n",
        touch("chord"),
        touch("sequence")
    ));

    // A key that is not bound comes out as it went in.
    bench.press(&[(A, 1), (A, 0)]);
    assert_eq!(bench.seen(), [(A, 1), (A, 0)]);

    // A chord is kept, all but its modifier, and runs.
    bench.press(&[(CTRL, 1), (SPACE, 1), (SPACE, 0), (CTRL, 0)]);
    assert_eq!(bench.seen(), [(CTRL, 1), (CTRL, 0)]);
    wait_for("the chord to run", || bench.ran("chord").then_some(()));

    // A sequence, and the tap of Ctrl that Alt is shown in place of A.
    bench.press(&[(ALT, 1), (A, 1), (A, 0), (ALT, 0), (B, 1), (B, 0)]);
    assert_eq!(bench.seen(), [(ALT, 1), (CTRL, 1), (CTRL, 0), (ALT, 0)]);
    wait_for("the sequence to run", || bench.ran("sequence").then_some(()));
    assert_eq!(bench.ask("status").matches("  1×").count(), 2);

    // The settings are read again when they are saved.
    fs::write(bench.dir.join("config.toml"), "[bind]\n\"ctrl+b\" = \"true\"\n").unwrap();
    wait_for("the new settings", || bench.ask("status").contains("ctrl+b").then_some(()));
    bench.press(&[(CTRL, 1), (SPACE, 1), (SPACE, 0), (CTRL, 0)]);
    assert_eq!(bench.seen(), [(CTRL, 1), (SPACE, 1), (SPACE, 0), (CTRL, 0)]);

    // Settings with a mistake in them change nothing.
    fs::write(bench.dir.join("config.toml"), "[bind]\n\"ctrl+nokey\" = \"true\"\n").unwrap();
    wait_for("the mistake to be noticed", || bench.ask("status").contains("nokey").then_some(()));
    bench.press(&[(CTRL, 1), (B, 1), (B, 0), (CTRL, 0)]);
    assert_eq!(bench.seen(), [(CTRL, 1), (CTRL, 0)]);

    // Paused, the keyboard is anyone's; resumed, it is clutch's again.
    assert!(!grabbable(&bench.node));
    bench.ask("pause");
    assert!(bench.ask("status").contains("paused"));
    assert!(grabbable(&bench.node));
    bench.ask("resume");
    wait_for("clutch to hold the keyboard again", || bench.held().then_some(()));
    bench.press(&[(A, 1), (A, 0)]);
    assert_eq!(bench.seen(), [(A, 1), (A, 0)]);

    // A key that is down when clutch pauses is not left down.
    bench.press(&[(A, 1)]);
    assert_eq!(bench.seen(), [(A, 1)]);
    bench.ask("pause");
    assert_eq!(bench.seen(), [(A, 0)]);
    bench.press(&[(A, 0)]);
    bench.ask("resume");
    wait_for("clutch to hold the keyboard once more", || bench.held().then_some(()));

    // The way out.
    bench.press(&[(BACKSPACE, 1), (ESC, 1), (ENTER, 1)]);
    let status = wait_for("clutch to stop", || bench.clutch.try_wait().unwrap());
    assert!(status.success());
    assert!(grabbable(&bench.node));

    let _ = fs::remove_dir_all(&bench.dir);
}
