use super::*;

pub fn inspect_project(path: &Path, mode: InspectMode) -> Result<Output> {
    let loaded = LoadedManifest::load(path)?;
    let manifest = &loaded.manifest;

    match mode {
        InspectMode::Summary => {
            let human = format!(
                "Project: {}\nRoot: {}\nPasses: {}\nAssets: {}\nFinal output: {}\nRender: {}x{} @ {} fps",
                manifest.project.name,
                loaded.root.display(),
                manifest.passes.len(),
                manifest.assets.len(),
                manifest.final_pass().name,
                manifest.render.width,
                manifest.render.height,
                manifest.render.fps,
            );
            let passes = manifest
                .passes
                .iter()
                .map(|pass| {
                    json!({
                        "name": pass.name,
                        "kind": format!("{:?}", pass.kind).to_lowercase(),
                        "source": pass.source,
                        "inputs": pass.inputs.len(),
                    })
                })
                .collect::<Vec<_>>();
            Ok(Output {
                human,
                json: json!({
                    "ok": true,
                    "project": manifest.project.name,
                    "root": loaded.root,
                    "final_output": manifest.final_pass().name,
                    "render": manifest.render,
                    "passes": passes,
                    "assets": manifest.assets,
                }),
            })
        }
        InspectMode::Graph => {
            let mut human = format!("Project graph: {}\n", manifest.project.name);
            let mut edges = Vec::new();
            for pass in &manifest.passes {
                human.push_str(&format!("  {} ({:?})\n", pass.name, pass.kind));
                for input in &pass.inputs {
                    let kind = manifest.infer_input_kind(input)?;
                    human.push_str(&format!(
                        "    iChannel{} <- {} ({:?}, {:?}, {:?}, {:?})\n",
                        input.channel, input.source, kind, input.frame, input.filter, input.wrap
                    ));
                    edges.push(json!({
                        "from": input.source,
                        "to": pass.name,
                        "channel": input.channel,
                        "kind": format!("{kind:?}").to_lowercase(),
                        "frame": format!("{:?}", input.frame).to_lowercase(),
                        "filter": format!("{:?}", input.filter).to_lowercase(),
                        "wrap": format!("{:?}", input.wrap).to_lowercase(),
                    }));
                }
            }
            Ok(Output {
                human,
                json: json!({
                    "ok": true,
                    "project": manifest.project.name,
                    "passes": manifest.passes,
                    "edges": edges,
                }),
            })
        }
        InspectMode::Pass(name) => {
            let pass = manifest
                .passes
                .iter()
                .find(|pass| pass.name == name)
                .with_context(|| format!("unknown pass '{name}'"))?;
            let human = format!(
                "Pass: {}\nKind: {:?}\nSource: {}\nInputs: {}",
                pass.name,
                pass.kind,
                pass.source,
                pass.inputs.len()
            );
            Ok(Output {
                human,
                json: json!({
                    "ok": true,
                    "project": manifest.project.name,
                    "pass": pass,
                    "is_final": pass.name == manifest.final_pass().name,
                }),
            })
        }
        InspectMode::Channels(name) => {
            let pass = manifest
                .passes
                .iter()
                .find(|pass| pass.name == name)
                .with_context(|| format!("unknown pass '{name}'"))?;
            let mut human = format!("Channels for {}:\n", pass.name);
            let mut channels = Vec::new();
            if pass.inputs.is_empty() {
                human.push_str("  (none)");
            }
            for input in &pass.inputs {
                let kind = manifest.infer_input_kind(input)?;
                human.push_str(&format!(
                    "  iChannel{} <- {} ({:?}, {:?}, {:?}, {:?})\n",
                    input.channel, input.source, kind, input.frame, input.filter, input.wrap
                ));
                channels.push(json!({
                    "channel": input.channel,
                    "source": input.source,
                    "kind": format!("{kind:?}").to_lowercase(),
                    "frame": format!("{:?}", input.frame).to_lowercase(),
                    "filter": format!("{:?}", input.filter).to_lowercase(),
                    "wrap": format!("{:?}", input.wrap).to_lowercase(),
                }));
            }
            Ok(Output {
                human,
                json: json!({
                    "ok": true,
                    "project": manifest.project.name,
                    "pass": pass.name,
                    "channels": channels,
                }),
            })
        }
    }
}

