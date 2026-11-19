//! Loading the built data at run time. Nothing of it is bundled: the host fetches it from dizione's official
//! release or the tiny-voca Hugging Face repo and passes the bytes in.
//!
//! Two sources are read:
//! * the `phonemizer/` folder of the tiny-voca repo (dizione's `built/` directory): `lexicon.tsv`, `align.json`,
//!   `g2p.json`, `g2p.npz`, `homographs.json` (5.4 MB);
//! * tiny-voca's one-file GGUF for the browser (`gguf/tiny-voca-js.gguf`, made by `js/pack.mjs`): the same files
//!   embedded under `phonemizer/` (the dictionary packed as `lexicon.packed.gz`), the model as `g2p/*` tensors and
//!   the hand-written tables, which then replace the built-in copies.

use std::collections::HashMap;

use crate::g2p::{self, Tensor};
use crate::symbols::{CONSONANTS, VOWELS};

/// Everything the phonemizer reads besides the hand-written tables.
pub struct Data {
    pub lexicon: HashMap<String, String>,
    pub align_json: String,
    pub g2p_json: Option<String>,
    pub g2p_tensors: HashMap<String, Tensor>,
    pub homographs_json: Option<String>,
    /// Tables found in the source (`abbreviations.txt`...), replacing the built-in ones.
    pub tables: HashMap<String, String>,
}

fn text(bytes: &[u8], name: &str) -> Result<String, String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{name}: not UTF-8"))
}

/// `lexicon.tsv`: one word, a tab and its phones per line.
pub fn lexicon_tsv(text: &str) -> HashMap<String, String> {
    text.split('\n')
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once('\t'))
        .map(|(w, p)| (w.to_string(), p.trim_end_matches('\r').to_string()))
        .collect()
}

/// `lexicon.packed.gz` of the JS GGUF (see `js/dizione/lexicon.js`): words then pronunciations, each stored as the
/// number of bytes shared with the previous one and the rest up to a 0; a pronunciation byte is a phone and its stress.
pub fn lexicon_packed(gz: &[u8]) -> Result<HashMap<String, String>, String> {
    let bytes = gunzip(gz)?;
    let count = u32::from_le_bytes(bytes.get(..4).ok_or("lexicon: empty")?.try_into().unwrap()) as usize;
    let phones: Vec<&str> = CONSONANTS.iter().chain(VOWELS.iter()).copied().collect();
    let marks = ["", "ˈ", "ˌ"];
    let mut at = 4;
    let mut item: Vec<u8> = Vec::with_capacity(256);
    let mut next = |at: &mut usize| -> Result<Vec<u8>, String> {
        let same = *bytes.get(*at).ok_or("lexicon: truncated")? as usize;
        *at += 1;
        let end = bytes[*at..].iter().position(|&b| b == 0).ok_or("lexicon: truncated")? + *at;
        item.truncate(same);
        item.extend_from_slice(&bytes[*at..end]);
        *at = end + 1;
        Ok(item.clone())
    };
    let mut words = Vec::with_capacity(count);
    for _ in 0..count {
        words.push(String::from_utf8(next(&mut at)?).map_err(|_| "lexicon: word not UTF-8")?);
    }
    let mut lexicon = HashMap::with_capacity(count);
    for word in words {
        let units: Vec<String> = next(&mut at)?
            .iter()
            .map(|&b| {
                let k = b as usize - 1;
                format!("{}{}", marks[(k / phones.len()).min(2)], phones[k % phones.len()])
            })
            .collect();
        lexicon.insert(word, units.join(" "));
    }
    Ok(lexicon)
}

