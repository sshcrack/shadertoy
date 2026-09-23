use super::*;

pub(super) fn check_numeric_assertions(
    test: &TestCase,
    variant: &RenderedVariant,
    values: &[f32],
    reasons: &mut Vec<String>,
) {
    let label = format!(
        "frame {} at {}x{}",
        variant.frame, variant.width, variant.height
    );
    let nan = values.iter().filter(|value| value.is_nan()).count();
    let inf = values.iter().filter(|value| value.is_infinite()).count();
    if test.assert_no_nan && nan != 0 {
        reasons.push(format!("{label}: found {nan} NaN values"));
    }
    if test.assert_no_inf && inf != 0 {
        reasons.push(format!("{label}: found {inf} infinite values"));
    }
    if let Some([min, max]) = test.mean_range {
        let mut sum = 0.0f64;
        let mut count = 0u64;
        for value in values.iter().copied().filter(|value| value.is_finite()) {
            sum += f64::from(value);
            count += 1;
        }
        if count == 0 {
            reasons.push(format!(
                "{label}: mean_range has no finite values to inspect"
            ));
        } else {
            let mean = sum / count as f64;
            if mean < f64::from(min) || mean > f64::from(max) {
                reasons.push(format!(
                    "{label}: finite RGBA mean {mean:.6} is outside [{min:.6}, {max:.6}]"
                ));
            }
        }
    }
}

pub(super) fn check_performance_assertions(
    test: &TestCase,
    variant: &RenderedVariant,
    reasons: &mut Vec<String>,
) {
    let label = format!(
        "frame {} at {}x{}",
        variant.frame, variant.width, variant.height
    );
    if let Some(max_ms) = test.max_gpu_ms
        && let Some(actual) = variant.gpu_total_ms
        && actual > f64::from(max_ms)
    {
        reasons.push(format!(
            "{label}: GPU pass total {actual:.3} ms exceeds {max_ms:.3} ms"
        ));
    }
    for (name, max_ms) in &test.max_pass_gpu_ms {
        match variant.pass_gpu_ms.get(name) {
            Some(actual) if *actual > f64::from(*max_ms) => reasons.push(format!(
                "{label}: pass '{name}' GPU time {actual:.3} ms exceeds {max_ms:.3} ms"
            )),
            None => reasons.push(format!(
                "{label}: pass '{name}' produced no profiling timing"
            )),
            _ => {}
        }
    }
}

pub(super) fn check_storage_assertions(
    loaded: &LoadedManifest,
    test: &TestCase,
    variant: &RenderedVariant,
    reasons: &mut Vec<String>,
) -> Result<()> {
    for assertion in &test.storage_assertions {
        let path = safe_reference_path(&loaded.root, &assertion.reference, false)?;
        if !path.is_file() {
            reasons.push(format!(
                "storage reference for '{}' is missing: {}",
                assertion.name,
                path.display()
            ));
            continue;
        }
        let expected = fs::read(&path)
            .with_context(|| format!("failed to read storage reference {}", path.display()))?;
        let actual = variant
            .storage
            .get(&assertion.name)
            .with_context(|| format!("test did not capture storage '{}'", assertion.name))?;
        if expected != *actual {
            let first_difference = expected
                .iter()
                .zip(actual)
                .position(|(left, right)| left != right);
            reasons.push(format!(
                "frame {} at {}x{} storage '{}' differs from {} (expected {} bytes, got {} bytes, first differing byte {:?})",
                variant.frame,
                variant.width,
                variant.height,
                assertion.name,
                path.display(),
                expected.len(),
                actual.len(),
                first_difference
            ));
        }
    }
    Ok(())
}

pub(super) fn enforce_rmse_bounds(
    test: &TestCase,
    rmse: f64,
    legacy_reference: bool,
    label: &str,
    reasons: &mut Vec<String>,
) {
    if let Some(min) = test.min_rmse
        && rmse < f64::from(min)
    {
        reasons.push(format!(
            "{label}: RMSE {rmse:.6} is below required minimum {min:.6}"
        ));
    }
    let implicit_legacy_max = legacy_reference.then_some(test.tolerance);
    let implicit_uniform_max =
        (test.min_rmse.is_none() && test.max_rmse.is_none()).then_some(test.tolerance);
    if let Some(max) = test
        .max_rmse
        .or(implicit_legacy_max)
        .or(implicit_uniform_max)
        && rmse > f64::from(max)
    {
        reasons.push(format!("{label}: RMSE {rmse:.6} exceeds maximum {max:.6}"));
    }
}

