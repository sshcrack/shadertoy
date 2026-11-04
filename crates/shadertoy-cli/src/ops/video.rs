use super::*;
use std::io::Write;
use std::process::{Command, Stdio};

const DEFAULT_VIDEO_DURATION_SECONDS: f32 = 5.0;
const MAX_VIDEO_FRAMES: u32 = 1_000_000;

pub fn render_video_project(options: &RenderVideoOptions) -> Result<Output> {
    let loaded = LoadedManifest::load_with_preset(&options.project, options.preset.as_deref())?;
    ensure_source_files_exist(&loaded)?;
    let media = crate::media::MediaInputs::new_headless(&loaded)?;

    let (width, height) = render::resolve_dimensions(&loaded, None, options.width, options.height)?;
    let fps = render::resolve_fps(&loaded, None, options.fps)?;
    if options.start_frame < 0 {
        bail!("--start-frame must be non-negative");
    }

    let frame_count = resolve_frame_count(options.frames, options.duration, fps)?;
    let end_frame = options
        .start_frame
        .checked_add(i32::try_from(frame_count - 1).context("video frame count exceeds i32")?)
        .context("video end frame overflow")?;

    let selected_pass = options
        .pass
        .as_deref()
        .unwrap_or(&loaded.manifest.final_pass().name);
    let pass = loaded
        .manifest
        .passes
        .iter()
        .find(|pass| pass.name == selected_pass)
        .with_context(|| format!("unknown render pass '{selected_pass}'"))?;
    if matches!(pass.kind, PassKind::Cubemap | PassKind::Sound) {
        bail!("render-video only supports the final image and 2D buffer/compute/mesh passes");
    }
    let (selected_width, selected_height) = loaded.manifest.pass_dimensions(pass, width, height);

    let output = options
        .output
        .clone()
        .unwrap_or_else(|| loaded.root.join("target/render.mp4"));
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut ffmpeg = ffmpeg_command(
        &output,
        selected_width,
        selected_height,
        fps,
        options.codec.as_deref(),
    )?;
    let mut child = ffmpeg
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            "failed to launch ffmpeg; install ffmpeg or set SHADERTOY_FFMPEG to its executable"
                .to_string()
        })?;
    let mut stdin = child.stdin.take().context("failed to open ffmpeg stdin")?;

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for video rendering")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_native_project(&loaded)?;
    runtime.load_project(&project)?;
    let uniform_values =
        crate::uniforms::parse_assignments(&loaded.manifest.uniforms, &options.set_uniforms)?;
    crate::uniforms::apply_to_runtime(&mut runtime, &uniform_values)?;

    let final_pass = loaded.manifest.final_pass().name.as_str();
    for frame in 0..=end_frame {
        if frame > 0 {
            runtime.tick_fixed(1.0 / fps, fps)?;
        }
        let media_time = runtime.time();
        media.update(&mut runtime, media_time)?;
        let final_image = runtime.render(width, height)?;
        if frame < options.start_frame {
            continue;
        }

        let image = if selected_pass == final_pass {
            final_image
        } else {
            runtime.snapshot_pass_rgb(selected_pass, selected_width, selected_height)?
        };
        let mut pixels = image.pixels;
        super::images::flip_rgb_rows(&mut pixels, image.width, image.height);
        stdin
            .write_all(&pixels)
            .with_context(|| format!("ffmpeg stopped while encoding frame {frame}"))?;
    }
    drop(stdin);

    let result = child
        .wait_with_output()
        .context("failed while waiting for ffmpeg")?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        bail!("ffmpeg failed with {}: {}", result.status, stderr.trim());
    }

    Ok(Output {
        human: format!(
            "Rendered {} deterministic frames ({}..={}) at {}x{} @ {} fps{} -> {}",
            frame_count,
            options.start_frame,
            end_frame,
            selected_width,
            selected_height,
            fps,
            options
                .pass
                .as_ref()
                .map(|name| format!(" pass '{name}'"))
                .unwrap_or_default(),
            output.display()
        ),
        json: json!({
            "ok": true,
            "audio": media.audio().to_json(),
            "project": loaded.manifest.project.name,
            "output": output,
            "pass": selected_pass,
            "width": selected_width,
            "height": selected_height,
            "fps": fps,
            "start_frame": options.start_frame,
            "end_frame": end_frame,
            "frames": frame_count,
        }),
    })
}

