use super::*;
use crate::manifest::{AssetKind, PassKind};
use crate::state::BufferDimensions;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const TRACE_FORMAT: u32 = 1;

#[derive(Debug, Clone)]
pub struct TraceCaptureOptions {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub output: PathBuf,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub frame: Option<i32>,
    pub time: Option<f32>,
    pub set_uniforms: Vec<String>,
    pub include_intermediates: bool,
}

#[derive(Debug, Clone)]
pub struct TraceReplayOptions {
    pub trace: PathBuf,
    pub output: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceArtifact {
    path: PathBuf,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceTiming {
    name: String,
    width: u32,
    height: u32,
    gpu_nanoseconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceMetadata {
    format: u32,
    cli_version: String,
    project: String,
    preset: Option<String>,
    width: u32,
    height: u32,
    fps: f32,
    frame: i32,
    time: f32,
    uniforms: serde_json::Value,
    resolved_manifest: serde_json::Value,
    source_sha256: BTreeMap<String, String>,
    asset_sha256: BTreeMap<String, String>,
    pass_timings: Vec<TraceTiming>,
    graph_diagnostics: serde_json::Value,
    state_header: serde_json::Value,
    synchronization: serde_json::Value,
    artifacts: BTreeMap<String, TraceArtifact>,
}

pub fn capture_trace(options: &TraceCaptureOptions) -> Result<Output> {
    validate_trace_dir(&options.output)?;
    if options.output.exists() {
        fs::remove_dir_all(&options.output)
            .with_context(|| format!("failed to replace trace {}", options.output.display()))?;
    }
    fs::create_dir_all(&options.output)?;

    let loaded = LoadedManifest::load_with_preset(&options.project, options.preset.as_deref())?;
    ensure_source_files_exist(&loaded)?;
    if loaded
        .manifest
        .assets
        .iter()
        .any(|asset| asset.kind == AssetKind::Video)
    {
        bail!(
            "trace capture currently requires deterministic static inputs; video assets are not embedded in STTF replay"
        );
    }
    if crate::media::manifest_uses_webcam(&loaded) {
        bail!("trace capture does not support live webcam inputs");
    }

    let width = options.width.unwrap_or(loaded.manifest.render.width);
    let height = options.height.unwrap_or(loaded.manifest.render.height);
    validate_dimensions(width, height)?;
    let fps = options.fps.unwrap_or(loaded.manifest.render.fps);
    validate_fps(fps)?;
    let frame = resolve_target_frame(&loaded, None, options.frame, options.time, fps)?;
    let media = crate::media::MediaInputs::new_headless(&loaded)?;
    let uniform_values =
        crate::uniforms::parse_assignments(&loaded.manifest.uniforms, &options.set_uniforms)?;

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for trace capture")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_native_project(&loaded)?;
    runtime.load_project(&project)?;
    crate::uniforms::apply_to_runtime(&mut runtime, &uniform_values)?;
    runtime.set_profiling_mode(true, true)?;
    let final_image = render_from_zero(&mut runtime, frame, fps, width, height, &[], &media)?
        .context("trace capture did not produce a final image")?;
    let timings = runtime.pass_timings()?;
    runtime.set_profiling(false)?;

    let sttf_path = options.output.join("project.sttf");
    runtime.save_sttf(&sttf_path)?;
    super::project::annotate_sttf_render_defaults(&sttf_path, width, height, fps)?;
    let final_path = options.output.join("final.png");
    save_rgb_png(&final_image, &final_path)?;

    let state = capture_runtime_state(&mut runtime, &loaded, width, height, fps)?;
    let state_path = options.output.join("state.ststate");
    state.save(&state_path)?;

    let mut artifacts = BTreeMap::new();
    add_artifact(&mut artifacts, "project_sttf", &options.output, &sttf_path)?;
    add_artifact(&mut artifacts, "final_image", &options.output, &final_path)?;
    add_artifact(&mut artifacts, "state", &options.output, &state_path)?;

    if options.include_intermediates {
        capture_intermediates(
            &mut runtime,
            &loaded,
            width,
            height,
            &options.output,
            &mut artifacts,
        )?;
    }

    let expanded = crate::source::expand_all(&loaded)?;
    let source_sha256 = expanded
        .iter()
        .map(|(name, source)| {
            let mut bytes = source.text.as_bytes().to_vec();
            if let Some(vertex) = &source.vertex_text {
                bytes.extend_from_slice(vertex.as_bytes());
            }
            (name.clone(), hex_sha256(&bytes))
        })
        .collect::<BTreeMap<_, _>>();
    let mut asset_sha256 = BTreeMap::new();
    for asset in &loaded.manifest.assets {
        let path = loaded.root.join(&asset.path);
        let bytes = fs::read(&path)
            .with_context(|| format!("failed to hash trace asset {}", path.display()))?;
        asset_sha256.insert(asset.name.clone(), hex_sha256(&bytes));
    }

    let diagnostics = super::graph::analyze_graph(&loaded, true)?;
    let synchronization = synchronization_summary(&loaded)?;
    let metadata = TraceMetadata {
        format: TRACE_FORMAT,
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        project: loaded.manifest.project.name.clone(),
        preset: options.preset.clone(),
        width,
        height,
        fps,
        frame: runtime.frame(),
        time: runtime.time(),
        uniforms: serde_json::to_value(&uniform_values)?,
        resolved_manifest: serde_json::to_value(&loaded.manifest)?,
        source_sha256,
        asset_sha256,
        pass_timings: timings
            .into_iter()
            .map(|timing| TraceTiming {
                name: timing.name,
                width: timing.width,
                height: timing.height,
                gpu_nanoseconds: timing.gpu_nanoseconds,
            })
            .collect(),
        graph_diagnostics: serde_json::to_value(diagnostics)?,
        state_header: serde_json::to_value(&state.header)?,
        synchronization,
        artifacts,
    };

    let metadata_path = options.output.join("trace.json");
    fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)
        .with_context(|| format!("failed to write {}", metadata_path.display()))?;

    Ok(Output {
        human: format!(
            "Captured trace {} at frame {} ({:.3}s), {}x{}; {} persistent buffers, {} SSBOs, {} timed passes",
            options.output.display(),
            metadata.frame,
            metadata.time,
            width,
            height,
            state.header.buffers.len(),
            state.header.storage_buffers.len(),
            metadata.pass_timings.len()
        ),
        json: json!({
            "ok": true,
            "action": "trace-capture",
            "trace": options.output,
            "metadata": metadata_path,
            "frame": metadata.frame,
            "time": metadata.time,
            "width": width,
            "height": height,
            "artifacts": metadata.artifacts,
        }),
    })
}

pub fn inspect_trace(path: &Path) -> Result<Output> {
    let metadata = load_trace(path)?;
    let trace_dir = trace_dir(path);
    verify_artifacts(&trace_dir, &metadata)?;
    let gpu_total_ns = metadata
        .pass_timings
        .iter()
        .map(|timing| timing.gpu_nanoseconds)
        .sum::<u64>();
    let mut human = format!(
        "Trace: {}\nProject: {}\nCLI: {}\nFrame: {} ({:.3}s)\nRender: {}x{} @ {} fps\nGPU pass total: {:.3} ms\nArtifacts: {}",
        trace_dir.display(),
        metadata.project,
        metadata.cli_version,
        metadata.frame,
        metadata.time,
        metadata.width,
        metadata.height,
        metadata.fps,
        gpu_total_ns as f64 / 1_000_000.0,
        metadata.artifacts.len()
    );
    if let Some(preset) = &metadata.preset {
        human.push_str(&format!("\nPreset: {preset}"));
    }
    human.push_str("\nPass timings:");
    for timing in &metadata.pass_timings {
        human.push_str(&format!(
            "\n  {} {}x{} {:.3} ms",
            timing.name,
            timing.width,
            timing.height,
            timing.gpu_nanoseconds as f64 / 1_000_000.0
        ));
    }

    Ok(Output {
        human,
        json: json!({
            "ok": true,
            "trace": trace_dir,
            "metadata": metadata,
            "gpu_pass_total_ms": gpu_total_ns as f64 / 1_000_000.0,
            "verified": true,
        }),
    })
}

pub fn replay_trace(options: &TraceReplayOptions) -> Result<Output> {
    let metadata = load_trace(&options.trace)?;
    let trace_dir = trace_dir(&options.trace);
    verify_artifacts(&trace_dir, &metadata)?;

    let sttf = artifact_path(&trace_dir, &metadata, "project_sttf")?;
    let expected_path = artifact_path(&trace_dir, &metadata, "final_image")?;
    let expected = load_rgb_bottom_up(&expected_path)?;

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for trace replay")?;
    let mut runtime = Runtime::new(&context)?;
    runtime.load_sttf(&sttf)?;
    let actual = render_sttf_from_zero(
        &mut runtime,
        metadata.frame,
        metadata.fps,
        metadata.width,
        metadata.height,
    )?;

    if let Some(output) = &options.output {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        save_rgb_png(&actual, output)?;
    }
    let rmse = normalized_rmse(&expected.pixels, &actual.pixels);
    let exact = expected.pixels == actual.pixels;
    let state_path = artifact_path(&trace_dir, &metadata, "state")?;
    let state = StateFile::load(&state_path)?;
    let state_valid = state.header.frame == metadata.frame
        && state.header.width == metadata.width
        && state.header.height == metadata.height;

    Ok(Output {
        human: format!(
            "Replayed {} -> frame {}: {} (RMSE {:.9}); captured state {}",
            trace_dir.display(),
            metadata.frame,
            if exact {
                "bit-exact RGB match"
            } else {
                "RGB differs"
            },
            rmse,
            if state_valid {
                "valid"
            } else {
                "metadata mismatch"
            }
        ),
        json: json!({
            "ok": exact && state_valid,
            "action": "trace-replay",
            "trace": trace_dir,
            "frame": metadata.frame,
            "exact": exact,
            "rmse": rmse,
            "state_valid": state_valid,
            "output": options.output,
        }),
    })
}

fn capture_runtime_state(
    runtime: &mut Runtime<'_>,
    loaded: &LoadedManifest,
    width: u32,
    height: u32,
    fps: f32,
) -> Result<StateFile> {
    let mut buffers = BTreeMap::new();
    let mut buffer_dimensions = BTreeMap::new();
    let mut buffer_formats = BTreeMap::new();
    for pass in &loaded.manifest.passes {
        if !matches!(
            pass.kind,
            PassKind::Buffer | PassKind::Compute | PassKind::Mesh
        ) {
            continue;
        }
        let (pass_width, pass_height) = loaded.manifest.pass_dimensions(pass, width, height);
        if let Ok(values) = runtime.snapshot_pass_rgba32f(&pass.name, pass_width, pass_height) {
            buffers.insert(pass.name.clone(), values);
            buffer_dimensions.insert(
                pass.name.clone(),
                BufferDimensions {
                    width: pass_width,
                    height: pass_height,
                },
            );
            buffer_formats.insert(pass.name.clone(), pass.format);
        }
    }

    let mut sizes = BTreeMap::new();
    for pass in &loaded.manifest.passes {
        for storage in &pass.storage {
            sizes.entry(storage.name.clone()).or_insert(storage.size);
        }
    }
    let mut storage_buffers = BTreeMap::new();
    for (name, size) in sizes {
        if let Ok(data) = runtime.snapshot_storage_buffer(
            &name,
            usize::try_from(size).context("trace SSBO size exceeds usize")?,
        ) {
            storage_buffers.insert(name, data);
        }
    }

    StateFile::new(
        loaded.manifest.project.name.clone(),
        width,
        height,
        fps,
        runtime.time(),
        runtime.frame(),
        buffers,
        buffer_dimensions,
        buffer_formats,
        storage_buffers,
    )
}

fn capture_intermediates(
    runtime: &mut Runtime<'_>,
    loaded: &LoadedManifest,
    width: u32,
    height: u32,
    trace_dir: &Path,
    artifacts: &mut BTreeMap<String, TraceArtifact>,
) -> Result<()> {
    let root = trace_dir.join("passes");
    fs::create_dir_all(&root)?;
    for pass in &loaded.manifest.passes {
        if !matches!(
            pass.kind,
            PassKind::Buffer | PassKind::Compute | PassKind::Mesh
        ) {
            continue;
        }
        let (pass_width, pass_height) = loaded.manifest.pass_dimensions(pass, width, height);
        let mut outputs = (0..=pass.extra_outputs.len())
            .map(|index| (index as u32, index.to_string()))
            .collect::<Vec<_>>();
        if pass.kind == PassKind::Mesh && pass.mesh.as_ref().is_some_and(|mesh| mesh.depth) {
            outputs.push((shadertoy::PASS_OUTPUT_DEPTH, "depth".to_string()));
        }
        for (native_output, output_label) in outputs {
            let Ok(values) = runtime.snapshot_pass_output_rgba32f(
                &pass.name,
                native_output,
                pass_width,
                pass_height,
            ) else {
                continue;
            };
            let stem = format!("{}-output-{}", file_safe_name(&pass.name), output_label);
            let raw_path = root.join(format!("{stem}.rgba32f"));
            let mut bytes = Vec::with_capacity(values.len() * 4);
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            fs::write(&raw_path, bytes)?;
            add_artifact(
                artifacts,
                &format!("pass:{}:output:{}:raw", pass.name, output_label),
                trace_dir,
                &raw_path,
            )?;

            if let Ok(image) =
                runtime.snapshot_pass_output_rgb(&pass.name, native_output, pass_width, pass_height)
            {
                let png_path = root.join(format!("{stem}.png"));
                save_rgb_png(&image, &png_path)?;
                add_artifact(
                    artifacts,
                    &format!("pass:{}:output:{}:png", pass.name, output_label),
                    trace_dir,
                    &png_path,
                )?;
            }
        }
    }
    Ok(())
}

fn synchronization_summary(loaded: &LoadedManifest) -> Result<serde_json::Value> {
    let mut current_edges = Vec::new();
    let mut previous_edges = Vec::new();
    let mut storage_users: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for pass in &loaded.manifest.passes {
        for input in &pass.inputs {
            if loaded.manifest.infer_input_kind(input)? != crate::manifest::InputKind::Pass {
                continue;
            }
            let edge = json!({
                "from": input.source,
                "to": pass.name,
                "channel": input.channel,
                "output": input.output,
            });
            match input.frame {
                crate::manifest::FrameRef::Current => current_edges.push(edge),
                crate::manifest::FrameRef::Previous => previous_edges.push(edge),
            }
        }
        for storage in &pass.storage {
            storage_users
                .entry(storage.name.clone())
                .or_default()
                .push(json!({
                    "pass": pass.name,
                    "binding": storage.binding,
                    "size": storage.size,
                }));
        }
    }
    Ok(json!({
        "runtime": "native automatic dependency ordering and storage/image memory barriers",
        "current_frame_edges": current_edges,
        "previous_frame_edges": previous_edges,
        "storage_users": storage_users,
    }))
}

fn render_sttf_from_zero(
    runtime: &mut Runtime<'_>,
    target_frame: i32,
    fps: f32,
    width: u32,
    height: u32,
) -> Result<RgbImage> {
    if target_frame < 0 {
        bail!("trace frame must be non-negative");
    }
    runtime.set_fixed_state(0.0, 0, fps)?;
    if target_frame == 0 {
        return runtime.render(width, height).map_err(Into::into);
    }
    let _ = runtime.render(width, height)?;
    for _ in 1..target_frame {
        runtime.tick_fixed(1.0 / fps, fps)?;
        let _ = runtime.render(width, height)?;
    }
    runtime.tick_fixed(1.0 / fps, fps)?;
    runtime.render(width, height).map_err(Into::into)
}

fn validate_trace_dir(path: &Path) -> Result<()> {
    if path.extension().and_then(|value| value.to_str()) != Some("sttrace") {
        bail!(
            "trace output must be a directory ending in .sttrace (got {})",
            path.display()
        );
    }
    if path.exists() && !path.is_dir() {
        bail!(
            "trace output exists and is not a directory: {}",
            path.display()
        );
    }
    Ok(())
}

fn load_trace(path: &Path) -> Result<TraceMetadata> {
    let dir = trace_dir(path);
    let metadata_path = dir.join("trace.json");
    let bytes = fs::read(&metadata_path)
        .with_context(|| format!("failed to read trace metadata {}", metadata_path.display()))?;
    let metadata: TraceMetadata =
        serde_json::from_slice(&bytes).context("invalid trace metadata JSON")?;
    if metadata.format != TRACE_FORMAT {
        bail!(
            "unsupported trace format {}; expected {}",
            metadata.format,
            TRACE_FORMAT
        );
    }
    Ok(metadata)
}

fn trace_dir(path: &Path) -> PathBuf {
    if path.file_name().and_then(|name| name.to_str()) == Some("trace.json") {
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        path.to_path_buf()
    }
}

fn add_artifact(
    artifacts: &mut BTreeMap<String, TraceArtifact>,
    key: &str,
    root: &Path,
    path: &Path,
) -> Result<()> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read trace artifact {}", path.display()))?;
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("trace artifact {} is outside trace root", path.display()))?
        .to_path_buf();
    artifacts.insert(
        key.to_string(),
        TraceArtifact {
            path: relative,
            bytes: bytes.len() as u64,
            sha256: hex_sha256(&bytes),
        },
    );
    Ok(())
}

