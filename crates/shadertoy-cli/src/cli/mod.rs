mod conversions;
use crate::docs;
use crate::ops;
use crate::ops::{
    BlindCreateOptions, BlindJudgeOptions, BlindRevealOptions, ChannelSetOptions, CheckOptions,
    ExperimentOptions, GraphOptions, InspectBufferOptions, InspectMode, InspectStorageOptions,
    InspectStorageType, InspectVisualization, Output, ProfileOptions, RenderAudioOptions,
    RenderFramesOptions, RenderOptions, RenderVideoOptions, ReplayOptions, SweepOptions,
    TestOptions, TraceCaptureOptions, TraceReplayOptions,
};
use crate::preview;
use crate::preview::{PreviewConfig, PreviewTransport};
use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "shadertoy",
    version,
    about = "Agent-friendly ShaderToy project, rendering, inspection, and live-preview CLI"
)]
struct Cli {
    /// Emit machine-readable JSON results and errors on stdout.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new ShaderToy project directory.
    New(NewArgs),
    /// Initialize the current/existing directory as a ShaderToy project.
    Init(InitArgs),
    /// Import a ShaderToy URL or shader id through Camoufox into a local project.
    Import(ImportArgs),
    /// Validate the manifest, graph, assets, and GLSL compilation.
    Check(ProjectPathArgs),
    /// Inspect the resolved pass/resource graph and optionally write Graphviz DOT.
    Graph(GraphArgs),
    /// Build the project into an STTF artifact.
    Build(BuildArgs),
    /// Deterministically render the final image or a named 2D buffer/compute/mesh pass.
    Render(RenderArgs),
    /// Render multiple deterministic frames in one runtime, optionally as a contact sheet.
    RenderFrames(RenderFramesArgs),
    /// Encode a deterministic animation through ffmpeg.
    RenderVideo(RenderVideoArgs),
    /// Render a ShaderToy Sound pass to deterministic stereo PCM WAV.
    RenderAudio(RenderAudioArgs),
    /// Measure per-pass and whole-frame GPU timings plus profiled CPU render-call cost.
    Profile(ProfileArgs),
    /// Render a Cartesian product of custom-uniform values for visual comparison.
    Sweep(SweepArgs),
    /// Run a reproducible multi-source visual/performance experiment.
    Experiment(ExperimentArgs),
    /// Create, judge, and reveal blinded visual comparisons.
    Blind(BlindArgs),
    /// Run deterministic visual and numeric regression tests from [[test]] cases.
    Test(TestArgs),
    /// Reproduce a recorded preview-input timeline.
    Replay(ReplayArgs),
    /// Capture, inspect, and replay self-contained render traces.
    Trace(TraceArgs),
    /// Run a native-rendered live preview web server with hot reload.
    Preview(PreviewArgs),
    /// Inspect project structure progressively, from summary to pass/channel detail.
    Inspect(InspectArgs),
    /// Capture, inspect, or modify resumable feedback-buffer state.
    State(StateArgs),
    /// Safely mutate shader passes in ShaderToy.toml.
    Pass(PassArgs),
    /// Safely mutate iChannel bindings in ShaderToy.toml.
    Channel(ChannelArgs),
    /// Read concise built-in project/agent documentation.
    Docs(DocsArgs),
}

#[derive(Debug, Args)]
struct NewArgs {
    /// Destination directory. Its basename becomes project.name.
    path: PathBuf,
    #[arg(long, value_enum, default_value_t = TemplateArg::Minimal)]
    template: TemplateArg,
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(default_value = ".")]
    path: PathBuf,
    #[arg(long, value_enum, default_value_t = TemplateArg::Minimal)]
    template: TemplateArg,
}

#[derive(Debug, Args)]
struct ImportArgs {
    /// ShaderToy view URL or shader id.
    source: String,
    /// Destination directory. Defaults to a filesystem-safe form of the shader name.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ProjectPathArgs {
    /// Project directory (or any path inside it).
    #[arg(value_name = "PATH", conflicts_with = "project")]
    path: Option<PathBuf>,
    /// Project directory (or any path inside it).
    #[arg(long, value_name = "PATH")]
    project: Option<PathBuf>,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    /// Enable advisory graph/resource lints and fail when any warning is found.
    #[arg(long)]
    pedantic: bool,
}

