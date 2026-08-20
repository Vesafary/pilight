//! Health, and the bulb-family catalogue.

use crate::app_state::AppState;
use crate::dto::{HealthResponse, LampTypeResponse};
use crate::error::ApiError;
use crate::response::ApiResponse;
use axum::extract::State;
use axum::{Json, Router, routing};
use pilight_db::repository::LampTypeRepository;
use pilight_proto::Transceiver;

/// Routes that need a token.
pub fn router<T: Transceiver + Send + 'static>() -> Router<AppState<T>> {
    Router::new().route("/lamp-types", routing::get(lamp_types))
}

/// Routes that do not.
pub fn health_router<T: Transceiver + Send + 'static>() -> Router<AppState<T>> {
    Router::new().route("/health", routing::get(health))
}

/// `GET /api/v1/lamp-types` — which bulb families exist, and which we can drive.
async fn lamp_types<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
) -> Result<Json<ApiResponse<Vec<LampTypeResponse>>>, ApiError> {
    let types = state
        .service
        .repositories()
        .types
        .find_all()
        .await?
        .into_iter()
        .map(LampTypeResponse::from)
        .collect();

    Ok(Json(ApiResponse::ok(types)))
}

/// `GET /health` — is the daemon well enough to be useful?
///
/// Always 200 with a body describing what is wrong, rather than a bare status
/// code: a monitor that only sees 503 cannot tell a dead database from a dead
/// process.
async fn health<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
) -> Json<ApiResponse<HealthResponse>> {
    let lamps = state.service.list().await;
    let database = lamps.is_ok();

    Json(ApiResponse::ok(HealthResponse {
        status: if database { "ok" } else { "degraded" }.to_owned(),
        database,
        lamps: lamps.map(|l| l.len()).unwrap_or_default(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }))
}
