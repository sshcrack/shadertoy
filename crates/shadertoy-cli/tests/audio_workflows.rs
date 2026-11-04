use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "shadertoy-cli-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temporary CLI test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn shadertoy(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_shadertoy"))
        .args(args)
        .output()
        .expect("run shadertoy test binary")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON ({error}): {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// 16-bit stereo WAV: a 60 Hz bass tone pulsed on every 120 BPM beat.
fn write_bass_wav(path: &Path, seconds: f32) {
    let rate = 48_000u32;
    let frames = (seconds * rate as f32) as usize;
    let mut data = Vec::with_capacity(frames * 4);
    for n in 0..frames {
        let t = n as f32 / rate as f32;
        let envelope = (-(t % 0.5) * 6.0).exp();
        let sample = 0.8 * envelope * (2.0 * std::f32::consts::PI * 60.0 * t).sin();
        let value = (sample * 32767.0) as i16;
        data.extend_from_slice(&value.to_le_bytes());
        data.extend_from_slice(&value.to_le_bytes());
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&data);
    std::fs::write(path, bytes).expect("write WAV fixture");
}

/// 1x1 project whose pixel encodes (iAudioBass, low spectrum bin, iAudioAvailable).
fn write_audio_project(root: &Path, extra: &str) {
    std::fs::create_dir_all(root.join("shaders")).expect("create shaders");
    std::fs::create_dir_all(root.join("assets")).expect("create assets");
    write_bass_wav(&root.join("assets/bass.wav"), 2.0);
    std::fs::write(
        root.join("ShaderToy.toml"),
        format!(
            r#"format = 1

[project]
name = "audio-workflows"

[render]
width = 1
height = 1
fps = 60.0
preview_time = 0.0
{extra}
[[pass]]
name = "image"
kind = "image"
source = "shaders/image.frag"

[[pass.input]]
channel = 0
source = "music"
kind = "music"
filter = "nearest"
wrap = "clamp"
"#
        ),
    )
    .expect("write manifest");
    std::fs::write(
        root.join("shaders/image.frag"),
        r#"void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    fragColor = vec4(iAudioBass, sampleAudioSpectrum(iChannel0, 0.01), iAudioAvailable, 1.0);
}
"#,
    )
    .expect("write shader");
}

fn render_pixel(root: &Path, frame: &str, flags: &[&str]) -> [u8; 3] {
    let output_path = root.join(format!(
        "target/pixel-{}.png",
        flags.join("_").replace('/', "-")
    ));
    let mut args = vec![
        "render",
        "--project",
        root.to_str().unwrap(),
        "--frame",
        frame,
        "-o",
        output_path.to_str().unwrap(),
    ];
    args.extend_from_slice(flags);
    let output = shadertoy(&args);
    assert_success(&output);
    let image = image::open(&output_path).expect("open render").to_rgb8();
    image.get_pixel(0, 0).0
}

#[cfg(target_os = "linux")]
#[test]
fn audio_sources_drive_music_channels_and_uniforms() {
    let temp = TempRoot::new("audio-sources");
    let root = temp.path();
    write_audio_project(root, "");

    assert_eq!(
        render_pixel(root, "30", &[]),
        [0, 0, 0],
        "default is silent"
    );

    let synthetic = render_pixel(root, "30", &["--audio", "synthetic"]);
    assert_eq!(
        synthetic[2], 255,
        "synthetic audio is available: {synthetic:?}"
    );
    assert!(
        synthetic[0] > 200,
        "synthetic kick drives bass: {synthetic:?}"
    );
    let bass_profile = render_pixel(root, "45", &["--audio-profile", "bass"]);
    let balanced = render_pixel(root, "45", &["--audio", "synthetic"]);
    assert!(
        bass_profile[0] > balanced[0],
        "{bass_profile:?} vs {balanced:?}"
    );

    let wav = root.join("assets/bass.wav");
    let on_beat = render_pixel(root, "31", &["--audio", wav.to_str().unwrap()]);
    let off_beat = render_pixel(root, "58", &["--audio", wav.to_str().unwrap()]);
    assert_eq!(on_beat[2], 255);
    assert!(
        on_beat[1] > 150,
        "60 Hz energy fills the low bins: {on_beat:?}"
    );
    assert!(on_beat[0] > off_beat[0], "{on_beat:?} vs {off_beat:?}");
    let past_end = render_pixel(root, "180", &["--audio", wav.to_str().unwrap()]);
    assert_eq!(past_end[1], 0, "silent after the file ends: {past_end:?}");
    let looped = render_pixel(
        root,
        "151",
        &["--audio", wav.to_str().unwrap(), "--audio-loop"],
    );
    assert!(looped[1] > 150, "looping restarts the file: {looped:?}");
}

