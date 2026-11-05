//! English or Spanish, by the language of Windows.

use std::env;
use std::sync::OnceLock;

unsafe extern "system" {
    /// The language of Windows' own windows (not the regional format), as a LANGID.
    fn GetUserDefaultUILanguage() -> u16;
}

/// LANGID of a language is its primary language in the low ten bits; 0x0a is Spanish.
fn is_spanish_langid(langid: u16) -> bool {
    langid & 0x03ff == 0x000a
}

/// Whether the messages are in Spanish. `HERDR_REPAIR_LANG=es|en` forces one, to try both.
pub fn spanish() -> bool {
    static SPANISH: OnceLock<bool> = OnceLock::new();
    *SPANISH.get_or_init(|| match env::var("HERDR_REPAIR_LANG").as_deref() {
        Ok("es") => true,
        Ok("en") => false,
        // SAFETY: no arguments, no pointers: it only reads the user's settings.
        _ => is_spanish_langid(unsafe { GetUserDefaultUILanguage() }),
    })
}

/// A message in both languages; the arguments go to the one that is used (like `format!`).
macro_rules! t {
    ($en:literal, $es:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::spanish() { format!($es $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

/// What counts as a yes, in either language.
pub fn is_yes(answer: &str) -> bool {
    matches!(answer.to_lowercase().as_str(), "yes" | "y" | "si" | "sí" | "s")
}

pub fn is_edit(answer: &str) -> bool {
    matches!(answer.to_lowercase().as_str(), "edit" | "e" | "editar")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanish_is_told_from_any_of_its_regions() {
        assert!(is_spanish_langid(0x0c0a)); // es-ES
        assert!(is_spanish_langid(0x080a)); // es-MX
        assert!(!is_spanish_langid(0x0409)); // en-US
    }

    #[test]
    fn answers_in_both_languages() {
        assert!(is_yes("Sí") && is_yes("yes") && is_yes("S") && !is_yes("no") && !is_yes(""));
        assert!(is_edit("editar") && is_edit("edit") && !is_edit("yes"));
    }
}
