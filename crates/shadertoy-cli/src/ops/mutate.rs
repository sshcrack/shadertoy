use super::*;

pub fn add_pass(
    project_path: &Path,
    name: &str,
    kind: PassKind,
    source: Option<&Path>,
) -> Result<Output> {
    use crate::manifest::{Pass, validate_project_relative_path};
    use crate::scaffold::write_manifest;

    let mut loaded = LoadedManifest::load(project_path)?;
    if loaded.manifest.passes.iter().any(|pass| pass.name == name)
        || loaded
            .manifest
            .assets
            .iter()
            .any(|asset| asset.name == name)
    {
        bail!("pass/asset name '{}' already exists", name);
    }
    if kind == PassKind::Image {
        bail!(
            "project already has its required final image pass; add a buffer or cubemap pass instead"
        );
    }

    let source_rel = match source {
        Some(path) => {
            let value = path
                .to_str()
                .context("pass source must be valid UTF-8")?
                .replace('\\', "/");
            PathBuf::from(value)
        }
        None => default_pass_source(&loaded, name, kind),
    };
    let source_text = source_rel
        .to_str()
        .context("pass source must be valid UTF-8")?
        .replace('\\', "/");
    validate_project_relative_path(&source_text, "pass source")?;
    let source_abs = project_write_target(&loaded.root, &source_rel)?;
    if source_abs.exists() && !source_abs.is_file() {
        bail!("pass source is not a file: {}", source_abs.display());
    }

    let mesh_vertex_rel =
        (kind == PassKind::Mesh).then(|| default_mesh_vertex_source(&loaded, name));
    let mesh_vertex_abs = mesh_vertex_rel
        .as_ref()
        .map(|path| project_write_target(&loaded.root, path))
        .transpose()?;
    if let Some(path) = &mesh_vertex_abs
        && path.exists()
        && !path.is_file()
    {
        bail!("mesh vertex source is not a file: {}", path.display());
    }
    let mesh_vertex_text = mesh_vertex_rel
        .as_ref()
        .map(|path| {
            path.to_str()
                .context("mesh vertex source must be valid UTF-8")
        })
        .transpose()?
        .map(|value| value.replace('\\', "/"));
    if let Some(vertex) = &mesh_vertex_text {
        validate_project_relative_path(vertex, "mesh vertex source")?;
    }

    loaded.manifest.passes.insert(
        loaded.manifest.passes.len().saturating_sub(1),
        Pass {
            name: name.to_string(),
            kind,
            source: source_text,
            vertex: mesh_vertex_text.clone(),
            mesh: (kind == PassKind::Mesh).then_some(crate::manifest::MeshConfig {
                topology: crate::manifest::MeshTopology::Grid,
                size: [33, 33],
                instances: 1,
                depth: true,
                cull: crate::manifest::MeshCull::None,
                clear: [0.0; 4],
            }),
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
        },
    );
    // Validate the complete mutation before touching the filesystem. In
    // particular, a bad pass name must not leave a source stub behind.
    loaded.manifest.validate_structure()?;

    let created_source = !source_abs.exists();
    let created_vertex = mesh_vertex_abs.as_ref().is_some_and(|path| !path.exists());
    if created_source {
        if let Some(parent) = source_abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&source_abs, pass_stub(kind))?;
    }
    if let Some(vertex_abs) = &mesh_vertex_abs
        && created_vertex
    {
        if let Some(parent) = vertex_abs.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Err(error) = fs::write(vertex_abs, crate::include_file!("templates/pass/mesh.vert"))
        {
            if created_source {
                let _ = fs::remove_file(&source_abs);
            }
            return Err(error).with_context(|| format!("failed to write {}", vertex_abs.display()));
        }
    }

    if let Err(error) = write_manifest(&loaded.root, &loaded.manifest) {
        if created_source {
            let _ = fs::remove_file(&source_abs);
        }
        if created_vertex && let Some(vertex_abs) = &mesh_vertex_abs {
            let _ = fs::remove_file(vertex_abs);
        }
        return Err(error);
    }

    Ok(Output {
        human: format!(
            "Added {:?} pass '{}' -> {}",
            kind,
            name,
            source_rel.display()
        ),
        json: json!({
            "ok": true,
            "action": "pass-add",
            "pass": name,
            "kind": format!("{kind:?}").to_lowercase(),
            "source": source_rel,
            "vertex": mesh_vertex_rel,
        }),
    })
}

