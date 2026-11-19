//! GGUF reader: the metadata, the tensors (F32, F16, Q8_0 and Q4_0, all dequantized to f32) and the files the
//! tiny-voca exporter embeds (`audiocpp.embedded_files.*`, among them `config.json`).
//! Port of `readGGUF` in tiny-voca `js/tiny-voca.js`. The bytes come from the host; nothing is bundled.

use std::collections::HashMap;

/// One metadata value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    /// An array of u8/i8: kept as raw bytes (the embedded files live in one).
    Bytes(Vec<u8>),
    Array(Vec<Value>),
}

impl Value {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Float(v) => Some(*v as i64),
            _ => None,
        }
    }
}

/// One tensor, dequantized, with its shape in PyTorch order (outermost first).
#[derive(Clone, Debug)]
pub struct Tensor {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

pub struct Gguf {
    pub meta: HashMap<String, Value>,
    pub tensors: HashMap<String, Tensor>,
    pub files: HashMap<String, Vec<u8>>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or("GGUF: truncated file")?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, String> {
        let n = self.u64()?;
        if n > self.bytes.len() as u64 {
            return Err("GGUF: count larger than the file".into());
        }
        Ok(n as usize)
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.count()?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "GGUF: string is not UTF-8".into())
    }
    fn value(&mut self, kind: u32, depth: u32) -> Result<Value, String> {
        let b = |r: &mut Self, n| -> Result<Vec<u8>, String> { Ok(r.take(n)?.to_vec()) };
        Ok(match kind {
            0 => Value::Int(self.take(1)?[0] as i64),
            1 => Value::Int(self.take(1)?[0] as i8 as i64),
            2 => Value::Int(u16::from_le_bytes(b(self, 2)?.try_into().unwrap()) as i64),
            3 => Value::Int(i16::from_le_bytes(b(self, 2)?.try_into().unwrap()) as i64),
            4 => Value::Int(self.u32()? as i64),
            5 => Value::Int(self.u32()? as i32 as i64),
            6 => Value::Float(f32::from_le_bytes(b(self, 4)?.try_into().unwrap()) as f64),
            7 => Value::Bool(self.take(1)?[0] != 0),
            8 => Value::Str(self.string()?),
            9 => {
                if depth > 4 {
                    return Err("GGUF: arrays nested too deep".into());
                }
                let inner = self.u32()?;
                let n = self.count()?;
                if inner < 2 {
                    Value::Bytes(self.take(n)?.to_vec())
                } else {
                    let mut items = Vec::with_capacity(n.min(1 << 16));
                    for _ in 0..n {
                        items.push(self.value(inner, depth + 1)?);
                    }
                    Value::Array(items)
                }
            }
            10 => Value::Int(self.u64()? as i64),
            11 => Value::Int(self.u64()? as i64),
            12 => Value::Float(f64::from_le_bytes(b(self, 8)?.try_into().unwrap())),
            _ => return Err(format!("GGUF: unknown value type {kind}")),
        })
    }
}

/// IEEE half to f32.
pub fn half(bits: u16) -> f32 {
    let exponent = ((bits >> 10) & 31) as i32;
    let fraction = (bits & 1023) as f32;
    let sign = if bits >> 15 != 0 { -1.0 } else { 1.0 };
    if exponent == 0 {
        return sign * fraction * (2.0f32).powi(-24);
    }
    if exponent == 31 {
        return if fraction != 0.0 { f32::NAN } else { sign * f32::INFINITY };
    }
    sign * (1.0 + fraction / 1024.0) * (2.0f32).powi(exponent - 15)
}

/// Bytes a tensor of `count` elements takes for GGUF type `kind`.
fn stored_size(kind: u32, count: usize) -> Result<usize, String> {
    Ok(match kind {
        0 => count * 4,
        1 => count * 2,
        8 => count.div_ceil(32) * 34,
        2 => count.div_ceil(32) * 18,
        _ => return Err(format!("GGUF: tensor type {kind} is not supported")),
    })
}

fn unpack(raw: &[u8], kind: u32, count: usize) -> Vec<f32> {
    let h = |at: usize| half(u16::from_le_bytes([raw[at], raw[at + 1]]));
    match kind {
        0 => raw.chunks_exact(4).take(count).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect(),
        1 => (0..count).map(|i| h(2 * i)).collect(),
        8 => (0..count)
            .map(|i| {
                let block = (i >> 5) * 34;
                h(block) * raw[block + 2 + (i & 31)] as i8 as f32
            })
            .collect(),
        2 => (0..count)
            .map(|i| {
                let block = (i >> 5) * 18;
                let byte = raw[block + 2 + (i & 15)];
                let q = if i & 16 != 0 { byte >> 4 } else { byte & 15 };
                h(block) * (q as f32 - 8.0)
            })
            .collect(),
        _ => unreachable!(),
    }
}

