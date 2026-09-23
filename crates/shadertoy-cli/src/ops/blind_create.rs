use super::*;
use std::ffi::OsStr;
use std::process::Command;
use tempfile::TempDir;

const MAX_BLIND_SOURCES: usize = 64;
const MAX_BLIND_IMAGES_PER_SOURCE: usize = 128;

#[derive(Clone)]
pub(super) struct PreparedSource {
    pub(super) identity: String,
    pub(super) images: Vec<RgbImage>,
    pub(super) normalize_dimensions: bool,
}

pub(super) struct PathSourceSpec {
    pub(super) path: PathBuf,
    pub(super) preset: Option<String>,
    pub(super) explicit_project: bool,
}

pub(super) struct GitWorktree {
    repo_root: PathBuf,
    checkout: PathBuf,
    _parent: TempDir,
}

enum DirectorySource {
    Project(PathBuf),
    Sttf(PathBuf),
    Images(Vec<PathBuf>),
}

impl Drop for GitWorktree {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.repo_root)
            .args(["worktree", "remove", "--force"])
            .arg(&self.checkout)
            .status();
    }
}

pub fn create_blind_comparison(options: &BlindCreateOptions) -> Result<Output> {
    if !(2..=MAX_BLIND_SOURCES).contains(&options.sources.len()) {
        bail!(
            "blind create requires 2..={MAX_BLIND_SOURCES} sources (got {})",
            options.sources.len()
        );
    }
    let frames = normalize_frames(&options.frames)?;
    let output_dir = options
        .output_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/blind-comparison"));
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let git_root = if options
        .sources
        .iter()
        .any(|source| source.starts_with("git:"))
    {
        Some(resolve_git_root(options.git_root.as_deref())?)
    } else {
        None
    };
    let implicit_git_subdir = git_root
        .as_deref()
        .map(current_project_subdir)
        .transpose()?
        .flatten();

    let mut worktrees = Vec::new();
    let mut prepared = Vec::with_capacity(options.sources.len());
    for source in &options.sources {
        if source.starts_with("git:") {
            let root = git_root.as_deref().expect("git root resolved above");
            let (identity, project, worktree) =
                materialize_git_source(root, source, implicit_git_subdir.as_deref())?;
            let images = prepare_path_source(
                &project,
                &frames,
                options.width,
                options.height,
                options.fps,
                None,
                &options.set_uniforms,
            )?;
            prepared.push(PreparedSource {
                identity,
                images,
                normalize_dimensions: false,
            });
            worktrees.push(worktree);
        } else {
            let spec = parse_path_source(source)?;
            let images = prepare_path_source(
                &spec.path,
                &frames,
                options.width,
                options.height,
                options.fps,
                spec.preset.as_deref(),
                &options.set_uniforms,
            )?;
            prepared.push(PreparedSource {
                identity: source.clone(),
                images,
                normalize_dimensions: spec.explicit_project,
            });
        }
    }

    let image_count = prepared
        .first()
        .map(|source| source.images.len())
        .unwrap_or(0);
    if image_count == 0 {
        bail!("blind sources produced no images");
    }
    if image_count > MAX_BLIND_IMAGES_PER_SOURCE {
        bail!("blind source contains {image_count} images; limit is {MAX_BLIND_IMAGES_PER_SOURCE}");
    }
    for source in &prepared {
        if source.images.len() != image_count {
            bail!(
                "blind sources must contain the same number of images; '{}' has {}, expected {}",
                source.identity,
                source.images.len(),
                image_count
            );
        }
    }

    normalize_project_source_dimensions(&mut prepared)?;
    let width = prepared[0].images[0].width;
    let height = prepared[0].images[0].height;

    let entropy_context = format!(
        "external|{}|{}|{}|{}",
        output_dir.display(),
        width,
        height,
        prepared
            .iter()
            .map(|source| source.identity.as_str())
            .collect::<Vec<_>>()
            .join("|")
    );
    let plan = super::blind::BlindPlan::new(prepared.len(), &entropy_context)?;
    let contact_path = output_dir.join("blind-contact-sheet.png");
    let cell_count = prepared
        .len()
        .checked_mul(image_count)
        .context("blind contact-sheet cell count overflow")?;
    let mut sheet = render::prepare_contact_sheet(
        &contact_path,
        cell_count,
        Some(prepared.len() as u32),
        width,
        height,
    )?;

    let variants_dir = output_dir.join("variants");
    if variants_dir.exists() {
        fs::remove_dir_all(&variants_dir).with_context(|| {
            format!(
                "failed to clear stale blind outputs {}",
                variants_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&variants_dir)?;

    let mut outputs = Vec::with_capacity(prepared.len());
    let mut public_variants = Vec::with_capacity(prepared.len());
    for (position, original_index) in plan.order().iter().copied().enumerate() {
        let label = plan.label(position);
        let anonymous_dir = variants_dir.join(label);
        fs::create_dir_all(&anonymous_dir)?;
        for (image_index, image) in prepared[original_index].images.iter().enumerate() {
            let path = anonymous_dir.join(format!("image-{image_index:03}.png"));
            save_rgb_png(image, &path)?;
            sheet.blit(image_index * prepared.len() + position, image)?;
        }
        outputs.push(anonymous_dir.clone());
        public_variants.push(json!({
            "label": label,
            "output": anonymous_dir,
        }));
    }
    let contact_sheet = sheet.save()?;

    let variants = prepared
        .iter()
        .map(|source| vec![format!("source={}", source.identity)])
        .collect::<Vec<_>>();
    let session = super::blind::write_blind_session(
        &plan,
        &super::blind::BlindSessionSpec {
            output_dir: &output_dir,
            project: "external-comparison",
            frame: *frames.first().unwrap_or(&0),
            pass: "external",
            width,
            height,
            contact_sheet: &contact_sheet,
            outputs: &outputs,
            variants: &variants,
        },
    )?;

    drop(worktrees);

    Ok(Output {
        human: format!(
            "Created blinded comparison with {} sources x {} image(s) -> {}; inspect {} and record a judgment with 'shadertoy blind judge {} --pick LABEL --reason ...' before revealing",
            prepared.len(),
            image_count,
            output_dir.display(),
            contact_sheet.display(),
            session.display()
        ),
        json: json!({
            "ok": true,
            "mode": "external",
            "output_dir": output_dir,
            "contact_sheet": contact_sheet,
            "blind_session": session,
            "variant_count": prepared.len(),
            "images_per_variant": image_count,
            "width": width,
            "height": height,
            "set_uniforms": options.set_uniforms,
            "variants": public_variants,
        }),
    })
}

pub(super) fn parse_path_source(source: &str) -> Result<PathSourceSpec> {
    let Some(spec) = source.strip_prefix("project:") else {
        return Ok(PathSourceSpec {
            path: PathBuf::from(source),
            preset: None,
            explicit_project: false,
        });
    };
    if spec.is_empty() {
        bail!("project: blind source must include a project path");
    }
    let (path, preset) = match spec.rsplit_once("@preset=") {
        Some((path, preset)) => {
            if path.is_empty() || preset.trim().is_empty() {
                bail!("project blind source must use project:PATH@preset=NAME");
            }
            (path, Some(preset.trim().to_string()))
        }
        None => (spec, None),
    };
    Ok(PathSourceSpec {
        path: PathBuf::from(path),
        preset,
        explicit_project: true,
    })
}

pub(super) fn normalize_project_source_dimensions(prepared: &mut [PreparedSource]) -> Result<()> {
    let mut target = None::<(u32, u32, u64)>;
    let mut dimensions_differ = false;
    let first = (prepared[0].images[0].width, prepared[0].images[0].height);
    for source in prepared.iter() {
        for image in &source.images {
            let area = u64::from(image.width) * u64::from(image.height);
            if target.is_none_or(|(_, _, current)| area > current) {
                target = Some((image.width, image.height, area));
            }
            dimensions_differ |= (image.width, image.height) != first;
        }
    }
    if !dimensions_differ {
        return Ok(());
    }
    if !prepared.iter().all(|source| source.normalize_dimensions) {
        bail!(
            "all blinded images must have identical dimensions; use explicit project:PATH@preset=NAME sources to compare quality presets with different render scales"
        );
    }

    let (width, height, _) = target.expect("blind sources contain at least one image");
    for source in prepared.iter_mut() {
        for image in &mut source.images {
            if u64::from(image.width) * u64::from(height)
                != u64::from(image.height) * u64::from(width)
            {
                bail!(
                    "quality-preset blind sources must keep the same aspect ratio; '{}' contains {}x{}, target is {}x{}",
                    source.identity,
                    image.width,
                    image.height,
                    width,
                    height
                );
            }
            if image.width == width && image.height == height {
                continue;
            }
            let decoded =
                ::image::RgbImage::from_raw(image.width, image.height, image.pixels.clone())
                    .context("invalid RGB buffer while normalizing blind preset dimensions")?;
            let resized = ::image::imageops::resize(
                &decoded,
                width,
                height,
                ::image::imageops::FilterType::Triangle,
            );
            *image = RgbImage::new(width, height, resized.into_raw());
        }
    }
    Ok(())
}

pub(super) fn prepare_path_source(
    path: &Path,
    frames: &[i32],
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f32>,
    preset: Option<&str>,
    set_uniforms: &[String],
) -> Result<Vec<RgbImage>> {
    if path.is_file() {
        if preset.is_some() {
            bail!("blind @preset is only valid for ShaderToy project sources");
        }
        if is_image_path(path) {
            return Ok(vec![load_image(path)?]);
        }
        if path.extension().and_then(OsStr::to_str) == Some("sttf") {
            return render_sttf_frames(path, frames, width, height, fps, set_uniforms);
        }
        bail!(
            "unsupported blind source file {}; expected PNG/JPEG or .sttf",
            path.display()
        );
    }
    if !path.is_dir() {
        bail!("blind source does not exist: {}", path.display());
    }
    match classify_directory_source(path)? {
        DirectorySource::Project(project) => {
            render_project_frames(&project, frames, width, height, fps, preset, set_uniforms)
        }
        DirectorySource::Sttf(sttf) => {
            if preset.is_some() {
                bail!("blind @preset is only valid for ShaderToy project sources");
            }
            render_sttf_frames(&sttf, frames, width, height, fps, set_uniforms)
        }
        DirectorySource::Images(paths) => {
            if preset.is_some() {
                bail!("blind @preset is only valid for ShaderToy project sources");
            }
            paths.iter().map(|path| load_image(path)).collect()
        }
    }
}

fn classify_directory_source(root: &Path) -> Result<DirectorySource> {
    if root.join("ShaderToy.toml").is_file() {
        return Ok(DirectorySource::Project(root.to_path_buf()));
    }

    let mut projects = Vec::new();
    collect_named_files(root, "ShaderToy.toml", &mut projects)?;
    projects.sort();
    if projects.len() == 1 {
        return Ok(DirectorySource::Project(
            projects[0]
                .parent()
                .expect("ShaderToy.toml discovered below a directory")
                .to_path_buf(),
        ));
    }
    if projects.len() > 1 {
        bail!(
            "blind source directory {} contains multiple ShaderToy projects; choose one explicitly",
            root.display()
        );
    }

    let mut sttfs = Vec::new();
    collect_extension_files(root, "sttf", &mut sttfs)?;
    sttfs.sort();
    if sttfs.len() == 1 {
        return Ok(DirectorySource::Sttf(sttfs.remove(0)));
    }
    if sttfs.len() > 1 {
        bail!(
            "blind source directory {} contains multiple STTF builds; choose one explicitly",
            root.display()
        );
    }

    let mut images = Vec::new();
    collect_images(root, &mut images)?;
    images.sort();
    if images.is_empty() {
        bail!(
            "blind source directory {} contains no ShaderToy project, STTF build, or PNG/JPEG images",
            root.display()
        );
    }
    if images.len() > MAX_BLIND_IMAGES_PER_SOURCE {
        bail!(
            "blind source directory {} contains {} images; limit is {}",
            root.display(),
            images.len(),
            MAX_BLIND_IMAGES_PER_SOURCE
        );
    }
    Ok(DirectorySource::Images(images))
}

fn render_project_frames(
    project: &Path,
    frames: &[i32],
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f32>,
    preset: Option<&str>,
    set_uniforms: &[String],
) -> Result<Vec<RgbImage>> {
    let temp = tempfile::tempdir().context("failed to create temporary blind render directory")?;
    let output = render::render_frames_project(&RenderFramesOptions {
        project: project.to_path_buf(),
        preset: preset.map(str::to_owned),
        output_dir: Some(temp.path().to_path_buf()),
        contact_sheet: None,
        columns: None,
        pass: None,
        wireframe: None,
        width,
        height,
        fps,
        frames: frames.to_vec(),
        range: None,
        set_uniforms: set_uniforms.to_vec(),
    })?;
    let paths = output.json["outputs"]
        .as_array()
        .context("project blind render did not report outputs")?;
    paths
        .iter()
        .map(|path| {
            let path = path
                .as_str()
                .context("project blind render output path is not a string")?;
            load_image(Path::new(path))
        })
        .collect()
}

fn render_sttf_frames(
    path: &Path,
    frames: &[i32],
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f32>,
    set_uniforms: &[String],
) -> Result<Vec<RgbImage>> {
    let (default_width, default_height, default_fps) = sttf_render_defaults(path)?;
    let width = width.unwrap_or(default_width);
    let height = height.unwrap_or(default_height);
    let fps = fps.unwrap_or(default_fps);
    validate_dimensions(width, height)?;
    validate_fps(fps)?;
    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for STTF blind comparison")?;
    let mut runtime = Runtime::new(&context)?;
    runtime
        .load_sttf(path)
        .with_context(|| format!("failed to load STTF build {}", path.display()))?;
    apply_sttf_uniform_overrides(&mut runtime, path, set_uniforms)?;

    let mut images = Vec::with_capacity(frames.len());
    let mut next = 0usize;
    let max_frame = *frames.last().expect("normalized frames are non-empty");
    for frame in 0..=max_frame {
        if frame > 0 {
            runtime.tick_fixed(1.0 / fps, fps)?;
        }
        let image = runtime.render(width, height)?;
        if frame == frames[next] {
            images.push(image);
            next += 1;
            if next == frames.len() {
                break;
            }
        }
    }
    Ok(images)
}

fn sttf_render_defaults(path: &Path) -> Result<(u32, u32, f32)> {
    let document: serde_json::Value = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("failed to read STTF build {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse STTF build {}", path.display()))?;
    let metadata = document
        .get("metadata")
        .and_then(serde_json::Value::as_object)
        .context("STTF build metadata must be an object")?;

    let parse_u32 = |key: &str, fallback: u32| -> Result<u32> {
        let Some(value) = metadata.get(key) else {
            return Ok(fallback);
        };
        let text = value
            .as_str()
            .with_context(|| format!("STTF metadata '{key}' must be a string"))?;
        text.parse::<u32>()
            .with_context(|| format!("invalid STTF metadata '{key}' value '{text}'"))
    };
    let parse_f32 = |key: &str, fallback: f32| -> Result<f32> {
        let Some(value) = metadata.get(key) else {
            return Ok(fallback);
        };
        let text = value
            .as_str()
            .with_context(|| format!("STTF metadata '{key}' must be a string"))?;
        text.parse::<f32>()
            .with_context(|| format!("invalid STTF metadata '{key}' value '{text}'"))
    };

    Ok((
        parse_u32("RenderWidth", 1280)?,
        parse_u32("RenderHeight", 720)?,
        parse_f32("RenderFps", 60.0)?,
    ))
}

fn apply_sttf_uniform_overrides(
    runtime: &mut Runtime<'_>,
    path: &Path,
    assignments: &[String],
) -> Result<()> {
    if assignments.is_empty() {
        return Ok(());
    }

    let document: serde_json::Value = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("failed to read STTF build {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse STTF build {}", path.display()))?;
    let uniforms = document
        .get("uniforms")
        .and_then(serde_json::Value::as_object)
        .context("STTF build does not declare custom uniforms")?;

    for assignment in assignments {
        let Some((name, raw)) = assignment.split_once('=') else {
            bail!("uniform override '{assignment}' must use NAME=VALUE");
        };
        let encoded = uniforms
            .get(name)
            .with_context(|| format!("unknown custom uniform '{name}'"))?;
        let uniform_type = encoded
            .get("type")
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("STTF custom uniform '{name}' is missing its type"))?;
        match uniform_type {
            "Int" => {
                let value = match raw.trim() {
                    "true" => 1,
                    "false" => 0,
                    value => value.parse::<i32>().with_context(|| {
                        format!("invalid value for uniform '{name}': expected integer or boolean")
                    })?,
                };
                runtime.set_uniform_i32(name, value)?;
            }
            "Float" => {
                let value = raw.trim().parse::<f32>().with_context(|| {
                    format!("invalid value for uniform '{name}': expected float")
                })?;
                runtime.set_uniform_f32(name, &[value])?;
            }
            "Vec2" => apply_sttf_float_vector(runtime, name, raw, 2)?,
            "Vec3" => apply_sttf_float_vector(runtime, name, raw, 3)?,
            "Vec4" => apply_sttf_float_vector(runtime, name, raw, 4)?,
            other => bail!("unsupported STTF custom uniform type '{other}' for '{name}'"),
        }
    }
    Ok(())
}

