use shadertoy::{AudioInput, Filter, HeadlessContext, InputKind, PassKind, Project, Runtime, Wrap};
#[cfg(target_os = "linux")]
use std::sync::Mutex;

#[cfg(target_os = "linux")]
static EGL_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(target_os = "linux")]
fn pixel(runtime: &mut Runtime<'_>) -> [u8; 3] {
    let image = runtime.render(1, 1).expect("render audio probe");
    [image.pixels[0], image.pixels[1], image.pixels[2]]
}

#[cfg(target_os = "linux")]
#[test]
fn audio_input_reaches_uniforms_and_music_texture() {
    let _egl_guard = EGL_TEST_LOCK.lock().expect("lock EGL test context");
    let context = HeadlessContext::new(16, 16).expect("create headless OpenGL context");
    let mut project = Project::new("audio-input").expect("create project");
    project
        .add_pass(
            "image",
            PassKind::Image,
            r#"
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    fragColor = vec4(iAudioBass, sampleAudioSpectrum(iChannel0, 0.5), iAudioAvailable, 1.0);
}
"#,
        )
        .expect("add image")
        .add_input(
            "image",
            0,
            InputKind::Music,
            "music",
            false,
            Filter::Nearest,
            Wrap::Clamp,
        )
        .expect("bind music");

    let mut runtime = Runtime::new(&context).expect("create runtime");
    runtime.load_project(&project).expect("load project");
    assert_eq!(pixel(&mut runtime), [0, 0, 0], "silent default");

    let audio = AudioInput {
        available: true,
        silence: false,
        sample_rate: 48000.0,
        spectrum: vec![0.75; 64],
        waveform: vec![0.0; 64],
        bass: 0.5,
        ..AudioInput::default()
    };
    runtime.set_audio_input(&audio).expect("set audio");
    let [red, green, blue] = pixel(&mut runtime);
    assert!((i16::from(red) - 128).abs() <= 2, "bass {red}");
    assert!((i16::from(green) - 191).abs() <= 2, "spectrum {green}");
    assert_eq!(blue, 255, "available");

    runtime.clear_audio_input().expect("clear audio");
    assert_eq!(pixel(&mut runtime), [0, 0, 0], "cleared");
}
