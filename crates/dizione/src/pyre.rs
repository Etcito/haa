//! Python `re` semantics over `fancy-regex`, so `normalize.rs` can stay line by line next to dizione's `normalize.py`.
//!
//! The JS port (`tiny-voca/js/dizione/pyre.js`) solves the same problem with its own engine. Here a Python pattern
//! is translated once into fancy-regex syntax with Python's meaning of the classes (`\w` = letters, numbers and `_`;
//! `\d` = decimal digits; `\s` = Python's white space; `\b` between those), `$` matching before a final newline,
//! `(?P=name)`, a literal `{` that opens no repeat, and the class characters Rust reads as set operators escaped.
//! Positions are byte offsets into the searched `&str`; the callers convert where Python counts characters.
//! Conditional groups `(?(name)...)` are not translated (fancy-regex reads them differently): the one pattern that
//! used one is written without it.

use std::sync::{LazyLock, OnceLock};

use fancy_regex::{Regex, RegexBuilder};

const WORD: &str = r"\p{L}\p{N}_";
const SPACE: &str = r"\t-\r\x1c-\x20\x85\xa0\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}";

/// A compiled Python pattern: searched as is, anchored for `match_`, anchored at both ends for `fullmatch`.
pub struct Re {
    source: String,
    search: Regex,
    anchored: OnceLock<Regex>,
    full: OnceLock<Regex>,
    names: Vec<Option<String>>,
}

/// One match: byte spans of the groups in `string`.
#[derive(Clone, Debug)]
pub struct M<'t> {
    pub string: &'t str,
    spans: Vec<Option<(usize, usize)>>,
    names: &'t [Option<String>],
}

impl<'t> M<'t> {
    pub fn start(&self) -> usize {
        self.spans[0].unwrap().0
    }
    pub fn end(&self) -> usize {
        self.spans[0].unwrap().1
    }
    /// The whole match.
    pub fn all(&self) -> &'t str {
        let (a, b) = self.spans[0].unwrap();
        &self.string[a..b]
    }
    /// Group `i`, `None` when it did not take part.
    pub fn g(&self, i: usize) -> Option<&'t str> {
        self.spans.get(i).copied().flatten().map(|(a, b)| &self.string[a..b])
    }
    /// Group `i`, empty when it did not take part.
    pub fn gs(&self, i: usize) -> &'t str {
        self.g(i).unwrap_or("")
    }
    fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.as_deref() == Some(name))
    }
    /// Named group, `None` when it did not take part or the pattern has no such group (`groupdict().get`).
    pub fn n(&self, name: &str) -> Option<&'t str> {
        self.g(self.index(name)?)
    }
    pub fn start_g(&self, i: usize) -> Option<usize> {
        self.spans.get(i).copied().flatten().map(|s| s.0)
    }
    pub fn end_g(&self, i: usize) -> Option<usize> {
        self.spans.get(i).copied().flatten().map(|s| s.1)
    }
    pub fn start_n(&self, name: &str) -> Option<usize> {
        self.start_g(self.index(name)?)
    }
    pub fn groups(&self) -> usize {
        self.spans.len()
    }
}

fn build(source: &str) -> Regex {
    RegexBuilder::new(source)
        .backtrack_limit(50_000_000)
        .build()
        .unwrap_or_else(|e| panic!("pattern {source:?}: {e}"))
}

/// Characters that are syntax in a Rust class and must be escaped there to stay literal.
fn class_meta(c: char) -> bool {
    matches!(c, '[' | ']' | '\\' | '^' | '&' | '~' | '-')
}

