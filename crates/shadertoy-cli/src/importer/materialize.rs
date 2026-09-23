use super::browser::BrowserCapture;
use super::model::{RenderPass, Sampler, ShaderInput, ShaderToyEntry};
use crate::manifest::{
    Asset, AssetKind, Filter, FrameRef, Input, InputKind, Manifest, Pass, PassKind, ProjectSection,
    RenderSection, ShaderSection, Wrap,
};
use crate::scaffold::write_manifest;
use anyhow::{Context, Result, bail};
use image::{ImageReader, RgbaImage, imageops};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const DYNAMIC_CUBEMAP_ID: &str = "4dX3Rr";
const INITIAL_BUFFER: &str = r#"void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    fragColor = vec4(0.0,0.0,1.0,1.0);
}
"#;
const INITIAL_CUBEMAP: &str = r#"void mainCubemap( out vec4 fragColor, in vec2 fragCoord, in vec3 rayOri, in vec3 rayDir )
{
    vec3 col = 0.5 + 0.5*rayDir;
    fragColor = vec4(col,1.0);
}
"#;

#[derive(Debug)]
pub struct ImportResult {
    pub root: PathBuf,
    pub name: String,
    pub shader_id: String,
    pub source_url: String,
    pub pass_count: usize,
    pub asset_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct ImportedPass {
    source: RenderPass,
    local_name: String,
    output_id: String,
    kind: PassKind,
}

#[derive(Debug)]
struct ProjectBuilder<'a> {
    capture: &'a BrowserCapture,
    root: &'a Path,
    common: String,
    imported: Vec<ImportedPass>,
    output_to_pass: HashMap<String, String>,
    pass_orders: HashMap<String, u32>,
    manifest_passes: Vec<Pass>,
    assets: Vec<Asset>,
    asset_cache: HashMap<String, String>,
    names: HashSet<String>,
    warnings: Vec<String>,
    synthetic_counter: usize,
    asset_counter: usize,
}

pub fn materialize_capture(
    capture: &BrowserCapture,
    destination: Option<&Path>,
) -> Result<ImportResult> {
    let response_bytes = fs::read(&capture.response)
        .with_context(|| format!("failed to read {}", capture.response.display()))?;
    let response: Vec<ShaderToyEntry> =
        serde_json::from_slice(&response_bytes).context("failed to parse ShaderToy response")?;
    let entry = response
        .into_iter()
        .next()
        .context("ShaderToy response did not contain a shader")?;

    let project_name = nonempty(&entry.info.name)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("ShaderToy {}", capture.shader_id));
    let root = destination.map(PathBuf::from).unwrap_or_else(|| {
        let generated = file_safe_name(&project_name);
        if generated.is_empty() {
            PathBuf::from(format!(
                "shadertoy-{}",
                capture.shader_id.to_ascii_lowercase()
            ))
        } else {
            PathBuf::from(generated)
        }
    });
    if root.exists() {
        bail!(
            "import destination {} already exists; choose a new directory",
            root.display()
        );
    }

    let parent = root
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create import parent {}", parent.display()))?;
    let temporary = tempfile::Builder::new()
        .prefix(".shadertoy-import-")
        .tempdir_in(parent)
        .with_context(|| {
            format!(
                "failed to create temporary import directory in {}",
                parent.display()
            )
        })?;
    let staged_root = temporary.path().join("project");
    fs::create_dir_all(staged_root.join("shaders"))?;
    fs::create_dir_all(staged_root.join("assets"))?;
    fs::create_dir_all(staged_root.join(".shadertoy"))?;

    let (common, imported) = prepare_passes(&entry.renderpass)?;
    let mut builder = ProjectBuilder::new(capture, &staged_root, common, imported);
    builder.write_passes()?;
    builder.wire_inputs()?;

    let manifest = Manifest {
        format: crate::manifest::FORMAT_VERSION,
        project: ProjectSection {
            name: project_name.clone(),
            author: nonempty(&entry.info.username).map(str::to_owned),
            description: nonempty(&entry.info.description).map(str::to_owned),
            source_url: Some(capture.source_url.clone()),
            source_id: Some(capture.shader_id.clone()),
        },
        render: RenderSection::default(),
        shader: ShaderSection::default(),
        uniforms: Vec::new(),
        assets: builder.assets,
        passes: builder.manifest_passes,
        tests: Vec::new(),
        presets: std::collections::BTreeMap::new(),
    };
    manifest.validate_structure()?;
    write_manifest(&staged_root, &manifest)?;
    write_support_files(&staged_root, &project_name)?;
    fs::copy(
        &capture.response,
        staged_root.join(".shadertoy/import-response.json"),
    )
    .context("failed to preserve the original ShaderToy response")?;
    fs::write(
        staged_root.join(".shadertoy/import.json"),
        serde_json::to_vec_pretty(&json!({
            "source": capture.source_url,
            "shader_id": capture.shader_id,
            "browser": "camoufox",
            "response": "import-response.json",
        }))?,
    )?;

    let pass_count = manifest.passes.len();
    let asset_count = manifest.assets.len();
    let warnings = builder.warnings;
    fs::rename(&staged_root, &root).with_context(|| {
        format!(
            "failed to move completed import into destination {}",
            root.display()
        )
    })?;

    Ok(ImportResult {
        root,
        name: project_name,
        shader_id: capture.shader_id.clone(),
        source_url: capture.source_url.clone(),
        pass_count,
        asset_count,
        warnings,
    })
}

