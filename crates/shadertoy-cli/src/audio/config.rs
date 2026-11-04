use anyhow::{Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

pub const MIN_BPM: f32 = 40.0;
pub const MAX_BPM: f32 = 240.0;
const MAX_GAIN: f32 = 64.0;

/// Where the host music analysis (`music` channels and `iAudio*` uniforms) comes from.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AudioSourceKind {
    /// Deterministic silence: `iAudioAvailable = 0`, `iAudioSilence = 1`.
    #[default]
    None,
    /// Deterministic synthetic beat-driven music analysis.
    Synthetic,
    /// Frame-accurate analysis of an audio file (WAV natively, other formats via ffmpeg).
    File,
}

/// Shape of the synthetic music analysis. Matches the led-matrix preview generator.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SyntheticProfile {
    #[default]
    Balanced,
    Bass,
    Percussion,
    Ambient,
}

impl SyntheticProfile {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text.to_ascii_lowercase().as_str() {
            "balanced" => Self::Balanced,
            "bass" => Self::Bass,
            "percussion" => Self::Percussion,
            "ambient" => Self::Ambient,
            other => bail!(
                "unknown audio profile '{other}'; expected balanced, bass, percussion, or ambient"
            ),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Bass => "bass",
            Self::Percussion => "percussion",
            Self::Ambient => "ambient",
        }
    }
}

/// Host audio driving `music` channels and the `iAudio*` uniforms during CLI rendering.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AudioSection {
    /// Audio source: "none" (default), "synthetic", or "file".
    pub source: AudioSourceKind,
    /// Synthetic analysis profile: balanced (default), bass, percussion, or ambient.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<SyntheticProfile>,
    /// Tempo in BPM (40..=240). Synthetic default is 120; for files it overrides the detected tempo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f32>,
    /// Project-relative audio file analysed when source = "file".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Seconds into the audio file that correspond to iTime = 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<f32>,
    /// Linear gain applied to file samples before analysis (default 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gain: Option<f32>,
    /// Loop the file instead of going silent after its end (default false).
    #[serde(skip_serializing_if = "Option::is_none", rename = "loop")]
    pub looped: Option<bool>,
}

impl AudioSection {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn validate(&self, context: &str) -> Result<()> {
        if let Some(bpm) = self.bpm
            && !(bpm.is_finite() && (MIN_BPM..=MAX_BPM).contains(&bpm))
        {
            bail!("{context} bpm must be in {MIN_BPM}..={MAX_BPM}");
        }
        if let Some(offset) = self.offset
            && !(offset.is_finite() && offset >= 0.0)
        {
            bail!("{context} offset must be a finite non-negative number of seconds");
        }
        if let Some(gain) = self.gain
            && !(gain.is_finite() && gain > 0.0 && gain <= MAX_GAIN)
        {
            bail!("{context} gain must be in (0, {MAX_GAIN}]");
        }
        match self.source {
            AudioSourceKind::File => {
                let Some(path) = &self.path else {
                    bail!("{context} source = \"file\" requires path");
                };
                if path.trim().is_empty() {
                    bail!("{context} path must not be empty");
                }
                if self.profile.is_some() {
                    bail!("{context} profile only applies to source = \"synthetic\"");
                }
            }
            AudioSourceKind::Synthetic | AudioSourceKind::None => {
                if self.path.is_some() || self.offset.is_some() || self.gain.is_some() {
                    bail!("{context} path/offset/gain only apply to source = \"file\"");
                }
                if self.looped.is_some() {
                    bail!("{context} loop only applies to source = \"file\"");
                }
                if self.source == AudioSourceKind::None
                    && (self.profile.is_some() || self.bpm.is_some())
                {
                    bail!("{context} profile/bpm require source = \"synthetic\" or \"file\"");
                }
            }
        }
        Ok(())
    }
}

/// Process-wide `--audio*` flags. They replace the manifest `[audio]` section but
/// never a `[[test]]` case's own `audio`, so reference images stay reproducible.
#[derive(Debug, Clone, Default)]
pub struct AudioOverride {
    pub source: Option<AudioOverrideSource>,
    pub profile: Option<SyntheticProfile>,
    pub bpm: Option<f32>,
    pub offset: Option<f32>,
    pub gain: Option<f32>,
    pub looped: bool,
}

#[derive(Debug, Clone)]
pub enum AudioOverrideSource {
    None,
    Synthetic,
    /// Absolute (or cwd-relative) path supplied on the command line.
    File(PathBuf),
}

impl AudioOverrideSource {
    pub fn parse(text: &str) -> Self {
        match text {
            "none" | "off" | "silent" => Self::None,
            "synthetic" | "synth" => Self::Synthetic,
            path => Self::File(PathBuf::from(path)),
        }
    }
}

impl AudioOverride {
    pub fn is_empty(&self) -> bool {
        self.source.is_none()
            && self.profile.is_none()
            && self.bpm.is_none()
            && self.offset.is_none()
            && self.gain.is_none()
            && !self.looped
    }
}

static CLI_OVERRIDE: OnceLock<AudioOverride> = OnceLock::new();

pub fn set_cli_override(value: AudioOverride) {
    let _ = CLI_OVERRIDE.set(value);
}

pub fn cli_override() -> Option<&'static AudioOverride> {
    CLI_OVERRIDE.get().filter(|value| !value.is_empty())
}
