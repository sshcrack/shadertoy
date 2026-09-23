use super::*;

pub fn new_project(path: &Path, template: Template) -> Result<Output> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .context("project path must end in a valid UTF-8 directory name")?;
    create_project(path, name, template, false)?;
    Ok(Output {
        human: format!("Created ShaderToy project {}", path.display()),
        json: json!({
            "ok": true,
            "action": "new",
            "project": path,
            "name": name,
            "template": template_name(template),
        }),
    })
}

pub fn init_project(path: &Path, template: Template) -> Result<Output> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let name = absolute
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .context("project directory must have a valid UTF-8 name")?;
    create_project(&absolute, name, template, true)?;
    Ok(Output {
        human: format!("Initialized ShaderToy project {}", absolute.display()),
        json: json!({
            "ok": true,
            "action": "init",
            "project": absolute,
            "name": name,
            "template": template_name(template),
        }),
    })
}

pub fn check_project(options: &CheckOptions) -> Result<Output> {
    let loaded = LoadedManifest::load_with_preset(&options.project, options.preset.as_deref())?;
    ensure_source_files_exist(&loaded)?;

    let diagnostics = super::graph::analyze_graph(&loaded, options.pedantic)?;
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.level == "error")
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.level == "warning")
        .count();

    if errors != 0 {
        let mut human = format!(
            "FAILED: {} ({} graph error{})",
            loaded.manifest.project.name,
            errors,
            if errors == 1 { "" } else { "s" }
        );
        for diagnostic in &diagnostics {
            human.push_str(&format!(
                "\n  {} [{}] {}",
                diagnostic.level.to_ascii_uppercase(),
                diagnostic.code,
                diagnostic.message
            ));
        }
        return Ok(Output {
            human,
            json: json!({
                "ok": false,
                "project": loaded.manifest.project.name,
                "root": loaded.root,
                "preset": options.preset,
                "compiled": false,
                "diagnostics": diagnostics,
                "errors": errors,
                "warnings": warnings,
            }),
        });
    }

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for shader compilation")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_native_project(&loaded)?;
    runtime.load_project(&project)?;
    crate::uniforms::apply_to_runtime(
        &mut runtime,
        &crate::uniforms::defaults(&loaded.manifest.uniforms),
    )?;
    let sound_passes = super::sound::check_sound_passes(&context, &loaded)?;

    let mesh_passes = loaded
        .manifest
        .passes
        .iter()
        .filter_map(|pass| {
            pass.mesh.as_ref().map(|mesh| {
                json!({
                    "name": pass.name,
                    "grid_size": mesh.size,
                    "instances": mesh.instances,
                    "vertices": mesh.vertex_count(),
                    "triangles": mesh.triangle_count(),
                    "depth": mesh.depth,
                    "cull": format!("{:?}", mesh.cull).to_lowercase(),
                })
            })
        })
        .collect::<Vec<_>>();

    let pedantic_failed = options.pedantic && warnings != 0;
    let mut human = format!(
        "{}: {} ({} passes, {} assets",
        if pedantic_failed { "FAILED" } else { "OK" },
        loaded.manifest.project.name,
        loaded.manifest.passes.len(),
        loaded.manifest.assets.len()
    );
    if warnings != 0 {
        human.push_str(&format!(
            ", {warnings} warning{}",
            if warnings == 1 { "" } else { "s" }
        ));
    }
    human.push(')');
    for diagnostic in &diagnostics {
        human.push_str(&format!(
            "\n  {} [{}] {}",
            diagnostic.level.to_ascii_uppercase(),
            diagnostic.code,
            diagnostic.message
        ));
    }

    Ok(Output {
        human,
        json: json!({
            "ok": !pedantic_failed,
            "project": loaded.manifest.project.name,
            "root": loaded.root,
            "preset": options.preset,
            "passes": loaded.manifest.passes.len(),
            "assets": loaded.manifest.assets.len(),
            "compiled": true,
            "sound_passes": sound_passes,
            "mesh_passes": mesh_passes,
            "pedantic": options.pedantic,
            "diagnostics": diagnostics,
            "errors": errors,
            "warnings": warnings,
        }),
    })
}

pub fn build_project(path: &Path, output: Option<&Path>, preset: Option<&str>) -> Result<Output> {
    let loaded = LoadedManifest::load_with_preset(path, preset)?;
    ensure_source_files_exist(&loaded)?;
    if loaded
        .manifest
        .passes
        .iter()
        .any(|pass| pass.kind == PassKind::Sound)
    {
        bail!(
            "STTF build does not encode Sound passes; use render-audio or keep the directory project"
        );
    }
    if loaded
        .manifest
        .assets
        .iter()
        .any(|asset| asset.kind == crate::manifest::AssetKind::Video)
        || crate::media::manifest_uses_webcam(&loaded)
    {
        bail!(
            "STTF build does not encode dynamic video/webcam playback; keep the directory project for media inputs"
        );
    }
    let output = output.map(PathBuf::from).unwrap_or_else(|| {
        loaded.root.join("target").join(format!(
            "{}.sttf",
            file_safe_name(&loaded.manifest.project.name)
        ))
    });
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for shader compilation")?;
    let mut runtime = Runtime::new(&context)?;
    let project = build_native_project(&loaded)?;
    runtime.load_project(&project)?;
    crate::uniforms::apply_to_runtime(
        &mut runtime,
        &crate::uniforms::defaults(&loaded.manifest.uniforms),
    )?;
    runtime.save_sttf(&output)?;

    Ok(Output {
        human: format!("Built {}", output.display()),
        json: json!({
            "ok": true,
            "project": loaded.manifest.project.name,
            "artifact": output,
            "format": "sttf",
        }),
    })
}

fn template_name(template: Template) -> &'static str {
    match template {
        Template::Minimal => "minimal",
        Template::Multipass => "multipass",
    }
}

fn file_safe_name(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            result.push(character);
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "project".to_string()
    } else {
        result.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_safe_name_has_nonempty_fallback() {
        assert_eq!(file_safe_name("☃"), "project");
    }
}
