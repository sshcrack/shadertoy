use crate::manifest::{LoadedManifest, Pass};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_INCLUDE_DEPTH: usize = 64;

#[derive(Debug, Clone)]
pub struct ExpandedPassSource {
    pub text: String,
    pub vertex_text: Option<String>,
    pub dependencies: BTreeSet<PathBuf>,
}

pub type SourceGraph = BTreeMap<String, ExpandedPassSource>;

pub fn expand_all(loaded: &LoadedManifest) -> Result<SourceGraph> {
    let mut graph = BTreeMap::new();
    for pass in &loaded.manifest.passes {
        graph.insert(pass.name.clone(), expand_pass(loaded, pass)?);
    }
    Ok(graph)
}

pub fn expand_pass(loaded: &LoadedManifest, pass: &Pass) -> Result<ExpandedPassSource> {
    let include_dirs = loaded
        .manifest
        .shader
        .include_dirs
        .iter()
        .map(|dir| checked_dir(&loaded.root, &loaded.root.join(dir)))
        .collect::<Result<Vec<_>>>()?;

    let declarations = crate::uniforms::declarations(&loaded.manifest.uniforms);
    let mut dependencies = BTreeSet::new();
    let text = expand_stage(
        loaded,
        &pass.source,
        "shader source",
        &include_dirs,
        &declarations,
        &mut dependencies,
    )
    .with_context(|| format!("while expanding fragment stage for pass '{}'", pass.name))?;
    let vertex_text = pass
        .vertex
        .as_deref()
        .map(|vertex| {
            expand_stage(
                loaded,
                vertex,
                "vertex shader source",
                &include_dirs,
                &declarations,
                &mut dependencies,
            )
            .with_context(|| format!("while expanding vertex stage for pass '{}'", pass.name))
        })
        .transpose()?;

    Ok(ExpandedPassSource {
        text,
        vertex_text,
        dependencies,
    })
}

fn expand_stage(
    loaded: &LoadedManifest,
    relative: &str,
    label: &str,
    include_dirs: &[PathBuf],
    declarations: &str,
    dependencies: &mut BTreeSet<PathBuf>,
) -> Result<String> {
    let source = checked_file(&loaded.root, &loaded.root.join(relative), label)?;
    let mut stack = Vec::new();
    let expanded = expand_file(
        &loaded.root,
        &source,
        include_dirs,
        dependencies,
        &mut stack,
        0,
    )?;
    Ok(if declarations.is_empty() {
        expanded
    } else {
        format!("{declarations}#line 1\n{expanded}")
    })
}

fn expand_file(
    root: &Path,
    path: &Path,
    include_dirs: &[PathBuf],
    dependencies: &mut BTreeSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
    depth: usize,
) -> Result<String> {
    if depth > MAX_INCLUDE_DEPTH {
        bail!("GLSL include nesting exceeds {MAX_INCLUDE_DEPTH} levels");
    }
    let canonical = checked_file(root, path, "GLSL include")?;
    if let Some(index) = stack.iter().position(|candidate| candidate == &canonical) {
        let mut cycle = stack[index..]
            .iter()
            .map(|entry| display_relative(root, entry))
            .collect::<Vec<_>>();
        cycle.push(display_relative(root, &canonical));
        bail!("GLSL include cycle: {}", cycle.join(" -> "));
    }

    dependencies.insert(canonical.clone());
    stack.push(canonical.clone());
    let source = fs::read_to_string(&canonical)
        .with_context(|| format!("failed to read GLSL source {}", canonical.display()))?;

    let mut expanded = String::with_capacity(source.len());
    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        if let Some(include) = parse_include(line)? {
            let resolved =
                resolve_include(root, &canonical, include, include_dirs).with_context(|| {
                    format!(
                        "{}:{} includes {:?}",
                        display_relative(root, &canonical),
                        line_number,
                        include
                    )
                })?;
            expanded.push_str(&format!(
                "// shadertoy-cli include begin: {}\n#line 1\n",
                display_relative(root, &resolved)
            ));
            expanded.push_str(&expand_file(
                root,
                &resolved,
                include_dirs,
                dependencies,
                stack,
                depth + 1,
            )?);
            if !expanded.ends_with('\n') {
                expanded.push('\n');
            }
            expanded.push_str(&format!(
                "// shadertoy-cli include end: {}\n#line {}\n",
                display_relative(root, &resolved),
                line_number + 1
            ));
        } else {
            expanded.push_str(line);
            expanded.push('\n');
        }
    }
    stack.pop();
    Ok(expanded)
}

