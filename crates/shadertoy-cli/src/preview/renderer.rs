use super::state::{
    ReloadPreservation, clear_error, project_render_dimensions, reload, reload_changed_sources,
    set_error, update_status,
};
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_loop(
    root: PathBuf,
    preset: Option<String>,
    shared: Shared,
    controls: mpsc::Receiver<Control>,
    preserve_reload_state: bool,
    wireframe: Option<String>,
    runtime: &mut Runtime<'_>,
    mut recorder: Option<ReplayRecorder>,
) {
    let mut preset = preset;
    let mut loaded: Option<LoadedManifest> = None;
    let mut sources: Option<SourceGraph> = None;
    let mut changed_paths = BTreeSet::new();
    let mut width = 1280u32;
    let mut height = 720u32;
    let mut resolution_override: Option<(u32, u32)> = None;
    let mut fps = 60.0f32;
    let mut paused = false;
    let mut view = String::from("image");
    let mut uniform_values = BTreeMap::new();
    let mut media = crate::media::MediaInputs::default();
    let mut fresh = true;
    let mut force_render = true;
    let mut reload_due: Option<Instant> = None;
    let mut next_frame = Instant::now();

    match reload(
        &root,
        preset.as_deref(),
        runtime,
        &mut loaded,
        &mut sources,
        &mut width,
        &mut height,
        resolution_override,
        &mut fps,
        &mut view,
        &mut uniform_values,
        wireframe.as_deref(),
        if preserve_reload_state {
            ReloadPreservation::RUNTIME_STATE
        } else {
            ReloadPreservation::RESET
        },
    ) {
        Ok(()) => {
            if let Some(project) = loaded.as_ref()
                && let Ok(next_media) = crate::media::MediaInputs::new(project)
            {
                media = next_media;
            }
            update_status(
                &shared,
                loaded.as_ref(),
                runtime,
                width,
                height,
                resolution_override.is_some(),
                fps,
                paused,
                &view,
                &uniform_values,
                None,
                false,
            );
        }
        Err(error) => {
            update_status(
                &shared,
                loaded.as_ref(),
                runtime,
                width,
                height,
                resolution_override.is_some(),
                fps,
                paused,
                &view,
                &uniform_values,
                Some(error.to_string()),
                false,
            );
        }
    }
    'main: loop {
        let mut pending = Vec::new();
        while let Ok(control) = controls.try_recv() {
            pending.push(control);
        }
        for control in pending {
            if !handle_control(
                control,
                &root,
                &mut preset,
                &shared,
                runtime,
                &mut recorder,
                &mut loaded,
                &mut sources,
                &mut changed_paths,
                &mut width,
                &mut height,
                &mut resolution_override,
                &mut fps,
                &mut paused,
                &mut view,
                &mut uniform_values,
                wireframe.as_deref(),
                &mut fresh,
                &mut force_render,
                &mut reload_due,
                &mut next_frame,
            ) {
                break 'main;
            }
        }

        if let Some(due) = reload_due
            && Instant::now() >= due
        {
            reload_due = None;
            let changed = std::mem::take(&mut changed_paths);
            let incremental = match (&loaded, &mut sources) {
                (Some(current), Some(current_sources)) => {
                    reload_changed_sources(&root, runtime, current, current_sources, &changed)
                }
                _ => Ok(false),
            };
            match incremental {
                Ok(true) => {
                    force_render = true;
                    clear_error(&shared);
                }
                Ok(false) => {
                    match reload(
                        &root,
                        preset.as_deref(),
                        runtime,
                        &mut loaded,
                        &mut sources,
                        &mut width,
                        &mut height,
                        resolution_override,
                        &mut fps,
                        &mut view,
                        &mut uniform_values,
                        wireframe.as_deref(),
                        if preserve_reload_state {
                            ReloadPreservation::RUNTIME_STATE
                        } else {
                            ReloadPreservation::RESET
                        },
                    ) {
                        Ok(()) => {
                            if let Some(project) = loaded.as_ref() {
                                match crate::media::MediaInputs::new(project) {
                                    Ok(next_media) => media = next_media,
                                    Err(error) => set_error(&shared, format!("{error:#}")),
                                }
                            }
                            fresh = true;
                            force_render = true;
                            clear_error(&shared);
                        }
                        Err(error) => {
                            // Full project reload compiles before replacing the active pipeline.
                            // Keep rendering the last good project and frame.
                            set_error(&shared, format!("{error:#}"));
                        }
                    }
                }
                Err(error) => {
                    // Per-pass reload is transactional and rolls back earlier impacted passes.
                    set_error(&shared, format!("{error:#}"));
                }
            }
        }

        let now = Instant::now();
        let clients = shared.clients.load(Ordering::Relaxed);
        let scheduled = !paused && clients > 0 && now >= next_frame;
        if loaded.is_some() && (force_render || scheduled) {
            let should_advance = !fresh && !paused && scheduled;
            if should_advance && let Err(error) = runtime.tick_fixed(1.0 / fps, fps) {
                set_error(&shared, error.to_string());
            }

            let media_time = runtime.time();
            if let Err(error) = media.update(runtime, media_time) {
                set_error(&shared, format!("{error:#}"));
            }
            match runtime.render(width, height) {
                Ok(final_image) => {
                    let selected = if let Some(project) = &loaded {
                        if view == project.manifest.final_pass().name {
                            Some(final_image)
                        } else {
                            let dimensions = project
                                .manifest
                                .passes
                                .iter()
                                .find(|pass| pass.name == view)
                                .map(|pass| project.manifest.pass_dimensions(pass, width, height));
                            match dimensions {
                                Some((pass_width, pass_height)) => {
                                    match runtime.snapshot_pass_rgb(&view, pass_width, pass_height)
                                    {
                                        Ok(image) => Some(image),
                                        Err(error) => {
                                            set_error(&shared, error.to_string());
                                            None
                                        }
                                    }
                                }
                                None => {
                                    set_error(&shared, format!("unknown preview pass '{view}'"));
                                    None
                                }
                            }
                        }
                    } else {
                        None
                    };

                    if let Some(image) = selected {
                        shared.frame_submitter.submit(image);
                        update_status(
                            &shared,
                            loaded.as_ref(),
                            runtime,
                            width,
                            height,
                            resolution_override.is_some(),
                            fps,
                            paused,
                            &view,
                            &uniform_values,
                            None,
                            true,
                        );
                    }
                    if let Some(recorder) = &mut recorder
                        && let Err(error) = recorder.record_frame(
                            runtime.frame(),
                            runtime.time(),
                            runtime.time_delta(),
                            runtime.frame_rate(),
                        )
                    {
                        set_error(&shared, format!("replay recording failed: {error:#}"));
                    }
                    let _ = runtime.clear_key_transients();
                }
                Err(error) => set_error(&shared, error.to_string()),
            }
            fresh = false;
            force_render = false;
            next_frame = next_frame_deadline_after_render(next_frame, Instant::now(), fps);
        }

        let timeout = if !paused && shared.clients.load(Ordering::Relaxed) > 0 {
            next_frame
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50))
        } else {
            Duration::from_millis(50)
        };
        match controls.recv_timeout(timeout) {
            Ok(control) => {
                if !handle_control(
                    control,
                    &root,
                    &mut preset,
                    &shared,
                    runtime,
                    &mut recorder,
                    &mut loaded,
                    &mut sources,
                    &mut changed_paths,
                    &mut width,
                    &mut height,
                    &mut resolution_override,
                    &mut fps,
                    &mut paused,
                    &mut view,
                    &mut uniform_values,
                    wireframe.as_deref(),
                    &mut fresh,
                    &mut force_render,
                    &mut reload_due,
                    &mut next_frame,
                ) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if let Some(recorder) = &mut recorder
        && let Err(error) = recorder.flush()
    {
        set_error(&shared, format!("replay recording flush failed: {error:#}"));
    }
}

fn next_frame_deadline_after_render(
    previous_deadline: Instant,
    completed_at: Instant,
    fps: f32,
) -> Instant {
    let interval = Duration::from_secs_f64(1.0 / f64::from(fps.max(1.0)));
    let scheduled = previous_deadline + interval;
    if scheduled <= completed_at {
        completed_at
    } else {
        scheduled
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_control(
    control: Control,
    root: &Path,
    preset: &mut Option<String>,
    shared: &Shared,
    runtime: &mut Runtime<'_>,
    recorder: &mut Option<ReplayRecorder>,
    loaded: &mut Option<LoadedManifest>,
    sources: &mut Option<SourceGraph>,
    changed_paths: &mut BTreeSet<PathBuf>,
    width: &mut u32,
    height: &mut u32,
    resolution_override: &mut Option<(u32, u32)>,
    fps: &mut f32,
    paused: &mut bool,
    view: &mut String,
    uniform_values: &mut BTreeMap<String, crate::uniforms::UniformValue>,
    wireframe: Option<&str>,
    fresh: &mut bool,
    force_render: &mut bool,
    reload_due: &mut Option<Instant>,
    next_frame: &mut Instant,
) -> bool {
    match control {
        Control::Shutdown => return false,
        Control::FilesChanged(paths) => {
            if let Some(recorder) = recorder
                && let Err(error) = recorder.invalidate(
                    "project files changed during recording; restart preview to capture a reproducible session",
                )
            {
                set_error(shared, format!("replay recording invalidation failed: {error:#}"));
            }
            changed_paths.extend(paths);
            *reload_due = Some(Instant::now() + Duration::from_millis(120));
        }
        Control::Pause => {
            if let Err(error) = runtime.pause() {
                set_error(shared, error.to_string());
            } else {
                *paused = true;
                *force_render = true;
            }
        }
        Control::Resume => {
            if let Err(error) = runtime.resume() {
                set_error(shared, error.to_string());
            } else {
                *paused = false;
                *next_frame = Instant::now();
            }
        }
        Control::Reset => match reload(
            root,
            preset.as_deref(),
            runtime,
            loaded,
            sources,
            width,
            height,
            *resolution_override,
            fps,
            view,
            uniform_values,
            wireframe,
            ReloadPreservation::RESET,
        ) {
            Ok(()) => {
                *fresh = true;
                *force_render = true;
                clear_error(shared);
                record_action(recorder, shared, ReplayAction::Reset);
            }
            Err(error) => set_error(shared, error.to_string()),
        },
        Control::Step => {
            let was_paused = *paused;
            if was_paused {
                let _ = runtime.resume();
            }
            if let Err(error) = runtime.tick_fixed(1.0 / *fps, *fps) {
                set_error(shared, error.to_string());
            } else {
                *fresh = true;
                *force_render = true;
            }
            if was_paused {
                let _ = runtime.pause();
            }
        }
        Control::Preset(requested) => {
            if requested == *preset {
                clear_error(shared);
            } else {
                if let Some(recorder) = recorder
                    && let Err(error) = recorder.invalidate(
                        "quality preset changed during recording; restart preview to capture a reproducible session",
                    )
                {
                    set_error(
                        shared,
                        format!("replay recording invalidation failed: {error:#}"),
                    );
                }
                match reload(
                    root,
                    requested.as_deref(),
                    runtime,
                    loaded,
                    sources,
                    width,
                    height,
                    *resolution_override,
                    fps,
                    view,
                    uniform_values,
                    wireframe,
                    // A quality preset is a different graph configuration, not a
                    // hot reload of the same simulation. Rebuild from zeroed GPU
                    // resources and frame history just like a cold preview while
                    // keeping explicit custom-uniform choices made by the reviewer.
                    ReloadPreservation::PRESET_SWITCH,
                ) {
                    Ok(()) => {
                        *preset = requested;
                        if let Ok(mut status) = shared.status.write() {
                            status.preset = preset.clone();
                        }
                        *fresh = true;
                        *force_render = true;
                        *next_frame = Instant::now();
                        clear_error(shared);
                    }
                    Err(error) => set_error(shared, format!("{error:#}")),
                }
            }
        }
        Control::View(pass) => {
            if let Some(project) = loaded {
                if project.manifest.passes.iter().any(|candidate| {
                    candidate.name == pass
                        && !matches!(candidate.kind, PassKind::Cubemap | PassKind::Sound)
                }) {
                    *view = pass;
                    *force_render = true;
                } else {
                    set_error(shared, format!("unknown/non-2D preview pass '{pass}'"));
                }
            }
        }
        Control::Resolution(new_width, new_height) => {
            if new_width == 0
                || new_height == 0
                || new_width > MAX_PREVIEW_DIMENSION
                || new_height > MAX_PREVIEW_DIMENSION
            {
                set_error(
                    shared,
                    format!(
                        "preview resolution must be between 1x1 and {0}x{0}",
                        MAX_PREVIEW_DIMENSION
                    ),
                );
            } else {
                *width = new_width;
                *height = new_height;
                *resolution_override = Some((new_width, new_height));
                // Keep the active runtime so fixed-size offscreen targets and their
                // feedback history survive output-resolution changes. Dynamic
                // output-sized buffers will resize on their next render.
                *force_render = true;
                clear_error(shared);
                record_action(
                    recorder,
                    shared,
                    ReplayAction::Resolution {
                        width: new_width,
                        height: new_height,
                    },
                );
            }
        }
        Control::ResolutionDefault => match project_render_dimensions(root) {
            Ok((new_width, new_height)) => {
                *width = new_width;
                *height = new_height;
                *resolution_override = None;
                *force_render = true;
                clear_error(shared);
                record_action(
                    recorder,
                    shared,
                    ReplayAction::Resolution {
                        width: new_width,
                        height: new_height,
                    },
                );
            }
            Err(error) => set_error(shared, format!("{error:#}")),
        },
        Control::TimeScale(value) => {
            if value.is_finite() && (-8.0..=8.0).contains(&value) {
                if let Err(error) = runtime.set_time_scale(value) {
                    set_error(shared, error.to_string());
                } else {
                    *force_render = true;
                    record_action(recorder, shared, ReplayAction::TimeScale { value });
                }
            } else {
                set_error(
                    shared,
                    "time scale must be a finite log2 value in [-8, 8]".into(),
                );
            }
        }
        Control::Uniform { name, value } => {
            let result = (|| -> Result<()> {
                let project = loaded.as_ref().context("preview project is not loaded")?;
                let definition = project
                    .manifest
                    .uniforms
                    .iter()
                    .find(|definition| definition.name() == name)
                    .with_context(|| format!("unknown custom uniform '{name}'"))?;
                definition.validate_value(&value)?;
                let mut one = BTreeMap::new();
                one.insert(name.clone(), value.clone());
                crate::uniforms::apply_to_runtime(runtime, &one)?;
                uniform_values.insert(name.clone(), value.clone());
                Ok(())
            })();
            match result {
                Ok(()) => {
                    *force_render = true;
                    clear_error(shared);
                    record_action(recorder, shared, ReplayAction::Uniform { name, value });
                }
                Err(error) => set_error(shared, format!("{error:#}")),
            }
        }
        Control::WebcamFrame(rgba) => {
            let result = (|| -> Result<()> {
                let project = loaded.as_ref().context("preview project is not loaded")?;
                if !crate::media::manifest_uses_webcam(project) {
                    bail!("project does not declare a webcam input");
                }
                runtime.update_texture_rgba8(
                    crate::media::WEBCAM_NAME,
                    crate::media::WEBCAM_WIDTH,
                    crate::media::WEBCAM_HEIGHT,
                    &rgba,
                )?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    *force_render = true;
                    clear_error(shared);
                }
                Err(error) => set_error(shared, format!("{error:#}")),
            }
        }
        Control::Mouse {
            x,
            y,
            down,
            clicked,
        } => {
            if let Err(error) = runtime.set_mouse(x, y, down, clicked) {
                set_error(shared, error.to_string());
            } else {
                record_action(
                    recorder,
                    shared,
                    ReplayAction::Mouse {
                        x,
                        y,
                        down,
                        clicked,
                    },
                );
                if *paused {
                    *force_render = true;
                }
            }
        }
        Control::Key {
            code,
            down,
            pressed,
        } => {
            if let Err(error) = runtime.set_key(code, down, pressed) {
                set_error(shared, error.to_string());
            } else {
                record_action(
                    recorder,
                    shared,
                    ReplayAction::Key {
                        code,
                        down,
                        pressed,
                    },
                );
                if *paused {
                    *force_render = true;
                }
            }
        }
    }
    true
}

fn record_action(recorder: &mut Option<ReplayRecorder>, shared: &Shared, action: ReplayAction) {
    if let Some(recorder) = recorder
        && let Err(error) = recorder.record(action)
    {
        set_error(shared, format!("replay recording failed: {error:#}"));
    }
}

#[cfg(test)]
mod pacing_tests {
    use super::*;

    #[test]
    fn render_work_does_not_get_added_to_the_frame_interval() {
        let start = Instant::now();
        let interval = Duration::from_secs_f64(1.0 / 60.0);
        let completed = start + Duration::from_millis(10);

        let next = next_frame_deadline_after_render(start, completed, 60.0);

        assert_eq!(next, start + interval);
    }

    #[test]
    fn over_budget_render_can_continue_without_an_extra_frame_sleep() {
        let start = Instant::now();
        let completed = start + Duration::from_millis(25);

        let next = next_frame_deadline_after_render(start, completed, 60.0);

        assert!(next <= completed);
    }
}
