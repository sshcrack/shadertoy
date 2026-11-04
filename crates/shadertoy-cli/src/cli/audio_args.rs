use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

/// Host audio overrides shared by every rendering command. See `shadertoy docs audio`.
#[derive(Debug, Args)]
pub(super) struct AudioArgs {
    /// Host audio for music channels and iAudio* uniforms: none, synthetic, or an audio file path.
    /// Replaces the manifest [audio] section (but not a [[test]] case's own audio).
    #[arg(long, global = true, value_name = "none|synthetic|FILE")]
    audio: Option<String>,
    /// Synthetic audio profile (implies --audio synthetic when no source is configured).
    #[arg(long, global = true, hide_short_help = true, value_parser = ["balanced", "bass", "percussion", "ambient"])]
    audio_profile: Option<String>,
    /// Tempo in BPM (40-240): synthetic tempo, or a replacement for an audio file's detected tempo.
    #[arg(long, global = true, hide_short_help = true)]
    audio_bpm: Option<f32>,
    /// Seconds into the audio file that correspond to iTime = 0.
    #[arg(long, global = true, hide_short_help = true)]
    audio_offset: Option<f32>,
    /// Linear gain applied to audio file samples before analysis.
    #[arg(long, global = true, hide_short_help = true)]
    audio_gain: Option<f32>,
    /// Loop the audio file instead of going silent after it ends.
    #[arg(long, global = true, hide_short_help = true)]
    audio_loop: bool,
}

impl AudioArgs {
    pub(super) fn into_override(self) -> Result<crate::audio::AudioOverride> {
        use crate::audio::{AudioOverrideSource, SyntheticProfile};
        let source = match self.audio.as_deref() {
            None => None,
            Some(text) => Some(match AudioOverrideSource::parse(text) {
                AudioOverrideSource::File(path) if path.is_relative() => {
                    AudioOverrideSource::File(std::env::current_dir()?.join(path))
                }
                other => other,
            }),
        };
        Ok(crate::audio::AudioOverride {
            source,
            profile: self
                .audio_profile
                .as_deref()
                .map(SyntheticProfile::parse)
                .transpose()?,
            bpm: self.audio_bpm,
            offset: self.audio_offset,
            gain: self.audio_gain,
            looped: self.audio_loop,
        })
    }
}

#[derive(Debug, Args)]
pub(super) struct AudioProbeArgs {
    /// Project directory (or any path inside it). Optional when --audio is given.
    #[arg(value_name = "PATH", conflicts_with = "project")]
    path: Option<PathBuf>,
    /// Project directory (or any path inside it).
    #[arg(long, value_name = "PATH")]
    project: Option<PathBuf>,
    /// Shader time in seconds to analyse. Repeatable; defaults to 0.
    #[arg(long = "time", value_name = "SECONDS")]
    times: Vec<f32>,
    /// Frame rate used for frame-timed synthetic pulses (default: manifest fps or 60).
    #[arg(long)]
    fps: Option<f32>,
    /// Include the 512-bin spectrum and waveform arrays in JSON output.
    #[arg(long)]
    arrays: bool,
}

impl AudioProbeArgs {
    pub(super) fn into_options(self) -> crate::ops::AudioProbeOptions {
        crate::ops::AudioProbeOptions {
            project: self.path.or(self.project),
            times: if self.times.is_empty() {
                vec![0.0]
            } else {
                self.times
            },
            fps: self.fps,
            arrays: self.arrays,
        }
    }
}
