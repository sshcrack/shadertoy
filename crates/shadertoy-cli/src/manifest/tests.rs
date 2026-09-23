use super::*;

#[test]
fn checked_in_schema_matches_manifest_types() {
    let expected = crate::include_file!("schema/shadertoy.schema.json").trim_end();
    let generated = schema_json().expect("schema generation should succeed");
    assert_eq!(expected, generated);
}

#[test]
fn multipass_template_validates() {
    Manifest::multipass("feedback")
        .validate_structure()
        .expect("multipass template should remain valid");
}

#[test]
fn manifest_rejects_paths_outside_project() {
    for source in ["../outside.frag", "/tmp/outside.frag", r"C:\outside.frag"] {
        let mut manifest = Manifest::minimal("demo");
        manifest.passes[0].source = source.into();
        assert!(
            manifest.validate_structure().is_err(),
            "source should be rejected: {source}"
        );
    }
}

#[test]
fn manifest_rejects_reserved_source_names() {
    for name in ["keyboard", "music", "webcam"] {
        let mut manifest = Manifest::minimal("demo");
        manifest.passes[0].name = name.into();
        assert!(
            manifest.validate_structure().is_err(),
            "pass name should be rejected: {name}"
        );
    }
}

#[test]
fn built_in_input_kind_requires_canonical_source_name() {
    let mut manifest = Manifest::minimal("demo");
    manifest.passes[0].inputs.push(Input {
        channel: 0,
        source: "keys".into(),
        kind: Some(InputKind::Keyboard),
        output: PassOutput::Index(0),
        frame: FrameRef::Current,
        filter: Filter::Linear,
        wrap: Wrap::Clamp,
    });
    assert!(manifest.validate_structure().is_err());
}

#[test]
fn fixed_dimensions_require_complete_buffer_pair() {
    let mut manifest = Manifest::multipass("fixed");
    manifest.passes[0].width = Some(256);
    assert!(manifest.validate_structure().is_err());

    manifest.passes[0].height = Some(256);
    manifest
        .validate_structure()
        .expect("buffer pass should accept fixed dimensions");

    manifest.passes[1].width = Some(256);
    manifest.passes[1].height = Some(256);
    assert!(manifest.validate_structure().is_err());
}

#[test]
fn multiple_outputs_validate_only_for_offscreen_2d_passes() {
    let mut manifest = Manifest::multipass("mrt");
    manifest.passes[0].extra_outputs = vec![RenderFormat::Rg32f, RenderFormat::Rgba16f];
    manifest
        .validate_structure()
        .expect("buffer MRT should validate");

    manifest.passes[1].extra_outputs = vec![RenderFormat::Rg32f];
    assert!(
        manifest.validate_structure().is_err(),
        "final image may not expose extra outputs"
    );
}

#[test]
fn input_output_index_must_exist_and_previous_feedback_uses_primary_output() {
    let mut manifest = Manifest::multipass("mrt-input");
    manifest.passes[0].extra_outputs = vec![RenderFormat::Rg32f];
    manifest.passes[1].inputs[0].output = PassOutput::Index(1);
    manifest
        .validate_structure()
        .expect("current-frame output 1 should validate");

    manifest.passes[1].inputs[0].output = PassOutput::Index(2);
    assert!(manifest.validate_structure().is_err());

    manifest.passes[1].inputs[0].output = PassOutput::Index(1);
    manifest.passes[1].inputs[0].frame = FrameRef::Previous;
    assert!(
        manifest.validate_structure().is_err(),
        "non-primary MRT feedback is intentionally not resumable"
    );
}

#[test]
fn compute_local_size_rejects_multiple_z_lanes() {
    let mut manifest = Manifest::minimal("compute-z");
    manifest.passes.insert(
        0,
        Pass {
            name: "simulation".into(),
            kind: PassKind::Compute,
            source: "shaders/simulation.comp".into(),
            vertex: None,
            mesh: None,
            width: Some(1),
            height: Some(1),
            format: RenderFormat::Rgba32f,
            extra_outputs: Vec::new(),
            iterations: 1,
            local_size: Some([1, 1, 2]),
            storage: Vec::new(),
            inputs: Vec::new(),
        },
    );

    let error = manifest
        .validate_structure()
        .expect_err("2D compute entrypoints must reject multiple local Z lanes");
    assert!(error.to_string().contains("z"), "{error}");
    assert!(error.to_string().contains("1"), "{error}");
}

#[test]
fn visual_reference_tests_reject_frame_resolution_matrices() {
    let source = r#"format = 1
[project]
name = "matrix-reference"

[[pass]]
name = "image"
kind = "image"
source = "shaders/image.frag"

[[test]]
name = "ambiguous-reference"
frames = [0, 1]
resolutions = [[16, 16], [32, 32]]
reference = "tests/reference.png"
"#;
    let manifest: Manifest = toml::from_str(source).expect("manifest should parse");
    let error = manifest
        .validate_structure()
        .expect_err("one reference path cannot describe a frame/resolution matrix");
    assert!(
        error.to_string().contains("single reference image"),
        "{error}"
    );
}

