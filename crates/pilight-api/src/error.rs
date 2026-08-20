//! Mapping failures onto HTTP status codes.

use crate::response::ApiResponse;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pilight_db::DbError;
use pilight_service::ServiceError;

/// An error on its way out of a handler.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The request was malformed in a way the extractors did not catch.
    #[error("{0}")]
    BadRequest(String),

    /// The thing being asked for is not here.
    #[error("{0}")]
    NotFound(String),

    /// The service could not do it.
    #[error(transparent)]
    Service(#[from] ServiceError),

    /// Storage could not do it.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl ApiError {
    /// The status this error should be reported as.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Service(error) => service_status(error),
            Self::Db(error) => db_status(error),
        }
    }
}

// The arms below repeat a few status codes. They are kept apart on purpose: each
// documents a different reason for the same answer, and they will diverge.
#[allow(clippy::match_same_arms)]
fn service_status(error: &ServiceError) -> StatusCode {
    match error {
        ServiceError::LampNotFound(_) => StatusCode::NOT_FOUND,
        // The lamp is fine and the request is well-formed; we simply cannot drive
        // that family yet. 501 says "not implemented here", which is the truth.
        ServiceError::UnsupportedFamily { .. } => StatusCode::NOT_IMPLEMENTED,
        // The caller asked for something impossible, and we caught it before the
        // radio was involved.
        ServiceError::InvalidCommand(_) => StatusCode::BAD_REQUEST,
        // The radio is an upstream dependency that failed, not a client mistake.
        ServiceError::Radio(_) => StatusCode::BAD_GATEWAY,
        ServiceError::Db(error) => db_status(error),
        ServiceError::TransmitTask(_) => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[allow(clippy::match_same_arms)]
fn db_status(error: &DbError) -> StatusCode {
    match error {
        DbError::LampNotFound(_) => StatusCode::NOT_FOUND,
        DbError::DuplicateAddress { .. } => StatusCode::CONFLICT,
        DbError::Invalid(_) | DbError::Protocol(_) => StatusCode::BAD_REQUEST,
        // A row we cannot represent is our bug, not the caller's.
        DbError::InvalidStoredValue { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        // The database is unreachable; the caller should retry.
        DbError::Pool(_) | DbError::BuildPool(_) | DbError::Connection(_) => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();

        // Server-side failures are worth a log line; a 404 from a typo is not.
        if status.is_server_error() {
            tracing::error!(%status, error = %self, "request failed");
        } else {
            tracing::debug!(%status, error = %self, "request rejected");
        }

        (status, axum::Json(ApiResponse::<()>::failed(self))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn a_missing_lamp_is_a_404() {
        let error = ApiError::Service(ServiceError::LampNotFound(Uuid::nil()));
        assert_eq!(error.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_duplicate_address_is_a_409() {
        // Registering the same bulb twice is a conflict, not a bad request: the
        // body was fine, the world disagreed.
        let error = ApiError::Db(DbError::DuplicateAddress {
            remote_type: "rgb_cct".into(),
            device_id: 0xBEEF,
            group: 1,
        });
        assert_eq!(error.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn a_bad_value_is_a_400() {
        assert_eq!(
            ApiError::Db(DbError::Invalid("no name".into())).status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::Db(DbError::Protocol(
                pilight_proto::Error::PercentageOutOfRange(200)
            ))
            .status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn an_impossible_command_is_a_400_not_a_radio_failure() {
        // The caller asked for 200% brightness. Blaming the radio would send them
        // looking at the hardware for their own typo.
        let error = ApiError::Service(ServiceError::InvalidCommand(
            pilight_proto::Error::PercentageOutOfRange(200),
        ));
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_radio_failure_is_a_502_not_a_500() {
        // The radio is upstream of us; blaming ourselves would send the caller
        // looking in the wrong place.
        let error = ApiError::Service(ServiceError::Radio(pilight_proto::Error::radio("no spi")));
        assert_eq!(error.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn an_undrivable_family_is_a_501() {
        let error = ApiError::Service(ServiceError::UnsupportedFamily {
            remote_type: "rgbw".into(),
        });
        assert_eq!(error.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn an_unreachable_database_is_a_503() {
        let error = ApiError::Db(DbError::Connection(diesel::ConnectionError::BadConnection(
            "down".into(),
        )));
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn a_corrupt_row_is_our_fault_not_the_callers() {
        let error = ApiError::Db(DbError::InvalidStoredValue {
            column: "hue",
            value: "999".into(),
            expected: "hue",
        });
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