pub fn inspect_buffer(options: &InspectBufferOptions) -> Result<Output> {
    let loaded = LoadedManifest::load(&options.project)?;
    ensure_source_files_exist(&loaded)?;
    let media = crate::media::MediaInputs::new_headless(&loaded)?;
    let pass = loaded
        .manifest
        .passes
        .iter()
        .find(|pass| pass.name == options.pass)
        .with_context(|| format!("unknown pass '{}'", options.pass))?;
    if !matches!(
        pass.kind,
        PassKind::Buffer | PassKind::Compute | PassKind::Mesh
    ) {
        bail!("runtime buffer inspection requires a 2D buffer/compute/mesh pass");
    }
    if usize::from(options.output_index) > pass.extra_outputs.len() {
        bail!(
            "pass '{}' exposes outputs 0..{}; requested output {}",
            pass.name,
            pass.extra_outputs.len(),
            options.output_index
        );
    }

    let (width, height) =
        super::render::resolve_dimensions(&loaded, None, options.width, options.height)?;
    let fps = super::render::resolve_fps(&loaded, None, options.fps)?;
    let frame = resolve_target_frame(&loaded, None, options.frame, options.time, fps)?;
    let (pass_width, pass_height) = loaded.manifest.pass_dimensions(pass, width, height);

    if let Some((x, y)) = options.pixel
        && (x >= pass_width || y >= pass_height)
    {
        bail!(
            "--pixel {x},{y} is outside pass '{}' dimensions {}x{}",
            pass.name,
            pass_width,
            pass_height
        );
    }

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for buffer inspection")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_native_project(&loaded)?;
    runtime.load_project(&project)?;
    let uniform_values =
        crate::uniforms::parse_assignments(&loaded.manifest.uniforms, &options.set_uniforms)?;
    crate::uniforms::apply_to_runtime(&mut runtime, &uniform_values)?;
    let _ = render_from_zero(&mut runtime, frame, fps, width, height, &[], &media)?;
    let values = runtime.snapshot_pass_output_rgba32f(
        &pass.name,
        options.output_index.into(),
        pass_width,
        pass_height,
    )?;

    let stats = BufferStats::from_rgba(&values);
    let pixel = options.pixel.map(|(x, y)| {
        let offset = ((y as usize * pass_width as usize) + x as usize) * 4;
        [
            values[offset],
            values[offset + 1],
            values[offset + 2],
            values[offset + 3],
        ]
    });

    if let Some(path) = &options.raw {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::with_capacity(values.len() * 4);
        for value in &values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fs::write(path, bytes)
            .with_context(|| format!("failed to write raw buffer {}", path.display()))?;
    }

    if let Some(path) = &options.output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let image = visualize_buffer(
            pass_width,
            pass_height,
            &values,
            options.visualization,
            &stats,
        );
        save_rgb_png(&image, path)?;
    }

    let mut human = format!(
        "Buffer: {} output {}\nReadback: RGBA32F\nResolution: {}x{}\nFrame: {} ({:.3}s)\nNaN: {}  Inf: {}\n",
        pass.name,
        options.output_index,
        pass_width,
        pass_height,
        runtime.frame(),
        runtime.time(),
        stats.nan_count,
        stats.inf_count,
    );
    for channel in 0..4 {
        human.push_str(&format!(
            "{}: min={:.6} max={:.6} mean={:.6}\n",
            ["R", "G", "B", "A"][channel],
            stats.min[channel],
            stats.max[channel],
            stats.mean[channel],
        ));
    }
    if let (Some((x, y)), Some(value)) = (options.pixel, pixel) {
        human.push_str(&format!(
            "Pixel {x},{y}: [{:.6}, {:.6}, {:.6}, {:.6}]\n",
            value[0], value[1], value[2], value[3]
        ));
    }
    if let Some(path) = &options.output {
        human.push_str(&format!("Visualization: {}\n", path.display()));
    }

    Ok(Output {
        human: human.trim_end().to_string(),
        json: json!({
            "ok": true,
            "project": loaded.manifest.project.name,
            "pass": pass.name,
            "output_index": options.output_index,
            "format": if options.output_index == 0 {
                format!("{:?}", pass.format).to_lowercase()
            } else {
                format!("{:?}", pass.extra_outputs[usize::from(options.output_index) - 1]).to_lowercase()
            },
            "width": pass_width,
            "height": pass_height,
            "frame": runtime.frame(),
            "time": runtime.time(),
            "stats": {
                "min": stats.min,
                "max": stats.max,
                "mean": stats.mean,
                "nan": stats.nan_count,
                "inf": stats.inf_count,
            },
            "pixel": options.pixel.map(|coordinates| json!({
                "x": coordinates.0,
                "y": coordinates.1,
                "rgba": pixel.expect("pixel value exists with pixel coordinates"),
            })),
            "output": options.output,
            "raw": options.raw,
        }),
    })
}

