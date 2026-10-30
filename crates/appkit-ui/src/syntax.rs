//! Syntax highlighting for the text editor: a small tokenizer for keywords,
//! strings, numbers and comments, with presets for SQL and Rust. Give it to
//! [`TextEditor::syntax`](crate::TextEditor::syntax).
//!
//! It's a scanner, not a parser: enough to colour code as Xcode or a
//! database tool does, fast enough to re-run over a few hundred kilobytes on
//! every keystroke.

use std::collections::HashSet;
use std::ops::Range;

use appkit_host::text::{TextColor, TextStyle};

/// How to colour a language. Build one with [`Syntax::new`] and its
/// methods, or start from [`Syntax::sql`] or [`Syntax::rust`].
///
/// ```
/// use appkit_ui::*;
///
/// let ini = Syntax::new()
///     .keywords(&["true", "false"], TextStyle::new().color(TextColor::Purple))
///     .strings("\"", TextStyle::new().color(TextColor::Red))
///     .line_comment(";", TextStyle::new().color(TextColor::Gray))
///     .numbers(TextStyle::new().color(TextColor::Blue));
/// let runs = ini.highlight("debug = true ; for now");
/// assert_eq!(runs[0], (8..12, TextStyle::new().color(TextColor::Purple)));
/// ```
#[derive(Clone, Default)]
pub struct Syntax {
    keywords: Vec<(HashSet<String>, TextStyle)>,
    ignore_case: bool,
    strings: Vec<char>,
    string_style: TextStyle,
    escape: Option<char>,
    doubled_quotes: bool,
    identifiers: Vec<(char, char)>,
    line_comments: Vec<String>,
    block_comment: Option<(String, String)>,
    comment_style: TextStyle,
    numbers: Option<TextStyle>,
}

impl Syntax {
    /// A syntax that styles nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Words shown in `style`. Call again for other groups (types,
    /// functions) in other styles; a word's first group wins.
    pub fn keywords(mut self, words: &[&str], style: TextStyle) -> Self {
        let words = words.iter().map(|w| if self.ignore_case { w.to_lowercase() } else { w.to_string() }).collect();
        self.keywords.push((words, style));
        self
    }

    /// Keywords match in any case (SQL). Call before [`keywords`](Self::keywords).
    pub fn ignore_case(mut self) -> Self {
        self.ignore_case = true;
        self
    }

    /// String literals: each character of `quotes` opens one, closed by the
    /// same character.
    pub fn strings(mut self, quotes: &str, style: TextStyle) -> Self {
        self.strings = quotes.chars().collect();
        self.string_style = style;
        self
    }

    /// Inside strings, `escape` makes the next character literal (`\` in
    /// most languages).
    pub fn escape(mut self, escape: char) -> Self {
        self.escape = Some(escape);
        self
    }

    /// Inside strings, a doubled quote is a literal quote (`'it''s'` in SQL).
    pub fn doubled_quotes(mut self) -> Self {
        self.doubled_quotes = true;
        self
    }

    /// Quoted identifiers (`"name"`, `` `name` ``, `[name]` in SQL), left
    /// unstyled so a quoted keyword isn't coloured as one.
    pub fn quoted_identifiers(mut self, pairs: &[(char, char)]) -> Self {
        self.identifiers = pairs.to_vec();
        self
    }

    /// Comments from `prefix` to the end of the line. Call again for more
    /// prefixes; the style is the last given (shared with block comments).
    pub fn line_comment(mut self, prefix: &str, style: TextStyle) -> Self {
        self.line_comments.push(prefix.into());
        self.comment_style = style;
        self
    }

    /// Comments from `open` to `close` (to the end of the text if unclosed).
    pub fn block_comment(mut self, open: &str, close: &str, style: TextStyle) -> Self {
        self.block_comment = Some((open.into(), close.into()));
        self.comment_style = style;
        self
    }

    /// Numbers: a word starting with a digit (`42`, `3.14`, `0x1F`, `1e-9`).
    pub fn numbers(mut self, style: TextStyle) -> Self {
        self.numbers = Some(style);
        self
    }

