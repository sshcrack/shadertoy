use super::*;
use crate::manifest::{Filter, FrameRef, InputKind, Pass, Wrap};
use crate::project::add_manifest_assets;
use crate::source::expand_pass;
use shadertoy::{
    Filter as NativeFilter, InputKind as NativeInputKind, PassKind as NativePassKind, Project,
    RenderFormat as NativeRenderFormat, Wrap as NativeWrap,
};
use std::io::Write;

const SOUND_COMPUTE_PASS: &str = "__shadertoy_sound_compute";
const SOUND_IMAGE_PASS: &str = "__shadertoy_sound_sink";
const SOUND_OFFSET_UNIFORM: &str = "_stInternalSoundSampleOffset";
const SOUND_CHUNK_SAMPLES: u32 = 4096;
const MAX_SOUND_SECONDS: f32 = 3600.0;

pub fn render_audio_project(options: &RenderAudioOptions) -> Result<Output> {
    let loaded = LoadedManifest::load(&options.project)?;
    ensure_source_files_exist(&loaded)?;
    validate_audio_options(options)?;

    let pass = select_sound_pass(&loaded, options.pass.as_deref())?;
    let sample_count = (f64::from(options.duration) * f64::from(options.sample_rate)).round();
    if sample_count < 1.0 || sample_count > u32::MAX as f64 {
        bail!("audio duration resolves to an unsupported sample count");
    }
    let sample_count = sample_count as u32;

    let output = options
        .output
        .clone()
        .unwrap_or_else(|| loaded.root.join("target/render.wav"));
    if output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| !extension.eq_ignore_ascii_case("wav"))
    {
        bail!("render-audio currently writes deterministic PCM WAV output; use a .wav path");
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for Sound rendering")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_sound_project(&loaded, pass, options.sample_rate)?;
    runtime.load_project(&project)?;

    let uniform_values =
        crate::uniforms::parse_assignments(&loaded.manifest.uniforms, &options.set_uniforms)?;
    crate::uniforms::apply_to_runtime(&mut runtime, &uniform_values)?;

    let mut file = fs::File::create(&output)
        .with_context(|| format!("failed to create audio output {}", output.display()))?;
    write_wav_header(&mut file, options.sample_rate, sample_count)?;

    let mut offset = 0u32;
    while offset < sample_count {
        runtime.set_uniform_i32(
            SOUND_OFFSET_UNIFORM,
            i32::try_from(offset).context("Sound sample offset exceeds i32")?,
        )?;
        let _ = runtime.render(1, 1)?;
        let values = runtime.snapshot_pass_rgba32f(SOUND_COMPUTE_PASS, SOUND_CHUNK_SAMPLES, 1)?;
        let count = (sample_count - offset).min(SOUND_CHUNK_SAMPLES);
        for sample in values.as_chunks::<4>().0.iter().take(count as usize) {
            for value in [sample[0], sample[1]] {
                let pcm = float_to_pcm16(value);
                file.write_all(&pcm.to_le_bytes())?;
            }
        }
        offset += count;
    }
    file.flush()?;

    Ok(Output {
        human: format!(
            "Rendered Sound pass '{}' -> {} samples @ {} Hz ({:.3}s) -> {}",
            pass.name,
            sample_count,
            options.sample_rate,
            options.duration,
            output.display()
        ),
        json: json!({
            "ok": true,
            "project": loaded.manifest.project.name,
            "pass": pass.name,
            "output": output,
            "sample_rate": options.sample_rate,
            "samples": sample_count,
            "duration": options.duration,
            "channels": 2,
            "format": "pcm_s16le",
        }),
    })
}

pub(super) fn check_sound_passes(
    context: &HeadlessContext,
    loaded: &LoadedManifest,
) -> Result<usize> {
    let mut count = 0usize;
    for pass in loaded
        .manifest
        .passes
        .iter()
        .filter(|pass| pass.kind == PassKind::Sound)
    {
        let project = build_sound_project(loaded, pass, 44_100)?;
        let mut runtime = Runtime::new(context)?;
        runtime
            .load_project(&project)
            .with_context(|| format!("Sound pass '{}' failed to compile", pass.name))?;
        crate::uniforms::apply_to_runtime(
            &mut runtime,
            &crate::uniforms::defaults(&loaded.manifest.uniforms),
        )?;
        count += 1;
    }
    Ok(count)
}

fn validate_audio_options(options: &RenderAudioOptions) -> Result<()> {
    if !options.duration.is_finite()
        || options.duration <= 0.0
        || options.duration > MAX_SOUND_SECONDS
    {
        bail!("--duration must be finite and in (0, {MAX_SOUND_SECONDS}] seconds");
    }
    if !(8_000..=192_000).contains(&options.sample_rate) {
        bail!("--sample-rate must be in 8000..=192000");
    }
    Ok(())
}