/// Characters that are syntax outside a class.
fn meta(c: char) -> bool {
    matches!(c, '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '#' | '&' | '-' | '~')
}

/// Python pattern -> fancy-regex pattern with the same meaning.
pub fn translate(p: &str, ignore_case: bool) -> String {
    let cs: Vec<char> = p.chars().collect();
    let mut out = String::with_capacity(p.len() * 2);
    if ignore_case {
        out.push_str("(?i)");
    }
    let boundary = format!(r"(?:(?<=[{WORD}])(?![{WORD}])|(?<![{WORD}])(?=[{WORD}]))");
    let not_boundary = format!(r"(?:(?<=[{WORD}])(?=[{WORD}])|(?<![{WORD}])(?![{WORD}]))");
    let mut i = 0;
    let mut in_class = false;
    let mut class_first = false;
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' {
            let n = cs[i + 1];
            i += 2;
            match n {
                'w' => out.push_str(&if in_class { WORD.to_string() } else { format!("[{WORD}]") }),
                'W' if !in_class => out.push_str(&format!("[^{WORD}]")),
                'd' => out.push_str(r"\p{Nd}"),
                'D' if !in_class => out.push_str(r"\P{Nd}"),
                's' => out.push_str(&if in_class { SPACE.to_string() } else { format!("[{SPACE}]") }),
                'S' if !in_class => out.push_str(&format!("[^{SPACE}]")),
                'b' if in_class => out.push_str(r"\x08"),
                'b' => out.push_str(&boundary),
                'B' if !in_class => out.push_str(&not_boundary),
                'Z' => out.push_str(r"\z"),
                'A' => out.push_str(r"\A"),
                'n' | 't' | 'r' | 'f' | 'v' => {
                    out.push('\\');
                    out.push(n);
                }
                'x' => {
                    let hex: String = cs[i..i + 2].iter().collect();
                    i += 2;
                    out.push_str(&format!(r"\x{{{hex}}}"));
                }
                'u' | 'U' => {
                    let len = if n == 'u' { 4 } else { 8 };
                    let hex: String = cs[i..i + len].iter().collect();
                    i += len;
                    out.push_str(&format!(r"\x{{{hex}}}"));
                }
                '1'..='9' if !in_class => {
                    out.push('\\');
                    out.push(n);
                }
                c if c.is_ascii_alphanumeric() => panic!("escape \\{c} not translated in {p:?}"),
                c => {
                    let escape = if in_class { class_meta(c) } else { meta(c) };
                    if escape {
                        out.push('\\');
                    }
                    out.push(c);
                }
            }
            class_first = false;
            continue;
        }
        if in_class {
            if c == ']' && !class_first {
                in_class = false;
                out.push(']');
            } else if c == '^' && class_first && cs[i - 1] == '[' {
                out.push('^');
                i += 1;
                continue;
            } else if c == ']' || matches!(c, '[' | '&' | '~') {
                out.push('\\');
                out.push(c);
            } else {
                out.push(c);
            }
            class_first = false;
            i += 1;
            continue;
        }
        match c {
            '[' => {
                in_class = true;
                class_first = true;
                out.push('[');
            }
            '$' => out.push_str(r"(?=\n?\z)"),
            '(' if cs[i..].starts_with(&['(', '?', 'P', '=']) => {
                let close = cs[i..].iter().position(|&x| x == ')').unwrap() + i;
                let name: String = cs[i + 4..close].iter().collect();
                out.push_str(&format!(r"\k<{name}>"));
                i = close + 1;
                continue;
            }
            '(' if cs[i..].starts_with(&['(', '?', '(']) => panic!("conditional group not translated in {p:?}"),
            '{' => {
                let rest: String = cs[i..].iter().take(16).collect();
                static QUANT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^\{[0-9]+(,[0-9]*)?\}").unwrap());
                if let Some(m) = QUANT.find(&rest) {
                    out.push_str(m.as_str());
                    i += m.as_str().chars().count();
                    continue;
                }
                out.push_str(r"\{");
            }
            '}' => out.push_str(r"\}"),
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

impl Re {
    pub fn new(pattern: &str) -> Re {
        Re::with(pattern, false)
    }
    pub fn new_i(pattern: &str) -> Re {
        Re::with(pattern, true)
    }
    pub fn with(pattern: &str, ignore_case: bool) -> Re {
        let source = translate(pattern, ignore_case);
        let search = build(&source);
        let names = search.capture_names().map(|n| n.map(str::to_string)).collect();
        Re { source, search, anchored: OnceLock::new(), full: OnceLock::new(), names }
    }

    fn wrap<'t>(&'t self, text: &'t str, caps: fancy_regex::Captures<'t, str>) -> M<'t> {
        let spans = (0..caps.len()).map(|i| caps.get(i).map(|m| (m.start(), m.end()))).collect();
        M { string: text, spans, names: &self.names }
    }

    fn run<'t>(&'t self, re: &'t Regex, text: &'t str, pos: usize) -> Option<M<'t>> {
        match re.captures_from_pos(text, pos) {
            Ok(Some(caps)) => Some(self.wrap(text, caps)),
            Ok(None) => None,
            Err(e) => panic!("pattern {:?} on {text:?}: {e}", self.source),
        }
    }

    /// `pattern.search(text, pos)`.
    pub fn search_from<'t>(&'t self, text: &'t str, pos: usize) -> Option<M<'t>> {
        self.run(&self.search, text, pos)
    }
    pub fn search<'t>(&'t self, text: &'t str) -> Option<M<'t>> {
        self.search_from(text, 0)
    }
    pub fn is_match(&self, text: &str) -> bool {
        self.search(text).is_some()
    }
    /// `pattern.match(text)`: anchored at the start.
    pub fn match_<'t>(&'t self, text: &'t str) -> Option<M<'t>> {
        let re = self.anchored.get_or_init(|| build(&format!(r"\A(?:{})", self.source)));
        self.run(re, text, 0)
    }
    /// `pattern.fullmatch(text)`.
    pub fn fullmatch<'t>(&'t self, text: &'t str) -> Option<M<'t>> {
        let re = self.full.get_or_init(|| build(&format!(r"\A(?:{})\z", self.source)));
        self.run(re, text, 0)
    }

    /// `pattern.finditer(text)`, with Python's rule for empty matches.
    pub fn finditer<'t>(&'t self, text: &'t str) -> Vec<M<'t>> {
        let mut out = Vec::new();
        let mut pos = 0;
        let mut must_advance = false;
        while pos <= text.len() {
            let Some(mut m) = self.search_from(text, pos) else { break };
            if must_advance && m.start() == pos && m.end() == pos {
                let Some(next) = next_char(text, pos) else { break };
                match self.search_from(text, next) {
                    Some(x) => m = x,
                    None => break,
                }
            }
            must_advance = m.end() == m.start();
            pos = m.end();
            out.push(m);
        }
        out
    }

    /// `pattern.sub(f, text)`.
    pub fn sub(&self, text: &str, mut f: impl FnMut(&M) -> String) -> String {
        let matches = self.finditer(text);
        if matches.is_empty() {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len() + 16);
        let mut last = 0;
        for m in &matches {
            out.push_str(&text[last..m.start()]);
            out.push_str(&f(m));
            last = m.end();
        }
        out.push_str(&text[last..]);
        out
    }

    /// `pattern.sub(template, text)` with `\1`-style references.
    pub fn sub_t(&self, text: &str, template: &str) -> String {
        self.sub(text, |m| expand(template, m))
    }

    /// `pattern.split(text)` for a pattern without groups.
    pub fn split<'t>(&'t self, text: &'t str) -> Vec<&'t str> {
        let mut out = Vec::new();
        let mut last = 0;
        for m in self.finditer(text) {
            out.push(&text[last..m.start()]);
            last = m.end();
        }
        out.push(&text[last..]);
        out
    }

    /// `pattern.findall(text)` for a pattern without groups.
    pub fn findall<'t>(&'t self, text: &'t str) -> Vec<&'t str> {
        self.finditer(text).iter().map(|m| m.all()).collect()
    }
}