#[test]
fn named_preset_scales_output_and_overrides_pass_settings() {
    let source = r#"format = 1
[project]
name = "preset-demo"

[render]
width = 1600
height = 900

[[pass]]
name = "waves"
kind = "compute"
source = "shaders/waves.comp"
width = 1024
height = 1024
iterations = 4

[[pass]]
name = "image"
kind = "image"
source = "shaders/image.frag"

[preset.low]
render_scale = 0.75

[preset.low.pass.waves]
width = 640
height = 360
iterations = 2
local_size = [16, 8, 1]
"#;

    let mut manifest: Manifest = toml::from_str(source).expect("preset manifest should parse");
    manifest
        .validate_structure()
        .expect("preset manifest should validate");
    manifest.apply_preset("low").expect("preset should apply");

    assert_eq!((manifest.render.width, manifest.render.height), (1200, 675));
    let waves = manifest
        .passes
        .iter()
        .find(|pass| pass.name == "waves")
        .expect("waves pass");
    assert_eq!((waves.width, waves.height), (Some(640), Some(360)));
    assert_eq!(waves.iterations, 2);
    assert_eq!(waves.local_size, Some([16, 8, 1]));
}

#[test]
fn named_preset_rejects_unknown_pass_and_unknown_selection() {
    let mut manifest = Manifest::minimal("preset-errors");
    manifest.presets.insert(
        "bad".into(),
        Preset {
            render_scale: Some(0.5),
            passes: BTreeMap::from([(
                "missing".into(),
                PresetPass {
                    width: Some(320),
                    height: Some(180),
                    ..PresetPass::default()
                },
            )]),
        },
    );
    assert!(manifest.validate_structure().is_err());

    let manifest = Manifest::minimal("preset-errors");
    let mut manifest = manifest;
    let error = manifest
        .apply_preset("missing")
        .expect_err("unknown preset should fail");
    assert!(error.to_string().contains("unknown preset"), "{error}");
}

#[test]
fn manifest_accepts_channel_15_and_rejects_channel_16() {
    let mut manifest = Manifest::minimal("extended-channels");
    manifest.passes[0].inputs.push(Input {
        channel: 15,
        source: "keyboard".into(),
        kind: Some(InputKind::Keyboard),
        output: PassOutput::Index(0),
        frame: FrameRef::Current,
        filter: Filter::Nearest,
        wrap: Wrap::Clamp,
    });
    manifest
        .validate_structure()
        .expect("channel 15 should be valid");

    manifest.passes[0].inputs[0].channel = 16;
    let error = manifest
        .validate_structure()
        .expect_err("channel 16 should be rejected");
    assert!(error.to_string().contains("0..=15"), "{error}");
}

#[test]
fn mesh_quality_preset_overrides_geometry_density() {
    let source = r#"format = 1
[project]
name = "mesh-preset"

[[pass]]
name = "surface"
kind = "mesh"
source = "shaders/surface.frag"
vertex = "shaders/surface.vert"

[pass.mesh]
size = [257, 257]
instances = 6
depth = true

[[pass]]
name = "image"
kind = "image"
source = "shaders/image.frag"

[[pass.input]]
channel = 0
source = "surface"

[preset.low.pass.surface]
mesh_size = [65, 65]
mesh_instances = 3
"#;

    let mut manifest: Manifest = toml::from_str(source).expect("mesh preset should parse");
    manifest
        .validate_structure()
        .expect("mesh preset should validate");
    manifest
        .apply_preset("low")
        .expect("mesh preset should apply");

    let surface = manifest
        .passes
        .iter()
        .find(|pass| pass.name == "surface")
        .expect("surface pass");
    let mesh = surface.mesh.as_ref().expect("mesh config");
    assert_eq!(mesh.size, [65, 65]);
    assert_eq!(mesh.instances, 3);
}

#[test]
fn mesh_quality_preset_rejects_geometry_controls_on_non_mesh_pass() {
    let source = r#"format = 1
[project]
name = "bad-mesh-preset"

[[pass]]
name = "buffer"
kind = "buffer"
source = "shaders/buffer.frag"

[[pass]]
name = "image"
kind = "image"
source = "shaders/image.frag"

[[pass.input]]
channel = 0
source = "buffer"

[preset.low.pass.buffer]
mesh_size = [65, 65]
"#;

    let manifest: Manifest = toml::from_str(source).expect("manifest should parse");
    let error = manifest
        .validate_structure()
        .expect_err("mesh_size on a buffer pass must be rejected");
    assert!(error.to_string().contains("mesh_size"), "{error}");
}

#[test]
fn test_named_depth_output_requires_depth_enabled_mesh() {
    let valid = r#"format = 1
[project]
name = "mesh-depth-test"

[[pass]]
name = "surface"
kind = "mesh"
source = "shaders/surface.frag"
vertex = "shaders/surface.vert"

[pass.mesh]
size = [3, 3]
instances = 1
depth = true

[[pass]]
name = "image"
kind = "image"
source = "shaders/image.frag"

[[pass.input]]
channel = 0
source = "surface"

[[test]]
name = "surface-depth"
pass = "surface"
output = "depth"
frame = 0
assert_no_nan = true
"#;

    let manifest: Manifest = toml::from_str(valid).expect("depth test should parse");
    manifest
        .validate_structure()
        .expect("depth-enabled mesh test should validate");

    let invalid = valid.replace("depth = true", "depth = false");
    let manifest: Manifest = toml::from_str(&invalid).expect("invalid depth test should parse");
    let error = manifest
        .validate_structure()
        .expect_err("depth test on depth-disabled mesh must fail");
    assert!(
        error.to_string().contains("no readable depth output"),
        "{error}"
    );
}
