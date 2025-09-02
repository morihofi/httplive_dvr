use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    Router,
    routing::{get, post},
};
use clap::Parser;
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing::{Level, error, info};

mod ffmpeg;
mod handlers;
mod recording;
mod state;

use handlers::{finalize, list_finished, list_live, start, stop};
use recording::start_ffmpeg;
use state::{AppState, RecordingManager};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Base directory for DVR files
    #[arg(long, env = "HTTPLIVE_BASE_DIR", default_value = ".")]
    base_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ffmpeg_dvr=info".parse()?)
                .add_directive("tower_http=info".parse()?),
        )
        .with_max_level(Level::INFO)
        .init();

    let args = Cli::parse();
    let root = if args.base_dir.is_absolute() {
        args.base_dir
    } else {
        std::env::current_dir()?.join(args.base_dir)
    };
    tokio::fs::create_dir_all(&root).await?;
    let pending_dir = root.join("pending_recordings");
    let finished_dir = root.join("finished_recordings");
    tokio::fs::create_dir_all(&pending_dir).await?;
    tokio::fs::create_dir_all(&finished_dir).await?;

    let manager = Arc::new(RecordingManager::new(root.join("active_recordings.json")));
    let state = AppState {
        pending_dir: pending_dir.clone(),
        finished_dir: finished_dir.clone(),
        manager: manager.clone(),
    };

    ffmpeg::check_ffmpeg().await?;
    info!("Self test with ffmpeg completed successfully");

    let existing = manager.load().await?;
    for req in existing {
        if let Err(e) = start_ffmpeg(&state, &req).await {
            error!(error=?e, name=%req.name, "failed to resume recording");
        }
    }

    //
    // API-Server (Steuerung)
    //
    let api_app = Router::new()
        .route("/api/start", post(start))
        .route("/api/stop/{name}", post(stop))
        .route("/api/finalize/{name}", post(finalize))
        .route("/api/live", get(list_live))
        .route("/api/finished", get(list_finished))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    //
    // VOD/Recording-Server (host only files)
    //
    let vod_app = Router::new()
        .nest_service("/live", ServeDir::new(pending_dir))
        .nest_service("/vod", ServeDir::new(finished_dir))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    //
    // Listener parallel starten
    //
    let api_addr: SocketAddr = ([0, 0, 0, 0], 8080).into();
    let vod_addr: SocketAddr = ([0, 0, 0, 0], 8081).into();

    let api_listener = tokio::net::TcpListener::bind(api_addr).await?;
    let vod_listener = tokio::net::TcpListener::bind(vod_addr).await?;

    info!("API server listening at http://{}", api_addr);
    info!("VOD server listening at http://{}", vod_addr);

    tokio::try_join!(
        axum::serve(api_listener, api_app),
        axum::serve(vod_listener, vod_app),
    )?;

    Ok(())
}