fn next_char(text: &str, pos: usize) -> Option<usize> {
    text[pos..].chars().next().map(|c| pos + c.len_utf8())
}

/// A replacement template: `\N` is group N (one or two digits), everything else is literal.
pub fn expand(template: &str, m: &M) -> String {
    let cs: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '\\' && i + 1 < cs.len() && cs[i + 1].is_ascii_digit() {
            let mut n = cs[i + 1].to_digit(10).unwrap() as usize;
            i += 2;
            if i < cs.len() && cs[i].is_ascii_digit() {
                let two = n * 10 + cs[i].to_digit(10).unwrap() as usize;
                if two < m.groups() {
                    n = two;
                    i += 1;
                }
            }
            out.push_str(m.gs(n));
            continue;
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// A static compiled pattern: `rx!("...")` or `rx!(format!(...))`; `rx!(i, ...)` ignores case.
#[macro_export]
macro_rules! rx {
    (i, $p:expr) => {{
        static R: std::sync::LazyLock<$crate::pyre::Re> = std::sync::LazyLock::new(|| $crate::pyre::Re::new_i(&$p));
        &*R
    }};
    ($p:expr) => {{
        static R: std::sync::LazyLock<$crate::pyre::Re> = std::sync::LazyLock::new(|| $crate::pyre::Re::new(&$p));
        &*R
    }};
}

// ---- Python string methods ----------------------------------------------------------------------------------------

macro_rules! class_test {
    ($name:ident, $pat:expr) => {
        pub fn $name(text: &str) -> bool {
            static R: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new($pat).unwrap());
            R.is_match(text)
        }
    };
}

class_test!(isalpha, r"^\p{L}+$");
class_test!(isdecimal, r"^\p{Nd}+$");
class_test!(isalnum, r"^[\p{L}\p{N}]+$");
class_test!(has_upper, r"\p{Uppercase}");
class_test!(has_lower, r"\p{Lowercase}");
class_test!(has_not_upper, r"[\p{Lowercase}\p{Lt}]");
class_test!(has_not_lower, r"[\p{Uppercase}\p{Lt}]");

/// `str.isdigit()`: decimal digits and the other digits Python counts (superscripts, circled digits...).
pub fn isdigit(text: &str) -> bool {
    static R: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(concat!(
            r"^[\p{Nd}\xb2\xb3\xb9\x{1369}-\x{1371}\x{19da}\x{2070}\x{2074}-\x{2079}\x{2080}-\x{2089}\x{2460}-\x{2468}",
            r"\x{2474}-\x{247c}\x{2488}-\x{2490}\x{24ea}\x{24f5}-\x{24fd}\x{24ff}\x{2776}-\x{277e}\x{2780}-\x{2788}",
            r"\x{278a}-\x{2792}\x{10a40}-\x{10a43}\x{10e60}-\x{10e68}\x{11052}-\x{1105a}\x{1f100}-\x{1f10a}]+$"
        ))
        .unwrap()
    });
    R.is_match(text)
}