fn verify_artifacts(root: &Path, metadata: &TraceMetadata) -> Result<()> {
    for (key, artifact) in &metadata.artifacts {
        let path = root.join(&artifact.path);
        let bytes = fs::read(&path)
            .with_context(|| format!("trace artifact '{key}' is missing: {}", path.display()))?;
        if bytes.len() as u64 != artifact.bytes || hex_sha256(&bytes) != artifact.sha256 {
            bail!("trace artifact '{key}' failed size/SHA-256 verification");
        }
    }
    Ok(())
}

fn artifact_path(root: &Path, metadata: &TraceMetadata, key: &str) -> Result<PathBuf> {
    let artifact = metadata
        .artifacts
        .get(key)
        .with_context(|| format!("trace metadata is missing artifact '{key}'"))?;
    Ok(root.join(&artifact.path))
}

fn load_rgb_bottom_up(path: &Path) -> Result<RgbImage> {
    let image = ImageReader::open(path)
        .with_context(|| format!("failed to open trace image {}", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode trace image {}", path.display()))?
        .to_rgb8();
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    super::images::flip_rgb_rows(&mut pixels, width, height);
    Ok(RgbImage::new(width, height, pixels))
}

fn normalized_rmse(a: &[u8], b: &[u8]) -> f64 {
    if a.len() != b.len() {
        return f64::INFINITY;
    }
    let sum = a
        .iter()
        .zip(b)
        .map(|(a, b)| {
            let delta = f64::from(*a) - f64::from(*b);
            delta * delta
        })
        .sum::<f64>();
    (sum / a.len().max(1) as f64).sqrt() / 255.0
}

fn hex_sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn file_safe_name(value: &str) -> String {
    let result = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = result.trim_matches('-');
    if trimmed.is_empty() {
        "pass".into()
    } else {
        trimmed.into()
    }
}