pub(super) fn declared_storage_size(loaded: &LoadedManifest, name: &str) -> Result<u64> {
    loaded
        .manifest
        .passes
        .iter()
        .flat_map(|pass| &pass.storage)
        .find(|storage| storage.name == name)
        .map(|storage| storage.size)
        .with_context(|| format!("unknown storage buffer '{name}'"))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_state_roundtrip(
    context: &HeadlessContext,
    loaded: &LoadedManifest,
    frame: i32,
    fps: f32,
    width: u32,
    height: u32,
    uniform_values: &BTreeMap<String, crate::uniforms::UniformValue>,
    raw_tolerance: f32,
) -> Result<()> {
    let media = crate::media::MediaInputs::new_headless(loaded)?;
    let project = build_native_project(loaded)?;
    let mut source = Runtime::new(context)?;
    source.load_project(&project)?;
    crate::uniforms::apply_to_runtime(&mut source, uniform_values)?;
    let _ = render_from_zero(&mut source, frame, fps, width, height, &[], &media)?;

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
        if let Ok(values) = source.snapshot_pass_rgba32f(&pass.name, pass_width, pass_height) {
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

    let mut storage_buffers = BTreeMap::new();
    let mut storage_sizes = BTreeMap::new();
    for pass in &loaded.manifest.passes {
        for storage in &pass.storage {
            storage_sizes
                .entry(storage.name.clone())
                .or_insert(storage.size);
        }
    }
    for (name, size) in storage_sizes {
        if let Ok(data) = source.snapshot_storage_buffer(
            &name,
            usize::try_from(size).context("storage size exceeds usize")?,
        ) {
            storage_buffers.insert(name, data);
        }
    }

    let state = StateFile::new(
        loaded.manifest.project.name.clone(),
        width,
        height,
        fps,
        source.time(),
        source.frame(),
        buffers,
        buffer_dimensions,
        buffer_formats,
        storage_buffers,
    )?;
    let temp = tempfile::tempdir().context("failed to create state round-trip temp directory")?;
    let path = temp.path().join("roundtrip.ststate");
    state.save(&path)?;
    let loaded_state = StateFile::load(&path)?;

    let mut restored = Runtime::new(context)?;
    restored.load_project(&project)?;
    crate::uniforms::apply_to_runtime(&mut restored, uniform_values)?;
    render::restore_state(&mut restored, &loaded_state)?;

    for name in &loaded_state.header.buffers {
        let expected = loaded_state
            .buffers
            .get(name)
            .expect("loaded state contains declared buffer");
        let dimensions = loaded_state.buffer_dimensions(name)?;
        let actual = restored.snapshot_pass_rgba32f(name, dimensions.width, dimensions.height)?;
        let comparison = compare_raw(expected, &actual, raw_tolerance)?;
        if comparison.mismatches != 0 {
            bail!(
                "buffer '{}' changed across state serialization/restore: {} values differ (max abs error {:.9})",
                name,
                comparison.mismatches,
                comparison.max_error
            );
        }
    }
    for (name, expected) in &loaded_state.storage_buffers {
        let actual = restored.snapshot_storage_buffer(name, expected.len())?;
        if actual != *expected {
            bail!(
                "storage '{}' changed across state serialization/restore",
                name
            );
        }
    }
    Ok(())
}

pub(super) struct RawComparison {
    pub(super) mismatches: usize,
    pub(super) max_error: f64,
}

pub(super) fn compare_raw(
    expected: &[f32],
    actual: &[f32],
    tolerance: f32,
) -> Result<RawComparison> {
    if expected.len() != actual.len() {
        bail!(
            "raw regression buffers have different lengths ({} vs {})",
            expected.len(),
            actual.len()
        );
    }
    let tolerance = f64::from(tolerance);
    let mut mismatches = 0usize;
    let mut max_error = 0.0f64;
    for (expected, actual) in expected.iter().zip(actual) {
        if expected.to_bits() == actual.to_bits() {
            continue;
        }
        if !expected.is_finite() || !actual.is_finite() {
            mismatches += 1;
            max_error = f64::INFINITY;
            continue;
        }
        let error = (f64::from(*expected) - f64::from(*actual)).abs();
        max_error = max_error.max(error);
        if error > tolerance {
            mismatches += 1;
        }
    }
    Ok(RawComparison {
        mismatches,
        max_error,
    })
}

pub(super) struct Comparison {
    pub(super) rmse: f64,
    pub(super) max_error: f64,
    pub(super) changed_fraction: f64,
    pub(super) diff: Vec<u8>,
}

pub(super) fn compare_rgb(expected: &[u8], actual: &[u8], tolerance: f32) -> Comparison {
    debug_assert_eq!(expected.len(), actual.len());
    let threshold = f64::from(tolerance) * 255.0;
    let mut squared = 0.0f64;
    let mut max = 0.0f64;
    let mut changed_pixels = 0usize;
    let mut diff = Vec::with_capacity(expected.len());

    for (expected_pixel, actual_pixel) in expected
        .as_chunks::<3>()
        .0
        .iter()
        .zip(actual.as_chunks::<3>().0.iter())
    {
        let mut pixel_changed = false;
        for channel in 0..3 {
            let delta =
                (f64::from(expected_pixel[channel]) - f64::from(actual_pixel[channel])).abs();
            squared += delta * delta;
            max = max.max(delta);
            pixel_changed |= delta > threshold;
            diff.push(delta.min(255.0).round() as u8);
        }
        changed_pixels += usize::from(pixel_changed);
    }

    let values = expected.len().max(1) as f64;
    let pixels = (expected.len() / 3).max(1) as f64;
    Comparison {
        rmse: (squared / values).sqrt() / 255.0,
        max_error: max / 255.0,
        changed_fraction: changed_pixels as f64 / pixels,
        diff,
    }
}

pub(super) fn top_down_rgb(image: &RgbImage) -> Vec<u8> {
    let mut pixels = image.pixels.clone();
    super::images::flip_rgb_rows(&mut pixels, image.width, image.height);
    pixels
}

pub(super) fn safe_reference_path(root: &Path, relative: &str, create: bool) -> Result<PathBuf> {
    crate::manifest::validate_project_relative_path(relative, "test reference")?;
    let path = root.join(relative);
    let canonical_root = fs::canonicalize(root)?;
    if path.exists() {
        let canonical = fs::canonicalize(&path)?;
        if !canonical.starts_with(&canonical_root) {
            bail!(
                "test reference resolves outside the project root: {}",
                path.display()
            );
        }
    } else if create && let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let canonical_parent = fs::canonicalize(parent)?;
        if !canonical_parent.starts_with(&canonical_root) {
            bail!(
                "test reference parent resolves outside the project root: {}",
                parent.display()
            );
        }
    }
    Ok(path)
}

pub(super) fn file_safe_name(value: &str) -> String {
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
        "test".into()
    } else {
        trimmed.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_reports_normalized_error() {
        let compared = compare_rgb(&[0, 0, 0], &[255, 0, 0], 0.0);
        assert!((compared.rmse - (1.0f64 / 3.0).sqrt()).abs() < 1e-9);
        assert_eq!(compared.max_error, 1.0);
        assert_eq!(compared.changed_fraction, 1.0);
    }

    #[test]
    fn rmse_bounds_support_difference_only_assertion() {
        let test = TestCase {
            name: "difference".into(),
            pass: None,
            frame: None,
            time: None,
            frames: Vec::new(),
            width: None,
            height: None,
            resolutions: Vec::new(),
            reference: None,
            tolerance: 0.002,
            assert_no_nan: false,
            assert_no_inf: false,
            mean_range: None,
            assert_deterministic: false,
            assert_resolution_independent: false,
            raw_tolerance: 0.0,
            uniforms: BTreeMap::new(),
            reference_uniforms: BTreeMap::new(),
            min_rmse: Some(0.1),
            max_rmse: None,
            max_gpu_ms: None,
            max_pass_gpu_ms: BTreeMap::new(),
            storage_assertions: Vec::<crate::manifest::TestStorageAssertion>::new(),
            assert_state_roundtrip: false,
        };
        let mut reasons = Vec::new();
        enforce_rmse_bounds(&test, 0.2, false, "variant", &mut reasons);
        assert!(reasons.is_empty());
    }
}
