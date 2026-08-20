//! End-to-end tests against a real MQTT broker and a real Postgres.
//!
//! These prove the parts unit tests cannot: that discovery actually reaches the
//! broker retained, that a command published the way Home Assistant publishes it
//! turns into radio traffic, and that state comes back.
//!
//! ```sh
//! docker compose up -d
//! export PILIGHT_TEST_DATABASE_URL=postgres://pilight:pilight@localhost:55432/pilight_test
//! export PILIGHT_TEST_MQTT_HOST=localhost PILIGHT_TEST_MQTT_PORT=51883
//! cargo test -p pilight-mqtt --test home_assistant
//! ```

use diesel_async::RunQueryDsl;
use pilight_db::repository::{LampRepository, LampTypeRepository};
use pilight_db::{NewLamp, RemoteType, Repositories, build_pool, run_migrations};
use pilight_mqtt::discovery::LightDiscovery;
use pilight_mqtt::{Bridge, LightPayload, MqttConfig};
use pilight_proto::{NullTransceiver, RgbCctTransmitter};
use pilight_service::LampService;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

/// The tests share one broker and one database, so they run one at a time.
static LOCK: AsyncMutex<()> = AsyncMutex::const_new(());
static MIGRATED: OnceCell<()> = OnceCell::const_new();

/// How long to wait for a message before giving up.
const TIMEOUT: Duration = Duration::from_secs(10);

