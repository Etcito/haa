//! French text → IPA words: a small exception list (function words, common irregulars), then spelling rules read
//! left to right, longest match first (digraphs and trigraphs, nasal vowels before a consonant or the word's end,
//! `ill` after a vowel, `s` between vowels, soft `c` / `g`), silent final consonants except c, f, l, r (`-er` reads
//! /e/), the mute final `e`, `es` and the verb ending `ent` after a vowel-final stem. Elided words (`j'`, `l'`, `c'`,
//! `d'`, `n'`, `qu'`, `s'`, `m'`, `t'`) join the next word. No liaison and no schwa deletion: each word is read
//! alone, as careful speech says it.

use super::Segment;
use super::ipa::tokenize_ipa;

/// Function words and irregular spellings the rules would misread.
const EXCEPTIONS: &[(&str, &str)] = &[
    ("et", "e"), ("est", "ɛ"), ("es", "ɛ"), ("les", "le"), ("des", "de"), ("mes", "me"), ("tes", "te"), ("ses", "se"),
    ("ces", "se"), ("un", "œ̃"), ("une", "yn"), ("à", "a"), ("au", "o"), ("aux", "o"), ("en", "ɑ̃"), ("dans", "dɑ̃"),
    ("pas", "pa"), ("vous", "vu"), ("nous", "nu"), ("mon", "mɔ̃"), ("ton", "tɔ̃"), ("son", "sɔ̃"), ("ce", "sə"),
    ("je", "ʒə"), ("me", "mə"), ("te", "tə"), ("se", "sə"), ("le", "lə"), ("de", "də"), ("ne", "nə"), ("que", "kə"),
    ("qui", "ki"), ("il", "il"), ("ils", "il"), ("elle", "ɛl"), ("elles", "ɛl"), ("femme", "fam"), ("monsieur", "məsjø"),
    ("oui", "wi"), ("eux", "ø"), ("très", "tʁɛ"), ("avec", "avɛk"), ("plus", "ply"), ("tout", "tu"), ("tous", "tus"),
    ("fils", "fis"), ("sept", "sɛt"), ("huit", "ɥit"), ("dix", "dis"), ("six", "sis"), ("ville", "vil"), ("mille", "mil"),
    ("tranquille", "tʁɑ̃kil"), ("second", "səɡɔ̃"), ("seconde", "səɡɔ̃d"), ("tom", "tɔm"), ("ai", "e"), ("as", "a"),
    ("a", "a"), ("ont", "ɔ̃"), ("sont", "sɔ̃"), ("font", "fɔ̃"), ("vont", "vɔ̃"), ("sur", "syʁ"), ("chez", "ʃe"),
    ("y", "i"), ("o", "o"), ("ou", "u"), ("où", "u"), ("bien", "bjɛ̃"), ("rien", "ʁjɛ̃"), ("hier", "jɛʁ"), ("fer", "fɛʁ"),
    ("mer", "mɛʁ"), ("cher", "ʃɛʁ"), ("hiver", "ivɛʁ"), ("enfer", "ɑ̃fɛʁ"), ("août", "ut"), ("pays", "pei"),
    ("aujourd'hui", "oʒuʁdɥi"), ("faisons", "fəzɔ̃"), ("faisait", "fəzɛ"), ("parce", "paʁs"), ("donc", "dɔ̃k"),
    ("second", "səɡɔ̃"), ("examen", "ɛɡzamɛ̃"), ("club", "klœb"), ("sous", "su"), ("loin", "lwɛ̃"), ("moins", "mwɛ̃"),
];

const VOWELS: &str = "aàâeéèêëiîïoôuùûüyœæ";

fn is_vowel(c: char) -> bool {
    VOWELS.contains(c)
}

