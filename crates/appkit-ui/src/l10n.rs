//! Localisation: `.strings` tables, the user's language, plurals and
//! numbers. See [`t`].

use std::cell::RefCell;
use std::collections::HashMap;

use objc2_foundation::{NSArray, NSBundle, NSLocale, NSNumber, NSNumberFormatter, NSNumberFormatterStyle, NSString};

type Table = HashMap<String, String>;

/// The framework's own strings, by language.
const BUILT_IN: [(&str, &str); 3] = [("de", include_str!("../locales/de.strings")), ("fr", include_str!("../locales/fr.strings")), ("ja", include_str!("../locales/ja.strings"))];

#[derive(Default)]
struct State {
    /// The app's tables, by language.
    app: HashMap<String, Table>,
    /// The framework's, parsed when first needed.
    built_in: Option<HashMap<String, Table>>,
    /// The language chosen (`None`: not yet, or "en").
    chosen: Option<Option<String>>,
    /// Set by [`set_language`].
    forced: Option<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Parses a `.strings` table: `"key" = "value";` lines, with `/* */` and
/// `//` comments and the escapes `\"`, `\\`, `\n`, `\t` and `\Uxxxx`.
/// Malformed entries are skipped.
pub fn parse_strings(text: &str) -> HashMap<String, String> {
    let mut table = HashMap::new();
    let mut chars = text.chars().peekable();
    let mut strings: Vec<String> = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('U' | 'u') => {
                                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                                s.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                            }
                            Some(other) => s.push(other),
                            None => {}
                        },
                        c => s.push(c),
                    }
                }
                strings.push(s);
            }
            ';' => {
                if let [key, value] = strings.as_slice() {
                    table.insert(key.clone(), value.clone());
                }
                strings.clear();
            }
            _ => {}
        }
    }
    table
}

/// Adds the app's tables (see [`App::translations`](crate::App::translations)).
pub(crate) fn add_translations(tables: &[(&str, &str)]) {
    STATE.with_borrow_mut(|s| {
        for (lang, text) in tables {
            s.app.entry(lang.to_string()).or_default().extend(parse_strings(text));
        }
        s.chosen = None;
    });
}

/// The user's preferred languages that `available` has, best first (what
/// AppKit uses: it honours the app's own language setting and
/// `-AppleLanguages`).
fn preferred(available: &[String]) -> Option<String> {
    let mut candidates: Vec<&str> = available.iter().map(String::as_str).collect();
    candidates.push("en");
    let array = NSArray::from_retained_slice(&candidates.iter().map(|l| NSString::from_str(l)).collect::<Vec<_>>());
    // The user's languages given explicitly: without them an app that isn't
    // bundled (no localizations of its own) always gets English.
    let chosen = NSBundle::preferredLocalizationsFromArray_forPreferences(&array, Some(&NSLocale::preferredLanguages()));
    chosen.firstObject().map(|l| l.to_string()).filter(|l| l != "en")
}

/// The language the app's strings are in: a code like `"de"` or `"pt-BR"`,
/// or `"en"` (the keys' own language).
pub fn language() -> String {
    STATE.with_borrow_mut(|s| {
        if let Some(f) = &s.forced {
            return f.clone();
        }
        if s.chosen.is_none() {
            // The app's languages when it has any (so the framework's
            // strings never switch language without the app's), else the
            // framework's.
            let available: Vec<String> = if s.app.is_empty() { BUILT_IN.iter().map(|(l, _)| l.to_string()).collect() } else { s.app.keys().cloned().collect() };
            s.chosen = Some(preferred(&available));
        }
        s.chosen.clone().flatten().unwrap_or_else(|| "en".into())
    })
}

/// Uses `language` (`"de"`, `"fr"`…) instead of the user's choice, or the
/// user's again with `None`: for tests and in-app language pickers. Strings
/// already shown (menus, labels that don't read a signal) stay as they are.
pub fn set_language(language: Option<&str>) {
    STATE.with_borrow_mut(|s| {
        s.forced = language.map(String::from);
        s.chosen = None;
    });
}

