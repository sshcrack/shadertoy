Host audio for audio-reactive shaders
=====================================

A `music` channel (512x2 spectrum/waveform texture) and the iAudio* uniforms
are fed by the CLI's host audio. Every rendering command uses it: render,
render-frames, render-video, test, profile, sweep, inspect, state, replay,
trace, blind, experiment, and the live preview.

Sources
-------

  none        default; deterministic silence (iAudioAvailable 0, iAudioSilence 1)
  synthetic   deterministic beat-driven analysis; profiles balanced (default),
              bass, percussion, ambient; 120 BPM unless bpm is set. Matches
              the led-matrix shader preview generator.
  file        frame-accurate analysis of an audio file. PCM/float WAV is read
              natively; other formats (mp3, ogg, flac, ...) are decoded with
              ffmpeg (SHADERTOY_FFMPEG overrides the executable).

Configure a project default in ShaderToy.toml:

  [audio]
  source = "synthetic"
  profile = "percussion"
  bpm = 132

  [audio]
  source = "file"
  path = "assets/loop.wav"   # project-relative
  offset = 12.5              # seconds into the file at iTime = 0
  gain = 1.0                 # linear gain before analysis
  loop = false               # silent after the end unless true
  # bpm = 128                # override the detected tempo

Override it for one command with global flags (paths are cwd-relative):

  shadertoy render --frame 240 --audio synthetic --audio-profile bass
  shadertoy render-frames --range 0:600:60 --audio song.mp3 --audio-offset 30
  shadertoy preview --audio-bpm 140          # implies synthetic when unset
  shadertoy render --audio none              # force silence

A [[test]] case may pin its own audio; it wins over both the manifest and the
flags, so reference images stay reproducible:

  [[test]]
  name = "kick-frame"
  frame = 60
  audio = { source = "synthetic", profile = "bass" }
  reference = "tests/kick.png"

Inspect the analysis without rendering:

  shadertoy audio --time 1.0 --time 1.25
  shadertoy audio --audio song.wav --time 30 --json --arrays

File analysis
-------------

Values depend only on the requested time (no hidden per-frame state), so any
frame renders identically however it was reached. The spectrum follows
ShaderToy/Web Audio conventions: a 2048-point Blackman FFT whose first 512
bins (0 to ~12 kHz at 48 kHz) map -100..-30 dB to 0..1, smoothed with Web
Audio's 0.8 time constant. The waveform is the latest 512 mono samples.

  loudness             RMS level, -60..0 dBFS mapped to 0..1
  bass/mid/treble      mean spectrum level in 20-250 Hz / 0.25-4 kHz / 4-16 kHz
  onset, spectralFlux  normalized spectral flux (onset with ~80 ms release)
  kick/snare/hihat     band-limited onsets (<150 Hz, 0.3-5 kHz, >6 kHz)
  bpm, beatPhase       autocorrelation tempo; beat grid anchored on kicks
  beatConfidence       periodicity strength of the detected tempo
  beatStrength         exp(-7 * beatPhase), scaled by confidence
  energyTrend          0.5 +/- recent (1 s) vs long (8 s) loudness
  drop                 bass surge relative to the previous 4 s
  sectionChange        timbre change between consecutive 4 s windows
  stereo*              width (side/mid), balance (-1 left..1 right), correlation
  spectralCentroid     log-frequency centroid, 0..1
  silence              true when the 200 ms RMS is below -60 dBFS or past the end

Traces record synthetic audio and replay it exactly; trace capture rejects
audio files because they are not embedded in the trace.
