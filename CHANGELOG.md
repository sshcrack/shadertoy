# Changelog

All notable changes to the Rust library and CLI are recorded here.

## Unreleased

- Added CLI host audio for `music` channels and `iAudio*` uniforms: deterministic silence (default), synthetic beat-driven analysis (balanced/bass/percussion/ambient profiles matching the led-matrix preview generator), or frame-accurate audio-file analysis (native PCM/float WAV, other formats through ffmpeg) with a ShaderToy-compatible 512-bin spectrum, waveform, band levels, kick/snare/hihat onsets, tempo/beat-phase detection, stereo, drop, and section-change features.
- Configure it with a manifest `[audio]` section, a per-`[[test]]` `audio` table that pins reference renders, or the global `--audio none|synthetic|FILE`, `--audio-profile`, `--audio-bpm`, `--audio-offset`, `--audio-gain`, and `--audio-loop` flags on every rendering command (including preview, profile, blind/experiment STTF sources, and replay). `check`, `render`, `render-frames`, and `render-video` report the resolved source in JSON, and traces record and exactly replay synthetic audio.
- Added `shadertoy audio` to print analysis values at chosen times and `shadertoy docs audio`.
- Added `st_audio_input` / `st_runtime_set_audio_input()` to the C ABI and `AudioInput` with `Runtime::set_audio_input()` / `clear_audio_input()` to the Rust library.

## 2.6.1

- Fixed first-frame mesh color/depth consumers by correcting wireframe-composite program validation so mesh construction no longer leaks `GL_INVALID_OPERATION` into subsequent sampler setup; this also fixes paused-preview mesh reload/preset-switch frames.
- STTF builds now retain resolved render width, height, and FPS defaults so `blind create` and `experiment` compare non-1280x720 project/STTF sources at matching dimensions while older STTF artifacts keep the legacy fallback.
- Promoted named mesh depth into inspection, trace intermediates, and manifest regression tests via `--pass-output depth` / `output = "depth"`.

## 2.6.0

- Added first-class `kind = "mesh"` raster passes with user-authored vertex shaders, generated indexed grids, instancing, shared channels/uniforms/SSBOs, culling, depth testing, and existing buffer-style color formats/MRTs.
- Added sampleable mesh `output = "depth"`, current-frame pass-output mip generation usable from vertex `textureLod`, and `--wireframe PASS` on `render`, `render-frames`, and `preview`.
- Added mesh geometry statistics to `check --json` and `profile --json`, mesh-density/instance quality-preset overrides, `pass add --kind mesh` scaffolding, plus end-to-end regression coverage for vertex-stage mip sampling, custom-uniform overrides, SSBO writes, depth consumption, and wireframe rendering.

## 2.5.1

- Added repeatable `--set NAME=VALUE` overrides to `blind create` and `experiment`, applying the same custom-uniform state to every rendered source; experiment profiling now uses the same overrides as its comparison frames.

## 2.5.0

- Extended per-pass inputs from four channels to sixteen (`iChannel0` through `iChannel15`) across manifests, native rendering, CLI mutation/inspection/graph/render workflows, sampler state, schema, and GLSL docs.

## 2.4.5

- Added --preview-transport auto|raw|mjpeg|png. Auto selects raw RGB24 WebSocket frames for loopback-bound previews and MJPEG for externally reachable binds; explicit raw, lossless PNG, and MJPEG overrides are available.
- Added direct WebGL2 upload for raw RGB preview frames, preserving the bounded latest-frame queue while removing image encode/decode entirely on the local fast path.


## 2.4.4

- Fixed live-preview pacing so render/readback work is no longer followed by an additional full frame interval; over-budget frames continue immediately instead of compounding work time with scheduler sleep.
- Moved live-preview frame transport off the graphics thread onto a bounded latest-frame worker. Preview now uses a persistent single-threaded FFmpeg MJPEG encoder for low-latency JPEG WebSocket frames (with an in-process JPEG fallback when FFmpeg is unavailable), so stale frames are dropped instead of queued and PNG encode/decode is removed from the live path.

