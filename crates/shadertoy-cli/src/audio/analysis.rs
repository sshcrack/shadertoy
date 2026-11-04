//! Deterministic, stateless music analysis of a decoded audio file.
//!
//! Every value depends only on the requested time, so `render --frame N`,
//! `render-frames`, tests, and traces agree regardless of which frames were
//! rendered before. Per-hop onset/energy envelopes and the tempo grid are
//! computed once when the track is loaded; spectra are computed on demand.

use super::config::{MAX_BPM, MIN_BPM};
use super::decode::Pcm;
use super::fft::Fft;
use shadertoy::AudioInput;

/// ShaderToy uses a 2048-sample analyser and shows its first 512 bins.
const SPECTRUM_FFT: usize = 2048;
const SPECTRUM_BINS: usize = 512;
const WAVEFORM_SAMPLES: usize = 512;
/// Smaller FFT for the per-hop onset envelope.
const ENVELOPE_FFT: usize = 1024;
/// Web Audio's default analyser range and smoothing.
const MIN_DECIBELS: f32 = -100.0;
const MAX_DECIBELS: f32 = -30.0;
const SMOOTHING: f32 = 0.8;
const SMOOTHING_TAPS: usize = 6;
const SMOOTHING_STEP_SECONDS: f32 = 1.0 / 60.0;
const HOPS_PER_SECOND: f32 = 100.0;
const SILENCE_RMS: f32 = 0.001;

#[derive(Clone, Copy)]
enum Band {
    Low,
    Mid,
    High,
}

/// Per-hop envelopes (one entry every 10 ms).
#[derive(Default)]
struct Envelopes {
    rms: Vec<f32>,
    flux: Vec<f32>,
    kick: Vec<f32>,
    snare: Vec<f32>,
    hihat: Vec<f32>,
    low: Vec<f32>,
    mid: Vec<f32>,
    high: Vec<f32>,
}

pub struct Track {
    pcm: Pcm,
    mono: Vec<f32>,
    hop: usize,
    looped: bool,
    envelopes: Envelopes,
    bpm: f32,
    beat_origin: f32,
    beat_confidence: f32,
    spectrum_fft: Fft,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoSummary {
    pub bpm: f32,
    pub confidence: f32,
    pub first_beat: f32,
    pub duration: f32,
}

fn decibels_to_unit(magnitude: f32) -> f32 {
    let db = 20.0 * magnitude.max(1e-12).log10();
    ((db - MIN_DECIBELS) / (MAX_DECIBELS - MIN_DECIBELS)).clamp(0.0, 1.0)
}

fn percentile(values: &[f32], fraction: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[((sorted.len() - 1) as f32 * fraction) as usize]
}

fn normalize(values: &mut [f32]) {
    // Onsets are sparse, so a low percentile would sit in the noise floor.
    let max = values.iter().copied().fold(0.0, f32::max);
    let scale = percentile(values, 0.99).max(0.25 * max).max(1e-6);
    for value in values {
        *value = (*value / scale).clamp(0.0, 1.0);
    }
}

impl Track {
    pub fn analyze(mut pcm: Pcm, gain: f32, looped: bool, bpm_override: Option<f32>) -> Self {
        for sample in pcm.left.iter_mut().chain(pcm.right.iter_mut()) {
            *sample *= gain;
        }
        let mono = pcm
            .left
            .iter()
            .zip(&pcm.right)
            .map(|(l, r)| 0.5 * (l + r))
            .collect();
        let hop = ((pcm.sample_rate as f32 / HOPS_PER_SECOND).round() as usize).max(1);
        let mut track = Self {
            pcm,
            mono,
            hop,
            looped,
            envelopes: Envelopes::default(),
            bpm: 0.0,
            beat_origin: 0.0,
            beat_confidence: 0.0,
            spectrum_fft: Fft::new(SPECTRUM_FFT),
        };
        track.compute_envelopes();
        track.estimate_tempo(bpm_override);
        track
    }

