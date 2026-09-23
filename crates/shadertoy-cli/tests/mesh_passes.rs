use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "shadertoy-cli-mesh-pass-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temporary test directory");
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn shadertoy(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_shadertoy"))
        .args(args)
        .output()
        .expect("run shadertoy")
}

fn write_mesh_project(root: &Path) {
    std::fs::create_dir_all(root.join("shaders")).expect("create shaders directory");
    std::fs::write(
        root.join("ShaderToy.toml"),
        r#"format = 1

[project]
name = "mesh-pass-cli"

[render]
width = 8
height = 8
fps = 60.0
preview_time = 0.0

[[uniform]]
name = "u_vertex_check"
type = "float"
default = 0.25

[[pass]]
name = "source"
kind = "buffer"
source = "shaders/source.frag"
width = 4
height = 4

[[pass]]
name = "mesh"
kind = "mesh"
vertex = "shaders/mesh.vert"
source = "shaders/mesh.frag"
format = "rgba16f"

[pass.mesh]
topology = "grid"
size = [3, 3]
instances = 2
depth = true
cull = "none"
clear = [0.0, 0.0, 0.0, 0.0]

[[pass.input]]
channel = 0
source = "source"
kind = "pass"
filter = "mipmap"
wrap = "clamp"

[[pass.storage]]
binding = 0
name = "vertex-count"
size = 4

[[pass]]
name = "image"
kind = "image"
source = "shaders/image.frag"

[[pass.input]]
channel = 0
source = "mesh"
kind = "pass"
filter = "nearest"
wrap = "clamp"

[[pass.input]]
channel = 1
source = "mesh"
kind = "pass"
output = "depth"
filter = "nearest"
wrap = "clamp"
"#,
    )
    .expect("write manifest");

    std::fs::write(
        root.join("shaders/source.frag"),
        r#"void mainImage(out vec4 color, in vec2 fragCoord) {
    float value = iFrame == 0 ? 0.25 : 0.75;
    color = vec4(value, 0.0, 0.0, 1.0);
}
"#,
    )
    .expect("write source shader");

    std::fs::write(
        root.join("shaders/mesh.vert"),
        r#"layout(std430, binding = 0) buffer VertexCount { uint count; };

out float vMipValue;
out float vContract;

void main() {
    atomicAdd(count, 1u);

    vMipValue = textureLod(iChannel0, vec2(0.5), 2.0).r;
    vContract =
        iMeshSize == ivec2(3, 3) &&
        iMeshInstances == 2 &&
        iFrame == 1 &&
        iChannelResolution[0].xy == vec2(4.0, 4.0) &&
        abs(u_vertex_check - 0.75) < 0.001
            ? 1.0
            : 0.0;

    vec2 p = a_grid * 2.0 - 1.0;
    p.x = p.x * 0.42 + (gl_InstanceID == 0 ? -0.48 : 0.48);
    p.y *= 0.8;
    gl_Position = vec4(p, 0.0, 1.0);
}
"#,
    )
    .expect("write mesh vertex shader");

    std::fs::write(
        root.join("shaders/mesh.frag"),
        r#"in float vMipValue;
in float vContract;

void mainImage(out vec4 color, in vec2 fragCoord) {
    color = vec4(vMipValue, vContract, 0.0, 1.0);
}
"#,
    )
    .expect("write mesh fragment shader");

    std::fs::write(
        root.join("shaders/image.frag"),
        r#"void mainImage(out vec4 color, in vec2 fragCoord) {
    vec2 probe = vec2(0.25, 0.5);
    vec4 mesh = texture(iChannel0, probe);
    float depth = texture(iChannel1, probe).r;
    color = vec4(mesh.r, mesh.g, depth, 1.0);
}
"#,
    )
    .expect("write final image shader");
}

