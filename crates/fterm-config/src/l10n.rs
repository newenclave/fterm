//! The language of the UI. English is built in (`assets/l10n/en.json`); another language is a JSON file with the
//! same keys. A key that the file does not have stays English.
//!
//! ```json
//! { "language": "ru", "name": "Русский",
//!   "strings": { "action.new_tab": "Новая вкладка",
//!                "close.tabs": { "one": "{n} вкладка", "few": "{n} вкладки", "many": "{n} вкладок" } } }
//! ```

use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use serde_json::Value;

/// The English text, built in.
const ENGLISH_JSON: &str = include_str!("../../../assets/l10n/en.json");

/// One text: plain, or a form for each plural category (`one`, `few`, `many`, `other`).
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Text(String),
    Plural(BTreeMap<String, String>),
}

/// The texts of one language.
#[derive(Clone, Debug, PartialEq)]
pub struct Strings {
    /// The language code, for example `ru` (it chooses the plural rule).
    pub language: String,
    /// The name of the language, for example `Русский`.
    pub name: String,
    table: HashMap<String, Entry>,
}

static ENGLISH: LazyLock<Strings> =
    LazyLock::new(|| Strings::parse_table(ENGLISH_JSON).expect("assets/l10n/en.json"));

static CURRENT: LazyLock<RwLock<Arc<Strings>>> =
    LazyLock::new(|| RwLock::new(Arc::new(ENGLISH.clone())));

/// The language in use (English until `set`).
pub fn current() -> Arc<Strings> {
    CURRENT
        .read()
        .map_or_else(|e| e.into_inner().clone(), |s| s.clone())
}

/// Uses `strings` from now on.
pub fn set(strings: Strings) {
    let strings = Arc::new(strings);
    match CURRENT.write() {
        Ok(mut s) => *s = strings,
        Err(e) => *e.into_inner() = strings,
    }
}

/// The built-in English.
pub fn english() -> Strings {
    ENGLISH.clone()
}

/// All keys of the English table, and if each is a plural.
pub fn english_keys() -> Vec<(String, bool)> {
    let mut keys: Vec<(String, bool)> = ENGLISH
        .table
        .iter()
        .map(|(k, e)| (k.clone(), matches!(e, Entry::Plural(_))))
        .collect();
    keys.sort();
    keys
}

/// The plural category of `n` in `language` (CLDR, for whole numbers).
pub fn plural_category(language: &str, n: u64) -> &'static str {
    let lang = language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (n10, n100) = (n % 10, n % 100);
    match lang.as_str() {
        "ru" | "uk" | "be" => {
            if n10 == 1 && n100 != 11 {
                "one"
            } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                "few"
            } else {
                "many"
            }
        }
        "pl" => {
            if n == 1 {
                "one"
            } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                "few"
            } else {
                "many"
            }
        }
        "cs" | "sk" => match n {
            1 => "one",
            2..=4 => "few",
            _ => "other",
        },
        "ja" | "zh" | "ko" | "vi" | "th" | "id" => "other",
        "fr" | "pt" => {
            if n <= 1 {
                "one"
            } else {
                "other"
            }
        }
        _ => {
            if n == 1 {
                "one"
            } else {
                "other"
            }
        }
    }
}

