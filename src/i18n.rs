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

/// Language for a locale name such as `pl-PL` or `en-US`; anything but Polish gets English.
pub fn language_from_locale(locale: &str) -> Language {
    let primary = locale.split(['-', '_']).next().unwrap_or("");
    if primary.eq_ignore_ascii_case("pl") {
        Language::Polish
    } else {
        Language::English
    }
}

/// The Windows display language, used on first start before the user has chosen one.
pub fn system_language() -> Language {
    #[cfg(windows)]
    {
        let mut buf = [0u16; 85];
        // SAFETY: the buffer is 85 u16 long (LOCALE_NAME_MAX_LENGTH) and the length is passed along.
        let len = unsafe {
            windows_sys::Win32::Globalization::GetUserDefaultLocaleName(
                buf.as_mut_ptr(),
                buf.len() as i32,
            )
        };
        if len > 1 {
            return language_from_locale(&String::from_utf16_lossy(&buf[..len as usize - 1]));
        }
    }
    Language::English
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
    fn locale_names_map_to_a_language() {
        assert_eq!(language_from_locale("pl-PL"), Language::Polish);
        assert_eq!(language_from_locale("pl"), Language::Polish);
        assert_eq!(language_from_locale("PL_pl"), Language::Polish);
        assert_eq!(language_from_locale("en-US"), Language::English);
        assert_eq!(language_from_locale("de-DE"), Language::English);
        assert_eq!(language_from_locale(""), Language::English);
    }

    #[test]
    fn language_survives_a_config_roundtrip() {
        for language in Language::ALL {
            let json = serde_json::to_string(&language).unwrap();
            assert_eq!(serde_json::from_str::<Language>(&json).unwrap(), language);
        }
    }
}
