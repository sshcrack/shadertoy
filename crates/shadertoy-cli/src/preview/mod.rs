use crate::manifest::{LoadedManifest, PassKind};
use crate::project::{build_native_project_with_sources, ensure_source_files_exist};
use crate::replay::{ReplayAction, ReplayRecorder};
use crate::source::{SourceGraph, expand_pass};
use crate::uniforms::UniformValue;
use anyhow::{Context, Result, bail};
use axum::body::Bytes;
use axum::extract::{
    Query, State,
    ws::{Message, WebSocket, WebSocketUpgrade},
};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::StreamExt;
use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use shadertoy::{HeadlessContext, Runtime};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::IsTerminal;
use std::net::{IpAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, oneshot};

mod frame_encoder;
mod renderer;
mod state;
mod web;

use frame_encoder::{FrameEncoder, FrameSubmitter};
use renderer::render_loop;
use state::{relevant_watch_path, reload_event_kind};
use web::{frame_image, index, status, validate_remote_auth, websocket};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum PreviewTransport {
    Auto,
    Raw,
    Mjpeg,
    Png,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectivePreviewTransport {
    Raw,
    Mjpeg,
    Png,
}

impl EffectivePreviewTransport {
    fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Mjpeg => "mjpeg",
            Self::Png => "png",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::Raw => "application/octet-stream",
            Self::Mjpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }
}

