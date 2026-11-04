//! Deterministic beat-driven music analysis.
//!
//! This is a port of the led-matrix shader preview generator
//! (`plugins/Shadertoy/matrix/preview/ShaderPreviewWorker.cpp`), so CLI renders
//! of audio-reactive shaders match the previews the matrix produces.

use super::config::{MAX_BPM, MIN_BPM, SyntheticProfile};
use shadertoy::AudioInput;
use std::f32::consts::PI;

pub const DEFAULT_BPM: f32 = 120.0;
const BINS: usize = 256;

struct Gains {
    low: f32,
    mid: f32,
    high: f32,
    transient: f32,
}

fn gains(profile: SyntheticProfile) -> Gains {
    match profile {
        SyntheticProfile::Balanced => Gains {
            low: 1.0,
            mid: 1.0,
            high: 1.0,
            transient: 1.0,
        },
        SyntheticProfile::Bass => Gains {
            low: 1.28,
            mid: 0.82,
            high: 0.58,
            transient: 0.90,
        },
        SyntheticProfile::Percussion => Gains {
            low: 1.08,
            mid: 0.92,
            high: 1.18,
            transient: 1.30,
        },
        SyntheticProfile::Ambient => Gains {
            low: 0.78,
            mid: 1.05,
            high: 0.82,
            transient: 0.45,
        },
    }
}

/// Synthetic analysis for `time` seconds at `fps` (fps only times the drop/section pulses).
pub fn synthetic_audio(time: f32, fps: f32, bpm: f32, profile: SyntheticProfile) -> AudioInput {
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        60.0
    };
    let seconds = time.max(0.0);
    let frame = (seconds * fps).round() as i64;
    let mut audio = AudioInput {
        available: true,
        silence: false,
        sample_rate: 48000.0,
        bpm: bpm.clamp(MIN_BPM, MAX_BPM),
        ..AudioInput::default()
    };

    let beats = seconds * audio.bpm / 60.0;
    audio.beat_phase = beats - beats.floor();
    audio.beat_confidence = 0.96;
    audio.beat_strength = (-7.0 * audio.beat_phase).exp();
    audio.kick = audio.beat_strength;
    let half_beat = (audio.beat_phase - 0.5).abs();
    audio.snare = (-45.0 * half_beat * half_beat).exp();
    audio.hihat = 0.25 + 0.55 * (beats * PI * 4.0).sin().max(0.0).powi(8);
    audio.onset = audio.kick.max(audio.snare).max(audio.hihat * 0.65);

    let gain = gains(profile);
    let ambient = profile == SyntheticProfile::Ambient;
    if ambient {
        audio.kick *= 0.35;
        audio.beat_strength *= 0.55;
    }

    let groove = 0.5 + 0.5 * (seconds * if ambient { 0.75 } else { 1.7 }).sin();
    audio.bass = ((0.32 + 0.58 * audio.kick) * gain.low).clamp(0.0, 1.0);
    audio.mid = ((0.34 + 0.20 * groove + 0.20 * audio.snare) * gain.mid).clamp(0.0, 1.0);
    audio.treble = ((0.30 + 0.32 * audio.hihat) * gain.high).clamp(0.0, 1.0);
    audio.loudness = (0.35 + 0.35 * audio.bass + 0.20 * audio.mid).clamp(0.0, 1.0);
    audio.onset = (audio.onset * gain.transient).clamp(0.0, 1.0);
    audio.kick = (audio.kick * gain.transient).clamp(0.0, 1.0);
    audio.snare = (audio.snare * gain.transient).clamp(0.0, 1.0);
    audio.hihat = (audio.hihat * gain.high * gain.transient).clamp(0.0, 1.0);
    audio.stereo_width = 0.62;
    audio.stereo_balance = 0.08 * (seconds * 0.7).sin();
    audio.stereo_correlation = 0.55;
    audio.energy_trend = 0.5 + 0.3 * (seconds * 0.22).sin();
    let pulse = |period_seconds: f32| {
        let period = ((fps * period_seconds) as i64).max(1);
        if frame.rem_euclid(period) < 8 {
            0.8
        } else {
            0.0
        }
    };
    audio.drop = pulse(8.0);
    audio.section_change = pulse(16.0);
    audio.spectral_centroid = 0.42 + 0.18 * (seconds * 0.33).sin();
    audio.spectral_flux = audio.onset;

    audio.spectrum = (0..BINS)
        .map(|index| {
            let x = index as f32 / (BINS - 1) as f32;
            let bass_peak = (-55.0 * (x - 0.10) * (x - 0.10)).exp() * audio.bass;
            let mid_peak = (-36.0 * (x - 0.42) * (x - 0.42)).exp() * audio.mid;
            let high_peak = (-28.0 * (x - 0.78) * (x - 0.78)).exp() * audio.treble;
            let ripple = 0.05 * (1.0 + (38.0 * x + seconds * 2.8).sin());
            (0.03 + ripple + 0.65 * bass_peak + 0.45 * mid_peak + 0.32 * high_peak).clamp(0.0, 1.0)
        })
        .collect();
    audio.waveform = (0..BINS)
        .map(|index| {
            let x = index as f32 / BINS as f32;
            (0.58 * (2.0 * PI * (3.0 * x + seconds * 0.8)).sin()
                + 0.24 * (2.0 * PI * (7.0 * x - seconds * 0.43)).sin())
            .clamp(-1.0, 1.0)
        })
        .collect();
    audio
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kick_peaks_on_the_beat_and_decays() {
        let on_beat = synthetic_audio(0.5, 60.0, 120.0, SyntheticProfile::Balanced);
        let off_beat = synthetic_audio(0.75, 60.0, 120.0, SyntheticProfile::Balanced);
        assert!(on_beat.beat_phase < 1e-4, "{}", on_beat.beat_phase);
        assert!(on_beat.kick > 0.99 && off_beat.kick < 0.05);
        assert!(on_beat.bass > off_beat.bass);
        assert_eq!(on_beat.spectrum.len(), BINS);
        assert!(on_beat.available && !on_beat.silence);
    }

    #[test]
    fn profiles_shape_the_bands() {
        let bass = synthetic_audio(1.0, 60.0, 120.0, SyntheticProfile::Bass);
        let ambient = synthetic_audio(1.0, 60.0, 120.0, SyntheticProfile::Ambient);
        let balanced = synthetic_audio(1.0, 60.0, 120.0, SyntheticProfile::Balanced);
        assert!(bass.bass > balanced.bass && bass.treble < balanced.treble);
        assert!(ambient.kick < balanced.kick);
    }

    #[test]
    fn deterministic_for_the_same_time() {
        let a = synthetic_audio(3.25, 30.0, 132.0, SyntheticProfile::Percussion);
        let b = synthetic_audio(3.25, 30.0, 132.0, SyntheticProfile::Percussion);
        assert_eq!(a, b);
    }
}
