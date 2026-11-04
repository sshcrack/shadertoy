/// Host-provided music analysis for audio-reactive shaders.
///
/// Mirrors the native `ShaderToy::AudioInput`. `spectrum` holds normalized
/// 0..1 magnitudes ordered from low to high frequency and `waveform` holds
/// -1..1 samples; the renderer resamples both into the conventional 512x2
/// music texture (row 0 spectrum, row 1 waveform remapped to 0..1). The scalar
/// fields are exposed to GLSL as the `iAudio*` aliases.
///
/// The default value is the renderer's silent state: `available = false`,
/// `silence = true`, and every analysis value zero.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioInput {
    pub available: bool,
    pub silence: bool,
    pub sample_rate: f32,
    pub spectrum: Vec<f32>,
    pub waveform: Vec<f32>,
    pub loudness: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub onset: f32,
    pub kick: f32,
    pub snare: f32,
    pub hihat: f32,
    pub bpm: f32,
    pub beat_phase: f32,
    pub beat_confidence: f32,
    pub beat_strength: f32,
    pub stereo_width: f32,
    pub stereo_balance: f32,
    pub stereo_correlation: f32,
    pub energy_trend: f32,
    pub drop: f32,
    pub section_change: f32,
    pub spectral_centroid: f32,
    pub spectral_flux: f32,
}

impl Default for AudioInput {
    fn default() -> Self {
        Self {
            available: false,
            silence: true,
            sample_rate: 0.0,
            spectrum: Vec::new(),
            waveform: Vec::new(),
            loudness: 0.0,
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            onset: 0.0,
            kick: 0.0,
            snare: 0.0,
            hihat: 0.0,
            bpm: 0.0,
            beat_phase: 0.0,
            beat_confidence: 0.0,
            beat_strength: 0.0,
            stereo_width: 0.0,
            stereo_balance: 0.0,
            stereo_correlation: 0.0,
            energy_trend: 0.0,
            drop: 0.0,
            section_change: 0.0,
            spectral_centroid: 0.0,
            spectral_flux: 0.0,
        }
    }
}

impl AudioInput {
    /// Silent, unavailable audio (the renderer default).
    pub fn silent() -> Self {
        Self::default()
    }
}
