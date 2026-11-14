//! The interface locale is immutable until the next process start.
use std::sync::OnceLock;
use throne_config::settings::Language;

static LANGUAGE: OnceLock<Language> = OnceLock::new();

pub fn init(language: Language) {
    LANGUAGE
        .set(language)
        .expect("interface language already initialized");
}

pub fn language() -> Language {
    LANGUAGE.get().copied().unwrap_or_default()
}

pub fn tr<'a>(ru: &'a str, en: &'a str) -> &'a str {
    select(language(), ru, en)
}

fn select<'a>(language: Language, ru: &'a str, en: &'a str) -> &'a str {
    match language {
        Language::Ru => ru,
        Language::En => en,
    }
}

#[macro_export]
macro_rules! tr_format {
    ($ru:literal, $en:literal $(, $($args:tt)*)?) => {
        match $crate::i18n::language() {
            throne_config::settings::Language::Ru => format!($ru $(, $($args)*)?),
            throne_config::settings::Language::En => format!($en $(, $($args)*)?),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_languages() {
        assert_eq!(select(Language::Ru, "да", "yes"), "да");
        assert_eq!(select(Language::En, "да", "yes"), "yes");
        let count = 2;
        assert_eq!(crate::tr_format!("{count}: {}", "{count}: {}", 3), "2: 3");
    }
}