fn parse_include(line: &str) -> Result<Option<&str>> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return Ok(None);
    }
    let directive = trimmed[1..].trim_start();
    let Some(rest) = directive.strip_prefix("include") else {
        return Ok(None);
    };
    if rest
        .chars()
        .next()
        .is_some_and(|character| !character.is_whitespace() && character != '"')
    {
        return Ok(None);
    }
    let rest = rest.trim_start();
    if !rest.starts_with('"') {
        bail!("only quoted GLSL includes are supported; use #include \"path.glsl\"");
    }
    let Some(end) = rest[1..].find('"') else {
        bail!("unterminated GLSL #include path");
    };
    let include = &rest[1..end + 1];
    if include.is_empty() {
        bail!("GLSL #include path must not be empty");
    }
    let trailing = rest[end + 2..].trim();
    if !trailing.is_empty() && !trailing.starts_with("//") {
        bail!("unexpected tokens after GLSL #include");
    }
    Ok(Some(include))
}

fn resolve_include(
    root: &Path,
    including_file: &Path,
    include: &str,
    include_dirs: &[PathBuf],
) -> Result<PathBuf> {
    validate_include_path(include)?;
    let relative = Path::new(include);
    let mut candidates = Vec::with_capacity(include_dirs.len() + 1);
    if let Some(parent) = including_file.parent() {
        candidates.push(parent.join(relative));
    }
    candidates.extend(include_dirs.iter().map(|dir| dir.join(relative)));

    for candidate in candidates {
        if candidate.is_file() {
            return checked_file(root, &candidate, "GLSL include");
        }
    }
    bail!("GLSL include {:?} was not found", include)
}

fn validate_include_path(include: &str) -> Result<()> {
    let normalized = include.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("GLSL include path must stay within the project root");
    }
    Ok(())
}

fn checked_file(root: &Path, path: &Path, label: &str) -> Result<PathBuf> {
    if !path.is_file() {
        bail!("{label} does not exist at {}", path.display());
    }
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("failed to resolve project root {}", root.display()))?;
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to resolve {label} {}", path.display()))?;
    if !canonical.starts_with(&canonical_root) {
        bail!(
            "{label} resolves outside the project root: {}",
            path.display()
        );
    }
    Ok(canonical)
}

fn checked_dir(root: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_dir() {
        bail!(
            "shader include directory does not exist: {}",
            path.display()
        );
    }
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("failed to resolve project root {}", root.display()))?;
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to resolve include directory {}", path.display()))?;
    if !canonical.starts_with(&canonical_root) {
        bail!(
            "shader include directory resolves outside the project root: {}",
            path.display()
        );
    }
    Ok(canonical)
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::LoadedManifest;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, LoadedManifest) {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("shaders/lib")).unwrap();
        fs::write(
            dir.path().join("ShaderToy.toml"),
            r#"format = 1
[project]
name = "include-test"
[shader]
include_dirs = ["shaders/lib"]
[[pass]]
name = "image"
kind = "image"
source = "shaders/image.frag"
"#,
        )
        .unwrap();
        let loaded = LoadedManifest::load(dir.path()).unwrap();
        (dir, loaded)
    }

    #[test]
    fn expands_nested_includes_and_tracks_dependencies() {
        let (dir, loaded) = fixture();
        fs::write(
            dir.path().join("shaders/image.frag"),
            "#include \"lib/common.glsl\"\nvoid mainImage(out vec4 c, in vec2 p){ c=foo(); }\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("shaders/lib/common.glsl"),
            "#include \"math.glsl\"\nvec4 foo(){return vec4(one());}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("shaders/lib/math.glsl"),
            "float one(){return 1.0;}\n",
        )
        .unwrap();

        let pass = &loaded.manifest.passes[0];
        let expanded = expand_pass(&loaded, pass).unwrap();
        assert!(expanded.text.contains("float one()"));
        assert!(expanded.text.contains("vec4 foo()"));
        assert_eq!(expanded.dependencies.len(), 3);
    }

    #[test]
    fn rejects_include_cycles() {
        let (dir, loaded) = fixture();
        fs::write(
            dir.path().join("shaders/image.frag"),
            "#include \"lib/a.glsl\"\nvoid mainImage(out vec4 c, in vec2 p){c=vec4(1.0);}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("shaders/lib/a.glsl"),
            "#include \"b.glsl\"\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("shaders/lib/b.glsl"),
            "#include \"a.glsl\"\n",
        )
        .unwrap();

        let error = expand_pass(&loaded, &loaded.manifest.passes[0]).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("include cycle"));
    }
}
