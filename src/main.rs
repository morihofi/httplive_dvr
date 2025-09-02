use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path as AxPath, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    process::Command,
    sync::{Mutex, oneshot},
    time::{Duration, sleep},
};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing::{Level, error, info};

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    pending_dir: PathBuf,
    finished_dir: PathBuf,
    manager: Arc<RecordingManager>,
}

#[derive(Default)]
struct RecordingManager {
    // name -> stop channel
    inner: Mutex<HashMap<String, RecordingControl>>,
}

struct RecordingControl {
    stop: Option<oneshot::Sender<()>>,
}

impl RecordingManager {
    async fn start(&self, name: String, stop: oneshot::Sender<()>) -> Result<()> {
        let mut map = self.inner.lock().await;
        if map.contains_key(&name) {
            anyhow::bail!("Recording '{}' läuft bereits", name);
        }
        map.insert(name, RecordingControl { stop: Some(stop) });
        Ok(())
    }

    async fn stop(&self, name: &str) -> Result<()> {
        let mut map = self.inner.lock().await;
        if let Some(mut ctrl) = map.remove(name) {
            if let Some(tx) = ctrl.stop.take() {
                let _ = tx.send(());
            }
        }
        Ok(())
    }

    async fn finish(&self, name: &str) {
        let mut map = self.inner.lock().await;
        map.remove(name);
    }

    async fn is_running(&self, name: &str) -> bool {
        let map = self.inner.lock().await;
        map.contains_key(name)
    }
}

#[derive(Deserialize)]
struct StartReq {
    name: String,
    input_url: String,
    #[serde(default = "default_hls_time")]
    hls_time: u32,
}
fn default_hls_time() -> u32 {
    6
}

#[derive(Serialize)]
struct ListItem {
    name: String,
    playlist: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ffmpeg_dvr=info".parse()?)
                .add_directive("tower_http=info".parse()?),
        )
        .with_max_level(Level::INFO)
        .init();

    let root = std::env::current_dir()?;
    let pending_dir = root.join("pending_recordings");
    let finished_dir = root.join("finished_recordings");
    fs::create_dir_all(&pending_dir).await?;
    fs::create_dir_all(&finished_dir).await?;

    let state = AppState {
        root,
        pending_dir,
        finished_dir,
        manager: Arc::new(RecordingManager::default()),
    };

    let app = Router::new()
        // API
        .route("/api/start", post(api_start))
        .route("/api/stop/{name}", post(api_stop))
        .route("/api/finalize/{name}", post(api_finalize))
        .route("/api/live", get(api_list_live))
        .route("/api/finished", get(api_list_finished))
        // Static file serving
        .nest_service("/live", ServeDir::new(state.pending_dir.clone()))
        .nest_service("/vod", ServeDir::new(state.finished_dir.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], 8080));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("DVR HTTP-Server listening at http://{}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn api_start(State(state): State<AppState>, Json(req): Json<StartReq>) -> impl IntoResponse {
    match start_ffmpeg(&state, &req).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"started"})),
        )
            .into_response(),
        Err(e) => {
            error!(error=?e, "start_ffmpeg failed");
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
    }
}

async fn api_stop(
    State(state): State<AppState>,
    AxPath(name): AxPath<String>,
) -> impl IntoResponse {
    match state.manager.stop(&name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"stopped"})),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn api_list_live(State(state): State<AppState>) -> impl IntoResponse {
    let mut items = Vec::new();
    if let Ok(mut rd) = fs::read_dir(&state.pending_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("m3u8") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    items.push(ListItem {
                        name: stem.to_string(),
                        playlist: format!("/live/{}", p.file_name().unwrap().to_string_lossy()),
                    });
                }
            }
        }
    }
    Json(items)
}

