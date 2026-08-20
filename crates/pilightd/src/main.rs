//! The pilight daemon.
//!
//! Wires the database, the radio, the MQTT bridge and the HTTP API together and
//! runs until interrupted.
//!
//! # Configuration
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `DATABASE_URL` | — | **Required.** Postgres connection string. |
//! | `PILIGHT_RADIO` | `nrf24` | `nrf24` for real hardware, `none` to discard. |
//! | `PILIGHT_MQTT_HOST` | `localhost` | Broker host. |
//! | `PILIGHT_MQTT_PORT` | `1883` | Broker port. |
//! | `PILIGHT_MQTT_USERNAME` / `_PASSWORD` | unset | Broker credentials. |
//! | `PILIGHT_MQTT_CLIENT_ID` | `pilight` | Client id on the broker. |
//! | `PILIGHT_MQTT_PREFIX` | `pilight` | Our topic prefix. |
//! | `PILIGHT_MQTT_DISCOVERY_PREFIX` | `homeassistant` | HA's discovery prefix. |
//! | `PILIGHT_MIN_KELVIN` / `PILIGHT_MAX_KELVIN` | `2700` / `6500` | Bulb range. |
//! | `PILIGHT_RADIO_REPEATS` | `50` | Bursts per command. |
//! | `PILIGHT_COMMAND_GAP_MS` | `300` | Pause between distinct commands. |
//! | `PILIGHT_API_ADDR` | `0.0.0.0:8080` | Where the HTTP API listens. |
//! | `PILIGHT_API_TOKEN` | unset | Bearer token. Unset means no auth, loudly. |
//! | `RUST_LOG` | `info` | Log filter. |

use pilight_api::ApiToken;
use pilight_db::repository::LampTypeRepository;
use pilight_db::{Repositories, build_pool, run_migrations};
use pilight_mqtt::{Bridge, MqttConfig};
use pilight_proto::{NullTransceiver, RgbCctTransmitter, Transceiver};
use pilight_service::LampService;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

/// Where the HTTP API listens if not told otherwise.
const DEFAULT_API_ADDR: &str = "0.0.0.0:8080";

/// Multi-threaded on purpose: migrations use `block_in_place`, and every radio
/// transmission is handed to `spawn_blocking`.
#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL is not set; pilightd needs a Postgres connection string")?;

    let pool = build_pool(&database_url)?;
    let applied = run_migrations(&pool).await?;
    if applied.is_empty() {
        tracing::info!("database schema is current");
    } else {
        tracing::info!(?applied, "applied migrations");
    }

    let repos = Repositories::new(pool);
    repos.types.sync_from_driver().await?;

    // A bulb ignores a command that arrives the instant the previous one ends.
    let gap = std::env::var("PILIGHT_COMMAND_GAP_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(pilight_service::DEFAULT_COMMAND_GAP, |ms: u64| {
            std::time::Duration::from_millis(ms)
        });
    tracing::info!(gap_ms = gap.as_millis() as u64, "inter-command gap");

    match open_radio()? {
        #[cfg(feature = "nrf24")]
        Radio::Real(t) => serve(LampService::new(repos, t).with_command_gap(gap)).await,
        Radio::Null(t) => serve(LampService::new(repos, t).with_command_gap(gap)).await,
    }
}

async fn serve<T: Transceiver + Send + 'static>(
    service: LampService<T>,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = MqttConfig::from_env();
    tracing::info!(
        host = %config.host,
        port = config.port,
        prefix = %config.prefix,
        discovery_prefix = %config.discovery_prefix,
        "starting the home assistant bridge"
    );

    // The bridge follows service events so that lamps registered or driven through
    // the HTTP API show up in Home Assistant without either side knowing about the
    // other.
    let events = service.subscribe();
    let (bridge, event_loop) = Bridge::connect(service.clone(), config);
    let bridge = Arc::new(bridge);

    let mqtt = tokio::spawn({
        let bridge = Arc::clone(&bridge);
        async move { bridge.run(event_loop).await }
    });
    let watcher = tokio::spawn({
        let bridge = Arc::clone(&bridge);
        async move { bridge.watch(events).await }
    });

    let addr: SocketAddr = std::env::var("PILIGHT_API_ADDR")
        .unwrap_or_else(|_| DEFAULT_API_ADDR.to_owned())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "http api listening");

    let api = tokio::spawn(async move {
        axum::serve(listener, pilight_api::app(service, ApiToken::from_env()))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    });

    tokio::select! {
        // The API is the only one of the three that finishes on its own, and it
        // does so when Ctrl-C is pressed.
        result = api => {
            result??;
            tracing::info!("shutting down");
        }
        result = mqtt => { result?; }
        result = watcher => { result?; }
    }

    // Tell Home Assistant at once rather than leaving it to notice the keep-alive
    // lapse and show stale state in the meantime.
    if let Err(error) = bridge.shutdown().await {
        tracing::warn!(%error, "could not publish our offline status");
    }

    Ok(())
}

/// Which radio the daemon is driving.
enum Radio {
    /// A real nRF24L01+.
    #[cfg(feature = "nrf24")]
    Real(RgbCctTransmitter<pilight_proto::Nrf24Transceiver>),
    /// A radio that discards everything.
    Null(RgbCctTransmitter<NullTransceiver>),
}

fn open_radio() -> Result<Radio, Box<dyn std::error::Error>> {
    let repeats = std::env::var("PILIGHT_RADIO_REPEATS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());

    let choice = std::env::var("PILIGHT_RADIO").unwrap_or_else(|_| "nrf24".to_owned());

    match choice.as_str() {
        "none" | "null" => {
            // Loud, because a silent no-op radio looks exactly like a broken one.
            tracing::warn!(
                "PILIGHT_RADIO=none: commands will be recorded but nothing will be transmitted"
            );
            Ok(Radio::Null(with_repeats(
                RgbCctTransmitter::new(NullTransceiver::new())?,
                repeats,
            )))
        }
        #[cfg(feature = "nrf24")]
        "nrf24" => {
            let radio = pilight_proto::Nrf24Transceiver::open()?;
            tracing::info!("nrf24l01+ ready");
            Ok(Radio::Real(with_repeats(
                RgbCctTransmitter::new(radio)?,
                repeats,
            )))
        }
        #[cfg(not(feature = "nrf24"))]
        "nrf24" => Err("this build has no nrf24 support; \
                        rebuild with --features nrf24 or set PILIGHT_RADIO=none"
            .into()),
        other => {
            Err(format!("unknown PILIGHT_RADIO value `{other}`; expected `nrf24` or `none`").into())
        }
    }
}

/// Apply the configured repeat count, if one was given.
///
/// A free function rather than a closure: a closure would be monomorphised to
/// whichever transceiver used it first, and both variants need it.
fn with_repeats<T: Transceiver>(
    transmitter: RgbCctTransmitter<T>,
    repeats: Option<usize>,
) -> RgbCctTransmitter<T> {
    match repeats {
        Some(repeats) => transmitter.with_repeats(repeats),
        None => transmitter,
    }
}
