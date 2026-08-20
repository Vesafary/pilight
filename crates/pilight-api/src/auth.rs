//! Optional bearer-token authentication.
//!
//! This daemon controls the lights in someone's house. On a trusted LAN that is
//! usually fine unauthenticated, and demanding a token before anything works would
//! be tiresome — so the token is optional, and its absence is **logged as a
//! warning** rather than passing silently.

use crate::response::ApiResponse;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The token the API expects, if any.
#[derive(Debug, Clone, Default)]
pub struct ApiToken(Option<String>);

impl ApiToken {
    /// Read `PILIGHT_API_TOKEN`, warning if it is not set.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("PILIGHT_API_TOKEN") {
            Ok(token) if !token.trim().is_empty() => Self(Some(token)),
            _ => {
                tracing::warn!(
                    "PILIGHT_API_TOKEN is not set: the HTTP API will accept any request. \
                     Set it, or make sure the listen address is not reachable off your LAN."
                );
                Self(None)
            }
        }
    }

    /// Require a specific token.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(Some(token.into()))
    }

    /// No token required.
    #[must_use]
    pub const fn none() -> Self {
        Self(None)
    }

    /// Whether a token is required at all.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        self.0.is_some()
    }

    /// Whether `presented` is acceptable.
    #[must_use]
    pub fn accepts(&self, presented: Option<&str>) -> bool {
        let Some(expected) = self.0.as_deref() else {
            return true;
        };
        let Some(presented) = presented.and_then(|value| value.strip_prefix("Bearer ")) else {
            return false;
        };

        constant_time_eq(expected.as_bytes(), presented.trim().as_bytes())
    }
}

/// Compare without leaking the answer through timing.
///
/// Length is not secret here — the token is a fixed local secret, not a per-user
/// one — but the byte comparison should not short-circuit on the first mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Reject requests that do not present the configured token.
pub async fn require_token(
    axum::extract::State(token): axum::extract::State<ApiToken>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    if token.accepts(presented) {
        return next.run(request).await;
    }

    (
        StatusCode::UNAUTHORIZED,
        axum::Json(ApiResponse::<()>::failed(
            "a valid bearer token is required",
        )),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_configured_token_everything_is_accepted() {
        let token = ApiToken::none();

        assert!(!token.is_required());
        assert!(token.accepts(None));
        assert!(token.accepts(Some("Bearer whatever")));
    }

    #[test]
    fn with_a_token_the_right_one_is_accepted() {
        let token = ApiToken::new("s3cret");

        assert!(token.is_required());
        assert!(token.accepts(Some("Bearer s3cret")));
    }

    #[test]
    fn with_a_token_anything_else_is_refused() {
        let token = ApiToken::new("s3cret");

        assert!(!token.accepts(None), "a missing header is not a free pass");
        assert!(!token.accepts(Some("Bearer wrong")));
        assert!(
            !token.accepts(Some("s3cret")),
            "the Bearer prefix is required"
        );
        assert!(!token.accepts(Some("Basic s3cret")));
        assert!(!token.accepts(Some("Bearer ")));
    }

    #[test]
    fn a_prefix_of_the_token_is_not_enough() {
        // The obvious bug in a hand-rolled comparison.
        let token = ApiToken::new("s3cret");

        assert!(!token.accepts(Some("Bearer s3c")));
        assert!(!token.accepts(Some("Bearer s3cretextra")));
    }

    #[test]
    fn comparison_handles_empty_and_unequal_lengths() {
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"a", b""));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
    }
}