impl<'a> ProjectBuilder<'a> {
    fn new(
        capture: &'a BrowserCapture,
        root: &'a Path,
        common: String,
        imported: Vec<ImportedPass>,
    ) -> Self {
        let mut output_to_pass = HashMap::new();
        let mut pass_orders = HashMap::new();
        let mut names = HashSet::new();
        for pass in &imported {
            output_to_pass.insert(pass.output_id.clone(), pass.local_name.clone());
            pass_orders.insert(
                pass.local_name.clone(),
                pass_order(nonempty(&pass.source.name).unwrap_or(&pass.local_name)),
            );
            names.insert(pass.local_name.clone());
        }
        Self {
            capture,
            root,
            common,
            imported,
            output_to_pass,
            pass_orders,
            manifest_passes: Vec::new(),
            assets: Vec::new(),
            asset_cache: HashMap::new(),
            names,
            warnings: Vec::new(),
            synthetic_counter: 0,
            asset_counter: 0,
        }
    }

    fn write_passes(&mut self) -> Result<()> {
        for pass in &self.imported {
            let source_path = format!("shaders/{}.frag", pass.local_name);
            fs::write(
                self.root.join(&source_path),
                format!("{}{}", self.common, pass.source.code),
            )
            .with_context(|| format!("failed to write shader source {source_path}"))?;
            self.manifest_passes.push(Pass {
                name: pass.local_name.clone(),
                kind: pass.kind,
                source: source_path,
                vertex: None,
                mesh: None,
                width: None,
                height: None,
                format: crate::manifest::RenderFormat::default(),
                extra_outputs: Vec::new(),
                iterations: 1,
                local_size: None,
                storage: Vec::new(),
                inputs: Vec::new(),
            });
        }
        Ok(())
    }

    fn wire_inputs(&mut self) -> Result<()> {
        for imported_index in 0..self.imported.len() {
            let imported = self.imported[imported_index].clone();
            for shader_input in &imported.source.inputs {
                let Some(input) = self.convert_input(&imported, shader_input)? else {
                    continue;
                };
                let pass = self
                    .manifest_passes
                    .iter_mut()
                    .find(|pass| pass.name == imported.local_name)
                    .expect("imported pass was materialized");
                pass.inputs.push(input);
                pass.inputs.sort_by_key(|input| input.channel);
            }
        }
        Ok(())
    }