## 2.4.3

- Fixed live preview quality-preset switching so each preset change starts with a cold graph/resource state instead of restoring persistent Buffer/Compute history from the previously selected tier. Custom-uniform review values still survive the switch.

## 2.4.2

- Split profiling samples into raw GPU execution timestamp intervals, CPU completion-wait durations, and the legacy post-completion attributed interval so host/fence stalls are no longer silently presented as shader execution.
- Added per-sample `sample_valid` diagnostics for timer queries that outrun asynchronous GPU work, plus raw sample details and MAD-based outlier flags; `profile --discard-outliers` excludes flagged samples from aggregates without deleting the raw evidence.
- Added an independent frame-level GPU timestamp interval alongside attributed-pass totals and completion-wait totals, giving optimization workflows a stable cross-run signal while retaining per-pass diagnostics.
- Added `shadertoy docs profile` and updated agent/README guidance for interpreting async-compute timings and fixed-resolution performance comparisons.


## 2.4.1

- Reworked the preview browser as a human-review control surface with prominent live quality-preset switching, common review resolutions, custom width/height controls, and clearer active quality/resolution/FPS status.
- Preview now keeps final Image output at the base project resolution across preset changes; explicit human-selected review resolutions also persist across presets. Preset `render_scale` can no longer make a tier appear faster in preview simply by shrinking the final image.
- `profile --preset NAME` now benchmarks at the base project output resolution by default and reports the preset-requested final scale separately, while still honoring internal pass resolution/iteration/local-size changes.


## 2.4.0

- Added `shadertoy experiment` for reproducible baseline/candidate and N-way comparisons across images, render directories, ShaderToy projects, STTF builds, quality presets, and Git revisions, with deterministic frame sets, RMSE/SSIM metrics, contact sheets, provenance, GPU profiles, and optional sealed blind evaluation.
- Expanded manifest-driven `shadertoy test` with preset/CI modes, uniform-vs-uniform RMSE bounds, total/per-pass GPU budgets, exact SSBO fixtures, and persistent buffer/SSBO state round-trip assertions.
- Added self-contained `.sttrace` capture/inspect/replay bundles containing STTF, final output, persistent state, source/asset hashes, graph/synchronization metadata, GPU timings, and optional intermediate pass readbacks with artifact integrity verification.
- Added top-level `shadertoy graph` with Graphviz DOT export and graph/resource diagnostics; `shadertoy check --pedantic` now turns advisory warnings such as unreachable passes, unused assets/uniforms, viewport-sized feedback, and unordered shared SSBO users into failures.


## 2.3.0

- Added first-class named quality presets in ShaderToy.toml with output render scaling and per-pass buffer/compute overrides; `check`, `build`, `preview`, `render`, `render-frames`, `render-video`, `sweep`, and `profile` accept `--preset NAME`, and preview exposes a live base/preset selector.
- Reworked GPU profiling around completion-synchronized pass timestamp boundaries so asynchronous compute stays attributed to the issuing pass; total GPU work is reported as the sum of those attributed intervals (avoiding driver-broken whole-frame timer queries), with per-pass median/p95 and optional `profile --sync-per-pass` maximum-isolation diagnostics.
- Added preset-aware blind project sources (`project:PATH@preset=NAME`), including deterministic frame sets and same-aspect-ratio normalization when quality tiers render at different resolutions.


## 2.2.9

- Preview now shows a rolling actual FPS measured from browser-displayed frames, alongside the configured target FPS.

## 2.2.8

- Fixed blind source auto-detection so ShaderToy projects and STTF builds take precedence over texture/image assets; bare Git revisions invoked from a nested ShaderToy project now compare that project instead of recursively collecting repository PNGs.
- Persisted typed custom-uniform values in STTF artifacts and restore them when loading a build, so project defaults survive `shadertoy build` and STTF blind comparisons preserve distinct configurations.

