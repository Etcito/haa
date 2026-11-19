//! Text -> the words to say and the punctuation marks between them (dizione `normalize.py`, branch
//! `fix/dotted-acronyms` 0d39f7c, plus the page fix of tiny-voca's JS port: "Chapter 3, p. 45").
//!
//! Numbers, ordinals, money, dates, times, telephone numbers, units, abbreviations, initialisms, symbols, chat
//! spellings, versions, codes, web addresses and file names become words, by rule. Emoticons and emoji are dropped.
//! A pronunciation can be given inline, `{HH AH0 L OW1}` or `{h ə l ˈoʊ}`. The functions follow the Python ones in
//! order and name; regular expressions keep Python's text (see `pyre.rs`). Positions are byte offsets; where Python
//! counts characters (`_before`, the 24 characters before a number) the code counts characters too.

use std::collections::{HashMap, HashSet};

use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;

use crate::convert::{convert, from_text, is_arpabet};
use crate::pyre::{self, char_at, char_before, Re, M};
use crate::rx;
use crate::symbols::{Ph, PUNCTUATION};
use crate::tables;

/// A word to say: letters and apostrophes in lower case.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Word {
    pub text: String,
    /// Continues the previous word (hyphenated compound, parts of one token).
    pub attach: bool,
    /// As written.
    pub raw: String,
    /// Read letter by letter.
    pub spell: bool,
    /// Pronunciation given inline by the user.
    pub phones: Option<Vec<Ph>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Word(Word),
    Punct(String),
}

impl Token {
    fn word(&self) -> Option<&Word> {
        match self {
            Token::Word(w) => Some(w),
            _ => None,
        }
    }
    fn mark(&self) -> Option<&str> {
        match self {
            Token::Punct(m) => Some(m),
            _ => None,
        }
    }
}

fn w(text: &str, raw: &str) -> Word {
    Word { text: text.into(), raw: raw.into(), ..Default::default() }
}

// Private-use characters that carry decisions through the rewriting of the text.
const SPELL: char = '\u{E000}';
const END: char = '\u{E001}';
const INLINE: char = '\u{E003}';
const INLINE_BASE: u32 = 0xE100;
const IS: char = '\u{E006}';
const FORMULA_SIGN: char = '\u{E004}';
/// dizione marks enclosed angle brackets with U+E005; a one-byte control character keeps byte offsets of the text and
/// of its marked copy equal. Nothing but inequality with '<' and '>' is ever asked of the mark.
const ANGLE_MARK: char = '\x05';

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen",
    "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 8] = ["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
const SCALES: [&str; 6] = ["thousand", "million", "billion", "trillion", "quadrillion", "quintillion"];
const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTH_PATTERN: &str =
    "January|February|March|April|May|June|July|August|September|October|November|December|Jan|Feb|Mar|Apr|Jun|Jul|Aug|Sept|Sep|Oct|Nov|Dec";
const DAY_PATTERN: &str = "Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday|Thurs|Thur|Tues|Mon|Tue|Wed|Thu|Fri|Sat|Sun";
const CURRENCY_CODES: &str = "USD|EUR|GBP|JPY|INR|CHF|CAD|AUD|CNY";
const ZONES: &str = "ET|CT|MT|PT|EST|EDT|CST|CDT|MST|MDT|PST|PDT|GMT|UTC|BST|CET|CEST|IST|AEST|AEDT|JST";
const SIZES: &str = "XXXL|XXL|XXS|XL|XS|S|M|L";
const LENGTHS: &str = "mm|cm|km|m|ft|in|inch|inches|feet|foot|metres?|meters?|yards?|yd|px|pixels?";
const VERSIONED: &str = concat!(
    "version|ver|release|firmware|patch|kernel|build|python|node|nodejs|ios|ipados|macos|watchos|tvos|android|",
    "chrome|firefox|safari|java|php|ruby|rust|swift|kotlin|golang|django|rails|react|angular|vue|docker|",
    "kubernetes|ubuntu|debian|fedora|windows|linux|postgres|postgresql|mysql|redis|numpy|pytorch|tensorflow|cuda|",
    "xcode|gcc|clang|unity|minecraft|typescript|bluetooth|usb|hdmi|opengl|vulkan|directx|",
    "upgrade[ds]?|update[ds]?|downgrade[ds]?"
);
const HALF_DAY: &str = r"(?:\s?([AaPp])\.[Mm]\.?|\s?([AaPp])[Mm]\b)";
const AMOUNT: &str = r"(?P<whole>\d{1,3}(?:,\d{3})+|\d+)(?:\.(?P<fraction>\d+))?";
const CLOCK: &str = r"\d{1,2}:[0-5]\d(?:\s?[AaPp]\.?[Mm]\.?)?|\b\d{1,2}\s?[AaPp]\.?[Mm]\.?";
const CORE: &str = r"[A-Z0-9]*\d[A-Z0-9]*(?: [A-Z0-9]*\d[A-Z0-9]*)+";
const SIGN: &str = r"[<>+*/^≤≥≠≈×÷±]";
const SPACED_SIGN: &str = r" [<>+*≤≥≠≈×÷±] ";
const NO_PHRASE: &str = r"(?![\w'’-]|\s+(?!(?:and|or|then|if|when|where|while|for|is|are|was|but|so|in|to)\b)[A-Za-z])";

fn sign_after() -> String {
    format!(r"[$£€¥₹](?![\w$£€¥₹])|(?:{CURRENCY_CODES})(?![A-Za-z])")
}

fn month_number(name: &str) -> Option<usize> {
    let key: String = name.to_lowercase().chars().take(3).collect();
    MONTHS.iter().position(|m| m.to_lowercase()[..3] == key).map(|i| i + 1)
}

fn day_name(key: &str) -> Option<&'static str> {
    Some(match key {
        "mon" => "Monday",
        "tue" => "Tuesday",
        "wed" => "Wednesday",
        "thu" => "Thursday",
        "fri" => "Friday",
        "sat" => "Saturday",
        "sun" => "Sunday",
        _ => return None,
    })
}

fn currency(symbol: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    match symbol {
        "$" | "USD" => ("dollar", "dollars", "cent", "cents"),
        "£" | "GBP" => ("pound", "pounds", "penny", "pence"),
        "€" | "EUR" => ("euro", "euros", "cent", "cents"),
        "¥" | "JPY" => ("yen", "yen", "", ""),
        "₹" | "INR" => ("rupee", "rupees", "paisa", "paise"),
        "CHF" => ("Swiss franc", "Swiss francs", "centime", "centimes"),
        "CAD" => ("Canadian dollar", "Canadian dollars", "cent", "cents"),
        "AUD" => ("Australian dollar", "Australian dollars", "cent", "cents"),
        _ => ("yuan", "yuan", "", ""),
    }
}

fn symbol_word(c: &str) -> &'static str {
    match c {
        "&" => "and",
        "@" => "at",
        "+" => "plus",
        "=" => "equals",
        "%" => "percent",
        "×" => "times",
        "÷" => "divided by",
        "±" => "plus or minus",
        "©" => "copyright",
        "§" => "section",
        "°" => "degrees",
        "≥" => "greater than or equal to",
        "≤" => "less than or equal to",
        "≠" => "not equal to",
        "≈" => "approximately",
        "∞" => "infinity",
        "√" => "square root of",
        _ => "pi",
    }
}

fn set(words: &str) -> HashSet<&str> {
    words.split_whitespace().collect()
}

macro_rules! words_set {
    ($name:ident, $text:expr) => {
        fn $name() -> &'static HashSet<&'static str> {
            static S: std::sync::LazyLock<HashSet<&'static str>> = std::sync::LazyLock::new(|| set($text));
            &S
        }
    };
}

words_set!(initialisms, "US IT AI ID UN EU UK LA AD BC AM PM OS PE PA MA LED SAT RIP ER");
words_set!(sounds, "sh shh ss zz hm hmm mm tsk psst pfft hmph psh pssh");
words_set!(
    clause_openers,
    "if elif while when whenever where whether that because since as and or but so then think thinks thought know knows say says said suppose assume means implies hence thus therefore although though unless until once else also clearly obviously indeed here now again first next finally similarly otherwise however moreover"
);
words_set!(
    sentence_starters,
    "the he she it i we they this that there but and in on at a an if when what why how who my our you then so now after before however"
);
words_set!(year_context, "in since by from until till before after during around circa of year early late mid summer winter spring fall autumn class");
words_set!(cardinal_after, "war chapter part book volume act scene section phase type class level stage title article appendix world");
words_set!(labelled, "plan vitamin grade type option class section part team group exhibit appendix point model series hepatitis level");
words_set!(phone_words, "call dial ring phone telephone tel mobile cell fax hotline text sms whatsapp");
words_set!(phone_links, "to on at or us me him her them back");
words_set!(pin_words, "code pin extension ext passcode password zip postcode cvv cvc otp");
words_set!(strong_words, "number id reference ref serial account tracking booking confirmation");
words_set!(
    weak_words,
    "order ticket invoice case claim policy receipt transaction parcel shipment container payment reservation membership customer batch lot item job issue bug request"
);
words_set!(link_words, "is was are were ends ending in with starts starting begins beginning");
words_set!(
    not_counted,
    "is was has does goes ships arrives departs leaves shows needs blocks remains includes contains says reads starts ends closes opens runs takes comes this his its us as yes plus minus less"
);
words_set!(
    numbered,
    "ref reference serial account order invoice tracking part model phone tel fax id room flight case ticket policy licence license booking customer vat tax reg registration"
);
words_set!(
    date_before,
    "on dated due until till by since from before after for shipped delivered arriving arrives arrive arrived expires expired ends ending starts starting begins beginning monday tuesday wednesday thursday friday saturday sunday mon tue tues wed thu thur thurs fri sat sun"
);
words_set!(ratio_words, "ratio ratios scale odds aspect mix split proportion");
words_set!(duration_words, "lasted took duration runtime recording video clip track lap pace length long");
words_set!(
    clock_words,
    "window slot open opens opening hours shift session served service available booked appointment delivery arrive arrives arriving"
);
words_set!(capital_words, "IN ON AT TO OF BY OR IS AS IF NO SO WE HE MY ME DO GO UP AN AM PM BE IT US THE AND FOR BUT NOT ALL");
words_set!(short_words, "my to in on of go up no do me we us it is at by be so or an as if he hi ok");
words_set!(leading_apostrophe, "'em 'tis 'twas 'til 'cause 'bout 'round 'n");

const CLASSIC_FRACTIONS: [i128; 11] = [2, 3, 4, 5, 6, 8, 10, 16, 32, 64, 100];

fn roman(text: &str) -> Option<i128> {
    Some(match text {
        "II" => 2,
        "III" => 3,
        "IV" => 4,
        "VI" => 6,
        "VII" => 7,
        "VIII" => 8,
        "IX" => 9,
        "XI" => 11,
        "XII" => 12,
        "XIII" => 13,
        "XIV" => 14,
        "XV" => 15,
        "XVI" => 16,
        "XVIII" => 18,
        "XX" => 20,
        _ => return None,
    })
}

fn accent_decides(word: &str) -> Option<&'static str> {
    Some(match word {
        "résumé" | "resumé" => "R EH1 Z AH0 M EY2",
        "exposé" => "EH2 K S P OW0 Z EY1",
        "rosé" => "R OW0 Z EY1",
        "pâté" => "P AA0 T EY1",
        "maté" => "M AA1 T EY2",
        "saké" => "S AA1 K EY2",
        "lamé" => "L AA0 M EY1",
        _ => return None,
    })
}

fn letter_form(c: char) -> Option<&'static str> {
    Some(match c {
        'ß' => "ss",
        'æ' => "ae",
        'Æ' => "Ae",
        'œ' => "oe",
        'Œ' => "Oe",
        'ø' => "o",
        'Ø' => "O",
        'đ' => "d",
        'ł' => "l",
        'Ł' => "L",
        _ => return None,
    })
}

