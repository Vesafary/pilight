//! Integration tests: the real router, the real database, a counting radio.
//!
//! ```sh
//! docker compose up -d postgres
//! export PILIGHT_TEST_DATABASE_URL=postgres://pilight:pilight@localhost:55432/pilight_test
//! cargo test -p pilight-api
//! ```

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use diesel_async::RunQueryDsl;
use http_body_util::BodyExt;
use pilight_api::{ApiToken, app};
use pilight_db::repository::LampTypeRepository;
use pilight_db::{Repositories, build_pool, run_migrations};
use pilight_proto::{NullTransceiver, RgbCctTransmitter};
use pilight_service::LampService;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::{Mutex, OnceCell};
use tower::ServiceExt;

static LOCK: Mutex<()> = Mutex::const_new(());
static MIGRATED: OnceCell<()> = OnceCell::const_new();

struct Harness {
    app: Router,
    radio: NullTransceiver,
    service: LampService<NullTransceiver>,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl Harness {
    async fn start(token: ApiToken) -> Option<Self> {
        let Ok(url) =
            std::env::var("PILIGHT_TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"))
        else {
            assert!(
                std::env::var("CI").is_err(),
                "PILIGHT_TEST_DATABASE_URL is not set, so these tests would be skipped. \
                 Refusing to do that in CI."
            );
            eprintln!("SKIPPING: set PILIGHT_TEST_DATABASE_URL to run the API tests");
            return None;
        };

        let guard = LOCK.lock().await;
        let pool = build_pool(&url).expect("pool");
        MIGRATED
            .get_or_init(|| async {
                run_migrations(&pool).await.expect("migrations");
            })
            .await;

        let repos = Repositories::new(pool.clone());
        repos.types.sync_from_driver().await.expect("type sync");

        let mut conn = pool.get().await.expect("connection");
        diesel::sql_query("TRUNCATE lamps, lamp_commands RESTART IDENTITY CASCADE")
            .execute(&mut conn)
            .await
            .expect("truncate");

        let radio = NullTransceiver::new();
        let transmitter = RgbCctTransmitter::new(radio.clone())
            .expect("the null radio always configures")
            .with_repeats(1)
            .with_gap(Duration::ZERO);
        // Zero gap: the tests assert ordering and packet counts, not bulb timing.
        let service = LampService::new(repos, transmitter).with_command_gap(Duration::ZERO);

        Some(Self {
            app: app(service.clone(), token),
            radio,
            service,
            _guard: guard,
        })
    }

    async fn request(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.request_with_auth(method, uri, body, None).await
    }