impl ProjectPathArgs {
    fn resolved(&self) -> PathBuf {
        self.project
            .clone()
            .or_else(|| self.path.clone())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

#[derive(Debug, Args)]
struct GraphArgs {
    /// Project directory (or any path inside it).
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    /// Write the graph as Graphviz DOT.
    #[arg(long, value_name = "PATH")]
    dot: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct BuildArgs {
    /// Project directory (or any path inside it).
    #[arg(value_name = "PATH", conflicts_with = "project")]
    path: Option<PathBuf>,
    /// Project directory (or any path inside it).
    #[arg(long, value_name = "PATH")]
    project: Option<PathBuf>,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    /// Output STTF path. Defaults to target/PROJECT.sttf.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct RenderArgs {
    /// Project directory (or any path inside it).
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    /// Output PNG path. Defaults to target/render.png.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Render/snapshot a named pass instead of the final Image pass.
    #[arg(long)]
    pass: Option<String>,
    /// Overlay a depth-tested wireframe for the named mesh pass.
    #[arg(long, value_name = "PASS")]
    wireframe: Option<String>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    /// Deterministic target iFrame.
    #[arg(long, conflicts_with = "time")]
    frame: Option<i32>,
    /// Deterministic target iTime in seconds.
    #[arg(long, conflicts_with = "frame")]
    time: Option<f32>,
    /// Resume from a previously captured .ststate artifact.
    #[arg(long)]
    state: Option<PathBuf>,
    /// Override a persistent 2D pass immediately before the target frame, e.g. buffer-a=fixture.png.
    #[arg(long = "set-buffer")]
    set_buffers: Vec<String>,
    /// Override a declared custom uniform, e.g. --set wave_height=2.0.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct RenderFramesArgs {
    /// Project directory (or any path inside it).
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    /// Directory for individual PNG frames. Defaults to PROJECT/target/frames.
    #[arg(long)]
    output_dir: Option<PathBuf>,
    /// Also write a contact-sheet PNG containing the requested frames.
    #[arg(long)]
    contact_sheet: Option<PathBuf>,
    /// Contact-sheet column count. Defaults to a near-square layout.
    #[arg(long, requires = "contact_sheet")]
    columns: Option<u32>,
    /// Render/snapshot a named pass instead of the final Image pass.
    #[arg(long)]
    pass: Option<String>,
    /// Overlay a depth-tested wireframe for the named mesh pass.
    #[arg(long, value_name = "PASS")]
    wireframe: Option<String>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    /// Deterministic iFrames to render, e.g. --frames 0,60,120,180.
    #[arg(long, value_delimiter = ',', num_args = 1.., required_unless_present = "range", conflicts_with = "range")]
    frames: Vec<i32>,
    /// Inclusive START:END[:STEP] deterministic frame range.
    #[arg(long, required_unless_present = "frames", conflicts_with = "frames")]
    range: Option<String>,
    /// Override a declared custom uniform for every rendered frame.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct RenderAudioArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Sound pass name. Required only when the project has multiple Sound passes.
    #[arg(long)]
    pass: Option<String>,
    #[arg(long, default_value_t = 10.0)]
    duration: f32,
    #[arg(long, default_value_t = 44_100)]
    sample_rate: u32,
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct RenderVideoArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    /// Encoded output. Extension selects sensible defaults for mp4/webm/gif.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Render/snapshot a named pass instead of final Image.
    #[arg(long)]
    pass: Option<String>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    #[arg(long, default_value_t = 0)]
    start_frame: i32,
    /// Number of output frames.
    #[arg(long, conflicts_with = "duration")]
    frames: Option<u32>,
    /// Output duration in seconds. Defaults to 5 seconds when --frames is omitted.
    #[arg(long, conflicts_with = "frames")]
    duration: Option<f32>,
    /// Optional explicit ffmpeg video codec.
    #[arg(long)]
    codec: Option<String>,
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct ProfileArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    #[arg(long, conflicts_with = "time")]
    frame: Option<i32>,
    #[arg(long, conflicts_with = "frame")]
    time: Option<f32>,
    /// Unmeasured frames rendered before sampling.
    #[arg(long, default_value_t = 3)]
    warmup: u32,
    /// Consecutive measured frames.
    #[arg(long, default_value_t = 20)]
    samples: u32,
    /// Also retire each post-completion timestamp before continuing; slower maximum-isolation diagnostics.
    #[arg(long)]
    sync_per_pass: bool,
    /// Exclude MAD-flagged timing outliers from aggregate statistics while retaining every raw sample in JSON.
    #[arg(long)]
    discard_outliers: bool,
    /// Override a declared custom uniform during profiling.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct SweepArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    /// Directory for variant PNGs. Defaults to PROJECT/target/sweep.
    #[arg(long)]
    output_dir: Option<PathBuf>,
    /// Contact-sheet path. Defaults to OUTPUT_DIR/contact-sheet.png.
    #[arg(long, conflicts_with = "no_contact_sheet")]
    contact_sheet: Option<PathBuf>,
    /// Skip contact-sheet generation.
    #[arg(long, conflicts_with = "blind")]
    no_contact_sheet: bool,
    /// Randomize/anonymize variants as A/B/C and seal the parameter mapping until judged.
    #[arg(long, conflicts_with = "no_contact_sheet")]
    blind: bool,
    /// Contact-sheet column count. Defaults to a near-square layout.
    #[arg(long)]
    columns: Option<u32>,
    /// Render/snapshot a named 2D buffer/compute/mesh pass instead of the final Image pass.
    #[arg(long)]
    pass: Option<String>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    #[arg(long, conflicts_with = "time")]
    frame: Option<i32>,
    #[arg(long, conflicts_with = "frame")]
    time: Option<f32>,
    /// Sweep a declared custom uniform. Scalars use commas; vectors use semicolons between vectors.
    #[arg(long = "set", value_name = "NAME=VALUES", required = true)]
    sweep_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct ExperimentArgs {
    /// Baseline source. Accepts the same forms as blind create.
    #[arg(long)]
    baseline: String,
    /// Candidate source. May be repeated for N-way experiments.
    #[arg(long, required = true)]
    candidate: Vec<String>,
    /// Additional named-neutral variants beyond candidate.
    #[arg(long = "variant")]
    variants: Vec<String>,
    /// Output directory. Defaults to target/experiment.
    #[arg(long)]
    output_dir: Option<PathBuf>,
    /// Deterministic frames for project/STTF sources.
    #[arg(long, value_delimiter = ',', default_value = "0")]
    frames: Vec<i32>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    /// Metrics to compute: rmse,ssim.
    #[arg(long = "metric", value_delimiter = ',', default_value = "rmse,ssim")]
    metrics: Vec<String>,
    /// Randomize/anonymize source ordering and seal the mapping until judged.
    #[arg(long)]
    blind: bool,
    /// Git repository root for git:REF sources.
    #[arg(long)]
    git_root: Option<PathBuf>,
    /// GPU profile samples for project/git sources; 0 disables profiling.
    #[arg(long, default_value_t = 8)]
    profile_samples: u32,
    /// Override a declared custom uniform for every rendered source. May be repeated.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct BlindArgs {
    #[command(subcommand)]
    command: BlindCommand,
}

#[derive(Debug, Subcommand)]
enum BlindCommand {
    /// Create a blinded comparison from images, render directories, ShaderToy projects, STTF builds, or git refs.
    Create(BlindCreateArgs),
    /// Commit a visual preference and reasoning while the source/parameter mapping remains sealed.
    Judge(BlindJudgeArgs),
    /// Reveal the source/parameter mapping after a judgment has been recorded.
    Reveal(BlindRevealArgs),
}

#[derive(Debug, Args)]
struct BlindCreateArgs {
    /// Sources to blind. Paths auto-detect images, image directories, ShaderToy projects, and .sttf builds. Use project:PATH@preset=NAME for a quality preset, or git:REF / git:REF::SUBDIR for revisions.
    #[arg(required = true, num_args = 2..)]
    sources: Vec<String>,
    /// Output directory. Defaults to target/blind-comparison.
    #[arg(long)]
    output_dir: Option<PathBuf>,
    /// Deterministic frames for project/STTF sources.
    #[arg(long, value_delimiter = ',', default_value = "0")]
    frames: Vec<i32>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    /// Git repository root for git:REF sources. Defaults to the containing repository.
    #[arg(long)]
    git_root: Option<PathBuf>,
    /// Override a declared custom uniform for every rendered source. May be repeated.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct BlindJudgeArgs {
    /// blind-session.json path or its containing comparison directory.
    session: PathBuf,
    /// Anonymous variant label to select, e.g. A or B.
    #[arg(long)]
    pick: String,
    /// Reasoning for the preference. Stored before the mapping can be revealed.
    #[arg(
        long,
        required_unless_present = "reason_file",
        conflicts_with = "reason_file"
    )]
    reason: Option<String>,
    /// Read the reasoning from a UTF-8 text/Markdown file.
    #[arg(long, required_unless_present = "reason", conflicts_with = "reason")]
    reason_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct BlindRevealArgs {
    /// blind-session.json path or its containing comparison directory.
    session: PathBuf,
}

#[derive(Debug, Args)]
struct ReplayArgs {
    /// .strec recording produced by preview --record.
    recording: PathBuf,
    #[arg(long, default_value = ".")]
    project: PathBuf,
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[arg(long)]
    pass: Option<String>,
    /// Recorded timeline frame. Defaults to the final captured frame.
    #[arg(long)]
    frame: Option<u64>,
    /// Replay despite a source/manifest/asset fingerprint mismatch.
    #[arg(long)]
    allow_project_changes: bool,
}

#[derive(Debug, Args)]
struct TestArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    /// Rewrite visual reference images from the current deterministic render.
    #[arg(long, conflicts_with = "ci")]
    update: bool,
    /// Run only test names containing this substring.
    #[arg(long)]
    filter: Option<String>,
    /// CI mode: never mutates references and emits stable failure-oriented output.
    #[arg(long)]
    ci: bool,
}

#[derive(Debug, Args)]
struct TraceArgs {
    #[command(subcommand)]
    command: TraceCommand,
}

#[derive(Debug, Subcommand)]
enum TraceCommand {
    /// Capture a deterministic trace bundle.
    Capture(TraceCaptureArgs),
    /// Inspect and integrity-check a trace bundle.
    Inspect { path: PathBuf },
    /// Replay a trace from its bundled STTF and compare against the captured RGB output.
    Replay(TraceReplayArgs),
}

#[derive(Debug, Args)]
struct TraceCaptureArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    /// Trace directory. Must end in .sttrace.
    #[arg(short, long, default_value = "target/trace.sttrace")]
    output: PathBuf,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    #[arg(long, conflicts_with = "time")]
    frame: Option<i32>,
    #[arg(long, conflicts_with = "frame")]
    time: Option<f32>,
    /// Override a declared custom uniform during capture.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
    /// Include PNG and RGBA32F snapshots of every reachable 2D pass output.
    #[arg(long)]
    include_intermediates: bool,
}

#[derive(Debug, Args)]
struct TraceReplayArgs {
    /// .sttrace directory or its trace.json.
    trace: PathBuf,
    /// Optional replayed PNG output.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct PreviewArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    /// Apply a named manifest quality preset.
    #[arg(long)]
    preset: Option<String>,
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 4321)]
    port: u16,
    /// Force opening the browser.
    #[arg(long)]
    open: bool,
    /// Never open the browser automatically.
    #[arg(long)]
    no_open: bool,
    /// Required when binding preview to a non-loopback address.
    #[arg(long)]
    token: Option<String>,
    /// Reset time/buffer state on every successful hot reload.
    #[arg(long)]
    reset_on_reload: bool,
    /// Overlay a depth-tested wireframe for the named mesh pass.
    #[arg(long, value_name = "PASS")]
    wireframe: Option<String>,
    /// Record shader-affecting preview input and exact rendered frame/time markers.
    #[arg(long, value_name = "PATH")]
    record: Option<PathBuf>,
    /// Live frame transport: auto uses raw RGB on loopback and MJPEG otherwise.
    #[arg(long = "preview-transport", value_enum, default_value_t = PreviewTransport::Auto)]
    preview_transport: PreviewTransport,
}