/// One word (lower case, no apostrophe inside) as IPA.
pub fn word_ipa(word: &str) -> String {
    let w: String = word.to_lowercase();
    if let Some((_, ipa)) = EXCEPTIONS.iter().find(|(k, _)| *k == w) {
        return ipa.to_string();
    }
    let mut c: Vec<char> = w.chars().filter(|ch| ch.is_alphabetic()).collect();
    if c.is_empty() {
        return String::new();
    }
    // silent endings: verb plural `-ent` after a consonant + vowel stem is too uncertain; `-ent` of verbs in
    // `-aient`, `-oient`, and a final `-es` / `-e` (after at least one other vowel) are mute
    let s: String = c.iter().collect();
    let vowels_in = |x: &[char]| x.iter().filter(|ch| is_vowel(**ch)).count();
    if s.ends_with("aient") || s.ends_with("oient") {
        c.truncate(c.len() - 3);
    } else if s.ends_with("es") && c.len() > 3 && vowels_in(&c[..c.len() - 2]) > 0 {
        c.truncate(c.len() - 1);
    }
    let n = c.len();
    let at = |i: usize| -> char { if i < n { c[i] } else { '#' } };
    let starts = |i: usize, pat: &str| -> bool { pat.chars().enumerate().all(|(k, p)| at(i + k) == p) };
    // the word's last pronounced position: a final mute e
    let mute_e = n > 2 && c[n - 1] == 'e' && vowels_in(&c[..n - 1]) > 0;
    let end = if mute_e { n - 1 } else { n };
    let mut out = String::new();
    let mut i = 0;
    while i < end {
        let ch = c[i];
        let next = at(i + 1);
        let prev = if i > 0 { c[i - 1] } else { '#' };
        // a nasal vowel: vowel letters + n/m not followed by a vowel or another n/m
        let nasal_after = |k: usize| -> bool {
            let m = at(k);
            (m == 'n' || m == 'm') && !is_vowel(at(k + 1)) && at(k + 1) != 'n' && at(k + 1) != 'm' && !(k + 1 >= end && mute_e && at(k + 1) == 'e')
        };
        // final consonant: silent unless c, f, l, r (and -er reads /e/)
        let is_final = i + 1 >= end;
        macro_rules! emit {
            ($s:expr, $k:expr) => {{
                out.push_str($s);
                i += $k;
                continue;
            }};
        }
        // trigraphs and vowel digraphs
        if starts(i, "eau") {
            emit!("o", 3);
        }
        if starts(i, "ien") && nasal_after(i + 2) {
            emit!("jɛ̃", 3);
        }
        if starts(i, "oin") && nasal_after(i + 2) {
            emit!("wɛ̃", 3);
        }
        if (starts(i, "ain") || starts(i, "aim") || starts(i, "ein")) && nasal_after(i + 2) {
            emit!("ɛ̃", 3);
        }
        if starts(i, "aill") {
            emit!("aj", 4);
        }
        if starts(i, "eill") {
            emit!("ɛj", 4);
        }
        if starts(i, "ouill") {
            emit!("uj", 5);
        }
        if starts(i, "ail") && i + 3 >= end {
            emit!("aj", 3);
        }
        if starts(i, "eil") && i + 3 >= end {
            emit!("ɛj", 3);
        }
        if starts(i, "euil") || starts(i, "œil") {
            emit!("œj", 4);
        }
        if starts(i, "ill") && !is_vowel(prev) {
            emit!("ij", 3);
        }
        if starts(i, "tion") && i > 0 {
            emit!("sjɔ̃", 4);
        }
        if starts(i, "œu") || starts(i, "eu") {
            // closed syllable before a pronounced consonant: œ
            let k = i + 2;
            let closed = at(k) == 'r' || at(k) == 'l' || at(k) == 'f';
            emit!(if closed { "œ" } else { "ø" }, 2);
        }
        if starts(i, "ou") || starts(i, "où") || starts(i, "oû") {
            if is_vowel(at(i + 2)) {
                emit!("w", 2);
            }
            emit!("u", 2);
        }
        if starts(i, "oi") || starts(i, "oî") {
            emit!("wa", 2);
        }
        if starts(i, "oy") {
            emit!("waj", 2);
        }
        if starts(i, "ai") || starts(i, "aî") || starts(i, "ei") {
            emit!("ɛ", 2);
        }
        if starts(i, "ay") {
            emit!("ɛj", 2);
        }
        if starts(i, "au") {
            emit!("o", 2);
        }
        if starts(i, "ui") && prev != 'q' && prev != 'g' {
            emit!("ɥi", 2);
        }
        // nasal vowels
        if (ch == 'a' || ch == 'e') && nasal_after(i + 1) {
            emit!("ɑ̃", 2);
        }
        if (ch == 'i' || ch == 'y') && nasal_after(i + 1) {
            emit!("ɛ̃", 2);
        }
        if ch == 'o' && nasal_after(i + 1) {
            emit!("ɔ̃", 2);
        }
        if ch == 'u' && nasal_after(i + 1) {
            emit!("œ̃", 2);
        }
        // consonant digraphs
        if starts(i, "ch") {
            emit!("ʃ", 2);
        }
        if starts(i, "ph") {
            emit!("f", 2);
        }
        if starts(i, "th") {
            emit!("t", 2);
        }
        if starts(i, "gn") {
            emit!("ɲ", 2);
        }
        if starts(i, "qu") {
            emit!("k", 2);
        }
        if starts(i, "gu") && matches!(at(i + 2), 'e' | 'é' | 'è' | 'ê' | 'i' | 'y') {
            emit!("ɡ", 2);
        }
        if starts(i, "ge") && matches!(at(i + 2), 'a' | 'o' | 'u') {
            emit!("ʒ", 2);
        }
        if starts(i, "sc") && matches!(at(i + 2), 'e' | 'i' | 'y') {
            emit!("s", 2);
        }
        if ch == 'e' && (starts(i, "er") || starts(i, "ez")) && i + 2 >= end && i > 0 {
            emit!("e", 2);
        }
        if ch == 'e' && starts(i, "et") && i + 2 >= end && i > 0 {
            emit!("ɛ", 2);
        }
        // doubled consonants read once (except cc before e/i, and ss which stays /s/)
        if !is_vowel(ch) && next == ch && ch != 'c' {
            i += 1;
            continue;
        }
        let ipa: &str = match ch {
            'a' | 'à' | 'â' => "a",
            'é' => "e",
            'è' | 'ê' | 'ë' => "ɛ",
            'e' => {
                // before two consonants (not a consonant + r/l onset) or a final pronounced consonant: ɛ; otherwise ə
                let c1 = at(i + 1);
                let c2 = at(i + 2);
                let cons = |x: char| x != '#' && !is_vowel(x);
                let closed = (cons(c1) && cons(c2) && !(matches!(c2, 'r' | 'l') && !matches!(c1, 'r' | 'l')) && c1 != 'h')
                    || (cons(c1) && i + 2 >= end && matches!(c1, 'c' | 'f' | 'l' | 'r'))
                    || (c1 == 'x');
                if i + 1 >= end && end == n { "ə" } else if closed { "ɛ" } else { "ə" }
            }
            'i' | 'î' | 'ï' => {
                if is_vowel(next) && i > 0 { "j" } else { "i" }
            }
            'y' => {
                if is_vowel(next) { "j" } else { "i" }
            }
            'o' | 'ô' => {
                // closed syllable: ɔ, open: o
                let c1 = at(i + 1);
                let closed = c1 != '#' && !is_vowel(c1) && (i + 2 >= end || !is_vowel(at(i + 2))) && !(i + 2 >= end && !matches!(c1, 'c' | 'f' | 'l' | 'r' | 'n' | 'm'));
                if ch == 'ô' { "o" } else if closed { "ɔ" } else { "o" }
            }
            'u' | 'û' | 'ù' | 'ü' => {
                if is_vowel(next) && next != 'e' { "ɥ" } else { "y" }
            }
            'œ' | 'æ' => "e",
            'b' => "b",
            'c' => {
                if matches!(next, 'e' | 'é' | 'è' | 'ê' | 'i' | 'î' | 'y') { "s" } else if starts(i, "cc") && matches!(at(i + 2), 'e' | 'i') { "k" } else { "k" }
            }
            'ç' => "s",
            'd' => "d",
            'f' => "f",
            'g' => {
                if matches!(next, 'e' | 'é' | 'è' | 'ê' | 'i' | 'î' | 'y') { "ʒ" } else { "ɡ" }
            }
            'h' => "",
            'j' => "ʒ",
            'k' => "k",
            'l' => "l",
            'm' => "m",
            'n' => "n",
            'p' => "p",
            'q' => "k",
            'r' => "ʁ",
            's' => {
                if is_vowel(prev) && is_vowel(next) { "z" } else { "s" }
            }
            't' => "t",
            'v' => "v",
            'w' => "w",
            'x' => {
                if i == 1 && prev == 'e' && is_vowel(next) { "ɡz" } else { "ks" }
            }
            'z' => "z",
            _ => "",
        };
        // silent final consonants (d g p s t x z; b and m stay)
        let silent_final = is_final && end == n && matches!(ch, 'd' | 'g' | 'p' | 's' | 't' | 'x' | 'z');
        if !silent_final {
            out.push_str(ipa);
        }
        i += 1;
    }
    out
}