pub fn remove_pass(project_path: &Path, name: &str, force: bool) -> Result<Output> {
    use crate::scaffold::write_manifest;

    let mut loaded = LoadedManifest::load(project_path)?;
    let index = loaded
        .manifest
        .passes
        .iter()
        .position(|pass| pass.name == name)
        .with_context(|| format!("unknown pass '{name}'"))?;
    if loaded.manifest.passes[index].kind == PassKind::Image {
        bail!("cannot remove the required final image pass");
    }

    let consumers = loaded
        .manifest
        .passes
        .iter()
        .filter(|pass| pass.inputs.iter().any(|input| input.source == name))
        .map(|pass| pass.name.clone())
        .collect::<Vec<_>>();
    if !consumers.is_empty() && !force {
        bail!(
            "pass '{}' is used by {}; remove those channels first or use --force",
            name,
            consumers.join(", ")
        );
    }
    if force {
        for pass in &mut loaded.manifest.passes {
            pass.inputs.retain(|input| input.source != name);
        }
    }
    let removed = loaded.manifest.passes.remove(index);
    write_manifest(&loaded.root, &loaded.manifest)?;

    Ok(Output {
        human: format!("Removed pass '{}'", name),
        json: json!({
            "ok": true,
            "action": "pass-remove",
            "pass": name,
            "source_left_on_disk": removed.source,
            "removed_consumer_channels": if force { consumers } else { Vec::<String>::new() },
        }),
    })
}

#[derive(Debug, Clone)]
pub struct ChannelSetOptions {
    pub pass: String,
    pub channel: u8,
    pub source: String,
    pub kind: Option<crate::manifest::InputKind>,
    pub output: crate::manifest::PassOutput,
    pub previous: bool,
    pub filter: crate::manifest::Filter,
    pub wrap: crate::manifest::Wrap,
}

pub fn set_channel(project_path: &Path, options: &ChannelSetOptions) -> Result<Output> {
    let pass_name = options.pass.as_str();
    let channel = options.channel;
    let source = options.source.as_str();
    let kind = options.kind;
    let output = options.output;
    let previous = options.previous;
    let filter = options.filter;
    let wrap = options.wrap;
    use crate::manifest::{FrameRef, Input};
    use crate::scaffold::write_manifest;

    if channel > crate::manifest::MAX_INPUT_CHANNEL {
        bail!(
            "channel must be between 0 and {}",
            crate::manifest::MAX_INPUT_CHANNEL
        );
    }
    let mut loaded = LoadedManifest::load(project_path)?;
    let pass = loaded
        .manifest
        .passes
        .iter_mut()
        .find(|pass| pass.name == pass_name)
        .with_context(|| format!("unknown pass '{pass_name}'"))?;
    let input = Input {
        channel,
        source: source.to_string(),
        kind,
        output,
        frame: if previous {
            FrameRef::Previous
        } else {
            FrameRef::Current
        },
        filter,
        wrap,
    };
    if let Some(existing) = pass
        .inputs
        .iter_mut()
        .find(|input| input.channel == channel)
    {
        *existing = input;
    } else {
        pass.inputs.push(input);
        pass.inputs.sort_by_key(|input| input.channel);
    }
    loaded.manifest.validate_structure()?;
    write_manifest(&loaded.root, &loaded.manifest)?;

    Ok(Output {
        human: format!("Set {} iChannel{} <- {}", pass_name, channel, source),
        json: json!({
            "ok": true,
            "action": "channel-set",
            "pass": pass_name,
            "channel": channel,
            "source": source,
            "output": output,
            "previous": previous,
            "filter": format!("{filter:?}").to_lowercase(),
            "wrap": format!("{wrap:?}").to_lowercase(),
        }),
    })
}

