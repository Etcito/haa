//! WAV writer (ARCHITECTURE §16): RIFF PCM 16-bit stereo, samples scaled by 32767,
//! rounded half away from zero and clamped to the 16-bit range.

use std::path::Path;

/// Scale one f32 sample to the signed 16-bit range like the JS host writer.
/// The product stays in f32, so a sample authored as `0.5 / 32767` lands exactly on a tie.
fn to_i16(sample: f32) -> i16 {
    (sample * 32767.0).round().clamp(-32767.0, 32767.0) as i16
}

/// Encode interleaved stereo PCM into a RIFF/WAVE byte buffer.
/// Frames are the shorter of the two channels.
pub fn encode_wav(left: &[f32], right: &[f32], sample_rate: u32) -> Vec<u8> {
    let frames = left.len().min(right.len());
    let data_size = (frames * 4) as u32;
    let mut out = Vec::with_capacity(44 + data_size as usize);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_size).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    for i in 0..frames {
        out.extend_from_slice(&to_i16(left[i]).to_le_bytes());
        out.extend_from_slice(&to_i16(right[i]).to_le_bytes());
    }
    out
}

/// Write a stereo WAV file, creating its parent directories.
pub fn write_wav(
    path: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, encode_wav(left, right, sample_rate))
}

/// Decode a RIFF/WAVE buffer (PCM 8/16/24/32-bit or IEEE float 32/64) to mono (channel mean)
/// and its sample rate.
pub fn decode_wav_mono(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".to_string());
    }
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(at + 4) as usize;
        let body = at + 8;
        let end = (body + size).min(bytes.len());
        if id == b"fmt " && size >= 16 {
            let mut format = u16_at(body);
            if format == 0xFFFE && size >= 26 {
                format = u16_at(body + 24);
            }
            fmt = Some((format, u16_at(body + 2), u32_at(body + 4), u16_at(body + 14)));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or("data chunk before fmt")?;
            let channels = channels.max(1) as usize;
            let width = (bits as usize).div_ceil(8);
            if width == 0 {
                return Err("zero sample width".to_string());
            }
            let data = &bytes[body..end];
            let frames = data.len() / (width * channels);
            let sample = |o: usize| -> Result<f32, String> {
                let s = &data[o..o + width];
                Ok(match (format, bits) {
                    (1, 8) => (s[0] as f32 - 128.0) / 128.0,
                    (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
                    (1, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
                    (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
                    (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                    (3, 64) => f64::from_le_bytes(s.try_into().unwrap()) as f32,
                    _ => return Err(format!("unsupported WAV format {format}/{bits}-bit")),
                })
            };
            let mut mono = Vec::with_capacity(frames);
            for f in 0..frames {
                let mut sum = 0.0f32;
                for c in 0..channels {
                    sum += sample((f * channels + c) * width)?;
                }
                mono.push(sum / channels as f32);
            }
            return Ok((mono, rate));
        }
        at = body + size + (size & 1);
    }
    Err("no data chunk".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    fn half(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
    }

    #[test]
    fn header_is_a_16_bit_stereo_riff_at_the_given_rate() {
        let frames = 4;
        let left = vec![0.0; frames];
        let right = vec![0.0; frames];
        let wav = encode_wav(&left, &right, 48_000);

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(word(&wav, 4), 36 + (frames * 4) as u32);
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(word(&wav, 16), 16);
        assert_eq!(half(&wav, 20), 1);
        assert_eq!(half(&wav, 22), 2);
        assert_eq!(word(&wav, 24), 48_000);
        assert_eq!(word(&wav, 28), 48_000 * 4);
        assert_eq!(half(&wav, 32), 4);
        assert_eq!(half(&wav, 34), 16);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(word(&wav, 40), (frames * 4) as u32);
        assert_eq!(wav.len(), 44 + frames * 4);
    }

    #[test]
    fn samples_round_half_away_from_zero_and_clamp() {
        let left = [0.5 / 32767.0, -0.5 / 32767.0, 2.0, -2.0, 1.0 / 32767.0, 0.0];
        let right = vec![0.0; left.len()];
        let wav = encode_wav(&left, &right, 48_000);
        let sample = |i: usize| i16::from_le_bytes(wav[44 + i * 4..46 + i * 4].try_into().unwrap());

        assert_eq!(sample(0), 1);
        assert_eq!(sample(1), -1);
        assert_eq!(sample(2), 32767);
        assert_eq!(sample(3), -32767);
        assert_eq!(sample(4), 1);
        assert_eq!(sample(5), 0);
    }

    #[test]
    fn write_wav_creates_parent_directories() {
        let dir = std::env::temp_dir().join(format!("haa-cli-wav-{}", std::process::id()));
        let path = dir.join("nested/render.wav");
        let _ = std::fs::remove_dir_all(&dir);

        write_wav(&path, &[0.0, 0.25], &[0.0, -0.25], 44_100).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(word(&bytes, 24), 44_100);
        assert_eq!(word(&bytes, 40), 2 * 4);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