/// An accented Latin letter without its accent; other characters are left alone.
fn plain_letter(c: char) -> String {
    if c.is_ascii() || !pyre::c_isalpha(c) {
        return c.to_string();
    }
    if let Some(form) = letter_form(c) {
        return form.into();
    }
    let base: String = c.to_string().nfkd().filter(|&x| canonical_combining_class(x) == 0).collect();
    if base.is_ascii() && !base.is_empty() && pyre::isalpha(&base) { base } else { c.to_string() }
}

// ---- numbers as words ----------------------------------------------------------------------------------------------

pub fn cardinal(number: i128) -> String {
    if number < 0 {
        return format!("minus {}", cardinal(-number));
    }
    if number < 20 {
        return ONES[number as usize].into();
    }
    if number < 100 {
        let (tens, ones) = (number / 10, number % 10);
        let mut s = TENS[tens as usize - 2].to_string();
        if ones != 0 {
            s += " ";
            s += ONES[ones as usize];
        }
        return s;
    }
    if number < 1000 {
        let (hundreds, rest) = (number / 100, number % 100);
        let mut s = format!("{} hundred", ONES[hundreds as usize]);
        if rest != 0 {
            s += " ";
            s += &cardinal(rest);
        }
        return s;
    }
    if number >= 1000i128.pow(SCALES.len() as u32 + 1) {
        return digits(&number.to_string(), "zero");
    }
    let mut parts = Vec::new();
    let mut n = number;
    for power in (1..=SCALES.len()).rev() {
        let unit = 1000i128.pow(power as u32);
        let count = n / unit;
        n %= unit;
        if count != 0 {
            parts.push(format!("{} {}", cardinal(count), SCALES[power - 1]));
        }
    }
    if n != 0 {
        parts.push(cardinal(n));
    }
    parts.join(" ")
}

pub fn ordinal(number: i128) -> String {
    let card = cardinal(number);
    let mut words: Vec<String> = card.split(' ').map(String::from).collect();
    let last = words.last().unwrap().clone();
    let ord = match last.as_str() {
        "one" => "first".to_string(),
        "two" => "second".into(),
        "three" => "third".into(),
        "five" => "fifth".into(),
        "eight" => "eighth".into(),
        "nine" => "ninth".into(),
        "twelve" => "twelfth".into(),
        l if l.ends_with('y') => format!("{}ieth", &l[..l.len() - 1]),
        l => format!("{l}th"),
    };
    *words.last_mut().unwrap() = ord;
    words.join(" ")
}

