//! What the handlers share.

use crate::auth::ApiToken;
use pilight_proto::Transceiver;
use pilight_service::LampService;

/// The state every handler receives.
///
/// Generic over the transceiver so the tests can run the whole router against a
/// radio that counts packets instead of transmitting them.
pub struct AppState<T: Transceiver> {
    /// The service that actually drives lamps.
    pub service: LampService<T>,
    /// The token the API requires, if any.
    pub token: ApiToken,
}

impl<T: Transceiver> Clone for AppState<T> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            token: self.token.clone(),
        }
    }
}

impl<T: Transceiver> AppState<T> {
    /// Build the shared state.
    pub const fn new(service: LampService<T>, token: ApiToken) -> Self {
        Self { service, token }
    }
}

impl<T: Transceiver> axum::extract::FromRef<AppState<T>> for ApiToken {
    fn from_ref(state: &AppState<T>) -> Self {
        state.token.clone()
    }
}
