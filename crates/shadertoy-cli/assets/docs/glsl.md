Shader sources are compiled with a host-provided ShaderToy-compatible GLSL
prelude. User source is inserted after a #line 1 directive, so compile errors
normally report line numbers relative to the shader file rather than the
generated wrapper.

Entry points
------------

Image, buffer, and mesh fragment passes:

  void mainImage(out vec4 fragColor, in vec2 fragCoord)

Mesh vertex passes define ordinary GLSL main() themselves. The host prepends:

  layout(location = 0) in vec2 a_grid;
  uniform ivec2 iMeshSize;
  uniform int iMeshInstances;

a_grid spans 0..1 across the generated indexed grid. gl_VertexID and
gl_InstanceID are available normally, and the shader owns gl_Position and all
user varyings.

Cubemap fragment passes:

  void mainCubemap(
      out vec4 fragColor,
      in vec2 fragCoord,
      in vec3 rayOrigin,
      in vec3 rayDirection)

Compute passes:

  void mainCompute(ivec2 coord)

The compute wrapper bounds-checks coord against iResolution before calling
mainCompute.

Common uniforms
---------------

Fragment, mesh vertex, and compute passes receive:

  vec3  iResolution
  float iTime
  float iTimeDelta
  float iFrameRate
  int   iFrame
  vec4  iMouse
  vec4  iDate
  vec3  iChannelResolution[16]
  float iChannelTime[16]

Compute passes additionally receive:

  int iIteration

Configured inputs are declared as iChannel0 through iChannel15. Their GLSL
sampler type is derived from the input kind:

  sampler2D    pass/buffer/compute/mesh output, texture, keyboard, music, video, webcam
  samplerCube  cubemap
  sampler3D    volume

Only configured channels are declared.

Compute outputs
---------------

Compute output 0 is:

  image2D iOutput

For extra_outputs, the host declares image2D iOutput1 through iOutput7 as
needed, each with the matching image format qualifier and binding.

Fragment multiple-render-target outputs are not predeclared. Output 0 is the
mainImage result; declare additional locations yourself, for example:

  layout(location = 1) out vec4 normalVelocity;

Audio helpers and semantic uniforms
-----------------------------------

The prelude provides:

  float sampleAudioSpectrum(sampler2D channel, float x)
  float sampleAudioWaveform(sampler2D channel, float x)

x is clamped to 0..1. The waveform helper maps the stored 0..1 texture sample
back to -1..1.

The shader-facing audio aliases are:

  iAudioLoudness
  iAudioBass
  iAudioMid
  iAudioTreble
  iAudioOnset
  iAudioKick
  iAudioSnare
  iAudioHihat
  iAudioBpm
  iAudioBeatPhase
  iAudioBeatConfidence
  iAudioBeatStrength
  iAudioStereoWidth
  iAudioStereoBalance
  iAudioStereoCorrelation
  iAudioEnergyTrend
  iAudioDrop
  iAudioSectionChange
  iAudioSpectralCentroid
  iAudioSpectralFlux
  iAudioAvailable
  iAudioSilence
  iAudioSampleRate

For the keyboard/music texture layouts, see:

  shadertoy docs channels

For target formats, iterations, MRTs, and SSBOs, see:

  shadertoy docs passes