    async fn request_with_auth(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
        bearer: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(bearer) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }

        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };

        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };

        (status, json)
    }

    /// Register a lamp and return its id.
    async fn add_lamp(&self, name: &str, group: u8) -> String {
        let (status, body) = self
            .request(
                "POST",
                "/api/v1/lamps",
                Some(json!({
                    "name": name,
                    "room": "Living room",
                    "remote_type": "rgb_cct",
                    "device_id": 0xBEEF,
                    "group": group,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        body["data"]["id"].as_str().unwrap().to_owned()
    }
}

macro_rules! harness {
    () => {
        match Harness::start(ApiToken::none()).await {
            Some(h) => h,
            None => return,
        }
    };
    ($token:expr) => {
        match Harness::start($token).await {
            Some(h) => h,
            None => return,
        }
    };
}

// ------------------------------------------------------------------- health

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_needs_no_token_and_reports_the_database() {
    let harness = harness!(ApiToken::new("s3cret"));

    let (status, body) = harness.request("GET", "/health", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["status"], "ok");
    assert_eq!(body["data"]["database"], true);
}

// --------------------------------------------------------------------- auth

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_configured_token_is_enforced_on_resources() {
    let harness = harness!(ApiToken::new("s3cret"));

    let (status, body) = harness.request("GET", "/api/v1/lamps", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["success"], false);

    let (status, _) = harness
        .request_with_auth("GET", "/api/v1/lamps", None, Some("wrong"))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = harness
        .request_with_auth("GET", "/api/v1/lamps", None, Some("s3cret"))
        .await;
    assert_eq!(status, StatusCode::OK);
}

// -------------------------------------------------------------------- lamps

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registering_a_lamp_returns_it_with_a_blank_state() {
    let harness = harness!();

    let (status, body) = harness
        .request(
            "POST",
            "/api/v1/lamps",
            Some(json!({
                "name": "Couch",
                "room": "Living room",
                "remote_type": "rgb_cct",
                "device_id": 48879,
                "group": 1,
            })),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["name"], "Couch");
    assert_eq!(body["data"]["remote_type"], "rgb_cct");
    assert_eq!(body["data"]["device_id"], 48879);
    assert_eq!(body["data"]["state"]["power"], false);
    assert!(body["data"]["state"]["brightness"].is_null());
    assert!(
        body["data"]["state"].get("next_sequence").is_none(),
        "the sequence byte is an internal detail and must not leak"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registering_the_same_address_twice_is_a_conflict() {
    let harness = harness!();
    harness.add_lamp("Couch", 1).await;

    let (status, body) = harness
        .request(
            "POST",
            "/api/v1/lamps",
            Some(json!({
                "name": "Different name",
                "remote_type": "rgb_cct",
                "device_id": 0xBEEF,
                "group": 1,
            })),
        )
        .await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["success"], false);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("already registered")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_group_the_family_does_not_have_is_a_bad_request() {
    let harness = harness!();

    let (status, body) = harness
        .request(
            "POST",
            "/api/v1/lamps",
            Some(json!({
                "name": "Couch",
                "remote_type": "rgb_cct",
                "device_id": 1,
                "group": 5,
            })),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undrivable_family_is_refused_as_not_implemented() {
    let harness = harness!();

    let (status, body) = harness
        .request(
            "POST",
            "/api/v1/lamps",
            Some(json!({
                "name": "Old bulb",
                "remote_type": "rgbw",
                "device_id": 1,
                "group": 1,
            })),
        )
        .await;

    // Registration is refused at the database layer, which reports it as invalid
    // input rather than an unimplemented feature.
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("not yet drivable"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lamps_are_listed_with_pagination_metadata() {
    let harness = harness!();
    for group in 1..=3 {
        harness.add_lamp(&format!("Lamp {group}"), group).await;
    }

    let (status, body) = harness.request("GET", "/api/v1/lamps", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"].as_array().unwrap().len(), 3);
    assert_eq!(body["meta"]["total"], 3);
    assert_eq!(body["meta"]["limit"], 50);

    let (_, body) = harness
        .request("GET", "/api/v1/lamps?limit=2&offset=1", None)
        .await;
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
    assert_eq!(
        body["meta"]["total"], 3,
        "total counts everything, not the page"
    );
    assert_eq!(body["meta"]["offset"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_lamp_is_a_404_everywhere() {
    let harness = harness!();
    let ghost = uuid::Uuid::new_v4();

    for (method, path) in [
        ("GET", format!("/api/v1/lamps/{ghost}")),
        ("GET", format!("/api/v1/lamps/{ghost}/history")),
        ("DELETE", format!("/api/v1/lamps/{ghost}")),
    ] {
        let (status, body) = harness.request(method, &path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {body}");
    }

    let (status, _) = harness
        .request(
            "PUT",
            &format!("/api/v1/lamps/{ghost}/state"),
            Some(json!({"power": true})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_uuid_is_a_bad_request_not_a_500() {
    let harness = harness!();

    let (status, _) = harness
        .request("GET", "/api/v1/lamps/not-a-uuid", None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lamp_can_be_renamed_and_have_its_room_cleared() {
    let harness = harness!();
    let id = harness.add_lamp("Couch", 1).await;

    let (status, body) = harness
        .request(
            "PATCH",
            &format!("/api/v1/lamps/{id}"),
            Some(json!({"name": "Reading lamp"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["name"], "Reading lamp");
    assert_eq!(
        body["data"]["room"], "Living room",
        "an absent field leaves the room alone"
    );

    let (_, body) = harness
        .request(
            "PATCH",
            &format!("/api/v1/lamps/{id}"),
            Some(json!({"room": null})),
        )
        .await;
    assert!(
        body["data"]["room"].is_null(),
        "an explicit null clears it: {body}"
    );
    assert_eq!(body["data"]["name"], "Reading lamp");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_lamp_is_idempotent_only_the_first_time() {
    let harness = harness!();
    let id = harness.add_lamp("Couch", 1).await;

    let (status, _) = harness
        .request("DELETE", &format!("/api/v1/lamps/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = harness
        .request("DELETE", &format!("/api/v1/lamps/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// -------------------------------------------------------------------- state

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn setting_state_reaches_the_radio_and_comes_back() {
    let harness = harness!();
    let id = harness.add_lamp("Couch", 1).await;

    let before = harness.radio.discarded();
    let (status, body) = harness
        .request(
            "PUT",
            &format!("/api/v1/lamps/{id}/state"),
            Some(json!({"power": true, "brightness": 60})),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["state"]["power"], true);
    assert_eq!(body["data"]["state"]["brightness"], 60);
    // on + brightness, each on three channels.
    assert_eq!(harness.radio.discarded() - before, 2 * 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_combined_change_is_ordered_so_the_colour_survives() {
    let harness = harness!();
    let id = harness.add_lamp("Couch", 1).await;

    let (status, body) = harness
        .request(
            "PUT",
            &format!("/api/v1/lamps/{id}/state"),
            Some(json!({
                "power": true,
                "kelvin": 40,
                "hue": 200,
                "saturation": 80,
                "brightness": 50,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Kelvin forces white mode; because the hue is sent after it, the lamp ends
    // up in colour mode rather than white.
    assert_eq!(body["data"]["state"]["bulb_mode"], "color");
    assert_eq!(body["data"]["state"]["hue"], 200);
    assert_eq!(body["data"]["state"]["kelvin"], 40);
    assert_eq!(body["data"]["state"]["brightness"], 50);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_out_of_range_percentage_is_refused_before_it_reaches_the_air() {
    let harness = harness!();
    let id = harness.add_lamp("Couch", 1).await;
    let before = harness.radio.discarded();

    let (status, body) = harness
        .request(
            "PUT",
            &format!("/api/v1/lamps/{id}/state"),
            Some(json!({"brightness": 200})),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        harness.radio.discarded(),
        before,
        "nothing should have been transmitted"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_change_is_accepted_and_transmits_nothing() {
    let harness = harness!();
    let id = harness.add_lamp("Couch", 1).await;
    let before = harness.radio.discarded();

    let (status, body) = harness
        .request("PUT", &format!("/api/v1/lamps/{id}/state"), Some(json!({})))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(harness.radio.discarded(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn night_mode_overrides_the_rest_of_the_request() {
    let harness = harness!();
    let id = harness.add_lamp("Couch", 1).await;
    let before = harness.radio.discarded();

    let (status, body) = harness
        .request(
            "PUT",
            &format!("/api/v1/lamps/{id}/state"),
            Some(json!({"night_mode": true, "brightness": 100})),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["state"]["bulb_mode"], "night");
    assert_eq!(
        harness.radio.discarded() - before,
        3,
        "one intent, on three channels"
    );
}

// ------------------------------------------------------------------ history

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_history_records_what_the_api_sent() {
    let harness = harness!();
    let id = harness.add_lamp("Couch", 1).await;

    harness
        .request(
            "PUT",
            &format!("/api/v1/lamps/{id}/state"),
            Some(json!({"power": true})),
        )
        .await;

    let (status, body) = harness
        .request("GET", &format!("/api/v1/lamps/{id}/history"), None)
        .await;

    assert_eq!(status, StatusCode::OK);
    let entries = body["data"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["command"], "on");
    assert_eq!(entries[0]["source"], "api");
    assert_eq!(entries[0]["succeeded"], true);
}

// -------------------------------------------------------------- lamp types

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_catalogue_says_which_families_can_be_driven() {
    let harness = harness!();

    let (status, body) = harness.request("GET", "/api/v1/lamp-types", None).await;
    assert_eq!(status, StatusCode::OK);

    let types = body["data"].as_array().unwrap();
    assert_eq!(types.len(), 7);

    let drivable: Vec<&str> = types
        .iter()
        .filter(|t| t["driver_supported"] == true)
        .map(|t| t["slug"].as_str().unwrap())
        .collect();
    assert_eq!(drivable, vec!["rgb_cct"]);
}

// ------------------------------------------------------------------- events

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changes_made_through_the_api_are_announced_to_subscribers() {
    use pilight_service::LampEvent;

    let harness = harness!();
    let mut events = harness.service.subscribe();

    let id = harness.add_lamp("Couch", 1).await;
    let id = uuid::Uuid::parse_str(&id).unwrap();

    // This is how the MQTT bridge learns that Home Assistant needs telling.
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("an event should arrive")
        .expect("the channel should be open");
    assert_eq!(event, LampEvent::Registered(id));

    harness
        .request(
            "PUT",
            &format!("/api/v1/lamps/{id}/state"),
            Some(json!({"power": true})),
        )
        .await;

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("an event should arrive")
        .expect("the channel should be open");
    assert_eq!(event, LampEvent::StateChanged(id));
}