    fn convert_input(
        &mut self,
        destination: &ImportedPass,
        input: &ShaderInput,
    ) -> Result<Option<Input>> {
        if input.channel > 3 {
            bail!(
                "ShaderToy pass '{}' uses invalid channel {}",
                destination.local_name,
                input.channel
            );
        }
        let filter = parse_filter(&input.sampler)?;
        let wrap = parse_wrap(&input.sampler)?;

        let (source, kind, frame, channel) = match input.kind.as_str() {
            "buffer" => {
                let source = self.ensure_pass_source(&input.id, PassKind::Buffer)?;
                let frame = self.frame_reference(&source, &destination.local_name);
                (source, InputKind::Pass, frame, input.channel)
            }
            "cubemap" if input.id == DYNAMIC_CUBEMAP_ID => {
                let Some(source_pass) = self
                    .imported
                    .iter()
                    .find(|pass| pass.kind == PassKind::Cubemap)
                    .cloned()
                else {
                    self.warnings.push(format!(
                        "pass '{}' references ShaderToy's dynamic cubemap but no cubemap pass was present",
                        destination.local_name
                    ));
                    return Ok(None);
                };
                let source = source_pass.local_name;
                let frame = self.frame_reference(&source, &destination.local_name);
                let channel = source_pass
                    .source
                    .outputs
                    .first()
                    .and_then(|output| output.channel)
                    .unwrap_or(input.channel);
                (source, InputKind::Pass, frame, channel)
            }
            "keyboard" => (
                "keyboard".into(),
                InputKind::Keyboard,
                FrameRef::Current,
                input.channel,
            ),
            "music" | "musicstream" | "mic" | "audio" => (
                "music".into(),
                InputKind::Music,
                FrameRef::Current,
                input.channel,
            ),
            "texture" => (
                self.ensure_asset(input, AssetKind::Texture)?,
                InputKind::Texture,
                FrameRef::Current,
                input.channel,
            ),
            "cubemap" => (
                self.ensure_asset(input, AssetKind::Cubemap)?,
                InputKind::Cubemap,
                FrameRef::Current,
                input.channel,
            ),
            "volume" => (
                self.ensure_asset(input, AssetKind::Volume)?,
                InputKind::Volume,
                FrameRef::Current,
                input.channel,
            ),
            "video" => (
                self.ensure_asset(input, AssetKind::Video)?,
                InputKind::Video,
                FrameRef::Current,
                input.channel,
            ),
            "webcam" => (
                "webcam".into(),
                InputKind::Webcam,
                FrameRef::Current,
                input.channel,
            ),
            unsupported => {
                self.warnings.push(format!(
                    "pass '{}' iChannel{} uses unsupported ShaderToy input type '{}'; it was left unbound",
                    destination.local_name, input.channel, unsupported
                ));
                return Ok(None);
            }
        };

        Ok(Some(Input {
            channel,
            source,
            kind: Some(kind),
            output: crate::manifest::PassOutput::Index(0),
            frame,
            filter,
            wrap,
        }))
    }

    fn ensure_pass_source(&mut self, output_id: &str, kind: PassKind) -> Result<String> {
        if let Some(name) = self.output_to_pass.get(output_id) {
            return Ok(name.clone());
        }

        self.synthetic_counter += 1;
        let base = match kind {
            PassKind::Cubemap => "cubemap",
            _ => "buffer",
        };
        let name = unique_name(
            &mut self.names,
            &format!("{base}-missing-{}", self.synthetic_counter),
        );
        let source_path = format!("shaders/{name}.frag");
        fs::write(
            self.root.join(&source_path),
            if kind == PassKind::Cubemap {
                format!("{}{}", self.common, INITIAL_CUBEMAP)
            } else {
                format!("{}{}", self.common, INITIAL_BUFFER)
            },
        )?;
        self.manifest_passes.push(Pass {
            name: name.clone(),
            kind,
            source: source_path,
            vertex: None,
            mesh: None,
            width: if kind == PassKind::Compute {
                Some(256)
            } else {
                None
            },
            height: if kind == PassKind::Compute {
                Some(256)
            } else {
                None
            },
            format: crate::manifest::RenderFormat::default(),
            extra_outputs: Vec::new(),
            iterations: 1,
            local_size: None,
            storage: Vec::new(),
            inputs: Vec::new(),
        });
        self.output_to_pass
            .insert(output_id.to_string(), name.clone());
        self.pass_orders.insert(name.clone(), pass_order(&name));
        self.warnings.push(format!(
            "ShaderToy referenced missing output id '{output_id}'; created placeholder pass '{name}'"
        ));
        Ok(name)
    }

