use crate::manifest::LoadedManifest;
use crate::source::expand_all;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub const REPLAY_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayFile {
    pub format: u32,
    pub project: String,
    pub project_fingerprint: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidated: Option<String>,
    #[serde(default)]
    pub frames: Vec<ReplayFrame>,
    #[serde(default)]
    pub events: Vec<ReplayEvent>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ReplayFrame {
    pub runtime_frame: i32,
    pub time: f32,
    pub time_delta: f32,
    pub frame_rate: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayEvent {
    pub frame: u64,
    #[serde(flatten)]
    pub action: ReplayAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ReplayAction {
    Reset,
    Resolution {
        width: u32,
        height: u32,
    },
    TimeScale {
        value: f32,
    },
    Uniform {
        name: String,
        value: crate::uniforms::UniformValue,
    },
    Mouse {
        x: f32,
        y: f32,
        down: bool,
        clicked: bool,
    },
    Key {
        code: u8,
        down: bool,
        pressed: bool,
    },
}

impl ReplayFile {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).with_context(|| format!("failed to read replay {}", path.display()))?;
        let recording: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse replay {}", path.display()))?;
        recording.validate()?;
        Ok(recording)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != REPLAY_FORMAT_VERSION {
            bail!(
                "unsupported replay format {}; expected {}",
                self.format,
                REPLAY_FORMAT_VERSION
            );
        }
        if let Some(reason) = &self.invalidated {
            bail!("replay recording is not reproducible: {reason}");
        }
        if self.width == 0 || self.height == 0 {
            bail!("replay initial resolution must be positive");
        }
        if !self.fps.is_finite() || self.fps <= 0.0 {
            bail!("replay fps must be finite and positive");
        }
        let mut previous = 0u64;
        for (index, event) in self.events.iter().enumerate() {
            if index != 0 && event.frame < previous {
                bail!("replay events are not ordered by frame");
            }
            if event.frame as usize > self.frames.len() {
                bail!("replay event targets a frame beyond the recorded timeline");
            }
            previous = event.frame;
        }
        for frame in &self.frames {
            if frame.runtime_frame < 0
                || !frame.time.is_finite()
                || frame.time < 0.0
                || !frame.time_delta.is_finite()
                || frame.time_delta < 0.0
                || !frame.frame_rate.is_finite()
                || frame.frame_rate < 0.0
            {
                bail!("replay contains an invalid runtime frame/time marker");
            }
        }
        Ok(())
    }
}

pub struct ReplayRecorder {
    path: PathBuf,
    file: ReplayFile,
    dirty_frames: usize,
}

impl ReplayRecorder {
    pub fn create(path: PathBuf, loaded: &LoadedManifest) -> Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let file = ReplayFile {
            format: REPLAY_FORMAT_VERSION,
            project: loaded.manifest.project.name.clone(),
            project_fingerprint: project_fingerprint(loaded)?,
            width: loaded.manifest.render.width,
            height: loaded.manifest.render.height,
            fps: loaded.manifest.render.fps,
            invalidated: None,
            frames: Vec::new(),
            events: Vec::new(),
        };
        let recorder = Self {
            path,
            file,
            dirty_frames: 0,
        };
        recorder.save()?;
        Ok(recorder)
    }

    pub fn timeline_frame(&self) -> u64 {
        self.file.frames.len() as u64
    }

    pub fn invalidate(&mut self, reason: impl Into<String>) -> Result<()> {
        if self.file.invalidated.is_none() {
            self.file.invalidated = Some(reason.into());
            self.save()?;
        }
        Ok(())
    }

    pub fn record(&mut self, action: ReplayAction) -> Result<()> {
        if self.file.invalidated.is_some() {
            return Ok(());
        }
        self.file.events.push(ReplayEvent {
            frame: self.timeline_frame(),
            action,
        });
        self.save()
    }

    pub fn record_frame(
        &mut self,
        runtime_frame: i32,
        time: f32,
        time_delta: f32,
        frame_rate: f32,
    ) -> Result<()> {
        if self.file.invalidated.is_some() {
            return Ok(());
        }
        self.file.frames.push(ReplayFrame {
            runtime_frame,
            time,
            time_delta,
            frame_rate,
        });
        self.dirty_frames += 1;
        if self.file.frames.len() == 1 || self.dirty_frames >= 60 {
            self.save()?;
            self.dirty_frames = 0;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.save()?;
        self.dirty_frames = 0;
        Ok(())
    }

    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.file)?;
        let temporary = self.path.with_extension("strec.tmp");
        fs::write(&temporary, bytes)
            .with_context(|| format!("failed to write replay {}", temporary.display()))?;
        if self.path.exists() {
            fs::remove_file(&self.path)
                .with_context(|| format!("failed to replace replay {}", self.path.display()))?;
        }
        fs::rename(&temporary, &self.path)
            .with_context(|| format!("failed to finalize replay {}", self.path.display()))
    }
}

impl Drop for ReplayRecorder {
    fn drop(&mut self) {
        let _ = self.save();
    }
}

pub fn project_fingerprint(loaded: &LoadedManifest) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(loaded.manifest.project.name.as_bytes());
    hash.update(serde_json::to_vec(&loaded.manifest.render)?);
    hash.update(serde_json::to_vec(&loaded.manifest.shader)?);
    hash.update(serde_json::to_vec(&loaded.manifest.uniforms)?);
    hash.update(serde_json::to_vec(&loaded.manifest.passes)?);
    hash.update(serde_json::to_vec(&loaded.manifest.assets)?);

    for (name, expanded) in expand_all(loaded)? {
        hash.update(name.as_bytes());
        hash.update(expanded.text.as_bytes());
        if let Some(vertex) = &expanded.vertex_text {
            hash.update(vertex.as_bytes());
        }
    }
    for asset in &loaded.manifest.assets {
        hash.update(asset.name.as_bytes());
        let path = loaded.root.join(&asset.path);
        hash.update(
            fs::read(&path)
                .with_context(|| format!("failed to fingerprint asset {}", path.display()))?,
        );
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_validation_rejects_unordered_events() {
        let replay = ReplayFile {
            format: REPLAY_FORMAT_VERSION,
            project: "x".into(),
            project_fingerprint: "0".into(),
            width: 1,
            height: 1,
            fps: 60.0,
            invalidated: None,
            frames: vec![
                ReplayFrame {
                    runtime_frame: 0,
                    time: 0.0,
                    time_delta: 0.0,
                    frame_rate: 0.0,
                },
                ReplayFrame {
                    runtime_frame: 1,
                    time: 1.0 / 60.0,
                    time_delta: 1.0 / 60.0,
                    frame_rate: 60.0,
                },
            ],
            events: vec![
                ReplayEvent {
                    frame: 1,
                    action: ReplayAction::Reset,
                },
                ReplayEvent {
                    frame: 0,
                    action: ReplayAction::Reset,
                },
            ],
        };
        assert!(replay.validate().is_err());
    }
}