/// `{name}` in `text` becomes the value of `name`; an unknown name stays as it is.
fn fill(text: &str, args: &[(&str, &dyn Display)]) -> String {
    if args.is_empty() || !text.contains('{') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match args.iter().find(|(k, _)| *k == name) {
                    Some((_, value)) => out.push_str(&value.to_string()),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The `{names}` in a text.
fn placeholders(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        names.push(after[..end].to_owned());
        rest = &after[end + 1..];
    }
    names
}

impl Strings {
    /// Reads a language file, with no checks against English.
    fn parse_table(json: &str) -> Result<Self, String> {
        let root: Value =
            serde_json::from_str(json.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
        let text_of = |key: &str| -> Result<String, String> {
            match root.get(key) {
                Some(Value::String(s)) => Ok(s.clone()),
                Some(_) => Err(format!("`{key}` must be a text")),
                None => Err(format!("`{key}` is missing")),
            }
        };
        let language = text_of("language")?;
        let name = text_of("name").unwrap_or_else(|_| language.clone());
        let mut table = HashMap::new();
        match root.get("strings") {
            None => {}
            Some(Value::Object(map)) => {
                for (key, value) in map {
                    let entry = match value {
                        Value::String(s) => Entry::Text(s.clone()),
                        Value::Object(forms) => {
                            let mut out = BTreeMap::new();
                            for (form, text) in forms {
                                if !["zero", "one", "two", "few", "many", "other"]
                                    .contains(&form.as_str())
                                {
                                    return Err(format!(
                                        "strings.{key}.{form}: a plural form is zero, one, two, few, many, or other"
                                    ));
                                }
                                let Value::String(text) = text else {
                                    return Err(format!("strings.{key}.{form} must be a text"));
                                };
                                out.insert(form.clone(), text.clone());
                            }
                            Entry::Plural(out)
                        }
                        _ => return Err(format!("strings.{key} must be a text or plural forms")),
                    };
                    table.insert(key.clone(), entry);
                }
            }
            Some(_) => return Err("`strings` must be an object".to_owned()),
        }
        Ok(Self {
            language,
            name,
            table,
        })
    }

    /// Reads a language file. The warnings say what is wrong but not fatal: keys that English does not have, a
    /// plain text where English has plural forms (or the other way), and `{names}` that English does not have.
    pub fn parse(json: &str) -> Result<(Self, Vec<String>), String> {
        let strings = Self::parse_table(json)?;
        let mut warnings = Vec::new();
        let mut keys: Vec<&String> = strings.table.keys().collect();
        keys.sort();
        for key in keys {
            let entry = &strings.table[key];
            let Some(english) = ENGLISH.table.get(key) else {
                warnings.push(format!("unknown key `{key}`"));
                continue;
            };
            let texts = |e: &Entry| -> Vec<String> {
                match e {
                    Entry::Text(t) => vec![t.clone()],
                    Entry::Plural(forms) => forms.values().cloned().collect(),
                }
            };
            match (english, entry) {
                (Entry::Text(_), Entry::Plural(_)) => {
                    warnings.push(format!("`{key}` must be a text, not plural forms"));
                    continue;
                }
                (Entry::Plural(_), Entry::Text(_)) => {
                    warnings.push(format!("`{key}` must be plural forms"));
                    continue;
                }
                _ => {}
            }
            let known: Vec<String> = texts(english)
                .iter()
                .flat_map(|t| placeholders(t))
                .collect();
            for text in texts(entry) {
                for name in placeholders(&text) {
                    if !known.contains(&name) {
                        warnings.push(format!("`{key}`: unknown {{{name}}}"));
                    }
                }
            }
        }
        Ok((strings, warnings))
    }

    fn entry(&self, key: &str) -> Option<&Entry> {
        self.table.get(key).or_else(|| ENGLISH.table.get(key))
    }

    /// The text of `key`, with `{names}` filled in. English when this language does not have it; the key itself
    /// when nobody has it.
    pub fn format(&self, key: &str, args: &[(&str, &dyn Display)]) -> String {
        match self.entry(key) {
            Some(Entry::Text(text)) => fill(text, args),
            Some(Entry::Plural(_)) => self.plural(key, 1, args),
            None => key.to_owned(),
        }
    }

    /// The text of `key` for the number `n` (also `{n}`).
    pub fn plural(&self, key: &str, n: u64, args: &[(&str, &dyn Display)]) -> String {
        let pick = |strings: &Strings, entry: &Entry| -> Option<String> {
            match entry {
                Entry::Plural(forms) => {
                    let category = plural_category(&strings.language, n);
                    forms
                        .get(category)
                        .or_else(|| forms.get("other"))
                        .or_else(|| forms.values().next())
                        .cloned()
                }
                Entry::Text(text) => Some(text.clone()),
            }
        };
        let text = self
            .table
            .get(key)
            .and_then(|e| pick(self, e))
            .or_else(|| ENGLISH.table.get(key).and_then(|e| pick(&ENGLISH, e)));
        let Some(text) = text else {
            return key.to_owned();
        };
        let mut all: Vec<(&str, &dyn Display)> = vec![("n", &n)];
        all.extend_from_slice(args);
        fill(&text, &all)
    }
}

/// How the user chose the language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice<'a> {
    /// No `language`, or `"en"`.
    English,
    /// The language of the system (`"system"`), for example `ru-RU`. `None` = unknown.
    System(Option<&'a str>),
    /// A language code, for example `ru`.
    Code(&'a str),
    /// A path to a `.json` file.
    File(&'a Path),
}

impl<'a> Choice<'a> {
    /// The choice for `language` in the config.
    pub fn of(language: Option<&'a str>, system: Option<&'a str>) -> Self {
        match language.map(str::trim) {
            None | Some("") => Self::English,
            Some(l) if l.eq_ignore_ascii_case("system") => Self::System(system),
            Some(l) if l.to_ascii_lowercase().ends_with(".json") || l.contains(['/', '\\']) => {
                Self::File(Path::new(l))
            }
            Some(l) if l.eq_ignore_ascii_case("en") => Self::English,
            Some(l) => Self::Code(l),
        }
    }
}

/// The texts for a choice: a file in `dirs` (`<code>.json`; for `ru-RU` also `ru.json`), else English.
/// `Err` = a file that cannot be read. The warnings come from `Strings::parse`.
pub fn load(choice: &Choice<'_>, dirs: &[PathBuf]) -> Result<(Strings, Vec<String>), String> {
    let read = |path: &Path| -> Result<(Strings, Vec<String>), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Strings::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    };
    let code = match choice {
        Choice::English => return Ok((english(), Vec::new())),
        Choice::File(path) => return read(path),
        Choice::System(None) => return Ok((english(), Vec::new())),
        Choice::System(Some(code)) | Choice::Code(code) => *code,
    };
    let mut names = vec![code.to_owned()];
    if let Some(base) = code.split(['-', '_']).next()
        && base != code
    {
        names.push(base.to_owned());
    }
    for name in &names {
        if name.eq_ignore_ascii_case("en") {
            return Ok((english(), Vec::new()));
        }
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let found = entries.flatten().map(|e| e.path()).find(|p| {
                p.extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("json"))
                    && p.file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.eq_ignore_ascii_case(name))
            });
            if let Some(path) = found {
                return read(&path);
            }
        }
    }
    match choice {
        // The system language has no file: English, with no fuss.
        Choice::System(_) => Ok((english(), Vec::new())),
        _ => Err(format!(
            "no language file `{code}.json` in the l10n folders"
        )),
    }
}

