//! The response envelope.
//!
//! Every response — success or failure — has the same shape, so a client can read
//! `success` before deciding what to do with the body.
//!
//! ```json
//! { "success": true,  "data": { … }, "error": null }
//! { "success": true,  "data": [ … ], "error": null, "meta": { "total": 3, "limit": 50, "offset": 0 } }
//! { "success": false, "data": null,  "error": "no lamp with id …" }
//! ```

use serde::{Deserialize, Serialize};

/// Rows returned when a request does not say.
pub const DEFAULT_LIMIT: i64 = 50;

/// Most rows a single request can ask for.
pub const MAX_LIMIT: i64 = 500;

/// Pagination for list endpoints.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageMeta {
    /// How many rows exist in total.
    pub total: usize,
    /// How many were asked for.
    pub limit: i64,
    /// How many were skipped.
    pub offset: i64,
}

/// Query parameters for list endpoints.
#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub struct Pagination {
    /// How many rows to return.
    pub limit: Option<i64>,
    /// How many rows to skip.
    pub offset: Option<i64>,
}

impl Pagination {
    /// The limit to use, clamped into something the database will not choke on.
    #[must_use]
    pub fn limit(self) -> i64 {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }

    /// The offset to use. Negative offsets are treated as zero.
    #[must_use]
    pub fn offset(self) -> i64 {
        self.offset.unwrap_or(0).max(0)
    }

    /// Apply this window to an already-loaded collection.
    #[must_use]
    pub fn apply<T: Clone>(self, items: &[T]) -> Vec<T> {
        let offset = usize::try_from(self.offset()).unwrap_or(0);
        let limit = usize::try_from(self.limit()).unwrap_or(usize::MAX);

        items.iter().skip(offset).take(limit).cloned().collect()
    }
}

/// The envelope every endpoint returns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiResponse<T> {
    /// Whether the request succeeded.
    pub success: bool,
    /// The payload. `null` on failure.
    pub data: Option<T>,
    /// What went wrong. `null` on success.
    pub error: Option<String>,
    /// Present on paginated responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<PageMeta>,
}

impl<T> ApiResponse<T> {
    /// A successful response.
    pub const fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            meta: None,
        }
    }

    /// A successful response carrying pagination.
    pub const fn paged(data: T, meta: PageMeta) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            meta: Some(meta),
        }
    }

    /// A failed response.
    pub fn failed(error: impl std::fmt::Display) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error.to_string()),
            meta: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_success_carries_data_and_a_null_error() {
        let response = ApiResponse::ok(42);
        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(json["success"], true);
        assert_eq!(json["data"], 42);
        assert!(
            json["error"].is_null(),
            "the error field is present but null"
        );
        assert!(json.get("meta").is_none(), "meta is omitted when absent");
    }

    #[test]
    fn a_failure_carries_a_message_and_null_data() {
        let response = ApiResponse::<i32>::failed("it broke");
        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(json["success"], false);
        assert!(json["data"].is_null());
        assert_eq!(json["error"], "it broke");
    }

    #[test]
    fn limits_are_clamped_rather_than_trusted() {
        // An unbounded limit from a query string is how a client accidentally
        // asks for the whole command history.
        assert_eq!(
            Pagination {
                limit: Some(100_000),
                offset: None
            }
            .limit(),
            MAX_LIMIT
        );
        assert_eq!(
            Pagination {
                limit: Some(0),
                offset: None
            }
            .limit(),
            1
        );
        assert_eq!(
            Pagination {
                limit: Some(-5),
                offset: None
            }
            .limit(),
            1
        );
        assert_eq!(Pagination::default().limit(), DEFAULT_LIMIT);
    }

    #[test]
    fn negative_offsets_are_treated_as_zero() {
        assert_eq!(
            Pagination {
                limit: None,
                offset: Some(-10)
            }
            .offset(),
            0
        );
    }

    #[test]
    fn a_window_takes_the_right_slice() {
        let items: Vec<i32> = (0..10).collect();

        let page = Pagination {
            limit: Some(3),
            offset: Some(2),
        };
        assert_eq!(page.apply(&items), vec![2, 3, 4]);

        let past_the_end = Pagination {
            limit: Some(3),
            offset: Some(99),
        };
        assert!(past_the_end.apply(&items).is_empty());
    }
}
