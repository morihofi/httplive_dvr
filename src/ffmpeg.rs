use anyhow::{Context, Result};
use tokio::process::Command;

fn has_word(output: &str, word: &str) -> bool {
    output.lines()
        .any(|l| l.split_whitespace().any(|tok| tok == word))
}


pub async fn check_ffmpeg() -> Result<()> {
    let proto = Command::new("ffmpeg")
        .arg("-protocols")
        .output()
        .await
        .context("failed to run ffmpeg -protocols")?;
    let list = String::from_utf8_lossy(&proto.stdout);
    for p in ["https", "tls"] {
        if !has_word(&list, p) {
            anyhow::bail!("ffmpeg missing required protocol: {}", p);
        }
    }

    let mux = Command::new("ffmpeg")
        .arg("-muxers")
        .output()
        .await
        .context("failed to run ffmpeg -muxers")?;
    let muxers = String::from_utf8_lossy(&mux.stdout);
    for m in ["hls", "flv"] {
        if !has_word(&muxers, m) {
            anyhow::bail!("ffmpeg missing required muxer: {}", m);
        }
    }
    Ok(())
}