## 2.2.7

- Fixed per-pass GPU profiling attribution for deferred compute work by isolating pass completion while profiling, so compute-heavy passes no longer report near-zero time while their cost is charged to a later consumer.
- Added `shadertoy blind create` for source-agnostic A/B/C comparisons of individual images, image/render directories, ShaderToy project directories, STTF builds, and Git revisions, with whole render sets kept together under one anonymous label.
- Added direct STTF loading to the Rust/C runtime bridge so built artifacts can participate in deterministic render comparisons.


## 2.2.6

- Added bias-resistant blind parameter sweeps with randomized A/B/C variant labels and a public contact sheet/session report that omits parameter assignments.
- Added shadertoy blind judge to commit a selected anonymous variant plus rationale before the parameter mapping can be revealed.
- Added shadertoy blind reveal to unlock the mapping only after judgment and write a combined blind-reveal.json report containing the decision, selected settings, and full mapping.

## 2.2.5

- Added `--set NAME=VALUE` custom-uniform overrides to `state capture`, `inspect buffer`, and `inspect storage`, completing override support across deterministic runtime inspection/state-capture workflows.
- Added `shadertoy sweep` for bounded Cartesian custom-uniform sweeps with deterministic variant PNGs, named-pass support, JSON metadata, and contact-sheet generation.
- Expanded profiler statistics with median and p95 for each GPU pass, aggregate GPU timing, and CPU render-call timing while preserving the existing mean field for compatibility.

## 2.2.2

- Re-verified the complete v2.2 advanced GPU pipeline on the applied release branch, including compute passes, typed render targets, MRT routing, persistent/shared SSBOs, fixed-size passes, state capture/restore, sampler controls, and headless EGL rendering.
- Added native and CLI regression matrices covering all four typed compute formats (`r32f`, `rg32f`, `rgba16f`, and `rgba32f`).

## 2.2.1

- Automatically refresh an existing project-local .shadertoy/shadertoy.schema.json when the installed CLI embeds a newer schema, while leaving projects without a local schema untouched.
- Fixed fragment/image/buffer/cubemap SSBO binding and shader-storage barriers so named storage is actually shared across fragment and compute passes and remains visible across frames.
- Reject compute local workgroup Z sizes other than 1 because the public compute entrypoint is two-dimensional (`mainCompute(ivec2)`) and larger Z sizes duplicated writes/atomics to the same pixel.
- Harden raw STTF loading/compilation against duplicate SSBO binding points and document SSBO pass-ordering and state-capture semantics.
- State format 3 records each persistent pass's render-target format and rejects incompatible typed-buffer restores instead of silently dropping or reinterpreting channels; state formats 1 and 2 remain readable.

## 2.2.0

- Added first-class OpenGL 4.3 compute passes with fixed dispatch dimensions, configurable local workgroups, writable typed image outputs, and repeated per-frame dispatch through iIteration.
- Added persistent zero-initialized named shader-storage buffers shared across passes, enabling structured GPU state and GLSL atomic operations without packing data into texture channels.
- Added explicit r32f, rg32f, rgba16f, and rgba32f formats for 2D buffer/compute outputs; inspection, state capture, regression tests, preview, and VRAM profiling understand compute outputs and their actual storage cost.
- Added up to eight render targets per buffer/compute pass, with fragment location outputs, compute iOutput1..iOutput7 images, and per-channel output selection for downstream passes.
- Kept ordinary shader compatibility by preferring OpenGL 4.3 contexts and falling back to OpenGL 4.1 when advanced compute/SSBO features are not used.

## 2.1.8

- Synchronized CLI help, repository/crate READMEs, built-in documentation references, and generated-project guidance with the current import, fixed-buffer/sampler, inspection/profiling, regression-test, and record/replay workflows.

## 2.1.7

