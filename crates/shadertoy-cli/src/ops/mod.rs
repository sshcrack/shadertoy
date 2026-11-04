mod audio_probe;
mod blind;
mod blind_create;
mod experiment;
mod graph;
mod images;
mod importer;
mod inspect;
mod mutate;
mod profile;
mod project;
mod regression;
mod render;
mod replay;
mod sound;
mod state;
mod sweep;
mod trace;
mod video;

use crate::manifest::{LoadedManifest, PassKind};
use crate::project::{build_native_project, ensure_source_files_exist};
use crate::scaffold::{Template, create_project};
use crate::state::StateFile;
use ::image::ImageReader;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use shadertoy::{HeadlessContext, RgbImage, Runtime};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use audio_probe::{AudioProbeOptions, probe_audio};
pub use blind::{judge_blind, reveal_blind};
pub use blind_create::create_blind_comparison;
pub use experiment::{ExperimentOptions, run_experiment};
pub use graph::{GraphOptions, graph_project};
pub use images::{preview_jpeg_bytes, preview_png_bytes, preview_raw_rgb_bytes};
pub use importer::import_project;
pub use inspect::{inspect_buffer, inspect_project, inspect_storage};
pub use mutate::{ChannelSetOptions, add_pass, remove_channel, remove_pass, set_channel};
pub use profile::profile_project;
pub use project::{build_project, check_project, init_project, new_project};
pub use regression::test_project;
pub(crate) use render::apply_wireframe;
pub use render::{render_frames_project, render_project};
pub use replay::replay_project;
pub use sound::render_audio_project;
pub use state::{capture_state, inspect_state, set_state_buffers, set_state_storage};
pub use sweep::sweep_project;
pub use trace::{
    TraceCaptureOptions, TraceReplayOptions, capture_trace, inspect_trace, replay_trace,
};
pub use video::render_video_project;

use images::{BufferOverride, flip_rgba_rows, load_overrides, save_rgb_png, split_assignment};
use render::{render_from_zero, resolve_target_frame, validate_dimensions, validate_fps};

#[derive(Debug)]
pub struct Output {
    pub human: String,
    pub json: Value,
}

#[derive(Debug, Clone)]
pub struct CheckOptions {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub pedantic: bool,
}

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub output: Option<PathBuf>,
    pub pass: Option<String>,
    pub wireframe: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub frame: Option<i32>,
    pub time: Option<f32>,
    pub state: Option<PathBuf>,
    pub set_buffers: Vec<String>,
    pub set_uniforms: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ReplayOptions {
    pub project: PathBuf,
    pub recording: PathBuf,
    pub output: Option<PathBuf>,
    pub pass: Option<String>,
    pub frame: Option<u64>,
    pub allow_project_changes: bool,
}

#[derive(Debug, Clone)]
pub struct TestOptions {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub update: bool,
    pub filter: Option<String>,
    pub ci: bool,
}

#[derive(Debug, Clone)]
pub struct ProfileOptions {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub frame: Option<i32>,
    pub time: Option<f32>,
    pub warmup: u32,
    pub samples: u32,
    pub sync_per_pass: bool,
    pub discard_outliers: bool,
    pub set_uniforms: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BlindCreateOptions {
    pub sources: Vec<String>,
    pub output_dir: Option<PathBuf>,
    pub frames: Vec<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub git_root: Option<PathBuf>,
    pub set_uniforms: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BlindJudgeOptions {
    pub session: PathBuf,
    pub pick: String,
    pub reason: Option<String>,
    pub reason_file: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct BlindRevealOptions {
    pub session: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SweepOptions {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub contact_sheet: Option<PathBuf>,
    pub no_contact_sheet: bool,
    pub blind: bool,
    pub columns: Option<u32>,
    pub pass: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub frame: Option<i32>,
    pub time: Option<f32>,
    pub sweep_uniforms: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RenderAudioOptions {
    pub project: PathBuf,
    pub output: Option<PathBuf>,
    pub pass: Option<String>,
    pub duration: f32,
    pub sample_rate: u32,
    pub set_uniforms: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RenderVideoOptions {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub output: Option<PathBuf>,
    pub pass: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub start_frame: i32,
    pub frames: Option<u32>,
    pub duration: Option<f32>,
    pub codec: Option<String>,
    pub set_uniforms: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RenderFramesOptions {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub contact_sheet: Option<PathBuf>,
    pub columns: Option<u32>,
    pub pass: Option<String>,
    pub wireframe: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub frames: Vec<i32>,
    pub range: Option<String>,
    pub set_uniforms: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum InspectVisualization {
    Auto,
    Rgb,
    Signed,
    Magnitude,
}

#[derive(Debug, Clone)]
pub struct InspectBufferOptions {
    pub project: PathBuf,
    pub pass: String,
    pub pass_output: crate::manifest::PassOutput,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub frame: Option<i32>,
    pub time: Option<f32>,
    pub pixel: Option<(u32, u32)>,
    pub output: Option<PathBuf>,
    pub raw: Option<PathBuf>,
    pub visualization: InspectVisualization,
    pub set_uniforms: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum InspectStorageType {
    Bytes,
    U32,
    I32,
    F32,
}

#[derive(Debug, Clone)]
pub struct InspectStorageOptions {
    pub project: PathBuf,
    pub name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub frame: Option<i32>,
    pub time: Option<f32>,
    pub offset: usize,
    pub count: usize,
    pub value_type: InspectStorageType,
    pub output: Option<PathBuf>,
    pub set_uniforms: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum InspectMode {
    Summary,
    Graph,
    Pass(String),
    Channels(String),
}
