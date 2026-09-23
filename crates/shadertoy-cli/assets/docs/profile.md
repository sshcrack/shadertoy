shadertoy profile measures GPU work while retaining the raw evidence needed to
detect driver/query artifacts instead of hiding them inside one per-pass number.

For optimization comparisons:

  shadertoy profile --preset ultra --frame 180 --warmup 8 --samples 30 --json

When a preset is selected, the final benchmark output stays at the base project's
render resolution by default. A smaller preset render_scale is reported but is not
counted as a performance optimization. Internal pass dimensions, compute
iterations/local size, algorithms, and shader work still apply normally.

Normal profiling is non-intrusive at pass boundaries: it does not glFinish after
every pass. The primary cross-run signal is gpu_frame_timestamp, an independent
start/end GPU timestamp interval across that natural profiled frame.

Mesh passes additionally report grid size, instance count, total vertices, and total triangles in JSON, so a quality-preset change in geometry density is visible alongside timing. The persistent GPU-state estimate includes mesh color targets, the sampleable depth copy, the depth attachment, and generated grid vertex/index buffers.

Each pass/sample also reports:

- gpu_execution_ms: GPU timestamp interval closed before any host completion wait.
- completion_wait_ms: CPU wall time blocked in the explicit completion wait.
- attributed_ms: compatibility start-to-post-completion timestamp interval.

In normal non-intrusive profiling completion_wait_ms is zero. Graphics-pass timer
intervals can still be useful, but asynchronous compute execution samples are
conservatively marked sample_valid=false because a graphics-queue timestamp cannot
prove that independent compute has completed.

For per-pass isolation diagnostics:

  shadertoy profile --frame 180 --samples 30 --sync-per-pass --json

--sync-per-pass explicitly completes each pass. That gives completion_wait_ms and
the legacy attributed interval, but the synchronization is intentionally intrusive.
To keep the optimization signal clean, gpu_frame_timestamp for a sync-per-pass
command is collected from a second deterministic runtime with no per-pass glFinish
boundaries. The JSON field gpu_frame_timestamp_source records which path produced it.

Some drivers let asynchronous compute outrun graphics timer timestamps even in
diagnostic mode. When gpu_execution_ms is implausibly small compared with the
observed completion wait, sample_valid is false. The raw numbers are still retained;
do not treat an invalid execution sample as shader cost.

Outliers are always MAD-flagged in sample_details. By default they remain in the
aggregate statistics for backward compatibility. To exclude flagged samples from
mean/median/p95 while keeping every raw sample in JSON:

  shadertoy profile --frame 180 --samples 30 --discard-outliers --json

For candidate comparisons, prefer a stable gpu_frame_timestamp series. Use
completion_wait_total and the per-pass validity/wait data to localize regressions;
the compatibility gpu_pass_total is diagnostic and may include synchronization or
driver scheduling effects in sync-per-pass mode.

A useful comparison should use the same project output resolution, frame/window,
warmup, sample count, preset, and custom-uniform values on an otherwise idle GPU.
