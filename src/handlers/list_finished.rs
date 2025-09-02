use axum::{Json, extract::State};
use tokio::fs;

use super::ListItem;
use crate::state::AppState;

pub async fn list_finished(State(state): State<AppState>) -> Json<Vec<ListItem>> {
    let mut items = Vec::new();
    if let Ok(mut rd) = fs::read_dir(&state.finished_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let p = entry.path();
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
