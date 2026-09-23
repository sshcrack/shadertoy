Every pass may bind up to sixteen inputs: iChannel0 through iChannel15.

An input selects:
  channel = 0..15
  source  = another pass, an asset, "keyboard", "music", or "webcam"
  output  = pass render-target index, default 0, or "depth" for a mesh (pass inputs only)
  frame   = "current" or "previous" (pass inputs only; output 0 for previous)
  filter  = "nearest", "linear", or "mipmap"
  wrap    = "clamp" or "repeat"

Use:

  shadertoy inspect channels image --json

to see exactly what a pass consumes.

Sampler settings are per input, including when the same source is connected to
multiple channels with different sampling behavior:

  [[pass.input]]
  channel = 0
  source = "spectrum"
  output = 0
  filter = "nearest"
  wrap = "repeat"

nearest is the right choice for exact numerical grids such as FFT stages.
linear enables interpolated sampling, while mipmap uses trilinear
minification and linear magnification. clamp maps out-of-range coordinates to
the edge texel; repeat wraps them. Omitting these fields keeps the existing
linear/repeat defaults.

The same controls are available when editing a project from the CLI:

  shadertoy channel set image 0 spectrum --output 0 --filter nearest --wrap repeat

For a multi-render-target pass, use --output 1 (or another valid attachment)
to bind that target without adding another producer pass. For a depth-enabled
mesh, --output depth binds its raw 0..1 window-depth texture.

Media channels
--------------

`kind = "video"` references a local `[[asset]] kind = "video"`. Headless
rendering decodes the frame for deterministic shader time through ffmpeg/ffprobe,
updates the channel texture before each rendered frame, and exposes that time in
`iChannelTime[channel]`.

`kind = "webcam"` uses reserved source `"webcam"`. It is available only in live
preview: the browser asks for camera permission, downsamples frames to 320x240,
and sends RGBA frames to the native renderer over the existing WebSocket.
Headless render/test/replay and preview recording reject webcam input because a
live camera cannot be deterministic.

Keyboard and music texture layout
---------------------------------

The reserved source "keyboard" is a 256x3 sampler2D. Key codes are x texels
0..255. Use texelFetch for exact key state:

  float down    = texelFetch(iChannel0, ivec2(key, 0), 0).r;
  float pressed = texelFetch(iChannel0, ivec2(key, 1), 0).r;
  float toggled = texelFetch(iChannel0, ivec2(key, 2), 0).r;

Row 0 is 1 while the key is held. Row 1 is a one-frame press pulse. Row 2
toggles between 0 and 1 on each press. The texture stores 0/1 in all RGBA
components.

The reserved source "music" is a 512x2 sampler2D:

  row 0  normalized spectrum, low frequencies on the left
  row 1  waveform encoded from -1..1 into 0..1

The GLSL prelude also provides sampleAudioSpectrum() and sampleAudioWaveform();
the latter maps row 1 back to -1..1.

The headless CLI does not capture live host audio. A music channel therefore
uses deterministic synthetic silence unless an embedding host supplies audio:
spectrum samples are 0, waveform samples are centered at 0.5,
iAudioAvailable is 0, and iAudioSilence is 1. This makes check/render/test/state
runs repeatable instead of depending on microphone or system-audio state.

For the sampler type selected for every input kind and the complete GLSL
prelude contract, see:

  shadertoy docs glsl