    pub fn duration(&self) -> f32 {
        self.pcm.frames() as f32 / self.pcm.sample_rate as f32
    }

    pub fn tempo(&self) -> TempoSummary {
        TempoSummary {
            bpm: self.bpm,
            confidence: self.beat_confidence,
            first_beat: self.beat_origin,
            duration: self.duration(),
        }
    }

    fn hop_count(&self) -> usize {
        self.envelopes.rms.len()
    }

    fn sample(&self, channel: &[f32], index: i64) -> f32 {
        let len = channel.len() as i64;
        if len == 0 {
            return 0.0;
        }
        if self.looped {
            channel[index.rem_euclid(len) as usize]
        } else if (0..len).contains(&index) {
            channel[index as usize]
        } else {
            0.0
        }
    }

    /// `count` samples of `channel` ending (exclusively) at sample index `end`.
    fn window(&self, channel: &[f32], end: i64, count: usize) -> Vec<f32> {
        (0..count as i64)
            .map(|offset| self.sample(channel, end - count as i64 + offset))
            .collect()
    }

    fn bin_range(&self, fft_size: usize, band: Band) -> std::ops::Range<usize> {
        let nyquist = self.pcm.sample_rate as f32 / 2.0;
        let (low, high) = match band {
            Band::Low => (20.0, 250.0),
            Band::Mid => (250.0, 4000.0),
            Band::High => (4000.0, 16000.0f32.min(nyquist)),
        };
        let hz_per_bin = self.pcm.sample_rate as f32 / fft_size as f32;
        let start = ((low / hz_per_bin).floor() as usize).max(1);
        let end = ((high / hz_per_bin).ceil() as usize).clamp(start + 1, fft_size / 2);
        start..end
    }

