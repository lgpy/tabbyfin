//! The interface's language: Fluent translations in `crates/app/i18n/<language>/tabbyfin.ftl`,
//! embedded in the binary. Text goes through [`tr!`], which `fl!` checks against the English file
//! when it compiles (message ids and their arguments) and looks up in the current language at
//! run time, falling back to English for a message a translation lacks.
//!
//! The loader is the one piece of process-wide state the app has: every widget, and messages
//! made on the runtime's threads, read it, and egui draws each frame anew, so a change of
//! language shows on the next frame. Text kept from an earlier frame (a toast, a saved error)
//! stays in the language it was made in.

use std::sync::LazyLock;

use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::unic_langid::LanguageIdentifier;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Translations;

/// Read by [`tr!`]; [`apply`] picks its language.
pub static LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader = fluent_language_loader!();
    select(&loader, Language::English);
    loader
});

/// Loads `language`'s messages into `loader`, over English's.
fn select(loader: &FluentLanguageLoader, language: Language) {
    let id: LanguageIdentifier = language.id().parse().expect("a valid language id");
    if let Err(err) = i18n_embed::select(loader, &Translations, &[id]) {
        tracing::warn!("couldn't switch the interface to {}: {err}", language.id());
    }
    // Fluent wraps arguments in bidi isolation marks, which the app's fonts draw as boxes; no
    // language the app has is right to left. Loading a language resets it.
    loader.set_use_isolating(false);
}

/// The text of a message in the interface's language, as a `String`: `tr!("message-id")`, or
/// `tr!("message-id", name = value)` for one with arguments.
macro_rules! tr {
    ($id:literal) => {
        i18n_embed_fl::fl!($crate::i18n::LOADER, $id)
    };
    ($id:literal, $($args:tt)*) => {
        i18n_embed_fl::fl!($crate::i18n::LOADER, $id, $($args)*)
    };
}
pub(crate) use tr;

/// The languages the interface has. Adding one is a variant here and its own
/// `i18n/<id>/tabbyfin.ftl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "pt-PT")]
    PortuguesePortugal,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::PortuguesePortugal];

    /// Its translations' folder, and the language id Fluent's plural rules go by.
    pub fn id(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::PortuguesePortugal => "pt-PT",
        }
    }

    /// Its name in itself, as language pickers show them: whoever looks for it may not read the
    /// language the interface is in.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::PortuguesePortugal => "Português (Portugal)",
        }
    }

    /// The language that stands in for a system locale (`pt-BR`, `en_US.UTF-8`, `pt`): ours in
    /// the same language. Any Portuguese gets Portugal's, the only one there is.
    fn for_locale(locale: &str) -> Option<Language> {
        let language = locale.split(['-', '_', '.', '@']).next()?.to_ascii_lowercase();
        Language::ALL
            .into_iter()
            .find(|ours| ours.id().split('-').next() == Some(language.as_str()))
    }

    /// The first of the system's preferred languages the app has, else English. Tests always
    /// get English, whatever the machine running them speaks.
    pub fn system() -> Language {
        if cfg!(test) {
            return Language::English;
        }
        sys_locale::get_locales()
            .find_map(|locale| Language::for_locale(&locale))
            .unwrap_or(Language::English)
    }
}

/// The interface's language for a config's `language`; `None` follows the system.
pub fn resolve(setting: Option<Language>) -> Language {
    setting.unwrap_or_else(Language::system)
}

/// Switches the interface to `language`, from the next frame on.
pub fn apply(language: Language) {
    select(&LOADER, language);
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{Language, Translations};

    fn file(language: Language) -> String {
        let path = format!("{}/tabbyfin.ftl", language.id());
        let file = Translations::get(&path).unwrap_or_else(|| panic!("{path} isn't embedded"));
        String::from_utf8(file.data.into_owned()).unwrap()
    }

    /// Every message of a Fluent file, with the variables its text names (`$count`).
    fn messages(source: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut messages: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in source.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let text = if line.starts_with([' ', '\t']) {
                line
            } else {
                let (id, text) = line.split_once('=').unwrap_or_else(|| panic!("not a message: {line}"));
                let id = id.trim().to_owned();
                messages.entry(id.clone()).or_default();
                current = Some(id);
                text
            };
            let id = current.as_ref().expect("a message before its continuation lines");
            let variables = messages.get_mut(id).unwrap();
            for piece in text.split('$').skip(1) {
                let name: String = piece
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                variables.insert(name);
            }
        }
        messages
    }

    /// Every translation has each English message, with the same arguments, and none English
    /// lacks: a missing one would show in English, a stale one never shows.
    #[test]
    fn translations_match_the_english_messages() {
        let english = messages(&file(Language::English));
        assert!(!english.is_empty());
        for language in Language::ALL.into_iter().filter(|&l| l != Language::English) {
            let translated = messages(&file(language));
            let missing: Vec<_> = english.keys().filter(|id| !translated.contains_key(*id)).collect();
            let stale: Vec<_> = translated.keys().filter(|id| !english.contains_key(*id)).collect();
            assert!(missing.is_empty(), "{} lacks {missing:?}", language.id());
            assert!(
                stale.is_empty(),
                "{} has messages English doesn't: {stale:?}",
                language.id()
            );
            for (id, variables) in &english {
                assert_eq!(
                    &translated[id],
                    variables,
                    "{}'s {id} names other arguments",
                    language.id()
                );
            }
        }
    }

    #[test]
    fn follows_the_system_language() {
        assert_eq!(Language::for_locale("pt-PT"), Some(Language::PortuguesePortugal));
        assert_eq!(Language::for_locale("pt_BR.UTF-8"), Some(Language::PortuguesePortugal));
        assert_eq!(Language::for_locale("en-US"), Some(Language::English));
        assert_eq!(Language::for_locale("de-DE"), None);
    }

    #[test]
    fn reads_a_translation() {
        // The tests share the app's loader, so this one has its own.
        let loader = i18n_embed::fluent::fluent_language_loader!();
        super::select(&loader, Language::PortuguesePortugal);
        let args = std::collections::HashMap::from([("language", "English")]);
        assert_eq!(
            loader.get_args("settings-language-system", args),
            "Seguir o sistema (English)"
        );
    }
}