fn apply_sttf_float_vector(
    runtime: &mut Runtime<'_>,
    name: &str,
    raw: &str,
    count: usize,
) -> Result<()> {
    let values = raw
        .split(',')
        .map(str::trim)
        .map(|value| value.parse::<f32>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| {
            format!("invalid value for uniform '{name}': expected {count} comma-separated floats")
        })?;
    if values.len() != count {
        bail!("invalid value for uniform '{name}': expected {count} comma-separated floats");
    }
    runtime.set_uniform_f32(name, &values)?;
    Ok(())
}

fn load_image(path: &Path) -> Result<RgbImage> {
    let image = ImageReader::open(path)
        .with_context(|| format!("failed to open blind image {}", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode blind image {}", path.display()))?
        .to_rgb8();
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    images::flip_rgb_rows(&mut pixels, width, height);
    Ok(RgbImage::new(width, height, pixels))
}

fn collect_images(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to read blind source directory {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_images(&path, out)?;
        } else if file_type.is_file() && is_image_path(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn collect_named_files(root: &Path, name: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to read blind source directory {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_named_files(&path, name, out)?;
        } else if file_type.is_file() && entry.file_name() == OsStr::new(name) {
            out.push(path);
        }
    }
    Ok(())
}

fn collect_extension_files(root: &Path, extension: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to read blind source directory {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_extension_files(&path, extension, out)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn is_image_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(OsStr::to_str)
            .map(|ext| ext.to_ascii_lowercase()),
        Some(ext) if matches!(ext.as_str(), "png" | "jpg" | "jpeg")
    )
}

pub(super) fn normalize_frames(frames: &[i32]) -> Result<Vec<i32>> {
    if frames.is_empty() {
        bail!("--frames must contain at least one frame");
    }
    let mut frames = frames.to_vec();
    if frames.iter().any(|frame| *frame < 0) {
        bail!("--frames values must be non-negative");
    }
    frames.sort_unstable();
    frames.dedup();
    if frames.len() > MAX_BLIND_IMAGES_PER_SOURCE {
        bail!(
            "--frames accepts at most {MAX_BLIND_IMAGES_PER_SOURCE} unique entries for blind comparisons"
        );
    }
    Ok(frames)
}

pub(super) fn resolve_git_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(root) = explicit {
        return Ok(root.to_path_buf());
    }
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("failed to run git while resolving blind git source")?;
    if !output.status.success() {
        bail!("git: blind sources require --git-root or running inside a Git repository");
    }
    let root = String::from_utf8(output.stdout).context("git root is not valid UTF-8")?;
    Ok(PathBuf::from(root.trim()))
}

pub(super) fn current_project_subdir(repo_root: &Path) -> Result<Option<PathBuf>> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    project_subdir_for_cwd(repo_root, &cwd)
}

fn project_subdir_for_cwd(repo_root: &Path, cwd: &Path) -> Result<Option<PathBuf>> {
    let repo_root = fs::canonicalize(repo_root)
        .with_context(|| format!("failed to resolve git root {}", repo_root.display()))?;
    let cwd = fs::canonicalize(cwd)
        .with_context(|| format!("failed to resolve current directory {}", cwd.display()))?;
    if !cwd.starts_with(&repo_root) {
        return Ok(None);
    }

    let mut cursor = cwd.as_path();
    loop {
        if cursor.join("ShaderToy.toml").is_file() {
            let relative = cursor
                .strip_prefix(&repo_root)
                .expect("cursor remains inside canonical git root");
            return Ok(Some(if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative.to_path_buf()
            }));
        }
        if cursor == repo_root {
            break;
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent;
    }
    Ok(None)
}

pub(super) fn materialize_git_source(
    repo_root: &Path,
    source: &str,
    implicit_subdir: Option<&Path>,
) -> Result<(String, PathBuf, GitWorktree)> {
    let spec = source
        .strip_prefix("git:")
        .context("internal blind git source parse error")?;
    let (git_ref, explicit_subdir) = match spec.split_once("::") {
        Some((git_ref, subdir)) => (git_ref, Some(Path::new(subdir))),
        None => (spec, None),
    };
    if git_ref.trim().is_empty() {
        bail!("git blind source must name a revision, e.g. git:HEAD::path/to/project");
    }
    let subdir = explicit_subdir
        .or(implicit_subdir)
        .unwrap_or_else(|| Path::new("."));
    if subdir.is_absolute()
        || subdir
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        bail!("git blind source subdirectory must stay inside the worktree");
    }

    let parent = tempfile::tempdir().context("failed to create temporary git worktree parent")?;
    let checkout = parent.path().join("checkout");
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["worktree", "add", "--detach", "--quiet"])
        .arg(&checkout)
        .arg(git_ref)
        .output()
        .with_context(|| format!("failed to run git worktree for '{source}'"))?;
    if !output.status.success() {
        bail!(
            "failed to materialize git revision '{}': {}",
            git_ref,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let project = checkout.join(subdir);
    if !project.exists() {
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["worktree", "remove", "--force"])
            .arg(&checkout)
            .status();
        bail!(
            "git blind source '{}' does not contain {}",
            git_ref,
            subdir.display()
        );
    }
    let identity = format!(
        "git:{}{}",
        git_ref,
        if subdir == Path::new(".") {
            String::new()
        } else {
            format!("::{}", subdir.display())
        }
    );
    Ok((
        identity,
        project,
        GitWorktree {
            repo_root: repo_root.to_path_buf(),
            checkout,
            _parent: parent,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn parses_project_preset_source() {
        let parsed = parse_path_source("project:.@preset=medium").unwrap();
        assert_eq!(parsed.path, PathBuf::from("."));
        assert_eq!(parsed.preset.as_deref(), Some("medium"));
        assert!(parsed.explicit_project);

        let plain = parse_path_source("renders/a.png").unwrap();
        assert_eq!(plain.path, PathBuf::from("renders/a.png"));
        assert!(plain.preset.is_none());
        assert!(!plain.explicit_project);
    }

    #[test]
    fn project_preset_sources_normalize_to_largest_resolution() {
        let mut prepared = vec![
            PreparedSource {
                identity: "project:.@preset=high".into(),
                images: vec![RgbImage::new(4, 2, vec![32; 4 * 2 * 3])],
                normalize_dimensions: true,
            },
            PreparedSource {
                identity: "project:.@preset=low".into(),
                images: vec![RgbImage::new(2, 1, vec![64; 2 * 3])],
                normalize_dimensions: true,
            },
        ];
        normalize_project_source_dimensions(&mut prepared).unwrap();
        assert!(
            prepared
                .iter()
                .all(|source| source.images[0].width == 4 && source.images[0].height == 2)
        );
    }

    #[test]
    fn directory_project_wins_over_texture_assets() {
        let root = tempfile::tempdir().unwrap();
        write_file(&root.path().join("ShaderToy.toml"), b"format = 1\n");
        write_file(&root.path().join("assets/foam.png"), b"not-an-image");

        match classify_directory_source(root.path()).unwrap() {
            DirectorySource::Project(project) => assert_eq!(project, root.path()),
            _ => panic!("ShaderToy project should win over image discovery"),
        }
    }

    #[test]
    fn unique_nested_project_wins_over_texture_assets() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("fable");
        write_file(&project.join("ShaderToy.toml"), b"format = 1\n");
        write_file(&project.join("assets/foam.png"), b"not-an-image");

        match classify_directory_source(root.path()).unwrap() {
            DirectorySource::Project(detected) => assert_eq!(detected, project),
            _ => panic!("nested ShaderToy project should win over image discovery"),
        }
    }

    #[test]
    fn unique_sttf_wins_over_image_directory() {
        let root = tempfile::tempdir().unwrap();
        let sttf = root.path().join("build.sttf");
        write_file(&sttf, b"{}");
        write_file(&root.path().join("foam.png"), b"not-an-image");

        match classify_directory_source(root.path()).unwrap() {
            DirectorySource::Sttf(detected) => assert_eq!(detected, sttf),
            _ => panic!("STTF should win over image discovery"),
        }
    }

    #[test]
    fn current_nested_project_becomes_implicit_git_subdir() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("fable");
        let child = project.join("assets");
        fs::create_dir_all(&child).unwrap();
        write_file(&project.join("ShaderToy.toml"), b"format = 1\n");

        assert_eq!(
            project_subdir_for_cwd(root.path(), &child).unwrap(),
            Some(PathBuf::from("fable"))
        );
    }
}