fn select_sound_pass<'a>(loaded: &'a LoadedManifest, requested: Option<&str>) -> Result<&'a Pass> {
    if let Some(name) = requested {
        let pass = loaded
            .manifest
            .passes
            .iter()
            .find(|pass| pass.name == name)
            .with_context(|| format!("unknown Sound pass '{name}'"))?;
        if pass.kind != PassKind::Sound {
            bail!("pass '{name}' is {:?}, not a Sound pass", pass.kind);
        }
        return Ok(pass);
    }

    let mut sounds = loaded
        .manifest
        .passes
        .iter()
        .filter(|pass| pass.kind == PassKind::Sound);
    let pass = sounds.next().context("project has no Sound pass")?;
    if sounds.next().is_some() {
        bail!("project has multiple Sound passes; select one with --pass");
    }
    Ok(pass)
}

fn build_sound_project(loaded: &LoadedManifest, pass: &Pass, sample_rate: u32) -> Result<Project> {
    let source = expand_pass(loaded, pass)?.text;
    let lowered = format!(
        "{source}
uniform int {SOUND_OFFSET_UNIFORM};
         void mainCompute(ivec2 coord) {{
             int samp = {SOUND_OFFSET_UNIFORM} + coord.x;
             float soundTime = float(samp) / {sample_rate}.0;
             vec2 stereo = mainSound(samp, soundTime);
             imageStore(iOutput, coord, vec4(stereo, 0.0, 1.0));
         }}
"
    );

    let mut project = Project::new(&format!("{} Sound", loaded.manifest.project.name))?;
    add_manifest_assets(&mut project, loaded)?;

    project.add_pass(SOUND_COMPUTE_PASS, NativePassKind::Compute, &lowered)?;
    project.set_pass_resolution(SOUND_COMPUTE_PASS, SOUND_CHUNK_SAMPLES, 1)?;
    project.set_pass_format(SOUND_COMPUTE_PASS, NativeRenderFormat::Rgba32f)?;
    project.set_compute_local_size(SOUND_COMPUTE_PASS, 64, 1, 1)?;

    for storage in &pass.storage {
        project.bind_storage_buffer(
            SOUND_COMPUTE_PASS,
            storage.binding,
            &storage.name,
            storage.size,
        )?;
    }

    for input in &pass.inputs {
        let kind = loaded.manifest.infer_input_kind(input)?;
        let native_kind = match kind {
            InputKind::Pass => bail!(
                "Sound pass '{}' uses pass input '{}'; offline Sound currently requires static/keyboard/music inputs",
                pass.name,
                input.source
            ),
            InputKind::Texture => NativeInputKind::Texture,
            InputKind::Cubemap => NativeInputKind::Cubemap,
            InputKind::Volume => NativeInputKind::Volume,
            InputKind::Keyboard => NativeInputKind::Keyboard,
            InputKind::Music => NativeInputKind::Music,
            InputKind::Video => bail!(
                "Sound pass '{}' cannot use a video input in deterministic offline audio rendering",
                pass.name
            ),
            InputKind::Webcam => bail!("Sound pass '{}' cannot use live webcam input", pass.name),
        };
        project.add_input_output(
            SOUND_COMPUTE_PASS,
            input.channel.into(),
            native_kind,
            &input.source,
            u32::from(
                input
                    .output
                    .color_index()
                    .expect("sound inputs cannot use mesh depth"),
            ),
            input.frame == FrameRef::Previous,
            match input.filter {
                Filter::Mipmap => NativeFilter::Mipmap,
                Filter::Linear => NativeFilter::Linear,
                Filter::Nearest => NativeFilter::Nearest,
            },
            match input.wrap {
                Wrap::Clamp => NativeWrap::Clamp,
                Wrap::Repeat => NativeWrap::Repeat,
            },
        )?;
    }

    project.add_pass(
        SOUND_IMAGE_PASS,
        NativePassKind::Image,
        "void mainImage(out vec4 c, in vec2 p) { c = texture(iChannel0, vec2(0.5)); }
",
    )?;
    project.add_input(
        SOUND_IMAGE_PASS,
        0,
        NativeInputKind::Pass,
        SOUND_COMPUTE_PASS,
        false,
        NativeFilter::Nearest,
        NativeWrap::Clamp,
    )?;
    Ok(project)
}

fn write_wav_header(file: &mut fs::File, sample_rate: u32, samples: u32) -> Result<()> {
    let channels = 2u16;
    let bits_per_sample = 16u16;
    let bytes_per_sample = u32::from(bits_per_sample / 8);
    let data_bytes = samples
        .checked_mul(u32::from(channels))
        .and_then(|value| value.checked_mul(bytes_per_sample))
        .context("WAV data size overflow")?;
    let riff_size = 36u32
        .checked_add(data_bytes)
        .context("WAV RIFF size overflow")?;
    let byte_rate = sample_rate
        .checked_mul(u32::from(channels))
        .and_then(|value| value.checked_mul(bytes_per_sample))
        .context("WAV byte rate overflow")?;
    let block_align = channels * (bits_per_sample / 8);

    file.write_all(b"RIFF")?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&channels.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&bits_per_sample.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    Ok(())
}

fn float_to_pcm16(value: f32) -> i16 {
    if !value.is_finite() {
        return 0;
    }
    (value.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_conversion_clamps_and_handles_non_finite_values() {
        assert_eq!(float_to_pcm16(1.0), i16::MAX);
        assert_eq!(float_to_pcm16(-2.0), -i16::MAX);
        assert_eq!(float_to_pcm16(f32::NAN), 0);
    }
}
