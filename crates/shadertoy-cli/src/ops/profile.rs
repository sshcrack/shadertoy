use super::*;
use std::time::Instant;

const MAX_PROFILE_SAMPLES: u32 = 10_000;
const OUTLIER_ABSOLUTE_FLOOR_NS: f64 = 250_000.0;

#[derive(Clone)]
struct ProfileSample {
    frame: i32,
    gpu_execution_ns: u64,
    attributed_ns: u64,
    completion_wait_ns: u64,
    sample_valid: bool,
}

#[derive(Default)]
struct TimingAggregate {
    width: u32,
    height: u32,
    samples: Vec<ProfileSample>,
}

pub fn profile_project(options: &ProfileOptions) -> Result<Output> {
    if options.samples == 0 || options.samples > MAX_PROFILE_SAMPLES {
        bail!("--samples must be in 1..={MAX_PROFILE_SAMPLES}");
    }
    if options.warmup > MAX_PROFILE_SAMPLES {
        bail!("--warmup must be at most {MAX_PROFILE_SAMPLES}");
    }

    let loaded = LoadedManifest::load_with_preset(&options.project, options.preset.as_deref())?;
    ensure_source_files_exist(&loaded)?;
    let media = crate::media::MediaInputs::new_headless(&loaded)?;
    let base_dimensions = if options.preset.is_some() {
        let base = LoadedManifest::load_with_preset(&options.project, None)?;
        Some((base.manifest.render.width, base.manifest.render.height))
    } else {
        None
    };
    let (width, height) = if let Some((base_width, base_height)) = base_dimensions {
        let width = options.width.unwrap_or(base_width);
        let height = options.height.unwrap_or(base_height);
        render::validate_dimensions(width, height)?;
        (width, height)
    } else {
        render::resolve_dimensions(&loaded, None, options.width, options.height)?
    };
    let fps = render::resolve_fps(&loaded, None, options.fps)?;
    let target_frame = resolve_target_frame(&loaded, None, options.frame, options.time, fps)?;

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for profiling")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_native_project(&loaded)?;
    runtime.load_project(&project)?;
    let uniform_values =
        crate::uniforms::parse_assignments(&loaded.manifest.uniforms, &options.set_uniforms)?;
    crate::uniforms::apply_to_runtime(&mut runtime, &uniform_values)?;

    let _ = render_from_zero(&mut runtime, target_frame, fps, width, height, &[], &media)?;
    for _ in 0..options.warmup {
        runtime.tick_fixed(1.0 / fps, fps)?;
        let media_time = runtime.time();
        media.update(&mut runtime, media_time)?;
        let _ = runtime.render(width, height)?;
    }

    runtime.set_profiling_mode(true, options.sync_per_pass)?;
    let first_sample_frame = runtime.frame().saturating_add(1);
    let mut cpu_samples = Vec::with_capacity(options.samples as usize);
    let mut attributed_total_samples = Vec::with_capacity(options.samples as usize);
    let mut frame_timestamp_samples = Vec::with_capacity(options.samples as usize);
    let mut completion_wait_total_samples = Vec::with_capacity(options.samples as usize);
    let mut frame_execution_total_samples = Vec::with_capacity(options.samples as usize);
    let mut frame_execution_valid = Vec::with_capacity(options.samples as usize);
    let mut frame_numbers = Vec::with_capacity(options.samples as usize);
    let mut passes: BTreeMap<String, TimingAggregate> = BTreeMap::new();

    for _ in 0..options.samples {
        runtime.tick_fixed(1.0 / fps, fps)?;
        let media_time = runtime.time();
        media.update(&mut runtime, media_time)?;
        let started = Instant::now();
        let _ = runtime.render(width, height)?;
        cpu_samples.push(started.elapsed().as_nanos() as u64);

        let frame = runtime.frame();
        let profile_samples = runtime.pass_profile_samples()?;
        let completion_wait_total = profile_samples
            .iter()
            .map(|sample| sample.completion_wait_nanoseconds)
            .sum::<u64>();
        let execution_total = profile_samples
            .iter()
            .map(|sample| sample.gpu_execution_nanoseconds)
            .sum::<u64>();
        let all_execution_valid = profile_samples.iter().all(|sample| sample.sample_valid);

        frame_numbers.push(frame);
        attributed_total_samples.push(runtime.frame_gpu_nanoseconds()?);
        frame_timestamp_samples.push(runtime.frame_gpu_timestamp_nanoseconds()?);
        completion_wait_total_samples.push(completion_wait_total);
        frame_execution_total_samples.push(execution_total);
        frame_execution_valid.push(all_execution_valid);

        for sample in profile_samples {
            let aggregate = passes.entry(sample.name).or_default();
            aggregate.width = sample.width;
            aggregate.height = sample.height;
            aggregate.samples.push(ProfileSample {
                frame,
                gpu_execution_ns: sample.gpu_execution_nanoseconds,
                attributed_ns: sample.attributed_nanoseconds,
                completion_wait_ns: sample.completion_wait_nanoseconds,
                sample_valid: sample.sample_valid,
            });
        }
    }
    runtime.set_profiling(false)?;

    // sync-per-pass deliberately serializes pass boundaries. Collect the frame-level
    // GPU timestamp from a second deterministic runtime without those waits so the
    // candidate-comparison signal is not polluted by the diagnostic isolation itself.
    let frame_timestamp_source = if options.sync_per_pass {
        let frame_media = crate::media::MediaInputs::new_headless(&loaded)?;
        let mut frame_runtime = Runtime::new(&context)?;
        let frame_project = build_native_project(&loaded)?;
        frame_runtime.load_project(&frame_project)?;
        crate::uniforms::apply_to_runtime(&mut frame_runtime, &uniform_values)?;
        let _ = render_from_zero(
            &mut frame_runtime,
            target_frame,
            fps,
            width,
            height,
            &[],
            &frame_media,
        )?;
        for _ in 0..options.warmup {
            frame_runtime.tick_fixed(1.0 / fps, fps)?;
            let media_time = frame_runtime.time();
            frame_media.update(&mut frame_runtime, media_time)?;
            let _ = frame_runtime.render(width, height)?;
        }
        frame_runtime.set_profiling_mode(true, false)?;
        frame_timestamp_samples.clear();
        for _ in 0..options.samples {
            frame_runtime.tick_fixed(1.0 / fps, fps)?;
            let media_time = frame_runtime.time();
            frame_media.update(&mut frame_runtime, media_time)?;
            let _ = frame_runtime.render(width, height)?;
            frame_timestamp_samples.push(frame_runtime.frame_gpu_timestamp_nanoseconds()?);
        }
        frame_runtime.set_profiling(false)?;
        "separate_non_intrusive_runtime"
    } else {
        "same_non_intrusive_runtime"
    };

    let cpu = timing_stats(&cpu_samples);
    let frame_outliers = combined_outlier_mask(&[
        &attributed_total_samples,
        &frame_timestamp_samples,
        &completion_wait_total_samples,
    ]);
    let selected_frame_attributed = select_samples(
        &attributed_total_samples,
        &frame_outliers,
        options.discard_outliers,
    );
    let selected_frame_timestamp = select_samples(
        &frame_timestamp_samples,
        &frame_outliers,
        options.discard_outliers,
    );
    let selected_wait_total = select_samples(
        &completion_wait_total_samples,
        &frame_outliers,
        options.discard_outliers,
    );
    let valid_execution_total = frame_execution_total_samples
        .iter()
        .zip(&frame_execution_valid)
        .zip(&frame_outliers)
        .filter_map(|((value, valid), outlier)| {
            (*valid && (!options.discard_outliers || !*outlier)).then_some(*value)
        })
        .collect::<Vec<_>>();

    let gpu_total = timing_stats(&selected_frame_attributed);
    let frame_timestamp = timing_stats(&selected_frame_timestamp);
    let completion_wait_total = timing_stats(&selected_wait_total);
    let frame_execution = timing_stats(&valid_execution_total);
    let persistent_buffer_bytes = estimated_persistent_buffer_bytes(&loaded, width, height)?;

    let mut human = format!(
        "Profile: {}x{} @ {} fps, {} samples (frames {}..={})\n",
        width,
        height,
        fps,
        options.samples,
        first_sample_frame,
        runtime.frame()
    );
    let timing_mode = if options.sync_per_pass {
        "sync-per-pass diagnostics (GPU execution interval + host completion wait separated)"
    } else {
        "non-intrusive frame timing (async compute pass timers may be marked invalid)"
    };
    human.push_str(&format!("Timing mode: {timing_mode}\n"));
    human.push_str(&format!(
        "Outliers: {} (MAD-based flags are always reported; {} aggregate statistics)\n",
        if options.discard_outliers {
            "discard"
        } else {
            "flag only"
        },
        if options.discard_outliers {
            "excluded from"
        } else {
            "retained in"
        }
    ));
    if let Some((base_width, base_height)) = base_dimensions {
        let requested = (loaded.manifest.render.width, loaded.manifest.render.height);
        if requested != (width, height) {
            human.push_str(&format!(
                "Output resolution policy: fixed benchmark output {}x{}; preset-requested final scaling to {}x{} is excluded from performance comparison\n",
                width, height, requested.0, requested.1
            ));
        } else {
            human.push_str(&format!(
                "Output resolution policy: fixed benchmark output {}x{} (project base {}x{})\n",
                width, height, base_width, base_height
            ));
        }
    }

    human.push_str(
        "Pass                         Resolution    exec med   wait med   attrib med  invalid outlier\n",
    );
    let mut json_passes = Vec::with_capacity(passes.len());
    for (name, aggregate) in &passes {
        let execution = aggregate
            .samples
            .iter()
            .map(|sample| sample.gpu_execution_ns)
            .collect::<Vec<_>>();
        let attributed = aggregate
            .samples
            .iter()
            .map(|sample| sample.attributed_ns)
            .collect::<Vec<_>>();
        let waits = aggregate
            .samples
            .iter()
            .map(|sample| sample.completion_wait_ns)
            .collect::<Vec<_>>();
        let outliers = combined_outlier_mask(&[&execution, &attributed, &waits]);

        let selected_attributed = select_samples(&attributed, &outliers, options.discard_outliers);
        let selected_waits = select_samples(&waits, &outliers, options.discard_outliers);
        let selected_execution = aggregate
            .samples
            .iter()
            .zip(&outliers)
            .filter_map(|(sample, outlier)| {
                (sample.sample_valid && (!options.discard_outliers || !*outlier))
                    .then_some(sample.gpu_execution_ns)
            })
            .collect::<Vec<_>>();

        let attributed_stats = timing_stats(&selected_attributed);
        let raw_attributed_stats = timing_stats(&attributed);
        let execution_stats = timing_stats(&selected_execution);
        let raw_execution_stats = timing_stats(&execution);
        let wait_stats = timing_stats(&selected_waits);
        let raw_wait_stats = timing_stats(&waits);
        let invalid_count = aggregate
            .samples
            .iter()
            .filter(|sample| !sample.sample_valid)
            .count();
        let outlier_count = outliers.iter().filter(|value| **value).count();

        let exec_text = if execution_stats.count == 0 {
            "     n/a".to_string()
        } else {
            format!("{:>8.3}", execution_stats.median_ns / 1_000_000.0)
        };
        human.push_str(&format!(
            "{name:<28} {:>5}x{:<5} {}ms {:>8.3}ms {:>9.3}ms {:>7} {:>7}\n",
            aggregate.width,
            aggregate.height,
            exec_text,
            wait_stats.median_ns / 1_000_000.0,
            attributed_stats.median_ns / 1_000_000.0,
            invalid_count,
            outlier_count,
        ));

        let sample_details = aggregate
            .samples
            .iter()
            .zip(&outliers)
            .enumerate()
            .map(|(index, (sample, outlier))| {
                json!({
                    "sample": index,
                    "frame": sample.frame,
                    "gpu_execution_ms": sample.gpu_execution_ns as f64 / 1_000_000.0,
                    "completion_wait_ms": sample.completion_wait_ns as f64 / 1_000_000.0,
                    "attributed_ms": sample.attributed_ns as f64 / 1_000_000.0,
                    "sample_valid": sample.sample_valid,
                    "attributed_includes_completion_wait": sample.completion_wait_ns > 0,
                    "outlier": outlier,
                    "included_in_execution_stats": sample.sample_valid && (!options.discard_outliers || !*outlier),
                    "included_in_attributed_stats": !options.discard_outliers || !*outlier,
                })
            })
            .collect::<Vec<_>>();

        let mesh_stats = loaded
            .manifest
            .passes
            .iter()
            .find(|pass| pass.name == *name)
            .and_then(|pass| pass.mesh.as_ref())
            .map(|mesh| {
                json!({
                    "grid_size": mesh.size,
                    "instances": mesh.instances,
                    "vertices": mesh.vertex_count(),
                    "triangles": mesh.triangle_count(),
                })
            });

        json_passes.push(json!({
            "name": name,
            "width": aggregate.width,
            "height": aggregate.height,
            "mesh": mesh_stats,
            "samples": attributed_stats.count,
            "raw_samples": aggregate.samples.len(),
            // Compatibility fields remain the legacy completion-attributed interval.
            "mean_ms": attributed_stats.mean_ns / 1_000_000.0,
            "median_ms": attributed_stats.median_ns / 1_000_000.0,
            "p95_ms": attributed_stats.p95_ns / 1_000_000.0,
            "min_ms": attributed_stats.min_ns as f64 / 1_000_000.0,
            "max_ms": attributed_stats.max_ns as f64 / 1_000_000.0,
            "gpu_execution": stats_json(&execution_stats),
            "raw_gpu_execution": stats_json(&raw_execution_stats),
            "completion_wait": stats_json(&wait_stats),
            "raw_completion_wait": stats_json(&raw_wait_stats),
            "attributed": stats_json(&attributed_stats),
            "raw_attributed": stats_json(&raw_attributed_stats),
            "invalid_samples": invalid_count,
            "outlier_samples": outlier_count,
            "sample_details": sample_details,
        }));
    }

    let frame_invalid_count = frame_execution_valid
        .iter()
        .filter(|value| !**value)
        .count();
    let frame_outlier_count = frame_outliers.iter().filter(|value| **value).count();
    human.push_str(&format!(
        "Frame GPU timestamp interval: median {:.3} ms, p95 {:.3} ms\n",
        frame_timestamp.median_ns / 1_000_000.0,
        frame_timestamp.p95_ns / 1_000_000.0,
    ));
    if frame_execution.count > 0 {
        human.push_str(&format!(
            "Valid summed pass execution: median {:.3} ms, p95 {:.3} ms ({} valid frames)\n",
            frame_execution.median_ns / 1_000_000.0,
            frame_execution.p95_ns / 1_000_000.0,
            frame_execution.count,
        ));
    } else {
        human.push_str(
            "Valid summed pass execution: n/a (one or more async pass timers outran GPU work in every sampled frame)\n",
        );
    }
    human.push_str(&format!(
        "Completion wait total: median {:.3} ms, p95 {:.3} ms\nLegacy attributed pass sum: median {:.3} ms, p95 {:.3} ms\nCPU profiled render call: median {:.3} ms, p95 {:.3} ms\nPersistent GPU state VRAM estimate: {:.2} MiB",
        completion_wait_total.median_ns / 1_000_000.0,
        completion_wait_total.p95_ns / 1_000_000.0,
        gpu_total.median_ns / 1_000_000.0,
        gpu_total.p95_ns / 1_000_000.0,
        cpu.median_ns / 1_000_000.0,
        cpu.p95_ns / 1_000_000.0,
        persistent_buffer_bytes as f64 / (1024.0 * 1024.0),
    ));

    let frame_details = frame_numbers
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            json!({
                "sample": index,
                "frame": frame,
                "gpu_frame_timestamp_ms": frame_timestamp_samples[index] as f64 / 1_000_000.0,
                "gpu_execution_sum_ms": frame_execution_total_samples[index] as f64 / 1_000_000.0,
                "completion_wait_total_ms": completion_wait_total_samples[index] as f64 / 1_000_000.0,
                "attributed_pass_sum_ms": attributed_total_samples[index] as f64 / 1_000_000.0,
                "sample_valid": frame_execution_valid[index],
                "outlier": frame_outliers[index],
                "included_in_stats": !options.discard_outliers || !frame_outliers[index],
            })
        })
        .collect::<Vec<_>>();

    Ok(Output {
        human,
        json: json!({
            "ok": true,
            "project": loaded.manifest.project.name,
            "preset": options.preset,
            "width": width,
            "height": height,
            "output_resolution_policy": if options.preset.is_some() { "fixed_project_output" } else { "manifest_or_explicit" },
            "preset_requested_width": loaded.manifest.render.width,
            "preset_requested_height": loaded.manifest.render.height,
            "fps": fps,
            "warmup": options.warmup,
            "samples": options.samples,
            "first_sample_frame": first_sample_frame,
            "last_sample_frame": runtime.frame(),
            "passes": json_passes,
            "timing_mode": if options.sync_per_pass { "sync_per_pass" } else { "completion_synchronized" },
            "sync_per_pass": options.sync_per_pass,
            "discard_outliers": options.discard_outliers,
            "outlier_method": "median absolute deviation; > max(6*MAD, 0.5*median, 0.25ms); raw samples are always retained",
            "recommended_gpu_metric": "gpu_frame_timestamp",
            "recommended_gpu_metric_note": "Use the non-intrusive frame timestamp series for cross-run optimization comparisons; use per-pass validity/wait diagnostics to localize changes.",
            "gpu_frame_mode": "attributed_pass_sum",
            "gpu_frame_note": "Compatibility total from post-completion pass timestamps. It can include host scheduling gaps; use gpu_frame_timestamp, completion_wait_total, pass sample_valid, and sample_details to diagnose contamination.",
            "gpu_frame": stats_json(&gpu_total),
            "gpu_frame_timestamp": stats_json(&frame_timestamp),
            "gpu_frame_timestamp_source": frame_timestamp_source,
            "gpu_frame_timestamp_note": "Independent non-intrusive start/end GPU timestamp interval. With --sync-per-pass it is collected from a second deterministic runtime so diagnostic glFinish boundaries are excluded. Drivers may still undercount truly independent asynchronous compute; compare with pass validity and completion waits.",
            "gpu_execution_sum": stats_json(&frame_execution),
            "gpu_execution_valid_frames": frame_execution.count,
            "gpu_execution_invalid_frames": frame_invalid_count,
            "completion_wait_total": stats_json(&completion_wait_total),
            "frame_outlier_samples": frame_outlier_count,
            "frame_sample_details": frame_details,
            "gpu_pass_total_mean_ms": gpu_total.mean_ns / 1_000_000.0,
            "gpu_pass_total": stats_json(&gpu_total),
            "cpu_render": stats_json(&cpu),
            "estimated_persistent_gpu_state_vram_bytes": persistent_buffer_bytes,
        }),
    })
}