    /// SQL (SQLite's dialect, which covers the common ones): keywords pink
    /// and bold, types purple, functions indigo, strings red, numbers blue,
    /// comments gray and italic.
    pub fn sql() -> Self {
        Syntax::new()
            .ignore_case()
            .keywords(SQL_KEYWORDS, TextStyle::new().color(TextColor::Pink).bold())
            .keywords(SQL_TYPES, TextStyle::new().color(TextColor::Purple))
            .keywords(SQL_FUNCTIONS, TextStyle::new().color(TextColor::Indigo))
            .strings("'", TextStyle::new().color(TextColor::Red))
            .doubled_quotes()
            .quoted_identifiers(&[('"', '"'), ('`', '`'), ('[', ']')])
            .line_comment("--", TextStyle::new().color(TextColor::Gray).italic())
            .block_comment("/*", "*/", TextStyle::new().color(TextColor::Gray).italic())
            .numbers(TextStyle::new().color(TextColor::Blue))
    }

    /// Rust: keywords pink and bold, common types purple, strings red,
    /// numbers blue, comments gray and italic.
    pub fn rust() -> Self {
        Syntax::new()
            .keywords(RUST_KEYWORDS, TextStyle::new().color(TextColor::Pink).bold())
            .keywords(RUST_TYPES, TextStyle::new().color(TextColor::Purple))
            .strings("\"", TextStyle::new().color(TextColor::Red))
            .escape('\\')
            .line_comment("//", TextStyle::new().color(TextColor::Gray).italic())
            .block_comment("/*", "*/", TextStyle::new().color(TextColor::Gray).italic())
            .numbers(TextStyle::new().color(TextColor::Blue))
    }

    /// The styled runs of `text`, in order (UTF-8 byte ranges).
    pub fn highlight(&self, text: &str) -> Vec<(Range<usize>, TextStyle)> {
        let mut runs = Vec::new();
        let mut i = 0;
        let word = |c: char| c.is_alphanumeric() || c == '_';
        while let Some(c) = text[i..].chars().next() {
            let rest = &text[i..];
            // Comments.
            if let Some((open, close)) = self.block_comment.as_ref().filter(|(open, _)| rest.starts_with(open.as_str())) {
                let end = rest[open.len()..].find(close.as_str()).map_or(text.len(), |e| i + open.len() + e + close.len());
                runs.push((i..end, self.comment_style));
                i = end;
                continue;
            }
            if self.line_comments.iter().any(|p| rest.starts_with(p.as_str())) {
                let end = rest.find('\n').map_or(text.len(), |e| i + e);
                runs.push((i..end, self.comment_style));
                i = end;
                continue;
            }
            // Strings and quoted identifiers.
            if self.strings.contains(&c) {
                let end = self.close(text, i, c, c);
                runs.push((i..end, self.string_style));
                i = end;
                continue;
            }
            if let Some(&(open, close)) = self.identifiers.iter().find(|(open, _)| *open == c) {
                i = self.close(text, i, open, close);
                continue;
            }
            // Words: numbers and keywords.
            if word(c) {
                let end = rest.find(|ch: char| !word(ch) && ch != '.').map_or(text.len(), |e| i + e);
                let mut end = end;
                if c.is_ascii_digit() {
                    // An exponent's sign belongs to the number (1e-9).
                    while end < text.len() && matches!(text.as_bytes()[end], b'+' | b'-') && matches!(text.as_bytes()[end - 1], b'e' | b'E') {
                        end = text[end + 1..].find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.').map_or(text.len(), |e| end + 1 + e);
                    }
                    if let Some(style) = self.numbers {
                        runs.push((i..end, style));
                    }
                } else {
                    // A word ends at a dot (table.column).
                    end = rest.find(|ch: char| !word(ch)).map_or(text.len(), |e| i + e);
                    let w = &text[i..end];
                    let lower;
                    let key = if self.ignore_case {
                        lower = w.to_lowercase();
                        lower.as_str()
                    } else {
                        w
                    };
                    // After a dot it's a member (orders.total), not a keyword.
                    let member = text[..i].ends_with('.');
                    if let Some((_, style)) = self.keywords.iter().find(|(set, _)| set.contains(key)).filter(|_| !member) {
                        runs.push((i..end, *style));
                    }
                }
                i = end;
                continue;
            }
            i += c.len_utf8();
        }
        runs
    }

    /// The end of a quoted run starting at `start` (after its closing
    /// quote, or the end of the text).
    fn close(&self, text: &str, start: usize, open: char, close: char) -> usize {
        let mut chars = text[start + open.len_utf8()..].char_indices().peekable();
        let base = start + open.len_utf8();
        while let Some((at, ch)) = chars.next() {
            if open == close && self.escape == Some(ch) && self.strings.contains(&open) {
                chars.next();
            } else if ch == close {
                if self.doubled_quotes && open == close && chars.peek().is_some_and(|(_, n)| *n == close) {
                    chars.next();
                    continue;
                }
                return base + at + ch.len_utf8();
            }
        }
        text.len()
    }
}