impl PreviewTransport {
    fn resolve(self, host: &str) -> EffectivePreviewTransport {
        match self {
            Self::Auto if is_loopback_host(host) => EffectivePreviewTransport::Raw,
            Self::Auto => EffectivePreviewTransport::Mjpeg,
            Self::Raw => EffectivePreviewTransport::Raw,
            Self::Mjpeg => EffectivePreviewTransport::Mjpeg,
            Self::Png => EffectivePreviewTransport::Png,
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub struct PreviewConfig {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub host: String,
    pub port: u16,
    pub open: bool,
    pub no_open: bool,
    pub token: Option<String>,
    pub preserve_reload_state: bool,
    pub wireframe: Option<String>,
    pub record: Option<PathBuf>,
    pub transport: PreviewTransport,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewUniformStatus {
    pub name: String,
    pub kind: String,
    pub value: UniformValue,
    pub min: Option<UniformValue>,
    pub max: Option<UniformValue>,
    pub step: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewStatus {
    pub project: String,
    pub preset: Option<String>,
    pub presets: Vec<String>,
    pub sequence: u64,
    pub frame: i32,
    pub time: f32,
    pub paused: bool,
    pub time_scale: f32,
    pub width: u32,
    pub height: u32,
    pub preset_width: u32,
    pub preset_height: u32,
    pub custom_resolution: bool,
    pub fps: f32,
    pub view: String,
    pub final_pass: String,
    pub passes: Vec<String>,
    pub uniforms: Vec<PreviewUniformStatus>,
    pub webcam: bool,
    pub transport: String,
    pub error: Option<String>,
}

impl Default for PreviewStatus {
    fn default() -> Self {
        Self {
            project: String::new(),
            preset: None,
            presets: Vec::new(),
            sequence: 0,
            frame: 0,
            time: 0.0,
            paused: false,
            time_scale: 0.0,
            width: 1280,
            height: 720,
            preset_width: 1280,
            preset_height: 720,
            custom_resolution: false,
            fps: 60.0,
            view: "image".into(),
            final_pass: "image".into(),
            passes: Vec::new(),
            uniforms: Vec::new(),
            webcam: false,
            transport: "mjpeg".into(),
            error: None,
        }
    }
}

#[derive(Clone)]
struct Shared {
    frame_image: Arc<RwLock<Bytes>>,
    status: Arc<RwLock<PreviewStatus>>,
    controls: mpsc::Sender<Control>,
    updates: broadcast::Sender<String>,
    frames: broadcast::Sender<Bytes>,
    clients: Arc<AtomicUsize>,
    token: Option<Arc<String>>,
    frame_submitter: FrameSubmitter,
    transport: EffectivePreviewTransport,
}

#[derive(Debug)]
enum Control {
    FilesChanged(Vec<PathBuf>),
    Pause,
    Resume,
    Reset,
    Step,
    Preset(Option<String>),
    View(String),
    Resolution(u32, u32),
    ResolutionDefault,
    TimeScale(f32),
    Uniform {
        name: String,
        value: UniformValue,
    },
    WebcamFrame(Vec<u8>),
    Mouse {
        x: f32,
        y: f32,
        down: bool,
        clicked: bool,
    },
    Key {
        code: u8,
        down: bool,
        pressed: bool,
    },
    Shutdown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum BrowserControl {
    Pause,
    Resume,
    Reset,
    Step,
    Preset {
        preset: Option<String>,
    },
    View {
        pass: String,
    },
    Resolution {
        width: u32,
        height: u32,
    },
    ResolutionDefault,
    TimeScale {
        value: f32,
    },
    Uniform {
        name: String,
        value: UniformValue,
    },
    Mouse {
        x: f32,
        y: f32,
        down: bool,
        #[serde(default)]
        clicked: bool,
    },
    Key {
        code: u8,
        down: bool,
        #[serde(default)]
        pressed: bool,
    },
}

#[derive(Debug, Deserialize, Default)]
struct AuthQuery {
    token: Option<String>,
}

struct SavedRuntimeState {
    time: f32,
    frame: i32,
    fps: f32,
    buffers: BTreeMap<String, (crate::state::BufferDimensions, Vec<f32>)>,
}
const MAX_PREVIEW_DIMENSION: u32 = 4096;

pub fn run(config: PreviewConfig, json_mode: bool) -> Result<()> {
    if config.open && config.no_open {
        bail!("--open and --no-open are mutually exclusive");
    }
    validate_remote_auth(&config)?;
    let transport = config.transport.resolve(&config.host);

    let loaded = LoadedManifest::load_with_preset(&config.project, config.preset.as_deref())?;
    validate_preview_dimensions(&loaded)?;
    let base_loaded = if config.preset.is_some() {
        Some(LoadedManifest::load_with_preset(&config.project, None)?)
    } else {
        None
    };
    let base_manifest = base_loaded
        .as_ref()
        .map(|base| &base.manifest)
        .unwrap_or(&loaded.manifest);
    if config.record.is_some() && crate::media::manifest_uses_webcam(&loaded) {
        bail!(
            "preview --record does not support live webcam input; use a deterministic video asset instead"
        );
    }
    let root = loaded.root.clone();
    let recorder = config
        .record
        .clone()
        .map(|path| ReplayRecorder::create(path, &loaded))
        .transpose()?;
    let initial_status = PreviewStatus {
        project: loaded.manifest.project.name.clone(),
        preset: config.preset.clone(),
        presets: loaded.manifest.presets.keys().cloned().collect(),
        width: base_manifest.render.width,
        height: base_manifest.render.height,
        preset_width: loaded.manifest.render.width,
        preset_height: loaded.manifest.render.height,
        custom_resolution: false,
        fps: loaded.manifest.render.fps,
        final_pass: loaded.manifest.final_pass().name.clone(),
        view: loaded.manifest.final_pass().name.clone(),
        passes: loaded
            .manifest
            .passes
            .iter()
            .filter(|pass| !matches!(pass.kind, PassKind::Cubemap | PassKind::Sound))
            .map(|pass| pass.name.clone())
            .collect(),
        uniforms: loaded
            .manifest
            .uniforms
            .iter()
            .map(|definition| PreviewUniformStatus {
                name: definition.name().to_string(),
                kind: definition.kind_name().to_string(),
                value: definition.default_value(),
                min: definition.preview_min(),
                max: definition.preview_max(),
                step: definition.preview_step(),
            })
            .collect(),
        webcam: crate::media::manifest_uses_webcam(&loaded),
        transport: transport.as_str().into(),
        ..PreviewStatus::default()
    };

    let (control_tx, control_rx) = mpsc::channel();
    let (update_tx, _) = broadcast::channel(128);
    let (frame_tx, _) = broadcast::channel(2);
    let current_frame = Arc::new(RwLock::new(Bytes::new()));
    let current_status = Arc::new(RwLock::new(initial_status));
    let frame_encoder = FrameEncoder::start(
        current_frame.clone(),
        current_status.clone(),
        update_tx.clone(),
        frame_tx.clone(),
        transport,
    )?;
    let shared = Shared {
        frame_image: current_frame,
        status: current_status,
        controls: control_tx.clone(),
        updates: update_tx.clone(),
        frames: frame_tx,
        clients: Arc::new(AtomicUsize::new(0)),
        token: config.token.clone().map(Arc::new),
        frame_submitter: frame_encoder.submitter(),
        transport,
    };

    let watcher_tx = control_tx.clone();
    let watch_root = root.clone();
    let ignored_record_paths = config
        .record
        .as_ref()
        .and_then(|path| fs::canonicalize(path).ok())
        .map(|path| vec![path.clone(), path.with_extension("strec.tmp")])
        .unwrap_or_default();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else {
            return;
        };
        if reload_event_kind(&event.kind) {
            let paths = event
                .paths
                .into_iter()
                .filter(|path| relevant_watch_path(&watch_root, path))
                .filter(|path| !ignored_record_paths.iter().any(|ignored| ignored == path))
                .collect::<Vec<_>>();
            if !paths.is_empty() {
                let _ = watcher_tx.send(Control::FilesChanged(paths));
            }
        }
    })
    .context("failed to create project file watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {}", root.display()))?;

    let listener = TcpListener::bind((config.host.as_str(), config.port)).with_context(|| {
        format!(
            "failed to bind preview server on {}:{}",
            config.host, config.port
        )
    })?;
    listener
        .set_nonblocking(true)
        .context("failed to configure preview server socket")?;
    let address = listener.local_addr()?;

    // Keep the graphics context/runtime on the process main thread; the native
    // context helper is thread-affine. Move only the HTTP/WebSocket server to a worker.
    let context = HeadlessContext::new(64, 64)
        .context("failed to create headless OpenGL context for preview")?;
    let mut runtime =
        Runtime::new(&context).context("failed to create native ShaderToy preview runtime")?;

    let app = Router::new()
        .route("/", get(index))
        .route("/frame", get(frame_image))
        .route("/frame.raw", get(frame_image))
        .route("/frame.jpg", get(frame_image))
        .route("/frame.png", get(frame_image))
        .route("/api/status", get(status))
        .route("/ws", get(websocket))
        .with_state(shared.clone());

    let url = preview_url(&config.host, address.port(), config.token.as_deref());
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel();
    let server_controls = control_tx.clone();
    let server_thread = thread::Builder::new()
        .name("shadertoy-preview-http".into())
        .spawn(move || {
            let result = (|| -> Result<()> {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("failed to create preview HTTP runtime")?;
                runtime.block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(listener)
                        .context("failed to adopt preview server socket")?;
                    axum::serve(listener, app)
                        .with_graceful_shutdown(async move {
                            tokio::select! {
                                _ = tokio::signal::ctrl_c() => {}
                                _ = server_shutdown_rx => {}
                            }
                        })
                        .await
                        .context("preview server failed")
                })
            })();
            let _ = server_controls.send(Control::Shutdown);
            result
        })
        .context("failed to start preview HTTP thread")?;

    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": true,
                "action": "preview",
                "url": url,
                "host": config.host,
                "port": address.port(),
                "project": root,
            }))?
        );
    } else {
        println!("ShaderToy preview: {url}");
        println!("Watching {}", root.display());
    }

    let should_open = config.open
        || (!config.no_open && std::io::stdout().is_terminal() && config.host != "0.0.0.0");
    if should_open && let Err(error) = open::that(&url) {
        eprintln!("warning: could not open browser: {error}");
    }

    render_loop(
        root,
        config.preset,
        shared,
        control_rx,
        config.preserve_reload_state,
        config.wireframe,
        &mut runtime,
        recorder,
    );
    drop(frame_encoder);

    let _ = server_shutdown_tx.send(());
    drop(watcher);
    match server_thread.join() {
        Ok(result) => result,
        Err(_) => bail!("preview HTTP thread panicked"),
    }
}

