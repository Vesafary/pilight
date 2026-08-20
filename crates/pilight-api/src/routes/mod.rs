//! The router.

mod lamps;
mod meta;

use crate::app_state::AppState;
use crate::auth::{ApiToken, require_token};
use axum::{Router, middleware};
use pilight_proto::Transceiver;
use pilight_service::LampService;
use tower_http::normalize_path::NormalizePathLayer;
use tower_http::trace::TraceLayer;

/// The version prefix every resource lives under.
pub const API_PREFIX: &str = "/api/v1";

/// Build the whole application.
///
/// `/health` sits outside the version prefix and outside authentication, so a
/// monitoring probe does not need a token.
pub fn app<T: Transceiver + Send + 'static>(service: LampService<T>, token: ApiToken) -> Router {
    let state = AppState::new(service, token);

    let protected = Router::new()
        .nest("/lamps", lamps::router())
        .merge(meta::router())
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .nest(API_PREFIX, protected)
        .merge(meta::health_router())
        .layer(TraceLayer::new_for_http())
        .layer(NormalizePathLayer::trim_trailing_slash())
        .with_state(state)
}
