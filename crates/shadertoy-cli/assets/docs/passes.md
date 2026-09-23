Pass kinds:

  image    Exactly one final Image pass is required.
  buffer   Offscreen 2D fragment pass. May be consumed by later passes and/or itself.
  mesh     Offscreen indexed-grid raster pass with a user-authored vertex stage.
  cubemap  Cubemap fragment pass.
  compute  Fixed-size OpenGL 4.3 compute pass with a writable image output.
  sound    ShaderToy mainSound(int,float) pass; rendered offline with render-audio.

Fragment passes use ShaderToy-style mainImage/mainCubemap entry points. Compute
passes implement:

  void mainCompute(ivec2 coord)

The host provides iResolution, iTime, iTimeDelta, iFrameRate, iFrame, iMouse,
iDate, iChannelResolution, iChannelTime, configured iChannel samplers, iIteration, and a
writable image2D named iOutput. The wrapper bounds-checks global invocation
coordinates, so local workgroups do not need to divide the pass dimensions.
Because mainCompute receives 2D coordinates, local_size Z must be 1.

Inspect one pass:

  shadertoy inspect pass buffer-a
  shadertoy render --pass buffer-a -o target/buffer-a.png

Buffer passes may optionally pin their render target to a fixed resolution.
Compute passes always require a fixed resolution:

  [[pass]]
  name = "spectrum"
  kind = "compute"
  source = "shaders/spectrum.comp"
  width = 256
  height = 256
  format = "rg32f"
  local_size = [8, 8, 1]
  iterations = 8

Available 2D target formats are r32f, rg32f, rgba16f, and rgba32f. Buffer and mesh
passes default to rgba32f and may use the same format field.

For fixed passes, iResolution uses the pass dimensions and
iChannelResolution reports the actual dimensions of every input. Changing
preview/output resolution does not resize or clear fixed pass state.

Mesh passes
-----------

A mesh pass keeps the normal 2D fragment-pass contract but adds a vertex source
and host-generated indexed grid:

  [[pass]]
  name = "ocean-mesh"
  kind = "mesh"
  vertex = "shaders/ocean-mesh.vert"
  source = "shaders/ocean-mesh.frag"
  format = "rgba16f"

  [pass.mesh]
  topology = "grid"
  size = [257, 257]
  instances = 6
  depth = true
  cull = "none"
  clear = [0.0, 0.0, 0.0, 0.0]

The vertex stage receives a_grid in 0..1, gl_VertexID/gl_InstanceID,
iMeshSize/iMeshInstances, the normal timing/channel/custom uniforms, and the
same configured SSBO bindings as the fragment stage. The shader writes
gl_Position; the CLI supplies no camera or matrices. User varyings pass through
unchanged.

Use explicit-LOD sampling such as textureLod or texelFetch in vertex shaders.
A pass input with filter = "mipmap" regenerates the producer's current mip
chain when the consumer binds it.

When depth = true, the rasterizer uses LESS depth testing and exposes raw
0..1 window depth as a named pass output:

  [[pass.input]]
  channel = 3
  source = "ocean-mesh"
  kind = "pass"
  output = "depth"
  filter = "nearest"
  wrap = "clamp"

The named depth output is separate from numeric MRT indices. Use
--wireframe ocean-mesh with render, render-frames, or preview for a flat,
depth-tested grid overlay. check --json and profile --json report mesh grid,
instance, vertex, and triangle counts.

Quality presets can override geometry independently from target resolution:

  [preset.low.pass.ocean-mesh]
  mesh_size = [129, 129]
  mesh_instances = 4

See docs/MESH_PASSES.md in the repository for the complete mesh contract.

Multiple render targets
-----------------------

Buffer, compute, and mesh passes can expose multiple 2D color outputs. format describes
output 0; extra_outputs appends outputs 1 through 7:

  [[pass]]
  name = "gbuffer"
  kind = "buffer"
  source = "shaders/gbuffer.frag"
  format = "rgba16f"
  extra_outputs = ["rg32f", "rgba16f"]

A fragment pass writes output 0 through the normal mainImage result. Additional
fragment outputs use ordinary GLSL locations:

  layout(location = 1) out vec4 normalVelocity;
  layout(location = 2) out vec4 materialData;

Compute passes receive image2D uniforms iOutput, iOutput1, iOutput2, and so on,
with the declared format of each target. This makes one dispatch suitable for
producing related simulation fields without repeating the work.

A downstream channel selects an attachment with output:

  [[pass.input]]
  channel = 0
  source = "gbuffer"
  output = 1
  filter = "nearest"
  wrap = "clamp"

Output 0 remains the default when output is omitted. Nonzero outputs are
current-frame dependencies in 2.2.0; previous-frame feedback is intentionally
limited to output 0 so .ststate capture/restore remains complete and
deterministic.

Compute iterations
------------------

iterations dispatches the same compute pass multiple times in one rendered
frame. iIteration is zero-based and a memory barrier is inserted between
dispatches. The same iOutput image remains bound, so imageLoad/imageStore can
carry per-pixel state from one dispatch to the next. Cross-pixel algorithms
that require strict Jacobi-style ping-pong should use separate storage regions
or ordinary previous-frame pass feedback.

Shader storage buffers
----------------------

Compute, fragment, and mesh vertex passes may bind persistent SSBOs by name:

  [[pass.storage]]
  binding = 3
  name = "particles"
  size = 4194304

Declare the matching block in GLSL:

  layout(std430, binding = 3) buffer Particles {
      vec4 particleData[];
  };

Storage with the same name and size is shared across passes even when each pass
uses a different binding index. New storage is zero-initialized. OpenGL 4.3
image load/store, SSBO operations, and atomics are therefore available to
advanced pipelines. Sharing an SSBO does not create an execution dependency;
when one pass must observe another pass's writes in the same frame, add an
explicit current-frame pass input to order them. A pass using compute or storage
buffers requires OpenGL 4.3; ordinary projects continue to work on the existing
OpenGL 4.1 path.

.ststate format 4 can snapshot named SSBO bytes as well as persistent 2D pass
textures. Use `shadertoy state capture --include-storage ...` when SSBO state must
survive resume. SSBO capture remains opt-in because large buffers can make state
artifacts substantially bigger.

Each pass input has independent sampler state. For an FFT/simulation grid,
bind the fixed grid with exact texel sampling:

  [[pass.input]]
  channel = 0
  source = "spectrum"
  filter = "nearest"
  wrap = "repeat"

Available filters are nearest, linear, and mipmap; wrap modes are clamp and
repeat. The settings are applied independently per iChannel, even if two
channels reference the same source.

Sound passes
------------

A Sound source implements ShaderToy's conventional entry point:

  vec2 mainSound(int samp, float time)

`shadertoy check` compiles Sound passes. Render deterministic stereo PCM WAV with:

  shadertoy render-audio --pass soundtrack --duration 10 --sample-rate 44100 -o target/sound.wav

The offline Sound path uses fixed sample indices/times rather than wall clock.
Static texture/cubemap/volume, keyboard, music, and SSBO declarations are
accepted by the manifest. Video and webcam inputs are rejected by offline Sound
because a changing live/media texture cannot be sampled independently for every
audio sample.