pub fn remove_channel(project_path: &Path, pass_name: &str, channel: u8) -> Result<Output> {
    use crate::scaffold::write_manifest;

    if channel > crate::manifest::MAX_INPUT_CHANNEL {
        bail!(
            "channel must be between 0 and {}",
            crate::manifest::MAX_INPUT_CHANNEL
        );
    }
    let mut loaded = LoadedManifest::load(project_path)?;
    let pass = loaded
        .manifest
        .passes
        .iter_mut()
        .find(|pass| pass.name == pass_name)
        .with_context(|| format!("unknown pass '{pass_name}'"))?;
    let before = pass.inputs.len();
    pass.inputs.retain(|input| input.channel != channel);
    if pass.inputs.len() == before {
        bail!("{} has no iChannel{} binding", pass_name, channel);
    }
    write_manifest(&loaded.root, &loaded.manifest)?;

    Ok(Output {
        human: format!("Removed {} iChannel{}", pass_name, channel),
        json: json!({
            "ok": true,
            "action": "channel-remove",
            "pass": pass_name,
            "channel": channel,
        }),
    })
}

fn default_pass_source(loaded: &LoadedManifest, name: &str, kind: PassKind) -> PathBuf {
    let base = slug(name);
    for suffix in 1usize.. {
        let stem = if suffix == 1 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        let extension = if kind == PassKind::Compute {
            "comp"
        } else {
            "frag"
        };
        let candidate = PathBuf::from(format!("shaders/{stem}.{extension}"));
        let serialized = candidate.to_string_lossy();
        if !loaded
            .manifest
            .passes
            .iter()
            .any(|pass| pass.source == serialized)
        {
            return candidate;
        }
    }
    unreachable!("unbounded pass-source suffix search must find a free name")
}

fn default_mesh_vertex_source(loaded: &LoadedManifest, name: &str) -> PathBuf {
    let base = slug(name);
    for suffix in 1usize.. {
        let stem = if suffix == 1 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        let candidate = PathBuf::from(format!("shaders/{stem}.vert"));
        let serialized = candidate.to_string_lossy();
        if !loaded
            .manifest
            .passes
            .iter()
            .filter_map(|pass| pass.vertex.as_deref())
            .any(|vertex| vertex == serialized)
        {
            return candidate;
        }
    }
    unreachable!("unbounded mesh-vertex suffix search must find a free name")
}

fn project_write_target(root: &Path, relative: &Path) -> Result<PathBuf> {
    let candidate = root.join(relative);
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("failed to resolve project root {}", root.display()))?;

    let candidate_metadata = fs::symlink_metadata(&candidate).ok();
    if candidate_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        bail!(
            "pass source must not be a symbolic link: {}",
            candidate.display()
        );
    }

    let mut probe = if candidate_metadata.is_some() {
        candidate.as_path()
    } else {
        candidate.parent().unwrap_or(root)
    };
    while !probe.exists() {
        probe = probe
            .parent()
            .context("pass source has no existing project ancestor")?;
    }
    let canonical_probe = fs::canonicalize(probe)
        .with_context(|| format!("failed to resolve pass source ancestor {}", probe.display()))?;
    if !canonical_probe.starts_with(&canonical_root) {
        bail!(
            "pass source resolves outside the project root: {}",
            candidate.display()
        );
    }
    Ok(candidate)
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "pass".to_string()
    } else {
        result.to_string()
    }
}

fn pass_stub(kind: PassKind) -> &'static str {
    match kind {
        PassKind::Buffer | PassKind::Image | PassKind::Mesh => {
            crate::include_file!("templates/pass/buffer.frag")
        }
        PassKind::Cubemap => crate::include_file!("templates/pass/cubemap.frag"),
        PassKind::Compute => crate::include_file!("templates/pass/compute.comp"),
        PassKind::Sound => crate::include_file!("templates/pass/sound.frag"),
    }
}
