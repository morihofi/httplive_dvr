use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    process::Command,
    sync::oneshot,
    time::{Duration, sleep},
};
use tracing::{error, info};

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

pub async fn start_ffmpeg(state: &AppState, req: &StartReq) -> Result<()> {
    // If already running: return error
    if state.manager.is_running(&req.name).await {
        anyhow::bail!("Recording '{}' is already running", req.name);
    }

    // Avoid collisions with existing playlists
    let pending_pl = state.pending_dir.join(format!("{}.m3u8", req.name));
    let finished_pl = state.finished_dir.join(&req.name).join("index.m3u8");
    if fs::metadata(&pending_pl).await.is_ok() || fs::metadata(&finished_pl).await.is_ok() {
        anyhow::bail!("Recording '{}' already exists", req.name);
    }

    let playlist_name = req.name.clone();
    let input_url = req.input_url.clone();
    let hls_time = req.hls_time;
    let pending_dir = state.pending_dir.clone();
    let manager = state.manager.clone();

    let (stop_tx, mut stop_rx) = oneshot::channel();
    state.manager.start(req.clone(), stop_tx).await?;

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
    let mut s = String::new();
    s.push_str("ffmpeg ");
    if let Some(args) = cmd.as_std().get_args().next() {
        let _ = args;
    }
    // Tokio does not provide direct args() iteration, so this is minimal.
    // We only log that ffmpeg was started. (Optionally build the string manually.)
    s
}

pub async fn finalize_to_vod(state: &AppState, name: &str) -> Result<()> {
    // 1) stop recording if active
    let _ = state.manager.stop(name).await;

    // 2) read event playlist
    let src_pl = state.pending_dir.join(format!("{}.m3u8", name));
    if !src_pl.exists() {
        anyhow::bail!("Event playlist does not exist: {}", src_pl.display());
    }

    let content = fs::read_to_string(&src_pl).await?;
    let segments = extract_segment_list(&content);

    // 3) prepare destination directory
    let dst_dir = state.finished_dir.join(name);
    let dst_pl = dst_dir.join("index.m3u8");
    if fs::metadata(&dst_pl).await.is_ok() {
        anyhow::bail!("Recording '{}' already finalized", name);
    }
    fs::create_dir_all(&dst_dir).await?;

    // 4) copy segments and adjust URIs
    for seg in &segments {
        let src = normalize_segment_path(&state.pending_dir, seg);
        let dst = dst_dir.join(Path::new(seg).file_name().unwrap());
        if let Err(e) = fs::copy(&src, &dst).await {
            error!(src=?src, dst=?dst, %e, "segment copy failed");
            anyhow::bail!("Could not copy segment: {}", src.display());
        }
    }

    // 5) rewrite playlist: EVENT -> VOD, basename URIs, ENDLIST
    let vod = rewrite_playlist_to_vod(&content)?;
    fs::write(&dst_pl, vod.as_bytes()).await?;

    // 6) optional: remove pending files (only if saving space)
    // fs::remove_file(&src_pl).await.ok();
    // for seg in &segments { let _ = fs::remove_file(normalize_segment_path(&state.pending_dir, seg)).await; }

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

fn normalize_segment_path(pending_dir: &Path, seg: &str) -> PathBuf {
    let p = Path::new(seg);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        pending_dir.join(p)
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