pub fn isupper(text: &str) -> bool {
    has_upper(text) && !has_not_upper(text)
}
pub fn islower(text: &str) -> bool {
    has_lower(text) && !has_not_lower(text)
}
pub fn isspace_char(c: char) -> bool {
    matches!(c, '\t'..='\r' | '\x1c'..='\x20' | '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
        | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}
pub fn isspace(text: &str) -> bool {
    !text.is_empty() && text.chars().all(isspace_char)
}
pub fn c_isalpha(c: char) -> bool {
    let mut b = [0u8; 4];
    isalpha(c.encode_utf8(&mut b))
}
pub fn c_isalnum(c: char) -> bool {
    let mut b = [0u8; 4];
    isalnum(c.encode_utf8(&mut b))
}
pub fn c_isupper(c: char) -> bool {
    let mut b = [0u8; 4];
    isupper(c.encode_utf8(&mut b))
}
pub fn c_islower(c: char) -> bool {
    let mut b = [0u8; 4];
    islower(c.encode_utf8(&mut b))
}
pub fn c_isdigit(c: char) -> bool {
    let mut b = [0u8; 4];
    isdigit(c.encode_utf8(&mut b))
}
pub fn strip(text: &str) -> &str {
    text.trim_matches(isspace_char)
}
pub fn lstrip(text: &str) -> &str {
    text.trim_start_matches(isspace_char)
}
pub fn rstrip(text: &str) -> &str {
    text.trim_end_matches(isspace_char)
}
/// `str.split()`.
pub fn words(text: &str) -> Vec<&str> {
    text.split(isspace_char).filter(|w| !w.is_empty()).collect()
}
pub fn lower(text: &str) -> String {
    text.to_lowercase()
}
pub fn upper(text: &str) -> String {
    text.to_uppercase()
}
/// Number of characters, as Python's `len`.
pub fn len(text: &str) -> usize {
    text.chars().count()
}
/// `int(text)` for decimal digits (any script); 0 when it is not a number.
pub fn int(text: &str) -> i64 {
    let mut n: i64 = 0;
    for c in text.chars() {
        let d = digit_value(c);
        n = n.saturating_mul(10).saturating_add(d as i64);
    }
    n
}
/// Value of a decimal digit of any script (the Nd blocks are runs of ten).
pub fn digit_value(c: char) -> u32 {
    if let Some(d) = c.to_digit(10) {
        return d;
    }
    let code = c as u32;
    // Every Unicode Nd run starts at a code point whose value is 0 and holds ten digits; the runs below are the
    // ones whose zero is not a multiple of 16 away from a block start, so search back for the zero.
    let mut zero = code;
    let mut b = [0u8; 4];
    while zero > 0 && code - zero < 9 {
        let prev = char::from_u32(zero - 1);
        match prev {
            Some(p) if isdecimal(p.encode_utf8(&mut b)) => zero -= 1,
            _ => break,
        }
    }
    (code - zero) % 10
}
/// The character before byte `pos`, if any.
pub fn char_before(text: &str, pos: usize) -> Option<char> {
    text[..pos].chars().next_back()
}
/// The character at byte `pos`, if any.
pub fn char_at(text: &str, pos: usize) -> Option<char> {
    text.get(pos..).and_then(|s| s.chars().next())
}
/// Byte offset `count` characters before `pos` (0 if fewer).
pub fn back_chars(text: &str, pos: usize, count: usize) -> usize {
    let mut at = pos;
    for (n, (i, _)) in text[..pos].char_indices().rev().enumerate() {
        if n >= count {
            break;
        }
        at = i;
    }
    if pos == 0 { 0 } else { at }
}