fn resolve_frame_count(frames: Option<u32>, duration: Option<f32>, fps: f32) -> Result<u32> {
    if frames.is_some() && duration.is_some() {
        bail!("use either --frames or --duration, not both");
    }
    if let Some(frames) = frames {
        if frames == 0 || frames > MAX_VIDEO_FRAMES {
            bail!("--frames must be in 1..={MAX_VIDEO_FRAMES}");
        }
        return Ok(frames);
    }

    let duration = duration.unwrap_or(DEFAULT_VIDEO_DURATION_SECONDS);
    if !duration.is_finite() || duration <= 0.0 {
        bail!("--duration must be a finite positive number");
    }
    let frames = (f64::from(duration) * f64::from(fps)).round();
    if frames < 1.0 || frames > f64::from(MAX_VIDEO_FRAMES) {
        bail!(
            "--duration resolves to an unsupported frame count; keep it within 1..={MAX_VIDEO_FRAMES}"
        );
    }
    Ok(frames as u32)
}

fn ffmpeg_command(
    output: &Path,
    width: u32,
    height: u32,
    fps: f32,
    codec_override: Option<&str>,
) -> Result<Command> {
    let executable = std::env::var_os("SHADERTOY_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let mut command = Command::new(executable);
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-s:v",
        &format!("{width}x{height}"),
        "-r",
        &format!("{fps:.6}"),
        "-i",
        "pipe:0",
        "-an",
    ]);

    if let Some(codec) = codec_override {
        command.args(["-c:v", codec]);
    } else {
        match output
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("mp4" | "m4v" | "mov") => {
                command.args(["-c:v", "libx264", "-crf", "18", "-preset", "medium"]);
                command.args([
                    "-vf",
                    "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                    "-pix_fmt",
                    "yuv420p",
                ]);
            }
            Some("webm") => {
                command.args(["-c:v", "libvpx-vp9", "-crf", "30", "-b:v", "0"]);
                command.args([
                    "-vf",
                    "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                    "-pix_fmt",
                    "yuv420p",
                ]);
            }
            Some("gif") => {}
            Some(_) | None => {}
        }
    }
    command.arg(output);
    Ok(command)
}

pub(super) fn parse_frame_range(value: &str) -> Result<Vec<i32>> {
    let parts = value.split(':').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        bail!("frame range must be START:END or START:END:STEP");
    }
    let start = parts[0]
        .parse::<i32>()
        .context("frame range START must be an integer")?;
    let end = parts[1]
        .parse::<i32>()
        .context("frame range END must be an integer")?;
    let step = if parts.len() == 3 {
        parts[2]
            .parse::<i32>()
            .context("frame range STEP must be an integer")?
    } else {
        1
    };
    if start < 0 || end < 0 {
        bail!("frame range values must be non-negative");
    }
    if end < start {
        bail!("frame range END must be greater than or equal to START");
    }
    if step <= 0 {
        bail!("frame range STEP must be positive");
    }

    let mut frames = Vec::new();
    let mut frame = start;
    while frame <= end {
        if frames.len() >= 1024 {
            bail!("frame range expands to more than 1024 entries");
        }
        frames.push(frame);
        match frame.checked_add(step) {
            Some(next) if next > frame => frame = next,
            _ => break,
        }
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inclusive_frame_ranges() {
        assert_eq!(parse_frame_range("0:4").unwrap(), vec![0, 1, 2, 3, 4]);
        assert_eq!(parse_frame_range("0:120:60").unwrap(), vec![0, 60, 120]);
        assert!(parse_frame_range("4:0").is_err());
        assert!(parse_frame_range("0:4:0").is_err());
    }

    #[test]
    fn duration_and_frame_count_are_mutually_exclusive() {
        assert!(resolve_frame_count(Some(10), Some(1.0), 60.0).is_err());
        assert_eq!(resolve_frame_count(None, Some(0.5), 60.0).unwrap(), 30);
    }
}