#[cfg(target_os = "linux")]
#[test]
fn manifest_audio_section_is_overridable_and_reported() {
    let temp = TempRoot::new("audio-manifest");
    let root = temp.path();
    write_audio_project(
        root,
        "\n[audio]\nsource = \"file\"\npath = \"assets/bass.wav\"\n",
    );

    let check = shadertoy(&["check", "--project", root.to_str().unwrap(), "--json"]);
    assert_success(&check);
    let check = json(&check);
    assert_eq!(check["audio"]["source"], "file");
    let bpm = check["audio"]["bpm"].as_f64().unwrap();
    assert!((bpm - 120.0).abs() < 3.0, "detected bpm {bpm}");

    let from_manifest = render_pixel(root, "31", &[]);
    assert_eq!(from_manifest[2], 255);
    assert!(from_manifest[1] > 150, "{from_manifest:?}");
    assert_eq!(render_pixel(root, "31", &["--audio", "none"]), [0, 0, 0]);

    let probe = shadertoy(&[
        "audio",
        root.to_str().unwrap(),
        "--time",
        "0.52",
        "--time",
        "0.9",
        "--json",
    ]);
    assert_success(&probe);
    let probe = json(&probe);
    let samples = probe["samples"].as_array().unwrap();
    let kick = |index: usize| samples[index]["audio"]["kick"].as_f64().unwrap();
    assert!(kick(0) > kick(1), "kick decays between beats: {samples:?}");
}

#[cfg(target_os = "linux")]
#[test]
fn test_cases_pin_audio_against_cli_overrides() {
    let temp = TempRoot::new("audio-tests");
    let root = temp.path();
    write_audio_project(
        root,
        r#"
[[test]]
name = "synthetic-kick"
frame = 30
audio = { source = "synthetic", profile = "bass" }
reference = "tests/kick.png"

[[test]]
name = "silent"
frame = 30
reference = "tests/silent.png"
"#,
    );
    let project = root.to_str().unwrap();
    // Baselines are written with the pinned audio even when the flags say otherwise.
    assert_success(&shadertoy(&[
        "test",
        "--project",
        project,
        "--update",
        "--audio",
        "none",
    ]));
    let kick = image::open(root.join("tests/kick.png"))
        .expect("kick baseline")
        .to_rgb8();
    assert_eq!(
        kick.get_pixel(0, 0).0[2],
        255,
        "pinned synthetic audio was used"
    );
    let silent = image::open(root.join("tests/silent.png"))
        .expect("silent baseline")
        .to_rgb8();
    assert_eq!(silent.get_pixel(0, 0).0, [0, 0, 0]);

    assert_success(&shadertoy(&["test", "--project", project]));
    assert_success(&shadertoy(&[
        "test",
        "--project",
        project,
        "--audio",
        "none",
    ]));
    // The unpinned case follows the override and no longer matches its silent baseline.
    let output = shadertoy(&[
        "test",
        "--project",
        project,
        "--json",
        "--audio",
        "synthetic",
    ]);
    assert!(
        !output.status.success(),
        "silent case must see the override"
    );
    let report = json(&output);
    assert_eq!(report["failed"], 1, "{report}");
}

#[test]
fn invalid_audio_configuration_is_rejected() {
    let temp = TempRoot::new("audio-invalid");
    let root = temp.path();
    write_audio_project(root, "\n[audio]\nsource = \"none\"\nprofile = \"bass\"\n");
    let output = shadertoy(&["check", "--project", root.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("profile/bpm require"));

    write_audio_project(
        root,
        "\n[audio]\nsource = \"file\"\npath = \"assets/missing.wav\"\n",
    );
    let output = shadertoy(&["check", "--project", root.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));

    let output = shadertoy(&["audio", "--audio", "synthetic", "--audio-offset", "2"]);
    assert!(!output.status.success());
}