    fn compute_envelopes(&mut self) {
        let fft = Fft::new(ENVELOPE_FFT);
        let hops = self.mono.len().div_ceil(self.hop).max(1);
        let ranges = [
            self.bin_range(ENVELOPE_FFT, Band::Low),
            self.bin_range(ENVELOPE_FFT, Band::Mid),
            self.bin_range(ENVELOPE_FFT, Band::High),
        ];
        let hz_per_bin = self.pcm.sample_rate as f32 / ENVELOPE_FFT as f32;
        let kick_end = ((150.0 / hz_per_bin).ceil() as usize).max(2);
        let snare = ((300.0 / hz_per_bin) as usize)..((5000.0 / hz_per_bin) as usize).max(3);
        let hihat_start = (6000.0 / hz_per_bin) as usize;
        let mut env = Envelopes::default();
        let mut previous: Vec<f32> = vec![0.0; ENVELOPE_FFT / 2];
        let mut magnitudes = Vec::new();
        let rms_window = self.hop * 2;
        for hop in 0..hops {
            let end = ((hop + 1) * self.hop) as i64;
            let samples = self.window(&self.mono, end, ENVELOPE_FFT);
            fft.magnitudes(&samples, &mut magnitudes);
            let compressed: Vec<f32> = magnitudes.iter().map(|m| (1.0 + 1000.0 * m).ln()).collect();
            let rise = |range: std::ops::Range<usize>| -> f32 {
                range
                    .map(|k| (compressed[k] - previous[k]).max(0.0))
                    .sum::<f32>()
            };
            env.flux.push(rise(1..ENVELOPE_FFT / 2));
            env.kick.push(rise(1..kick_end));
            env.snare.push(rise(snare.clone()));
            env.hihat.push(rise(
                hihat_start.min(ENVELOPE_FFT / 2 - 1)..ENVELOPE_FFT / 2,
            ));
            let level = |range: &std::ops::Range<usize>| -> f32 {
                let count = range.len().max(1) as f32;
                range
                    .clone()
                    .map(|k| decibels_to_unit(magnitudes[k]))
                    .sum::<f32>()
                    / count
            };
            env.low.push(level(&ranges[0]));
            env.mid.push(level(&ranges[1]));
            env.high.push(level(&ranges[2]));
            let tail = &samples[ENVELOPE_FFT - rms_window.min(ENVELOPE_FFT)..];
            env.rms
                .push((tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt());
            previous = compressed;
        }
        for values in [&mut env.flux, &mut env.kick, &mut env.snare, &mut env.hihat] {
            normalize(values);
        }
        self.envelopes = env;
    }

    fn estimate_tempo(&mut self, bpm_override: Option<f32>) {
        let onset = &self.envelopes.flux;
        let hops_per_second = self.pcm.sample_rate as f32 / self.hop as f32;
        // Remove the local mean so sustained loudness does not dominate the autocorrelation.
        let radius = (hops_per_second * 0.1) as usize;
        let detrended: Vec<f32> = (0..onset.len())
            .map(|i| {
                let lo = i.saturating_sub(radius);
                let hi = (i + radius + 1).min(onset.len());
                let mean = onset[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
                (onset[i] - mean).max(0.0)
            })
            .collect();
        let min_lag = (hops_per_second * 60.0 / MAX_BPM).floor() as usize;
        // Short clips only search tempos that repeat at least twice.
        let max_lag = ((hops_per_second * 60.0 / MIN_BPM).ceil() as usize).min(onset.len() / 2);
        let energy: f32 = detrended.iter().map(|v| v * v).sum();
        let lag_seconds = if let Some(bpm) = bpm_override {
            self.beat_confidence = 1.0;
            60.0 / bpm
        } else if max_lag > min_lag + 2 && energy > 1e-6 {
            let autocorrelation = |lag: usize| -> f32 {
                detrended[lag..]
                    .iter()
                    .zip(&detrended)
                    .map(|(a, b)| a * b)
                    .sum::<f32>()
            };
            let scores: Vec<(usize, f32, f32)> = (min_lag..=max_lag)
                .map(|lag| {
                    let ac = autocorrelation(lag);
                    let bpm = 60.0 * hops_per_second / lag as f32;
                    // Log-normal tempo prior centred on 120 BPM.
                    let prior = (-0.5 * ((bpm / 120.0).log2() / 0.7).powi(2)).exp();
                    (lag, ac, ac * prior)
                })
                .collect();
            let Some(&(mut lag, mut ac, _)) = scores.iter().max_by(|a, b| a.2.total_cmp(&b.2))
            else {
                return;
            };
            // Backbeat patterns (kick + snare every other beat) make the two-beat period
            // strongest; prefer the beat itself when its periodicity is nearly as strong.
            let half = (lag as f32 / 2.0).round() as usize;
            if half >= min_lag {
                let (_, half_ac, _) = scores[half - min_lag];
                if half_ac > 0.5 * ac {
                    lag = half;
                    ac = half_ac;
                }
            }
            let mut refined = lag as f32;
            if lag > min_lag && lag < max_lag {
                let before = scores[lag - min_lag - 1].1;
                let after = scores[lag - min_lag + 1].1;
                let curvature = before - 2.0 * ac + after;
                if curvature.abs() > 1e-9 {
                    refined += (0.5 * (before - after) / curvature).clamp(-0.5, 0.5);
                }
            }
            self.beat_confidence = (ac / energy).clamp(0.0, 1.0);
            refined / hops_per_second
        } else {
            return;
        };
        self.bpm = (60.0 / lag_seconds).clamp(MIN_BPM, MAX_BPM);
        let period = 60.0 / self.bpm * hops_per_second;
        // Anchor the beat grid where low-frequency (kick) onsets line up best, so
        // beatPhase 0 lands on the downbeat rather than on off-beat hi-hats.
        let anchor: Vec<f32> = self
            .envelopes
            .kick
            .iter()
            .zip(&detrended)
            .map(|(kick, onset)| 2.0 * kick + onset)
            .collect();
        let candidates = period.ceil() as usize;
        let best = (0..candidates)
            .map(|phase| {
                let mut score = 0.0;
                let mut position = phase as f32;
                while (position.round() as usize) < anchor.len() {
                    score += anchor[position.round() as usize];
                    position += period;
                }
                (phase, score)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(phase, _)| phase)
            .unwrap_or(0);
        self.beat_origin = best as f32 / hops_per_second;
    }

    fn hop_value(&self, values: &[f32], hop: i64) -> f32 {
        let count = values.len() as i64;
        if count == 0 {
            return 0.0;
        }
        if self.looped {
            values[hop.rem_euclid(count) as usize]
        } else if (0..count).contains(&hop) {
            values[hop as usize]
        } else {
            0.0
        }
    }

    /// Peak-hold with exponential release over the recent hops (stateless).
    fn decayed(&self, values: &[f32], hop: i64, release_seconds: f32) -> f32 {
        let hops_per_second = self.pcm.sample_rate as f32 / self.hop as f32;
        let span = (release_seconds * hops_per_second * 4.0) as i64;
        (0..=span)
            .map(|age| {
                self.hop_value(values, hop - age)
                    * (-(age as f32) / (release_seconds * hops_per_second)).exp()
            })
            .fold(0.0, f32::max)
    }

    fn mean(&self, values: &[f32], hop: i64, seconds: f32) -> f32 {
        let hops_per_second = self.pcm.sample_rate as f32 / self.hop as f32;
        let span = ((seconds * hops_per_second) as i64).max(1);
        (0..span)
            .map(|age| self.hop_value(values, hop - age))
            .sum::<f32>()
            / span as f32
    }

    /// Mean over the hops of the window that exist in the track. `None` when less
    /// than half of the window lies inside it, so comparisons against "before the
    /// start" do not register as drops or section changes.
    fn history_mean(&self, values: &[f32], hop: i64, seconds: f32) -> Option<f32> {
        if self.looped {
            return Some(self.mean(values, hop, seconds));
        }
        let hops_per_second = self.pcm.sample_rate as f32 / self.hop as f32;
        let span = ((seconds * hops_per_second) as i64).max(1);
        let count = values.len() as i64;
        let (lo, hi) = ((hop - span + 1).max(0), hop.min(count - 1));
        if hi < lo || (hi - lo + 1) * 2 < span {
            return None;
        }
        Some(values[lo as usize..=hi as usize].iter().sum::<f32>() / (hi - lo + 1) as f32)
    }

    fn smoothed_spectrum(&self, end: i64) -> Vec<f32> {
        let step = (SMOOTHING_STEP_SECONDS * self.pcm.sample_rate as f32) as i64;
        let mut accumulated = vec![0.0f32; SPECTRUM_FFT / 2];
        let mut magnitudes = Vec::new();
        let mut total_weight = 0.0;
        for tap in 0..SMOOTHING_TAPS {
            let weight = (1.0 - SMOOTHING) * SMOOTHING.powi(tap as i32);
            let samples = self.window(
                &self.mono,
                end - tap as i64 * step,
                self.spectrum_fft.size(),
            );
            self.spectrum_fft.magnitudes(&samples, &mut magnitudes);
            for (sum, magnitude) in accumulated.iter_mut().zip(&magnitudes) {
                *sum += weight * magnitude;
            }
            total_weight += weight;
        }
        for value in &mut accumulated {
            *value /= total_weight;
        }
        accumulated
    }

    /// Analysis at `time` seconds into the track (already offset by the caller).
    pub fn analyze_at(&self, time: f32) -> AudioInput {
        let rate = self.pcm.sample_rate as f32;
        let mut audio = AudioInput {
            available: true,
            silence: true,
            sample_rate: rate,
            ..AudioInput::default()
        };
        let past_end = !self.looped && (time >= self.duration() || time < 0.0);
        if past_end || self.hop_count() == 0 {
            audio.spectrum = vec![0.0; SPECTRUM_BINS];
            audio.waveform = vec![0.0; WAVEFORM_SAMPLES];
            return audio;
        }
        let end = (time * rate).round() as i64;
        // Latest hop whose analysis window ends at or before `end` (no lookahead).
        let hop = end.div_euclid(self.hop as i64) - 1;

        let linear = self.smoothed_spectrum(end);
        let unit: Vec<f32> = linear.iter().map(|m| decibels_to_unit(*m)).collect();
        audio.spectrum = unit[..SPECTRUM_BINS].to_vec();
        audio.waveform = self.window(&self.mono, end, WAVEFORM_SAMPLES);

        let band = |band: Band| {
            let range = self.bin_range(SPECTRUM_FFT, band);
            let count = range.len().max(1) as f32;
            unit[range].iter().sum::<f32>() / count
        };
        audio.bass = band(Band::Low);
        audio.mid = band(Band::Mid);
        audio.treble = band(Band::High);

        let weighted: f32 = linear.iter().enumerate().map(|(k, m)| k as f32 * m).sum();
        let total: f32 = linear.iter().sum();
        if total > 1e-9 {
            let centroid_hz = (weighted / total) * rate / SPECTRUM_FFT as f32;
            let nyquist = rate / 2.0;
            audio.spectral_centroid =
                ((centroid_hz.max(20.0) / 20.0).ln() / (nyquist / 20.0).ln()).clamp(0.0, 1.0);
        }

        let env = &self.envelopes;
        let rms = self.mean(&env.rms, hop, 0.1);
        audio.loudness = ((20.0 * rms.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
        audio.silence = self.mean(&env.rms, hop, 0.2) < SILENCE_RMS;
        audio.spectral_flux = self.hop_value(&env.flux, hop);
        audio.onset = self.decayed(&env.flux, hop, 0.08);
        audio.kick = self.decayed(&env.kick, hop, 0.10);
        audio.snare = self.decayed(&env.snare, hop, 0.08);
        audio.hihat = self.decayed(&env.hihat, hop, 0.05);

        if self.bpm > 0.0 {
            let beats = (time - self.beat_origin) * self.bpm / 60.0;
            audio.bpm = self.bpm;
            audio.beat_phase = beats - beats.floor();
            audio.beat_confidence = self.beat_confidence;
            audio.beat_strength =
                (-7.0 * audio.beat_phase).exp() * (0.5 + 0.5 * self.beat_confidence);
        }

        let hops_per_second = rate / self.hop as f32;
        let to_unit = |rms: f32| ((20.0 * rms.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
        audio.energy_trend = match (
            self.history_mean(&env.rms, hop, 1.0),
            self.history_mean(&env.rms, hop, 8.0),
        ) {
            (Some(short), Some(long)) => {
                (0.5 + 2.0 * (to_unit(short) - to_unit(long))).clamp(0.0, 1.0)
            }
            _ => 0.5,
        };
        let earlier = hop - (0.5 * hops_per_second) as i64;
        if let (Some(recent_low), Some(earlier_low)) = (
            self.history_mean(&env.low, hop, 0.5),
            self.history_mean(&env.low, earlier, 4.0),
        ) {
            audio.drop = ((recent_low - earlier_low - 0.08) * 6.0).clamp(0.0, 1.0);
        }
        let window = (4.0 * hops_per_second) as i64;
        let profile = |at: i64| -> Option<[f32; 3]> {
            Some([
                self.history_mean(&env.low, at, 4.0)?,
                self.history_mean(&env.mid, at, 4.0)?,
                self.history_mean(&env.high, at, 4.0)?,
            ])
        };
        if let (Some(now), Some(before)) = (profile(hop), profile(hop - window)) {
            let distance: f32 = now.iter().zip(&before).map(|(a, b)| (a - b).abs()).sum();
            audio.section_change = ((distance - 0.05) * 4.0).clamp(0.0, 1.0);
        }

        let left = self.window(&self.pcm.left, end, SPECTRUM_FFT);
        let right = self.window(&self.pcm.right, end, SPECTRUM_FFT);
        let (mut ll, mut rr, mut lr, mut side) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for (l, r) in left.iter().zip(&right) {
            ll += l * l;
            rr += r * r;
            lr += l * r;
            side += 0.25 * (l - r) * (l - r);
        }
        let mid = 0.25 * (ll + rr + 2.0 * lr);
        if ll + rr > 1e-9 {
            audio.stereo_width = (side.sqrt() / (mid.sqrt() + side.sqrt() + 1e-9)).clamp(0.0, 1.0);
            audio.stereo_balance =
                ((rr.sqrt() - ll.sqrt()) / (rr.sqrt() + ll.sqrt())).clamp(-1.0, 1.0);
            audio.stereo_correlation = (lr / (ll * rr).sqrt().max(1e-9)).clamp(-1.0, 1.0);
        }
        audio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const RATE: u32 = 48_000;

    /// 120 BPM: a decaying 55 Hz kick on every beat over a quiet 3 kHz tone.
    fn kick_track(seconds: f32, bpm: f32) -> Pcm {
        let frames = (seconds * RATE as f32) as usize;
        let beat = 60.0 / bpm;
        let samples: Vec<f32> = (0..frames)
            .map(|n| {
                let t = n as f32 / RATE as f32;
                let since = t % beat;
                0.8 * (-since * 25.0).exp() * (2.0 * PI * 55.0 * since).sin()
                    + 0.05 * (2.0 * PI * 3000.0 * t).sin()
            })
            .collect();
        Pcm {
            sample_rate: RATE,
            left: samples.clone(),
            right: samples,
        }
    }

    #[test]
    fn detects_tempo_and_beat_phase() {
        let track = Track::analyze(kick_track(12.0, 120.0), 1.0, false, None);
        let tempo = track.tempo();
        assert!((tempo.bpm - 120.0).abs() < 2.0, "bpm {}", tempo.bpm);
        let on_beat = track.analyze_at(4.02);
        let off_beat = track.analyze_at(4.25);
        assert!(
            on_beat.kick > off_beat.kick + 0.3,
            "{} {}",
            on_beat.kick,
            off_beat.kick
        );
        assert!(on_beat.bass > off_beat.bass);
        assert!(
            on_beat.beat_phase < 0.1 || on_beat.beat_phase > 0.9,
            "{}",
            on_beat.beat_phase
        );
    }

    #[test]
    fn spectrum_layout_and_silence_after_end() {
        let track = Track::analyze(kick_track(2.0, 120.0), 1.0, false, Some(128.0));
        let audio = track.analyze_at(1.0);
        assert_eq!(audio.spectrum.len(), SPECTRUM_BINS);
        assert_eq!(audio.waveform.len(), WAVEFORM_SAMPLES);
        assert!(audio.available && !audio.silence);
        assert_eq!(audio.bpm, 128.0);
        // Mono content: perfectly correlated, centred, no width.
        assert!(audio.stereo_correlation > 0.99 && audio.stereo_width < 0.01);
        let after = track.analyze_at(5.0);
        assert!(after.silence && after.spectrum.iter().all(|v| *v == 0.0));
        let looped = Track::analyze(kick_track(2.0, 120.0), 1.0, true, None);
        assert!(!looped.analyze_at(5.0).silence);
    }

    #[test]
    fn analysis_is_deterministic() {
        let track = Track::analyze(kick_track(3.0, 120.0), 1.0, false, None);
        assert_eq!(track.analyze_at(1.234), track.analyze_at(1.234));
    }
}