#[derive(Debug, Args)]
struct InspectArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    #[command(subcommand)]
    command: Option<InspectCommand>,
}

#[derive(Debug, Subcommand)]
enum InspectCommand {
    /// Show the complete pass/channel dependency graph.
    Graph,
    /// Inspect one pass.
    Pass { name: String },
    /// Inspect one pass's iChannel bindings.
    Channels { name: String },
    /// Render and inspect a 2D buffer/compute/mesh pass's floating-point contents.
    Buffer(InspectBufferArgs),
    /// Render and inspect a named shader-storage buffer.
    Storage(InspectStorageArgs),
    /// Inspect a .ststate artifact.
    State { path: PathBuf },
}

#[derive(Debug, Args)]
struct InspectBufferArgs {
    name: String,
    /// Render-target index for MRT passes (0 is the primary output).
    #[arg(long, default_value_t = 0)]
    output_index: u8,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    #[arg(long, conflicts_with = "time")]
    frame: Option<i32>,
    #[arg(long, conflicts_with = "frame")]
    time: Option<f32>,
    /// Shader-coordinate pixel (x,y) to print.
    #[arg(long, value_parser = parse_pixel)]
    pixel: Option<(u32, u32)>,
    /// Optional diagnostic PNG.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Optional little-endian raw RGBA32F dump.
    #[arg(long)]
    raw: Option<PathBuf>,
    /// Diagnostic PNG mapping: auto, rgb, signed, or magnitude.
    ///
    /// auto uses rgb for values inside 0..1 and signed otherwise; signed centers
    /// zero at 0.5 using the largest absolute RGB value; magnitude writes
    /// normalized vector length as grayscale.
    #[arg(long, value_enum, default_value_t = InspectVisualizationArg::Auto)]
    visualization: InspectVisualizationArg,
    /// Override a declared custom uniform during inspection.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct InspectStorageArgs {
    name: String,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    #[arg(long, conflicts_with = "time")]
    frame: Option<i32>,
    #[arg(long, conflicts_with = "frame")]
    time: Option<f32>,
    /// Byte offset into the SSBO.
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Number of typed values to print.
    #[arg(long, default_value_t = 16)]
    count: usize,
    #[arg(long = "type", value_enum, default_value_t = InspectStorageTypeArg::F32)]
    value_type: InspectStorageTypeArg,
    /// Optional complete raw SSBO dump.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Override a declared custom uniform during inspection.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct StateArgs {
    #[command(subcommand)]
    command: StateCommand,
}

