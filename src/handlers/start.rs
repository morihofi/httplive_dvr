use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use tracing::error;

use crate::{
    recording::{StartReq, start_ffmpeg},
    state::AppState,
};

pub async fn start(State(state): State<AppState>, Json(req): Json<StartReq>) -> impl IntoResponse {
    match start_ffmpeg(&state, &req, false).await {
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
