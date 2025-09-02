use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use tracing::error;

use crate::{recording::finalize_to_vod, state::AppState};

pub async fn finalize(
    State(state): State<AppState>,
    Path(name): Path<String>,
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