#[derive(Debug, Subcommand)]
enum StateCommand {
    /// Render deterministically to a point and capture lossless persistent buffer state.
    Capture(StateCaptureArgs),
    /// Inspect .ststate metadata without loading OpenGL.
    Inspect { path: PathBuf },
    /// Replace one or more buffers inside an existing state using exact-size images.
    Set(StateSetArgs),
    /// Replace one or more captured SSBOs using exact-size binary files.
    SetStorage(StateSetStorageArgs),
}

#[derive(Debug, Args)]
struct StateCaptureArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    #[arg(short, long)]
    output: PathBuf,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    fps: Option<f32>,
    #[arg(long, conflicts_with = "time")]
    frame: Option<i32>,
    #[arg(long, conflicts_with = "frame")]
    time: Option<f32>,
    /// Include named shader-storage buffers in the resumable state.
    #[arg(long)]
    include_storage: bool,
    /// Override a declared custom uniform while producing the captured state.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set_uniforms: Vec<String>,
}

#[derive(Debug, Args)]
struct StateSetArgs {
    input: PathBuf,
    /// BUFFER=IMAGE assignments.
    #[arg(required = true)]
    assignments: Vec<String>,
    #[arg(short, long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct StateSetStorageArgs {
    input: PathBuf,
    /// STORAGE=BINARY assignments.
    #[arg(required = true)]
    assignments: Vec<String>,
    #[arg(short, long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct PassArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    #[command(subcommand)]
    command: PassCommand,
}

