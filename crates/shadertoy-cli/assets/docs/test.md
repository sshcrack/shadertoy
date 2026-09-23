Shader tests
============

Declare deterministic tests as [[test]] entries in ShaderToy.toml and run:

  shadertoy test
  shadertoy test --ci
  shadertoy test --preset medium --ci

--ci is non-mutating and is intended for build systems. --update deliberately
rewrites visual PNG references and cannot be combined with --ci.

Visual and numeric assertions
-----------------------------

A PNG reference keeps the original RMSE workflow:

  [[test]]
  name = "frame-120"
  frame = 120
  reference = "tests/frame-120.png"
  tolerance = 0.002

Buffer/compute/mesh tests can assert finite data, determinism, and output-resolution
independence. Select MRTs by numeric index or a mesh's named depth output with
`output = "depth"`:

  [[test]]
  name = "fft-grid"
  pass = "fft"
  frames = [0, 60, 120]
  resolutions = [[640, 360], [1280, 720]]
  assert_no_nan = true
  assert_no_inf = true
  assert_deterministic = true
  assert_resolution_independent = true
  raw_tolerance = 0.0



A depth-enabled mesh can target its raw 0..1 window-depth output directly:

  [[test]]
  name = "surface-depth"
  pass = "surface"
  output = "depth"
  frame = 0
  assert_no_nan = true
  assert_no_inf = true
  assert_deterministic = true

The same named selector is available for inspection with
`shadertoy inspect buffer surface --pass-output depth`.

Compare two uniform configurations without maintaining a PNG fixture:

  [[test]]
  name = "storm-differs-from-calm"
  frame = 120
  uniforms = { storm = 1.0 }
  reference_uniforms = { storm = 0.0 }
  min_rmse = 0.05

Use max_rmse when the two variants must stay close. If neither min_rmse nor
max_rmse is specified for reference_uniforms, tolerance is the maximum RMSE.

Performance, storage, and state
-------------------------------

GPU budgets use the renderer's attributed pass timings:

  max_gpu_ms = 8.0
  max_pass_gpu_ms = { waves = 2.5, foam = 1.0 }

Exact SSBO fixtures are project-relative binary files:

  [[test.storage]]
  name = "particle-state"
  reference = "tests/particle-state.bin"

Use assert_state_roundtrip = true to serialize persistent pass/SSBO state into
a .ststate artifact internally, load it into a fresh runtime, and verify exact
SSBO plus raw pass restoration. This is useful for catching regressions in
feedback/state persistence.
