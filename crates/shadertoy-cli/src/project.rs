use crate::manifest::{
    AssetKind, Filter, FrameRef, InputKind, LoadedManifest, PassKind, Wrap,
    validate_project_relative_path,
};
use crate::source::{SourceGraph, expand_all};
use anyhow::{Context, Result, bail};
use image::ImageReader;
use shadertoy::{
    Filter as NativeFilter, InputKind as NativeInputKind, MeshCull as NativeMeshCull,
    PASS_OUTPUT_DEPTH as NATIVE_DEPTH_OUTPUT, PassKind as NativePassKind, Project,
    RenderFormat as NativeRenderFormat, Wrap as NativeWrap,
};
use std::fs;
use std::path::Path;

pub(crate) fn add_manifest_assets(project: &mut Project, loaded: &LoadedManifest) -> Result<()> {
    for asset in &loaded.manifest.assets {
        let path = existing_project_file(&loaded.root, &asset.path, "asset", &asset.name)?;
        match asset.kind {
            AssetKind::Texture => {
                let image = ImageReader::open(&path)
                    .with_context(|| format!("failed to open texture {}", path.display()))?
                    .decode()
                    .with_context(|| format!("failed to decode texture {}", path.display()))?
                    .to_rgba8();
                let (width, height) = image.dimensions();
                project.add_texture_rgba8(&asset.name, width, height, image.as_raw())?;
            }
            AssetKind::Cubemap => {
                let image = ImageReader::open(&path)
                    .with_context(|| format!("failed to open cubemap {}", path.display()))?
                    .decode()
                    .with_context(|| format!("failed to decode cubemap {}", path.display()))?
                    .to_rgba8();
                let (width, height) = image.dimensions();
                let expected_width = height
                    .checked_mul(6)
                    .context("cubemap strip width overflow")?;
                if width != expected_width {
                    bail!(
                        "cubemap '{}' must be a horizontal 6-face strip (got {}x{}, expected {}x{})",
                        asset.name,
                        width,
                        height,
                        expected_width,
                        height
                    );
                }
                let face_bytes = (height as usize)
                    .checked_mul(height as usize)
                    .and_then(|pixels| pixels.checked_mul(4))
                    .context("cubemap face size overflow")?;
                let mut faces =
                    Vec::with_capacity(face_bytes.checked_mul(6).context("cubemap size overflow")?);
                let raw = image.as_raw();
                let row_bytes = (width as usize)
                    .checked_mul(4)
                    .context("cubemap row size overflow")?;
                let face_row_bytes = (height as usize)
                    .checked_mul(4)
                    .context("cubemap face row size overflow")?;
                for face in 0..6usize {
                    for y in 0..height as usize {
                        let start = y
                            .checked_mul(row_bytes)
                            .and_then(|offset| offset.checked_add(face * face_row_bytes))
                            .context("cubemap offset overflow")?;
                        faces.extend_from_slice(&raw[start..start + face_row_bytes]);
                    }
                }
                project.add_cubemap_rgba8(&asset.name, height, &faces)?;
            }
            AssetKind::Volume => {
                let bytes = fs::read(&path)
                    .with_context(|| format!("failed to read volume {}", path.display()))?;
                let (size, channels, data) = decode_shadertoy_volume(&bytes, &asset.name)?;
                project.add_volume_u8(&asset.name, size, channels, data)?;
            }
            AssetKind::Video => {
                let (width, height, rgba) = crate::media::initial_video_frame(&path)?;
                project.add_texture_rgba8(&asset.name, width, height, &rgba)?;
            }
        }
    }
    if crate::media::manifest_uses_webcam(loaded) {
        let rgba =
            vec![0u8; (crate::media::WEBCAM_WIDTH * crate::media::WEBCAM_HEIGHT * 4) as usize];
        project.add_texture_rgba8(
            crate::media::WEBCAM_NAME,
            crate::media::WEBCAM_WIDTH,
            crate::media::WEBCAM_HEIGHT,
            &rgba,
        )?;
    }
    Ok(())
}

pub fn build_native_project(loaded: &LoadedManifest) -> Result<Project> {
    build_native_project_with_sources(loaded).map(|(project, _)| project)
}