async fn api_list_finished(State(state): State<AppState>) -> impl IntoResponse {
    let mut items = Vec::new();
    if let Ok(mut rd) = fs::read_dir(&state.finished_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let p = entry.path();
            // Wir legen je Aufnahme einen Unterordner an: finished_recordings/<name>/index.m3u8
            if p.is_dir() {
                let idx = p.join("index.m3u8");
                if idx.exists() {
                    if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                        items.push(ListItem {
                            name: name.to_string(),
                            playlist: format!("/vod/{}/index.m3u8", name),
                        });
                    }
                }
            }
        }
    }
    Json(items)
}

async fn api_finalize(
    State(state): State<AppState>,
    AxPath(name): AxPath<String>,
) -> impl IntoResponse {
    match finalize_to_vod(&state, &name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"finalized"})),
        )
            .into_response(),
        Err(e) => {
            error!(error=?e, "finalize failed");
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
    }
}

async fn start_ffmpeg(state: &AppState, req: &StartReq) -> Result<()> {
    // Falls noch läuft: Fehler werfen
    if state.manager.is_running(&req.name).await {
        anyhow::bail!("Recording '{}' is already running", req.name);
    }

    let playlist_name = req.name.clone();
    let input_url = req.input_url.clone();
    let hls_time = req.hls_time;
    let pending_dir = state.pending_dir.clone();
    let manager = state.manager.clone();

    let (stop_tx, mut stop_rx) = oneshot::channel();
    state.manager.start(playlist_name.clone(), stop_tx).await?;

    tokio::spawn(async move {
        loop {
            let playlist = pending_dir.join(format!("{}.m3u8", playlist_name));
            let seg_pattern =
                pending_dir.join(format!("{}_seg_%Y-%m-%d_%H-%M-%S_%03d.ts", playlist_name));

            let mut cmd = Command::new("ffmpeg");
            cmd.kill_on_drop(true)
                .arg("-y")
                .args(["-i", &input_url])
                .args(["-c", "copy"])
                .args(["-f", "hls"])
                .args(["-hls_time", &hls_time.to_string()])
                .args(["-hls_list_size", "0"])
                .args(["-hls_playlist_type", "event"])
                .args([
                    "-hls_flags",
                    "append_list+discont_start+program_date_time+temp_file",
                ])
                .args(["-strftime", "1"])
                .args(["-hls_segment_filename", &seg_pattern.to_string_lossy()])
                .arg(playlist.to_string_lossy().to_string());

            info!("Starte ffmpeg: {}", format_command(&cmd));

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    error!(error=?e, "ffmpeg could not be started");
                    break;
                }
            };

            let mut restart = false;
            tokio::select! {
                res = child.wait() => {
                    match res {
                        Ok(status) if status.success() => {
                            // normal beendet
                        }
                        Ok(_) => {
                            restart = true;
                        }
                        Err(e) => {
                            error!(error=?e, "ffmpeg wait fehlgeschlagen");
                        }
                    }
                }
                _ = &mut stop_rx => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            }

            if !restart {
                break;
            }
            info!("ffmpeg beendet - versuche Neustart in 3s");
            sleep(Duration::from_secs(3)).await;
        }

        manager.finish(&playlist_name).await;
    });

    Ok(())
}

fn format_command(cmd: &Command) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    s.push_str("ffmpeg ");
    if let Some(args) = cmd.as_std().get_args().next() {
        let _ = args;
    }
    // Tokio bietet keine direkte args()-Iteration, daher hacky:
    // Wir loggen nur, dass ffmpeg gestartet wurde. (Optional: selbst String bauen.)
    s
}