#[derive(Debug, Subcommand)]
enum PassCommand {
    /// Add an offscreen buffer/cubemap/compute pass and create a source stub if needed.
    Add {
        name: String,
        #[arg(long, value_enum, default_value_t = PassKindArg::Buffer)]
        kind: PassKindArg,
        /// Source path relative to the project root.
        #[arg(long)]
        source: Option<PathBuf>,
    },
    /// Remove a non-final pass. Source files are intentionally left on disk.
    Remove {
        name: String,
        /// Also remove channel bindings that consume this pass.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Args)]
struct ChannelArgs {
    #[arg(long, default_value = ".")]
    project: PathBuf,
    #[command(subcommand)]
    command: ChannelCommand,
}

#[derive(Debug, Subcommand)]
enum ChannelCommand {
    /// Set/replace one iChannel binding.
    Set {
        pass: String,
        /// iChannel index, 0..15.
        channel: u8,
        source: String,
        #[arg(long, value_enum)]
        kind: Option<InputKindArg>,
        /// Render-target index, or "depth" for a depth-enabled mesh pass.
        #[arg(long, default_value = "0")]
        output: String,
        #[arg(long)]
        previous: bool,
        #[arg(long, value_enum, default_value_t = FilterArg::Linear)]
        filter: FilterArg,
        #[arg(long, value_enum, default_value_t = WrapArg::Repeat)]
        wrap: WrapArg,
    },
    /// Remove one iChannel binding.
    Remove {
        pass: String,
        /// iChannel index, 0..15.
        channel: u8,
    },
}

#[derive(Debug, Args)]
struct DocsArgs {
    /// agent, project, import, manifest, passes, glsl, assets, buffers, channels, state, sweep, blind, experiment, profile, test, trace, graph, or preview.
    #[arg(default_value = "agent")]
    topic: String,
    /// Print the exact JSON Schema for ShaderToy.toml (manifest topic only).
    #[arg(long)]
    schema: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum InspectVisualizationArg {
    #[default]
    Auto,
    Rgb,
    Signed,
    Magnitude,
}

impl From<InspectVisualizationArg> for InspectVisualization {
    fn from(value: InspectVisualizationArg) -> Self {
        match value {
            InspectVisualizationArg::Auto => Self::Auto,
            InspectVisualizationArg::Rgb => Self::Rgb,
            InspectVisualizationArg::Signed => Self::Signed,
            InspectVisualizationArg::Magnitude => Self::Magnitude,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum InspectStorageTypeArg {
    Bytes,
    U32,
    I32,
    #[default]
    F32,
}

impl From<InspectStorageTypeArg> for InspectStorageType {
    fn from(value: InspectStorageTypeArg) -> Self {
        match value {
            InspectStorageTypeArg::Bytes => Self::Bytes,
            InspectStorageTypeArg::U32 => Self::U32,
            InspectStorageTypeArg::I32 => Self::I32,
            InspectStorageTypeArg::F32 => Self::F32,
        }
    }
}

fn parse_pixel(value: &str) -> std::result::Result<(u32, u32), String> {
    let (x, y) = value
        .split_once(',')
        .ok_or_else(|| "pixel must be formatted as X,Y".to_string())?;
    let x = x
        .parse::<u32>()
        .map_err(|_| "pixel X must be an unsigned integer".to_string())?;
    let y = y
        .parse::<u32>()
        .map_err(|_| "pixel Y must be an unsigned integer".to_string())?;
    Ok((x, y))
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum TemplateArg {
    #[default]
    Minimal,
    Multipass,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum PassKindArg {
    #[default]
    Buffer,
    Mesh,
    Cubemap,
    Compute,
    Sound,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InputKindArg {
    Pass,
    Texture,
    Keyboard,
    Music,
    Video,
    Webcam,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum FilterArg {
    Mipmap,
    #[default]
    Linear,
    Nearest,
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum WrapArg {
    Clamp,
    #[default]
    Repeat,
}

pub fn run() -> ExitCode {
    let args = std::env::args_os().collect::<Vec<_>>();
    let json_requested = args.iter().skip(1).any(|arg| arg == "--json");
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = if error.use_stderr() { 2 } else { 0 };
            if json_requested && error.use_stderr() {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "ok": false,
                        "error": error.to_string(),
                        "kind": format!("{:?}", error.kind()),
                    }))
                    .expect("clap error JSON serialization cannot fail")
                );
            } else {
                let _ = error.print();
            }
            return ExitCode::from(exit_code);
        }
    };
    let json_mode = cli.json;

    let result = dispatch(cli.command, json_mode);
    match result {
        Ok(Some(output)) => {
            let success = output
                .json
                .get("ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            emit(output, json_mode);
            if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(error) => {
            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "ok": false,
                        "error": format!("{error:#}"),
                    }))
                    .expect("error JSON serialization cannot fail")
                );
            } else {
                eprintln!("error: {error:#}");
            }
            ExitCode::from(1)
        }
    }
}

fn dispatch(command: Command, json_mode: bool) -> Result<Option<Output>> {
    let output = match command {
        Command::New(args) => ops::new_project(&args.path, args.template.into())?,
        Command::Init(args) => ops::init_project(&args.path, args.template.into())?,
        Command::Import(args) => ops::import_project(&args.source, args.output.as_deref())?,
        Command::Check(args) => ops::check_project(&CheckOptions {
            project: args.resolved(),
            preset: args.preset,
            pedantic: args.pedantic,
        })?,
        Command::Graph(args) => ops::graph_project(&GraphOptions {
            project: args.project,
            preset: args.preset,
            dot: args.dot,
        })?,
        Command::Build(args) => {
            let project = args
                .project
                .clone()
                .or_else(|| args.path.clone())
                .unwrap_or_else(|| PathBuf::from("."));
            ops::build_project(&project, args.output.as_deref(), args.preset.as_deref())?
        }
        Command::Render(args) => ops::render_project(&RenderOptions {
            project: args.project,
            preset: args.preset,
            output: args.output,
            pass: args.pass,
            wireframe: args.wireframe,
            width: args.width,
            height: args.height,
            fps: args.fps,
            frame: args.frame,
            time: args.time,
            state: args.state,
            set_buffers: args.set_buffers,
            set_uniforms: args.set_uniforms,
        })?,
        Command::RenderFrames(args) => ops::render_frames_project(&RenderFramesOptions {
            project: args.project,
            preset: args.preset,
            output_dir: args.output_dir,
            contact_sheet: args.contact_sheet,
            columns: args.columns,
            pass: args.pass,
            wireframe: args.wireframe,
            width: args.width,
            height: args.height,
            fps: args.fps,
            frames: args.frames,
            range: args.range,
            set_uniforms: args.set_uniforms,
        })?,
        Command::RenderAudio(args) => ops::render_audio_project(&RenderAudioOptions {
            project: args.project,
            output: args.output,
            pass: args.pass,
            duration: args.duration,
            sample_rate: args.sample_rate,
            set_uniforms: args.set_uniforms,
        })?,
        Command::RenderVideo(args) => ops::render_video_project(&RenderVideoOptions {
            project: args.project,
            preset: args.preset,
            output: args.output,
            pass: args.pass,
            width: args.width,
            height: args.height,
            fps: args.fps,
            start_frame: args.start_frame,
            frames: args.frames,
            duration: args.duration,
            codec: args.codec,
            set_uniforms: args.set_uniforms,
        })?,
        Command::Profile(args) => ops::profile_project(&ProfileOptions {
            project: args.project,
            preset: args.preset,
            width: args.width,
            height: args.height,
            fps: args.fps,
            frame: args.frame,
            time: args.time,
            warmup: args.warmup,
            samples: args.samples,
            sync_per_pass: args.sync_per_pass,
            discard_outliers: args.discard_outliers,
            set_uniforms: args.set_uniforms,
        })?,
        Command::Sweep(args) => ops::sweep_project(&SweepOptions {
            project: args.project,
            preset: args.preset,
            output_dir: args.output_dir,
            contact_sheet: args.contact_sheet,
            no_contact_sheet: args.no_contact_sheet,
            blind: args.blind,
            columns: args.columns,
            pass: args.pass,
            width: args.width,
            height: args.height,
            fps: args.fps,
            frame: args.frame,
            time: args.time,
            sweep_uniforms: args.sweep_uniforms,
        })?,
        Command::Experiment(args) => {
            let mut candidates = args.candidate;
            candidates.extend(args.variants);
            ops::run_experiment(&ExperimentOptions {
                baseline: args.baseline,
                candidates,
                output_dir: args.output_dir,
                frames: args.frames,
                width: args.width,
                height: args.height,
                fps: args.fps,
                metrics: args.metrics,
                blind: args.blind,
                git_root: args.git_root,
                profile_samples: args.profile_samples,
                set_uniforms: args.set_uniforms,
            })?
        }
        Command::Blind(args) => match args.command {
            BlindCommand::Create(args) => ops::create_blind_comparison(&BlindCreateOptions {
                sources: args.sources,
                output_dir: args.output_dir,
                frames: args.frames,
                width: args.width,
                height: args.height,
                fps: args.fps,
                git_root: args.git_root,
                set_uniforms: args.set_uniforms,
            })?,
            BlindCommand::Judge(args) => ops::judge_blind(&BlindJudgeOptions {
                session: args.session,
                pick: args.pick,
                reason: args.reason,
                reason_file: args.reason_file,
            })?,
            BlindCommand::Reveal(args) => ops::reveal_blind(&BlindRevealOptions {
                session: args.session,
            })?,
        },
        Command::Test(args) => ops::test_project(&TestOptions {
            project: args.project,
            preset: args.preset,
            update: args.update,
            filter: args.filter,
            ci: args.ci,
        })?,
        Command::Replay(args) => ops::replay_project(&ReplayOptions {
            project: args.project,
            recording: args.recording,
            output: args.output,
            pass: args.pass,
            frame: args.frame,
            allow_project_changes: args.allow_project_changes,
        })?,
        Command::Trace(args) => match args.command {
            TraceCommand::Capture(args) => ops::capture_trace(&TraceCaptureOptions {
                project: args.project,
                preset: args.preset,
                output: args.output,
                width: args.width,
                height: args.height,
                fps: args.fps,
                frame: args.frame,
                time: args.time,
                set_uniforms: args.set_uniforms,
                include_intermediates: args.include_intermediates,
            })?,
            TraceCommand::Inspect { path } => ops::inspect_trace(&path)?,
            TraceCommand::Replay(args) => ops::replay_trace(&TraceReplayOptions {
                trace: args.trace,
                output: args.output,
            })?,
        },
        Command::Preview(args) => {
            preview::run(
                PreviewConfig {
                    project: args.project,
                    preset: args.preset,
                    host: args.host,
                    port: args.port,
                    open: args.open,
                    no_open: args.no_open,
                    token: args.token,
                    preserve_reload_state: !args.reset_on_reload,
                    wireframe: args.wireframe,
                    record: args.record,
                    transport: args.preview_transport,
                },
                json_mode,
            )?;
            return Ok(None);
        }
        Command::Inspect(args) => match args.command {
            None => ops::inspect_project(&args.project, InspectMode::Summary)?,
            Some(InspectCommand::Graph) => ops::inspect_project(&args.project, InspectMode::Graph)?,
            Some(InspectCommand::Pass { name }) => {
                ops::inspect_project(&args.project, InspectMode::Pass(name))?
            }
            Some(InspectCommand::Channels { name }) => {
                ops::inspect_project(&args.project, InspectMode::Channels(name))?
            }
            Some(InspectCommand::Buffer(buffer)) => ops::inspect_buffer(&InspectBufferOptions {
                project: args.project,
                pass: buffer.name,
                output_index: buffer.output_index,
                width: buffer.width,
                height: buffer.height,
                fps: buffer.fps,
                frame: buffer.frame,
                time: buffer.time,
                pixel: buffer.pixel,
                output: buffer.output,
                raw: buffer.raw,
                visualization: buffer.visualization.into(),
                set_uniforms: buffer.set_uniforms,
            })?,
            Some(InspectCommand::Storage(storage)) => {
                ops::inspect_storage(&InspectStorageOptions {
                    project: args.project,
                    name: storage.name,
                    width: storage.width,
                    height: storage.height,
                    fps: storage.fps,
                    frame: storage.frame,
                    time: storage.time,
                    offset: storage.offset,
                    count: storage.count,
                    value_type: storage.value_type.into(),
                    output: storage.output,
                    set_uniforms: storage.set_uniforms,
                })?
            }
            Some(InspectCommand::State { path }) => ops::inspect_state(&path)?,
        },
        Command::State(args) => match args.command {
            StateCommand::Capture(args) => ops::capture_state(
                &args.project,
                &args.output,
                args.width,
                args.height,
                args.fps,
                args.frame,
                args.time,
                args.include_storage,
                &args.set_uniforms,
            )?,
            StateCommand::Inspect { path } => ops::inspect_state(&path)?,
            StateCommand::Set(args) => {
                ops::set_state_buffers(&args.input, &args.output, &args.assignments)?
            }
            StateCommand::SetStorage(args) => {
                ops::set_state_storage(&args.input, &args.output, &args.assignments)?
            }
        },
        Command::Pass(args) => match args.command {
            PassCommand::Add { name, kind, source } => {
                ops::add_pass(&args.project, &name, kind.into(), source.as_deref())?
            }
            PassCommand::Remove { name, force } => ops::remove_pass(&args.project, &name, force)?,
        },
        Command::Channel(args) => match args.command {
            ChannelCommand::Set {
                pass,
                channel,
                source,
                kind,
                output,
                previous,
                filter,
                wrap,
            } => ops::set_channel(
                &args.project,
                &ChannelSetOptions {
                    pass,
                    channel,
                    source,
                    kind: kind.map(Into::into),
                    output: parse_pass_output(&output)?,
                    previous,
                    filter: filter.into(),
                    wrap: wrap.into(),
                },
            )?,
            ChannelCommand::Remove { pass, channel } => {
                ops::remove_channel(&args.project, &pass, channel)?
            }
        },
        Command::Docs(args) => {
            let text = docs::topic(&args.topic, args.schema)?;
            Output {
                human: text.clone(),
                json: json!({
                    "ok": true,
                    "topic": args.topic,
                    "schema": args.schema,
                    "text": text,
                }),
            }
        }
    };
    Ok(Some(output))
}

fn parse_pass_output(value: &str) -> Result<crate::manifest::PassOutput> {
    if value.eq_ignore_ascii_case("depth") {
        return Ok(crate::manifest::PassOutput::Named(
            crate::manifest::NamedPassOutput::Depth,
        ));
    }
    let index = value.parse::<u8>().map_err(|_| {
        anyhow::anyhow!("pass output must be an integer in 0..255 or 'depth' (got '{value}')")
    })?;
    Ok(crate::manifest::PassOutput::Index(index))
}

fn emit(output: Output, json_mode: bool) {
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&output.json)
                .expect("command output JSON serialization cannot fail")
        );
    } else {
        println!("{}", output.human);
    }
}