pub fn build_native_project_with_sources(
    loaded: &LoadedManifest,
) -> Result<(Project, SourceGraph)> {
    let sources = expand_all(loaded)?;
    let mut project = Project::new(&loaded.manifest.project.name)?;

    add_manifest_assets(&mut project, loaded)?;

    for pass in &loaded.manifest.passes {
        if pass.kind == PassKind::Sound {
            continue;
        }
        let source = &sources
            .get(&pass.name)
            .expect("all manifest passes have expanded sources")
            .text;
        project.add_pass(
            &pass.name,
            match pass.kind {
                PassKind::Image => NativePassKind::Image,
                PassKind::Buffer => NativePassKind::Buffer,
                PassKind::Mesh => NativePassKind::Mesh,
                PassKind::Cubemap => NativePassKind::Cubemap,
                PassKind::Compute => NativePassKind::Compute,
                PassKind::Sound => unreachable!("sound passes are lowered separately"),
            },
            source,
        )?;
        if let Some(mesh) = &pass.mesh {
            let vertex = sources
                .get(&pass.name)
                .and_then(|source| source.vertex_text.as_deref())
                .expect("validated mesh pass has expanded vertex source");
            project.configure_mesh(
                &pass.name,
                vertex,
                mesh.size[0],
                mesh.size[1],
                mesh.instances,
                mesh.depth,
                match mesh.cull {
                    crate::manifest::MeshCull::None => NativeMeshCull::None,
                    crate::manifest::MeshCull::Back => NativeMeshCull::Back,
                    crate::manifest::MeshCull::Front => NativeMeshCull::Front,
                },
                mesh.clear,
            )?;
        }
        if let (Some(width), Some(height)) = (pass.width, pass.height) {
            project.set_pass_resolution(&pass.name, width, height)?;
        }
        if matches!(
            pass.kind,
            PassKind::Buffer | PassKind::Compute | PassKind::Mesh
        ) {
            project.set_pass_format(
                &pass.name,
                match pass.format {
                    crate::manifest::RenderFormat::R32f => NativeRenderFormat::R32f,
                    crate::manifest::RenderFormat::Rg32f => NativeRenderFormat::Rg32f,
                    crate::manifest::RenderFormat::Rgba16f => NativeRenderFormat::Rgba16f,
                    crate::manifest::RenderFormat::Rgba32f => NativeRenderFormat::Rgba32f,
                },
            )?;
        }
        for format in &pass.extra_outputs {
            project.add_pass_output(
                &pass.name,
                match format {
                    crate::manifest::RenderFormat::R32f => NativeRenderFormat::R32f,
                    crate::manifest::RenderFormat::Rg32f => NativeRenderFormat::Rg32f,
                    crate::manifest::RenderFormat::Rgba16f => NativeRenderFormat::Rgba16f,
                    crate::manifest::RenderFormat::Rgba32f => NativeRenderFormat::Rgba32f,
                },
            )?;
        }
        if pass.iterations != 1 {
            project.set_pass_iterations(&pass.name, pass.iterations)?;
        }
        if pass.kind == PassKind::Compute {
            let [x, y, z] = pass.local_size.unwrap_or([8, 8, 1]);
            project.set_compute_local_size(&pass.name, x, y, z)?;
        }
        for storage in &pass.storage {
            project.bind_storage_buffer(
                &pass.name,
                storage.binding,
                &storage.name,
                storage.size,
            )?;
        }
    }

    for pass in &loaded.manifest.passes {
        if pass.kind == PassKind::Sound {
            continue;
        }
        for input in &pass.inputs {
            let kind = loaded.manifest.infer_input_kind(input)?;
            project.add_input_output(
                &pass.name,
                input.channel.into(),
                match kind {
                    InputKind::Pass => NativeInputKind::Pass,
                    InputKind::Texture => NativeInputKind::Texture,
                    InputKind::Cubemap => NativeInputKind::Cubemap,
                    InputKind::Volume => NativeInputKind::Volume,
                    InputKind::Keyboard => NativeInputKind::Keyboard,
                    InputKind::Music => NativeInputKind::Music,
                    InputKind::Video | InputKind::Webcam => NativeInputKind::Texture,
                },
                &input.source,
                match input.output {
                    crate::manifest::PassOutput::Index(index) => u32::from(index),
                    crate::manifest::PassOutput::Named(crate::manifest::NamedPassOutput::Depth) => {
                        NATIVE_DEPTH_OUTPUT
                    }
                },
                input.frame == FrameRef::Previous,
                match input.filter {
                    Filter::Mipmap => NativeFilter::Mipmap,
                    Filter::Linear => NativeFilter::Linear,
                    Filter::Nearest => NativeFilter::Nearest,
                },
                match input.wrap {
                    Wrap::Clamp => NativeWrap::Clamp,
                    Wrap::Repeat => NativeWrap::Repeat,
                },
            )?;
        }
    }

    Ok((project, sources))
}

