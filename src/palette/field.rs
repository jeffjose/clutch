//! The line that is typed into, with the editing keys of any text box.

use std::io::Write;
use std::process::{Command, Stdio};
use winit::keyboard::{Key, ModifiersState, NamedKey};

pub struct TextField {
    pub text: Vec<char>,
    pub cursor: usize,
    /// Where a selection started; it runs from here to the cursor.
    anchor: Option<usize>,
}

impl TextField {
    pub fn new(text: &str) -> TextField {
        let text: Vec<char> = text.chars().collect();
        TextField { cursor: text.len(), text, anchor: None }
    }

    pub fn string(&self) -> String {
        self.text.iter().collect()
    }

    /// The selected stretch, if there is one: where it starts and ends.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor.filter(|anchor| *anchor != self.cursor)?;
        Some((anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    /// Takes out what is selected. Returns whether anything was.
    fn cut(&mut self) -> bool {
        let selection = self.selection();
        if let Some((from, to)) = selection {
            self.text.drain(from..to);
            self.cursor = from;
        }
        self.anchor = None;
        selection.is_some()
    }

    fn insert(&mut self, typed: &str) {
        self.cut();
        // What is pasted may run over several lines; this is one.
        for c in typed.chars().filter(|c| !c.is_control()) {
            self.text.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// Where the word before, or after, the cursor ends.
    fn word_edge(&self, forward: bool) -> usize {
        let mut at = self.cursor;
        let space = |at: usize| self.text[at].is_whitespace();
        if forward {
            while at < self.text.len() && space(at) {
                at += 1;
            }
            while at < self.text.len() && !space(at) {
                at += 1;
            }
        } else {
            while at > 0 && space(at - 1) {
                at -= 1;
            }
            while at > 0 && !space(at - 1) {
                at -= 1;
            }
        }
        at
    }

    /// Applies an editing key. Returns whether the text changed.
    ///
    /// Beyond typing: the arrows, Home and End move, by a word with Ctrl
    /// and selecting with Shift; Ctrl+A selects everything; Ctrl+C, Ctrl+X
    /// and Ctrl+V copy, cut and paste; Ctrl+Backspace or Ctrl+W takes out
    /// the word before the cursor, and Ctrl+U everything.
    pub fn key(&mut self, key: Key<&str>, modifiers: ModifiersState) -> bool {
        let (ctrl, shift) = (modifiers.control_key(), modifiers.shift_key());
        let typing = !ctrl && !modifiers.alt_key() && !modifiers.super_key();
        let before = self.text.clone();
        // Moving the cursor with Shift held stretches the selection to it.
        let move_to = |field: &mut TextField, to: usize| {
            field.anchor = if shift { field.anchor.or(Some(field.cursor)) } else { None };
            field.cursor = to;
        };
        match key {
            Key::Character(letter) if ctrl => match letter.to_lowercase().as_str() {
                "a" => (self.anchor, self.cursor) = (Some(0), self.text.len()),
                "c" | "x" => {
                    let (from, to) = self.selection().unwrap_or((0, self.text.len()));
                    set_clipboard(&self.text[from..to].iter().collect::<String>());
                    if letter.eq_ignore_ascii_case("x") {
                        self.cut();
                    }
                }
                "v" => self.insert(&clipboard().unwrap_or_default()),
                "u" => (self.text, self.cursor, self.anchor) = (Vec::new(), 0, None),
                "w" => {
                    self.anchor = Some(self.word_edge(false));
                    self.cut();
                }
                _ => {}
            },
            Key::Named(NamedKey::Backspace | NamedKey::Delete) if self.cut() => {}
            Key::Named(NamedKey::Backspace) if ctrl => {
                self.anchor = Some(self.word_edge(false));
                self.cut();
            }
            Key::Named(NamedKey::Backspace) if self.cursor > 0 => {
                self.cursor -= 1;
                self.text.remove(self.cursor);
            }
            Key::Named(NamedKey::Delete) if self.cursor < self.text.len() => {
                self.text.remove(self.cursor);
            }
            Key::Named(NamedKey::Space) if typing => self.insert(" "),
            Key::Character(typed) if typing => self.insert(typed),
            Key::Named(NamedKey::ArrowLeft) if ctrl => move_to(self, self.word_edge(false)),
            Key::Named(NamedKey::ArrowRight) if ctrl => move_to(self, self.word_edge(true)),
            Key::Named(NamedKey::ArrowLeft) => move_to(self, self.cursor.saturating_sub(1)),
            Key::Named(NamedKey::ArrowRight) => move_to(self, (self.cursor + 1).min(self.text.len())),
            Key::Named(NamedKey::Home) => move_to(self, 0),
            Key::Named(NamedKey::End) => move_to(self, self.text.len()),
            _ => {}
        }
        self.text != before
    }
}

/// Puts text on the clipboard, by way of whichever helper is installed.
fn set_clipboard(text: &str) -> bool {
    let helpers: [&[&str]; 3] =
        [&["xclip", "-selection", "clipboard"], &["xsel", "--clipboard", "--input"], &["wl-copy"]];
    helpers.iter().any(|helper| {
        let child = Command::new(helper[0])
            .args(&helper[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return false };
        let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        written && child.wait().is_ok_and(|status| status.success())
    })
}

fn clipboard() -> Option<String> {
    let helpers: [&[&str]; 3] =
        [&["xclip", "-selection", "clipboard", "-o"], &["xsel", "--clipboard", "--output"], &["wl-paste", "-n"]];
    helpers.iter().find_map(|helper| {
        let output = Command::new(helper[0]).args(&helper[1..]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    })
}
