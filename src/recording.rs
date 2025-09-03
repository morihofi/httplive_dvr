use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    process::Command,
    sync::oneshot,
    time::{Duration, sleep},
};
use tracing::{debug, error, info};

use crate::state::AppState;

#[derive(Clone, Serialize, Deserialize)]
pub struct StartReq {
    pub name: String,
    pub input_url: String,
    #[serde(default = "default_hls_time")]
    pub hls_time: u32,
}

fn default_hls_time() -> u32 {
    6
}

pub fn sanitize_name(name: &str) -> Result<String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!("invalid name: {}", name);
    }
    Ok(name.to_string())
}

pub async fn start_ffmpeg(state: &AppState, req: &StartReq, allow_existing: bool) -> Result<()> {
    let name = sanitize_name(&req.name)?;

    // If already running: return error
    if state.manager.is_running(&name).await {
        anyhow::bail!("Recording '{}' is already running", name);
    }

    // Avoid collisions with existing playlists when creating new jobs via API.
    // Resumed recordings may already have on-disk state; in that case we allow it.
    if !allow_existing {
        let pending_pl = state.pending_dir.join(format!("{}.m3u8", name));
        let finished_pl = state.finished_dir.join(&name).join("index.m3u8");
        if fs::metadata(&pending_pl).await.is_ok() || fs::metadata(&finished_pl).await.is_ok() {
            anyhow::bail!("Recording '{}' already exists", name);
        }
    }

    let playlist_name = name.clone();
    let input_url = req.input_url.clone();
    let hls_time = req.hls_time;
    let pending_dir = state.pending_dir.clone();
    let manager = state.manager.clone();

    let (stop_tx, mut stop_rx) = oneshot::channel();
    let sanitized_req = StartReq {
        name: name.clone(),
        input_url: req.input_url.clone(),
        hls_time: req.hls_time,
    };
    state.manager.start(sanitized_req, stop_tx).await?;

    tokio::spawn(async move {
        loop {
            let playlist = pending_dir.join(format!("{}.m3u8", playlist_name));
            let seg_pattern =
                pending_dir.join(format!("{}_seg_%Y-%m-%d_%H-%M-%S_%03d.ts", playlist_name));

            let mut cmd = Command::new("ffmpeg");
            cmd.kill_on_drop(true)
                .arg("-y")
                //.args(["-rtsp_transport", "tcp"])
                .arg("-re")
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

            info!("Starting ffmpeg: {}", format_command(&cmd));

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
                            // finished normally
                        }
                        Ok(_) => {
                            restart = true;
                        }
                        Err(e) => {
                            error!(error=?e, "ffmpeg wait failed");
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
            info!("ffmpeg exited - retrying in 3s");
            sleep(Duration::from_secs(3)).await;
        }

        manager.finish(&playlist_name).await;
    });

    Ok(())
}

fn format_command(cmd: &Command) -> String {
    let mut s = String::from("ffmpeg");
    for arg in cmd.as_std().get_args() {
        s.push(' ');
        s.push_str(&arg.to_string_lossy());
    }
    s
}

pub async fn finalize_to_vod(state: &AppState, name: &str) -> Result<()> {
    let name = sanitize_name(name)?;

    // 1) stop recording if active
    let _ = state.manager.stop(&name).await;

    // 2) read event playlist
    let src_pl = state.pending_dir.join(format!("{}.m3u8", name));
    if !src_pl.exists() {
        anyhow::bail!("Event playlist does not exist: {}", src_pl.display());
    }

    let content = fs::read_to_string(&src_pl).await?;
    let segments = extract_segment_list(&content);

    // 3) prepare destination directory
    let dst_dir = state.finished_dir.join(&name);
    let dst_pl = dst_dir.join("index.m3u8");
    if fs::metadata(&dst_pl).await.is_ok() {
        anyhow::bail!("Recording '{}' already finalized", name);
    }
    fs::create_dir_all(&dst_dir).await?;

    // 4) move segments without duplication and adjust URIs
    info!(%name, total_segments=segments.len(), "finalizing recording - moving segments");
    for seg in &segments {
        let src = normalize_segment_path(&state.pending_dir, seg)?;
        let dst = dst_dir.join(Path::new(seg).file_name().unwrap());
        if fs::metadata(&dst).await.is_ok() {
            debug!(dst=?dst, "segment already moved, skipping");
            continue;
        }
        debug!(src=?src, dst=?dst, "moving segment");
        match fs::rename(&src, &dst).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
                // Different filesystem: try hard link + remove
                if let Err(e2) = fs::hard_link(&src, &dst).await {
                    error!(src=?src, dst=?dst, error=?e2, "segment move failed");
                    anyhow::bail!("Could not move segment: {}", src.display());
                }
                fs::remove_file(&src).await.ok();
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound && fs::metadata(&dst).await.is_ok() {
                    debug!(dst=?dst, "segment already moved, skipping");
                    continue;
                }
                error!(src=?src, dst=?dst, error=?e, "segment move failed");
                anyhow::bail!("Could not move segment: {}", src.display());
            }
        }
    }

    // 5) rewrite playlist: EVENT -> VOD, basename URIs, ENDLIST
    let vod = rewrite_playlist_to_vod(&content)?;
    fs::write(&dst_pl, vod.as_bytes()).await?;
    info!(playlist=?dst_pl, "VOD playlist written");

    // 6) remove pending playlist to save space
    if let Err(e) = fs::remove_file(&src_pl).await {
        error!(file=?src_pl, error=?e, "failed to remove pending playlist");
    }

    info!(%name, "recording finalized");
    Ok(())
}

fn extract_segment_list(playlist: &str) -> Vec<String> {
    // Every non-comment, non-empty line is considered a URI
    playlist
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|s| s.to_string())
        .collect()
}

fn rewrite_playlist_to_vod(original: &str) -> Result<String> {
    // Keep metadata lines, replace or insert PLAYLIST-TYPE:VOD, add ENDLIST, replace segment URIs with basenames
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
        // Keep other lines (including PROGRAM-DATE-TIME) as-is
        if l.starts_with('#') {
            out.push_str(l);
            out.push('\n');
        } else {
            // Segment URI -> basename only
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

fn normalize_segment_path(pending_dir: &Path, seg: &str) -> Result<PathBuf> {
    let p = Path::new(seg);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        pending_dir.join(p)
    };

    let base = std::fs::canonicalize(pending_dir).with_context(|| {
        format!(
            "failed to canonicalize pending dir {}",
            pending_dir.display()
        )
    })?;
    let canon = std::fs::canonicalize(&joined)
        .with_context(|| format!("failed to canonicalize segment path {}", joined.display()))?;

    if canon.starts_with(&base) {
        Ok(canon)
    } else {
        anyhow::bail!(
            "segment path {} escapes pending directory",
            joined.display()
        );
    }
}

// Example of a simple probe call via ffmpeg-next (not critical for DVR)
#[allow(dead_code)]
pub fn _probe_input(url: &str) -> Result<()> {
    // Warning: requires correctly installed FFmpeg libs at build time
    ffmpeg_next::format::network::init();
    let ictx = ffmpeg_next::format::input(&url).context("ffmpeg-next: opening input failed")?;
    let _ = ictx.streams();
    Ok(())
}
