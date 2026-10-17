//! What becomes of each key: passed on, or kept because it is part of a
//! binding. Nothing here touches a device, so all of it can be tested.

use crate::keys::{modifier_of, Chord, ALT, CTRL, KEY_LEFTCTRL, SUPER};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Out {
    /// A key event for the programs: the key, and 1, 0 or 2 for down, up, repeat.
    Key(u16, i32),
    /// Run the binding with this number.
    Run(usize),
}

#[derive(Default)]
struct Node {
    next: HashMap<Chord, Node>,
    binding: Option<usize>,
}

/// The bindings as a tree, one level for each chord of a sequence.
#[derive(Default)]
pub struct Bindings {
    root: Node,
    /// The bindings that count only while visor is in.
    while_visor: Vec<usize>,
}

impl Bindings {
    /// A binding that counts only while visor is in, and is a key like
    /// any other while it is not.
    pub fn add_while_visor(&mut self, sequence: &[Chord], binding: usize) -> Result<(), &'static str> {
        self.add(sequence, binding)?;
        self.while_visor.push(binding);
        Ok(())
    }

    pub fn add(&mut self, sequence: &[Chord], binding: usize) -> Result<(), &'static str> {
        let mut node = &mut self.root;
        for chord in sequence {
            if node.binding.is_some() {
                return Err("starts with keys that are a binding of their own");
            }
            node = node.next.entry(*chord).or_default();
        }
        if !node.next.is_empty() {
            return Err("is the start of a longer binding");
        }
        if node.binding.is_some() {
            return Err("is bound twice");
        }
        node.binding = Some(binding);
        Ok(())
    }

    fn at(&self, path: &[Chord]) -> Option<&Node> {
        let mut node = &self.root;
        for chord in path {
            node = node.next.get(chord)?;
        }
        Some(node)
    }
}

pub struct Engine {
    bindings: Bindings,
    timeout: Duration,
    /// The modifier keys that are down.
    held: Vec<u16>,
    /// The keys whose press was kept, so their repeats and release are too.
    kept: Vec<u16>,
    /// The chords of a sequence so far, and when waiting for the next ends.
    pending: Option<(Vec<Chord>, Instant)>,
    /// Whether visor is in, asked as one of its keys is pressed.
    visor_in: Box<dyn FnMut() -> bool + Send>,
}

impl Engine {
    pub fn new(bindings: Bindings, timeout: Duration) -> Engine {
        Engine { bindings, timeout, held: Vec::new(), kept: Vec::new(), pending: None, visor_in: Box::new(|| false) }
    }

    /// How to tell whether visor is in.
    pub fn ask_visor(&mut self, visor_in: Box<dyn FnMut() -> bool + Send>) {
        self.visor_in = visor_in;
    }

    /// New bindings, with the keys that are down left as they are.
    pub fn rebind(&mut self, bindings: Bindings, timeout: Duration) {
        self.bindings = bindings;
        self.timeout = timeout;
        self.pending = None;
    }

    /// Forget every key: the keyboards were let go.
    pub fn reset(&mut self) {
        self.held.clear();
        self.kept.clear();
        self.pending = None;
    }