struct TimingStats {
    count: usize,
    mean_ns: f64,
    median_ns: f64,
    p95_ns: f64,
    min_ns: u64,
    max_ns: u64,
}

fn timing_stats(samples: &[u64]) -> TimingStats {
    let min_ns = samples.iter().copied().min().unwrap_or(0);
    let max_ns = samples.iter().copied().max().unwrap_or(0);
    let mean_ns = if samples.is_empty() {
        0.0
    } else {
        samples.iter().map(|value| *value as f64).sum::<f64>() / samples.len() as f64
    };
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let median_ns = median_sorted(&sorted);
    let p95_ns = if sorted.is_empty() {
        0.0
    } else {
        let rank = ((sorted.len() as f64 * 0.95).ceil() as usize).clamp(1, sorted.len());
        sorted[rank - 1] as f64
    };
    TimingStats {
        count: samples.len(),
        mean_ns,
        median_ns,
        p95_ns,
        min_ns,
        max_ns,
    }
}

fn median_sorted(sorted: &[u64]) -> f64 {
    match sorted.len() {
        0 => 0.0,
        len if len % 2 == 1 => sorted[len / 2] as f64,
        len => (sorted[len / 2 - 1] as f64 + sorted[len / 2] as f64) / 2.0,
    }
}

fn outlier_mask(samples: &[u64]) -> Vec<bool> {
    if samples.len() < 5 {
        return vec![false; samples.len()];
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let median = median_sorted(&sorted);
    let mut deviations = samples
        .iter()
        .map(|value| ((*value as f64) - median).abs() as u64)
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    let mad = median_sorted(&deviations);
    let threshold = (6.0 * mad).max(0.5 * median).max(OUTLIER_ABSOLUTE_FLOOR_NS);
    samples
        .iter()
        .map(|value| ((*value as f64) - median).abs() > threshold)
        .collect()
}

fn combined_outlier_mask(series: &[&[u64]]) -> Vec<bool> {
    let len = series.first().map_or(0, |samples| samples.len());
    let mut combined = vec![false; len];
    for samples in series {
        debug_assert_eq!(samples.len(), len);
        for (index, outlier) in outlier_mask(samples).into_iter().enumerate() {
            combined[index] |= outlier;
        }
    }
    combined
}

fn select_samples(samples: &[u64], outliers: &[bool], discard_outliers: bool) -> Vec<u64> {
    samples
        .iter()
        .zip(outliers)
        .filter_map(|(sample, outlier)| (!discard_outliers || !*outlier).then_some(*sample))
        .collect()
}

fn stats_json(stats: &TimingStats) -> serde_json::Value {
    if stats.count == 0 {
        return json!({
            "samples": 0,
            "mean_ms": null,
            "median_ms": null,
            "p95_ms": null,
            "min_ms": null,
            "max_ms": null,
        });
    }
    json!({
        "samples": stats.count,
        "mean_ms": stats.mean_ns / 1_000_000.0,
        "median_ms": stats.median_ns / 1_000_000.0,
        "p95_ms": stats.p95_ns / 1_000_000.0,
        "min_ms": stats.min_ns as f64 / 1_000_000.0,
        "max_ms": stats.max_ns as f64 / 1_000_000.0,
    })
}

fn estimated_persistent_buffer_bytes(
    loaded: &LoadedManifest,
    output_width: u32,
    output_height: u32,
) -> Result<u64> {
    let feedback_sources = loaded
        .manifest
        .passes
        .iter()
        .flat_map(|pass| &pass.inputs)
        .filter(|input| input.frame == crate::manifest::FrameRef::Previous)
        .map(|input| input.source.as_str())
        .collect::<std::collections::HashSet<_>>();

    let mut bytes = 0u64;
    for pass in &loaded.manifest.passes {
        if !matches!(
            pass.kind,
            PassKind::Buffer | PassKind::Compute | PassKind::Mesh
        ) {
            continue;
        }
        let (width, height) = loaded
            .manifest
            .pass_dimensions(pass, output_width, output_height);
        let bytes_per_pixel = |format| match format {
            crate::manifest::RenderFormat::R32f => 4u64,
            crate::manifest::RenderFormat::Rg32f | crate::manifest::RenderFormat::Rgba16f => 8,
            crate::manifest::RenderFormat::Rgba32f => 16,
        };
        let target_bytes = std::iter::once(pass.format)
            .chain(pass.extra_outputs.iter().copied())
            .try_fold(0u64, |total, format| {
                total
                    .checked_add(bytes_per_pixel(format))
                    .context("persistent MRT VRAM estimate overflow")
            })?
            .checked_add(
                if pass.kind == PassKind::Mesh && pass.mesh.as_ref().is_some_and(|mesh| mesh.depth)
                {
                    4 // sampleable R32F depth copy
                } else {
                    0
                },
            )
            .context("mesh depth-copy VRAM estimate overflow")?;
        let copies = if feedback_sources.contains(pass.name.as_str()) {
            2u64
        } else {
            1
        };
        let pass_bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(target_bytes))
            .and_then(|value| value.checked_mul(copies))
            .context("persistent buffer VRAM estimate overflow")?;
        bytes = bytes
            .checked_add(pass_bytes)
            .context("persistent buffer VRAM estimate overflow")?;
        if let Some(mesh) = &pass.mesh {
            if mesh.depth {
                let depth_attachment_bytes = u64::from(width)
                    .checked_mul(u64::from(height))
                    .and_then(|value| value.checked_mul(4))
                    .context("mesh depth attachment VRAM estimate overflow")?;
                bytes = bytes
                    .checked_add(depth_attachment_bytes)
                    .context("persistent buffer VRAM estimate overflow")?;
            }
            let vertex_bytes = u64::from(mesh.size[0])
                .checked_mul(u64::from(mesh.size[1]))
                .and_then(|value| value.checked_mul(8))
                .context("mesh vertex-buffer VRAM estimate overflow")?;
            let index_bytes = u64::from(mesh.size[0] - 1)
                .checked_mul(u64::from(mesh.size[1] - 1))
                .and_then(|value| value.checked_mul(6))
                .and_then(|value| value.checked_mul(4))
                .context("mesh index-buffer VRAM estimate overflow")?;
            bytes = bytes
                .checked_add(vertex_bytes)
                .and_then(|value| value.checked_add(index_bytes))
                .context("mesh geometry VRAM estimate overflow")?;
        }
    }

    let mut storage = std::collections::HashMap::new();
    for pass in &loaded.manifest.passes {
        for buffer in &pass.storage {
            storage.entry(buffer.name.as_str()).or_insert(buffer.size);
        }
    }
    for size in storage.into_values() {
        bytes = bytes
            .checked_add(size)
            .context("persistent storage VRAM estimate overflow")?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_stats_handles_empty_and_values() {
        let empty = timing_stats(&[]);
        assert_eq!(empty.count, 0);
        assert_eq!(empty.mean_ns, 0.0);
        assert_eq!(empty.median_ns, 0.0);
        assert_eq!(empty.p95_ns, 0.0);

        let values = timing_stats(&[10, 20, 30, 40]);
        assert_eq!(values.min_ns, 10);
        assert_eq!(values.max_ns, 40);
        assert_eq!(values.mean_ns, 25.0);
        assert_eq!(values.median_ns, 25.0);
        assert_eq!(values.p95_ns, 40.0);
    }

    #[test]
    fn outlier_detection_flags_large_stalls_without_hiding_normal_jitter() {
        let values = [
            7_000_000, 7_100_000, 6_900_000, 7_050_000, 7_200_000, 6_950_000, 39_500_000,
        ];
        let mask = outlier_mask(&values);
        assert_eq!(mask.iter().filter(|value| **value).count(), 1);
        assert!(mask[6]);

        let normal = [6_000_000, 6_500_000, 7_000_000, 7_500_000, 8_000_000];
        assert!(outlier_mask(&normal).iter().all(|value| !*value));
    }
}
