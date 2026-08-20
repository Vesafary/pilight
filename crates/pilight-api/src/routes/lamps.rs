//! Registering, listing and driving lamps.

use crate::app_state::AppState;
use crate::dto::{CommandResponse, LampResponse, NewLampRequest, StateRequest, UpdateLampRequest};
use crate::error::ApiError;
use crate::response::{ApiResponse, PageMeta, Pagination};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing};
use pilight_db::CommandSource;
use pilight_proto::Transceiver;
use pilight_service::StateChange;
use uuid::Uuid;

/// Routes under `/lamps`.
pub fn router<T: Transceiver + Send + 'static>() -> Router<AppState<T>> {
    Router::new()
        .route("/", routing::get(list).post(register))
        .route("/{id}", routing::get(get_one).patch(update).delete(remove))
        .route("/{id}/state", routing::put(set_state))
        .route("/{id}/history", routing::get(history))
        .route("/{id}/pair", routing::post(pair))
        .route("/{id}/unpair", routing::post(unpair))
}

type ApiResult<T> = Result<T, ApiError>;

/// `GET /lamps` — every registered lamp with its state.
async fn list<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
    Query(page): Query<Pagination>,
) -> ApiResult<Json<ApiResponse<Vec<LampResponse>>>> {
    let all = state.service.list().await?;
    let total = all.len();
    let lamps: Vec<LampResponse> = page
        .apply(&all)
        .into_iter()
        .map(LampResponse::from)
        .collect();

    Ok(Json(ApiResponse::paged(
        lamps,
        PageMeta {
            total,
            limit: page.limit(),
            offset: page.offset(),
        },
    )))
}

/// `POST /lamps` — register a lamp.
async fn register<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
    Json(request): Json<NewLampRequest>,
) -> ApiResult<(StatusCode, Json<ApiResponse<LampResponse>>)> {
    let lamp = state.service.register(request.into()).await?;

    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(LampResponse::from(lamp))),
    ))
}

/// `GET /lamps/{id}` — one lamp.
async fn get_one<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ApiResponse<LampResponse>>> {
    let lamp = state.service.get(id).await?;

    Ok(Json(ApiResponse::ok(LampResponse::from(lamp))))
}

/// `PATCH /lamps/{id}` — rename, or move to another room.
async fn update<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateLampRequest>,
) -> ApiResult<Json<ApiResponse<LampResponse>>> {
    let lamp = state.service.rename(id, request.into()).await?;

    Ok(Json(ApiResponse::ok(LampResponse::from(lamp))))
}

/// `DELETE /lamps/{id}` — forget a lamp.
///
/// The bulb keeps whatever it was paired with; this only forgets it here. Use
/// `POST /lamps/{id}/unpair` first to reset the bulb itself.
async fn remove<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if state.service.remove(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!("no lamp with id {id}")))
    }
}

/// `PUT /lamps/{id}/state` — change what the lamp is doing.
///
/// Absent fields are left alone. One request can ask for several things at once;
/// they are expanded into correctly ordered radio commands.
async fn set_state<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
    Path(id): Path<Uuid>,
    Json(request): Json<StateRequest>,
) -> ApiResult<Json<ApiResponse<LampResponse>>> {
    let change = StateChange::from(request);
    let lamp = state.service.change(id, change, CommandSource::Api).await?;

    Ok(Json(ApiResponse::ok(LampResponse::from(lamp))))
}

/// `GET /lamps/{id}/history` — what we sent, and whether it worked.
async fn history<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
    Path(id): Path<Uuid>,
    Query(page): Query<Pagination>,
) -> ApiResult<Json<ApiResponse<Vec<CommandResponse>>>> {
    // Fail loudly for an unknown lamp rather than returning a plausible empty list.
    state.service.get(id).await?;

    let entries = state.service.history(id, Some(page.limit())).await?;
    let total = entries.len();

    Ok(Json(ApiResponse::paged(
        entries.into_iter().map(CommandResponse::from).collect(),
        PageMeta {
            total,
            limit: page.limit(),
            offset: page.offset(),
        },
    )))
}

/// `POST /lamps/{id}/pair` — teach a bulb this lamp's identity.
///
/// Power-cycle the bulb, then call this within about three seconds.
async fn pair<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ApiResponse<LampResponse>>> {
    let lamp = state
        .service
        .apply(
            id,
            pilight_proto::RgbCctIntent::Power(true),
            CommandSource::Api,
        )
        .await?;

    Ok(Json(ApiResponse::ok(LampResponse::from(lamp))))
}

/// `POST /lamps/{id}/unpair` — factory-reset the bulb.
///
/// Power-cycle the bulb, then call this within about three seconds. The lamp stays
/// registered here; `DELETE` it separately if you are done with it.
async fn unpair<T: Transceiver + Send + 'static>(
    State(state): State<AppState<T>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    state.service.unpair(id, CommandSource::Api).await?;

    Ok(StatusCode::ACCEPTED)
}
