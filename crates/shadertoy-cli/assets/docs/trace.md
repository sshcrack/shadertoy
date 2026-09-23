Render traces
=============

Use shadertoy trace to freeze a difficult deterministic render-graph bug into a
self-contained .sttrace directory:

  shadertoy trace capture \
    --project . \
    --frame 120 \
    --set storm=1.0 \
    --include-intermediates \
    -o target/bug.sttrace

A trace contains:
- project.sttf with the compiled project and resolved custom-uniform values;
- final.png;
- state.ststate with persistent buffer and SSBO contents;
- trace.json with resolved manifest/configuration, source/asset hashes, graph
  diagnostics, dependency/synchronization metadata, and GPU pass timings;
- optional per-pass PNG and RGBA32F output snapshots, including
  `PASS-output-depth.*` for depth-enabled mesh passes.

Inspect and integrity-check every recorded artifact:

  shadertoy trace inspect target/bug.sttrace --json

Replay without the original project checkout:

  shadertoy trace replay target/bug.sttrace -o target/replayed.png --json

Replay loads only the bundled STTF and requires the deterministic RGB result to
match final.png bit-for-bit. Artifact sizes and SHA-256 hashes are verified
before inspection/replay.

Trace capture intentionally rejects live webcam input and currently rejects
file-backed video assets because those external time-varying inputs are not
embedded in the STTF replay bundle.