pub fn ensure_source_files_exist(loaded: &LoadedManifest) -> Result<()> {
    for pass in &loaded.manifest.passes {
        existing_project_file(&loaded.root, &pass.source, "shader source", &pass.name)?;
        if let Some(vertex) = &pass.vertex {
            existing_project_file(&loaded.root, vertex, "vertex shader source", &pass.name)?;
        }
    }
    for asset in &loaded.manifest.assets {
        existing_project_file(&loaded.root, &asset.path, "asset", &asset.name)?;
    }
    Ok(())
}

fn decode_shadertoy_volume<'a>(bytes: &'a [u8], name: &str) -> Result<(u32, u32, &'a [u8])> {
    if bytes.len() < 20 {
        bail!("volume '{name}' is smaller than the ShaderToy 20-byte header");
    }
    let read_u32 = |offset: usize| -> Result<u32> {
        let raw: [u8; 4] = bytes[offset..offset + 4]
            .try_into()
            .expect("validated volume header bounds");
        Ok(u32::from_le_bytes(raw))
    };
    let x = read_u32(4)?;
    let y = read_u32(8)?;
    let z = read_u32(12)?;
    if x == 0 || x != y || y != z {
        bail!("volume '{name}' must have positive cubic dimensions (got {x}x{y}x{z})");
    }
    let metadata = read_u32(16)?;
    let channels = metadata & 0xff;
    let layout = (metadata >> 8) & 0xff;
    let format = (metadata >> 16) & 0xffff;
    if channels != 1 && channels != 4 {
        bail!("volume '{name}' uses unsupported channel count {channels}");
    }
    if layout != 0 || format != 0 {
        bail!("volume '{name}' uses unsupported ShaderToy layout/format ({layout}/{format})");
    }
    let expected = (x as usize)
        .checked_mul(x as usize)
        .and_then(|value| value.checked_mul(x as usize))
        .and_then(|value| value.checked_mul(channels as usize))
        .and_then(|value| value.checked_add(20))
        .context("volume payload size overflow")?;
    if bytes.len() != expected {
        bail!(
            "volume '{name}' payload has {} bytes, expected {expected}",
            bytes.len()
        );
    }
    Ok((x, channels, &bytes[20..]))
}

pub(crate) fn existing_project_file(
    root: &Path,
    relative: &str,
    kind: &str,
    name: &str,
) -> Result<std::path::PathBuf> {
    validate_project_relative_path(relative, &format!("{kind} path for '{name}'"))?;
    let path = root.join(relative);
    if !path.is_file() {
        bail!("{kind} '{name}' does not exist at {}", path.display());
    }

    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("failed to resolve project root {}", root.display()))?;
    let canonical_path = fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve {kind} {}", path.display()))?;
    if !canonical_path.starts_with(&canonical_root) {
        bail!(
            "{kind} '{name}' resolves outside the project root: {}",
            path.display()
        );
    }
    Ok(canonical_path)
}

#[cfg(test)]
mod volume_tests {
    use super::*;

    #[test]
    fn decodes_shader_toy_volume_header() {
        let mut bytes = vec![0u8; 20];
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
        bytes.extend(0u8..8);
        let (size, channels, data) = decode_shadertoy_volume(&bytes, "fixture").unwrap();
        assert_eq!(size, 2);
        assert_eq!(channels, 1);
        assert_eq!(data, &(0u8..8).collect::<Vec<_>>()[..]);
    }

    #[test]
    fn rejects_non_cubic_volume() {
        let mut bytes = vec![0u8; 20];
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&3u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_shadertoy_volume(&bytes, "bad").is_err());
    }
}