/// Parses a whole GGUF file (version 2 or 3).
pub fn read(bytes: &[u8]) -> Result<Gguf, String> {
    let mut r = Reader { bytes, at: 0 };
    if r.u32()? != 0x4655_4747 {
        return Err("not a GGUF file".into());
    }
    let version = r.u32()?;
    if !(2..=3).contains(&version) {
        return Err(format!("GGUF version {version} is not supported"));
    }
    let tensor_count = r.count()?;
    let key_count = r.count()?;
    let mut meta = HashMap::new();
    for _ in 0..key_count {
        let key = r.string()?;
        let kind = r.u32()?;
        meta.insert(key, r.value(kind, 0)?);
    }
    let mut list = Vec::with_capacity(tensor_count.min(4096));
    for _ in 0..tensor_count {
        let name = r.string()?;
        let dims = r.u32()? as usize;
        if dims > 8 {
            return Err("GGUF: tensor with too many dimensions".into());
        }
        let mut shape = Vec::with_capacity(dims);
        for _ in 0..dims {
            shape.push(r.count()?);
        }
        shape.reverse();
        let kind = r.u32()?;
        let offset = r.u64()? as usize;
        list.push((name, shape, kind, offset));
    }
    let align = meta.get("general.alignment").and_then(Value::as_int).unwrap_or(32).max(1) as usize;
    let start = r.at.div_ceil(align) * align;
    let mut tensors = HashMap::new();
    for (name, shape, kind, offset) in list {
        let count = shape.iter().product::<usize>();
        let size = stored_size(kind, count)?;
        let from = start.checked_add(offset).ok_or("GGUF: bad offset")?;
        let raw = bytes.get(from..from + size).ok_or_else(|| format!("GGUF: tensor {name} runs past the end"))?;
        tensors.insert(name, Tensor { shape, data: unpack(raw, kind, count) });
    }
    let mut files = HashMap::new();
    if let (Some(Value::Array(names)), Some(Value::Array(offsets)), Some(Value::Bytes(data))) = (
        meta.get("audiocpp.embedded_files.names"),
        meta.get("audiocpp.embedded_files.offsets"),
        meta.get("audiocpp.embedded_files.data"),
    ) {
        for (i, name) in names.iter().enumerate() {
            let (Value::Str(name), Some(a), Some(b)) =
                (name, offsets.get(i).and_then(Value::as_int), offsets.get(i + 1).and_then(Value::as_int))
            else {
                continue;
            };
            if let Some(slice) = data.get(a as usize..b as usize) {
                files.insert(name.clone(), slice.to_vec());
            }
        }
    }
    Ok(Gguf { meta, tensors, files })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as u64).to_le_bytes());
        out.extend(s.as_bytes());
    }

    /// One tensor of each supported type and an embedded file, dequantized to the values written.
    #[test]
    fn reads_every_tensor_type_and_embedded_files() {
        let mut h = Vec::new();
        h.extend(0x4655_4747u32.to_le_bytes());
        h.extend(3u32.to_le_bytes());
        h.extend(4u64.to_le_bytes()); // tensors
        h.extend(3u64.to_le_bytes()); // keys
        string(&mut h, "audiocpp.embedded_files.names");
        h.extend(9u32.to_le_bytes());
        h.extend(8u32.to_le_bytes());
        h.extend(1u64.to_le_bytes());
        string(&mut h, "config.json");
        string(&mut h, "audiocpp.embedded_files.offsets");
        h.extend(9u32.to_le_bytes());
        h.extend(4u32.to_le_bytes());
        h.extend(2u64.to_le_bytes());
        h.extend(0u32.to_le_bytes());
        h.extend(2u32.to_le_bytes());
        string(&mut h, "audiocpp.embedded_files.data");
        h.extend(9u32.to_le_bytes());
        h.extend(0u32.to_le_bytes());
        h.extend(2u64.to_le_bytes());
        h.extend(b"{}");
        // data blocks: f32 [2], f16 [2] (1.0, -2.0), q8_0 [32] (scale 0.5), q4_0 [32] (scale 1.0)
        let mut data = Vec::new();
        let mut offsets = Vec::new();
        offsets.push(data.len());
        data.extend(1.5f32.to_le_bytes());
        data.extend((-3.25f32).to_le_bytes());
        offsets.push(data.len());
        data.extend(0x3C00u16.to_le_bytes());
        data.extend(0xC000u16.to_le_bytes());
        while data.len() % 32 != 0 { data.push(0); }
        offsets.push(data.len());
        data.extend(0x3800u16.to_le_bytes());
        data.extend((0..32).map(|i| (i as i8 - 16) as u8));
        while data.len() % 32 != 0 { data.push(0); }
        offsets.push(data.len());
        data.extend(0x3C00u16.to_le_bytes());
        data.extend((0..16u8).map(|i| i | (15 - i) << 4));
        for (i, (name, dims, kind)) in [("a", vec![2u64], 0u32), ("b", vec![2], 1), ("c", vec![32], 8), ("d", vec![16, 2], 2)].iter().enumerate() {
            string(&mut h, name);
            h.extend((dims.len() as u32).to_le_bytes());
            for d in dims { h.extend(d.to_le_bytes()); }
            h.extend(kind.to_le_bytes());
            h.extend((offsets[i] as u64).to_le_bytes());
        }
        while h.len() % 32 != 0 { h.push(0); }
        h.extend(data);

        let g = read(&h).unwrap();
        assert_eq!(g.files["config.json"], b"{}");
        assert_eq!(g.tensors["a"].data, vec![1.5, -3.25]);
        assert_eq!(g.tensors["b"].data, vec![1.0, -2.0]);
        assert_eq!(g.tensors["c"].data, (0..32).map(|i| 0.5 * (i - 16) as f32).collect::<Vec<_>>());
        assert_eq!(g.tensors["d"].shape, vec![2, 16]);
        let q4: Vec<f32> = (0..32).map(|i| if i < 16 { i as f32 - 8.0 } else { (15 - (i - 16)) as f32 - 8.0 }).collect();
        assert_eq!(g.tensors["d"].data, q4);
    }
}