/// The translation of `key`, or `key` itself.
fn lookup(key: &str) -> Option<String> {
    let lang = language();
    if lang == "en" {
        return None;
    }
    // "pt-BR", then "pt".
    let langs = [lang.clone(), lang.split(['-', '_']).next().unwrap_or_default().to_string()];
    STATE.with_borrow_mut(|s| {
        if s.built_in.is_none() {
            s.built_in = Some(BUILT_IN.iter().map(|(l, text)| (l.to_string(), parse_strings(text))).collect());
        }
        let built_in = s.built_in.as_ref().expect("parsed above");
        langs.iter().find_map(|l| s.app.get(l).and_then(|t| t.get(key)).or_else(|| built_in.get(l).and_then(|t| t.get(key))).cloned())
    })
}

/// `key` translated into the app's language, or `key` itself if there's no
/// translation.
///
/// Keys are the English text (`t("Save As…")`), so an untranslated string
/// shows in English. The app's tables ([`App::translations`](crate::App::translations))
/// name the languages it speaks; the language used is the first of the
/// user's preferred languages (System Settings ▸ General ▸ Language &
/// Region, or the app's own choice there) that it has. The framework's own
/// strings (the menus it makes, document alerts, "Untitled") come
/// translated into German, French and Japanese.
///
/// ```no_run
/// use appkit_ui::*;
///
/// App::new("Notes")
///     .translations(&[("de", "\"Notes\" = \"Notizen\";\n\"{n} note\" = \"{n} Notiz\";\n\"{n} notes\" = \"{n} Notizen\";")])
///     .window(Window::new(&t("Notes")).view(label(|| t_plural(3, "{n} note", "{n} notes"))))
///     .run();
/// ```
pub fn t(key: &str) -> String {
    lookup(key).unwrap_or_else(|| key.to_string())
}

/// [`t`] with `{name}` placeholders filled in: `t_with("Open “{file}”",
/// &[("file", &name)])`. Translations can put them in any order.
pub fn t_with(key: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = t(key);
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

/// The plural category of `n` in `language` (CLDR's rules for integers):
/// `"one"`, `"few"`, `"many"` or `"other"`.
pub fn plural_category(language: &str, n: i64) -> &'static str {
    let n = n.unsigned_abs();
    let (m10, m100) = (n % 10, n % 100);
    let slavic = |one_ok: bool| {
        if one_ok && m10 == 1 && m100 != 11 {
            "one"
        } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
            "few"
        } else {
            "many"
        }
    };
    match language.split(['-', '_']).next().unwrap_or_default() {
        "ja" | "zh" | "ko" | "th" | "vi" | "id" | "ms" => "other",
        "fr" if n <= 1 => "one",
        "fr" => "other",
        "pt" if language.contains("PT") => if n == 1 { "one" } else { "other" },
        "pt" if n <= 1 => "one",
        "pt" => "other",
        "ru" | "uk" | "be" => slavic(true),
        "pl" if n == 1 => "one",
        "pl" => slavic(false),
        "cs" | "sk" if n == 1 => "one",
        "cs" | "sk" if (2..=4).contains(&n) => "few",
        "cs" | "sk" => "other",
        _ if n == 1 => "one",
        _ => "other",
    }
}

/// A count in the app's language: `t_plural(n, "{n} row", "{n} rows")`.
/// The English forms are the keys. A table gives each form as
/// `"{n} rows#one"`, `"{n} rows#few"`, `"{n} rows#many"`,
/// `"{n} rows#other"` (or just `"{n} rows"` for the other form). `{n}` is
/// the number formatted for the user's region (1,000 or 1.000).
pub fn t_plural(n: i64, one: &str, other: &str) -> String {
    let lang = language();
    let category = plural_category(&lang, n);
    let text = if lang == "en" {
        if category == "one" { one.to_string() } else { other.to_string() }
    } else {
        lookup(&format!("{other}#{category}"))
            .or_else(|| (category == "one").then(|| lookup(one)).flatten())
            .or_else(|| lookup(&format!("{other}#other")))
            .unwrap_or_else(|| t(if category == "one" { one } else { other }))
    };
    text.replace("{n}", &format_number(n as f64, 0))
}

