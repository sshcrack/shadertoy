# Mesh passes

Mesh passes are rasterized 2D render passes with a user-authored vertex stage. They are intended for geometry experiments—clipmaps, displaced water surfaces, terrain grids, particle ribbons—without turning the CLI into a scene renderer.

## Manifest

```toml
[[pass]]
name = "ocean-mesh"
kind = "mesh"
vertex = "shaders/ocean-mesh.vert"
source = "shaders/ocean-mesh.frag"
format = "rgba16f"
extra_outputs = ["rg32f"] # optional; same MRT contract as buffer passes

[pass.mesh]
topology = "grid"
size = [257, 257]
instances = 6
depth = true
cull = "none"             # none | back | front
clear = [0.0, 0.0, 0.0, 0.0]

[[pass.input]]
channel = 0
source = "waves"
kind = "pass"
filter = "mipmap"
wrap = "repeat"
```

The generated grid contains `size.x * size.y` vertices per instance. `a_grid` is a `vec2` vertex attribute from 0..1 in both axes. Adjacent grid cells are emitted as indexed counter-clockwise triangles and the mesh is drawn with instancing.

The CLI deliberately supplies no camera, matrices, world coordinates, clipmap policy, or displacement logic. The vertex source owns `gl_Position`.

## Vertex-stage contract

The vertex stage receives:

- `layout(location = 0) in vec2 a_grid`
- GLSL built-ins such as `gl_VertexID` and `gl_InstanceID`
- `uniform ivec2 iMeshSize`
- `uniform int iMeshInstances`
- the normal timing, mouse, date, audio, `iResolution`, `iChannelResolution[]`, and `iChannelTime[]` uniforms
- every configured `iChannel0..15`
- declared custom uniforms, including command-line `--set` overrides
- configured `[[pass.storage]]` SSBO bindings

Quoted includes and generated custom-uniform declarations use the same source expansion and `#line` behavior as fragment stages. User-defined vertex `out` / fragment `in` varyings are linked unchanged.

Vertex texture sampling should use explicit-LOD operations such as `textureLod` or `texelFetch`. If a pass input uses `filter = "mipmap"`, the source texture's mip chain is regenerated when the consumer binds it, after the producing pass has written the current frame.

## Quality presets

Geometry density can vary independently from render-target resolution:

    [preset.low.pass.ocean-mesh]
    mesh_size = [129, 129]
    mesh_instances = 4

    [preset.ultra.pass.ocean-mesh]
    mesh_size = [257, 257]
    mesh_instances = 6

This is preferable to using a smaller pass width/height as a proxy for geometry quality: target resolution controls raster/output cost, while mesh_size and mesh_instances control vertex/triangle cost and clipmap coverage.

## Fragment stage and outputs

The fragment source uses the normal `mainImage(out vec4 color, in vec2 fragCoord)` contract. Color format and `extra_outputs` behave exactly like a buffer pass.

When `depth = true`, the mesh also owns a real depth attachment used with `GL_LESS`, clears it to 1.0, and writes depth normally. A downstream pass can sample a float copy of the resulting window-space depth using:

```toml
[[pass.input]]
channel = 3
source = "ocean-mesh"
kind = "pass"
output = "depth"
filter = "nearest"
wrap = "clamp"
```

The sampled depth is raw `gl_FragCoord.z` in the 0..1 window-depth range. Depth is a named output and does not consume a user-visible MRT index. A depth-enabled mesh can therefore expose up to seven color render targets plus its internal depth-copy target.

Previous-frame feedback from the named depth output is intentionally rejected. Normal color output 0 retains the existing feedback rules.

## Wireframe review

`render`, `render-frames`, and `preview` accept:

```bash
shadertoy render --wireframe ocean-mesh
shadertoy render-frames --wireframe ocean-mesh --range 0:120:30
shadertoy preview --wireframe ocean-mesh
```

The mesh is first rendered normally. It is then redrawn in polygon line mode into a transparent debug target while the mesh's own depth attachment is still bound, using LEQUAL and no depth writes. Hidden edges therefore never enter the overlay. After the normal pass graph finishes, that transparent wireframe target is blended over the final image so bloom, tonemapping, or other downstream passes cannot hide the grid. Wireframe requires depth = true.

Because wireframe is a real second draw, vertex-stage SSBO side effects also execute a second time while wireframe is enabled. Do not use wireframe during SSBO-count/spike assertions unless the shader accounts for that debug redraw.

## Inspection and profiling

`shadertoy check --json` reports each mesh pass's grid size, instance count, total vertex count, total triangle count, depth state, and cull mode.

`shadertoy profile --json` adds mesh geometry statistics beside the existing per-pass GPU timing data.

## Deliberate scope

The first mesh implementation supports generated indexed grids only. It does not import OBJ files, supply cameras, or implement clipmap rings in host code. Those policies belong in GLSL so experiments remain portable and directly comparable.

MSAA and mesh-configuration sweeps are natural follow-ups, but are not part of the initial mesh-pass contract. Color outputs remain normal sampleable textures, which keeps graph composition, named depth inputs, captures, and downstream passes simple.
