use super::*;
use crate::audio::{AudioDriver, AudioSection, audio_input_json};

#[derive(Debug, Clone)]
pub struct AudioProbeOptions {
    pub project: Option<PathBuf>,
    pub times: Vec<f32>,
    pub fps: Option<f32>,
    pub arrays: bool,
}

pub fn probe_audio(options: &AudioProbeOptions) -> Result<Output> {
    // A project is optional so `shadertoy audio --audio song.wav --time 3` works anywhere.
    let loaded = match &options.project {
        Some(path) => Some(LoadedManifest::load(path)?),
        None => LoadedManifest::load(".").ok(),
    };
    let (root, section) = match &loaded {
        Some(loaded) => (loaded.root.clone(), loaded.manifest.audio.clone()),
        None => (std::env::current_dir()?, AudioSection::default()),
    };
    let driver = AudioDriver::resolve(&root, &section, None)?;
    let fps = options
        .fps
        .or(loaded.as_ref().map(|loaded| loaded.manifest.render.fps))
        .unwrap_or(60.0);
    validate_fps(fps)?;

    let mut human = format!("audio: {}", driver.describe());
    let mut samples = Vec::new();
    for &time in &options.times {
        if !time.is_finite() || time < 0.0 {
            bail!("--time must be a finite non-negative number of seconds");
        }
        let Some(audio) = driver.input_at(time, fps) else {
            human.push_str(&format!("\n  t={time:.3}s  silent (iAudioAvailable = 0)"));
            samples.push(json!({ "time": time, "audio": null }));
            continue;
        };
        human.push_str(&format!(
            "\n  t={time:.3}s  loud {:.2}  bass {:.2}  mid {:.2}  treble {:.2}  kick {:.2}  snare {:.2}  hihat {:.2}  onset {:.2}  beat {:.2}@{:.1}bpm{}",
            audio.loudness,
            audio.bass,
            audio.mid,
            audio.treble,
            audio.kick,
            audio.snare,
            audio.hihat,
            audio.onset,
            audio.beat_phase,
            audio.bpm,
            if audio.silence { "  (silence)" } else { "" },
        ));
        samples.push(json!({
            "time": time,
            "audio": audio_input_json(&audio, options.arrays),
        }));
    }
    Ok(Output {
        human,
        json: json!({
            "ok": true,
            "project": loaded.as_ref().map(|loaded| loaded.manifest.project.name.clone()),
            "fps": fps,
            "source": driver.to_json(),
            "samples": samples,
        }),
    })
}