/// `value` with `decimals` decimals, grouped and pointed as the user's
/// region writes numbers (1,234.5 in the US, 1.234,5 in Germany).
pub fn format_number(value: f64, decimals: usize) -> String {
    format_number_in(value, decimals, &NSLocale::currentLocale())
}

/// [`format_number`] for the locale named `identifier` (`"de_DE"`).
pub fn format_number_for(value: f64, decimals: usize, identifier: &str) -> String {
    format_number_in(value, decimals, &NSLocale::localeWithLocaleIdentifier(&NSString::from_str(identifier)))
}

fn format_number_in(value: f64, decimals: usize, locale: &NSLocale) -> String {
    let f = NSNumberFormatter::new();
    f.setNumberStyle(NSNumberFormatterStyle::DecimalStyle);
    f.setLocale(Some(locale));
    f.setMinimumFractionDigits(decimals);
    f.setMaximumFractionDigits(decimals);
    f.stringFromNumber(&NSNumber::new_f64(value)).map(|s| s.to_string()).unwrap_or_else(|| format!("{value:.decimals$}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_format() {
        let t = parse_strings(
            "/* A comment */\n\"Save\" = \"Sichern\";\n// another\n\"Say \\\"hi\\\"\" = \"Sag \\\"hallo\\\"\";\n\"Two\\nlines\" = \"Zwei\\nZeilen\";\n\"\\U00e9t\\U00e9\" = \"Sommer\";\nbroken = \"no\";\n\"Lone\";\n",
        );
        assert_eq!(t.get("Save").map(String::as_str), Some("Sichern"));
        assert_eq!(t.get("Say \"hi\"").map(String::as_str), Some("Sag \"hallo\""));
        assert_eq!(t.get("Two\nlines").map(String::as_str), Some("Zwei\nZeilen"));
        assert_eq!(t.get("été").map(String::as_str), Some("Sommer"));
        assert_eq!(t.len(), 4);
    }

    #[test]
    fn plurals() {
        // (112 ends in 12: "many" in Russian and Polish, like 12.)
        let cats = |lang: &str| [0, 1, 2, 5, 11, 21, 22, 25, 112].map(|n| plural_category(lang, n));
        assert_eq!(cats("en"), ["other", "one", "other", "other", "other", "other", "other", "other", "other"]);
        assert_eq!(cats("fr"), ["one", "one", "other", "other", "other", "other", "other", "other", "other"]);
        assert_eq!(cats("ru"), ["many", "one", "few", "many", "many", "one", "few", "many", "many"]);
        assert_eq!(cats("pl"), ["many", "one", "few", "many", "many", "many", "few", "many", "many"]);
        assert_eq!(cats("ja"), ["other"; 9]);
        assert_eq!(plural_category("pt-PT", 0), "other");
        assert_eq!(plural_category("pt-BR", 0), "one");
    }

    #[test]
    fn built_in_tables_cover_the_same_keys() {
        let tables: Vec<(&str, Table)> = BUILT_IN.iter().map(|(l, text)| (*l, parse_strings(text))).collect();
        let (first, keys) = (&tables[0].0, &tables[0].1);
        assert!(keys.len() >= 40, "{} keys", keys.len());
        for (lang, table) in &tables[1..] {
            let missing: Vec<_> = keys.keys().filter(|k| !table.contains_key(*k)).collect();
            let extra: Vec<_> = table.keys().filter(|k| !keys.contains_key(*k)).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{lang} vs {first}: missing {missing:?}, extra {extra:?}");
            // Placeholders survive translation.
            for (k, v) in table {
                for p in ["{name}", "{n}"] {
                    assert_eq!(k.contains(p), v.contains(p), "{lang}: {k:?} → {v:?}");
                }
            }
        }
    }
}