- Added RGBA32F buffer inspection with per-channel statistics, NaN/Inf detection, pixel probing, and diagnostic visualizations.
- Added per-pass GPU timer-query profiling, CPU render-call timing, and persistent-buffer VRAM estimates.
- Added project-local GLSL include support with dependency-aware transactional hot reload that preserves feedback buffers.
- Added deterministic manifest-driven visual/numeric regression tests with baseline updates and diff artifacts.
- Added preview input recording and deterministic replay with exact ShaderToy timing state, SHA-256 project fingerprints, and invalidation when project files change.

## 2.1.6

- Added fixed per-buffer render resolutions for stable simulation/FFT grids while preserving output-sized behavior by default.
- Added per-channel sampler controls for nearest/linear/mipmap filtering and clamp/repeat wrapping, including independent sampler state when the same source is bound multiple times.
- Hardened fixed-buffer state restore, mixed-resolution channel reporting, native dimension validation, and raw shader channel-slot validation.

## 2.1.5

- Fixed the Windows release triplet selection: the static triplet must be passed via `VCPKG_DEFAULT_TRIPLET` (the env var the vcpkg tool honors in manifest mode), not `VCPKG_TARGET_TRIPLET` (only read as a CMake variable, so 2.1.3/2.1.4 silently built the dynamic triplet again).
- Installed EGL runtime (`libegl1`, Mesa DRI drivers) on Linux CI so headless render tests can create a context.

## 2.1.4

- Fixed the Windows release link (`LNK2019` on `__imp_*` CRT symbols): when Rust links the static CRT, the native code is now compiled `/MT` too (`CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded` in `shadertoy-sys`).
- Fixed `clippy -D warnings` on current stable (`chunks_exact_to_as_chunks` in `state.rs`).

## 2.1.3

- Fixed the Windows CLI dying on startup with a missing-DLL error (`STATUS_DLL_NOT_FOUND`): the release build now links vcpkg ports (notably `glfw3`) and the CRT statically (`x64-windows-static`, `+crt-static`), so the shipped `shadertoy.exe` needs no sidecar DLLs or VC redist.

## 2.1.2

- Fixed `cargo binstall` on Windows: use an exact-target override (`x86_64-pc-windows-msvc`, `zip`) instead of a `cfg(target_os = "windows")` override, which `cargo-binstall` versions before ~1.17 silently ignore (they then probe for a nonexistent `.tgz` instead of the shipped `.zip`).

## 2.1.1

- Fixed heap corruption when rendering or previewing RGB output whose width is not 4-byte aligned.
- Hardened related OpenGL pixel-transfer paths, including tightly packed single-channel volumes.
- Made the live WebSocket/canvas preview responsive and touch-friendly on mobile.

## 2.1.0

- Made Linux offscreen rendering display-less (surfaceless EGL) so headless rendering needs no X/Wayland display.
- Added deterministic multi-frame rendering with frame streaming.
- Streamed live preview frames over the WebSocket canvas.
- Added Camoufox-backed ShaderToy import for materializing remote shaders into local projects.

## 2.0.1

- Hardened CLI project mutations and manifest paths against traversal, symlink escapes, reserved input-name collisions, and partial failed writes.
- Fixed live preview lifecycle so GLFW/OpenGL context creation, rendering, and destruction remain on the process main thread; improved auth URL handling and shutdown behavior.
- Made JSON mode cover command-line parse failures and added project-path option aliases for check and build.
- Hardened state, STTF, C ABI, and safe Rust buffer/dimension handling against malformed sizes, overflow, unsupported volume layouts, and allocation failures.
- Expanded CLI/native regression coverage for filesystem safety, malformed state/STTF data, and invalid C API dimensions.

## 2.0.0

- Breaking release relative to the previous 1.x C++ line: separated the reusable renderer/library from the desktop editor and made the native API suitable for external consumers.
- Added the agent-first `shadertoy` CLI, safe Rust wrapper, and generated sys bindings.
- Added schema-backed directory projects, deterministic rendering, resumable buffer state, and native live preview.
- Added static native linkage for the CLI and crates.io/cargo-binstall release infrastructure.
