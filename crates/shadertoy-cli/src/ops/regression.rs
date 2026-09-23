use super::*;
use crate::manifest::TestCase;
use crate::state::BufferDimensions;

mod support;
use support::*;

#[derive(Debug, serde::Serialize)]
struct CaseReport {
    name: String,
    pass: String,
    frame: i32,
    passed: bool,
    reasons: Vec<String>,
    rmse: Option<f64>,
    max_error: Option<f64>,
    changed_fraction: Option<f64>,
    gpu_total_ms: Option<f64>,
    pass_gpu_ms: BTreeMap<String, f64>,
    storage_assertions: usize,
    state_roundtrip: bool,
    artifacts: Vec<PathBuf>,
    reference_updated: bool,
}

pub fn test_project(options: &TestOptions) -> Result<Output> {
    let loaded = LoadedManifest::load_with_preset(&options.project, options.preset.as_deref())?;
    ensure_source_files_exist(&loaded)?;

    let selected = loaded
        .manifest
        .tests
        .iter()
        .filter(|test| {
            options
                .filter
                .as_ref()
                .is_none_or(|filter| test.name.contains(filter))
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        bail!(
            "no [[test]] cases{}",
            options
                .filter
                .as_ref()
                .map(|filter| format!(" match filter '{filter}'"))
                .unwrap_or_default()
        );
    }

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for regression tests")?;
    let mut reports = Vec::with_capacity(selected.len());
    for test in selected {
        reports.push(run_case(&context, &loaded, test, options.update));
    }

    let passed = reports.iter().filter(|report| report.passed).count();
    let failed = reports.len() - passed;
    let mut human = format!("Shader tests: {passed} passed, {failed} failed\n");
    for report in &reports {
        human.push_str(&format!(
            "{} {} (pass {}, frame {})",
            if report.passed { "PASS" } else { "FAIL" },
            report.name,
            report.pass,
            report.frame,
        ));
        if let Some(rmse) = report.rmse {
            human.push_str(&format!(", RMSE={rmse:.6}"));
        }
        if let Some(gpu_ms) = report.gpu_total_ms {
            human.push_str(&format!(", GPU={gpu_ms:.3}ms"));
        }
        if !report.reasons.is_empty() {
            human.push_str(&format!(": {}", report.reasons.join("; ")));
        }
        human.push('\n');
        for artifact in &report.artifacts {
            human.push_str(&format!("  {}\n", artifact.display()));
        }
    }

    Ok(Output {
        human: human.trim_end().to_string(),
        json: json!({
            "ok": failed == 0,
            "project": loaded.manifest.project.name,
            "preset": options.preset,
            "passed": passed,
            "failed": failed,
            "updated": options.update,
            "ci": options.ci,
            "cases": reports,
        }),
    })
}

fn run_case(
    context: &HeadlessContext,
    loaded: &LoadedManifest,
    test: &TestCase,
    update: bool,
) -> CaseReport {
    match run_case_inner(context, loaded, test, update) {
        Ok(report) => report,
        Err(error) => CaseReport {
            name: test.name.clone(),
            pass: test
                .pass
                .clone()
                .unwrap_or_else(|| loaded.manifest.final_pass().name.clone()),
            frame: test.frame.unwrap_or(-1),
            passed: false,
            reasons: vec![format!("{error:#}")],
            rmse: None,
            max_error: None,
            changed_fraction: None,
            gpu_total_ms: None,
            pass_gpu_ms: BTreeMap::new(),
            storage_assertions: 0,
            state_roundtrip: false,
            artifacts: Vec::new(),
            reference_updated: false,
        },
    }
}

#[derive(Debug)]
struct RenderedVariant {
    frame: i32,
    width: u32,
    height: u32,
    image: RgbImage,
    raw: Option<Vec<f32>>,
    gpu_total_ms: Option<f64>,
    pass_gpu_ms: BTreeMap<String, f64>,
    storage: BTreeMap<String, Vec<u8>>,
}

fn run_case_inner(
    context: &HeadlessContext,
    loaded: &LoadedManifest,
    test: &TestCase,
    update: bool,
) -> Result<CaseReport> {
    let fps = loaded.manifest.render.fps;
    render::validate_fps(fps)?;
    let frames = test_frames(loaded, test, fps)?;
    let resolutions = test_resolutions(loaded, test);

    let selected_name = test
        .pass
        .as_deref()
        .unwrap_or(&loaded.manifest.final_pass().name);
    let pass = loaded
        .manifest
        .passes
        .iter()
        .find(|pass| pass.name == selected_name)
        .with_context(|| format!("unknown test pass '{selected_name}'"))?;
    if pass.kind == PassKind::Cubemap {
        bail!("test pass '{selected_name}' is a cubemap; only 2D passes are supported");
    }

    let uniform_values = crate::uniforms::merge_values(&loaded.manifest.uniforms, &test.uniforms)?;
    let reference_uniform_values = if test.reference_uniforms.is_empty() {
        None
    } else {
        Some(crate::uniforms::merge_values(
            &loaded.manifest.uniforms,
            &test.reference_uniforms,
        )?)
    };
    let raw_requested = matches!(
        pass.kind,
        PassKind::Buffer | PassKind::Compute | PassKind::Mesh
    ) && (test.assert_no_nan
        || test.assert_no_inf
        || test.mean_range.is_some()
        || test.assert_deterministic
        || test.assert_resolution_independent);
    let profile_requested = test.max_gpu_ms.is_some() || !test.max_pass_gpu_ms.is_empty();
    let storage_names = test
        .storage_assertions
        .iter()
        .map(|assertion| assertion.name.clone())
        .collect::<Vec<_>>();

    let mut variants = Vec::with_capacity(frames.len() * resolutions.len());
    for frame in &frames {
        for (width, height) in &resolutions {
            variants.push(render_variant(
                context,
                loaded,
                pass,
                *frame,
                fps,
                *width,
                *height,
                &uniform_values,
                raw_requested,
                profile_requested,
                &storage_names,
            )?);
        }
    }

    let mut reasons = Vec::new();
    for variant in &variants {
        if let Some(values) = &variant.raw {
            check_numeric_assertions(test, variant, values, &mut reasons);
        }
        check_performance_assertions(test, variant, &mut reasons);
        check_storage_assertions(loaded, test, variant, &mut reasons)?;
    }

    if test.assert_deterministic {
        for variant in &variants {
            let repeated = render_variant(
                context,
                loaded,
                pass,
                variant.frame,
                fps,
                variant.width,
                variant.height,
                &uniform_values,
                raw_requested,
                false,
                &storage_names,
            )?;
            if let (Some(first), Some(second)) = (&variant.raw, &repeated.raw) {
                let compared = compare_raw(first, second, test.raw_tolerance)?;
                if compared.mismatches != 0 {
                    reasons.push(format!(
                        "frame {} at {}x{} is not deterministic: {} raw values differ (max abs error {:.9})",
                        variant.frame,
                        variant.width,
                        variant.height,
                        compared.mismatches,
                        compared.max_error
                    ));
                }
            } else if variant.image.pixels != repeated.image.pixels {
                reasons.push(format!(
                    "frame {} at {}x{} is not deterministic: RGB output differs between fresh runs",
                    variant.frame, variant.width, variant.height
                ));
            }
            for name in &storage_names {
                if variant.storage.get(name) != repeated.storage.get(name) {
                    reasons.push(format!(
                        "frame {} at {}x{} storage '{}' is not deterministic",
                        variant.frame, variant.width, variant.height, name
                    ));
                }
            }
        }
    }

    if test.assert_resolution_independent {
        for frame in &frames {
            let same_frame = variants
                .iter()
                .filter(|variant| variant.frame == *frame)
                .collect::<Vec<_>>();
            let baseline = same_frame
                .first()
                .and_then(|variant| variant.raw.as_ref())
                .context("resolution-independence test did not capture raw buffer data")?;
            for variant in same_frame.iter().skip(1) {
                let values = variant
                    .raw
                    .as_ref()
                    .context("resolution-independence test did not capture raw buffer data")?;
                let compared = compare_raw(baseline, values, test.raw_tolerance)?;
                if compared.mismatches != 0 {
                    let first = same_frame[0];
                    reasons.push(format!(
                        "frame {} fixed pass depends on output resolution: {}x{} vs {}x{} differ in {} raw values (max abs error {:.9})",
                        frame,
                        first.width,
                        first.height,
                        variant.width,
                        variant.height,
                        compared.mismatches,
                        compared.max_error
                    ));
                }
            }
        }
    }

    if test.assert_state_roundtrip {
        for variant in &variants {
            check_state_roundtrip(
                context,
                loaded,
                variant.frame,
                fps,
                variant.width,
                variant.height,
                &uniform_values,
                test.raw_tolerance,
            )
            .with_context(|| {
                format!(
                    "state round-trip failed at frame {} {}x{}",
                    variant.frame, variant.width, variant.height
                )
            })?;
        }
    }

    let baseline = variants
        .first()
        .context("regression test produced no frame/resolution variants")?;
    let artifact_dir = loaded
        .root
        .join("target/tests")
        .join(file_safe_name(&test.name));
    let mut artifacts = Vec::new();
    let mut rmse = None;
    let mut max_error = None;
    let mut changed_fraction = None;
    let mut reference_updated = false;

    if let Some(reference) = &test.reference {
        let reference_path = safe_reference_path(&loaded.root, reference, update)?;
        if update {
            if let Some(parent) = reference_path.parent() {
                fs::create_dir_all(parent)?;
            }
            save_rgb_png(&baseline.image, &reference_path)?;
            reference_updated = true;
        } else if !reference_path.is_file() {
            reasons.push(format!(
                "reference image is missing: {}",
                reference_path.display()
            ));
        } else {
            let expected = ImageReader::open(&reference_path)
                .with_context(|| format!("failed to open {}", reference_path.display()))?
                .decode()
                .with_context(|| format!("failed to decode {}", reference_path.display()))?
                .to_rgb8();
            if expected.width() != baseline.image.width
                || expected.height() != baseline.image.height
            {
                reasons.push(format!(
                    "reference is {}x{} but baseline variant is {}x{}",
                    expected.width(),
                    expected.height(),
                    baseline.image.width,
                    baseline.image.height
                ));
            } else {
                let actual = top_down_rgb(&baseline.image);
                let comparison = compare_rgb(expected.as_raw(), &actual, test.tolerance);
                rmse = Some(comparison.rmse);
                max_error = Some(comparison.max_error);
                changed_fraction = Some(comparison.changed_fraction);
                enforce_rmse_bounds(test, comparison.rmse, true, "reference image", &mut reasons);

                if !reasons.is_empty() {
                    fs::create_dir_all(&artifact_dir)?;
                    let actual_path = artifact_dir.join("actual.png");
                    save_rgb_png(&baseline.image, &actual_path)?;
                    artifacts.push(actual_path);

                    let expected_path = artifact_dir.join("expected.png");
                    fs::copy(&reference_path, &expected_path)?;
                    artifacts.push(expected_path);

                    let mut diff_bottom_up = comparison.diff;
                    super::images::flip_rgb_rows(
                        &mut diff_bottom_up,
                        baseline.image.width,
                        baseline.image.height,
                    );
                    let diff_path = artifact_dir.join("diff.png");
                    save_rgb_png(
                        &RgbImage::new(baseline.image.width, baseline.image.height, diff_bottom_up),
                        &diff_path,
                    )?;
                    artifacts.push(diff_path);
                }
            }
        }
    }

    if let Some(reference_uniform_values) = reference_uniform_values {
        let mut max_observed_rmse = 0.0f64;
        let mut max_observed_error = 0.0f64;
        let mut max_changed_fraction = 0.0f64;
        for variant in &variants {
            let reference_variant = render_variant(
                context,
                loaded,
                pass,
                variant.frame,
                fps,
                variant.width,
                variant.height,
                &reference_uniform_values,
                false,
                false,
                &[],
            )?;
            let expected = top_down_rgb(&reference_variant.image);
            let actual = top_down_rgb(&variant.image);
            let comparison = compare_rgb(&expected, &actual, test.tolerance);
            let label = format!(
                "reference_uniforms at frame {} {}x{}",
                variant.frame, variant.width, variant.height
            );
            enforce_rmse_bounds(test, comparison.rmse, false, &label, &mut reasons);
            max_observed_rmse = max_observed_rmse.max(comparison.rmse);
            max_observed_error = max_observed_error.max(comparison.max_error);
            max_changed_fraction = max_changed_fraction.max(comparison.changed_fraction);
        }
        rmse = Some(max_observed_rmse);
        max_error = Some(max_observed_error);
        changed_fraction = Some(max_changed_fraction);
    }

    let gpu_total_ms = variants
        .iter()
        .filter_map(|variant| variant.gpu_total_ms)
        .max_by(f64::total_cmp);
    let mut pass_gpu_ms = BTreeMap::new();
    for variant in &variants {
        for (name, value) in &variant.pass_gpu_ms {
            pass_gpu_ms
                .entry(name.clone())
                .and_modify(|current: &mut f64| *current = current.max(*value))
                .or_insert(*value);
        }
    }

    Ok(CaseReport {
        name: test.name.clone(),
        pass: pass.name.clone(),
        frame: baseline.frame,
        passed: reasons.is_empty(),
        reasons,
        rmse,
        max_error,
        changed_fraction,
        gpu_total_ms,
        pass_gpu_ms,
        storage_assertions: test.storage_assertions.len(),
        state_roundtrip: test.assert_state_roundtrip,
        artifacts,
        reference_updated,
    })
}

fn test_frames(loaded: &LoadedManifest, test: &TestCase, fps: f32) -> Result<Vec<i32>> {
    if !test.frames.is_empty() {
        let mut frames = test.frames.clone();
        frames.sort_unstable();
        return Ok(frames);
    }
    Ok(vec![resolve_target_frame(
        loaded, None, test.frame, test.time, fps,
    )?])
}

fn test_resolutions(loaded: &LoadedManifest, test: &TestCase) -> Vec<(u32, u32)> {
    if !test.resolutions.is_empty() {
        return test
            .resolutions
            .iter()
            .map(|[width, height]| (*width, *height))
            .collect();
    }
    vec![(
        test.width.unwrap_or(loaded.manifest.render.width),
        test.height.unwrap_or(loaded.manifest.render.height),
    )]
}

#[allow(clippy::too_many_arguments)]
fn render_variant(
    context: &HeadlessContext,
    loaded: &LoadedManifest,
    pass: &crate::manifest::Pass,
    frame: i32,
    fps: f32,
    width: u32,
    height: u32,
    uniform_values: &BTreeMap<String, crate::uniforms::UniformValue>,
    raw_requested: bool,
    profile_requested: bool,
    storage_names: &[String],
) -> Result<RenderedVariant> {
    render::validate_dimensions(width, height)?;
    let (pass_width, pass_height) = loaded.manifest.pass_dimensions(pass, width, height);
    let media = crate::media::MediaInputs::new_headless(loaded)?;
    let mut runtime = Runtime::new(context)?;
    let project = build_native_project(loaded)?;
    runtime.load_project(&project)?;
    crate::uniforms::apply_to_runtime(&mut runtime, uniform_values)?;
    if profile_requested {
        runtime.set_profiling_mode(true, true)?;
    }
    let final_image = render_from_zero(&mut runtime, frame, fps, width, height, &[], &media)?
        .context("test render did not produce a final image")?;
    let timings = if profile_requested {
        let timings = runtime.pass_timings()?;
        runtime.set_profiling(false)?;
        Some(timings)
    } else {
        None
    };
    let image = if pass.name == loaded.manifest.final_pass().name {
        final_image
    } else {
        runtime.snapshot_pass_rgb(&pass.name, pass_width, pass_height)?
    };
    let raw = if raw_requested {
        Some(runtime.snapshot_pass_rgba32f(&pass.name, pass_width, pass_height)?)
    } else {
        None
    };

    let mut storage = BTreeMap::new();
    for name in storage_names {
        let size = declared_storage_size(loaded, name)?;
        storage.insert(
            name.clone(),
            runtime.snapshot_storage_buffer(
                name,
                usize::try_from(size).context("storage size exceeds usize")?,
            )?,
        );
    }

    let mut pass_gpu_ms = BTreeMap::new();
    let gpu_total_ms = timings.map(|timings| {
        let mut total = 0u64;
        for timing in timings {
            total = total.saturating_add(timing.gpu_nanoseconds);
            pass_gpu_ms.insert(timing.name, timing.gpu_nanoseconds as f64 / 1_000_000.0);
        }
        total as f64 / 1_000_000.0
    });

    Ok(RenderedVariant {
        frame,
        width,
        height,
        image,
        raw,
        gpu_total_ms,
        pass_gpu_ms,
        storage,
    })
}