    fn frame_reference(&self, source: &str, destination: &str) -> FrameRef {
        let source_order = self
            .pass_orders
            .get(source)
            .copied()
            .unwrap_or_else(|| pass_order(source));
        let destination_order = self
            .pass_orders
            .get(destination)
            .copied()
            .unwrap_or_else(|| pass_order(destination));
        if source_order >= destination_order {
            FrameRef::Previous
        } else {
            FrameRef::Current
        }
    }

    fn ensure_asset(&mut self, input: &ShaderInput, kind: AssetKind) -> Result<String> {
        if input.filepath.is_empty() {
            bail!(
                "ShaderToy {} input '{}' has no filepath",
                asset_kind_name(kind),
                input.id
            );
        }
        let cache_key = format!(
            "{}\0{}\0{}\0{}",
            asset_kind_name(kind),
            input.id,
            input.filepath,
            input.sampler.vflip
        );
        if let Some(name) = self.asset_cache.get(&cache_key) {
            return Ok(name.clone());
        }

        self.asset_counter += 1;
        let stem = Path::new(input.filepath.split('?').next().unwrap_or(&input.filepath))
            .file_stem()
            .and_then(|value| value.to_str())
            .map(file_safe_name)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("asset-{}", self.asset_counter));
        let base = format!("{}-{stem}", asset_kind_name(kind));
        let name = unique_name(&mut self.names, &base);