fn validate_preview_dimensions(loaded: &LoadedManifest) -> Result<()> {
    let width = loaded.manifest.render.width;
    let height = loaded.manifest.render.height;
    if width > MAX_PREVIEW_DIMENSION || height > MAX_PREVIEW_DIMENSION {
        bail!(
            "preview resolution must be between 1x1 and {0}x{0}; manifest requests {width}x{height}",
            MAX_PREVIEW_DIMENSION
        );
    }
    Ok(())
}

fn preview_url(host: &str, port: u16, token: Option<&str>) -> String {
    let display_host = match host {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        other => other,
    };
    let authority_host = match display_host.parse::<IpAddr>() {
        Ok(IpAddr::V6(_)) => format!("[{display_host}]"),
        _ => display_host.to_string(),
    };
    let query = token
        .map(|token| format!("?token={}", percent_encode_query(token)))
        .unwrap_or_default();
    format!("http://{authority_host}:{port}{query}")
}

fn percent_encode_query(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_url_brackets_ipv6_and_encodes_token() {
        assert_eq!(
            preview_url("::1", 4321, Some("a b&c")),
            "http://[::1]:4321?token=a%20b%26c"
        );
        assert_eq!(preview_url("::", 4321, None), "http://[::1]:4321");
    }

    #[test]
    fn auto_transport_uses_raw_only_for_loopback_bindings() {
        for host in ["localhost", "127.0.0.1", "::1"] {
            assert_eq!(
                PreviewTransport::Auto.resolve(host),
                EffectivePreviewTransport::Raw,
                "{host}"
            );
        }
        for host in ["0.0.0.0", "::", "192.168.1.20"] {
            assert_eq!(
                PreviewTransport::Auto.resolve(host),
                EffectivePreviewTransport::Mjpeg,
                "{host}"
            );
        }
        assert_eq!(
            PreviewTransport::Raw.resolve("0.0.0.0"),
            EffectivePreviewTransport::Raw
        );
        assert_eq!(
            PreviewTransport::Png.resolve("127.0.0.1"),
            EffectivePreviewTransport::Png
        );
        assert_eq!(
            PreviewTransport::Mjpeg.resolve("127.0.0.1"),
            EffectivePreviewTransport::Mjpeg
        );
    }
}