/// A UI text: `tr!("key")`, or `tr!("key", name = value, ...)` for `{name}` in it.
#[macro_export]
macro_rules! tr {
    ($key:literal) => {
        $crate::l10n::current().format($key, &[])
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::l10n::current().format(
            $key,
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    };
}

/// A UI text for a number: `trn!("key", n)`, or `trn!("key", n, name = value, ...)`. `{n}` is the number.
#[macro_export]
macro_rules! trn {
    ($key:literal, $n:expr) => {
        $crate::l10n::current().plural($key, ($n) as u64, &[])
    };
    ($key:literal, $n:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::l10n::current().plural(
            $key,
            ($n) as u64,
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn russian(strings: &str) -> Strings {
        Strings::parse_table(&format!(
            r#"{{ "language": "ru", "name": "Русский", "strings": {strings} }}"#
        ))
        .unwrap()
    }

    #[test]
    fn plural_rules() {
        let ru: Vec<&str> = [1, 2, 5, 11, 12, 21, 22, 25, 111, 0]
            .iter()
            .map(|n| plural_category("ru", *n))
            .collect();
        assert_eq!(
            ru,
            [
                "one", "few", "many", "many", "many", "one", "few", "many", "many", "many"
            ]
        );
        assert_eq!(
            plural_category("ru-RU", 3),
            "few",
            "the region does not matter"
        );
        assert_eq!(plural_category("en", 1), "one");
        assert_eq!(plural_category("en", 0), "other");
        assert_eq!(plural_category("de", 2), "other");
        assert_eq!(plural_category("pl", 21), "many", "Polish: 21 is many");
        assert_eq!(plural_category("fr", 0), "one");
        assert_eq!(plural_category("ja", 1), "other");
    }

    #[test]
    fn placeholders_are_filled() {
        let name = "Nord";
        assert_eq!(fill("Theme: {name}", &[("name", &name)]), "Theme: Nord");
        assert_eq!(
            fill("{a} and {b}", &[("a", &1)]),
            "1 and {b}",
            "an unknown name stays"
        );
        assert_eq!(fill("no { end", &[("a", &1)]), "no { end");
        assert_eq!(placeholders("{n} of {total}"), ["n", "total"]);
    }

    #[test]
    fn a_file_with_texts_and_plurals() {
        let ru = russian(
            r#"{ "x.text": "Привет, {who}",
                 "x.tabs": { "one": "{n} вкладка", "few": "{n} вкладки", "many": "{n} вкладок" } }"#,
        );
        assert_eq!(ru.language, "ru");
        assert_eq!(ru.name, "Русский");
        assert_eq!(ru.format("x.text", &[("who", &"мир")]), "Привет, мир");
        assert_eq!(ru.plural("x.tabs", 1, &[]), "1 вкладка");
        assert_eq!(ru.plural("x.tabs", 3, &[]), "3 вкладки");
        assert_eq!(ru.plural("x.tabs", 5, &[]), "5 вкладок");
        // No key anywhere: the key itself, so a missing text is easy to see.
        assert_eq!(ru.format("x.nothing", &[]), "x.nothing");
    }

    #[test]
    fn a_missing_plural_form_uses_other() {
        let ru = russian(r#"{ "x.files": { "one": "{n} файл", "other": "{n} файлов" } }"#);
        assert_eq!(ru.plural("x.files", 3, &[]), "3 файлов");
    }

    #[test]
    fn bad_files() {
        let bad = |json: &str| Strings::parse_table(json).unwrap_err();
        assert!(bad("{").contains("EOF"), "bad JSON");
        assert!(bad(r#"{ "strings": {} }"#).contains("language"));
        assert!(bad(r#"{ "language": "ru", "strings": [] }"#).contains("strings"));
        assert!(bad(r#"{ "language": "ru", "strings": { "a": 1 } }"#).contains("strings.a"));
        assert!(
            bad(r#"{ "language": "ru", "strings": { "a": { "lots": "x" } } }"#)
                .contains("strings.a.lots")
        );
        // The name is optional; a BOM is fine.
        let s = Strings::parse_table("\u{feff}{ \"language\": \"de\" }").unwrap();
        assert_eq!(s.name, "de");
    }

    #[test]
    fn warnings_for_keys_that_english_does_not_have() {
        let (_, warnings) =
            Strings::parse(r#"{ "language": "ru", "strings": { "no.such.key": "x" } }"#).unwrap();
        assert_eq!(warnings, ["unknown key `no.such.key`"]);
    }

    #[test]
    fn the_english_file_is_good() {
        let (english, warnings) = Strings::parse(ENGLISH_JSON).unwrap();
        assert_eq!(english.language, "en");
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn the_choice_of_the_config() {
        assert_eq!(Choice::of(None, Some("ru-RU")), Choice::English);
        assert_eq!(Choice::of(Some("en"), None), Choice::English);
        assert_eq!(Choice::of(Some("ru"), None), Choice::Code("ru"));
        assert_eq!(
            Choice::of(Some("System"), Some("ru-RU")),
            Choice::System(Some("ru-RU"))
        );
        assert_eq!(
            Choice::of(Some("my/ru.json"), None),
            Choice::File(Path::new("my/ru.json"))
        );
        assert_eq!(
            Choice::of(Some("de.JSON"), None),
            Choice::File(Path::new("de.JSON"))
        );
    }

    #[test]
    fn finding_the_file_of_a_language() {
        let dir = std::env::temp_dir().join(format!("fterm-l10n-{}", std::process::id()));
        let (first, second) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(
            second.join("RU.json"),
            r#"{ "language": "ru", "name": "Русский" }"#,
        )
        .unwrap();
        let dirs = vec![first.clone(), second.clone()];
        let name = |choice: Choice| load(&choice, &dirs).map(|(s, _)| s.name);
        assert_eq!(name(Choice::Code("ru")).unwrap(), "Русский", "any case");
        assert_eq!(
            name(Choice::System(Some("ru-RU"))).unwrap(),
            "Русский",
            "ru-RU falls back to ru"
        );
        assert_eq!(name(Choice::English).unwrap(), "English");
        assert_eq!(
            name(Choice::System(Some("de-DE"))).unwrap(),
            "English",
            "no file for the system language"
        );
        assert_eq!(name(Choice::System(None)).unwrap(), "English");
        assert!(name(Choice::Code("de")).unwrap_err().contains("de.json"));
        let file = second.join("RU.json");
        assert_eq!(name(Choice::File(&file)).unwrap(), "Русский");
        assert!(name(Choice::File(&first.join("x.json"))).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The keys of `tr!` (false) and `trn!` (true) in the source of all crates (not this file).
    fn keys_in_the_code() -> Vec<(String, bool, String)> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|x| x == "rs") {
                    out.push(path);
                }
            }
        }
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        for krate in std::fs::read_dir(&crates).unwrap().flatten() {
            walk(&krate.path().join("src"), &mut files);
        }
        let mut keys = Vec::new();
        for file in files {
            if file.ends_with("l10n.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            for (mac, plural) in [("tr!(", false), ("trn!(", true)] {
                let mut rest = text.as_str();
                while let Some(at) = rest.find(mac) {
                    let before = rest[..at].chars().last();
                    rest = &rest[at + mac.len()..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let call = rest.trim_start();
                    let Some(call) = call.strip_prefix('"') else {
                        continue;
                    };
                    let Some(end) = call.find('"') else { continue };
                    keys.push((call[..end].to_owned(), plural, file.display().to_string()));
                }
            }
        }
        keys
    }

    #[test]
    fn every_key_in_the_code_is_in_english_and_every_english_key_is_used() {
        let english: HashMap<String, bool> = english_keys().into_iter().collect();
        let used = keys_in_the_code();
        for (key, plural, file) in &used {
            match english.get(key) {
                None => panic!("`{key}` ({file}) is not in assets/l10n/en.json"),
                Some(p) if p != plural => panic!(
                    "`{key}` ({file}): use {} for it",
                    if *p { "trn!" } else { "tr!" }
                ),
                Some(_) => {}
            }
        }
        // `action.*` are made from the action names (a test in keys.rs checks them).
        for key in english.keys().filter(|k| !k.starts_with("action.")) {
            assert!(
                used.iter().any(|(k, _, _)| k == key),
                "`{key}` in assets/l10n/en.json is not used"
            );
        }
    }

    #[test]
    fn the_macros_use_the_current_language() {
        // The current language is English in tests: a missing key comes back as the key.
        assert_eq!(crate::tr!("test.none"), "test.none");
        let n = 3;
        assert_eq!(crate::trn!("test.none", n), "test.none");
    }
}
