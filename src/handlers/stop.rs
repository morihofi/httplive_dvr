use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{recording::sanitize_name, state::AppState};

pub async fn stop(
    State(state): State<AppState>,
    Path(raw_name): Path<String>,
) -> impl IntoResponse {
    let name = match sanitize_name(&raw_name) {
        Ok(n) => n,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    match state.manager.stop(&name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"stopped"})),
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    }
}
