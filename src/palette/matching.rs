//! Matching what is typed against the names of things.

/// How well `text` matches what was typed, higher being better, or `None`
/// if it does not. Every word typed must be in it. With `loose`, a word's
/// letters may be scattered through the text in order; without, they must
/// be together.
pub fn score(typed: &str, text: &str, loose: bool) -> Option<i32> {
    let text = text.to_lowercase();
    let mut total = 0;
    for word in typed.split_whitespace() {
        let letters: Vec<char> = word.to_lowercase().chars().collect();
        total += fuzzy(&letters, &text, loose)?;
    }
    Some(total)
}

/// Scores `letters` within `name`: together as typed, or, when `loose`,
/// scattered through it in that order. Runs of adjacent letters and
/// matches at the start of the name or of a word count for more, so that
/// the closest names come first.
fn fuzzy(letters: &[char], name: &str, loose: bool) -> Option<i32> {
    let name: Vec<char> = name.chars().collect();
    let word_start = |at: usize| at == 0 || !name[at - 1].is_alphanumeric();

    // The letters together, as typed, beat any scattered match.
    let together = name.windows(letters.len()).position(|window| window == letters);
    if let Some(at) = together {
        let place = if at == 0 { 60 } else if word_start(at) { 30 } else { 0 };
        return Some(100 + place + 10 * letters.len() as i32);
    }

    if !loose {
        return None;
    }
    let (mut score, mut wanted, mut previous) = (0, 0, None);
    for (at, letter) in name.iter().enumerate() {
        if wanted < letters.len() && *letter == letters[wanted] {
            score += 10;
            if previous == Some(at.wrapping_sub(1)) {
                score += 15;
            }
            if word_start(at) {
                score += 10;
            }
            previous = Some(at);
            wanted += 1;
        }
    }
    (wanted == letters.len()).then_some(score)
}

/// The places of the things that match, best first; those that tie stay
/// in the order they came.
pub fn in_order<'a>(typed: &str, texts: impl Iterator<Item = &'a str>, loose: bool) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> =
        texts.enumerate().filter_map(|(at, text)| Some((score(typed, text, loose)?, at))).collect();
    scored.sort_by_key(|(score, at)| (-score, *at));
    scored.into_iter().map(|(_, at)| at).collect()
}

/// `in_order`, with scattered letters counting only when nothing has
/// them together.
pub fn ranked<'a>(typed: &str, texts: impl Iterator<Item = &'a str> + Clone) -> Vec<usize> {
    let together = in_order(typed, texts.clone(), false);
    if together.is_empty() { in_order(typed, texts, true) } else { together }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[&str] = &["Firefox", "Files", "Mozilla Firefox — clutch", "Terminal", "Text Editor"];

    fn matches(typed: &str) -> Vec<&'static str> {
        ranked(typed, NAMES.iter().copied()).into_iter().map(|at| NAMES[at]).collect()
    }

    #[test]
    fn the_closest_first() {
        assert_eq!(matches("fire"), ["Firefox", "Mozilla Firefox — clutch"]);
        assert_eq!(matches("te"), ["Terminal", "Text Editor"]);
        assert_eq!(matches("FIRE clutch"), ["Mozilla Firefox — clutch"]);
        assert!(matches("zzz").is_empty());
    }

    #[test]
    fn scattered_letters_only_when_nothing_has_them_together() {
        assert_eq!(matches("fls"), ["Files"]);
        assert_eq!(matches("txed"), ["Text Editor"]);
    }

    #[test]
    fn nothing_typed_is_everything_in_order() {
        assert_eq!(matches(" "), NAMES);
    }
}
