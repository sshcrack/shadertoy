//! Host audio for CLI rendering: deterministic silence, synthetic music
//! analysis, or frame-accurate analysis of an audio file.
//!
//! The effective source is chosen per render as: a `[[test]]` case's `audio`,
//! else the global `--audio*` flags, else the manifest `[audio]` section.

mod analysis;
mod config;
mod decode;
mod fft;
mod synthetic;

pub use config::{
    AudioOverride, AudioOverrideSource, AudioSection, AudioSourceKind, SyntheticProfile,
    set_cli_override,
};

use analysis::Track;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use shadertoy::{AudioInput, Runtime};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// Resolved, ready-to-sample audio source.
#[derive(Clone, Default)]
pub enum AudioDriver {
    #[default]
    Silent,
    Synthetic {
        profile: SyntheticProfile,
        bpm: f32,
    },
    File {
        path: PathBuf,
        offset: f32,
        looped: bool,
        track: Arc<Track>,
    },
}

impl std::fmt::Debug for AudioDriver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.describe())
    }
}

impl AudioDriver {
    /// Resolve the effective audio for `root`'s manifest section, honouring the
    /// CLI override unless a test case supplies its own section.
    pub fn resolve(
        root: &Path,
        manifest: &AudioSection,
        test: Option<&AudioSection>,
    ) -> Result<Self> {
        if let Some(section) = test {
            return Self::from_section(root, section, "test audio");
        }
        if let Some(cli) = config::cli_override() {
            return Self::from_override(root, manifest, cli);
        }
        Self::from_section(root, manifest, "[audio]")
    }

    fn from_section(root: &Path, section: &AudioSection, context: &str) -> Result<Self> {
        section.validate(context)?;
        Ok(match section.source {
            AudioSourceKind::None => Self::Silent,
            AudioSourceKind::Synthetic => Self::Synthetic {
                profile: section.profile.unwrap_or_default(),
                bpm: section.bpm.unwrap_or(synthetic::DEFAULT_BPM),
            },
            AudioSourceKind::File => {
                let relative = section.path.as_deref().unwrap_or_default();
                let path =
                    crate::project::existing_project_file(root, relative, "audio file", "audio")?;
                Self::file(path, section)?
            }
        })
    }

    fn from_override(root: &Path, manifest: &AudioSection, cli: &AudioOverride) -> Result<Self> {
        let mut section = manifest.clone();
        let mut cli_path = None;
        match &cli.source {
            Some(AudioOverrideSource::None) => return Ok(Self::Silent),
            Some(AudioOverrideSource::Synthetic) => {
                section = AudioSection {
                    source: AudioSourceKind::Synthetic,
                    ..AudioSection::default()
                };
            }
            Some(AudioOverrideSource::File(path)) => {
                section = AudioSection {
                    source: AudioSourceKind::File,
                    path: Some(path.to_string_lossy().into_owned()),
                    ..AudioSection::default()
                };
                cli_path = Some(path.clone());
            }
            None if section.source == AudioSourceKind::None => {
                // `--audio-profile`/`--audio-bpm` alone imply synthetic audio.
                section.source = AudioSourceKind::Synthetic;
            }
            None => {}
        }
        let file = section.source == AudioSourceKind::File;
        if !file && (cli.offset.is_some() || cli.gain.is_some() || cli.looped) {
            anyhow::bail!("--audio-offset/--audio-gain/--audio-loop only apply to audio files");
        }
        if cli.profile.is_some() {
            section.profile = cli.profile;
        }
        section.bpm = cli.bpm.or(section.bpm);
        if file {
            section.offset = cli.offset.or(section.offset);
            section.gain = cli.gain.or(section.gain);
            if cli.looped {
                section.looped = Some(true);
            }
        }
        section.validate("--audio")?;
        match cli_path {
            Some(path) => {
                if !path.is_file() {
                    anyhow::bail!("audio file {} does not exist", path.display());
                }
                Self::file(path, &section)
            }
            None => Self::from_section(root, &section, "[audio]"),
        }
    }

    fn file(path: PathBuf, section: &AudioSection) -> Result<Self> {
        let gain = section.gain.unwrap_or(1.0);
        let looped = section.looped.unwrap_or(false);
        let track = load_track(&path, gain, looped, section.bpm)?;
        Ok(Self::File {
            path,
            offset: section.offset.unwrap_or(0.0),
            looped,
            track,
        })
    }

