.ststate is ShaderToy CLI's resumable debugging artifact.

It stores:
  - render dimensions
  - fixed frame rate
  - shader time and iFrame
  - lossless RGBA32F snapshots of persistent 2D buffer/compute/mesh passes
  - each captured buffer's actual dimensions (including fixed-size passes)
  - each captured buffer's render-target format
  - optional named SSBO byte contents (format 4)

SSBO capture is opt-in:

  shadertoy state capture --frame 300 --include-storage -o target/frame300.ststate

Captured SSBOs can be replaced from exact-size binary files:

  shadertoy state set-storage target/frame300.ststate \
    particles=fixtures/particles.bin \
    -o target/modified.ststate

The captured texture/timing state is deterministic and independent of wall-clock playback.

Capture:
  shadertoy state capture --frame 300 --set storm=1.0 -o target/frame300.ststate

Declared custom uniforms accept the same --set NAME=VALUE overrides used by render, profiling, and runtime inspection. The override affects the deterministic frames used to build the captured buffer/SSBO state.

Inspect:
  shadertoy state inspect target/frame300.ststate --json

Modify one buffer:
  shadertoy state set target/frame300.ststate \
    buffer-a=fixtures/known.png \
    -o target/modified.ststate

Resume:
  shadertoy render --state target/modified.ststate -o target/next.png


State format 4 extends format 3 with optional named SSBO byte payloads. Format 3
introduced per-buffer dimensions and render-target formats so typed buffer state
cannot be silently restored into an incompatible format. Formats 1 through 3
remain readable. A
state containing only fixed-size persistent passes can be resumed at a different
output resolution; output-sized feedback buffers still require the captured
resolution so their state is not silently discarded.

Cubemap limitation
------------------

.ststate captures persistent 2D buffer and compute targets only. Cubemap pass
faces are not included in state artifacts, and cubemap passes cannot be
replaced with render --set-buffer or state set. Named-pass PNG snapshots
likewise support the final Image pass and 2D buffer/compute/mesh passes, not cubemap
passes.

Use ordinary cubemap assets for fixed cube data, or move debuggable persistent
state into a 2D buffer/compute/mesh pass when capture/override is required.
