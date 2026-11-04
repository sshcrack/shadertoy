//! Audio file decoding into interleaved stereo f32 PCM.
//!
//! PCM/float WAV files are parsed natively so deterministic tests need no
//! external tools; every other container/codec is decoded through ffmpeg
//! (`SHADERTOY_FFMPEG` overrides the executable, as for video inputs).

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

const FFMPEG_SAMPLE_RATE: u32 = 48_000;
/// One hour of 48 kHz stereo audio.
const MAX_FRAMES: usize = 48_000 * 60 * 60;

/// Decoded audio: `left`/`right` hold one sample per frame (mono files duplicate the channel).
#[derive(Debug, Clone)]
pub struct Pcm {
    pub sample_rate: u32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    fn from_interleaved(sample_rate: u32, channels: usize, samples: &[f32]) -> Result<Self> {
        if channels == 0 {
            bail!("audio has no channels");
        }
        let frames = samples.len() / channels;
        if frames > MAX_FRAMES {
            bail!("audio is longer than the one-hour analysis limit");
        }
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for frame in samples.chunks_exact(channels) {
            left.push(frame[0]);
            right.push(if channels > 1 { frame[1] } else { frame[0] });
        }
        Ok(Self {
            sample_rate,
            left,
            right,
        })
    }
}

pub fn decode(path: &Path) -> Result<Pcm> {
    let is_wav = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"));
    if is_wav {
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read audio file {}", path.display()))?;
        match parse_wav(&bytes) {
            Ok(pcm) => return Ok(pcm),
            // Compressed/extensible WAV payloads fall through to ffmpeg.
            Err(error) if error.to_string().contains("unsupported WAV") => {}
            Err(error) => {
                return Err(error).with_context(|| format!("invalid WAV file {}", path.display()));
            }
        }
    }
    decode_with_ffmpeg(path)
}

fn le_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub fn parse_wav(bytes: &[u8]) -> Result<Pcm> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a RIFF/WAVE file");
    }
    let mut format: Option<(u16, usize, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    let mut cursor = 12usize;
    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = le_u32(bytes, cursor + 4) as usize;
        let body_start = cursor + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    bail!("WAV fmt chunk is truncated");
                }
                let mut tag = le_u16(body, 0);
                let channels = usize::from(le_u16(body, 2));
                let rate = le_u32(body, 4);
                let bits = le_u16(body, 14);
                // WAVE_FORMAT_EXTENSIBLE stores the real format in the sub-format GUID.
                if tag == 0xFFFE && body.len() >= 26 {
                    tag = le_u16(body, 24);
                }
                format = Some((tag, channels, rate, bits));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        cursor = body_start.saturating_add(size).saturating_add(size & 1);
    }
    let (tag, channels, rate, bits) = format.context("WAV file has no fmt chunk")?;
    let data = data.context("WAV file has no data chunk")?;
    if channels == 0 || rate == 0 {
        bail!("WAV file declares zero channels or sample rate");
    }
    let samples: Vec<f32> = match (tag, bits) {
        (1, 8) => data
            .iter()
            .map(|&b| (f32::from(b) - 128.0) / 128.0)
            .collect(),
        (1, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0)
            .collect(),
        (1, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (1, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0)
            .collect(),
        (3, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        (3, 64) => data
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c) as f32)
            .collect(),
        _ => bail!("unsupported WAV encoding (format tag {tag}, {bits} bits)"),
    };
    let samples: Vec<f32> = samples
        .into_iter()
        .map(|sample| if sample.is_finite() { sample } else { 0.0 })
        .collect();
    Pcm::from_interleaved(rate, channels, &samples)
}

fn decode_with_ffmpeg(path: &Path) -> Result<Pcm> {
    let executable = std::env::var_os("SHADERTOY_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let output = Command::new(executable)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vn",
            "-ac",
            "2",
            "-ar",
            &FFMPEG_SAMPLE_RATE.to_string(),
            "-f",
            "f32le",
            "pipe:1",
        ])
        .output()
        .with_context(|| {
            "failed to launch ffmpeg to decode audio; use a PCM .wav file, install ffmpeg, or set SHADERTOY_FFMPEG"
        })?;
    if !output.status.success() {
        bail!(
            "ffmpeg failed to decode audio {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let samples: Vec<f32> = output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .map(|sample| if sample.is_finite() { sample } else { 0.0 })
        .collect();
    Pcm::from_interleaved(FFMPEG_SAMPLE_RATE, 2, &samples)
}

/// Encode interleaved f32 samples as a 16-bit PCM WAV (test fixtures).
#[cfg(test)]
pub fn encode_wav_i16(sample_rate: u32, channels: u16, interleaved: &[f32]) -> Vec<u8> {
    let data_len = (interleaved.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
    bytes.extend_from_slice(&(channels * 2).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in interleaved {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stereo_pcm16() {
        let bytes = encode_wav_i16(8000, 2, &[0.5, -0.5, 0.25, -0.25]);
        let pcm = parse_wav(&bytes).expect("parse");
        assert_eq!(pcm.sample_rate, 8000);
        assert_eq!(pcm.frames(), 2);
        assert!((pcm.left[0] - 0.5).abs() < 1e-3 && (pcm.right[1] + 0.25).abs() < 1e-3);
    }

    #[test]
    fn mono_is_duplicated() {
        let bytes = encode_wav_i16(8000, 1, &[0.5, 0.25]);
        let pcm = parse_wav(&bytes).expect("parse");
        assert_eq!(pcm.left, pcm.right);
    }

    #[test]
    fn rejects_non_wav() {
        assert!(parse_wav(b"not audio at all").is_err());
    }
}