pub fn inspect_storage(options: &InspectStorageOptions) -> Result<Output> {
    let loaded = LoadedManifest::load(&options.project)?;
    ensure_source_files_exist(&loaded)?;
    let media = crate::media::MediaInputs::new_headless(&loaded)?;

    let declared_size = loaded
        .manifest
        .passes
        .iter()
        .flat_map(|pass| &pass.storage)
        .find(|storage| storage.name == options.name)
        .map(|storage| storage.size)
        .with_context(|| format!("unknown storage buffer '{}'", options.name))?;
    let declared_size =
        usize::try_from(declared_size).context("storage buffer size does not fit this platform")?;

    let (width, height) =
        super::render::resolve_dimensions(&loaded, None, options.width, options.height)?;
    let fps = super::render::resolve_fps(&loaded, None, options.fps)?;
    let frame = resolve_target_frame(&loaded, None, options.frame, options.time, fps)?;

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for storage inspection")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_native_project(&loaded)?;
    runtime.load_project(&project)?;
    let uniform_values =
        crate::uniforms::parse_assignments(&loaded.manifest.uniforms, &options.set_uniforms)?;
    crate::uniforms::apply_to_runtime(&mut runtime, &uniform_values)?;
    let _ = render_from_zero(&mut runtime, frame, fps, width, height, &[], &media)?;
    let data = runtime.snapshot_storage_buffer(&options.name, declared_size)?;

    if let Some(path) = &options.output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &data)
            .with_context(|| format!("failed to write storage dump {}", path.display()))?;
    }

    let element_size = match options.value_type {
        InspectStorageType::Bytes => 1,
        InspectStorageType::U32 | InspectStorageType::I32 | InspectStorageType::F32 => 4,
    };
    if !options.offset.is_multiple_of(element_size) {
        bail!(
            "--offset {} is not aligned for {:?} values ({} bytes)",
            options.offset,
            options.value_type,
            element_size
        );
    }
    if options.offset > data.len() {
        bail!(
            "--offset {} exceeds storage buffer size {}",
            options.offset,
            data.len()
        );
    }
    let available = (data.len() - options.offset) / element_size;
    let count = options.count.min(available);
    let bytes = &data[options.offset..options.offset + count * element_size];

    let mut human_values = Vec::with_capacity(count);
    let mut json_values = Vec::with_capacity(count);
    for chunk in bytes.chunks_exact(element_size) {
        match options.value_type {
            InspectStorageType::Bytes => {
                human_values.push(format!("{}", chunk[0]));
                json_values.push(json!(chunk[0]));
            }
            InspectStorageType::U32 => {
                let value = u32::from_le_bytes(chunk.try_into().expect("four-byte chunk"));
                human_values.push(value.to_string());
                json_values.push(json!(value));
            }
            InspectStorageType::I32 => {
                let value = i32::from_le_bytes(chunk.try_into().expect("four-byte chunk"));
                human_values.push(value.to_string());
                json_values.push(json!(value));
            }
            InspectStorageType::F32 => {
                let value = f32::from_le_bytes(chunk.try_into().expect("four-byte chunk"));
                human_values.push(format!("{value:.9}"));
                json_values.push(if value.is_finite() {
                    json!(value)
                } else if value.is_nan() {
                    json!("NaN")
                } else if value.is_sign_positive() {
                    json!("+Inf")
                } else {
                    json!("-Inf")
                });
            }
        }
    }

    let kind = match options.value_type {
        InspectStorageType::Bytes => "bytes",
        InspectStorageType::U32 => "u32",
        InspectStorageType::I32 => "i32",
        InspectStorageType::F32 => "f32",
    };
    let mut human = format!(
        "Storage: {}\nSize: {} bytes\nFrame: {} ({:.3}s)\n{} @ byte {}: [{}]",
        options.name,
        data.len(),
        runtime.frame(),
        runtime.time(),
        kind,
        options.offset,
        human_values.join(", ")
    );
    if count < options.count {
        human.push_str(&format!(
            "\nRequested {} values; {} were available",
            options.count, count
        ));
    }
    if let Some(path) = &options.output {
        human.push_str(&format!("\nRaw dump: {}", path.display()));
    }

    Ok(Output {
        human,
        json: json!({
            "ok": true,
            "project": loaded.manifest.project.name,
            "storage": options.name,
            "size": data.len(),
            "frame": runtime.frame(),
            "time": runtime.time(),
            "offset": options.offset,
            "type": kind,
            "count": count,
            "values": json_values,
            "output": options.output,
        }),
    })
}

