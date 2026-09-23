use super::*;

#[allow(clippy::too_many_arguments)]
pub fn capture_state(
    project_path: &Path,
    output: &Path,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f32>,
    frame: Option<i32>,
    time: Option<f32>,
    include_storage: bool,
    set_uniforms: &[String],
) -> Result<Output> {
    let loaded = LoadedManifest::load(project_path)?;
    ensure_source_files_exist(&loaded)?;
    let media = crate::media::MediaInputs::new_headless(&loaded)?;
    let width = width.unwrap_or(loaded.manifest.render.width);
    let height = height.unwrap_or(loaded.manifest.render.height);
    validate_dimensions(width, height)?;
    let fps = fps.unwrap_or(loaded.manifest.render.fps);
    validate_fps(fps)?;
    let target_frame = resolve_target_frame(&loaded, None, frame, time, fps)?;

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for state capture")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_native_project(&loaded)?;
    runtime.load_project(&project)?;
    let uniform_values =
        crate::uniforms::parse_assignments(&loaded.manifest.uniforms, set_uniforms)?;
    crate::uniforms::apply_to_runtime(&mut runtime, &uniform_values)?;
    let _ = render_from_zero(&mut runtime, target_frame, fps, width, height, &[], &media)?;

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
        match runtime.snapshot_pass_rgba32f(&pass.name, pass_width, pass_height) {
            Ok(data) => {
                buffers.insert(pass.name.clone(), data);
                buffer_dimensions.insert(
                    pass.name.clone(),
                    crate::state::BufferDimensions {
                        width: pass_width,
                        height: pass_height,
                    },
                );
                buffer_formats.insert(pass.name.clone(), pass.format);
            }
            Err(error) => {
                // Unreachable/unused passes are not compiled into the execution pipeline.
                eprintln!("warning: state capture skipped '{}': {error}", pass.name);
            }
        }
    }

    let mut storage_buffers = BTreeMap::new();
    if include_storage {
        let mut sizes = BTreeMap::new();
        for pass in &loaded.manifest.passes {
            for storage in &pass.storage {
                sizes.entry(storage.name.clone()).or_insert(storage.size);
            }
        }
        for (name, size) in sizes {
            let size = usize::try_from(size).context("storage buffer size exceeds usize")?;
            match runtime.snapshot_storage_buffer(&name, size) {
                Ok(data) => {
                    storage_buffers.insert(name, data);
                }
                Err(error) => {
                    eprintln!("warning: state capture skipped storage '{name}': {error}");
                }
            }
        }
    }

    let state = StateFile::new(
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
    )?;
    state.save(output)?;

    Ok(Output {
        human: format!(
            "Captured frame {} ({:.3}s), {} buffers, {} storage buffers -> {}",
            state.header.frame,
            state.header.time,
            state.header.buffers.len(),
            state.header.storage_buffers.len(),
            output.display()
        ),
        json: json!({
            "ok": true,
            "action": "state-capture",
            "output": output,
            "project": state.header.project,
            "width": state.header.width,
            "height": state.header.height,
            "fps": state.header.fps,
            "frame": state.header.frame,
            "time": state.header.time,
            "buffers": state.header.buffers,
            "storage_buffers": state.header.storage_buffers,
        }),
    })
}

pub fn inspect_state(path: &Path) -> Result<Output> {
    let header = StateFile::inspect_header(path)?;
    Ok(Output {
        human: format!(
            "State: {}\nProject: {}\nFrame: {}\nTime: {:.3}s\nRender: {}x{} @ {} fps\nBuffers: {}",
            path.display(),
            header.project,
            header.frame,
            header.time,
            header.width,
            header.height,
            header.fps,
            if header.buffers.is_empty() {
                "(none)".to_string()
            } else {
                header.buffers.join(", ")
            },
        ),
        json: json!({
            "ok": true,
            "state": path,
            "header": header,
        }),
    })
}

pub fn set_state_buffers(input: &Path, output: &Path, assignments: &[String]) -> Result<Output> {
    if assignments.is_empty() {
        bail!("state set requires at least one BUFFER=IMAGE assignment");
    }
    let mut state = StateFile::load(input)?;
    let mut changed = Vec::new();
    for assignment in assignments {
        let (name, path) = split_assignment(assignment)?;
        let image = ImageReader::open(path)
            .with_context(|| format!("failed to open replacement image {}", path.display()))?
            .decode()
            .with_context(|| format!("failed to decode replacement image {}", path.display()))?
            .to_rgba8();
        let (width, height) = image.dimensions();
        let mut rgba = image.into_raw();
        flip_rgba_rows(&mut rgba, width, height);
        state.replace_buffer_rgba8(name, width, height, &rgba)?;
        changed.push(name.to_string());
    }
    state.save(output)?;
    Ok(Output {
        human: format!("Updated {} -> {}", changed.join(", "), output.display()),
        json: json!({
            "ok": true,
            "action": "state-set",
            "input": input,
            "output": output,
            "buffers": changed,
        }),
    })
}

pub fn set_state_storage(input: &Path, output: &Path, assignments: &[String]) -> Result<Output> {
    if assignments.is_empty() {
        bail!("state set-storage requires at least one STORAGE=BINARY assignment");
    }
    let mut state = StateFile::load(input)?;
    let mut changed = Vec::new();
    for assignment in assignments {
        let (name, path) = split_assignment(assignment)?;
        let data = fs::read(path)
            .with_context(|| format!("failed to read replacement storage {}", path.display()))?;
        state.replace_storage_buffer(name, data)?;
        changed.push(name.to_string());
    }
    state.save(output)?;
    Ok(Output {
        human: format!(
            "Updated storage {} -> {}",
            changed.join(", "),
            output.display()
        ),
        json: json!({
            "ok": true,
            "action": "state-set-storage",
            "input": input,
            "output": output,
            "storage_buffers": changed,
        }),
    })
}
