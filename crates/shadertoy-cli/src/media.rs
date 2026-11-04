use crate::audio::AudioDriver;
use crate::manifest::{AssetKind, LoadedManifest};
use crate::project::existing_project_file;
use anyhow::{Context, Result, bail};
use shadertoy::Runtime;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const WEBCAM_NAME: &str = "webcam";
pub const WEBCAM_WIDTH: u32 = 320;
pub const WEBCAM_HEIGHT: u32 = 240;

#[derive(Debug, Clone)]
pub struct VideoAsset {
    pub name: String,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub duration: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct MediaInputs {
    videos: Vec<VideoAsset>,
    audio: AudioDriver,
    /// Silent audio only needs one reset per runtime/media pairing.
    audio_cleared: std::cell::Cell<bool>,
}

impl MediaInputs {
    pub fn new_headless(loaded: &LoadedManifest) -> Result<Self> {
        if manifest_uses_webcam(loaded) {
            bail!(
                "webcam inputs are live preview-only; use a deterministic video asset for headless rendering"
            );
        }
        Self::new(loaded)
    }

    pub fn new(loaded: &LoadedManifest) -> Result<Self> {
        let mut videos = Vec::new();
        for asset in loaded
            .manifest
            .assets
            .iter()
            .filter(|asset| asset.kind == AssetKind::Video)
        {
            let path =
                existing_project_file(&loaded.root, &asset.path, "video asset", &asset.name)?;
            let (width, height) = probe_video_dimensions(&path)?;
            let duration = probe_video_duration(&path)?;
            videos.push(VideoAsset {
                name: asset.name.clone(),
                path,
                width,
                height,
                duration,
            });
        }
        let audio = AudioDriver::resolve(
            &loaded.root,
            &loaded.manifest.audio,
            loaded.audio_pinned.then_some(&loaded.manifest.audio),
        )?;
        Ok(Self {
            videos,
            audio,
            audio_cleared: std::cell::Cell::new(false),
        })
    }

    pub fn audio(&self) -> &AudioDriver {
        &self.audio
    }

    pub fn update(&self, runtime: &mut Runtime<'_>, time: f32) -> Result<()> {
        for video in &self.videos {
            let sample_time = video
                .duration
                .filter(|duration| *duration > 0.0)
                .map(|duration| time.rem_euclid(duration))
                .unwrap_or(time);
            let rgba =
                decode_video_frame_rgba8(&video.path, video.width, video.height, sample_time)?;
            runtime.update_texture_rgba8(&video.name, video.width, video.height, &rgba)?;
        }
        if !self.audio.is_silent() {
            self.audio.apply(runtime, time)?;
        } else if !self.audio_cleared.replace(true) {
            runtime.clear_audio_input()?;
        }
        Ok(())
    }
}

pub fn initial_video_frame(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let (width, height) = probe_video_dimensions(path)?;
    let rgba = decode_video_frame_rgba8(path, width, height, 0.0)?;
    Ok((width, height, rgba))
}

pub fn probe_video_dimensions(path: &Path) -> Result<(u32, u32)> {
    let executable = std::env::var_os("SHADERTOY_FFPROBE").unwrap_or_else(|| "ffprobe".into());
    let output = Command::new(executable)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
        ])
        .arg(path)
        .output()
        .with_context(
            || "failed to launch ffprobe; install ffmpeg/ffprobe or set SHADERTOY_FFPROBE",
        )?;
    if !output.status.success() {
        bail!(
            "ffprobe failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let dimensions = String::from_utf8(output.stdout)
        .context("ffprobe returned non-UTF-8 dimensions")?
        .trim()
        .to_string();
    let (width, height) = dimensions
        .split_once('x')
        .with_context(|| format!("ffprobe returned invalid dimensions '{dimensions}'"))?;
    let width = width.parse::<u32>().context("invalid ffprobe width")?;
    let height = height.parse::<u32>().context("invalid ffprobe height")?;
    if width == 0
        || height == 0
        || width > crate::manifest::MAX_RENDER_DIMENSION
        || height > crate::manifest::MAX_RENDER_DIMENSION
    {
        bail!("video dimensions {width}x{height} are outside the supported range");
    }
    Ok((width, height))
}

pub fn probe_video_duration(path: &Path) -> Result<Option<f32>> {
    let executable = std::env::var_os("SHADERTOY_FFPROBE").unwrap_or_else(|| "ffprobe".into());
    let output = Command::new(executable)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .with_context(
            || "failed to launch ffprobe; install ffmpeg/ffprobe or set SHADERTOY_FFPROBE",
        )?;
    if !output.status.success() {
        bail!(
            "ffprobe failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8(output.stdout).context("ffprobe returned non-UTF-8 duration")?;
    let duration = text.trim().parse::<f32>().ok();
    Ok(duration.filter(|duration| duration.is_finite() && *duration > 0.0))
}

pub fn decode_video_frame_rgba8(
    path: &Path,
    width: u32,
    height: u32,
    time: f32,
) -> Result<Vec<u8>> {
    if !time.is_finite() || time < 0.0 {
        bail!("video sample time must be finite and non-negative");
    }
    let executable = std::env::var_os("SHADERTOY_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let output = Command::new(executable)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-ss",
            &format!("{time:.9}"),
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ])
        .output()
        .with_context(|| "failed to launch ffmpeg while decoding a video input")?;
    if !output.status.success() {
        bail!(
            "ffmpeg failed to decode {} at {:.6}s: {}",
            path.display(),
            time,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("video frame size overflow")?;
    if output.stdout.len() != expected {
        bail!(
            "ffmpeg decoded {} bytes for {}x{} video frame; expected {}",
            output.stdout.len(),
            width,
            height,
            expected
        );
    }
    Ok(output.stdout)
}

pub fn manifest_uses_webcam(loaded: &LoadedManifest) -> bool {
    loaded
        .manifest
        .passes
        .iter()
        .flat_map(|pass| &pass.inputs)
        .any(|input| {
            input.source == WEBCAM_NAME
                || matches!(input.kind, Some(crate::manifest::InputKind::Webcam))
        })
}