struct BufferStats {
    min: [f32; 4],
    max: [f32; 4],
    mean: [f64; 4],
    finite_count: [u64; 4],
    nan_count: u64,
    inf_count: u64,
}

impl BufferStats {
    fn from_rgba(values: &[f32]) -> Self {
        let mut result = Self {
            min: [f32::INFINITY; 4],
            max: [f32::NEG_INFINITY; 4],
            mean: [0.0; 4],
            finite_count: [0; 4],
            nan_count: 0,
            inf_count: 0,
        };
        for pixel in values.as_chunks::<4>().0 {
            for (channel, value) in pixel.iter().copied().enumerate() {
                if value.is_nan() {
                    result.nan_count += 1;
                } else if value.is_infinite() {
                    result.inf_count += 1;
                } else {
                    result.min[channel] = result.min[channel].min(value);
                    result.max[channel] = result.max[channel].max(value);
                    result.mean[channel] += f64::from(value);
                    result.finite_count[channel] += 1;
                }
            }
        }
        for channel in 0..4 {
            if result.finite_count[channel] == 0 {
                result.min[channel] = f32::NAN;
                result.max[channel] = f32::NAN;
                result.mean[channel] = f64::NAN;
            } else {
                result.mean[channel] /= result.finite_count[channel] as f64;
            }
        }
        result
    }
}

fn visualize_buffer(
    width: u32,
    height: u32,
    values: &[f32],
    requested: InspectVisualization,
    stats: &BufferStats,
) -> RgbImage {
    let visualization = match requested {
        InspectVisualization::Auto
            if stats.min[..3].iter().any(|value| *value < 0.0)
                || stats.max[..3].iter().any(|value| *value > 1.0) =>
        {
            InspectVisualization::Signed
        }
        InspectVisualization::Auto => InspectVisualization::Rgb,
        other => other,
    };

    let signed_scale = stats.min[..3]
        .iter()
        .chain(&stats.max[..3])
        .filter(|value| value.is_finite())
        .map(|value| value.abs())
        .fold(0.0f32, f32::max)
        .max(f32::EPSILON);
    let magnitude_scale = values
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            let rgb = [
                if pixel[0].is_finite() { pixel[0] } else { 0.0 },
                if pixel[1].is_finite() { pixel[1] } else { 0.0 },
                if pixel[2].is_finite() { pixel[2] } else { 0.0 },
            ];
            (rgb[0] * rgb[0] + rgb[1] * rgb[1] + rgb[2] * rgb[2]).sqrt()
        })
        .fold(0.0f32, f32::max)
        .max(f32::EPSILON);

    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    let to_byte = |value: f32| -> u8 {
        if value.is_nan() {
            return 255;
        }
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    for pixel in values.as_chunks::<4>().0 {
        let rgb = match visualization {
            InspectVisualization::Auto | InspectVisualization::Rgb => {
                [pixel[0], pixel[1], pixel[2]]
            }
            InspectVisualization::Signed => [
                0.5 + 0.5 * pixel[0] / signed_scale,
                0.5 + 0.5 * pixel[1] / signed_scale,
                0.5 + 0.5 * pixel[2] / signed_scale,
            ],
            InspectVisualization::Magnitude => {
                let magnitude = (pixel[0] * pixel[0] + pixel[1] * pixel[1] + pixel[2] * pixel[2])
                    .sqrt()
                    / magnitude_scale;
                [magnitude; 3]
            }
        };
        pixels.extend(rgb.map(to_byte));
    }
    RgbImage::new(width, height, pixels)
}
