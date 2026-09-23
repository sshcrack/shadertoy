use anyhow::{Context, Result, bail};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::uniforms::{UniformDefinition, UniformValue, validate_definitions};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const MANIFEST_NAME: &str = "ShaderToy.toml";
pub const FORMAT_VERSION: u32 = 1;
pub const MAX_RENDER_DIMENSION: u32 = 16384;
pub const MAX_RENDER_FPS: f32 = 1000.0;
pub const MAX_INPUT_CHANNEL: u8 = 15;
const RESERVED_INPUT_NAMES: [&str; 3] = ["keyboard", "music", "webcam"];

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Project manifest format version. Version 1 is the only supported value.
    pub format: u32,
    pub project: ProjectSection,
    #[serde(default)]
    pub render: RenderSection,
    #[serde(default)]
    pub shader: ShaderSection,
    #[serde(default, rename = "uniform", skip_serializing_if = "Vec::is_empty")]
    pub uniforms: Vec<UniformDefinition>,
    #[serde(default, rename = "asset", skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<Asset>,
    #[serde(rename = "pass")]
    pub passes: Vec<Pass>,
    #[serde(default, rename = "test", skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<TestCase>,
    /// Named quality/configuration presets applied on top of this manifest.
    #[serde(default, rename = "preset", skip_serializing_if = "BTreeMap::is_empty")]
    pub presets: BTreeMap<String, Preset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectSection {
    /// Human-readable project name.
    pub name: String,
    /// Original ShaderToy author, when this project was imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Original ShaderToy description, when this project was imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Canonical source URL for imported projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// ShaderToy shader id for imported projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RenderSection {
    /// Default render/preview width in pixels.
    pub width: u32,
    /// Default render/preview height in pixels.
    pub height: u32,
    /// Fixed timestep frame rate used by deterministic rendering.
    pub fps: f32,
    /// Representative time used by `shadertoy render` when no frame/time is supplied.
    pub preview_time: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ShaderSection {
    /// Project-relative directories searched after the including file's directory for quoted #include paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_dirs: Vec<String>,
}

impl Default for RenderSection {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            fps: 60.0,
            preview_time: 1.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Preset {
    /// Scale applied to the manifest's default output width/height.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_scale: Option<f32>,
    /// Overrides keyed by pass name, e.g. [preset.low.pass.surface].
    #[serde(default, rename = "pass", skip_serializing_if = "BTreeMap::is_empty")]
    pub passes: BTreeMap<String, PresetPass>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PresetPass {
    /// Fixed pass width override. Must be paired with height.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Fixed pass height override. Must be paired with width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Compute dispatch count override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    /// Compute local workgroup override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_size: Option<[u32; 3]>,
    /// Mesh grid size override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh_size: Option<[u32; 2]>,
    /// Mesh instance-count override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh_instances: Option<u32>,
}

fn default_test_tolerance() -> f32 {
    0.002
}

fn is_zero_f32(value: &f32) -> bool {
    *value == 0.0
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestStorageAssertion {
    /// Shared SSBO resource name.
    pub name: String,
    /// Project-relative binary fixture compared byte-for-byte with the SSBO contents.
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestCase {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<f32>,
    /// Additional deterministic frames. Mutually exclusive with frame/time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Output-resolution matrix. Mutually exclusive with width/height.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolutions: Vec<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default = "default_test_tolerance")]
    pub tolerance: f32,
    #[serde(default)]
    pub assert_no_nan: bool,
    #[serde(default)]
    pub assert_no_inf: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_range: Option<[f32; 2]>,
    /// Re-run every matrix entry from a fresh runtime and require identical output.
    #[serde(default)]
    pub assert_deterministic: bool,
    /// Require a fixed-size buffer/compute/mesh pass to be unchanged by output resolution.
    #[serde(default)]
    pub assert_resolution_independent: bool,
    /// Absolute tolerance for raw floating-point comparisons.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub raw_tolerance: f32,
    /// Per-test custom uniform overrides.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub uniforms: BTreeMap<String, UniformValue>,
    /// Render a second deterministic variant with these uniforms and compare it with the primary uniforms.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reference_uniforms: BTreeMap<String, UniformValue>,
    /// Minimum normalized RGB RMSE required against a reference image or reference uniforms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_rmse: Option<f32>,
    /// Maximum normalized RGB RMSE allowed against a reference image or reference uniforms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rmse: Option<f32>,
    /// Maximum summed attributed GPU pass time for each rendered test variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gpu_ms: Option<f32>,
    /// Per-pass GPU timing budgets in milliseconds.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub max_pass_gpu_ms: BTreeMap<String, f32>,
    /// Exact SSBO fixture assertions evaluated after each rendered variant.
    #[serde(default, rename = "storage", skip_serializing_if = "Vec::is_empty")]
    pub storage_assertions: Vec<TestStorageAssertion>,
    /// Serialize persistent pass/SSBO state, reload it into a fresh runtime, and verify exact round-trip contents.
    #[serde(default)]
    pub assert_state_roundtrip: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PassKind {
    Image,
    Buffer,
    Mesh,
    Cubemap,
    Compute,
    Sound,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MeshTopology {
    #[default]
    Grid,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MeshCull {
    #[default]
    None,
    Back,
    Front,
}

fn default_mesh_instances() -> u32 {
    1
}

fn default_mesh_depth() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshConfig {
    #[serde(default)]
    pub topology: MeshTopology,
    /// Grid vertices per instance.
    pub size: [u32; 2],
    #[serde(default = "default_mesh_instances")]
    pub instances: u32,
    #[serde(default = "default_mesh_depth")]
    pub depth: bool,
    #[serde(default)]
    pub cull: MeshCull,
    #[serde(default)]
    pub clear: [f32; 4],
}

impl MeshConfig {
    pub fn vertex_count(&self) -> u64 {
        u64::from(self.size[0]) * u64::from(self.size[1]) * u64::from(self.instances)
    }

    pub fn triangle_count(&self) -> u64 {
        u64::from(self.size[0] - 1) * u64::from(self.size[1] - 1) * 2 * u64::from(self.instances)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RenderFormat {
    R32f,
    Rg32f,
    Rgba16f,
    #[default]
    Rgba32f,
}

fn default_iterations() -> u32 {
    1
}

fn is_default_iterations(value: &u32) -> bool {
    *value == 1
}

fn is_default_render_format(value: &RenderFormat) -> bool {
    *value == RenderFormat::Rgba32f
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StorageBuffer {
    /// Shader storage binding index used by an explicit std430 binding.
    pub binding: u32,
    /// Shared resource name. Bindings with the same name share persistent GPU storage across passes.
    pub name: String,
    /// Persistent storage size in bytes.
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pass {
    /// Unique pass name. Names are used by inspect/render/debug commands.
    pub name: String,
    pub kind: PassKind,
    /// GLSL fragment source path relative to the project root.
    pub source: String,
    /// GLSL vertex source for mesh passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex: Option<String>,
    /// Rasterization configuration for mesh passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshConfig>,
    /// Optional fixed width for a buffer/compute/mesh pass. Must be paired with height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Optional fixed height for a buffer/compute/mesh pass. Must be paired with width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Floating-point render-target format for buffer/compute/mesh outputs.
    #[serde(default, skip_serializing_if = "is_default_render_format")]
    pub format: RenderFormat,
    /// Additional render targets beyond output 0. Indices are 1-based after the primary format.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_outputs: Vec<RenderFormat>,
    /// Number of compute dispatches per rendered frame. iIteration is 0-based.
    #[serde(
        default = "default_iterations",
        skip_serializing_if = "is_default_iterations"
    )]
    pub iterations: u32,
    /// Compute local workgroup dimensions. Defaults to [8, 8, 1]; Z must be 1 for the 2D mainCompute entrypoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_size: Option<[u32; 3]>,
    /// Persistent shader-storage buffers bound for this pass.
    #[serde(default, rename = "storage", skip_serializing_if = "Vec::is_empty")]
    pub storage: Vec<StorageBuffer>,
    #[serde(default, rename = "input", skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<Input>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InputKind {
    Pass,
    Texture,
    Cubemap,
    Volume,
    Keyboard,
    Music,
    Video,
    Webcam,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NamedPassOutput {
    Depth,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(untagged)]
pub enum PassOutput {
    Index(u8),
    Named(NamedPassOutput),
}

impl Default for PassOutput {
    fn default() -> Self {
        Self::Index(0)
    }
}

impl PassOutput {
    pub fn color_index(self) -> Option<u8> {
        match self {
            Self::Index(index) => Some(index),
            Self::Named(NamedPassOutput::Depth) => None,
        }
    }

    pub fn is_primary(&self) -> bool {
        *self == Self::Index(0)
    }

    pub fn is_depth(&self) -> bool {
        *self == Self::Named(NamedPassOutput::Depth)
    }
}

impl std::fmt::Display for PassOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Index(index) => index.fmt(formatter),
            Self::Named(NamedPassOutput::Depth) => formatter.write_str("depth"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FrameRef {
    #[default]
    Current,
    Previous,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Filter {
    Mipmap,
    #[default]
    Linear,
    Nearest,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Wrap {
    Clamp,
    #[default]
    Repeat,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Input channel index. Must be between 0 and 15.
    #[schemars(range(max = 15))]
    pub channel: u8,
    /// Pass/asset name, or the reserved names "keyboard" and "music".
    pub source: String,
    /// Optional explicit source kind. Normally inferred from the source name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<InputKind>,
    /// Render-target selection when consuming another pass. Output 0 is the
    /// primary color target; depth-enabled mesh passes also expose "depth".
    #[serde(default, skip_serializing_if = "PassOutput::is_primary")]
    pub output: PassOutput,
    #[serde(default)]
    pub frame: FrameRef,
    /// Per-input texture interpolation/minification mode.
    /// nearest = exact texel sampling, linear = bilinear interpolation,
    /// mipmap = trilinear minification with linear magnification.
    #[serde(default)]
    pub filter: Filter,
    /// Per-input texture coordinate addressing mode.
    #[serde(default)]
    pub wrap: Wrap,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AssetKind {
    Texture,
    Cubemap,
    Volume,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    /// Unique asset name used by pass inputs.
    pub name: String,
    /// Asset decoder/layout selected for this file. Cubemap assets are horizontal six-face strips; volume assets use the ShaderToy 20-byte binary header documented by `shadertoy docs assets`.
    pub kind: AssetKind,
    /// Asset file path relative to the project root. See `shadertoy docs assets` for cubemap and volume on-disk layouts.
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct LoadedManifest {
    pub root: PathBuf,
    pub manifest: Manifest,
}

impl Manifest {
    pub fn minimal(name: impl Into<String>) -> Self {
        Self {
            format: FORMAT_VERSION,
            project: ProjectSection {
                name: name.into(),
                author: None,
                description: None,
                source_url: None,
                source_id: None,
            },
            render: RenderSection::default(),
            shader: ShaderSection::default(),
            uniforms: Vec::new(),
            assets: Vec::new(),
            passes: vec![Pass {
                name: "image".into(),
                kind: PassKind::Image,
                source: "shaders/image.frag".into(),
                vertex: None,
                mesh: None,
                width: None,
                height: None,
                format: RenderFormat::default(),
                extra_outputs: Vec::new(),
                iterations: 1,
                local_size: None,
                storage: Vec::new(),
                inputs: Vec::new(),
            }],
            tests: Vec::new(),
            presets: BTreeMap::new(),
        }
    }

    pub fn multipass(name: impl Into<String>) -> Self {
        Self {
            format: FORMAT_VERSION,
            project: ProjectSection {
                name: name.into(),
                author: None,
                description: None,
                source_url: None,
                source_id: None,
            },
            render: RenderSection::default(),
            shader: ShaderSection::default(),
            uniforms: Vec::new(),
            assets: Vec::new(),
            passes: vec![
                Pass {
                    name: "buffer-a".into(),
                    kind: PassKind::Buffer,
                    source: "shaders/buffer-a.frag".into(),
                    vertex: None,
                    mesh: None,
                    width: None,
                    height: None,
                    format: RenderFormat::default(),
                    extra_outputs: Vec::new(),
                    iterations: 1,
                    local_size: None,
                    storage: Vec::new(),
                    inputs: vec![Input {
                        channel: 0,
                        source: "buffer-a".into(),
                        kind: Some(InputKind::Pass),
                        output: PassOutput::Index(0),
                        frame: FrameRef::Previous,
                        filter: Filter::Linear,
                        wrap: Wrap::Clamp,
                    }],
                },
                Pass {
                    name: "image".into(),
                    kind: PassKind::Image,
                    source: "shaders/image.frag".into(),
                    vertex: None,
                    mesh: None,
                    width: None,
                    height: None,
                    format: RenderFormat::default(),
                    extra_outputs: Vec::new(),
                    iterations: 1,
                    local_size: None,
                    storage: Vec::new(),
                    inputs: vec![Input {
                        channel: 0,
                        source: "buffer-a".into(),
                        kind: Some(InputKind::Pass),
                        output: PassOutput::Index(0),
                        frame: FrameRef::Current,
                        filter: Filter::Linear,
                        wrap: Wrap::Clamp,
                    }],
                },
            ],
            tests: Vec::new(),
            presets: BTreeMap::new(),
        }
    }

    pub fn validate_structure(&self) -> Result<()> {
        if self.format != FORMAT_VERSION {
            bail!(
                "unsupported ShaderToy.toml format {}; supported format is {}",
                self.format,
                FORMAT_VERSION
            );
        }
        if self.project.name.trim().is_empty() {
            bail!("project.name must not be empty");
        }
        if self.render.width == 0 || self.render.height == 0 {
            bail!("render width and height must be positive");
        }
        if self.render.width > MAX_RENDER_DIMENSION || self.render.height > MAX_RENDER_DIMENSION {
            bail!("render dimensions exceed the {MAX_RENDER_DIMENSION} pixel safety limit");
        }
        if !self.render.fps.is_finite()
            || self.render.fps <= 0.0
            || self.render.fps > MAX_RENDER_FPS
        {
            bail!("render.fps must be finite and in the range (0, {MAX_RENDER_FPS}]");
        }
        if !self.render.preview_time.is_finite() || self.render.preview_time < 0.0 {
            bail!("render.preview_time must be a finite non-negative number");
        }
        for include_dir in &self.shader.include_dirs {
            validate_project_relative_path(include_dir, "shader.include_dirs entry")?;
        }
        self.validate_presets()?;
        validate_definitions(&self.uniforms)?;
        if self.passes.is_empty() {
            bail!("project must contain at least one [[pass]]");
        }

        let mut names = HashSet::new();
        let mut pass_kinds = HashMap::new();
        let mut image_count = 0usize;
        for pass in &self.passes {
            if pass.name.trim().is_empty() {
                bail!("pass name must not be empty");
            }
            if is_reserved_input_name(&pass.name) {
                bail!("pass name '{}' is reserved for a built-in input", pass.name);
            }
            validate_project_relative_path(
                &pass.source,
                &format!("source for pass '{}'", pass.name),
            )?;
            match (&pass.vertex, &pass.mesh, pass.kind) {
                (Some(vertex), Some(mesh), PassKind::Mesh) => {
                    validate_project_relative_path(
                        vertex,
                        &format!("vertex source for pass '{}'", pass.name),
                    )?;
                    if mesh.size[0] < 2 || mesh.size[1] < 2 {
                        bail!("mesh pass '{}' grid size must be at least 2x2", pass.name);
                    }
                    if mesh.instances == 0 || mesh.instances > i32::MAX as u32 {
                        bail!(
                            "mesh pass '{}' instances must be in 1..={}",
                            pass.name,
                            i32::MAX
                        );
                    }
                    if mesh.clear.iter().any(|value| !value.is_finite()) {
                        bail!("mesh pass '{}' clear values must be finite", pass.name);
                    }
                    let vertices = u64::from(mesh.size[0]) * u64::from(mesh.size[1]);
                    let indices = u64::from(mesh.size[0] - 1) * u64::from(mesh.size[1] - 1) * 6;
                    if vertices > i32::MAX as u64 || indices > i32::MAX as u64 {
                        bail!("mesh pass '{}' grid exceeds OpenGL draw limits", pass.name);
                    }
                }
                (None, None, PassKind::Mesh) => {
                    bail!("mesh pass '{}' requires vertex and [pass.mesh]", pass.name);
                }
                (None, Some(_), PassKind::Mesh) => {
                    bail!("mesh pass '{}' requires vertex", pass.name);
                }
                (Some(_), None, PassKind::Mesh) => {
                    bail!("mesh pass '{}' requires [pass.mesh]", pass.name);
                }
                (Some(_), _, _) => {
                    bail!("pass '{}' vertex is only valid for mesh passes", pass.name);
                }
                (_, Some(_), _) => {
                    bail!(
                        "pass '{}' [pass.mesh] is only valid for mesh passes",
                        pass.name
                    );
                }
                (None, None, _) => {}
            }
            if !names.insert(pass.name.as_str()) {
                bail!("duplicate pass/asset name '{}'", pass.name);
            }
            match (pass.width, pass.height) {
                (None, None) => {
                    if pass.kind == PassKind::Compute {
                        bail!("compute pass '{}' must specify width and height", pass.name);
                    }
                }
                (Some(width), Some(height)) => {
                    if !matches!(
                        pass.kind,
                        PassKind::Buffer | PassKind::Compute | PassKind::Mesh
                    ) {
                        bail!(
                            "fixed width/height are only supported for buffer/compute/mesh passes ('{}')",
                            pass.name
                        );
                    }
                    if width == 0 || height == 0 {
                        bail!("fixed pass dimensions must be positive ('{}')", pass.name);
                    }
                    if width > MAX_RENDER_DIMENSION || height > MAX_RENDER_DIMENSION {
                        bail!(
                            "fixed pass dimensions for '{}' exceed the {} pixel safety limit",
                            pass.name,
                            MAX_RENDER_DIMENSION
                        );
                    }
                }
                _ => bail!(
                    "pass '{}' must specify both width and height or neither",
                    pass.name
                ),
            }
            if !matches!(
                pass.kind,
                PassKind::Buffer | PassKind::Compute | PassKind::Mesh
            ) && pass.format != RenderFormat::Rgba32f
            {
                bail!(
                    "pass '{}' can only set format on buffer/compute/mesh passes",
                    pass.name
                );
            }
            if !pass.extra_outputs.is_empty()
                && !matches!(
                    pass.kind,
                    PassKind::Buffer | PassKind::Compute | PassKind::Mesh
                )
            {
                bail!(
                    "pass '{}' extra_outputs are only valid for buffer/compute/mesh passes",
                    pass.name
                );
            }
            if pass.extra_outputs.len() > 7 {
                bail!(
                    "pass '{}' may expose at most 8 color render targets",
                    pass.name
                );
            }
            if pass.kind == PassKind::Mesh
                && pass.mesh.as_ref().is_some_and(|mesh| mesh.depth)
                && pass.extra_outputs.len() > 6
            {
                bail!(
                    "depth-enabled mesh pass '{}' may expose at most 7 color render targets",
                    pass.name
                );
            }
            if pass.iterations == 0 || pass.iterations > 4096 {
                bail!("pass '{}' iterations must be in 1..=4096", pass.name);
            }
            if pass.kind != PassKind::Compute && pass.iterations != 1 {
                bail!(
                    "pass '{}' iterations are currently supported only for compute passes",
                    pass.name
                );
            }
            if let Some([x, y, z]) = pass.local_size {
                if pass.kind != PassKind::Compute {
                    bail!(
                        "pass '{}' local_size is only valid for compute passes",
                        pass.name
                    );
                }
                if x == 0 || y == 0 || z == 0 || u64::from(x) * u64::from(y) * u64::from(z) > 1024 {
                    bail!(
                        "pass '{}' local_size must be positive and at most 1024 total invocations",
                        pass.name
                    );
                }
                if z != 1 {
                    bail!(
                        "pass '{}' local_size z must be 1 because mainCompute receives 2D coordinates",
                        pass.name
                    );
                }
            }
            let mut storage_bindings = HashSet::new();
            for storage in &pass.storage {
                if storage.name.trim().is_empty() {
                    bail!(
                        "pass '{}' has a storage buffer with an empty name",
                        pass.name
                    );
                }
                if storage.size == 0 {
                    bail!(
                        "pass '{}' storage buffer '{}' must have a positive size",
                        pass.name,
                        storage.name
                    );
                }
                if !storage_bindings.insert(storage.binding) {
                    bail!(
                        "pass '{}' binds storage slot {} more than once",
                        pass.name,
                        storage.binding
                    );
                }
            }
            if pass.kind == PassKind::Image {
                image_count += 1;
            }
            pass_kinds.insert(pass.name.as_str(), pass.kind);
        }

        let mut asset_names = HashSet::new();
        let mut asset_kinds = HashMap::new();
        for asset in &self.assets {
            if asset.name.trim().is_empty() {
                bail!("asset name must not be empty");
            }
            if is_reserved_input_name(&asset.name) {
                bail!(
                    "asset name '{}' is reserved for a built-in input",
                    asset.name
                );
            }
            validate_project_relative_path(
                &asset.path,
                &format!("path for asset '{}'", asset.name),
            )?;
            if !names.insert(asset.name.as_str()) || !asset_names.insert(asset.name.as_str()) {
                bail!("duplicate pass/asset name '{}'", asset.name);
            }
            asset_kinds.insert(asset.name.as_str(), asset.kind);
        }

        if image_count != 1 {
            bail!("project must contain exactly one image pass (found {image_count})");
        }

        let mut storage_sizes = HashMap::new();
        for pass in &self.passes {
            for storage in &pass.storage {
                if let Some(existing) = storage_sizes.insert(storage.name.as_str(), storage.size)
                    && existing != storage.size
                {
                    bail!(
                        "storage buffer '{}' uses inconsistent sizes ({} vs {})",
                        storage.name,
                        existing,
                        storage.size
                    );
                }
            }
        }

        for pass in &self.passes {
            let mut channels = HashSet::new();
            for input in &pass.inputs {
                if input.channel > MAX_INPUT_CHANNEL {
                    bail!(
                        "pass '{}' uses invalid channel {}; valid channels are 0..={}",
                        pass.name,
                        input.channel,
                        MAX_INPUT_CHANNEL
                    );
                }
                if !channels.insert(input.channel) {
                    bail!(
                        "pass '{}' assigns iChannel{} more than once",
                        pass.name,
                        input.channel
                    );
                }
                let kind = self.infer_input_kind(input)?;
                if kind != InputKind::Pass && !input.output.is_primary() {
                    bail!(
                        "pass '{}' channel {} selects output {} on a non-pass input",
                        pass.name,
                        input.channel,
                        input.output
                    );
                }
                if input.frame == FrameRef::Previous && kind != InputKind::Pass {
                    bail!(
                        "pass '{}' channel {} uses previous frame on a non-pass input",
                        pass.name,
                        input.channel
                    );
                }
                if input.frame == FrameRef::Previous && !input.output.is_primary() {
                    bail!(
                        "pass '{}' channel {} uses previous-frame feedback from output {}; only output 0 is resumable",
                        pass.name,
                        input.channel,
                        input.output
                    );
                }
                if kind == InputKind::Pass {
                    let Some(source_kind) = pass_kinds.get(input.source.as_str()) else {
                        bail!(
                            "pass '{}' channel {} references unknown pass '{}'",
                            pass.name,
                            input.channel,
                            input.source
                        );
                    };
                    if input.frame == FrameRef::Previous && *source_kind == PassKind::Image {
                        bail!("the final image pass cannot be a previous-frame source");
                    }
                    if *source_kind == PassKind::Sound {
                        bail!("sound passes cannot be used as iChannel sources");
                    }
                    if pass.kind == PassKind::Sound {
                        bail!(
                            "sound passes currently support static/keyboard/music inputs, not pass inputs"
                        );
                    }
                    let source_pass = self
                        .passes
                        .iter()
                        .find(|candidate| candidate.name == input.source)
                        .expect("pass kind map and pass list stay in sync");
                    if input.output.is_depth() {
                        if source_pass.kind != PassKind::Mesh
                            || !source_pass.mesh.as_ref().is_some_and(|mesh| mesh.depth)
                        {
                            bail!(
                                "pass '{}' channel {} selects depth from '{}', which has no readable depth output",
                                pass.name,
                                input.channel,
                                input.source
                            );
                        }
                    } else if usize::from(input.output.color_index().expect("non-depth output"))
                        > source_pass.extra_outputs.len()
                    {
                        bail!(
                            "pass '{}' channel {} selects output {} from '{}', which exposes outputs 0..{}",
                            pass.name,
                            input.channel,
                            input.output,
                            input.source,
                            source_pass.extra_outputs.len()
                        );
                    }
                }
                if matches!(
                    kind,
                    InputKind::Texture | InputKind::Cubemap | InputKind::Volume | InputKind::Video
                ) {
                    let Some(asset_kind) = asset_kinds.get(input.source.as_str()) else {
                        bail!(
                            "pass '{}' channel {} references unknown asset '{}'",
                            pass.name,
                            input.channel,
                            input.source
                        );
                    };
                    let expected = match kind {
                        InputKind::Texture => AssetKind::Texture,
                        InputKind::Cubemap => AssetKind::Cubemap,
                        InputKind::Volume => AssetKind::Volume,
                        InputKind::Video => AssetKind::Video,
                        _ => unreachable!(),
                    };
                    if *asset_kind != expected {
                        bail!(
                            "pass '{}' channel {} expects {:?} asset '{}' but it is {:?}",
                            pass.name,
                            input.channel,
                            expected,
                            input.source,
                            asset_kind
                        );
                    }
                }
                if kind == InputKind::Keyboard && input.source != "keyboard" {
                    bail!(
                        "pass '{}' channel {} uses keyboard input with source '{}'; use source 'keyboard'",
                        pass.name,
                        input.channel,
                        input.source
                    );
                }
                if kind == InputKind::Music && input.source != "music" {
                    bail!(
                        "pass '{}' channel {} uses music input with source '{}'; use source 'music'",
                        pass.name,
                        input.channel,
                        input.source
                    );
                }
                if kind == InputKind::Webcam && input.source != "webcam" {
                    bail!(
                        "pass '{}' channel {} uses webcam input with source '{}'; use source 'webcam'",
                        pass.name,
                        input.channel,
                        input.source
                    );
                }
            }
        }
        let mut test_names = HashSet::new();
        for test in &self.tests {
            if test.name.trim().is_empty() {
                bail!("test name must not be empty");
            }
            if !test_names.insert(test.name.as_str()) {
                bail!("duplicate test name '{}'", test.name);
            }
            if test.frame.is_some() && test.time.is_some() {
                bail!(
                    "test '{}' must use either frame or time, not both",
                    test.name
                );
            }
            if !test.frames.is_empty() && (test.frame.is_some() || test.time.is_some()) {
                bail!(
                    "test '{}' frames is mutually exclusive with frame/time",
                    test.name
                );
            }
            if test.frames.len() > 1024 {
                bail!("test '{}' may contain at most 1024 frames", test.name);
            }
            if test.frame.is_some_and(|frame| frame < 0)
                || test.frames.iter().any(|frame| *frame < 0)
            {
                bail!("test '{}' frame values must be non-negative", test.name);
            }
            let mut unique_frames = HashSet::new();
            if test
                .frames
                .iter()
                .any(|frame| !unique_frames.insert(*frame))
            {
                bail!("test '{}' frames contains duplicates", test.name);
            }
            if test
                .time
                .is_some_and(|time| !time.is_finite() || time < 0.0)
            {
                bail!("test '{}' time must be finite and non-negative", test.name);
            }
            if !test.resolutions.is_empty() && (test.width.is_some() || test.height.is_some()) {
                bail!(
                    "test '{}' resolutions is mutually exclusive with width/height",
                    test.name
                );
            }
            if test.resolutions.len() > 32 {
                bail!("test '{}' may contain at most 32 resolutions", test.name);
            }
            for [width, height] in &test.resolutions {
                if *width == 0
                    || *height == 0
                    || *width > MAX_RENDER_DIMENSION
                    || *height > MAX_RENDER_DIMENSION
                {
                    bail!(
                        "test '{}' resolution {}x{} must be positive and at most {}",
                        test.name,
                        width,
                        height,
                        MAX_RENDER_DIMENSION
                    );
                }
            }
            let matrix_frames = test.frames.len().max(1);
            let matrix_resolutions = test.resolutions.len().max(1);
            if matrix_frames.saturating_mul(matrix_resolutions) > 1024 {
                bail!(
                    "test '{}' expands to more than 1024 frame/resolution variants",
                    test.name
                );
            }
            match (test.width, test.height) {
                (None, None) => {}
                (Some(width), Some(height))
                    if width > 0
                        && height > 0
                        && width <= MAX_RENDER_DIMENSION
                        && height <= MAX_RENDER_DIMENSION => {}
                (Some(_), Some(_)) => bail!(
                    "test '{}' dimensions must be positive and at most {}",
                    test.name,
                    MAX_RENDER_DIMENSION
                ),
                _ => bail!(
                    "test '{}' must specify both width and height or neither",
                    test.name
                ),
            }
            if !test.tolerance.is_finite() || !(0.0..=1.0).contains(&test.tolerance) {
                bail!(
                    "test '{}' tolerance must be finite and in [0, 1]",
                    test.name
                );
            }
            if let Some(reference) = &test.reference {
                validate_project_relative_path(
                    reference,
                    &format!("reference for test '{}'", test.name),
                )?;
            }
            if test.reference.is_some() && (test.frames.len() > 1 || test.resolutions.len() > 1) {
                bail!(
                    "test '{}' uses a single reference image and therefore cannot also define a frame/resolution matrix",
                    test.name
                );
            }
            if let Some(pass) = &test.pass
                && !self.passes.iter().any(|candidate| candidate.name == *pass)
            {
                bail!("test '{}' references unknown pass '{}'", test.name, pass);
            }
            if let Some(pass) = &test.pass
                && self
                    .passes
                    .iter()
                    .any(|candidate| candidate.name == *pass && candidate.kind == PassKind::Sound)
            {
                bail!(
                    "test '{}' selects Sound pass '{}'; use render-audio for Sound validation",
                    test.name,
                    pass
                );
            }
            if !test.raw_tolerance.is_finite() || test.raw_tolerance < 0.0 {
                bail!(
                    "test '{}' raw_tolerance must be finite and non-negative",
                    test.name
                );
            }
            if test.assert_resolution_independent {
                if test.resolutions.len() < 2 {
                    bail!(
                        "test '{}' assert_resolution_independent requires at least two resolutions",
                        test.name
                    );
                }
                let selected = test
                    .pass
                    .as_deref()
                    .map(|name| {
                        self.passes
                            .iter()
                            .find(|candidate| candidate.name == name)
                            .expect("test pass was validated above")
                    })
                    .unwrap_or_else(|| self.final_pass());
                if !matches!(
                    selected.kind,
                    PassKind::Buffer | PassKind::Compute | PassKind::Mesh
                ) || selected.width.is_none()
                    || selected.height.is_none()
                {
                    bail!(
                        "test '{}' resolution independence requires a fixed-size buffer/compute/mesh pass",
                        test.name
                    );
                }
            }
            if let Some([min, max]) = test.mean_range
                && (!min.is_finite() || !max.is_finite() || min > max)
            {
                bail!(
                    "test '{}' mean_range must contain finite [min, max]",
                    test.name
                );
            }
            if test.reference.is_some() && !test.reference_uniforms.is_empty() {
                bail!(
                    "test '{}' must use either reference or reference_uniforms, not both",
                    test.name
                );
            }
            for (set_name, values) in [
                ("uniforms", &test.uniforms),
                ("reference_uniforms", &test.reference_uniforms),
            ] {
                for (name, value) in values {
                    let definition = self
                        .uniforms
                        .iter()
                        .find(|definition| definition.name() == name)
                        .with_context(|| {
                            format!(
                                "test '{}' {} references unknown custom uniform '{}'",
                                test.name, set_name, name
                            )
                        })?;
                    definition.validate_value(value).with_context(|| {
                        format!(
                            "invalid custom uniform '{}' in test '{}' {}",
                            name, test.name, set_name
                        )
                    })?;
                }
            }
            for (name, value) in [("min_rmse", test.min_rmse), ("max_rmse", test.max_rmse)] {
                if value.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
                    bail!("test '{}' {} must be finite and in [0, 1]", test.name, name);
                }
            }
            if let (Some(min), Some(max)) = (test.min_rmse, test.max_rmse)
                && min > max
            {
                bail!("test '{}' min_rmse cannot exceed max_rmse", test.name);
            }
            if (test.min_rmse.is_some() || test.max_rmse.is_some())
                && test.reference.is_none()
                && test.reference_uniforms.is_empty()
            {
                bail!(
                    "test '{}' RMSE bounds require reference or reference_uniforms",
                    test.name
                );
            }
            if test
                .max_gpu_ms
                .is_some_and(|value| !value.is_finite() || value < 0.0)
            {
                bail!(
                    "test '{}' max_gpu_ms must be finite and non-negative",
                    test.name
                );
            }
            for (pass_name, budget) in &test.max_pass_gpu_ms {
                if !budget.is_finite() || *budget < 0.0 {
                    bail!(
                        "test '{}' GPU budget for pass '{}' must be finite and non-negative",
                        test.name,
                        pass_name
                    );
                }
                if !self.passes.iter().any(|pass| pass.name == *pass_name) {
                    bail!(
                        "test '{}' GPU budget references unknown pass '{}'",
                        test.name,
                        pass_name
                    );
                }
            }
            let storage_sizes = self
                .passes
                .iter()
                .flat_map(|pass| &pass.storage)
                .map(|storage| (storage.name.as_str(), storage.size))
                .collect::<HashMap<_, _>>();
            let mut storage_names = HashSet::new();
            for assertion in &test.storage_assertions {
                if !storage_names.insert(assertion.name.as_str()) {
                    bail!(
                        "test '{}' contains duplicate storage assertion '{}'",
                        test.name,
                        assertion.name
                    );
                }
                if !storage_sizes.contains_key(assertion.name.as_str()) {
                    bail!(
                        "test '{}' storage assertion references unknown SSBO '{}'",
                        test.name,
                        assertion.name
                    );
                }
                validate_project_relative_path(
                    &assertion.reference,
                    &format!(
                        "storage reference '{}' for test '{}'",
                        assertion.name, test.name
                    ),
                )?;
            }
            if test.reference.is_none()
                && test.reference_uniforms.is_empty()
                && !test.assert_no_nan
                && !test.assert_no_inf
                && test.mean_range.is_none()
                && !test.assert_deterministic
                && !test.assert_resolution_independent
                && test.max_gpu_ms.is_none()
                && test.max_pass_gpu_ms.is_empty()
                && test.storage_assertions.is_empty()
                && !test.assert_state_roundtrip
            {
                bail!(
                    "test '{}' must define a reference or at least one assertion",
                    test.name
                );
            }
        }

        Ok(())
    }

    fn validate_presets(&self) -> Result<()> {
        for (name, preset) in &self.presets {
            if name.trim().is_empty() {
                bail!("preset name must not be empty");
            }
            if let Some(scale) = preset.render_scale {
                if !scale.is_finite() || scale <= 0.0 {
                    bail!("preset '{name}' render_scale must be finite and positive");
                }
                let width = scaled_dimension(self.render.width, scale)
                    .with_context(|| format!("preset '{name}' render_scale"))?;
                let height = scaled_dimension(self.render.height, scale)
                    .with_context(|| format!("preset '{name}' render_scale"))?;
                if width > MAX_RENDER_DIMENSION || height > MAX_RENDER_DIMENSION {
                    bail!(
                        "preset '{name}' render_scale produces dimensions above the {MAX_RENDER_DIMENSION} pixel safety limit"
                    );
                }
            }
            for (pass_name, override_) in &preset.passes {
                let pass = self
                    .passes
                    .iter()
                    .find(|pass| pass.name == *pass_name)
                    .with_context(|| {
                        format!("preset '{name}' references unknown pass '{pass_name}'")
                    })?;
                match (override_.width, override_.height) {
                    (None, None) => {}
                    (Some(width), Some(height)) => {
                        if !matches!(
                            pass.kind,
                            PassKind::Buffer | PassKind::Compute | PassKind::Mesh
                        ) {
                            bail!(
                                "preset '{name}' can only override dimensions for buffer/compute/mesh pass '{pass_name}'"
                            );
                        }
                        if width == 0
                            || height == 0
                            || width > MAX_RENDER_DIMENSION
                            || height > MAX_RENDER_DIMENSION
                        {
                            bail!(
                                "preset '{name}' pass '{pass_name}' dimensions must be positive and at most {MAX_RENDER_DIMENSION}"
                            );
                        }
                    }
                    _ => bail!(
                        "preset '{name}' pass '{pass_name}' must specify both width and height or neither"
                    ),
                }
                if let Some(size) = override_.mesh_size {
                    if pass.kind != PassKind::Mesh {
                        bail!(
                            "preset '{name}' mesh_size override is only valid for mesh pass '{pass_name}'"
                        );
                    }
                    if size[0] < 2 || size[1] < 2 {
                        bail!("preset '{name}' mesh_size for '{pass_name}' must be at least 2x2");
                    }
                    let vertices = u64::from(size[0]) * u64::from(size[1]);
                    let indices = u64::from(size[0] - 1) * u64::from(size[1] - 1) * 6;
                    if vertices > i32::MAX as u64 || indices > i32::MAX as u64 {
                        bail!(
                            "preset '{name}' mesh_size for '{pass_name}' exceeds OpenGL draw limits"
                        );
                    }
                }
                if let Some(instances) = override_.mesh_instances {
                    if pass.kind != PassKind::Mesh {
                        bail!(
                            "preset '{name}' mesh_instances override is only valid for mesh pass '{pass_name}'"
                        );
                    }
                    if instances == 0 || instances > i32::MAX as u32 {
                        bail!(
                            "preset '{name}' mesh_instances for '{pass_name}' must be in 1..={}",
                            i32::MAX
                        );
                    }
                }
                if let Some(iterations) = override_.iterations {
                    if pass.kind != PassKind::Compute {
                        bail!(
                            "preset '{name}' iterations override is only valid for compute pass '{pass_name}'"
                        );
                    }
                    if iterations == 0 || iterations > 4096 {
                        bail!("preset '{name}' pass '{pass_name}' iterations must be in 1..=4096");
                    }
                }
                if let Some([x, y, z]) = override_.local_size {
                    if pass.kind != PassKind::Compute {
                        bail!(
                            "preset '{name}' local_size override is only valid for compute pass '{pass_name}'"
                        );
                    }
                    if x == 0
                        || y == 0
                        || z != 1
                        || u64::from(x) * u64::from(y) * u64::from(z) > 1024
                    {
                        bail!(
                            "preset '{name}' pass '{pass_name}' local_size must be positive, use z=1, and have at most 1024 invocations"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub fn apply_preset(&mut self, name: &str) -> Result<()> {
        let preset = self.presets.get(name).cloned().with_context(|| {
            format!(
                "unknown preset '{name}'; available presets: {}",
                if self.presets.is_empty() {
                    "<none>".to_string()
                } else {
                    self.presets.keys().cloned().collect::<Vec<_>>().join(", ")
                }
            )
        })?;

        if let Some(scale) = preset.render_scale {
            self.render.width = scaled_dimension(self.render.width, scale)
                .with_context(|| format!("preset '{name}' render_scale"))?;
            self.render.height = scaled_dimension(self.render.height, scale)
                .with_context(|| format!("preset '{name}' render_scale"))?;
        }
        for (pass_name, override_) in preset.passes {
            let pass = self
                .passes
                .iter_mut()
                .find(|pass| pass.name == pass_name)
                .expect("preset pass names are validated before application");
            if let (Some(width), Some(height)) = (override_.width, override_.height) {
                pass.width = Some(width);
                pass.height = Some(height);
            }
            if let Some(iterations) = override_.iterations {
                pass.iterations = iterations;
            }
            if let Some(local_size) = override_.local_size {
                pass.local_size = Some(local_size);
            }
            if let Some(size) = override_.mesh_size {
                pass.mesh
                    .as_mut()
                    .expect("mesh preset override was validated")
                    .size = size;
            }
            if let Some(instances) = override_.mesh_instances {
                pass.mesh
                    .as_mut()
                    .expect("mesh preset override was validated")
                    .instances = instances;
            }
        }
        Ok(())
    }

    pub fn infer_input_kind(&self, input: &Input) -> Result<InputKind> {
        if let Some(kind) = input.kind {
            return Ok(kind);
        }
        if input.source == "keyboard" {
            return Ok(InputKind::Keyboard);
        }
        if input.source == "music" {
            return Ok(InputKind::Music);
        }
        if input.source == "webcam" {
            return Ok(InputKind::Webcam);
        }
        if let Some(asset) = self.assets.iter().find(|asset| asset.name == input.source) {
            return Ok(match asset.kind {
                AssetKind::Texture => InputKind::Texture,
                AssetKind::Cubemap => InputKind::Cubemap,
                AssetKind::Volume => InputKind::Volume,
                AssetKind::Video => InputKind::Video,
            });
        }
        if self.passes.iter().any(|pass| pass.name == input.source) {
            return Ok(InputKind::Pass);
        }
        bail!(
            "cannot infer input kind for unknown source '{}'",
            input.source
        )
    }

    pub fn pass_dimensions(
        &self,
        pass: &Pass,
        output_width: u32,
        output_height: u32,
    ) -> (u32, u32) {
        match (pass.width, pass.height) {
            (Some(width), Some(height)) => (width, height),
            _ => (output_width, output_height),
        }
    }

    pub fn final_pass(&self) -> &Pass {
        self.passes
            .iter()
            .find(|pass| pass.kind == PassKind::Image)
            .expect("validated manifest always has one image pass")
    }

    pub fn to_pretty_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("failed to serialize ShaderToy.toml")
    }
}

impl LoadedManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_preset(path, None)
    }

    pub fn load_with_preset(path: impl AsRef<Path>, preset: Option<&str>) -> Result<Self> {
        let root = find_project_root(path.as_ref())?;
        let manifest_path = root.join(MANIFEST_NAME);
        let source = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let mut manifest: Manifest = toml::from_str(&source)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
        manifest.validate_structure()?;
        if let Some(preset) = preset {
            manifest.apply_preset(preset)?;
        }
        crate::project_schema::refresh_existing(&root)?;
        Ok(Self { root, manifest })
    }
}

fn scaled_dimension(base: u32, scale: f32) -> Result<u32> {
    let value = (base as f64 * scale as f64).round();
    if !value.is_finite() || value < 1.0 || value > u32::MAX as f64 {
        bail!("scaled render dimension is outside the supported range");
    }
    Ok(value as u32)
}

pub fn find_project_root(start: &Path) -> Result<PathBuf> {
    let start = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };
    let absolute = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir()?.join(start)
    };

    for candidate in absolute.ancestors() {
        if candidate.join(MANIFEST_NAME).is_file() {
            return Ok(candidate.to_path_buf());
        }
    }
    bail!(
        "could not find {MANIFEST_NAME} from {}; run 'shadertoy new' or 'shadertoy init'",
        absolute.display()
    )
}

pub fn schema_json() -> Result<String> {
    serde_json::to_string_pretty(&schema_for!(Manifest))
        .context("failed to serialize manifest schema")
}

pub fn validate_project_relative_path(value: &str, label: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} must not be empty");
    }

    // Manifests use portable project-relative paths. Normalize separators before
    // checking so a Windows traversal is rejected even when inspected on Unix.
    let normalized = value.replace('\\', "/");
    let bytes = normalized.as_bytes();
    let windows_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let path = Path::new(&normalized);
    if windows_drive
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("{label} must stay relative to the project root");
    }
    Ok(())
}

fn is_reserved_input_name(value: &str) -> bool {
    RESERVED_INPUT_NAMES.contains(&value)
}

#[cfg(test)]
mod tests;
