//! Which letters spell which phoneme (dizione `align.py`, run time only).
//!
//! Every phoneme gets a run of zero to four letters of the word, in order, so that the runs spell the word. The
//! table of log P(letters | phoneme) is `align.json` of the built data.

use std::collections::HashMap;

use crate::convert::base;

const MAX_RUN: usize = 4;
const UNSEEN: f64 = -14.0;

pub struct Aligner {
    table: HashMap<String, HashMap<String, f64>>,
}

impl Aligner {
    pub fn from_json(text: &str) -> Result<Aligner, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("align.json: {e}"))?;
        let object = value.as_object().ok_or("align.json: not an object")?;
        let mut table = HashMap::new();
        for (phone, runs) in object {
            let runs = runs.as_object().ok_or("align.json: runs not an object")?;
            let runs = runs.iter().map(|(run, v)| (run.clone(), v.as_f64().unwrap_or(f64::NAN))).collect();
            table.insert(phone.clone(), runs);
        }
        Ok(Aligner { table })
    }

    fn score(&self, phone: &str, run: &str) -> f64 {
        match self.table.get(phone).and_then(|known| known.get(run)) {
            Some(&value) => value,
            None => UNSEEN - 2.0 * (run.chars().count() as f64 - 1.0).abs(),
        }
    }

    /// One run of letters per phoneme of `word`, or `None` when the word is too long for the pronunciation.
    pub fn align<S: AsRef<str>>(&self, word: &str, phones: &[S]) -> Option<Vec<String>> {
        let phones: Vec<&str> = phones.iter().map(|p| base(p.as_ref())).collect();
        let letters: Vec<char> = word.chars().collect();
        let (n, count) = (letters.len(), phones.len());
        if count == 0 || n > MAX_RUN * count {
            return None;
        }
        let low = -1e18;
        let mut best = vec![vec![low; n + 1]; count + 1];
        let mut step = vec![vec![0usize; n + 1]; count + 1];
        best[0][0] = 0.0;
        let slice = |a: usize, b: usize| letters[a..b].iter().collect::<String>();
        for j in 1..=count {
            let phone = phones[j - 1];
            for i in 0..=n {
                for k in 0..=MAX_RUN.min(i) {
                    if best[j - 1][i - k] > low {
                        let value = best[j - 1][i - k] + self.score(phone, &slice(i - k, i));
                        if value > best[j][i] {
                            best[j][i] = value;
                            step[j][i] = k;
                        }
                    }
                }
            }
        }
        if best[count][n] <= low {
            return None;
        }
        let mut runs = Vec::with_capacity(count);
        let mut i = n;
        for j in (1..=count).rev() {
            let k = step[j][i];
            runs.push(slice(i - k, i));
            i -= k;
        }
        runs.reverse();
        Some(runs)
    }
}