        let path = match kind {
            AssetKind::Texture => self.materialize_texture(input, &name)?,
            AssetKind::Cubemap => self.materialize_cubemap(input, &name)?,
            AssetKind::Volume => self.materialize_volume(input, &name)?,
            AssetKind::Video => self.materialize_video(input, &name)?,
        };
        self.assets.push(Asset {
            name: name.clone(),
            kind,
            path,
        });
        self.asset_cache.insert(cache_key, name.clone());
        Ok(name)
    }

    fn materialize_texture(&self, input: &ShaderInput, name: &str) -> Result<String> {
        let source = self.resource(&input.filepath)?;
        let mut image = ImageReader::open(source)
            .with_context(|| format!("failed to open imported texture {}", source.display()))?
            .decode()
            .with_context(|| format!("failed to decode imported texture {}", source.display()))?
            .to_rgba8();
        if sampler_vflip(&input.sampler) {
            imageops::flip_vertical_in_place(&mut image);
        }
        let relative = format!("assets/{name}.png");
        image
            .save(self.root.join(&relative))
            .with_context(|| format!("failed to write imported texture {relative}"))?;
        Ok(relative)
    }

    fn materialize_cubemap(&self, input: &ShaderInput, name: &str) -> Result<String> {
        let paths = cubemap_face_paths(&input.filepath)?;
        let mut faces = Vec::with_capacity(6);
        let mut size = None;
        for path in paths {
            let resource = self.resource(&path)?;
            let mut image = ImageReader::open(resource)
                .with_context(|| {
                    format!(
                        "failed to open imported cubemap face {}",
                        resource.display()
                    )
                })?
                .decode()
                .with_context(|| {
                    format!(
                        "failed to decode imported cubemap face {}",
                        resource.display()
                    )
                })?
                .to_rgba8();
            if sampler_vflip(&input.sampler) {
                imageops::flip_vertical_in_place(&mut image);
            }
            let (width, height) = image.dimensions();
            if width == 0 || width != height {
                bail!(
                    "ShaderToy cubemap face '{}' is not square ({}x{})",
                    path,
                    width,
                    height
                );
            }
            if let Some(expected) = size {
                if expected != width {
                    bail!(
                        "ShaderToy cubemap '{}' has inconsistent face sizes",
                        input.filepath
                    );
                }
            } else {
                size = Some(width);
            }
            faces.push(image);
        }
        let size = size.context("ShaderToy cubemap has no faces")?;
        let width = size
            .checked_mul(6)
            .context("cubemap strip width overflow")?;
        let mut strip = RgbaImage::new(width, size);
        for (index, face) in faces.iter().enumerate() {
            imageops::replace(&mut strip, face, i64::from(size) * index as i64, 0);
        }
        let relative = format!("assets/{name}.png");
        strip
            .save(self.root.join(&relative))
            .with_context(|| format!("failed to write imported cubemap {relative}"))?;
        Ok(relative)
    }

    fn materialize_volume(&self, input: &ShaderInput, name: &str) -> Result<String> {
        let source = self.resource(&input.filepath)?;
        let relative = format!("assets/{name}.bin");
        fs::copy(source, self.root.join(&relative))
            .with_context(|| format!("failed to write imported volume {relative}"))?;
        Ok(relative)
    }

    fn materialize_video(&self, input: &ShaderInput, name: &str) -> Result<String> {
        let source = self.resource(&input.filepath)?;
        let extension = Path::new(input.filepath.split('?').next().unwrap_or(&input.filepath))
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("mp4");
        let relative = format!("assets/{name}.{extension}");
        fs::copy(source, self.root.join(&relative))
            .with_context(|| format!("failed to write imported video {relative}"))?;
        Ok(relative)
    }

    fn resource(&self, remote: &str) -> Result<&Path> {
        self.capture
            .resources
            .get(remote)
            .map(PathBuf::as_path)
            .with_context(|| format!("Camoufox capture is missing ShaderToy resource '{remote}'"))
    }
}

fn prepare_passes(render_passes: &[RenderPass]) -> Result<(String, Vec<ImportedPass>)> {
    let common = render_passes
        .iter()
        .filter(|pass| pass.kind == "common")
        .map(|pass| pass.code.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let common = if common.is_empty() {
        String::new()
    } else {
        format!("{common}\n")
    };

    let mut names = HashSet::new();
    let mut synthetic = 0usize;
    let mut imported = Vec::new();
    for pass in render_passes {
        let kind = match pass.kind.as_str() {
            "image" => PassKind::Image,
            "buffer" => PassKind::Buffer,
            "cubemap" => PassKind::Cubemap,
            "sound" => PassKind::Sound,
            "common" => continue,
            _ => continue,
        };
        synthetic += 1;
        let fallback = match kind {
            PassKind::Image => "image".to_string(),
            PassKind::Buffer => format!("buffer-{synthetic}"),
            PassKind::Mesh => format!("mesh-{synthetic}"),
            PassKind::Cubemap => format!("cubemap-{synthetic}"),
            PassKind::Compute => format!("compute-{synthetic}"),
            PassKind::Sound => format!("sound-{synthetic}"),
        };
        let base = nonempty(&pass.name)
            .map(file_safe_name)
            .filter(|name| !name.is_empty())
            .unwrap_or(fallback);
        let local_name = unique_name(&mut names, &base);
        let output_id = pass
            .outputs
            .first()
            .and_then(|output| nonempty(&output.id))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("__synthetic_output_{synthetic}"));
        imported.push(ImportedPass {
            source: pass.clone(),
            local_name,
            output_id,
            kind,
        });
    }
    if imported.is_empty() {
        bail!("ShaderToy response contains no supported image/buffer/cubemap/sound passes");
    }
    Ok((common, imported))
}

