//! Text as the voice should say it: spelled-out words letter by letter.

/// Accept actual separated letters, not a model's guess about a word.
pub fn letters(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches(['.', '!', '?']);
    let words: Vec<_> =
        text.split(|c: char| c.is_whitespace() || matches!(c, '-' | ',' | '.')).filter(|w| !w.is_empty()).collect();
    (words.len() >= 2 && words.len() <= 40 && words.iter().all(|w| w.len() == 1 && w.as_bytes()[0].is_ascii_alphabetic()))
        .then(|| words.concat().to_uppercase())
}

/// Only isolated spelling runs are converted; ordinary prose and acronyms stay intact.
pub fn letter_name(letter: u8) -> &'static str {
    const NAMES: [&str; 26] = [
        "ay", "bee", "see", "dee", "ee", "eff", "jee", "aitch", "eye", "jay", "kay", "ell", "em", "en", "oh", "pee", "cue",
        "ar", "ess", "tee", "you", "vee", "double you", "ex", "why", "zee",
    ];
    NAMES[(letter.to_ascii_uppercase() - b'A') as usize]
}

pub fn for_speech(text: &str) -> String {
    text.split_whitespace()
        .map(|token| {
            let bare = token.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | ';' | '!' | '?' | '"'));
            if bare.contains('-') {
                if let Some(word) = letters(bare) {
                    return word.bytes().map(letter_name).collect::<Vec<_>>().join(". ") + ".";
                }
            }
            token.into()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spelling_is_said_letter_by_letter() {
        assert_eq!(for_speech("C-A-L-E-R-A"), "see. ay. ell. ee. ar. ay.");
        assert_eq!(for_speech("NASA is well-known."), "NASA is well-known.");
        assert_eq!(letters("regular words"), None);
    }
}