fn gunzip(gz: &[u8]) -> Result<Vec<u8>, String> {
    if gz.get(..3) != Some(&[0x1f, 0x8b, 8]) {
        return Err("gzip: bad header".into());
    }
    let flags = gz[3];
    let mut at = 10;
    if flags & 4 != 0 {
        let len = u16::from_le_bytes([gz[at], gz[at + 1]]) as usize;
        at += 2 + len;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            at += gz[at..].iter().position(|&b| b == 0).ok_or("gzip: header")? + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    miniz_oxide::inflate::decompress_to_vec(&gz[at..]).map_err(|e| format!("gzip: {e:?}"))
}

impl Data {
    /// From the files of the `phonemizer/` folder, by file name (`lexicon.tsv`, `g2p.npz`...). Files with other
    /// names are ignored, except the hand-written tables (`*.txt`), which replace the built-in copies.
    pub fn from_files(files: &HashMap<String, Vec<u8>>) -> Result<Data, String> {
        let get = |n: &str| files.get(n).or_else(|| files.get(&format!("phonemizer/{n}")));
        let lexicon = if let Some(b) = get("lexicon.tsv") {
            lexicon_tsv(&text(b, "lexicon.tsv")?)
        } else if let Some(b) = get("lexicon.packed.gz") {
            lexicon_packed(b)?
        } else {
            return Err("no lexicon.tsv: pass the phonemizer files of the tiny-voca repo".into());
        };
        let align_json = text(get("align.json").ok_or("no align.json")?, "align.json")?;
        let g2p_json = get("g2p.json").map(|b| text(b, "g2p.json")).transpose()?;
        let g2p_tensors = match get("g2p.npz") {
            Some(b) => g2p::npz(b)?,
            None => HashMap::new(),
        };
        let homographs_json = get("homographs.json").map(|b| text(b, "homographs.json")).transpose()?;
        let mut tables = HashMap::new();
        for (name, bytes) in files {
            let base = name.rsplit('/').next().unwrap();
            if base.ends_with(".txt") {
                tables.insert(base.to_string(), text(bytes, base)?);
            }
        }
        Ok(Data { lexicon, align_json, g2p_json, g2p_tensors, homographs_json, tables })
    }

    /// From tiny-voca's one-file GGUF (`tiny-voca-js.gguf`, or any GGUF made by its `pack.mjs`).
    pub fn from_gguf(bytes: &[u8]) -> Result<Data, String> {
        let gguf = Gguf::read(bytes)?;
        let mut data = Data::from_files(&gguf.files)?;
        data.g2p_tensors = gguf.tensors.into_iter().filter_map(|(n, t)| n.strip_prefix("g2p/").map(|s| (s.to_string(), t))).collect();
        Ok(data)
    }
}

// ---- GGUF ------------------------------------------------------------------------------------------------------------

enum Meta {
    Num(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Meta>),
}

struct Gguf {
    tensors: HashMap<String, Tensor>,
    files: HashMap<String, Vec<u8>>,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self.b.get(self.at..self.at + n).ok_or("gguf: truncated")?;
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.u64()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).to_string())
    }
    fn value(&mut self, kind: u32) -> Result<Meta, String> {
        let size = [1, 1, 2, 2, 4, 4, 4, 1, 0, 0, 8, 8, 8];
        Ok(match kind {
            8 => Meta::Str(self.string()?),
            9 => {
                let inner = self.u32()?;
                let count = self.u64()? as usize;
                if inner < 2 {
                    Meta::Bytes(self.take(count)?.to_vec())
                } else {
                    Meta::List((0..count).map(|_| self.value(inner)).collect::<Result<_, _>>()?)
                }
            }
            k if (k as usize) < size.len() => {
                let s = self.take(size[k as usize])?;
                Meta::Num(match k {
                    0 | 7 => s[0] as f64,
                    1 => s[0] as i8 as f64,
                    2 => u16::from_le_bytes([s[0], s[1]]) as f64,
                    3 => i16::from_le_bytes([s[0], s[1]]) as f64,
                    4 => u32::from_le_bytes(s.try_into().unwrap()) as f64,
                    5 => i32::from_le_bytes(s.try_into().unwrap()) as f64,
                    6 => f32::from_le_bytes(s.try_into().unwrap()) as f64,
                    10 => u64::from_le_bytes(s.try_into().unwrap()) as f64,
                    11 => i64::from_le_bytes(s.try_into().unwrap()) as f64,
                    _ => f64::from_le_bytes(s.try_into().unwrap()),
                })
            }
            k => return Err(format!("gguf: value type {k}")),
        })
    }
}

fn half(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((bits >> 10) & 31) as i32;
    let frac = (bits & 1023) as f64;
    sign * match exp {
        0 => frac * 2f64.powi(-24),
        31 => if frac == 0.0 { f64::INFINITY } else { f64::NAN },
        e => (1.0 + frac / 1024.0) * 2f64.powi(e - 15),
    }
}

impl Gguf {
    fn read(bytes: &[u8]) -> Result<Gguf, String> {
        let mut r = Reader { b: bytes, at: 0 };
        if r.u32()? != 0x4655_4747 {
            return Err("not a GGUF file".into());
        }
        r.u32()?;
        let (tensor_count, key_count) = (r.u64()?, r.u64()?);
        let mut meta = HashMap::new();
        for _ in 0..key_count {
            let key = r.string()?;
            let kind = r.u32()?;
            meta.insert(key, r.value(kind)?);
        }
        let mut list = Vec::new();
        for _ in 0..tensor_count {
            let name = r.string()?;
            let dims = r.u32()?;
            let mut shape: Vec<usize> = (0..dims).map(|_| r.u64().map(|v| v as usize)).collect::<Result<_, _>>()?;
            shape.reverse();
            let (kind, offset) = (r.u32()?, r.u64()? as usize);
            list.push((name, shape, kind, offset));
        }
        let align = match meta.get("general.alignment") {
            Some(Meta::Num(a)) => *a as usize,
            _ => 32,
        };
        let start = r.at.div_ceil(align) * align;
        let mut tensors = HashMap::new();
        for (name, shape, kind, offset) in list {
            if !name.starts_with("g2p/") {
                continue;
            }
            let count: usize = shape.iter().product();
            let at = start + offset;
            let data = match kind {
                0 => bytes.get(at..at + 4 * count).ok_or("gguf: tensor data")?.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap()) as f64).collect(),
                1 => bytes.get(at..at + 2 * count).ok_or("gguf: tensor data")?.chunks_exact(2).map(|c| half(u16::from_le_bytes([c[0], c[1]]))).collect(),
                k => return Err(format!("gguf: tensor {name} of type {k} (the unknown-word model is float32)")),
            };
            tensors.insert(name, Tensor { shape, data });
        }
        let mut files = HashMap::new();
        if let (Some(Meta::List(names)), Some(Meta::List(offsets)), Some(Meta::Bytes(blob))) = (
            meta.get("audiocpp.embedded_files.names"),
            meta.get("audiocpp.embedded_files.offsets"),
            meta.get("audiocpp.embedded_files.data"),
        ) {
            for (i, name) in names.iter().enumerate() {
                if let (Meta::Str(name), Some(Meta::Num(a)), Some(Meta::Num(b))) = (name, offsets.get(i), offsets.get(i + 1)) {
                    files.insert(name.clone(), blob.get(*a as usize..*b as usize).ok_or("gguf: embedded file")?.to_vec());
                }
            }
        }
        Ok(Gguf { tensors, files })
    }
}
