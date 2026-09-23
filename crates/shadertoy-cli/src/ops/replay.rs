use super::*;
use crate::replay::{ReplayAction, ReplayFile, project_fingerprint};

pub fn replay_project(options: &ReplayOptions) -> Result<Output> {
    let recording = ReplayFile::load(&options.recording)?;
    let loaded = LoadedManifest::load(&options.project)?;
    ensure_source_files_exist(&loaded)?;
    let media = crate::media::MediaInputs::new_headless(&loaded)?;
    if recording.project != loaded.manifest.project.name {
        bail!(
            "replay belongs to project '{}' but current project is '{}'",
            recording.project,
            loaded.manifest.project.name
        );
    }
    let fingerprint = project_fingerprint(&loaded)?;
    if fingerprint != recording.project_fingerprint && !options.allow_project_changes {
        bail!(
            "project contents differ from the recorded session (recorded {}, current {}); use --allow-project-changes only if this is intentional",
            recording.project_fingerprint,
            fingerprint
        );
    }
    if recording.frames.is_empty() {
        bail!("replay contains no rendered frames");
    }

    let target = options.frame.unwrap_or(recording.frames.len() as u64 - 1);
    if target >= recording.frames.len() as u64 {
        bail!(
            "requested replay frame {} exceeds the recorded timeline (last frame {})",
            target,
            recording.frames.len() - 1
        );
    }

    let output = options
        .output
        .clone()
        .unwrap_or_else(|| loaded.root.join("target/replay.png"));
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    render::validate_dimensions(recording.width, recording.height)?;
    render::validate_fps(recording.fps)?;
    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for replay")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_native_project(&loaded)?;
    runtime.load_project(&project)?;
    let default_uniforms = crate::uniforms::defaults(&loaded.manifest.uniforms);
    crate::uniforms::apply_to_runtime(&mut runtime, &default_uniforms)?;

    let mut width = recording.width;
    let mut height = recording.height;
    let mut event_index = 0usize;
    let mut final_image = None;

    for timeline_frame in 0..=target {
        while event_index < recording.events.len()
            && recording.events[event_index].frame == timeline_frame
        {
            match &recording.events[event_index].action {
                ReplayAction::Reset => {
                    runtime.load_project(&project)?;
                    crate::uniforms::apply_to_runtime(&mut runtime, &default_uniforms)?;
                }
                ReplayAction::Resolution {
                    width: new_width,
                    height: new_height,
                } => {
                    render::validate_dimensions(*new_width, *new_height)?;
                    width = *new_width;
                    height = *new_height;
                }
                ReplayAction::TimeScale { value } => {
                    if !value.is_finite() || !(-8.0..=8.0).contains(value) {
                        bail!("replay contains an invalid time-scale event");
                    }
                    runtime.set_time_scale(*value)?;
                }
                ReplayAction::Uniform { name, value } => {
                    let definition = loaded
                        .manifest
                        .uniforms
                        .iter()
                        .find(|definition| definition.name() == name)
                        .with_context(|| {
                            format!("replay references unknown custom uniform '{name}'")
                        })?;
                    definition.validate_value(value)?;
                    let mut one = BTreeMap::new();
                    one.insert(name.clone(), value.clone());
                    crate::uniforms::apply_to_runtime(&mut runtime, &one)?;
                }
                ReplayAction::Mouse {
                    x,
                    y,
                    down,
                    clicked,
                } => runtime.set_mouse(*x, *y, *down, *clicked)?,
                ReplayAction::Key {
                    code,
                    down,
                    pressed,
                } => runtime.set_key(*code, *down, *pressed)?,
            }
            event_index += 1;
        }

        let marker = recording.frames[timeline_frame as usize];
        runtime.set_replay_state(
            marker.time,
            marker.time_delta,
            marker.runtime_frame,
            marker.frame_rate,
        )?;
        media.update(&mut runtime, marker.time)?;
        let image = runtime.render(width, height)?;
        if timeline_frame == target {
            final_image = Some(image);
        }
        runtime.clear_key_transients()?;
    }

    let selected = options
        .pass
        .as_deref()
        .unwrap_or(&loaded.manifest.final_pass().name);
    let pass = loaded
        .manifest
        .passes
        .iter()
        .find(|pass| pass.name == selected)
        .with_context(|| format!("unknown replay pass '{selected}'"))?;
    if matches!(pass.kind, PassKind::Cubemap | PassKind::Sound) {
        bail!("replay output only supports the final image and 2D buffer/compute/mesh passes");
    }
    let image = if pass.name == loaded.manifest.final_pass().name {
        final_image.context("replay did not render its target frame")?
    } else {
        let (pass_width, pass_height) = loaded.manifest.pass_dimensions(pass, width, height);
        runtime.snapshot_pass_rgb(&pass.name, pass_width, pass_height)?
    };
    save_rgb_png(&image, &output)?;

    let marker = recording.frames[target as usize];
    Ok(Output {
        human: format!(
            "Replayed timeline frame {} -> iFrame {} ({:.3}s), {}x{} pass '{}' -> {}",
            target,
            marker.runtime_frame,
            marker.time,
            image.width,
            image.height,
            pass.name,
            output.display()
        ),
        json: json!({
            "ok": true,
            "project": loaded.manifest.project.name,
            "recording": options.recording,
            "timeline_frame": target,
            "runtime_frame": marker.runtime_frame,
            "time": marker.time,
            "time_delta": marker.time_delta,
            "frame_rate": marker.frame_rate,
            "pass": pass.name,
            "width": image.width,
            "height": image.height,
            "output": output,
            "fingerprint_match": fingerprint == recording.project_fingerprint,
        }),
    })
}
