use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::state::AppState;

pub async fn stop(State(state): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    match state.manager.stop(&name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"stopped"})),
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    }
}