const SQL_KEYWORDS: &[&str] = &[
    "abort", "action", "add", "after", "all", "alter", "always", "analyze", "and", "as", "asc", "attach", "autoincrement", "before", "begin",
    "between", "by", "cascade", "case", "cast", "check", "collate", "column", "commit", "conflict", "constraint", "create", "cross",
    "current_date", "current_time", "current_timestamp", "database", "default", "deferred", "delete", "desc", "detach", "distinct", "do",
    "drop", "each", "else", "end", "escape", "except", "exclusive", "exists", "explain", "fail", "filter", "first", "following", "for",
    "foreign", "from", "full", "generated", "glob", "group", "having", "if", "ignore", "immediate", "in", "index", "indexed", "initially",
    "inner", "insert", "instead", "intersect", "into", "is", "isnull", "join", "key", "last", "left", "like", "limit", "match",
    "materialized", "natural", "no", "not", "nothing", "notnull", "null", "nulls", "of", "offset", "on", "or", "order", "outer", "over",
    "partition", "plan", "pragma", "preceding", "primary", "query", "raise", "range", "recursive", "references", "regexp", "reindex",
    "release", "rename", "replace", "restrict", "returning", "right", "rollback", "row", "rows", "savepoint", "select", "set", "strict",
    "table", "temp", "temporary", "then", "ties", "to", "transaction", "trigger", "unbounded", "union", "unique", "update", "using",
    "vacuum", "values", "view", "virtual", "when", "where", "window", "with", "without", "true", "false",
];

const SQL_TYPES: &[&str] = &["integer", "int", "real", "text", "blob", "numeric", "boolean", "date", "datetime", "varchar", "char", "float", "double", "decimal", "bigint"];

const SQL_FUNCTIONS: &[&str] = &[
    "abs", "avg", "char", "coalesce", "count", "date", "datetime", "glob", "group_concat", "hex", "ifnull", "iif", "instr", "json",
    "json_extract", "julianday", "length", "lower", "ltrim", "max", "min", "nullif", "printf", "quote", "random", "round", "rtrim",
    "sqlite_version", "strftime", "substr", "substring", "sum", "time", "total", "trim", "typeof", "unicode", "upper", "zeroblob",
];

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while",
];

const RUST_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "str", "String",
    "Vec", "Option", "Result", "Box", "Rc", "Arc", "Some", "None", "Ok", "Err",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(syntax: &Syntax, text: &str) -> Vec<(String, TextStyle)> {
        syntax.highlight(text).into_iter().map(|(r, s)| (text[r].to_string(), s)).collect()
    }

    #[test]
    fn sql_tokens() {
        let sql = Syntax::sql();
        let kw = TextStyle::new().color(TextColor::Pink).bold();
        let got = spans(&sql, "SELECT name, count(*) FROM \"select\" WHERE x = 'it''s' AND y > 1.5e-3 -- done\n/* a\nb */ limit 10");
        let texts: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(texts, ["SELECT", "count", "FROM", "WHERE", "'it''s'", "AND", "1.5e-3", "-- done", "/* a\nb */", "limit", "10"]);
        assert_eq!(got[0].1, kw);
        assert_eq!(got[1].1, TextStyle::new().color(TextColor::Indigo));
        assert_eq!(got[4].1, TextStyle::new().color(TextColor::Red));
        assert_eq!(got[6].1, TextStyle::new().color(TextColor::Blue));
    }

    #[test]
    fn unclosed_runs_reach_the_end() {
        let sql = Syntax::sql();
        assert_eq!(sql.highlight("SELECT 'abc").last().unwrap().0, 7..11);
        assert_eq!(sql.highlight("/* open").last().unwrap().0, 0..7);
    }

    #[test]
    fn dotted_names_and_unicode() {
        let sql = Syntax::sql();
        let got = spans(&sql, "select o.total, 'café' from orders o");
        let texts: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(texts, ["select", "'café'", "from"]);
    }

    #[test]
    fn rust_escapes() {
        let rust = Syntax::rust();
        let got = spans(&rust, r#"let s: String = "a \" b"; // c"#);
        let texts: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(texts, ["let", "String", r#""a \" b""#, "// c"]);
    }
}