fn write_support_files(root: &Path, name: &str) -> Result<()> {
    crate::project_schema::write_current(root)?;
    fs::write(
        root.join(".taplo.toml"),
        crate::include_file!("templates/project/.taplo.toml"),
    )?;
    fs::write(root.join(".gitignore"), "target/\n")?;
    let note = "This project was imported from ShaderToy through Camoufox. The original response is preserved in .shadertoy/import-response.json and all supported static resources are local under assets/.";
    let readme = crate::include_file!("templates/project/README.md")
        .replace("{{name}}", name)
        .replace("{{template_note}}", note);
    fs::write(root.join("README.md"), readme)?;
    Ok(())
}

fn parse_filter(sampler: &Sampler) -> Result<Filter> {
    match sampler.filter.as_str() {
        "" | "linear" => Ok(Filter::Linear),
        "nearest" => Ok(Filter::Nearest),
        "mipmap" => Ok(Filter::Mipmap),
        other => bail!("unsupported ShaderToy sampler filter '{other}'"),
    }
}

fn parse_wrap(sampler: &Sampler) -> Result<Wrap> {
    match sampler.wrap.as_str() {
        "" | "repeat" => Ok(Wrap::Repeat),
        "clamp" => Ok(Wrap::Clamp),
        other => bail!("unsupported ShaderToy sampler wrap mode '{other}'"),
    }
}

fn sampler_vflip(sampler: &Sampler) -> bool {
    sampler.vflip.eq_ignore_ascii_case("true")
}

fn cubemap_face_paths(path: &str) -> Result<Vec<String>> {
    let (base_with_path, query) = path
        .split_once('?')
        .map_or((path, None), |(base, query)| (base, Some(query)));
    let dot = base_with_path
        .rfind('.')
        .context("ShaderToy cubemap filepath has no extension")?;
    let base = &base_with_path[..dot];
    let extension = &base_with_path[dot..];
    Ok(["", "_1", "_2", "_3", "_4", "_5"]
        .into_iter()
        .map(|suffix| {
            let mut value = format!("{base}{suffix}{extension}");
            if let Some(query) = query {
                value.push('?');
                value.push_str(query);
            }
            value
        })
        .collect())
}

fn pass_order(name: &str) -> u32 {
    let first = name
        .bytes()
        .next()
        .map(|byte| byte.to_ascii_uppercase() as u32)
        .unwrap_or(0);
    let last = name
        .bytes()
        .next_back()
        .map(|byte| byte.to_ascii_uppercase() as u32)
        .unwrap_or(0);
    first * 1000 + last
}

fn unique_name(names: &mut HashSet<String>, base: &str) -> String {
    let base = if base.is_empty() { "item" } else { base };
    if names.insert(base.to_string()) {
        return base.to_string();
    }
    for index in 2usize.. {
        let candidate = format!("{base}-{index}");
        if names.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

fn file_safe_name(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    result.trim_matches('-').to_string()
}

fn asset_kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Texture => "texture",
        AssetKind::Cubemap => "cubemap",
        AssetKind::Volume => "volume",
        AssetKind::Video => "video",
    }
}

fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cubemap_paths_preserve_query() {
        assert_eq!(
            cubemap_face_paths("/media/cube.png?v=1").unwrap(),
            vec![
                "/media/cube.png?v=1",
                "/media/cube_1.png?v=1",
                "/media/cube_2.png?v=1",
                "/media/cube_3.png?v=1",
                "/media/cube_4.png?v=1",
                "/media/cube_5.png?v=1",
            ]
        );
    }

    #[test]
    fn pass_order_matches_legacy_buffer_ordering() {
        assert!(pass_order("Buffer A") < pass_order("Buffer B"));
        assert_eq!(pass_order("Buffer A"), pass_order("Buffer A"));
    }

    #[test]
    fn names_are_file_safe_and_unique() {
        let mut names = HashSet::new();
        assert_eq!(file_safe_name("Buffer A!"), "buffer-a");
        assert_eq!(unique_name(&mut names, "image"), "image");
        assert_eq!(unique_name(&mut names, "image"), "image-2");
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::manifest::LoadedManifest;

    #[test]
    fn materializes_response_into_local_editable_project() {
        let staging = tempfile::tempdir().unwrap();
        let resource = staging.path().join("texture.png");
        RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]))
            .save(&resource)
            .unwrap();
        let response = staging.path().join("response.json");
        fs::write(
            &response,
            r#"[
              {
                "info": {
                  "name": "Imported Demo",
                  "username": "artist",
                  "description": "fixture"
                },
                "renderpass": [
                  {
                    "name": "Common",
                    "type": "common",
                    "code": "float sharedValue() { return 1.0; }",
                    "inputs": [],
                    "outputs": []
                  },
                  {
                    "name": "Buffer A",
                    "type": "buffer",
                    "code": "void mainImage(out vec4 c, in vec2 p) { c = vec4(sharedValue()); }",
                    "inputs": [
                      {
                        "id": "bufA",
                        "type": "buffer",
                        "channel": 0,
                        "filepath": "",
                        "sampler": {"filter":"linear","wrap":"clamp","vflip":"false"}
                      }
                    ],
                    "outputs": [{"id":"bufA"}]
                  },
                  {
                    "name": "Image",
                    "type": "image",
                    "code": "void mainImage(out vec4 c, in vec2 p) { c = texture(iChannel0, p/iResolution.xy) + texture(iChannel1, p/iResolution.xy); }",
                    "inputs": [
                      {
                        "id": "bufA",
                        "type": "buffer",
                        "channel": 0,
                        "filepath": "",
                        "sampler": {"filter":"linear","wrap":"clamp","vflip":"false"}
                      },
                      {
                        "id": "tex0",
                        "type": "texture",
                        "channel": 1,
                        "filepath": "/media/test.png",
                        "sampler": {"filter":"nearest","wrap":"repeat","vflip":"true"}
                      }
                    ],
                    "outputs": [{"id":"Image"}]
                  }
                ]
              }
            ]"#,
        )
        .unwrap();
        let mut resources = HashMap::new();
        resources.insert("/media/test.png".to_string(), resource);
        let capture = BrowserCapture {
            shader_id: "abc123".into(),
            source_url: "https://www.shadertoy.com/view/abc123".into(),
            response,
            resources,
        };
        let target = staging.path().join("project");
        let result = materialize_capture(&capture, Some(&target)).unwrap();
        assert_eq!(result.pass_count, 2);
        assert_eq!(result.asset_count, 1);

        let loaded = LoadedManifest::load(&target).unwrap();
        assert_eq!(loaded.manifest.project.author.as_deref(), Some("artist"));
        assert_eq!(
            loaded.manifest.project.source_url.as_deref(),
            Some("https://www.shadertoy.com/view/abc123")
        );
        let buffer = loaded
            .manifest
            .passes
            .iter()
            .find(|pass| pass.kind == PassKind::Buffer)
            .unwrap();
        assert_eq!(buffer.inputs[0].frame, FrameRef::Previous);
        let image = loaded
            .manifest
            .passes
            .iter()
            .find(|pass| pass.kind == PassKind::Image)
            .unwrap();
        assert_eq!(image.inputs[0].frame, FrameRef::Current);
        assert_eq!(image.inputs[1].kind, Some(InputKind::Texture));
        assert_eq!(image.inputs[1].filter, Filter::Nearest);
        assert!(target.join(".shadertoy/import-response.json").is_file());
        assert!(target.join("assets/texture-test.png").is_file());
        assert!(
            fs::read_to_string(target.join("shaders/image.frag"))
                .unwrap()
                .contains("sharedValue")
        );
    }
}