    /// Self-contained manifest form of a silent or synthetic driver (`None` for silence/files).
    pub fn to_section(&self) -> Option<AudioSection> {
        match self {
            Self::Synthetic { profile, bpm } => Some(AudioSection {
                source: AudioSourceKind::Synthetic,
                profile: Some(*profile),
                bpm: Some(*bpm),
                ..AudioSection::default()
            }),
            Self::Silent | Self::File { .. } => None,
        }
    }

    pub fn is_silent(&self) -> bool {
        matches!(self, Self::Silent)
    }

    /// Analysis for shader time `time` at `fps`; `None` means the silent default.
    pub fn input_at(&self, time: f32, fps: f32) -> Option<AudioInput> {
        match self {
            Self::Silent => None,
            Self::Synthetic { profile, bpm } => {
                Some(synthetic::synthetic_audio(time, fps, *bpm, *profile))
            }
            Self::File { offset, track, .. } => Some(track.analyze_at(time + offset)),
        }
    }

    pub fn apply(&self, runtime: &mut Runtime<'_>, time: f32) -> Result<()> {
        match self.input_at(time, runtime.frame_rate()) {
            Some(audio) => runtime.set_audio_input(&audio)?,
            None => runtime.clear_audio_input()?,
        }
        Ok(())
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Silent => "none".into(),
            Self::Synthetic { profile, bpm } => {
                format!("synthetic ({}, {bpm:.0} BPM)", profile.as_str())
            }
            Self::File { path, track, .. } => {
                let tempo = track.tempo();
                format!(
                    "file {} ({:.1}s, {:.1} BPM)",
                    path.display(),
                    tempo.duration,
                    tempo.bpm
                )
            }
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Silent => json!({ "source": "none" }),
            Self::Synthetic { profile, bpm } => {
                json!({ "source": "synthetic", "profile": profile.as_str(), "bpm": bpm })
            }
            Self::File {
                path,
                offset,
                looped,
                track,
            } => {
                let tempo = track.tempo();
                json!({
                    "source": "file",
                    "path": path,
                    "offset": offset,
                    "loop": looped,
                    "duration": tempo.duration,
                    "bpm": tempo.bpm,
                    "beat_confidence": tempo.confidence,
                    "first_beat": tempo.first_beat,
                })
            }
        }
    }
}

/// Full analysis values for JSON/diagnostic output.
pub fn audio_input_json(audio: &AudioInput, include_arrays: bool) -> Value {
    let mut value = json!({
        "available": audio.available,
        "silence": audio.silence,
        "sample_rate": audio.sample_rate,
        "loudness": audio.loudness,
        "bass": audio.bass,
        "mid": audio.mid,
        "treble": audio.treble,
        "onset": audio.onset,
        "kick": audio.kick,
        "snare": audio.snare,
        "hihat": audio.hihat,
        "bpm": audio.bpm,
        "beat_phase": audio.beat_phase,
        "beat_confidence": audio.beat_confidence,
        "beat_strength": audio.beat_strength,
        "stereo_width": audio.stereo_width,
        "stereo_balance": audio.stereo_balance,
        "stereo_correlation": audio.stereo_correlation,
        "energy_trend": audio.energy_trend,
        "drop": audio.drop,
        "section_change": audio.section_change,
        "spectral_centroid": audio.spectral_centroid,
        "spectral_flux": audio.spectral_flux,
    });
    if include_arrays {
        value["spectrum"] = json!(audio.spectrum);
        value["waveform"] = json!(audio.waveform);
    }
    value
}

type TrackKey = (PathBuf, Option<SystemTime>, u64, u32, bool, Option<u32>);

/// Decoded tracks are cached per process so preview reloads and multi-runtime
/// commands (tests, profiling, blind comparisons) analyse each file once.
fn load_track(path: &Path, gain: f32, looped: bool, bpm: Option<f32>) -> Result<Arc<Track>> {
    static CACHE: OnceLock<Mutex<HashMap<TrackKey, Arc<Track>>>> = OnceLock::new();
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read audio file {}", path.display()))?;
    let key = (
        path.to_path_buf(),
        metadata.modified().ok(),
        metadata.len(),
        gain.to_bits(),
        looped,
        bpm.map(f32::to_bits),
    );
    let cache = CACHE.get_or_init(Default::default);
    if let Some(track) = cache.lock().expect("audio cache poisoned").get(&key) {
        return Ok(track.clone());
    }
    let pcm = decode::decode(path)?;
    if pcm.frames() == 0 {
        anyhow::bail!("audio file {} contains no samples", path.display());
    }
    let track = Arc::new(Track::analyze(pcm, gain, looped, bpm));
    cache
        .lock()
        .expect("audio cache poisoned")
        .insert(key, track.clone());
    Ok(track)
}