pub fn digits(text: &str, zero: &str) -> String {
    text.chars()
        .filter(|&c| pyre::c_isdigit(c))
        .map(|c| if c == '0' { zero.to_string() } else { ONES[pyre::digit_value(c) as usize % 10].to_string() })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn year(number: i128) -> String {
    if !(1000..=2999).contains(&number) {
        return cardinal(number);
    }
    let (high, low) = (number / 100, number % 100);
    if low == 0 {
        return if high % 10 == 0 { cardinal(number) } else { format!("{} hundred", cardinal(high)) };
    }
    if high == 20 && low < 10 {
        return format!("two thousand {}", ONES[low as usize]);
    }
    if low < 10 { format!("{} oh {}", cardinal(high), ONES[low as usize]) } else { format!("{} {}", cardinal(high), cardinal(low)) }
}

pub fn pairs(number: i128) -> String {
    let (high, low) = (number / 100, number % 100);
    if !(100..=9999).contains(&number) || (low == 0 && (high < 10 || high % 10 == 0)) {
        return cardinal(number);
    }
    if low == 0 {
        return format!("{} hundred", cardinal(high));
    }
    if low < 10 { format!("{} oh {}", cardinal(high), ONES[low as usize]) } else { format!("{} {}", cardinal(high), cardinal(low)) }
}

fn int(text: &str) -> i128 {
    let mut n: i128 = 0;
    for c in text.chars() {
        n = n.saturating_mul(10).saturating_add(pyre::digit_value(c) as i128);
    }
    n
}

/// An unsigned number as written: `1,234`, `3.14`, `007`.
pub fn number_words(text: &str) -> String {
    let plain = text.replace(',', "");
    let (whole, fraction) = plain.split_once('.').unwrap_or((&plain, ""));
    if !fraction.is_empty() {
        let head = if whole.is_empty() { String::new() } else { cardinal(int(whole)) };
        return format!("{head} point {}", digits(fraction, "zero"));
    }
    let n = pyre::len(whole);
    if (n > 1 && whole.starts_with('0')) || n > 18 {
        return digits(whole, "zero");
    }
    cardinal(int(whole))
}

fn duration(hours: i128, minutes: i128, seconds: i128) -> String {
    let parts: Vec<String> = [(hours, "hour"), (minutes, "minute"), (seconds, "second")]
        .iter()
        .filter(|(count, _)| *count != 0)
        .map(|(count, name)| format!("{} {}", cardinal(*count), if *count == 1 { name.to_string() } else { format!("{name}s") }))
        .collect();
    if parts.len() < 2 {
        return parts.first().cloned().unwrap_or_else(|| "zero seconds".into());
    }
    format!("{} and {}", parts[..parts.len() - 1].join(", "), parts.last().unwrap())
}

fn plural_of(words: &str) -> String {
    let (head, last) = words.rsplit_once(' ').unwrap_or(("", words));
    let last = if let Some(stem) = last.strip_suffix('y') { format!("{stem}ies") } else { format!("{last}s") };
    if head.is_empty() { last } else { format!("{head} {last}") }
}

fn fraction_words(top: i128, bottom: i128, article: bool) -> String {
    let (one, many) = match bottom {
        2 => ("half".to_string(), "halves".to_string()),
        4 => ("quarter".into(), "quarters".into()),
        _ => (ordinal(bottom), format!("{}s", ordinal(bottom))),
    };
    if top != 1 {
        return format!("{} {many}", cardinal(top));
    }
    let lead = if article {
        if "aeiou".contains(one.chars().next().unwrap()) { "an " } else { "a " }
    } else {
        "one "
    };
    format!("{lead}{one}")
}

fn version_part(part: &str) -> String {
    if pyre::len(part) > 2 { digits(part, "zero") } else { number_words(part) }
}

fn minor(part: &str) -> String {
    if part.starts_with('0') { digits(part, "oh") } else { cardinal(int(part)) }
}

fn short_number(text: &str) -> bool {
    if text.starts_with('0') {
        return false;
    }
    pyre::len(text) <= 2 || text.chars().skip(1).all(|c| c == '0')
}

fn spelled(letters: &str) -> String {
    format!("{SPELL}{letters}{END}")
}

/// The words just before `position` in the same clause, in lower case.
fn before_words(text: &str, position: usize, count: usize) -> Vec<String> {
    let from = pyre::back_chars(text, position, 80);
    let pieces = rx!(r#"[.!?;…]\s|[()"]"#).split(&text[from..position]);
    let clause = pieces.last().unwrap();
    let words: Vec<String> = rx!(r"[A-Za-z]+").findall(clause).iter().map(|w| w.to_lowercase()).collect();
    words[words.len().saturating_sub(count)..].to_vec()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Phone,
    Pin,
    Strong,
    Weak,
    Enter,
}

fn identifier(text: &str, position: usize) -> Option<Kind> {
    let words = before_words(text, position, 4);
    if words.is_empty() {
        return None;
    }
    let last = words.last().unwrap().as_str();
    let n = words.len();
    if phone_words().contains(last)
        || (phone_links().contains(last) && words[n.saturating_sub(3)..n - 1].iter().any(|w| phone_words().contains(w.as_str())))
    {
        return Some(Kind::Phone);
    }
    let mut index = n - 1;
    while index > 0 && link_words().contains(words[index].as_str()) {
        index -= 1;
    }
    let word = words[index].as_str();
    for (kind, group) in [(Kind::Phone, phone_words()), (Kind::Pin, pin_words()), (Kind::Strong, strong_words()), (Kind::Weak, weak_words())] {
        if group.contains(word) {
            return Some(kind);
        }
    }
    if last == "enter" { Some(Kind::Enter) } else { None }
}

fn decimal_comma(text: &str) -> String {
    if rx!(r"\d+,\d{1,2}").fullmatch(text).is_some() { text.replace(',', ".") } else { text.to_string() }
}

fn address_digits(text: &str) -> String {
    let n = pyre::len(text);
    if n <= 2 && !(n == 2 && text.starts_with('0')) {
        return cardinal(int(text));
    }
    if n == 4 && (1900..=2099).contains(&int(text)) {
        return year(int(text));
    }
    digits(text, "zero")
}

/// `re.escape` of Python 3.7+.
fn re_escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if "()[]{}?*+-|^$\\.&~# \t\n\r\x0b\x0c".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// What the words "follow a new sentence" need: the text after a match starts a new sentence.
fn new_sentence(following: &str) -> bool {
    let first = rx!(r"[A-Za-z']+").match_(following);
    following.is_empty()
        || (pyre::c_isupper(following.chars().next().unwrap())
            && first.is_some_and(|f| sentence_starters().contains(f.all().to_lowercase().as_str())))
}

// ---- tables and patterns built once ------------------------------------------------------------------------------

/// The tables and the patterns that depend on them, built once per phonemizer.
pub struct Tables {
    abbreviations: HashMap<String, (String, String)>,
    units: HashMap<String, (String, String)>,
    exact_units: HashMap<String, (String, String)>,
    chat: HashMap<String, String>,
    replies: HashSet<String>,
    endings: HashMap<String, String>,
    unit: Re,
    area: Re,
    area_plain: Re,
    dotted_form: Re,
    dot_times_after: Re,
    unit_after: Re,
}

impl Tables {
    pub fn new(abbreviations: &str, units: &str, chat: &str, endings: &str) -> Result<Tables, String> {
        let abbreviations: HashMap<String, (String, String)> = tables::rows(abbreviations)
            .into_iter()
            .map(|(_, c)| (c[0].clone(), (c.get(1).cloned().unwrap_or_default(), c.get(2).cloned().unwrap_or_default())))
            .collect();
        let short: Vec<&str> = abbreviations.keys().map(String::as_str).chain(["st", "dr"]).filter(|k| pyre::len(k) == 2).collect();
        let mut titles: Vec<String> = short
            .iter()
            .flat_map(|k| {
                let mut cs = k.chars();
                let first = cs.next().unwrap().to_uppercase().to_string();
                [format!("{first}{}", cs.as_str().to_lowercase()), k.to_uppercase()]
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        titles.sort();
        let titles = titles.join("|");
        let part = format!(r"(?:[A-Za-z]|(?!(?:{titles})\.)[A-Z][A-Za-z])\.");
        let last = r"(?:(?:(?<=[a-z]\.)[a-z]|(?<=[A-Z]\.)[A-Z])(?![A-Za-z\d]))";
        let dotted_form = Re::new(&format!(
            r"\b(?:(?:{part}){{2,}}{last}?|(?:[A-Z]|(?!(?:{titles})\.)[B-DF-HJ-NP-TV-Z][b-df-hj-np-tv-z])\.[A-Z](?![A-Za-z\d])|(?:[Ee]\.[Gg]|[Ii]\.[Ee])(?![A-Za-z\d]))"
        ));
        let mut unit_map = HashMap::new();
        let mut exact = HashMap::new();
        for (_, c) in tables::rows(units) {
            let names = (c.get(1).cloned().unwrap_or_default(), c.get(2).cloned().unwrap_or_default());
            if c.len() > 3 && c[3] == "exact" {
                exact.insert(c[0].clone(), names);
            } else {
                unit_map.insert(c[0].to_lowercase(), names);
            }
        }
        let mut chat_map = HashMap::new();
        let mut replies = HashSet::new();
        for (number, c) in tables::rows(chat) {
            if c.len() > 2 && c[2..] != ["alone".to_string()] {
                return Err(format!("chat.txt line {number}: after the words to say there can only be \"alone\""));
            }
            chat_map.insert(c[0].clone(), c.get(1).cloned().unwrap_or_else(|| "-".into()));
            if c.len() > 2 {
                replies.insert(c[0].clone());
            }
        }
        let mut names: Vec<String> =
            unit_map.keys().chain(exact.keys()).map(|u| re_escape(u)).collect::<HashSet<_>>().into_iter().collect();
        names.sort_by(|a, b| pyre::len(b).cmp(&pyre::len(a)).then_with(|| a.cmp(b)));
        let unit_names = names.join("|");
        let unit = Re::new_i(&format!(
            r"(?<![\w.])(\d[\d,]*(?:\.\d+)?)(?:[-–](\d[\d,]*(?:\.\d+)?))?\s?({unit_names})(?![A-Za-z])(\.(?=\s+(?-i:[a-z])))?"
        ));
        Ok(Tables {
            abbreviations,
            units: unit_map,
            exact_units: exact,
            chat: chat_map,
            replies,
            endings: tables::mapping(endings),
            unit,
            area: Re::new(r"(?<![\w.])(\d[\d,]*(?:\.\d+)?)\s?(km|cm|mm|m|ft|mi|yd|in)([²³])"),
            area_plain: Re::new(r"(?<![\w.])(\d[\d,]*(?:\.\d+)?)\s?(km|cm|mm|m|ft)([23])(?![\w])"),
            dotted_form,
            dot_times_after: Re::new_i(&format!(r"\s?(?:%|percent|million|billion|{unit_names})(?![A-Za-z])")),
            unit_after: Re::new_i(&format!(r"\s?(?:{unit_names})(?![A-Za-z])")),
        })
    }
}

// ---- the normaliser ------------------------------------------------------------------------------------------------

/// One run of the normaliser over a text.
pub struct Normalizer<'a> {
    t: &'a Tables,
    is_word: &'a dyn Fn(&str) -> bool,
    inline: Vec<(String, Vec<Ph>)>,
    day_first: bool,
    dated: bool,
}

fn without_angles(text: &str) -> String {
    let mut text = text.to_string();
    let angles = rx!(format!(
        r"(?<![\w){END}{IS}])<[A-Za-z0-9/!?][^<>(){{}}+*;|\n]*>|(?<=[\w{END}{IS}])<[^<>(){{}}+*;|\n]{{1,80}}>(?![\w$])|</?[A-Za-z][\w-]+/?>"
    ));
    while text.contains('<') && text.contains('>') {
        let marked = angles.sub(&text, |m| {
            let g = m.all();
            format!("{ANGLE_MARK}{}{ANGLE_MARK}", &g[1..g.len() - 1])
        });
        if marked == text {
            break;
        }
        text = marked;
    }
    text
}

fn letter_places() -> &'static [Re] {
    static P: std::sync::LazyLock<Vec<Re>> = std::sync::LazyLock::new(|| {
        [
            format!(r"(?<![\w'’.-])(?P<letter>[Aakur])(?={IS}?\s?[<>!=]?=)"),
            format!(r"[<>!=]?=\s?(?P<letter>[Aakur]){NO_PHRASE}"),
            format!(r"(?<![\w'’.-])(?P<letter>[Aakur])(?={IS}?{SIGN}(?:[\w($]|\.\d))"),
            format!(r"(?<=[\w){IS}]{SIGN})(?<![Ww]/)(?P<letter>[Aakur])(?![\w'’-])"),
            format!(r"(?<![\w'’.-])(?P<letter>[Aakur])(?={IS}?{SPACED_SIGN}(?:[A-Za-z]|\d[\d.,]*)(?![\w'’]))"),
            format!(r"(?<![\w'’.])(?:[A-Za-z]|\d[\d.,]*){IS}?{SPACED_SIGN}(?P<letter>[Aakur]){NO_PHRASE}"),
            r"(?<=\()(?P<letter>[Aakur])(?=\)|, )".to_string(),
            r"(?<=, )(?P<letter>[Aakur])(?=\))".to_string(),
            r"(?:(?<=\s)|^)(?P<letter>[akur])(?=\)\s)".to_string(),
        ]
        .iter()
        .map(|p| Re::new(p))
        .collect()
    });
    &P
}

fn chat_words(chat: &str, raw: &str) -> Vec<Word> {
    if chat == "-" {
        return Vec::new();
    }
    chat.split_whitespace()
        .map(|part| match part.strip_prefix('*') {
            Some(letters) => Word { text: letters.to_lowercase(), raw: raw.into(), spell: true, ..Default::default() },
            None => w(&part.to_lowercase(), part),
        })
        .collect()
}

impl<'a> Normalizer<'a> {
    pub fn new(t: &'a Tables, is_word: &'a dyn Fn(&str) -> bool) -> Normalizer<'a> {
        Normalizer { t, is_word, inline: Vec::new(), day_first: false, dated: false }
    }

    fn is_word(&self, word: &str) -> bool {
        (self.is_word)(word)
    }

    // -- pieces of the text that are not words --

    fn amount(&self, symbol: &str, whole: &str, fraction: Option<&str>, scale: Option<&str>) -> String {
        let (one, many, cent, cents) = currency(symbol);
        if let Some(scale) = scale {
            let lower = scale.to_lowercase();
            let scale = match lower.as_str() {
                "k" => "thousand",
                "m" => "million",
                "b" | "bn" => "billion",
                "t" => "trillion",
                s => s,
            };
            let value = match fraction {
                Some(f) if !f.is_empty() => format!("{whole}.{f}"),
                _ => whole.to_string(),
            };
            return format!("{} {scale} {many}", number_words(&value));
        }
        let fraction = fraction.unwrap_or("");
        if !fraction.is_empty() && (pyre::len(fraction) > 2 || cent.is_empty()) {
            return format!("{} {many}", number_words(&format!("{whole}.{fraction}")));
        }
        let amount = int(whole);
        let change = int(&format!("{:0<2}", if fraction.is_empty() { "0" } else { fraction }));
        let mut parts = Vec::new();
        if amount != 0 || change == 0 {
            parts.push(format!("{} {}", cardinal(amount), if amount == 1 { one } else { many }));
        }
        if change != 0 && !cent.is_empty() {
            parts.push(format!("{} {}", cardinal(change), if change == 1 { cent } else { cents }));
        }
        parts.join(" and ")
    }

    fn continental(m: &M) -> String {
        let groups: Vec<&str> = m.gs(1).split('.').collect();
        if groups.len() == 4 && groups.iter().all(|g| int(g) <= 255) {
            return m.all().to_string();
        }
        format!("{}.{}", groups.join(","), m.gs(2))
    }

    fn money(&self, m: &M) -> String {
        let whole = m.n("whole").unwrap_or("").replace([',', ' '], "");
        let words = self.amount(m.n("symbol").unwrap_or("$"), &whole, m.n("fraction"), m.n("scale"));
        let minus = m.n("minus").is_some_and(|s| !s.is_empty()) || m.n("inner").is_some_and(|s| !s.is_empty());
        format!("{}{words} ", if minus { " minus " } else { " " })
    }

    fn half_day(m: &M, dotted: Option<&str>, plain: Option<&str>) -> String {
        let letter = match dotted.filter(|s| !s.is_empty()).or(plain.filter(|s| !s.is_empty())) {
            Some(l) => l,
            None => return String::new(),
        };
        let following = pyre::lstrip(&m.string[m.end()..]);
        let closes = dotted.is_some_and(|d| !d.is_empty()) && m.all().ends_with('.') && new_sentence(following);
        format!("{SPELL}{}M{END}{} ", letter.to_uppercase(), if closes { " ." } else { "" })
    }

    fn time(&self, m: &M) -> String {
        let (hour, minute, second) = (int(m.gs(1)), int(m.gs(2)), m.g(3));
        let half = m.g(4).or(m.g(5)).filter(|s| !s.is_empty());
        let before = before_words(m.string, m.start(), 3);
        if let Some(second) = second {
            if half.is_none() && !(before.last().is_some_and(|w| w == "at")) {
                return format!(" {} ", duration(hour, minute, int(second)));
            }
        }
        if hour > 24 && second.is_none() && half.is_none() {
            let after = rx!(r"\s*([A-Za-z]+)").match_(&m.string[m.end()..]);
            if before.iter().any(|w| duration_words().contains(w.as_str()))
                || after.is_some_and(|a| duration_words().contains(a.gs(1).to_lowercase().as_str()))
            {
                return format!(" {} ", duration(0, hour, minute));
            }
            return format!(" {} {}{} ", cardinal(hour), if minute < 10 { "oh " } else { "" }, cardinal(minute));
        }
        if hour > 24 {
            return m.all().to_string();
        }
        let mut words = cardinal(hour);
        if minute == 0 {
            if half.is_some() {
            } else if hour == 0 {
                words = "midnight".into();
            } else if hour > 12 {
                words += " hundred";
            } else {
                words += " o'clock";
            }
        } else {
            words += if minute < 10 { " oh " } else { " " };
            words += &cardinal(minute);
        }
        if let Some(second) = second.filter(|s| !s.is_empty()) {
            let s = int(second);
            words += &format!(" and {}{}", cardinal(s), if s == 1 { " second" } else { " seconds" });
        }
        format!(" {words} {}", Self::half_day(m, m.g(4), m.g(5)))
    }

    fn dot_times(&self, m: &M) -> String {
        let following = &m.string[m.end()..];
        let before = before_words(m.string, m.start(), 4);
        if int(m.gs(1)) > 23
            || int(m.gs(4)) > 23
            || !before.iter().any(|w| clock_words().contains(w.as_str()))
            || self.t.dot_times_after.match_(following).is_some()
        {
            return m.all().to_string();
        }
        format!("{}:{}{}{}:{}", m.gs(1), m.gs(2), m.gs(3), m.gs(4), m.gs(5))
    }

    fn ratio(&self, m: &M) -> String {
        let (first, second) = (m.gs(1), m.gs(2));
        if pyre::len(first) <= 2 && pyre::len(second) == 2 && "012345".contains(second.chars().next().unwrap()) {
            let after = rx!(r"\s*([A-Za-z]+)").match_(&m.string[m.end()..]);
            let said = before_words(m.string, m.start(), 3).iter().any(|w| ratio_words().contains(w.as_str()))
                || after.is_some_and(|a| ratio_words().contains(a.gs(1).to_lowercase().as_str()));
            if !said {
                return m.all().to_string();
            }
        }
        format!(" {} to {} ", number_words(first), number_words(second))
    }

    fn date(&self, month: i128, day: i128, year_text: Option<&str>, the: bool) -> Option<String> {
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let name = MONTHS[month as usize - 1];
        let mut words = if the { format!("the {} of {name}", ordinal(day)) } else { format!("{name} {}", ordinal(day)) };
        if let Some(y) = year_text.filter(|y| !y.is_empty()) {
            words += " ";
            words += &if pyre::len(y) == 4 {
                year(int(y))
            } else {
                format!("{}{}", if int(y) < 10 { "oh " } else { "" }, cardinal(int(y)))
            };
        }
        Some(format!(" {words} "))
    }

    fn slash_date(&self, m: &M) -> String {
        let (mut first, mut second) = (int(m.gs(1)), int(m.gs(2)));
        if (first > 12 && 12 >= second) || (self.day_first && second <= 12) {
            std::mem::swap(&mut first, &mut second);
        }
        self.date(first, second, m.g(3), false).unwrap_or_else(|| m.all().to_string())
    }

    fn dot_date(&self, m: &M) -> String {
        let (mut day, mut month) = (int(m.gs(1)), int(m.gs(2)));
        if month > 12 && 12 >= day {
            std::mem::swap(&mut day, &mut month);
        }
        self.date(month, day, m.g(3), false).unwrap_or_else(|| m.all().to_string())
    }

    fn day_month(&self, m: &M) -> String {
        let month = m.gs(2);
        let Some(spoken) = self.date(month_number(month).unwrap() as i128, int(m.gs(1)), m.g(4), true) else {
            return m.all().to_string();
        };
        let stop = m.g(3).is_some_and(|s| !s.is_empty()) && MONTHS.contains(&month);
        format!("{spoken}{}", if stop { ". " } else { "" })
    }

    fn days_of_month(&self, m: &M) -> String {
        let month_first = pyre::c_isalpha(m.gs(1).chars().next().unwrap());
        let (month, first, second) = if month_first { (m.gs(1), m.gs(2), m.gs(3)) } else { (m.gs(3), m.gs(1), m.gs(2)) };
        let (first, second) = (int(first), int(second));
        if !((1..=31).contains(&first) && (1..=31).contains(&second)) {
            return m.all().to_string();
        }
        let name = MONTHS[month_number(month).unwrap() - 1];
        let words = if month_first {
            format!("{name} {} to {}", ordinal(first), ordinal(second))
        } else {
            format!("the {} to the {} of {name}", ordinal(first), ordinal(second))
        };
        let y = m.g(4).filter(|s| !s.is_empty()).map(|y| format!(" {}", year(int(y)))).unwrap_or_default();
        format!(" {words}{y} ")
    }

    fn day_range(&self, m: &M) -> String {
        let mut names = Vec::new();
        for name in [m.gs(1), m.gs(3)] {
            let key = name.to_lowercase();
            names.push(if let Some(a) = self.t.abbreviations.get(&key) {
                a.1.clone()
            } else {
                day_name(&key).map(String::from).unwrap_or_else(|| name.to_string())
            });
        }
        let following = pyre::lstrip(&m.string[m.end()..]);
        let ends = m.g(4).is_some_and(|s| !s.is_empty()) && new_sentence(following);
        format!("{} to {}{}", names[0], names[1], if ends { " ." } else { "" })
    }

    fn short_date(&self, m: &M) -> Option<String> {
        let (ft, st) = (m.gs(1), m.gs(2));
        if pyre::len(ft) > 2 || pyre::len(st) > 2 {
            return None;
        }
        let (mut first, mut second) = (int(ft), int(st));
        let before = before_words(m.string, m.start(), 4);
        if (1..=12).contains(&first) && (20..=49).contains(&second) && before.iter().any(|w| w == "card") {
            return Some(format!(" {} {} ", MONTHS[first as usize - 1], year(2000 + second)));
        }
        let last = before.last().map(String::as_str).unwrap_or("");
        let classic = first < second && CLASSIC_FRACTIONS.contains(&second);
        let zero = (pyre::len(ft) == 2 && ft.starts_with('0')) || (pyre::len(st) == 2 && st.starts_with('0'));
        if !(zero
            || (12 < first && first <= 31 && second <= 12 && (first, second) != (24, 7))
            || (date_before().contains(last) && (!classic || last == "on"))
            || (self.dated && !classic && first != second))
        {
            return None;
        }
        if first > 12 || (self.day_first && second <= 12) {
            std::mem::swap(&mut first, &mut second);
        }
        self.date(first, second, None, false)
    }

    fn fraction(&self, m: &M) -> String {
        if let Some(spoken) = self.short_date(m) {
            return spoken;
        }
        let (top, bottom) = (int(m.gs(1)), int(m.gs(2)));
        if (top, bottom) == (24, 7) || top == bottom {
            return format!(" {} {} ", cardinal(top), cardinal(bottom));
        }
        if !(0 < top && top < 100) || !(1 < bottom && bottom <= 100) {
            return format!(" {} over {} ", cardinal(top), cardinal(bottom));
        }
        format!(" {} ", fraction_words(top, bottom, false))
    }

    fn mixed_fraction(&self, m: &M) -> String {
        let (top, bottom) = (int(m.gs(2)), int(m.gs(3)));
        if !(0 < top && top < bottom) {
            return m.all().to_string();
        }
        format!(" {} and {} ", number_words(m.gs(1)), fraction_words(top, bottom, true))
    }

    fn vulgar(&self, m: &M) -> String {
        let normal: String = m.gs(2).nfkc().collect();
        let mut parts = normal.split('⁄').map(int);
        let (top, bottom) = (parts.next().unwrap_or(0), parts.next().unwrap_or(1));
        match m.g(1) {
            None => format!(" {top}/{bottom} "),
            Some(whole) => format!(" {} and {} ", cardinal(int(whole)), fraction_words(top, bottom, true)),
        }
    }

    fn phone(&self, m: &M) -> String {
        let groups: Vec<String> = (1..m.groups())
            .filter_map(|i| m.g(i))
            .filter(|g| !g.is_empty())
            .map(|g| if g == "800" { "eight hundred".to_string() } else { digits(g, "oh") })
            .collect();
        format!(" {} ", groups.join(", "))
    }

    fn measure(&self, m: &M) -> String {
        let unit = m.gs(3);
        let names = self.t.exact_units.get(unit).or_else(|| self.t.units.get(&unit.to_lowercase()));
        let end3 = m.end_g(3).unwrap();
        let next = char_at(m.string, end3);
        if names.is_none() || (pyre::len(unit) == 1 && matches!(next, Some('-') | Some('\'') | Some('/'))) {
            return m.all().to_string();
        }
        // tiny-voca JS port: "p. 45" after a space is a page, not pence.
        if unit == "p"
            && char_before(m.string, end3 - 1).is_some_and(pyre::isspace_char)
            && rx!(r"\.\s?\d").match_(&m.string[end3..]).is_some()
        {
            return m.all().to_string();
        }
        let (one, many) = names.unwrap();
        let value = decimal_comma(m.gs(1));
        if let Some(upper) = m.g(2).filter(|s| !s.is_empty()) {
            return format!(" {} to {} {many} ", number_words(&value), number_words(&decimal_comma(upper)));
        }
        format!(" {} {} ", number_words(&value), if value == "1" || value == "1.0" { one } else { many })
    }

    fn square(&self, m: &M) -> String {
        let unit = m.gs(2);
        let inch = ("inch".to_string(), "inches".to_string());
        let (one, many) = if unit == "in" {
            &inch
        } else {
            self.t.exact_units.get(unit).or_else(|| self.t.units.get(unit)).unwrap()
        };
        let kind = if "²2".contains(m.gs(3)) { "square" } else { "cubic" };
        let value = decimal_comma(m.gs(1));
        format!(" {} {kind} {} ", number_words(&value), if value == "1" || value == "1.0" { one } else { many })
    }

    fn counted(&self, text: &str, end: usize) -> bool {
        let Some(after) = rx!(r"\s*([A-Za-z]+)").match_(&text[end..]) else { return false };
        let word = after.gs(1).to_lowercase();
        if ["people", "men", "women", "children", "feet", "miles"].contains(&word.as_str()) {
            return true;
        }
        word.len() > 3
            && word.ends_with('s')
            && !word.ends_with("ss")
            && !not_counted().contains(word.as_str())
            && self.is_word(&word[..word.len() - 1])
    }

    fn number(&self, m: &M, text: &str) -> String {
        let value = m.all();
        let plain = value.replace(',', "");
        if !plain.contains('.') && !value.contains(',') && !plain.starts_with('0') && pyre::len(&plain) > 2 {
            let kind = identifier(text, m.start());
            let limit = match kind {
                Some(Kind::Phone) | Some(Kind::Pin) => 3,
                Some(Kind::Strong) | Some(Kind::Enter) => 4,
                Some(Kind::Weak) => 5,
                None => 7,
            };
            let n = pyre::len(&plain);
            if n >= limit
                && !(kind.is_none() && plain.ends_with("000"))
                && !(kind == Some(Kind::Enter) && n == 4 && (1900..=2099).contains(&int(&plain)))
                && !(matches!(kind, None | Some(Kind::Phone) | Some(Kind::Weak)) && self.counted(text, m.end()))
            {
                return format!(" {} ", digits(&plain, if kind == Some(Kind::Phone) { "oh" } else { "zero" }));
            }
        }
        if !plain.contains('.') && !value.contains(',') && pyre::len(&plain) == 4 && !plain.starts_with('0') {
            let number = int(&plain);
            let from = pyre::back_chars(text, m.start(), 24);
            let before = rx!(r"[A-Za-z]+").findall(&text[from..m.start()]);
            let after = rx!(r"\s*([A-Za-z]+)").match_(&text[m.end()..]);
            let counted = after.is_some_and(|a| {
                rx!(r"(?:[^s]s|people|men|women|children|feet|miles)$").search(&a.gs(1).to_lowercase()).is_some()
            });
            let dated = before.last().is_some_and(|b| {
                let l = b.to_lowercase();
                year_context().contains(l.as_str()) || month_number(&l).is_some() && l.len() >= 3
            });
            let model = rx!(r"(?<![A-Za-z])([A-Z]{2,5}) $").search(&text[..m.start()]);
            if model.is_some_and(|x| !capital_words().contains(x.gs(1))) && !counted {
                return format!(" {} ", pairs(number));
            }
            if (1100..=1999).contains(&number) || ((2000..=2099).contains(&number) && (dated || !counted)) {
                return format!(" {} ", year(number));
            }
        }
        format!(" {} ", number_words(value))
    }

    // -- codes and reference numbers --

    fn reads_as_word(&self, letters: &str) -> bool {
        let lower = letters.to_lowercase();
        if pyre::len(&lower) < 3 || !self.is_word(&lower) {
            return false;
        }
        pyre::len(&lower) > 3 || ["s", "es", "ed", "ing"].iter().any(|e| self.is_word(&format!("{lower}{e}")))
    }

    fn alnum(group: &str, zero: &str) -> String {
        rx!(r"[A-Za-z]+|\d+")
            .findall(group)
            .iter()
            .map(|run| if pyre::isdigit(run) { digits(run, zero) } else { spelled(run) })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn digit_groups(groups: &[&str], zero: &str) -> String {
        if groups.iter().all(|g| short_number(g) && pyre::len(g) <= 2) {
            return groups.iter().map(|g| cardinal(int(g))).collect::<Vec<_>>().join(" , ");
        }
        groups.iter().map(|g| digits(g, zero)).collect::<Vec<_>>().join(" , ")
    }

    fn code_words(&self, groups: &[&str], m: &M) -> Option<String> {
        let token = groups.join("-");
        if !token.chars().any(|c| pyre::c_isdigit(c)) {
            return None;
        }
        let kinds: Vec<char> = groups
            .iter()
            .map(|g| {
                if pyre::isdigit(g) {
                    'd'
                } else if g.chars().any(pyre::c_isdigit) {
                    'm'
                } else if pyre::isupper(g) {
                    'u'
                } else {
                    'l'
                }
            })
            .collect();
        if kinds.iter().all(|&k| k == 'd') {
            if groups.len() == 2 {
                let years = groups.iter().all(|g| pyre::len(g) == 4 && (1000..=2100).contains(&int(g)));
                let leading_zero = groups.iter().any(|g| pyre::len(g) > 1 && g.starts_with('0'));
                let min_len = groups.iter().map(|g| pyre::len(g)).min().unwrap();
                if !(m.g(1).is_some_and(|s| !s.is_empty())
                    || leading_zero
                    || (min_len >= 4 && !years)
                    || matches!(identifier(m.string, m.start()), Some(Kind::Strong) | Some(Kind::Pin)))
                {
                    return None;
                }
            }
            return Some(Self::digit_groups(groups, "zero"));
        }
        if groups.len() == 2 && kinds[1] == 'd' && (kinds[0] == 'u' || kinds[0] == 'l') {
            let (letters, number) = (groups[0], groups[1]);
            if (kinds[0] == 'l' && pyre::len(letters) > 1) || self.reads_as_word(letters) {
                return None;
            }
            if short_number(number) {
                return Some(format!("{} {}", spelled(letters), cardinal(int(number))));
            }
            return Some(format!("{} , {}", spelled(letters), digits(number, "zero")));
        }
        if groups.len() == 2
            && kinds[0] == 'd'
            && (kinds[1] == 'l' || pyre::len(groups[1]) == 1 || rx!(r"\d+[A-Za-z]+").fullmatch(groups[1]).is_some())
        {
            return None;
        }
        if kinds.iter().zip(groups).any(|(&k, g)| {
            (k == 'l' && pyre::len(g) > 2) || (k == 'm' && rx!(r"\d+(?:s|st|nd|rd|th)").fullmatch(g).is_some())
        }) {
            return None;
        }
        if !token.chars().any(pyre::c_isupper) && (kinds.contains(&'l') || kinds.contains(&'m')) {
            return None;
        }
        let parts: Vec<String> = kinds
            .iter()
            .zip(groups)
            .map(|(&k, g)| match k {
                'd' => digits(g, "zero"),
                'm' => Self::alnum(g, "zero"),
                _ if pyre::len(g) > 3 && self.is_word(&g.to_lowercase()) => g.to_string(),
                _ => {
                    if pyre::len(g) <= 5 {
                        spelled(g)
                    } else {
                        g.to_string()
                    }
                }
            })
            .collect();
        Some(parts.join(" , "))
    }

    fn code(&self, m: &M) -> String {
        let groups: Vec<&str> = m.gs(2).split('-').collect();
        match self.code_words(&groups, m) {
            None => m.all().to_string(),
            Some(words) => format!("{}{words} ", if m.g(1).is_some_and(|s| !s.is_empty()) { " number " } else { " " }),
        }
    }

    fn grouped(&self, m: &M) -> String {
        let text = m.string;
        let plus = m.n("plus").filter(|s| !s.is_empty());
        let mut lead = m.n("lead").filter(|s| !s.is_empty());
        let mut trail = m.n("trail").filter(|s| !s.is_empty());
        let core_text = m.n("core").or(m.n("core2")).unwrap_or("");
        let core: Vec<&str> = core_text.split(' ').collect();
        let (mut head, mut tail) = (String::new(), String::new());
        if let Some(l) = lead {
            if capital_words().contains(l) {
                head = format!("{l} ");
                tail = trail.map(|t| format!(" {t}")).unwrap_or_default();
                lead = None;
                trail = None;
            }
        }
        let kind = identifier(text, m.start());
        let pure = core.iter().all(|p| pyre::isdigit(p));
        let first_char = core[0].chars().next().unwrap_or(' ');
        if pure && plus.is_none() && lead.is_none() {
            let following = &text[m.end()..];
            let amount = self.counted(text, m.end()) || self.t.unit_after.match_(following).is_some();
            if kind.is_none()
                && first_char != '0'
                && pyre::len(core[0]) <= 3
                && core[1..].iter().all(|p| pyre::len(p) == 3)
                && (*core.last().unwrap() == "000" || (amount && core.len() > 2))
            {
                return format!("{head}{}{tail}", core.join(","));
            }
            if !(kind.is_some()
                || (pyre::len(core[0]) > 1 && first_char == '0')
                || (core.len() > 2 && core.iter().any(|p| pyre::len(p) > 2)))
            {
                return m.all().to_string();
            }
        } else if !pure && lead.is_none() && plus.is_none() && kind.is_none() {
            return m.all().to_string();
        }
        let total: usize = core.iter().map(|p| pyre::len(p)).sum();
        let zero = if plus.is_some() || kind == Some(Kind::Phone) || (pure && first_char == '0' && total >= 10) { "oh" } else { "zero" };
        let mut spoken = if pure {
            Self::digit_groups(&core, zero)
        } else {
            core.iter()
                .map(|p| if pyre::isdigit(p) { digits(p, zero) } else { Self::alnum(p, zero) })
                .collect::<Vec<_>>()
                .join(" , ")
        };
        if let Some(l) = lead {
            spoken = format!("{} , {spoken}", spelled(l));
        }
        if let Some(t) = trail {
            spoken = format!("{spoken} , {}", spelled(t));
        }
        format!("{head}{}{spoken} {tail}", if plus.is_some() { " plus " } else { " " })
    }

    fn glued(&self, m: &M) -> String {
        let mut out = Vec::new();
        for run in rx!(r"[A-Za-z]+|\d+").findall(m.all()) {
            if pyre::isdigit(run) {
                out.push(if short_number(run) { cardinal(int(run)) } else { digits(run, "zero") });
            } else if pyre::len(run) == 1 || ((pyre::isupper(run) || pyre::len(run) <= 3) && !self.reads_as_word(run)) {
                out.push(spelled(run));
            } else {
                out.push(run.to_string());
            }
        }
        format!(" {} ", out.join(" "))
    }

    // -- web addresses and file names --

    fn address_part(&self, part: &str) -> String {
        let lower = part.to_lowercase();
        let n = pyre::len(part);
        if n > 1 && self.is_word(&lower) && (n > 2 || short_words().contains(lower.as_str())) {
            return part.to_string();
        }
        if n <= 3 { spelled(part) } else { part.to_string() }
    }

    fn spoken_address(&self, core: &str) -> String {
        let pieces = rx!(r"[A-Za-z]+|\d+|.").findall(core);
        let mail = core.contains('@');
        let mut host = false;
        let mut out: Vec<String> = Vec::new();
        for (index, piece) in pieces.iter().enumerate() {
            let piece = *piece;
            let following = pieces.get(index + 1).copied().unwrap_or("");
            if piece == "@" {
                host = true;
            }
            let lower = piece.to_lowercase();
            if pyre::isdigit(piece) {
                out.push(address_digits(piece));
            } else if !pyre::isalpha(piece) {
                out.push(
                    match piece {
                        "." => "dot",
                        "/" => "slash",
                        "@" => "at",
                        "-" => "dash",
                        "_" => "underscore",
                        ":" => "colon",
                        "=" => "equals",
                        "&" => "and",
                        "+" => "plus",
                        _ => "",
                    }
                    .to_string(),
                );
            } else if index > 0
                && pieces[index - 1] == "."
                && (host || !mail)
                && (["", "/", ":", "?", "#"].contains(&following)
                    || (pyre::len(piece) <= 3 && (self.t.endings.contains_key(&lower) || !self.is_word(&lower))))
            {
                out.push(if self.t.endings.get(&lower).is_some_and(|e| e == "word") { lower } else { spelled(piece) });
            } else {
                let mut letters = String::new();
                for part in rx!(r"[A-Z]?[a-z]+|[A-Z]+(?![a-z])").findall(piece) {
                    let spoken = self.address_part(part);
                    if spoken.starts_with(SPELL) {
                        letters += part;
                        continue;
                    }
                    if !letters.is_empty() {
                        out.push(spelled(&letters));
                        letters.clear();
                    }
                    out.push(spoken);
                }
                if !letters.is_empty() {
                    out.push(spelled(&letters));
                }
            }
        }
        out.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
    }

    fn address(&self, m: &M) -> String {
        let whole = m.all();
        let text = whole.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
        let tail = &whole[text.len()..];
        if m.n("file").is_some() {
            let ending = m.n("ending").unwrap_or("");
            let known = self.t.endings.contains_key(&ending.to_lowercase());
            let marked = m.n("folder").is_some_and(|s| !s.is_empty())
                || m.n("path").is_some_and(|s| !s.is_empty())
                || m.n("stem").unwrap_or("").contains('_');
            if !(known
                || (marked
                    && pyre::len(ending) <= 5
                    && (pyre::islower(ending) || pyre::isupper(ending))
                    && !self.is_word(&ending.to_lowercase())))
            {
                return whole.to_string();
            }
        }
        let core = rx!(r"^(?:https?://)?(?:www\.)?").sub(text, |_| String::new());
        let core = core.trim_end_matches('/');
        format!(" {} {tail}", self.spoken_address(core))
    }

    fn chat_form(&self, m: &M) -> String {
        let Some(said) = self.t.chat.get(&m.all().to_lowercase()) else {
            return m.all().to_string();
        };
        if said == "-" {
            return " ".into();
        }
        let parts: Vec<String> = said
            .split_whitespace()
            .map(|p| match p.strip_prefix('*') {
                Some(letters) => format!("{SPELL}{letters}{END}"),
                None => p.to_string(),
            })
            .collect();
        format!(" {} ", parts.join(" "))
    }

    fn inline(&mut self, m: &M) -> String {
        let symbols = pyre::words(m.gs(2));
        if symbols.is_empty() || symbols.iter().all(|s| pyre::len(s) == 1 && s.is_ascii()) {
            return m.all().to_string();
        }
        let phones = if is_arpabet(&symbols) { convert("", &symbols, None) } else { from_text(&symbols.join(" ")) };
        let Ok(phones) = phones else {
            return m.all().to_string();
        };
        self.inline.push((m.g(1).unwrap_or("").to_string(), phones));
        let code = char::from_u32(INLINE_BASE + self.inline.len() as u32 - 1).unwrap_or(INLINE);
        format!(" {INLINE}{code}{INLINE} ")
    }

    fn dotted(&self, m: &M, text: &str) -> String {
        let letters = m.all().replace('.', "");
        let key = letters.to_lowercase();
        let following = pyre::lstrip(&text[m.end()..]);
        let words = match key.as_str() {
            "eg" => "for example".to_string(),
            "ie" => "that is".to_string(),
            _ => format!("{SPELL}{}{END}", letters.to_uppercase()),
        };
        let ends = new_sentence(following);
        format!(" {words}{} ", if ends && m.all().ends_with('.') { " ." } else { "" })
    }

    fn abbreviation(&self, m: &M, text: &str) -> String {
        let (token, stop) = (m.gs(1), m.gs(2));
        let stop = !stop.is_empty();
        let key = token.to_lowercase();
        let following = &text[m.end()..];
        let next_word = rx!(r"\s*([A-Za-z]+|\d)").match_(following);
        let capital_next = next_word.as_ref().is_some_and(|n| pyre::c_isupper(n.gs(1).chars().next().unwrap()));
        let capital_before = rx!(r"[A-Z][a-z]+\s+$").search(&text[..m.start()]).is_some();
        let first_upper = pyre::c_isupper(token.chars().next().unwrap());
        if (key == "st" || key == "dr") && first_upper {
            if capital_next && (stop || key == "dr") {
                return if key == "st" { " saint " } else { " doctor " }.into();
            }
            if capital_before && stop {
                return format!(
                    "{}{}",
                    if key == "st" { " street" } else { " drive" },
                    if pyre::strip(following).is_empty() { " ." } else { " " }
                );
            }
            return m.all().to_string();
        }
        let Some((when, words)) = self.t.abbreviations.get(&key) else {
            return m.all().to_string();
        };
        let mut words = words.clone();
        let number_next = next_word.as_ref().is_some_and(|n| pyre::isdigit(n.gs(1)));
        match when.as_str() {
            "title" => {
                if !(first_upper && (capital_next || stop)) {
                    return m.all().to_string();
                }
            }
            "number" => {
                let previous = rx!(r"([A-Za-z]+)\.?\s+$").search(&text[..m.start()]);
                if !(stop && (number_next || (key == "no" && previous.is_some_and(|p| numbered().contains(p.gs(1).to_lowercase().as_str())))))
                {
                    return m.all().to_string();
                }
            }
            _ => {
                if !stop || (when == "small" && pyre::isupper(token)) {
                    return m.all().to_string();
                } else if ["mar", "sun", "sat", "jan", "dec", "max", "min", "est", "no"].contains(&key.as_str())
                    && !first_upper
                    && !number_next
                {
                    let lower_next = next_word.as_ref().is_some_and(|n| pyre::c_islower(n.gs(1).chars().next().unwrap()));
                    if !(["max", "min", "est"].contains(&key.as_str()) && lower_next) {
                        return m.all().to_string();
                    }
                }
            }
        }
        if key == "est" && !number_next {
            words = "estimated".into();
        }
        let ends = when != "title"
            && (pyre::strip(following).is_empty()
                || (capital_next && sentence_starters().contains(next_word.as_ref().unwrap().gs(1).to_lowercase().as_str())));
        format!(" {words}{} ", if ends { " ." } else { "" })
    }

    /// The text with every non-word written out, markers included.
    pub fn rewrite(&mut self, text: &str) -> String {
        let mut text = rx!(r"(?<![\d.,/])(?:(\d+)\s?)?([½⅓⅔¼¾⅕⅖⅗⅘⅙⅚⅐⅛⅜⅝⅞⅑⅒])").sub(text, |m| self.vulgar(m));
        text = self.t.area.sub(&text, |m| self.square(m));
        text = text.replace('²', " squared ").replace('³', " cubed ");
        text = text.nfkc().collect::<String>().replace('⁄', "/").replace('−', " minus ");
        if !text.is_ascii() {
            text = rx!(i, r"\b(résumé|resumé|exposé|rosé|pâté|maté|saké|lamé)(s?)\b").sub(&text, |m| {
                let arpa = accent_decides(&m.gs(1).to_lowercase()).unwrap_or("");
                format!("{{{arpa}{}}}", if m.gs(2).is_empty() { "" } else { " Z" })
            });
            text = text.chars().map(plain_letter).collect();
        }
        text = rx!(r"(?<=\d)–(?=\d)").sub(&text, |_| "-".into());
        text = rx!(format!(r"\b({DAY_PATTERN}|{MONTH_PATTERN})\b(\.?)\s?[-–]\s?({DAY_PATTERN}|{MONTH_PATTERN})\b(\.?)"))
            .sub(&text, |m| self.day_range(m));
        for (old, new) in [
            ('‘', "'"),
            ('’', "'"),
            ('‚', "'"),
            ('‛', "'"),
            ('`', "'"),
            ('´', "'"),
            ('“', "\""),
            ('”', "\""),
            ('„', "\""),
            ('‟', "\""),
            ('«', "\""),
            ('»', "\""),
            ('–', "—"),
            ('―', "—"),
            ('[', "("),
            (']', ")"),
        ] {
            text = text.replace(old, new);
        }
        self.inline.clear();
        let inline_re = rx!(r"([A-Za-z']+)?\{([^{}]*)\}");
        let found = inline_re.finditer(&text);
        if !found.is_empty() {
            let mut out = String::new();
            let mut last = 0;
            for m in &found {
                out.push_str(&text[last..m.start()]);
                let r = self.inline(m);
                out.push_str(&r);
                last = m.end();
            }
            out.push_str(&text[last..]);
            text = out;
        }
        text = text.replace('{', "(").replace('}', ")");
        text = rx!(r#"(?P<url>(?:https?://|www\.)[^\s<>\"']+)|(?P<mail>[\w.+-]+@[\w-]+(?:\.[\w-]+)+)|(?P<file>(?<![\w@./-])(?P<folder>/?(?:[\w~-]+/)+)?(?P<stem>[\w-]+(?:\.[\w-]+)*)\.(?P<ending>[A-Za-z][A-Za-z0-9]{0,9})(?![\w-])(?P<path>/[^\s<>\"']*)?)"#)
            .sub(&text, |m| self.address(m));
        text = text.replace('→', " to ");
        text = rx!(r"[\U0001F000-\U0001FAFF☀-➿\U0001F1E6-\U0001F1FF←-⇿⬀-⯿️‍⃣]+")
            .sub(&text, |_| " ".into());
        text = rx!(concat!(
            r"(?<![A-Za-z0-9])(?:[:;=]['^-]?[DPpOo3|/\\*\]\[}{><]+|[)(\]\[][-^']?[:;=]|<3+|</3|\^_?\^|-_-|[oO]_[oO]|>_<|T_T|\\o/|:'[()])(?![A-Za-z0-9])",
            r"|[:;]['^-]?[)(]+",
            r"|(?<![A-Za-z0-9)])=['^-]?[)(]+|=['^-]?[)(]+(?![A-Za-z0-9(.$-])",
            r"|(?<=[A-Za-z]{2})<3+(?![.,]?\d)"
        ))
        .sub(&text, |_| " ".into());
        text = rx!(r"\.{3,}").sub(&text, |_| "…".into());
        text = rx!(r"(?<=\d)\s-\s(?=\d[\d.,]*\s?=)").sub(&text, |_| " minus ".into());
        text = rx!(r"(?<![\d.,/-])(\d[\d.,]*)-(?=\d[\d.,]*=)").sub_t(&text, r"\1 minus ");
        text = rx!(r"-{2,}|\s-\s").sub(&text, |_| "—".into());
        text = rx!(i, r"\bw/o\b").sub(&text, |_| " without ".into());
        text = rx!(i, r"\bw/(?=\s|$)").sub(&text, |_| " with ".into());
        text = rx!(i, r"\bn/a\b").sub(&text, |_| " not applicable ".into());
        text = text.replace("'n'", " 'n ");
        if rx!(r"[<>≥≤≠]|!=").is_match(&text) {
            text = Self::mark_comparisons(&text);
        }
        if rx!(r"\d").is_match(&text) {
            text = self.figures(&text);
        }
        self.signs(&text)
    }

    fn figures(&mut self, text: &str) -> String {
        let mut text = rx!(concat!(
            r"(?<![\w$])(?<!\d\s)\$(?:(?=[^\s\d$.,;:!?)])[^$\n]{0,80}",
            r"|(?=\d)[^$\n]{0,80}[=^_\\{}+*/<>][^$\n]{0,80}",
            r"|(?=\s)[^$\n]{0,80}[=^_\\{}+*<>][^$\n]{0,80})\d\s?\$(?![\w$])"
        ))
        .sub(text, |m| {
            let g = m.all();
            format!("{}{FORMULA_SIGN}", &g[..g.len() - 1])
        });
        text = rx!(r"(?<![\w.,])([1-9]\d{0,2}(?:\.\d{3})+),(\d{1,2})(?!\d|[.,]\d)").sub(&text, Self::continental);
        text = rx!(format!(r"((?:[$£€¥₹]|(?<![A-Za-z])(?:{CURRENCY_CODES}))\s?-?\d+),(\d{{1,2}})(?!\d|[.,]\d)")).sub_t(&text, r"\1.\2");
        text = rx!(format!(r"(?<![\w.,])(\d+),(\d{{1,2}})(?=\s?(?:%|{}))", sign_after())).sub_t(&text, r"\1.\2");
        text = rx!(format!(
            r"(?:(?<![\w.])(?P<minus>-)\s?)?(?P<symbol>[$£€¥₹]|(?<![A-Za-z])(?:{CURRENCY_CODES})(?![A-Za-z]))\s?(?P<inner>-)?{AMOUNT}(?:\s?(?P<scale>thousand|million|billion|trillion|bn|[kKmMbB])(?![A-Za-z]))?"
        ))
        .sub(&text, |m| self.money(m));
        text = rx!(format!(r"(?<![\w.,-])(\d[\d,]*(?:\.\d+)?)-(?=\d[\d,]*(?:\.\d+)?\s?(?:{}))", sign_after())).sub_t(&text, r"\1 to ");
        text = rx!(format!(
            r"(?:(?<![\w.])(?P<minus>-)\s?)?(?<![\w.,^])(?P<whole>\d{{1,3}}(?:,\d{{3}})+|[1-9]\d{{0,2}}(?: \d{{3}})+|\d+)(?:\.(?P<fraction>\d+))?\s?(?P<symbol>{})",
            sign_after()
        ))
        .sub(&text, |m| self.money(m));
        text = rx!(r"(?<![\w.])(\d[\d,]*(?:\.\d+)?)\s?¢")
            .sub(&text, |m| format!(" {} {} ", number_words(m.gs(1)), if m.gs(1) == "1" { "cent" } else { "cents" }));

        // times of day, lengths of time, ratios
        text = rx!(format!(r"({CLOCK})\s({ZONES})\b")).sub(&text, |m| format!("{} {}", m.gs(1), spelled(m.gs(2))));
        text = rx!(r"(?<![\d.,:])(\d{1,2})\.([0-5]\d)(?=\s?[AaPp]\.?[Mm]\b)").sub_t(&text, r"\1:\2");
        text = rx!(r"(?<![\d.,:])(\d{1,2})\.(00|15|30|45)(\s?(?:to|until|till|and|[-—])\s?)(\d{1,2})\.(00|15|30|45)(?!\d|\.\d)")
            .sub(&text, |m| self.dot_times(m));
        text = rx!(format!(r"({CLOCK})\s?[-—]\s?(?=\d{{1,2}}(?::[0-5]\d|\s?[AaPp]\.?[Mm]\b))")).sub_t(&text, r"\1 to ");
        text = rx!(r"(?<![\d:.,])(\d{1,3}):(\d{1,3}(?:,\d{3})+|\d+)(?![\d:]|[.,]\d)").sub(&text, |m| self.ratio(m));
        text = rx!(format!(r"(?<![\d:])(\d{{1,2}}):([0-5]\d)(?::([0-5]\d))?{HALF_DAY}?(?![\d:])")).sub(&text, |m| self.time(m));
        text = rx!(format!(r"(?<![\d.:,])(\d{{1,2}}){HALF_DAY}(?![A-Za-z])"))
            .sub(&text, |m| format!(" {} {}", cardinal(int(m.gs(1))), Self::half_day(m, m.g(2), m.g(3))));

        // dates
        let seen: Vec<(i128, i128)> = rx!(r"(?<![\d/.])(\d{1,2})/(\d{1,2})(?:/(?:\d{4}|\d{2}))?(?![\d/])")
            .finditer(&text)
            .iter()
            .map(|m| (int(m.gs(1)), int(m.gs(2))))
            .filter(|p| *p != (24, 7))
            .collect();
        self.day_first = seen.iter().any(|&(a, b)| a > 12 && 12 >= b) && !seen.iter().any(|&(a, b)| b > 12 && 12 >= a);
        self.dated = rx!(r"(?<![\d/])\d{1,2}/\d{1,2}/(?:\d{4}|\d{2})(?![\d/])").is_match(&text);
        text = rx!(r"(?<![\d-])(\d{4})-(\d{1,2})-(\d{1,2})(?![\d-])")
            .sub(&text, |m| self.date(int(m.gs(2)), int(m.gs(3)), m.g(1), false).unwrap_or_else(|| m.all().to_string()));
        text = rx!(r"(?<![\d/])(\d{4})/(\d{1,2})/(\d{1,2})(?![\d/])")
            .sub(&text, |m| self.date(int(m.gs(2)), int(m.gs(3)), m.g(1), false).unwrap_or_else(|| m.all().to_string()));
        text = rx!(r"(?<![\d.])(\d{1,2})\.(\d{1,2})\.(\d{4})(?!\d|\.\d)").sub(&text, |m| self.dot_date(m));
        text = rx!(r"(?<![\d/])(\d{1,2})/(\d{1,2})/(\d{4}|\d{2})(?![\d/])").sub(&text, |m| self.slash_date(m));
        text = rx!(format!(
            r"\b({MONTH_PATTERN})\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?\s?[-—]\s?(\d{{1,2}})(?:st|nd|rd|th)?(?:,?\s+(\d{{4}}))?(?![\d:])"
        ))
        .sub(&text, |m| self.days_of_month(m));
        text = rx!(format!(
            r"(?<![\d.,/:-])(\d{{1,2}})(?:st|nd|rd|th)?\s?[-—]\s?(\d{{1,2}})(?:st|nd|rd|th)?\s+({MONTH_PATTERN})\b(?:,?\s+(\d{{4}}))?"
        ))
        .sub(&text, |m| self.days_of_month(m));
        text = rx!(format!(r"\b({MONTH_PATTERN})\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?(?:,?\s+(\d{{4}}))?(?![\d:])")).sub(&text, |m| {
            self.date(month_number(m.gs(1)).unwrap() as i128, int(m.gs(2)), m.g(3), false).unwrap_or_else(|| m.all().to_string())
        });
        text = rx!(format!(
            r"(?<![\d.,/:-])(\d{{1,2}})(?:st|nd|rd|th)?\s+({MONTH_PATTERN})\b(\.(?=\s+[a-z\d]))?(?:,?\s+(\d{{4}}))?"
        ))
        .sub(&text, |m| self.day_month(m));

        // telephone numbers, versions, codes
        text = rx!(r"(?<![\d-])(?:\+?(1)[-.\s])?\(?(\d{3})\)?[-.\s](\d{3})[-.\s](\d{4})(?![\d-])").sub(&text, |m| self.phone(m));
        text = rx!(r"(?<![\w.,-])(\d{3})-(\d{4})(?![\w-])").sub(&text, |m| self.phone(m));
        text = rx!(r"(?<![\w.])([vV])?(\d+(?:\.\d+){2,})(?:-(\d+)(?![\w-]|\.\d))?(?![\w])").sub(&text, |m| {
            let v = m.g(1).filter(|s| !s.is_empty()).map(|v| format!("{} ", spelled(v))).unwrap_or_default();
            let parts = m.gs(2).split('.').map(version_part).collect::<Vec<_>>().join(" point ");
            let dash = m.g(3).filter(|s| !s.is_empty()).map(|d| format!(" dash {}", number_words(d))).unwrap_or_default();
            format!(" {v}{parts}{dash} ")
        });
        text = rx!(r"(?<![\w.])([vV])(\d+)\.(\d{2,3})(?!\d|\.\d|\w)")
            .sub(&text, |m| format!(" {} {} point {} ", spelled(m.gs(1)), cardinal(int(m.gs(2))), minor(m.gs(3))));
        text = rx!(
            i,
            format!(
                r"\b({VERSIONED})(\.?\s(?:(?:from|to)\s)?)(\d+)\.(\d{{2,3}})(?:(\s(?:to|or|and)\s)(\d+)\.(\d{{2,3}}))?(?!\d|\.\d|\s?%|\s(?:percent|million|billion)\b)"
            )
        )
        .sub(&text, |m| {
            let second = if m.g(5).is_some() {
                format!("{}{} point {}", m.gs(5), cardinal(int(m.gs(6))), minor(m.gs(7)))
            } else {
                String::new()
            };
            format!("{}{}{} point {}{second} ", m.gs(1), m.gs(2), cardinal(int(m.gs(3))), minor(m.gs(4)))
        });
        text = rx!(r"(?<![\w.,/-])(#)?([A-Za-z0-9]+(?:-[A-Za-z0-9]+)+)(?![\w-]|[.,]\d)").sub(&text, |m| self.code(m));
        text = rx!(r"(?<![\w.,/-])(\d+(?:/\d+){2,})(?![\w/]|[.,]\d)").sub(&text, |m| {
            let groups: Vec<&str> = m.gs(1).split('/').collect();
            format!(" {} ", Self::digit_groups(&groups, "zero"))
        });
        // dizione writes this with a conditional group, `(?(lead)(?: (?P<trail>...))?)`: the same as an alternation.
        text = rx!(format!(
            r"(?<![\w.,:/#+-])(?P<plus>\+)?(?:(?P<lead>[A-Z]{{2}}) (?P<core>{CORE})(?: (?P<trail>[A-Z]{{2}})(?![\w]))?|(?P<core2>{CORE}))(?![\w]|[.,:/-]\d|\s?[%°])"
        ))
        .sub(&text, |m| self.grouped(m));

        text = rx!(r"(?<![\w.])-(?=\d)").sub(&text, |_| " minus ".into());
        text = rx!(format!(r"(?:(?<=\d)\s?[xX×]|×)\s?(?=\d[\d.,]*\s?(?:{LENGTHS})\b)")).sub(&text, |_| " by ".into());
        text = rx!(r"(?<=\d)\s[xX×]\s(?=\d)").sub(&text, |_| " times ".into());
        text = rx!(r"(?<![\w/'])[A-Za-z0-9/]*[0-9/][A-Za-z0-9/]*(?![\w/'])").sub(&text, |m| self.chat_form(m));
        text = rx!(r"(?<![\w.])'?(\d{2}|\d{4})'?s\b").sub(&text, |m| {
            let g = m.gs(1);
            format!(" {} ", plural_of(&if pyre::len(g) == 4 { year(int(g)) } else { cardinal(int(g)) }))
        });
        text = rx!(r"(?<![\w.])(\d+)(?:st|nd|rd|th)\b").sub(&text, |m| format!(" {} ", ordinal(int(m.gs(1)))));
        text = rx!(r"(?<![\w.])(144|240|360|480|540|576|720|900|1080|1440|2160|4320)([pi])\b")
            .sub(&text, |m| format!(" {} {} ", pairs(int(m.gs(1))), spelled(m.gs(2))));
        text = rx!(r"(\d[\d,]*(?:\.\d+)?)\s?%").sub(&text, |m| format!(" {} percent ", number_words(m.gs(1))));
        text = self.t.area_plain.sub(&text, |m| self.square(m));
        text = self.t.unit.sub(&text, |m| self.measure(m));
        text = rx!(r"(?<![\d/.,-])(\d+) (\d)/(2|3|4|5|6|8|10|16)(?![\d/]|\.\d)").sub(&text, |m| self.mixed_fraction(m));
        text = rx!(r"(?<![\d/.])(\d+)/(\d+)(?![\d/]|\.\d)").sub(&text, |m| self.fraction(m));
        text = rx!(r"(?<![\w.,-])(\d{4})[-–](\d{4})(?![\w.,-])")
            .sub(&text, |m| format!(" {} to {} ", year(int(m.gs(1))), year(int(m.gs(2)))));
        text = rx!(r"(?<![\w.,-])(\d+)[-–](\d+)(?![\w-]|[.,]\d)")
            .sub(&text, |m| format!(" {} to {} ", number_words(m.gs(1)), number_words(m.gs(2))));
        text = rx!(r"(?<![\w.])(\d+)x\b").sub(&text, |m| format!(" {} times ", number_words(m.gs(1))));
        text = rx!(r"(?<=\d)\^(?=\d)").sub(&text, |_| " to the power of ".into());
        text
    }

    fn named_letters(text: &str) -> String {
        if !"=<>+*/^()≤≥≠≈×÷±".chars().any(|c| text.contains(c)) {
            return text.to_string();
        }
        let plain = without_angles(text);
        let mut places = HashSet::new();
        for pattern in letter_places() {
            for m in pattern.finditer(&plain) {
                places.insert(m.start_n("letter").unwrap());
            }
        }
        if places.is_empty() {
            return text.to_string();
        }
        let mut out = String::new();
        for (i, c) in text.char_indices() {
            if places.contains(&i) {
                out.push(SPELL);
                out.extend(c.to_uppercase());
                out.push(END);
            } else {
                out.push(c);
            }
        }
        out
    }

    fn mark_comparisons(text: &str) -> String {
        let plain = without_angles(text);
        rx!(r"(?P<gap>[ \t]?)(?P<sign>>=|<=|!=|[<>≥≤≠])(?P=gap)").sub(text, |m| {
            let before = &plain[..m.start()];
            let after = &plain[m.end()..];
            let value = rx!(r"(?:(?<![\w.,])\d[\d.,]*%?|(?<![\w'’.-])[A-Za-z]|[)\]])$").search(before);
            let sign = m.n("sign").unwrap();
            let sign_start = m.start_n("sign").unwrap();
            let same = char_at(&plain, sign_start) == sign.chars().next();
            let after_ok = rx!(r"(?:[-+]?[$£€¥₹]?\d[\d.,]*%?(?!\w)|\.\d|[A-Za-z](?![\w'’])|[(\[])").match_(after).is_some();
            let Some(value) = value.filter(|_| same && after_ok) else {
                return m.all().to_string();
            };
            let chars: Vec<char> = before.chars().collect();
            let mut start = before[..value.start()].chars().count() as isize;
            if value.all() == ")" || value.all() == "]" {
                let mut depth = 0i32;
                while start >= 0 {
                    let c = chars[start as usize];
                    depth += ((c == ')' || c == ']') as i32) - ((c == '(' || c == '[') as i32);
                    if depth == 0 {
                        break;
                    }
                    start -= 1;
                }
                while start > 0 && {
                    let c = chars[start as usize - 1];
                    pyre::c_isalnum(c) || c == '_' || c == '.'
                } {
                    start -= 1;
                }
            }
            let head: String = chars[..start.max(0) as usize].iter().collect();
            let word = rx!(r"[A-Za-z']+$").search(pyre::rstrip(&head)).map(|w| w.all().to_lowercase());
            if word.is_some_and(|w| !clause_openers().contains(w.as_str())) {
                return m.all().to_string();
            }
            format!("{IS}{}", m.all())
        })
    }

    fn compared(m: &M, words: &str) -> String {
        let marked = m.all().starts_with(IS) || m.string[..m.start()].trim_end_matches(' ').ends_with(IS);
        format!(" {}{words} ", if marked { "is " } else { "" })
    }

    fn joined_comparisons(text: &str) -> String {
        let plain = without_angles(text);
        rx!(format!(r"(?<=[\w){END}{IS}])[<>](?=[\w({SPELL}]|\.\d| minus \d)")).sub(text, |m| {
            let before = text[..m.start()].trim_end_matches(IS);
            let after = &text[m.end()..];
            if char_at(&plain, m.start()) != m.all().chars().next() {
                return m.all().to_string();
            }
            let figure = rx!(r"(?<![\w.,])\d[\d.,]*%?$").search(before).is_some()
                || rx!(r"(?:\d[\d.,]*%?(?!\w)|\.\d| minus \d)").match_(after).is_some();
            let letters = rx!(format!(r"(?:(?<![\w'’.])[A-Za-z]|[){END}])$")).search(before).is_some()
                && rx!(format!(r"(?:[A-Za-z](?![\w'’])|[({SPELL}])")).match_(after).is_some();
            if !(figure || letters) {
                return m.all().to_string();
            }
            Self::compared(m, if m.all() == "<" { "less than" } else { "greater than" })
        })
    }

    fn signs(&self, text: &str) -> String {
        let mut text = Self::named_letters(text);
        if text.contains('<') || text.contains('>') {
            text = Self::joined_comparisons(&text);
        }
        text = rx!(format!(r"(?<![\w-])({SIZES})-({SIZES})(?![\w-])"))
            .sub(&text, |m| format!(" {} to {} ", spelled(m.gs(1)), spelled(m.gs(2))));
        text = rx!(r"#(?=\d)").sub(&text, |_| " number ".into());
        text = rx!(r"#(?=[A-Z]{1,4}-?\d)").sub(&text, |_| " number ".into());
        text = rx!(r"#(?=[A-Za-z])").sub(&text, |_| " hashtag ".into());
        text = rx!(r"~(?=\s?\d)").sub(&text, |_| " about ".into());
        text = rx!(format!(r"(?<=[\w\s{END}{IS}])(>=|<=|!=)(?=[\w\s{SPELL}])")).sub(&text, |m| {
            let words = match m.gs(1) {
                ">=" => "greater than or equal to",
                "<=" => "less than or equal to",
                _ => "not equal to",
            };
            Self::compared(m, words)
        });
        text = rx!(r"(?<=\d) Ti\b").sub(&text, |_| format!(" {}", spelled("TI")));
        text = rx!(r"\b([Rr])ef\.\s?[Nn]o\.(?=\s)").sub_t(&text, r"\1eference number");
        if text.contains('.') {
            text = rx!(r"(?<![\w.])(?:[A-Z]\.(?: [A-Z]\.)+|[Ee]\. g\.|[Ii]\. e\.)").sub(&text, |m| {
                let g = m.all();
                if g == "E. G." || g == "I. E." {
                    format!(" {} ", spelled(&format!("{}{}", &g[0..1], &g[3..4])))
                } else {
                    g.replace(' ', "")
                }
            });
            let snapshot = text.clone();
            text = self.t.dotted_form.sub(&snapshot, |m| self.dotted(m, &snapshot));
        }
        let snapshot = text.clone();
        text = rx!(r"\b([A-Za-z]{1,6})\b(\.?)(?![A-Za-z'’])").sub(&snapshot, |m| self.abbreviation(m, &snapshot));
        if rx!(r"\d").is_match(&text) {
            text = rx!(r"(?<![\w'.])(?=[A-Za-z0-9]*[A-Za-z])(?=[A-Za-z0-9]*\d)[A-Za-z0-9]+(?![\w']|\.\d)").sub(&text, |m| self.glued(m));
            let snapshot = text.clone();
            text = rx!(r"(?<![\w.])\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?|(?<![\w.])\.\d+")
                .sub(&snapshot, |m| self.number(m, &snapshot));
        }
        text = rx!(r"(?<=[A-Za-z])/(?=[A-Za-z])").sub(&text, |_| " ".into());
        text = rx!(format!(r"(?<=\d){IS}?\s?[<>]\s?(?=\d)|(?<=\s)[<>](?=\s)"))
            .sub(&text, |m| Self::compared(m, if m.all().contains('<') { "less than" } else { "greater than" }));
        text = rx!(r"[&@+=%×÷±©§°≥≤≠≈∞√π]").sub(&text, |m| {
            let c = m.all();
            if "≥≤≠".contains(c) { Self::compared(m, symbol_word(c)) } else { format!(" {} ", symbol_word(c)) }
        });
        text.replace(IS, "")
    }

    // -- words --

    fn unstretched(word: &str) -> Vec<String> {
        if !rx!(r"(.)\1\1").is_match(word) {
            return Vec::new();
        }
        let cut = |inside: bool, fin: bool| {
            rx!(r"(.)\1{2,}").sub(word, |m| {
                let is_final = m.end() == word.len();
                let double = if is_final { fin } else { inside };
                m.gs(1).repeat(if double { 2 } else { 1 })
            })
        };
        let liked = rx!(r"([lfsz])\1\1+$").is_match(word)
            || word.chars().all(|c| c == 'm') && !word.is_empty()
            || rx!(r"(.)\1{2,}$").sub_t(word, r"\1") == "to";
        let ordered = [cut(true, liked), cut(true, !liked), cut(false, liked), cut(false, !liked)];
        let mut out: Vec<String> = Vec::new();
        for o in ordered {
            if !out.contains(&o) {
                out.push(o);
            }
        }
        out
    }

    fn reply(out: &[Token], place: usize) -> bool {
        let mut marks: Vec<String> = Vec::new();
        for step in [-1isize, 1] {
            let mut index = place as isize + step;
            while index >= 0 && (index as usize) < out.len() && out[index as usize].mark().is_some_and(|m| "\"'()".contains(m)) {
                index += step;
            }
            if index < 0 || index as usize >= out.len() {
                marks.push(String::new());
            } else if out[index as usize].word().is_some() {
                return false;
            } else {
                let mark = out[index as usize].mark().unwrap();
                marks.push(mark.to_string());
                let b = index + step;
                let beyond = if b >= 0 && (b as usize) < out.len() { out[b as usize].word() } else { None };
                if mark == "," && beyond.is_some_and(|w| pyre::len(&w.text) == 1 && w.text != "i") {
                    return false;
                }
            }
        }
        marks != [",", ","]
    }

    fn word_tokens(&self, raw: &str, all_caps: bool, previous: &str, following: bool, fresh: bool) -> Vec<Word> {
        let lower = raw.to_lowercase();
        if let Some(chat) = self.t.chat.get(&lower) {
            if (pyre::len(raw) > 1 || pyre::islower(raw)) && !self.t.replies.contains(&lower) {
                return chat_words(chat, raw);
            }
        }
        if let Some(number) = roman(raw) {
            if previous.chars().next().is_some_and(pyre::c_isupper) {
                let words = if cardinal_after().contains(previous.to_lowercase().as_str()) {
                    cardinal(number)
                } else {
                    format!("the {}", ordinal(number))
                };
                return words.split(' ').map(|x| w(x, raw)).collect();
            }
        }
        let letters = raw.replace('\'', "");
        let possessive = lower.ends_with("'s");
        let stem = if possessive { &raw[..raw.len() - 2] } else { raw };
        let stem_len = pyre::len(stem);
        if stem_len == 1 && pyre::isalpha(stem) {
            let label = stem == "A"
                && !previous.is_empty()
                && (!following || (!fresh && labelled().contains(previous.to_lowercase().as_str())));
            if ["a", "A", "I", "i"].contains(&stem) && !possessive && !label {
                return vec![w(&stem.to_lowercase(), raw)];
            }
            return vec![Word { text: lower, raw: raw.into(), spell: true, ..Default::default() }];
        }
        let distinct: HashSet<char> = lower.chars().collect();
        if stem_len > 2 && "SsZz".contains(stem.chars().next().unwrap()) && distinct.len() == 1 && (all_caps || !pyre::isupper(stem)) {
            let ph = crate::symbols::phone(&lower[..1]).unwrap();
            return vec![Word { text: lower.clone(), raw: raw.into(), phones: Some(vec![(ph, 0)]), ..Default::default() }];
        }
        if pyre::isupper(stem) && pyre::isalpha(stem) && (2..=5).contains(&stem_len) && (!all_caps || !self.is_word(&stem.to_lowercase()))
        {
            if initialisms().contains(stem) || !self.is_word(&stem.to_lowercase()) {
                return vec![Word { text: lower, raw: raw.into(), spell: true, ..Default::default() }];
            }
        }
        if !pyre::isalpha(&letters) {
            return Vec::new();
        }
        let capitals = rx!(r"[a-z]+(?:[A-Z][a-z]+)*[A-Z]{2,}(?:'s)?").fullmatch(raw).is_some();
        if capitals || !self.is_word(&lower) {
            if !capitals {
                for candidate in Self::unstretched(&lower) {
                    if self.is_word(&candidate) {
                        return vec![w(&candidate, raw)];
                    }
                }
            }
            let parts = rx!(r"[A-Z]?[a-z']+|[A-Z]+(?![a-z])").findall(raw);
            if parts.len() > 1 && parts.concat() == raw {
                let mut out: Vec<Word> = Vec::new();
                for part in parts {
                    let named = capitals && pyre::isupper(part) && initialisms().contains(part);
                    let spell = pyre::len(part) == 1 || named || (pyre::isupper(part) && !self.is_word(&part.to_lowercase()));
                    if capitals && out.last().is_some_and(|l| l.spell) && (spell || part == "'s") {
                        let last = out.last_mut().unwrap();
                        last.text += &part.to_lowercase();
                        last.raw += part;
                    } else {
                        let attach = !out.is_empty();
                        out.push(Word { text: part.to_lowercase(), attach, raw: part.into(), spell, phones: None });
                    }
                }
                return out;
            }
        }
        vec![w(&lower, raw)]
    }

    /// Words and punctuation marks of `text`.
    pub fn tokens(&mut self, text: &str) -> Vec<Token> {
        let text = self.rewrite(text);
        let runs = rx!(r"[A-Za-z]{2,}").findall(&text);
        let upper_runs = runs.iter().filter(|r| pyre::isupper(r)).count();
        let all_caps = runs.len() >= 3 && upper_runs as f64 >= 0.6 * runs.len() as f64;
        let mut out: Vec<Token> = Vec::new();
        let pattern = rx!(format!(r"{SPELL}([A-Za-z ]+){END}|{INLINE}(.){INLINE}|('?[A-Za-z]+(?:'[A-Za-z]+)*)|(\d+)|(-)|(\S)"));
        let mut attach = false;
        let mut previous = String::new();
        let mut fresh = true;
        let mut short = Vec::new();
        let mut replies = Vec::new();
        for m in pattern.finditer(&text) {
            let before = char_before(&text, m.start()).unwrap_or(' ');
            let rest = &text[m.end()..];
            let next = char_at(&text, m.end());
            if m.g(5).is_some() {
                attach = matches!(out.last(), Some(Token::Word(_))) && !pyre::isspace_char(before) && next.is_some_and(pyre::c_isalnum);
                continue;
            }
            let mut new: Vec<Word> = Vec::new();
            if let Some(spell) = m.g(1) {
                new = vec![Word { text: spell.replace(' ', "").to_lowercase(), raw: spell.into(), spell: true, ..Default::default() }];
            } else if let Some(code) = m.g(2) {
                let k = (code.chars().next().unwrap() as u32).checked_sub(INLINE_BASE);
                if let Some((written, phones)) = k.and_then(|k| self.inline.get(k as usize)).cloned() {
                    new = vec![Word { text: written.to_lowercase(), raw: written, phones: Some(phones), ..Default::default() }];
                }
            } else if let Some(found) = m.g(3) {
                let mut word = found.to_string();
                if word.starts_with('\'') && !leading_apostrophe().contains(word.to_lowercase().as_str()) {
                    out.push(Token::Punct("'".into()));
                    word = word[1..].to_string();
                } else if word.starts_with('\'') && !self.is_word(&word.to_lowercase()) {
                    word = word[1..].to_string();
                }
                let following = rx!(r"\s*[A-Za-z]").match_(rest).is_some();
                new = self.word_tokens(&word, all_caps, &previous, following, fresh);
                previous = word.clone();
                if ["Ss", "ss", "Zz", "zz"].contains(&word.as_str())
                    && new.len() == 1
                    && (pyre::isspace_char(before) || "\"'(".contains(before))
                    && rx!(r#"[.!?…,;:—"')]*(?:\s|$)"#).match_(rest).is_some()
                {
                    short.push(out.len());
                }
                if self.t.replies.contains(&word.to_lowercase())
                    && (pyre::len(&word) > 1 || pyre::islower(&word))
                    && new.len() == 1
                    && (pyre::isspace_char(before) || "\"'".contains(before))
                    && rx!(r#"(?:[.?…,;—]*["']?(?:\s|$)|!+["']?\s*$)"#).match_(rest).is_some()
                {
                    replies.push(out.len());
                }
            } else if let Some(number) = m.g(4) {
                new = number_words(number).split_whitespace().map(|part| w(part, number)).collect();
            } else if let Some(other) = m.g(6) {
                if PUNCTUATION.contains(&other) {
                    let inner_apostrophe = other == "'" && pyre::c_isalnum(before) && next.is_some_and(pyre::c_isalnum);
                    if !inner_apostrophe {
                        out.push(Token::Punct(other.into()));
                    }
                    if ".!?;…".contains(other) {
                        fresh = true;
                    }
                    attach = false;
                    continue;
                }
            }
            if !new.is_empty() && attach {
                new[0].attach = true;
            }
            let empty = new.is_empty();
            out.extend(new.into_iter().map(Token::Word));
            attach = false;
            fresh = fresh && empty;
        }
        for place in short {
            let (mut first, mut last) = (place, place + 1);
            let joins = |t: &Token| t.word().is_some() || t.mark().is_some_and(|m| "\"'()".contains(m));
            while first > 0 && joins(&out[first - 1]) {
                first -= 1;
            }
            while last < out.len() && joins(&out[last]) {
                last += 1;
            }
            let all = out[first..last].iter().filter_map(Token::word).all(|t| {
                sounds().contains(t.text.as_str()) || (t.phones.is_some() && t.text.chars().collect::<HashSet<_>>().len() == 1)
            });
            if all {
                if let Token::Word(word) = &mut out[place] {
                    let ph = crate::symbols::phone(&word.text[..1]).unwrap();
                    word.phones = Some(vec![(ph, 0)]);
                }
            }
        }
        for &place in replies.iter().rev() {
            if Self::reply(&out, place) {
                let raw = out[place].word().unwrap().raw.clone();
                let words = chat_words(&self.t.chat[&raw.to_lowercase()], &raw);
                out.splice(place..place + 1, words.into_iter().map(Token::Word));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_python_does() {
        assert_eq!(cardinal(1234), "one thousand two hundred thirty four");
        assert_eq!(ordinal(21), "twenty first");
        assert_eq!(year(2005), "two thousand five");
        assert_eq!(pairs(4070), "forty seventy");
        assert_eq!(number_words("3.14"), "three point one four");
        assert_eq!(number_words("007"), "zero zero seven");
    }
}
