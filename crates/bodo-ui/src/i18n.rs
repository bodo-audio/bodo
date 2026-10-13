//! Translations of the interface (project.md section 14.7, ADR 0033).
//!
//! Every user-facing string is written in English in the code and goes through [`t`], or [`tf!`](crate::tf) when it
//! has values in it: `t("Save")`, `tf!("Delete {name}", name = channel.name)`. The English text is the key. Each
//! language has a catalog, `crates/bodo-ui/i18n/<code>.tsv`, of `English<TAB>translation` lines, built into the app;
//! a string a catalog lacks shows in English. Placeholders such as `{name}` keep their names in translations, which
//! may move them. Tests check that every key the code uses is in every catalog, that no catalog keeps stale keys, and
//! that each translation has its key's placeholders.
//!
//! Helpers that take English text and show it, such as [`crate::BodoApp::hint`] and [`crate::windows::icon_button`],
//! translate it themselves; labels recorded for the control channel stay in English, so agents and tests find the
//! same elements in every language.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;

/// A language the interface speaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Language {
    /// English, the language of the code and the fallback.
    #[default]
    English,
    /// Turkish.
    Turkish,
}

impl Language {
    /// Every language, in menu order.
    pub const ALL: [Self; 2] = [Self::English, Self::Turkish];

    /// Its code, the catalog's file name: `en`, `tr`.
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Turkish => "tr",
        }
    }

    /// Its name in itself, as language menus show it.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Turkish => "Türkçe",
        }
    }

    /// The language a locale such as `tr-TR` or `en_US.UTF-8` names, English for any other.
    pub fn from_locale(locale: &str) -> Self {
        let code = locale.split(['-', '_', '.']).next().unwrap_or_default().to_ascii_lowercase();
        Self::ALL.into_iter().find(|l| l.code() == code).unwrap_or(Self::English)
    }

    fn index(self) -> usize {
        match self {
            Self::English => 0,
            Self::Turkish => 1,
        }
    }

    /// Its catalog's text; English has none, being the keys.
    fn source(self) -> Option<&'static str> {
        match self {
            Self::English => None,
            Self::Turkish => Some(include_str!("../i18n/tr.tsv")),
        }
    }
}

/// The language the user chose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguagePref {
    /// English.
    #[default]
    English,
    /// Turkish.
    Turkish,
    /// The operating system's language, when Bodo speaks it, else English.
    System,
}

impl LanguagePref {
    /// Every choice, in menu order.
    pub const ALL: [Self; 3] = [Self::System, Self::English, Self::Turkish];

    /// Its name in menus, in the language itself; System's goes through [`t`].
    pub fn label(self) -> &'static str {
        match self {
            Self::English => Language::English.native_name(),
            Self::Turkish => Language::Turkish.native_name(),
            Self::System => t("System language"),
        }
    }

    /// Its id in `ui.set_language`: `system`, `en`, `tr`.
    pub fn id(self) -> &'static str {
        match self {
            Self::English => Language::English.code(),
            Self::Turkish => Language::Turkish.code(),
            Self::System => "system",
        }
    }

    /// The choice `id` names, as [`Self::id`] writes it, or a language's name.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(id) || p.label().eq_ignore_ascii_case(id))
    }

    /// The language to show, asking the operating system for System.
    pub fn resolve(self) -> Language {
        match self {
            Self::English => Language::English,
            Self::Turkish => Language::Turkish,
            Self::System => sys_locale::get_locale().map_or(Language::English, |l| Language::from_locale(&l)),
        }
    }
}

thread_local! {
    /// The language of the interface on this thread. The interface draws on one thread (detached windows too), and
    /// tests that run side by side each keep their own.
    static CURRENT: Cell<Language> = const { Cell::new(Language::English) };
}

/// Makes `language` the one [`t`] translates into on this thread.
pub fn set_language(language: Language) {
    CURRENT.with(|current| current.set(language));
}

/// The language [`t`] translates into on this thread.
pub fn language() -> Language {
    CURRENT.with(Cell::get)
}

/// A parsed catalog: English key to translation.
pub type Catalog = HashMap<String, &'static str>;

/// `language`'s catalog, parsed on first use; `None` for English.
pub fn catalog(language: Language) -> Option<&'static Catalog> {
    static CATALOGS: [OnceLock<Catalog>; 2] = [OnceLock::new(), OnceLock::new()];
    let source = language.source()?;
    Some(CATALOGS[language.index()].get_or_init(|| parse(source)))
}

/// Reads a catalog: `English<TAB>translation` per line, `#` comments and blank lines skipped, with `\t`, `\n` and
/// `\\` escapes. The first line is the header, `en<TAB><code>`.
///
/// Complexity: O(n) in the catalog's length.
pub fn parse(source: &'static str) -> Catalog {
    let mut catalog = Catalog::new();
    for line in source.lines().skip(1) {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('\t') else { continue };
        let value: &'static str = if value.contains('\\') { Box::leak(unescape(value).into_boxed_str()) } else { value };
        catalog.insert(unescape(key), value);
    }
    catalog
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Marks English text that is translated where it shows rather than here, such as an enum's name that is also an id
/// (`ThemePref::label`). It returns `text` unchanged; the catalog checks find the text through it.
pub const fn en(text: &'static str) -> &'static str {
    text
}

/// `key`, English text, in the interface's language; `key` itself when the catalog lacks it.
///
/// Complexity: one hash lookup.
pub fn t(key: &'static str) -> &'static str {
    lookup(key).unwrap_or(key)
}

/// `key`'s translation, if the interface's language has one: for text built at run time from English pieces the
/// catalogs hold, such as a label a helper receives.
pub fn lookup(key: &str) -> Option<&'static str> {
    catalog(language()).and_then(|c| c.get(key).copied())
}

/// `key` in the interface's language, or `key` itself, owned.
pub fn tr(key: &str) -> String {
    lookup(key).map_or_else(|| key.to_owned(), str::to_owned)
}

/// Translates English text with placeholders and fills them in: `tf!("Delete {name}", name = channel.name)`.
#[macro_export]
macro_rules! tf {
    ($key:literal $(, $name:ident = $value:expr)* $(,)?) => {{
        #[allow(unused_mut, reason = "a key without placeholders fills nothing in")]
        let mut text = ::std::string::String::from($crate::i18n::t($key));
        $(
            text = text.replace(concat!("{", stringify!($name), "}"), &::std::string::ToString::to_string(&$value));
        )*
        text
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_parse_with_escapes_and_fall_back_to_english() {
        let catalog = parse("en\txx\n# a comment\nSave\tKaydet\nTwo\\tparts\tİki\\nsatır\n");
        assert_eq!(catalog.get("Save"), Some(&"Kaydet"));
        assert_eq!(catalog.get("Two\tparts"), Some(&"İki\nsatır"));
        assert_eq!(catalog.len(), 2);
    }

    #[test]
    fn locales_name_their_language() {
        assert_eq!(Language::from_locale("tr-TR"), Language::Turkish);
        assert_eq!(Language::from_locale("tr_TR.UTF-8"), Language::Turkish);
        assert_eq!(Language::from_locale("en-GB"), Language::English);
        assert_eq!(Language::from_locale("de-DE"), Language::English, "a language Bodo doesn't speak falls back");
        assert_eq!(LanguagePref::from_id("tr"), Some(LanguagePref::Turkish));
        assert_eq!(LanguagePref::from_id("Türkçe"), Some(LanguagePref::Turkish));
    }
}