/// A text as words: (the word as written, its IPA), elided words joined to the next one.
pub fn words(text: &str) -> Vec<(String, String)> {
    let norm = text.replace(['’', '‘'], "'").to_lowercase();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut carry = String::new();
    let mut carry_w = String::new();
    for raw in norm.split(|ch: char| ch.is_whitespace() || ch == '-') {
        let t: String = raw.chars().filter(|ch| ch.is_alphabetic() || *ch == '\'').collect();
        if t.is_empty() {
            continue;
        }
        let mut rest = t.as_str();
        // elisions: c' d' j' l' m' n' s' t' qu'
        while let Some(p) = rest.find('\'') {
            let head = &rest[..p];
            let ipa = match head {
                "c" | "s" => "s",
                "d" => "d",
                "j" => "ʒ",
                "l" => "l",
                "m" => "m",
                "n" => "n",
                "t" => "t",
                "qu" => "k",
                "jusqu" => "ʒysk",
                "lorsqu" => "lɔʁsk",
                "puisqu" => "pɥisk",
                _ => "",
            };
            if head == "aujourd" {
                break;
            }
            carry.push_str(ipa);
            carry_w.push_str(&rest[..=p]);
            rest = &rest[p + 1..];
        }
        if rest.is_empty() {
            continue;
        }
        let ipa = if t == "aujourd'hui" { word_ipa("aujourd'hui") } else { word_ipa(rest) };
        out.push((format!("{carry_w}{rest}"), format!("{carry}{ipa}")));
        carry.clear();
        carry_w.clear();
    }
    out
}

/// A text as the planner's segments, each word's last segment marking its end.
pub fn to_segments(text: &str) -> (Vec<Segment>, Vec<String>) {
    let mut segs = Vec::new();
    let mut said = Vec::new();
    for (w, ipa) in words(text) {
        let mut ws = tokenize_ipa(&ipa);
        ws.retain(|s| s.ipa != "_");
        if let Some(last) = ws.last_mut() {
            last.word_end = Some(true);
            segs.append(&mut ws);
            said.push(w);
        }
    }
    (segs, said)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(t: &str) -> String {
        words(t).into_iter().map(|(_, i)| i).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn reads_the_sample_sentences() {
        assert_eq!(line("Tom vit et travaille à Boston."), "tɔm vi e tʁavaj a bɔstɔ̃");
        assert_eq!(line("Je ne vendrai pas mon âme."), "ʒə nə vɑ̃dʁɛ pa mɔ̃ am");
        assert_eq!(line("J'ai l'intention de l'écouter ce soir."), "ʒe lɛ̃tɑ̃sjɔ̃ də lekute sə swaʁ");
        assert_eq!(line("C'est mon travail de vous convaincre."), "sɛ mɔ̃ tʁavaj də vu kɔ̃vɛ̃kʁ");
    }
}