    /// When a sequence that is waiting for its next key gives up.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|(_, deadline)| *deadline)
    }

    pub fn tick(&mut self, now: Instant) {
        if self.deadline().is_some_and(|deadline| now >= deadline) {
            self.pending = None;
        }
    }

    fn mods(&self) -> u8 {
        self.held.iter().fold(0, |mods, code| mods | modifier_of(*code))
    }

    pub fn key(&mut self, code: u16, value: i32, now: Instant, out: &mut Vec<Out>) {
        self.tick(now);
        match value {
            0 => {
                self.held.retain(|held| *held != code);
                if let Some(at) = self.kept.iter().position(|kept| *kept == code) {
                    self.kept.swap_remove(at);
                } else {
                    out.push(Out::Key(code, 0));
                }
            }
            1 => self.press(code, now, out),
            _ => {
                if !self.kept.contains(&code) {
                    out.push(Out::Key(code, value));
                }
            }
        }
    }

    fn press(&mut self, code: u16, now: Instant, out: &mut Vec<Out>) {
        // Modifiers always go through: the programs need them for every
        // key that is not ours.
        if modifier_of(code) != 0 {
            if !self.held.contains(&code) {
                self.held.push(code);
            }
            out.push(Out::Key(code, 1));
            return;
        }

        let chord = Chord { mods: self.mods(), code };
        let mut path = self.pending.take().map(|(path, _)| path).unwrap_or_default();
        let in_sequence = !path.is_empty();
        path.push(chord);

        let node = self.bindings.at(&path);
        // One of visor's keys, while it is not in, is any other key.
        let for_visor = node.and_then(|node| node.binding).is_some_and(|binding| self.bindings.while_visor.contains(&binding));
        if for_visor && !in_sequence && !(self.visor_in)() {
            out.push(Out::Key(code, 1));
            return;
        }
        let Some(node) = node else {
            if in_sequence {
                // A key that does not continue the sequence ends it, and
                // is not typed into whatever is in front.
                self.kept.push(code);
            } else {
                out.push(Out::Key(code, 1));
            }
            return;
        };

        self.kept.push(code);
        // To a program, Alt or Super going down and up with nothing
        // between is a tap of it, which opens a menu. It never saw our
        // key, so it is shown another in its place.
        if chord.mods & (ALT | SUPER) != 0 && chord.mods & CTRL == 0 {
            out.push(Out::Key(KEY_LEFTCTRL, 1));
            out.push(Out::Key(KEY_LEFTCTRL, 0));
        }
        match node.binding {
            Some(binding) => out.push(Out::Run(binding)),
            None => self.pending = Some((path, now + self.timeout)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::parse_sequence;
    use Out::{Key, Run};

    const CTRL_L: u16 = 29;
    const CTRL_R: u16 = 97;
    const SHIFT_L: u16 = 42;
    const ALT_L: u16 = 56;
    const SPACE: u16 = 57;
    const A: u16 = 30;
    const B: u16 = 48;
    const T: u16 = 20;

    fn engine(bound: &[&str]) -> Engine {
        let mut bindings = Bindings::default();
        for (number, keys) in bound.iter().enumerate() {
            bindings.add(&parse_sequence(keys).unwrap(), number).unwrap();
        }
        Engine::new(bindings, Duration::from_secs(2))
    }

    fn feed(engine: &mut Engine, now: Instant, events: &[(u16, i32)]) -> Vec<Out> {
        let mut out = Vec::new();
        for (code, value) in events {
            engine.key(*code, *value, now, &mut out);
        }
        out
    }

    #[test]
    fn unbound_keys_pass() {
        let mut e = engine(&["ctrl+space"]);
        let out = feed(&mut e, Instant::now(), &[(A, 1), (A, 2), (A, 0), (SPACE, 1), (SPACE, 0)]);
        assert_eq!(out, [Key(A, 1), Key(A, 2), Key(A, 0), Key(SPACE, 1), Key(SPACE, 0)]);
    }

    #[test]
    fn a_chord_is_kept_and_runs_once() {
        let mut e = engine(&["ctrl+space"]);
        let out = feed(
            &mut e,
            Instant::now(),
            &[(CTRL_L, 1), (SPACE, 1), (SPACE, 2), (SPACE, 2), (SPACE, 0), (CTRL_L, 0)],
        );
        assert_eq!(out, [Key(CTRL_L, 1), Run(0), Key(CTRL_L, 0)]);
    }

    #[test]
    fn the_modifiers_must_be_exactly_those() {
        let mut e = engine(&["ctrl+space"]);
        let out = feed(
            &mut e,
            Instant::now(),
            &[(CTRL_L, 1), (SHIFT_L, 1), (SPACE, 1), (SPACE, 0), (SHIFT_L, 0), (CTRL_L, 0)],
        );
        assert!(!out.contains(&Run(0)));
        assert!(out.contains(&Key(SPACE, 1)));
    }

    #[test]
    fn either_ctrl_and_both() {
        let mut e = engine(&["ctrl+space"]);
        let now = Instant::now();
        assert!(feed(&mut e, now, &[(CTRL_R, 1), (SPACE, 1), (SPACE, 0)]).contains(&Run(0)));
        // The left one going down and up leaves the right one held.
        feed(&mut e, now, &[(CTRL_L, 1), (CTRL_L, 0)]);
        assert!(feed(&mut e, now, &[(SPACE, 1), (SPACE, 0)]).contains(&Run(0)));
        feed(&mut e, now, &[(CTRL_R, 0)]);
        assert_eq!(feed(&mut e, now, &[(SPACE, 1)]), [Key(SPACE, 1)]);
    }

    #[test]
    fn the_release_is_kept_after_the_modifier_goes_first() {
        let mut e = engine(&["ctrl+space"]);
        let out = feed(
            &mut e,
            Instant::now(),
            &[(CTRL_L, 1), (SPACE, 1), (CTRL_L, 0), (SPACE, 2), (SPACE, 0)],
        );
        assert_eq!(out, [Key(CTRL_L, 1), Run(0), Key(CTRL_L, 0)]);
    }

    #[test]
    fn alt_and_super_are_shown_another_key() {
        let mut e = engine(&["alt+space"]);
        let out = feed(&mut e, Instant::now(), &[(ALT_L, 1), (SPACE, 1), (SPACE, 0), (ALT_L, 0)]);
        assert_eq!(
            out,
            [Key(ALT_L, 1), Key(KEY_LEFTCTRL, 1), Key(KEY_LEFTCTRL, 0), Run(0), Key(ALT_L, 0)]
        );
    }

    #[test]
    fn a_sequence() {
        let mut e = engine(&["ctrl+space t", "ctrl+space ctrl+b"]);
        let now = Instant::now();
        let out = feed(
            &mut e,
            now,
            &[(CTRL_L, 1), (SPACE, 1), (SPACE, 0), (CTRL_L, 0), (T, 1), (T, 0)],
        );
        assert_eq!(out, [Key(CTRL_L, 1), Key(CTRL_L, 0), Run(0)]);
        let out = feed(
            &mut e,
            now,
            &[(CTRL_L, 1), (SPACE, 1), (SPACE, 0), (B, 1), (B, 0), (CTRL_L, 0)],
        );
        assert_eq!(out, [Key(CTRL_L, 1), Run(1), Key(CTRL_L, 0)]);
        // And it is over: the same key is a key again.
        assert_eq!(feed(&mut e, now, &[(T, 1), (T, 0)]), [Key(T, 1), Key(T, 0)]);
    }

    #[test]
    fn a_wrong_key_ends_a_sequence_and_is_not_typed() {
        let mut e = engine(&["ctrl+space t"]);
        let now = Instant::now();
        feed(&mut e, now, &[(CTRL_L, 1), (SPACE, 1), (SPACE, 0), (CTRL_L, 0)]);
        assert_eq!(feed(&mut e, now, &[(A, 1), (A, 0)]), []);
        assert_eq!(feed(&mut e, now, &[(T, 1), (T, 0)]), [Key(T, 1), Key(T, 0)]);
    }

    #[test]
    fn a_sequence_gives_up() {
        let mut e = engine(&["ctrl+space t"]);
        let now = Instant::now();
        feed(&mut e, now, &[(CTRL_L, 1), (SPACE, 1), (SPACE, 0), (CTRL_L, 0)]);
        assert_eq!(e.deadline(), Some(now + Duration::from_secs(2)));
        let later = now + Duration::from_secs(3);
        assert_eq!(feed(&mut e, later, &[(T, 1), (T, 0)]), [Key(T, 1), Key(T, 0)]);
        assert_eq!(e.deadline(), None);
    }

    #[test]
    fn new_bindings_keep_the_keys_that_are_down() {
        let mut e = engine(&["ctrl+space"]);
        let now = Instant::now();
        feed(&mut e, now, &[(CTRL_L, 1), (SPACE, 1)]);
        e.rebind(Bindings::default(), Duration::from_secs(2));
        assert_eq!(feed(&mut e, now, &[(SPACE, 0), (CTRL_L, 0)]), [Key(CTRL_L, 0)]);
    }

    #[test]
    fn visors_keys_only_while_it_is_in() {
        const SUPER_L: u16 = 125;
        const LEFT: u16 = 105;
        let mut bindings = Bindings::default();
        bindings.add(&parse_sequence("ctrl+space").unwrap(), 0).unwrap();
        bindings.add_while_visor(&parse_sequence("super+left").unwrap(), 1).unwrap();
        let mut e = Engine::new(bindings, Duration::from_secs(2));
        let shown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let asked = shown.clone();
        e.ask_visor(Box::new(move || asked.load(std::sync::atomic::Ordering::SeqCst)));
        let now = Instant::now();
        let press = [(SUPER_L, 1), (LEFT, 1), (LEFT, 0), (SUPER_L, 0)];
        // Away, the window manager has the key.
        assert_eq!(feed(&mut e, now, &press), [Key(SUPER_L, 1), Key(LEFT, 1), Key(LEFT, 0), Key(SUPER_L, 0)]);
        shown.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(feed(&mut e, now, &press), [Key(SUPER_L, 1), Key(KEY_LEFTCTRL, 1), Key(KEY_LEFTCTRL, 0), Run(1), Key(SUPER_L, 0)]);
        // The rest are as ever, in or not.
        shown.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(feed(&mut e, now, &[(CTRL_L, 1), (SPACE, 1), (SPACE, 0), (CTRL_L, 0)]).contains(&Run(0)));
    }

    #[test]
    fn bindings_that_cannot_both_be() {
        let seq = |keys| parse_sequence(keys).unwrap();
        let mut b = Bindings::default();
        b.add(&seq("ctrl+space"), 0).unwrap();
        assert!(b.add(&seq("ctrl+space"), 1).is_err());
        assert!(b.add(&seq("ctrl+space t"), 1).is_err());
        let mut b = Bindings::default();
        b.add(&seq("ctrl+space t"), 0).unwrap();
        assert!(b.add(&seq("ctrl+space"), 1).is_err());
    }
}
