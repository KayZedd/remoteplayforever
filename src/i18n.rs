//! Interface language. English is the default; the choice is stored in the config.
//!
//! Texts live next to the code that shows them: `tr("English", "Polski")`.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    English,
    Polish,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::Polish];

    /// Name of the language in that language.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Polish => "Polski",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(Language::English as u8);

pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    if CURRENT.load(Ordering::Relaxed) == Language::Polish as u8 {
        Language::Polish
    } else {
        Language::English
    }
}

fn pick<'a>(language: Language, en: &'a str, pl: &'a str) -> &'a str {
    match language {
        Language::English => en,
        Language::Polish => pl,
    }
}

/// The text for the current interface language.
pub fn tr<'a>(en: &'a str, pl: &'a str) -> &'a str {
    pick(language(), en, pl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_the_default() {
        assert_eq!(Language::default(), Language::English);
        assert_eq!(pick(Language::default(), "Play", "Graj"), "Play");
    }

    #[test]
    fn pick_follows_the_language() {
        assert_eq!(pick(Language::Polish, "Play", "Graj"), "Graj");
        assert_eq!(pick(Language::English, "Play", "Graj"), "Play");
    }

    #[test]
    fn language_survives_a_config_roundtrip() {
        for language in Language::ALL {
            let json = serde_json::to_string(&language).unwrap();
            assert_eq!(serde_json::from_str::<Language>(&json).unwrap(), language);
        }
    }
}
