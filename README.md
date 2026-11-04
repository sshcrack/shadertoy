# shadertoy

[![build-windows](https://github.com/sshcrack/shadertoy/actions/workflows/build-windows.yml/badge.svg)](https://github.com/sshcrack/shadertoy/actions/workflows/build-windows.yml)
[![build-linux](https://github.com/sshcrack/shadertoy/actions/workflows/build-linux.yml/badge.svg)](https://github.com/sshcrack/shadertoy/actions/workflows/build-linux.yml)
[![build-macos](https://github.com/sshcrack/shadertoy/actions/workflows/build-macos.yml/badge.svg)](https://github.com/sshcrack/shadertoy/actions/workflows/build-macos.yml)

An unofficial ShaderToy renderer and live editor.

The repository has several frontends built on one rendering implementation:

- **shadertoy::shadertoy** — the standalone C++23 renderer/library.
- **shadertoy-c** — a deliberately small stable C ABI over project construction and runtime rendering.
- **Rust shadertoy-sys / shadertoy-native crates** — generated raw bindings plus a safe, idiomatic wrapper.
- **shadertoy CLI** — the agent-first project/check/render/debug/live-preview workflow.
- **desktop editor** — the existing ImGui editor. It remains a C++ library client; direct directory-project editing will be adapted later.

The core C++ library does not depend on ImGui, HelloImGui, the node editor, GLFW, or native file dialogs. The embedding application owns the OpenGL context and makes it current. The optional C ABI includes an offscreen context helper for command-line hosts. On Linux it creates a surfaceless EGL context directly, so CLI compilation/rendering does not require an X11/Wayland display.

## Architecture

~~~
 ShaderToy.toml + GLSL/assets
             |
             v
      project semantics
             |
             v
      ShaderDocument
             |
             v
       C++ Runtime  -----------------> desktop GUI
             |
             v
       stable C ABI
             |
       +-----+------+
       |            |
 shadertoy-sys   other FFI
       |
 safe Rust shadertoy-native (crate name: shadertoy)
       |
 shadertoy CLI
  check / render / inspect / state / preview
~~~

ShaderDocument remains GUI-independent. Directory-project graph semantics live below the frontends, while the Rust CLI owns TOML/schema/file-watching/web-server concerns. shadertoy-sys generates its bindings directly from shadertoy-c/shadertoy.h; CLI code never handles raw C pointers.

## Using the library

For Rust, the safe wrapper is published as `shadertoy-native` while its library target remains `shadertoy`:

~~~bash
cargo add shadertoy-native
~~~

~~~rust
use shadertoy::Runtime;
~~~

For C++, after installing the project:

~~~cmake
find_package(shadertoy CONFIG REQUIRED)
target_link_libraries(my_app PRIVATE shadertoy::shadertoy)
~~~

Create the graphics context in your application and make it current before compiling or rendering a document:

~~~cpp
#include <shadertoy/ShaderToy.hpp>

ShaderToy::Runtime runtime;

// A compatible OpenGL context must be current here.
auto loaded = runtime.loadImageShader(
    "example",
    R"(
        void mainImage(out vec4 color, in vec2 fragCoord) {
            vec2 uv = fragCoord / iResolution.xy;
            color = vec4(uv, 0.5 + 0.5 * sin(iTime), 1.0);
        }
    )"
);

if (!loaded)
    throw loaded.error();

runtime.tickFixed(1.0f / 60.0f, 60.0f);
auto rgb = runtime.renderToBuffer({128.0f, 128.0f});
~~~

The host may instead construct/load a ShaderDocument, import a ShaderToy URL/response, or load an STTF file and pass the resulting document to Runtime::setDocument().

Runtime can be constructed before an OpenGL context exists. A current context is required when a document is compiled and whenever it is rendered. The library initializes its own OpenGL loader; the host does not need to use GLEW directly.

## Agent-first CLI

Install the prebuilt CLI with cargo-binstall:

~~~bash
cargo binstall shadertoy-cli
shadertoy --version
~~~

The binary package is `shadertoy-cli`; the installed executable is `shadertoy`. Source installation with `cargo install shadertoy-cli` is also supported when the required native build dependencies are available, but cargo-binstall is the preferred fast path.

A CLI project is an editable directory rather than a monolithic STTF file:

~~~text
my-shader/
├── ShaderToy.toml
├── README.md
├── shaders/
│   ├── image.frag
│   └── buffer-a.frag
├── assets/
└── target/          # generated, gitignored
~~~

Create a minimal project, a working feedback-buffer example, or import a public ShaderToy:

~~~bash
shadertoy new hello
shadertoy new feedback --template multipass
shadertoy import https://www.shadertoy.com/view/lsX3W4 -o mandelbrot
~~~

ShaderToy import uses a Camoufox browser session rather than direct HTTP, then materializes supported shader passes, pass/feedback wiring, sampler settings, textures, cubemaps, and volumes into the normal editable directory format. Imported projects retain source metadata plus the exact browser response in .shadertoy/import-response.json. On first use the CLI creates a private cached Python environment for its pinned Camoufox adapter and fetches the matching browser; Python 3.10+ is required.

The multipass template intentionally demonstrates Buffer A previous-frame feedback and wiring the current Buffer A result into Image through iChannel0.

Buffer passes can also use a fixed render target independent of the output size, which is useful for FFT/simulation grids:

~~~toml
[[pass]]
name = "spectrum"
kind = "buffer"
source = "shaders/spectrum.frag"
width = 256
height = 256
~~~

Omitting width/height preserves the existing output-sized behavior. Fixed buffers report their own dimensions through iResolution, consumers see the real input dimensions through iChannelResolution, and their feedback survives preview/output resolution changes.

For heavier GPU work, compute passes provide a much larger pipeline surface for FFTs, particles, fluids, voxel work, and other simulations:

~~~toml
[[pass]]
name = "simulation"
kind = "compute"
source = "shaders/simulation.comp"
width = 256
height = 256
format = "rg32f"
local_size = [8, 8, 1]
iterations = 4

[[pass.storage]]
binding = 3
name = "particle-state"
size = 4194304
~~~

Compute sources implement mainCompute(ivec2 coord). The host supplies a writable image2D named iOutput plus the normal timing/input uniforms, iIteration, and configured iChannel samplers. Local workgroups are two-dimensional, so local_size Z must be 1. Available target formats are r32f, rg32f, rgba16f, and rgba32f. Named SSBOs are persistent, zero-initialized, and shared across passes by name, so OpenGL 4.3 image load/store and atomics are available without packing structured state into RGBA textures. Ordinary shaders still run on the OpenGL 4.1 compatibility path when 4.3 is unavailable.

Buffer and compute passes can also expose up to eight render targets with extra_outputs. Fragment shaders write locations 1..N with normal GLSL layout(location = N) outputs; compute shaders receive iOutput1, iOutput2, and so on. Consumers select a target with output = N on the pass input.

Mesh passes add a user-authored vertex stage while preserving the same pass graph, channel, uniform, SSBO, format, and MRT model. The CLI generates an indexed 0..1 grid and draws it with instancing; the shader owns `gl_Position`, camera math, displacement, and LOD policy. Depth-enabled meshes expose a sampleable named `output = "depth"`, and `render`, `render-frames`, and `preview` accept `--wireframe PASS` for depth-tested geometry review. See [docs/MESH_PASSES.md](docs/MESH_PASSES.md) for the complete contract.

Inputs also expose independent sampler controls. Numerical grids such as FFT stages normally use exact texel sampling:

~~~toml
[[pass.input]]
channel = 0
source = "spectrum"
filter = "nearest" # nearest | linear | mipmap
wrap = "repeat"    # repeat | clamp
~~~

Sampler state is per iChannel, so the same source can be bound more than once with different interpolation or wrap behavior. Omitting the sampler fields preserves the existing linear/repeat defaults. The equivalent mutation command is `shadertoy channel set image 0 spectrum --filter nearest --wrap repeat`.

Project-level `[[uniform]]` declarations provide typed float/int/bool/vector
parameters with defaults and optional ranges. Deterministic commands accept
`--set name=value`, preview exposes live controls, and regression cases can
override declared values per test.

Named manifest quality presets keep one shader/asset tree while overriding render
scale and expensive pass settings. For example, `[preset.low] render_scale = 0.75`
and `[preset.low.pass.foam]` can set a smaller fixed `width`/`height`; compute
presets may also override `iterations` and `local_size`. Use the same tier across
commands with `--preset low` (`check`, `preview`, `render`, `render-frames`,
`render-video`, `sweep`, `profile`, and `build`). Presets never change the project
root, so includes and assets keep resolving exactly as they do in the base
manifest.

For human/performance review, lowering the final Image resolution is deliberately
not treated as an optimization. Preview keeps the base project output size while
switching presets unless the human explicitly selects another review resolution.
Likewise, `profile --preset NAME` benchmarks at the base project output size by
default and reports any preset-requested final scale separately; internal
pass-size, iteration, algorithm, and shader-work reductions still count normally.

Quality tiers can be blinded directly with
`shadertoy blind create 'project:.@preset=high' 'project:.@preset=low' ...`.

ShaderToy Sound passes are compiled by `shadertoy check` and can be rendered
offline with `shadertoy render-audio ... -o target/sound.wav`. Local `video`
assets are decoded deterministically with ffmpeg/ffprobe and expose
`iChannelTime`.

The common agent loop is deliberately small:

~~~bash
cd feedback
shadertoy inspect --json
shadertoy check --json
shadertoy check --pedantic --json
shadertoy graph --dot target/graph.dot --json
shadertoy render -o target/check.png
shadertoy render-frames --range 0:180:60 --contact-sheet target/contact.png
shadertoy render-video --frames 180 -o target/clip.mp4
shadertoy sweep --frame 120 --set foam_gain=0.8,1.0,1.2
shadertoy sweep --blind --frame 120 --set foam_gain=0.8,1.0,1.2
shadertoy blind create target/old-renders target/new-renders --output-dir target/old-vs-new
shadertoy blind create 'project:.@preset=high' 'project:.@preset=medium' 'project:.@preset=low' --frames 60,180,300 --set u_storm=1
shadertoy blind judge target/old-vs-new/blind-session.json --pick B --reason "preferred breakup"
shadertoy blind reveal target/old-vs-new/blind-session.json
shadertoy experiment --baseline git:main --candidate git:HEAD --frames 0,60,120 --set u_storm=1 --blind
shadertoy test --ci
shadertoy trace capture --frame 120 --include-intermediates -o target/bug.sttrace
shadertoy trace inspect target/bug.sttrace --json
shadertoy trace replay target/bug.sttrace --json
shadertoy preview
~~~

When something is wrong, the same interface drills down instead of requiring renderer internals:

~~~bash
shadertoy inspect graph --json
shadertoy inspect pass buffer-a --json
shadertoy inspect channels image --json
shadertoy inspect buffer buffer-a --frame 120 --pixel 8,8 --json
shadertoy inspect buffer gbuffer --output-index 1 --raw target/gbuffer1.rgba32f
shadertoy inspect storage particle-state --frame 120 --type f32 --count 16 --json
shadertoy render --pass buffer-a -o target/buffer-a.png
shadertoy profile --frame 120 --samples 30 --json
shadertoy profile --preset medium --frame 120 --samples 30 --sync-per-pass --json
~~~

`profile` reports raw per-pass GPU execution timestamps separately from CPU completion waits and the legacy post-completion attributed interval. Samples whose timer query clearly outruns asynchronous work are marked invalid instead of being presented as precise shader cost; the JSON retains every raw sample plus MAD-based outlier flags. `--discard-outliers` excludes flagged samples from aggregate statistics while retaining the raw data. An independent non-intrusive frame-level GPU timestamp interval is reported alongside the attributed pass sum and completion-wait total; use that frame series as the primary cross-run optimization signal and the per-pass validity/wait data for localization. When a quality preset is profiled, the benchmark output stays at the base project resolution by default, so a smaller final frame cannot be presented as a performance win. `profile --sync-per-pass` additionally retires each attribution timestamp before the next pass for maximum-isolation diagnostics. `sweep` renders the Cartesian product of repeated `--set NAME=VALUES` dimensions, writes deterministic variant PNGs, and creates a contact sheet by default. Scalar alternatives are comma separated; vector alternatives use semicolons because vector components already use commas. Blind sweep mode randomizes parameter variants, while `blind create` does the same for arbitrary images/render directories, ShaderToy projects, STTF builds, Git revisions, or named quality presets such as `project:.@preset=medium`. `blind create --set NAME=VALUE` applies the same custom-uniform override to every rendered source, so states such as `u_storm=1` can be judged fairly without editing manifests. Both keep the real mapping out of public session metadata until `blind judge` records a choice and rationale, after which `blind reveal` writes the combined report.

`experiment` lifts that workflow to reproducible baseline/candidate or N-way comparisons: it renders deterministic frame sets from the same arbitrary source forms, writes a contact sheet and JSON report, computes normalized RMSE and windowed SSIM, records source provenance, and profiles project/git sources. Its repeatable `--set NAME=VALUE` uses the same overrides for rendering and profiling. `experiment --blind` reuses the sealed blind-judgment lifecycle. `graph` exposes the resolved resource/pass graph and Graphviz DOT; `check --pedantic` turns advisory graph/resource diagnostics into CI failures.

shadertoy state captures lossless RGBA32F feedback-buffer state together with deterministic time/frame metadata. That makes multipass bugs resumable and lets an agent replace one buffer with a known exact-size image:

~~~bash
shadertoy state capture --frame 300 --include-storage --set storm=1.0 -o target/frame300.ststate
shadertoy state inspect target/frame300.ststate --json

shadertoy render   --state target/frame300.ststate   --set-buffer buffer-a=fixtures/known.png   -o target/debug.png
~~~

Deterministic regression cases live in `ShaderToy.toml` as `[[test]]` entries. `shadertoy test` runs visual PNG comparisons and numeric buffer assertions; matrix cases can cover multiple frames/resolutions, repeat fresh runs with `assert_deterministic`, and verify fixed simulation grids with `assert_resolution_independent`. Tests can also compare two custom-uniform configurations with RMSE bounds, enforce total/per-pass GPU budgets, compare exact SSBO fixtures, and exercise `.ststate` serialization/restoration with `assert_state_roundtrip`. `shadertoy test --ci` is non-mutating; `shadertoy test --update` deliberately rewrites visual baselines.

For deterministic renderer bugs, `shadertoy trace capture` creates a self-contained `.sttrace` directory containing STTF, final image, persistent buffer/SSBO state, timings, hashes, graph/synchronization metadata, and optional per-pass readbacks. `trace inspect` verifies artifact hashes and `trace replay` reproduces the frame from the bundled STTF without the original checkout. For input-sensitive live-preview bugs, `shadertoy preview --record target/repro.strec` records shader-affecting controls and exact timing markers, and `shadertoy replay target/repro.strec -o target/replayed.png` reproduces the captured timeline headlessly.

Use shadertoy docs agent for the concise workflow embedded in the executable. Other topics include project, import, manifest, passes, glsl, assets, buffers, channels, state, sweep, blind, experiment, profile, test, trace, graph, and preview. `shadertoy docs glsl` documents the generated shader prelude and entry-point contract; `shadertoy docs assets` documents on-disk cubemap and volume formats.

### Schema-backed ShaderToy.toml

The canonical JSON Schema is checked in at crates/shadertoy-cli/assets/schema/shadertoy.schema.json and generated from the same Rust types that parse the manifest. A test prevents the checked-in schema from drifting from those types.

New projects receive a local .shadertoy/shadertoy.schema.json, a #:schema directive in ShaderToy.toml, and a .taplo.toml association. Editors with Taplo / compatible TOML schema support can therefore validate keys and types and provide completion. Whenever a project with that generated local schema is loaded, the CLI compares it with the schema embedded in the installed CLI and automatically refreshes the file if it is stale. Projects without a local schema are left unchanged. Agents can print the exact same schema with:

~~~bash
shadertoy docs manifest --schema
~~~

shadertoy check adds semantic validation that JSON Schema cannot express, including graph references/cycles and real GLSL compilation through the native renderer. `shadertoy check --pedantic` additionally fails on advisory resource/graph diagnostics such as unreachable passes, unused assets/uniforms, viewport-sized feedback, and unordered shared SSBO users.

### Live native preview

shadertoy preview starts a local web server, but the browser is only a viewer/controller: rendering remains in the native C++ renderer. Rendered PNG frames are pushed over the existing WebSocket and drawn into a canvas, so pass selection and controls do not fight a continuously refreshed HTTP image. It watches the manifest, shader sources, and assets, keeps the last successful frame when a new edit fails compilation, and hot-reloads automatically after the error is fixed.

The preview is designed as the human review surface. It exposes a quality-preset selector when the manifest declares presets, but preset changes keep the final Image at the base project output resolution so `render_scale` cannot fake a performance improvement. The reviewer can choose common output sizes or enter a custom width/height, and that explicit review resolution remains fixed while switching presets. The UI also exposes final Image and named 2D buffers, pause/resume, reset, frame step, time scale, mouse, keyboard, and declared custom-uniform controls. File-backed video channels advance from shader time. Webcam channels use browser camera permission and feed the native renderer over the WebSocket; they are intentionally unavailable to headless render/replay and preview recording. It binds to 127.0.0.1 by default; non-loopback binds require --token.

ShaderToy-page import is available through shadertoy import URL_OR_ID. It is isolated behind the Camoufox browser adapter; normal project checking, rendering, building, and previewing remain browser-independent.

## Interactive editor

The desktop application keeps the original editing workflow on top of the library:

- visual render-pass/pipeline editor;
- GLSL source editor;
- ShaderToy URL import;
- STTF open/save;
- live canvas with mouse and keyboard channels;
- pause/reset/time-scale controls;
- screenshots and application logging.

Run it with an optional STTF file or ShaderToy URL:

~~~bash
./build/app/shadertoy [shader.sttf|https://www.shadertoy.com/view/...]
~~~

## Host audio / music visualization

Embedders can feed live music analysis through Runtime::setAudioInput() (C++), st_runtime_set_audio_input() (C), or Runtime::set_audio_input() (Rust).

The CLI drives the same inputs with deterministic silence (default), synthetic beat-driven analysis, or frame-accurate analysis of an audio file, configured by an `[audio]` manifest section, per-test `audio`, or the global `--audio none|synthetic|FILE` flags. See `shadertoy docs audio`:

~~~bash
shadertoy render-frames --range 0:600:30 --audio song.mp3 --audio-offset 45
shadertoy render --frame 120 --audio synthetic --audio-profile bass --audio-bpm 128
shadertoy audio --audio song.mp3 --time 45.5 --json   # inspect the analysis
~~~

Shaders that declare a ShaderToy music, musicstream, mic, or audio input receive the conventional 512x2 audio texture in their assigned iChannelN:

- row 0: normalized spectrum, low frequencies on the left;
- row 1: waveform encoded as 0..1 (sampleAudioWaveform() returns -1..1).

The renderer also exposes semantic aliases including iAudioLoudness, iAudioBass, iAudioMid, iAudioTreble, iAudioKick, iAudioSnare, iAudioHihat, iAudioOnset, iAudioBpm, iAudioBeatPhase, iAudioBeatConfidence, iAudioBeatStrength, iAudioDrop, and iAudioSectionChange. iAudioAvailable is 1 when the host supplied a current frame.

A minimal audio-reactive shader can use both the ShaderToy-style audio texture and semantic uniforms:

~~~glsl
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float spectrum = sampleAudioSpectrum(iChannel0, clamp(abs(uv.x), 0.0, 1.0));
    float glow = exp(-10.0 * abs(length(uv) - (0.25 + 0.12 * iAudioBass)));
    vec3 color = (0.35 + 0.65 * cos(iTime + vec3(0.0, 2.0, 4.0)))
               * (glow + spectrum * 0.6);
    color *= 0.7 + 0.45 * iAudioBeatStrength;
    fragColor = vec4(color, 1.0);
}
~~~

## Features

Render passes:

- [x] Image
- [x] Cubemap
- [x] Buffer
- [x] Common
- [ ] Sound

Channels:

- [x] Textures
- [x] Volumes
- [x] Cubemaps
- [x] Buffer / previous frame
- [x] Keyboard
- [x] Music and microphone (host-provided)
- [ ] Video
- [ ] Webcam

Editor/runtime utilities:

- [x] Import from shadertoy.com
- [x] Import directly from a ShaderToy response body
- [x] Render pass editor
- [x] GLSL shader editor
- [x] STTF import/export
- [x] Off-screen RGB rendering
- [x] Deterministic fixed-step rendering
- [x] Screenshots
- [ ] Video recording
- [ ] Custom uniforms
- [ ] Custom meshes
- [ ] Anti-aliasing controls

## Build

### Prerequisites

- Windows, Linux, or macOS
- CMake 3.20+
- a C++23 compiler
- [vcpkg](https://github.com/microsoft/vcpkg)
- OpenGL
- for the Rust CLI: stable Rust with Edition 2024 support plus Clang/libclang for bindgen

Clone with the text-editor submodule:

~~~bash
git clone --recursive https://github.com/sshcrack/shadertoy.git
cd shadertoy
~~~

Set VCPKG_ROOT, then use one of the checked-in presets:

~~~bash
# GUI + library
cmake --preset default
cmake --build --preset default

# Standalone library only: no GUI dependencies are selected
cmake --preset library
cmake --build --preset library
ctest --preset library

# Library + legacy deterministic preview fixture
cmake --preset preview
cmake --build --preset preview

# Library + stable C ABI
cmake --preset c-api
cmake --build --preset c-api

# Agent-first Rust CLI. shadertoy-sys builds the C ABI and generates bindings.
cargo build -p shadertoy-cli
./target/debug/shadertoy --help
~~~

The Rust CLI statically links the ShaderToy C ABI, C++ renderer, GLAD loader, and vcpkg-provided native dependencies into the executable. On Linux the resulting binary has no link-time X11, GLX, OpenGL, GLFW, or EGL dependency; the display-less helper loads `libEGL.so.1` at runtime and GLAD resolves the current OpenGL context. Normal C/C++ runtime libraries remain dynamic, and no adjacent `libshadertoy_c`/GLFW/GLEW/GLAD/fmt/OpenSSL/Brotli shared libraries are required.

The equivalent CMake switches are `SHADERTOY_BUILD_GUI`, `SHADERTOY_BUILD_PREVIEW_TOOL`, `SHADERTOY_BUILD_C_API`, `SHADERTOY_BUILD_C_API_STATIC`, and `BUILD_TESTING`.

### Laptop MCP

Current Laptop MCP managed images install the pinned `shadertoy-cli` release globally, so fresh sandboxes can invoke `shadertoy` without repository configuration or a source build. This repository therefore does not declare a ShaderToy-specific workspace bootstrap contract.

The generic `[image].cargo_binstall` and per-session `sandbox_config_update(cargo_binstall=[...])` mechanisms remain available for other exact-version prebuilt Rust tools.

### Install the standalone package

~~~bash
cmake --preset library -DCMAKE_INSTALL_PREFIX=/your/prefix
cmake --build --preset library
cmake --install build-lib
~~~

This installs the public headers, libshadertoy, and a CMake package exporting shadertoy::shadertoy.

## Deterministic preview tool

The optional shadertoy-preview executable creates a hidden GLFW context, advances fixed frame time, injects repeatable synthetic music analysis, renders through the same library pipeline, and writes a PNG:

~~~bash
./build-preview/shadertoy-preview examples/music_neon_orbit.frag preview.png 256 128 120
~~~

On Linux without a display server, run it under a virtual/headless display that provides OpenGL.

## Gallery

[![goo](https://user-images.githubusercontent.com/15650457/236786522-80c10c46-f3b0-46f3-88ef-abbe39c3cd5f.png)](https://www.shadertoy.com/view/lllBDM)

[![expansive reaction-diffusion](https://user-images.githubusercontent.com/15650457/236787527-b26fa835-1d36-4dc6-be59-6d508e898e04.png)](https://www.shadertoy.com/view/4dcGW2)

[![mandelbrot](https://user-images.githubusercontent.com/15650457/236788040-2411c757-7c51-407a-869f-5c6709bf5e5d.png)](https://www.shadertoy.com/view/lsX3W4)

[![MultiscaleMIPFluid](https://user-images.githubusercontent.com/15650457/236790106-5ebeb8a2-0c16-4cbd-a7cf-d8bbb21ad613.png)](https://www.shadertoy.com/view/tsKXR3)

[![RainbowSand](https://user-images.githubusercontent.com/15650457/236790355-c20303e1-7abd-4d42-9088-2133a0e756fa.png)](https://www.shadertoy.com/view/stdyRr)

[![subsurface](https://user-images.githubusercontent.com/15650457/236790664-3defcade-c5b4-4f9c-9f21-0a1b67b72536.png)](https://www.shadertoy.com/view/dltGWl)

[![noise-contour](https://user-images.githubusercontent.com/15650457/236791146-b3b9cdff-6754-42ae-83c3-d69ef2ea9387.png)](https://www.shadertoy.com/view/MscSzf)

[![cubemaps](https://github.com/dtcxzyw/shadertoy/assets/15650457/e1719ed0-2748-47e0-a7eb-db6d35cd8dec)](https://www.shadertoy.com/view/MsXGz4)

See [examples](examples) for more shaders.

## Releases

See the [Releases page](https://github.com/sshcrack/shadertoy/releases) for pre-built binaries.

## License

This repository is licensed under the Apache License 2.0. See [LICENSE](LICENSE) for details.