async fn finalize_to_vod(state: &AppState, name: &str) -> Result<()> {
    // 1) Falls aktiv: stoppen
    let _ = state.manager.stop(name).await;

    // 2) Event-Playlist lesen
    let src_pl = state.pending_dir.join(format!("{}.m3u8", name));
    if !src_pl.exists() {
        anyhow::bail!("Event-Playlist existiert nicht: {}", src_pl.display());
    }

    let content = fs::read_to_string(&src_pl).await?;
    let segments = extract_segment_list(&content);

    // 3) Zielordner vorbereiten
    let dst_dir = state.finished_dir.join(name);
    fs::create_dir_all(&dst_dir).await?;

    // 4) Segmente verschieben/kopieren & URIs anpassen
    //    (Wir kopieren, danach kann optional gelöscht werden.)
    for seg in &segments {
        let src = normalize_segment_path(&state.pending_dir, seg);
        let dst = dst_dir.join(Path::new(seg).file_name().unwrap());
        if let Err(e) = fs::copy(&src, &dst).await {
            error!(src=?src, dst=?dst, %e, "Segment-Kopie fehlgeschlagen");
            anyhow::bail!("Segment konnte nicht kopiert werden: {}", src.display());
        }
    }

    // 5) Playlist umschreiben: EVENT ➜ VOD, Basename-URIs, ENDLIST
    let vod = rewrite_playlist_to_vod(&content, &segments)?;
    let dst_pl = dst_dir.join("index.m3u8");
    fs::write(&dst_pl, vod.as_bytes()).await?;

    // 6) Optional: Pending-Dateien löschen (sicher nur, wenn du Speicher sparen willst)
    //    fs::remove_file(&src_pl).await.ok();
    //    for seg in &segments { let _ = fs::remove_file(normalize_segment_path(&state.pending_dir, seg)).await; }

    Ok(())
}

fn extract_segment_list(playlist: &str) -> Vec<String> {
    // Alles was keine '#' Kommentarzeile ist, gilt als URI; trimmen.
    playlist
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|s| s.to_string())
        .collect()
}

fn rewrite_playlist_to_vod(original: &str, segments: &Vec<String>) -> Result<String> {
    // Bewahre alle Metadaten-Zeilen, ersetze/füge PLAYLIST-TYPE:VOD, füge ENDLIST hinzu, ersetze Segment-URIs durch Basenames.
    let mut out = String::new();
    let mut has_header = false;
    let mut has_type = false;
    let mut has_endlist = false;

    for line in original.lines() {
        let l = line.trim_end();
        if l.starts_with("#EXTM3U") {
            has_header = true;
            out.push_str("#EXTM3U\n");
            continue;
        }
        if l.starts_with("#EXT-X-PLAYLIST-TYPE:") {
            has_type = true;
            out.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");
            continue;
        }
        if l.starts_with("#EXT-X-ENDLIST") {
            has_endlist = true;
        }
        // Alle anderen Zeilen (inkl. PROGRAM-DATE-TIME) unverändert übernehmen
        if l.starts_with('#') {
            out.push_str(l);
            out.push('\n');
        } else {
            // Segment-URI ➜ nur Basename
            let base = Path::new(l)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| l.to_string());
            out.push_str(&base);
            out.push('\n');
        }
    }

    if !has_header {
        out = format!("#EXTM3U\n{}", out);
    }
    if !has_type {
        out = out.replacen("#EXTM3U\n", "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n", 1);
    }
    if !has_endlist {
        out.push_str("#EXT-X-ENDLIST\n");
    }

    Ok(out)
}

fn normalize_segment_path(pending_dir: &Path, seg: &str) -> PathBuf {
    let p = Path::new(seg);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        pending_dir.join(p)
    }
}

// Optional: Beispiel eines einfachen Probe-Aufrufs via ffmpeg-next (nicht kritisch für DVR)
#[allow(dead_code)]
fn _probe_input(url: &str) -> Result<()> {
    // Warnung: erfordert korrekt installierte FFmpeg-Libs zur Buildzeit
    ffmpeg_next::format::network::init();
    let mut ictx =
        ffmpeg_next::format::input(&url).context("ffmpeg-next: input öffnen fehlgeschlagen")?;
    let _ = ictx.streams();
    Ok(())
}