fn database_url() -> Option<String> {
    std::env::var("PILIGHT_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
}

fn broker() -> Option<(String, u16)> {
    let host = std::env::var("PILIGHT_TEST_MQTT_HOST").ok()?;
    let port = std::env::var("PILIGHT_TEST_MQTT_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(1883);
    Some((host, port))
}

/// Everything under test, wired together.
struct Harness {
    repos: Repositories,
    radio: NullTransceiver,
    config: MqttConfig,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl Harness {
    async fn start() -> Option<(Self, Arc<Recorder>)> {
        let (Some(url), Some((host, port))) = (database_url(), broker()) else {
            assert!(
                std::env::var("CI").is_err(),
                "PILIGHT_TEST_DATABASE_URL and PILIGHT_TEST_MQTT_HOST must be set in CI, \
                 or these tests silently pass without testing anything"
            );
            eprintln!("SKIPPING: set PILIGHT_TEST_DATABASE_URL and PILIGHT_TEST_MQTT_HOST");
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

        // A unique prefix per run keeps retained messages from an earlier run from
        // leaking into this one.
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let config = MqttConfig {
            host,
            port,
            client_id: format!("pilight-test-{unique}"),
            prefix: format!("pilight-test-{unique}"),
            discovery_prefix: format!("ha-test-{unique}"),
            ..Default::default()
        };

        let recorder = Recorder::subscribe(&config).await;
        let harness = Self {
            repos,
            radio: NullTransceiver::new(),
            config,
            _guard: guard,
        };

        Some((harness, recorder))
    }

    /// Start the bridge in the background, and wait until it has announced itself.
    async fn run_bridge(&self) -> Arc<Bridge<NullTransceiver>> {
        let transmitter = RgbCctTransmitter::new(self.radio.clone())
            .expect("the null radio always configures")
            // Keep the tests quick: the real default is 50 bursts with 5ms gaps.
            .with_repeats(1)
            .with_gap(Duration::ZERO);
        // Zero gap: the tests assert ordering and packet counts, not bulb timing.
        let service =
            LampService::new(self.repos.clone(), transmitter).with_command_gap(Duration::ZERO);

        let (bridge, event_loop) = Bridge::connect(service, self.config.clone());
        let bridge = Arc::new(bridge);

        let running = Arc::clone(&bridge);
        tokio::spawn(async move { running.run(event_loop).await });

        bridge
    }

    async fn add_lamp(&self, name: &str, group: u8) -> pilight_db::Lamp {
        self.repos
            .lamps
            .create(NewLamp {
                name: name.to_owned(),
                room: Some("Living room".to_owned()),
                remote_type: RemoteType::RgbCct,
                device_id: 0xBEEF,
                group,
            })
            .await
            .expect("the lamp should be created")
    }
}

/// A second MQTT client that records everything published, standing in for HA.
struct Recorder {
    client: AsyncClient,
    seen: Mutex<HashMap<String, Vec<u8>>>,
}

impl Recorder {
    async fn subscribe(config: &MqttConfig) -> Arc<Self> {
        let mut options = MqttOptions::new(
            format!("{}-recorder", config.client_id),
            &config.host,
            config.port,
        );
        options.set_keep_alive(Duration::from_secs(5));
        let (client, mut event_loop) = AsyncClient::new(options, 64);

        for topic in [
            format!("{}/#", config.prefix),
            format!("{}/#", config.discovery_prefix),
        ] {
            client
                .subscribe(topic, QoS::AtLeastOnce)
                .await
                .expect("subscribe");
        }

        let recorder = Arc::new(Self {
            client,
            seen: Mutex::new(HashMap::new()),
        });

        let sink = Arc::clone(&recorder);
        tokio::spawn(async move {
            while let Ok(event) = event_loop.poll().await {
                if let Event::Incoming(Incoming::Publish(publish)) = event {
                    sink.seen
                        .lock()
                        .unwrap()
                        .insert(publish.topic, publish.payload.to_vec());
                }
            }
        });

        // Give the subscription time to land before anyone publishes.
        tokio::time::sleep(Duration::from_millis(300)).await;
        recorder
    }

    /// Wait for a topic to carry a non-empty payload.
    async fn wait_for(&self, topic: &str) -> Vec<u8> {
        self.wait_until(topic, |payload| !payload.is_empty())
            .await
            .unwrap_or_else(|| panic!("timed out waiting for {topic}"))
    }

    /// Wait until a topic's payload satisfies `predicate`.
    async fn wait_until(&self, topic: &str, predicate: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
        tokio::time::timeout(TIMEOUT, async {
            loop {
                if let Some(payload) = self.seen.lock().unwrap().get(topic) {
                    if predicate(payload) {
                        return payload.clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .ok()
    }

    fn forget(&self, topic: &str) {
        self.seen.lock().unwrap().remove(topic);
    }

    async fn publish(&self, topic: &str, payload: &str) {
        self.client
            .publish(topic, QoS::AtLeastOnce, false, payload.as_bytes())
            .await
            .expect("publish");
    }
}

macro_rules! harness {
    () => {
        match Harness::start().await {
            Some(pair) => pair,
            None => return,
        }
    };
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lamp_appears_in_home_assistant_with_a_usable_config() {
    let (harness, recorder) = harness!();
    let lamp = harness.add_lamp("Couch", 1).await;
    let bridge = harness.run_bridge().await;

    let payload = recorder.wait_for(&bridge.topics().discovery(lamp.id)).await;
    let config: LightDiscovery = serde_json::from_slice(&payload).expect("valid discovery json");

    assert_eq!(config.schema, "json");
    assert_eq!(config.state_topic, bridge.topics().state(lamp.id));
    assert_eq!(config.command_topic, bridge.topics().command(lamp.id));
    assert_eq!(config.device.name, "Couch");
    assert_eq!(config.device.suggested_area.as_deref(), Some("Living room"));
    assert!(config.color_temp_kelvin);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bridge_reports_itself_online() {
    let (harness, recorder) = harness!();
    let bridge = harness.run_bridge().await;

    let payload = recorder.wait_for(&bridge.topics().availability()).await;
    assert_eq!(payload, b"online");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_command_from_home_assistant_reaches_the_radio_and_comes_back_as_state() {
    let (harness, recorder) = harness!();
    let lamp = harness.add_lamp("Couch", 1).await;
    let bridge = harness.run_bridge().await;

    let state_topic = bridge.topics().state(lamp.id);
    recorder.wait_for(&state_topic).await;
    recorder.forget(&state_topic);

    let before = harness.radio.discarded();
    recorder
        .publish(
            &bridge.topics().command(lamp.id),
            r#"{"state":"ON","brightness":255}"#,
        )
        .await;

    let payload = recorder.wait_for(&state_topic).await;
    let state: LightPayload = serde_json::from_slice(&payload).expect("valid state json");

    assert_eq!(state.state.as_deref(), Some("ON"));
    assert_eq!(state.brightness, Some(255));
    assert!(
        harness.radio.discarded() > before,
        "the command should have reached the radio"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_combined_command_produces_one_packet_per_intent() {
    let (harness, recorder) = harness!();
    let lamp = harness.add_lamp("Couch", 1).await;
    let bridge = harness.run_bridge().await;

    let state_topic = bridge.topics().state(lamp.id);
    recorder.wait_for(&state_topic).await;

    let before = harness.radio.discarded();
    recorder
        .publish(
            &bridge.topics().command(lamp.id),
            r#"{"state":"ON","brightness":128,"color":{"h":200.0,"s":80.0}}"#,
        )
        .await;

    // on + hue + saturation + brightness, each on three channels.
    let expected = 4 * 3;
    let payload = recorder
        .wait_until(&state_topic, |bytes| {
            serde_json::from_slice::<LightPayload>(bytes)
                .is_ok_and(|state| state.brightness == Some(128))
        })
        .await
        .expect("state should reflect the brightness");

    let state: LightPayload = serde_json::from_slice(&payload).unwrap();
    assert_eq!(state.color_mode.as_deref(), Some("hs"));
    assert_eq!(state.color.unwrap().h, Some(200.0));
    assert_eq!(
        harness.radio.discarded() - before,
        expected,
        "one packet per intent, per channel"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_command_is_written_to_the_audit_log() {
    use pilight_db::repository::CommandLogRepository;

    let (harness, recorder) = harness!();
    let lamp = harness.add_lamp("Couch", 1).await;
    let bridge = harness.run_bridge().await;
    recorder.wait_for(&bridge.topics().state(lamp.id)).await;

    recorder
        .publish(&bridge.topics().command(lamp.id), r#"{"state":"OFF"}"#)
        .await;

    let logged = tokio::time::timeout(TIMEOUT, async {
        loop {
            let entries = harness
                .repos
                .commands
                .recent_for_lamp(lamp.id, None)
                .await
                .expect("history");
            if !entries.is_empty() {
                return entries;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the command should be recorded");

    assert_eq!(logged[0].command, "off");
    assert_eq!(logged[0].source, pilight_db::CommandSource::Mqtt);
    assert!(logged[0].succeeded);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_home_assistant_restart_gets_the_lamps_re_announced() {
    let (harness, recorder) = harness!();
    let lamp = harness.add_lamp("Couch", 1).await;
    let bridge = harness.run_bridge().await;

    let discovery_topic = bridge.topics().discovery(lamp.id);
    recorder.wait_for(&discovery_topic).await;

    // A lamp added after the bridge started is not yet known to HA.
    let late = harness.add_lamp("Overhead", 2).await;
    let late_topic = bridge.topics().discovery(late.id);
    assert!(
        recorder.seen.lock().unwrap().get(&late_topic).is_none(),
        "the new lamp has not been announced yet"
    );

    // HA's birth message should make the bridge announce everything again.
    recorder
        .publish(&bridge.topics().home_assistant_status(), "online")
        .await;

    let payload = recorder.wait_for(&late_topic).await;
    let config: LightDiscovery = serde_json::from_slice(&payload).unwrap();
    assert_eq!(config.device.name, "Overhead");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nonsense_on_a_command_topic_is_ignored_rather_than_fatal() {
    let (harness, recorder) = harness!();
    let lamp = harness.add_lamp("Couch", 1).await;
    let bridge = harness.run_bridge().await;

    let state_topic = bridge.topics().state(lamp.id);
    recorder.wait_for(&state_topic).await;
    recorder.forget(&state_topic);

    recorder
        .publish(&bridge.topics().command(lamp.id), "{not json at all")
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The bridge must still be alive and answering.
    recorder
        .publish(&bridge.topics().command(lamp.id), r#"{"state":"ON"}"#)
        .await;
    let payload = recorder.wait_for(&state_topic).await;
    let state: LightPayload = serde_json::from_slice(&payload).unwrap();

    assert_eq!(state.state.as_deref(), Some("ON"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutting_down_tells_home_assistant_at_once() {
    let (harness, recorder) = harness!();
    let bridge = harness.run_bridge().await;

    let availability = bridge.topics().availability();
    recorder.wait_for(&availability).await;

    bridge.shutdown().await.expect("shutdown");

    let payload = recorder
        .wait_until(&availability, |bytes| bytes == b"offline")
        .await
        .expect("the bridge should publish offline");
    assert_eq!(payload, b"offline");
}