#[cfg(target_os = "linux")]
#[test]
fn mesh_pass_supports_vertex_sampling_depth_storage_and_wireframe() {
    let temp = TempRoot::new();
    let project = temp.0.join("project");
    write_mesh_project(&project);
    let project_arg = project.to_string_lossy().into_owned();

    let checked = shadertoy(&["--json", "check", "--project", &project_arg]);
    assert!(checked.status.success(), "{checked:?}");
    let check_report: serde_json::Value =
        serde_json::from_slice(&checked.stdout).expect("parse check report");
    let mesh = &check_report["mesh_passes"][0];
    assert_eq!(mesh["name"], "mesh");
    assert_eq!(mesh["grid_size"], serde_json::json!([3, 3]));
    assert_eq!(mesh["instances"], 2);
    assert_eq!(mesh["vertices"], 18);
    assert_eq!(mesh["triangles"], 16);

    // Frame 1 forces a source-pass rewrite after frame 0. The vertex stage
    // explicitly samples mip 2, so 0.75 proves pass-output mips were rebuilt
    // after the current frame's source write rather than left stale at 0.25.
    let output = temp.0.join("mesh.png");
    let output_arg = output.to_string_lossy().into_owned();
    let rendered = shadertoy(&[
        "render",
        "--project",
        &project_arg,
        "--frame",
        "1",
        "--set",
        "u_vertex_check=0.75",
        "-o",
        &output_arg,
    ]);
    assert!(rendered.status.success(), "{rendered:?}");

    let image = image::open(&output).expect("open mesh render").to_rgb8();
    let pixel = image.get_pixel(0, 0).0;
    assert!((i16::from(pixel[0]) - 191).abs() <= 2, "{pixel:?}");
    assert!(pixel[1] >= 252, "{pixel:?}");
    assert!((i16::from(pixel[2]) - 128).abs() <= 2, "{pixel:?}");

    let inspected = shadertoy(&[
        "--json",
        "inspect",
        "--project",
        &project_arg,
        "storage",
        "vertex-count",
        "--frame",
        "1",
        "--type",
        "u32",
        "--count",
        "1",
    ]);
    assert!(inspected.status.success(), "{inspected:?}");
    let storage: serde_json::Value =
        serde_json::from_slice(&inspected.stdout).expect("parse storage report");
    // 3*3*2 vertices are run once at frame 0 and once at frame 1.
    assert_eq!(storage["values"][0], 36);

    let wireframe = temp.0.join("wireframe.png");
    let wireframe_arg = wireframe.to_string_lossy().into_owned();
    let wire = shadertoy(&[
        "render",
        "--project",
        &project_arg,
        "--frame",
        "1",
        "--set",
        "u_vertex_check=0.75",
        "--wireframe",
        "mesh",
        "-o",
        &wireframe_arg,
    ]);
    assert!(wire.status.success(), "{wire:?}");
    let wire_image = image::open(&wireframe)
        .expect("open wireframe render")
        .to_rgb8();
    assert!(
        wire_image
            .pixels()
            .any(|pixel| pixel[0] >= 250 && pixel[1] <= 5 && pixel[2] >= 250),
        "wireframe was not composited over the final image"
    );

    let mesh_only = temp.0.join("mesh-only.png");
    let mesh_only_arg = mesh_only.to_string_lossy().into_owned();
    let rendered_mesh = shadertoy(&[
        "render",
        "--project",
        &project_arg,
        "--frame",
        "1",
        "--set",
        "u_vertex_check=0.75",
        "--pass",
        "mesh",
        "-o",
        &mesh_only_arg,
    ]);
    assert!(rendered_mesh.status.success(), "{rendered_mesh:?}");
    assert!(mesh_only.is_file());

    let rewired = shadertoy(&[
        "channel",
        "--project",
        &project_arg,
        "set",
        "image",
        "1",
        "mesh",
        "--output",
        "depth",
        "--filter",
        "nearest",
        "--wrap",
        "clamp",
    ]);
    assert!(rewired.status.success(), "{rewired:?}");
    let manifest = std::fs::read_to_string(project.join("ShaderToy.toml")).expect("read manifest");
    assert!(manifest.contains("output = \"depth\""), "{manifest}");
}

#[test]
fn pass_add_mesh_creates_fragment_vertex_and_grid_config() {
    let temp = TempRoot::new();
    let project = temp.0.join("generated-project");
    let project_arg = project.to_string_lossy().into_owned();
    let created = shadertoy(&["new", &project_arg]);
    assert!(created.status.success(), "{created:?}");

    let added = shadertoy(&[
        "pass",
        "--project",
        &project_arg,
        "add",
        "surface",
        "--kind",
        "mesh",
    ]);
    assert!(added.status.success(), "{added:?}");
    assert!(project.join("shaders/surface.frag").is_file());
    assert!(project.join("shaders/surface.vert").is_file());

    let manifest = std::fs::read_to_string(project.join("ShaderToy.toml")).expect("read manifest");
    assert!(manifest.contains("kind = \"mesh\""), "{manifest}");
    assert!(
        manifest.contains("vertex = \"shaders/surface.vert\""),
        "{manifest}"
    );
    assert!(manifest.contains("[pass.mesh]"), "{manifest}");
    assert!(manifest.contains("    33,"), "{manifest}");
    assert!(manifest.contains("depth = true"), "{manifest}");

    let checked = shadertoy(&["check", "--project", &project_arg]);
    assert!(checked.status.success(), "{checked:?}");
}
