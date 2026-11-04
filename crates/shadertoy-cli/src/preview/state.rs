use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) struct ReloadPreservation {
    pub uniform_values: bool,
    pub runtime_state: bool,
}

impl ReloadPreservation {
    pub const RESET: Self = Self {
        uniform_values: false,
        runtime_state: false,
    };
    pub const PRESET_SWITCH: Self = Self {
        uniform_values: true,
        runtime_state: false,
    };
    pub const RUNTIME_STATE: Self = Self {
        uniform_values: true,
        runtime_state: true,
    };
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reload(
    root: &Path,
    preset: Option<&str>,
    runtime: &mut Runtime<'_>,
    loaded: &mut Option<LoadedManifest>,
    sources: &mut Option<SourceGraph>,
    width: &mut u32,
    height: &mut u32,
    resolution_override: Option<(u32, u32)>,
    fps: &mut f32,
    view: &mut String,
    uniform_values: &mut BTreeMap<String, crate::uniforms::UniformValue>,
    wireframe: Option<&str>,
    preservation: ReloadPreservation,
) -> Result<()> {
    let previous_uniform_values = preservation.uniform_values.then(|| uniform_values.clone());
    let saved = if preservation.runtime_state {
        loaded
            .as_ref()
            .map(|current| save_runtime_state(runtime, current, *width, *height, *fps))
            .transpose()?
    } else {
        None
    };

    let candidate = LoadedManifest::load_with_preset(root, preset)?;
    ensure_source_files_exist(&candidate)?;
    let (project, candidate_sources) = build_native_project_with_sources(&candidate)?;
    runtime.load_project(&project)?;
    crate::ops::apply_wireframe(runtime, &candidate, wireframe)?;
    let mut next_uniform_values = crate::uniforms::defaults(&candidate.manifest.uniforms);
    if let Some(previous) = previous_uniform_values {
        for (name, value) in previous {
            if let Some(definition) = candidate
                .manifest
                .uniforms
                .iter()
                .find(|definition| definition.name() == name)
                && definition.validate_value(&value).is_ok()
            {
                next_uniform_values.insert(name, value);
            }
        }
    }
    crate::uniforms::apply_to_runtime(runtime, &next_uniform_values)?;
    *uniform_values = next_uniform_values;

    match resolution_override {
        Some((override_width, override_height)) => {
            *width = override_width;
            *height = override_height;
        }
        None => {
            let (project_width, project_height) = project_render_dimensions(root)?;
            *width = project_width;
            *height = project_height;
        }
    }
    *fps = candidate.manifest.render.fps;

    if let Some(saved) = saved {
        runtime.set_fixed_state(saved.time, saved.frame, saved.fps)?;
        for (name, (saved_dimensions, data)) in saved.buffers {
            if let Some(pass) = candidate.manifest.passes.iter().find(|pass| {
                pass.name == name
                    && matches!(
                        pass.kind,
                        PassKind::Buffer | PassKind::Compute | PassKind::Mesh
                    )
            }) {
                let (pass_width, pass_height) =
                    candidate.manifest.pass_dimensions(pass, *width, *height);
                if (pass_width, pass_height) == (saved_dimensions.width, saved_dimensions.height) {
                    let _ = runtime.restore_pass_rgba32f(&name, pass_width, pass_height, &data);
                }
            }
        }
    }

    let final_pass = candidate.manifest.final_pass().name.clone();
    if !candidate
        .manifest
        .passes
        .iter()
        .any(|pass| pass.name == *view)
    {
        *view = final_pass;
    }
    *loaded = Some(candidate);
    *sources = Some(candidate_sources);
    Ok(())
}

pub(super) fn project_render_dimensions(root: &Path) -> Result<(u32, u32)> {
    let base = LoadedManifest::load_with_preset(root, None)?;
    Ok((base.manifest.render.width, base.manifest.render.height))
}

pub(super) fn reload_changed_sources(
    root: &Path,
    runtime: &mut Runtime<'_>,
    loaded: &LoadedManifest,
    sources: &mut SourceGraph,
    changed: &BTreeSet<PathBuf>,
) -> Result<bool> {
    let manifest_path = root.join(crate::manifest::MANIFEST_NAME);
    if changed.iter().any(|path| same_path(path, &manifest_path)) {
        return Ok(false);
    }

    let mut impacted = Vec::new();
    for pass in &loaded.manifest.passes {
        let Some(current) = sources.get(&pass.name) else {
            return Ok(false);
        };
        if changed.iter().any(|path| {
            current
                .dependencies
                .iter()
                .any(|dependency| same_path(path, dependency))
        }) {
            impacted.push(pass);
        }
    }

    if impacted.is_empty() {
        let source_like = changed.iter().all(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("glsl" | "frag" | "vert" | "comp")
            )
        });
        return Ok(source_like);
    }

    if impacted.iter().any(|pass| pass.kind == PassKind::Mesh) {
        // Mesh hot reload changes a linked vertex+fragment program. Rebuild the
        // project transactionally rather than partially replacing one stage.
        return Ok(false);
    }

    let mut replacements = Vec::with_capacity(impacted.len());
    for pass in impacted {
        replacements.push((pass.name.clone(), expand_pass(loaded, pass)?));
    }

    let mut applied: Vec<String> = Vec::new();
    for (name, replacement) in &replacements {
        if let Err(error) = runtime.reload_pass_source(name, &replacement.text) {
            for previous_name in applied.iter().rev() {
                if let Some(previous) = sources.get(previous_name) {
                    let _ = runtime.reload_pass_source(previous_name, &previous.text);
                }
            }
            return Err(error.into());
        }
        applied.push(name.clone());
    }
    for (name, replacement) in replacements {
        sources.insert(name, replacement);
    }
    Ok(true)
}

