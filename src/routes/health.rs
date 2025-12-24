use axum::{extract::State, response::IntoResponse};
use crate::state::AppState;

pub async fn health(
    State(_state): State<AppState>,
) -> impl IntoResponse {
    "ok"
}
