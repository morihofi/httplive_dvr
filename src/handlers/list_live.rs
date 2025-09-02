use axum::{Json, extract::State};
use tokio::fs;

use super::ListItem;
use crate::state::AppState;

pub async fn list_live(State(state): State<AppState>) -> Json<Vec<ListItem>> {
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