fn same_path(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalize(left) == normalize(right)
}

fn save_runtime_state(
    runtime: &mut Runtime<'_>,
    loaded: &LoadedManifest,
    width: u32,
    height: u32,
    fps: f32,
) -> Result<SavedRuntimeState> {
    let mut buffers = BTreeMap::new();
    for pass in &loaded.manifest.passes {
        if !matches!(
            pass.kind,
            PassKind::Buffer | PassKind::Compute | PassKind::Mesh
        ) {
            continue;
        }
        let (pass_width, pass_height) = loaded.manifest.pass_dimensions(pass, width, height);
        if let Ok(data) = runtime.snapshot_pass_rgba32f(&pass.name, pass_width, pass_height) {
            buffers.insert(
                pass.name.clone(),
                (
                    crate::state::BufferDimensions {
                        width: pass_width,
                        height: pass_height,
                    },
                    data,
                ),
            );
        }
    }
    Ok(SavedRuntimeState {
        time: runtime.time(),
        frame: runtime.frame(),
        fps,
        buffers,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_status(
    shared: &Shared,
    loaded: Option<&LoadedManifest>,
    runtime: &Runtime<'_>,
    width: u32,
    height: u32,
    custom_resolution: bool,
    fps: f32,
    paused: bool,
    view: &str,
    uniform_values: &BTreeMap<String, crate::uniforms::UniformValue>,
    error: Option<String>,
    increment_sequence: bool,
) {
    let mut status = shared.status.write().expect("preview status lock poisoned");
    if let Some(loaded) = loaded {
        status.project = loaded.manifest.project.name.clone();
        status.presets = loaded.manifest.presets.keys().cloned().collect();
        status.final_pass = loaded.manifest.final_pass().name.clone();
        status.passes = loaded
            .manifest
            .passes
            .iter()
            .filter(|pass| !matches!(pass.kind, PassKind::Cubemap | PassKind::Sound))
            .map(|pass| pass.name.clone())
            .collect();
        status.uniforms = loaded
            .manifest
            .uniforms
            .iter()
            .map(|definition| PreviewUniformStatus {
                name: definition.name().to_string(),
                kind: definition.kind_name().to_string(),
                value: uniform_values
                    .get(definition.name())
                    .cloned()
                    .unwrap_or_else(|| definition.default_value()),
                min: definition.preview_min(),
                max: definition.preview_max(),
                step: definition.preview_step(),
            })
            .collect();
        status.webcam = crate::media::manifest_uses_webcam(loaded);
        status.preset_width = loaded.manifest.render.width;
        status.preset_height = loaded.manifest.render.height;
    }
    status.frame = runtime.frame();
    status.time = runtime.time();
    status.paused = paused;
    status.time_scale = runtime.time_scale();
    status.width = width;
    status.height = height;
    status.custom_resolution = custom_resolution;
    status.fps = fps;
    status.view = view.to_string();
    if error.is_some() {
        status.error = error;
    } else if increment_sequence {
        status.error = None;
    }
    if increment_sequence {
        status.sequence = status.sequence.wrapping_add(1);
    }
    broadcast_status(shared, &status);
}

pub(super) fn set_error(shared: &Shared, message: String) {
    let mut status = shared.status.write().expect("preview status lock poisoned");
    status.error = Some(message);
    broadcast_status(shared, &status);
}

pub(super) fn clear_error(shared: &Shared) {
    let mut status = shared.status.write().expect("preview status lock poisoned");
    status.error = None;
    broadcast_status(shared, &status);
}

fn broadcast_status(shared: &Shared, status: &PreviewStatus) {
    if let Ok(message) = serde_json::to_string(status) {
        let _ = shared.updates.send(message);
    }
}

pub(super) fn reload_event_kind(kind: &notify::EventKind) -> bool {
    matches!(
        kind,
        notify::EventKind::Any
            | notify::EventKind::Create(_)
            | notify::EventKind::Modify(_)
            | notify::EventKind::Remove(_)
    )
}

pub(super) fn relevant_watch_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let Some(first) = relative.components().next() else {
        return false;
    };
    let first = first.as_os_str().to_string_lossy();
    if matches!(first.as_ref(), "target" | ".git" | ".shadertoy") {
        return false;
    }
    true
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn preset_switch_matches_cold_state_and_keeps_explicit_uniforms() {
        let temp = tempfile::tempdir().expect("create temp project");
        let root = temp.path();
        fs::create_dir_all(root.join("shaders")).expect("create shaders");
        fs::write(
            root.join(crate::manifest::MANIFEST_NAME),
            r#"format = 1

[project]
name = "preview-preset-reset"

[render]
width = 2
height = 2
fps = 60.0
preview_time = 0.0

[[uniform]]
name = "gain"
type = "float"
default = 1.0
min = 0.0
max = 2.0

[[pass]]
name = "history"
kind = "compute"
source = "shaders/history.comp"
width = 2
height = 2
local_size = [1, 1, 1]

[[pass.input]]
channel = 0
source = "history"
frame = "previous"
filter = "nearest"
wrap = "clamp"

[[pass]]
name = "aux"
kind = "buffer"
source = "shaders/aux.frag"
width = 2
height = 2

[[pass]]
name = "image"
kind = "image"
source = "shaders/image.frag"

[[pass.input]]
channel = 0
source = "history"
frame = "current"
filter = "nearest"
wrap = "clamp"

[preset.ultra]

[preset.low.pass.aux]
width = 1
height = 1
"#,
        )
        .expect("write manifest");
        fs::write(
            root.join("shaders/history.comp"),
            r#"void mainCompute(ivec2 coord) {
    float previous = texture(iChannel0, vec2(0.5)).r;
    imageStore(iOutput, coord, vec4(previous + 0.1 * gain, 0.0, 0.0, 1.0));
}
"#,
        )
        .expect("write history shader");
        fs::write(
            root.join("shaders/aux.frag"),
            "void mainImage(out vec4 c, in vec2 p) { c = vec4(0.0); }\n",
        )
        .expect("write aux shader");
        fs::write(
            root.join("shaders/image.frag"),
            "void mainImage(out vec4 c, in vec2 p) { c = texture(iChannel0, p / iResolution.xy); }\n",
        )
        .expect("write image shader");

        let context = HeadlessContext::new(64, 64).expect("create headless context");
        let mut runtime = Runtime::new(&context).expect("create runtime");
        let mut loaded = None;
        let mut sources = None;
        let mut width = 0;
        let mut height = 0;
        let mut fps = 0.0;
        let mut view = "image".to_string();
        let mut uniform_values = BTreeMap::new();

        reload(
            root,
            Some("ultra"),
            &mut runtime,
            &mut loaded,
            &mut sources,
            &mut width,
            &mut height,
            None,
            &mut fps,
            &mut view,
            &mut uniform_values,
            None,
            ReloadPreservation::RESET,
        )
        .expect("load cold ultra");
        let cold_ultra = runtime.render(width, height).expect("render cold ultra");

        let explicit_gain = crate::uniforms::UniformValue::Float(0.5);
        uniform_values.insert("gain".into(), explicit_gain.clone());
        crate::uniforms::apply_to_runtime(&mut runtime, &uniform_values)
            .expect("apply explicit uniform");

        reload(
            root,
            Some("low"),
            &mut runtime,
            &mut loaded,
            &mut sources,
            &mut width,
            &mut height,
            None,
            &mut fps,
            &mut view,
            &mut uniform_values,
            None,
            ReloadPreservation::PRESET_SWITCH,
        )
        .expect("switch to low");
        assert_eq!(uniform_values.get("gain"), Some(&explicit_gain));
        for _ in 0..3 {
            runtime.render(width, height).expect("advance low history");
        }

        reload(
            root,
            Some("ultra"),
            &mut runtime,
            &mut loaded,
            &mut sources,
            &mut width,
            &mut height,
            None,
            &mut fps,
            &mut view,
            &mut uniform_values,
            None,
            ReloadPreservation::PRESET_SWITCH,
        )
        .expect("switch back to ultra");
        let switched_ultra = runtime
            .render(width, height)
            .expect("render switched ultra");

        // The explicit 0.5 gain survives, so compare against an independent cold
        // Ultra runtime with the same user value rather than the manifest default.
        let mut cold_runtime = Runtime::new(&context).expect("create comparison runtime");
        let mut cold_loaded = None;
        let mut cold_sources = None;
        let mut cold_width = 0;
        let mut cold_height = 0;
        let mut cold_fps = 0.0;
        let mut cold_view = "image".to_string();
        let mut cold_uniforms = BTreeMap::new();
        reload(
            root,
            Some("ultra"),
            &mut cold_runtime,
            &mut cold_loaded,
            &mut cold_sources,
            &mut cold_width,
            &mut cold_height,
            None,
            &mut cold_fps,
            &mut cold_view,
            &mut cold_uniforms,
            None,
            ReloadPreservation::RESET,
        )
        .expect("load independent cold ultra");
        cold_uniforms.insert("gain".into(), explicit_gain);
        crate::uniforms::apply_to_runtime(&mut cold_runtime, &cold_uniforms)
            .expect("apply comparison uniform");
        let expected = cold_runtime
            .render(cold_width, cold_height)
            .expect("render comparison ultra");

        assert_eq!(switched_ultra.pixels, expected.pixels);
        assert_ne!(cold_ultra.pixels, expected.pixels);
        assert_eq!(runtime.frame(), 0, "preset switch must reset frame history");
        assert_eq!(runtime.time(), 0.0, "preset switch must reset shader time");
    }
}
