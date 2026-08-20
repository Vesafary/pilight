//! The bridge: connect, announce, listen, obey, report.

use crate::config::{DEFAULT_CAPACITY, MqttConfig};
use crate::discovery::LightDiscovery;
use crate::error::Result;
use crate::payload::LightPayload;
use crate::topics::{PAYLOAD_OFFLINE, PAYLOAD_ONLINE, Topics};
use pilight_db::CommandSource;
use pilight_proto::Transceiver;
use pilight_service::{LampEvent, LampService, LampWithState};
use rumqttc::{AsyncClient, Event, Incoming, LastWill, MqttOptions, QoS};
use std::time::Duration;
use uuid::Uuid;

/// QoS for everything we publish. At-least-once: the bulbs are unreliable enough
/// without the broker adding to it, and every message here is idempotent.
const QOS: QoS = QoS::AtLeastOnce;

/// How long to wait before reconnecting after the event loop drops.
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// Bridges Home Assistant to the lamps.
///
/// Responsibilities, in the order they matter:
///
/// 1. Publish a retained discovery config per lamp, so HA creates the entities.
/// 2. Publish availability, with a last will so HA greys the lights out if the
///    daemon dies rather than showing stale state.
/// 3. Subscribe to command topics and apply what arrives.
/// 4. Publish state after every change, since the bulbs never report anything.
/// 5. Re-announce when Home Assistant restarts.
pub struct Bridge<T: Transceiver> {
    service: LampService<T>,
    client: AsyncClient,
    topics: Topics,
    config: MqttConfig,
}

impl<T: Transceiver + Send + 'static> Bridge<T> {
    /// Connect to the broker and start the bridge.
    ///
    /// Returns the bridge and the event loop to drive. Nothing happens until
    /// [`Bridge::run`] is polled, because rumqttc only makes progress while its
    /// event loop is being advanced.
    #[must_use]
    pub fn connect(service: LampService<T>, config: MqttConfig) -> (Self, rumqttc::EventLoop) {
        let topics = config.topics();

        let mut options = MqttOptions::new(&config.client_id, &config.host, config.port);
        options.set_keep_alive(config.keep_alive);
        // If this process dies, HA should grey the lights out rather than keep
        // showing state that has stopped being updated.
        options.set_last_will(LastWill::new(
            topics.availability(),
            PAYLOAD_OFFLINE,
            QOS,
            true,
        ));
        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            options.set_credentials(username, password);
        }

        let (client, event_loop) = AsyncClient::new(options, DEFAULT_CAPACITY);

        (
            Self {
                service,
                client,
                topics,
                config,
            },
            event_loop,
        )
    }

    /// A handle to the underlying client, for callers that need to publish too.
    #[must_use]
    pub fn client(&self) -> &AsyncClient {
        &self.client
    }

    /// The topic layout in use.
    #[must_use]
    pub const fn topics(&self) -> &Topics {
        &self.topics
    }

    /// Drive the bridge until the process ends.
    ///
    /// Reconnects on its own: rumqttc surfaces a disconnect as an error from the
    /// event loop and resumes on the next poll, so a broker restart is survivable
    /// without restarting the daemon.
    pub async fn run(&self, mut event_loop: rumqttc::EventLoop) -> ! {
        loop {
            match event_loop.poll().await {
                Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                    tracing::info!(
                        host = %self.config.host,
                        port = self.config.port,
                        "connected to the mqtt broker"
                    );
                    if let Err(error) = self.on_connected().await {
                        tracing::error!(%error, "could not announce ourselves");
                    }
                }
                Ok(Event::Incoming(Incoming::Publish(publish))) => {
                    let topic = publish.topic.clone();
                    if let Err(error) = self.on_message(&topic, &publish.payload).await {
                        tracing::error!(%topic, %error, "could not handle an mqtt message");
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "mqtt connection lost, retrying");
                    tokio::time::sleep(RECONNECT_DELAY).await;
                }
            }
        }
    }

    /// Announce ourselves: availability, discovery, current state, subscriptions.
    pub async fn on_connected(&self) -> Result<()> {
        self.client
            .publish(self.topics.availability(), QOS, true, PAYLOAD_ONLINE)
            .await?;

        // Subscribe before announcing, so a command sent the instant HA sees the
        // entity is not dropped on the floor.
        self.client
            .subscribe(self.topics.command_wildcard(), QOS)
            .await?;
        self.client
            .subscribe(self.topics.home_assistant_status(), QOS)
            .await?;

        self.announce_all().await
    }

    /// Publish discovery and current state for every lamp.
    ///
    /// Idempotent — the messages are retained, so republishing is how a restarted
    /// Home Assistant gets its entities back.
    pub async fn announce_all(&self) -> Result<()> {
        let lamps = self.service.list().await?;
        tracing::info!(count = lamps.len(), "announcing lamps to home assistant");

        for lamp in &lamps {
            self.announce(lamp).await?;
        }

        Ok(())
    }

    /// Publish discovery and current state for one lamp.
    pub async fn announce(&self, entry: &LampWithState) -> Result<()> {
        let discovery = LightDiscovery::for_lamp(&entry.lamp, &self.topics, self.config.kelvin);

        self.client
            .publish(
                self.topics.discovery(entry.lamp.id),
                QOS,
                true,
                serde_json::to_vec(&discovery)?,
            )
            .await?;

        self.publish_state(entry).await
    }

    /// Remove a lamp from Home Assistant.
    ///
    /// An empty retained payload on the discovery topic is how HA is told to
    /// forget an entity; without it a deleted lamp lingers as "unavailable".
    pub async fn retract(&self, lamp_id: Uuid) -> Result<()> {
        self.client
            .publish(self.topics.discovery(lamp_id), QOS, true, Vec::new())
            .await?;

        Ok(())
    }

    /// Publish a lamp's state.
    pub async fn publish_state(&self, entry: &LampWithState) -> Result<()> {
        let payload = LightPayload::from_state(&entry.state, self.config.kelvin);

        self.client
            .publish(
                self.topics.state(entry.lamp.id),
                QOS,
                true,
                serde_json::to_vec(&payload)?,
            )
            .await?;

        Ok(())
    }

    /// Handle one incoming message.
    async fn on_message(&self, topic: &str, payload: &[u8]) -> Result<()> {
        if topic == self.topics.home_assistant_status() {
            return self.on_home_assistant_status(payload).await;
        }

        let Some(lamp_id) = self.topics.lamp_id_from_command(topic) else {
            tracing::debug!(%topic, "ignoring a topic we did not ask for");
            return Ok(());
        };

        self.on_command(lamp_id, payload).await
    }

    /// Home Assistant restarted; re-announce so it finds the lamps again.
    async fn on_home_assistant_status(&self, payload: &[u8]) -> Result<()> {
        if payload == PAYLOAD_ONLINE.as_bytes() {
            tracing::info!("home assistant came online, re-announcing");
            self.announce_all().await?;
        }

        Ok(())
    }

    /// Apply a command from Home Assistant, then report what happened.
    async fn on_command(&self, lamp_id: Uuid, payload: &[u8]) -> Result<()> {
        let command: LightPayload = match serde_json::from_slice(payload) {
            Ok(command) => command,
            Err(error) => {
                // Bad JSON is the sender's problem, not ours: log it and carry on
                // rather than tearing down the connection.
                tracing::warn!(%lamp_id, %error, "ignoring an unparseable command");
                return Ok(());
            }
        };

        let change = command.to_change(self.config.kelvin);
        if change.is_empty() {
            tracing::debug!(%lamp_id, "command asked for nothing");
            return Ok(());
        }

        // Report whatever we ended up with, even on a partial failure: HA's guess
        // is otherwise left standing and drifts from reality.
        let entry = match self
            .service
            .change(lamp_id, change, CommandSource::Mqtt)
            .await
        {
            Ok(entry) => entry,
            Err(error) => {
                tracing::error!(%lamp_id, %error, "could not apply a command");
                self.service.get(lamp_id).await?
            }
        };

        self.publish_state(&entry).await
    }

    /// Follow changes made elsewhere — the HTTP API, or a future scheduler — and
    /// keep Home Assistant in step with them.
    pub async fn watch(&self, mut events: tokio::sync::broadcast::Receiver<LampEvent>) -> ! {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let Err(error) = self.on_event(event).await {
                        tracing::error!(?event, %error, "could not react to a lamp event");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    // We cannot know which lamps we missed, so re-announce the lot.
                    tracing::warn!(missed, "fell behind on lamp events, re-announcing");
                    if let Err(error) = self.announce_all().await {
                        tracing::error!(%error, "could not re-announce after lagging");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    // Only happens if every service handle is dropped, which means
                    // the process is going down anyway.
                    tracing::info!("lamp event stream closed");
                    std::future::pending::<()>().await;
                    unreachable!("pending never resolves")
                }
            }
        }
    }

    async fn on_event(&self, event: LampEvent) -> Result<()> {
        match event {
            LampEvent::Removed(lamp_id) => self.retract(lamp_id).await,
            LampEvent::Registered(lamp_id) | LampEvent::Updated(lamp_id) => {
                let entry = self.service.get(lamp_id).await?;
                self.announce(&entry).await
            }
            LampEvent::StateChanged(lamp_id) => {
                let entry = self.service.get(lamp_id).await?;
                self.publish_state(&entry).await
            }
        }
    }

    /// Publish `offline` before shutting down, so HA greys the lights out at once
    /// rather than waiting for the keep-alive to lapse.
    pub async fn shutdown(&self) -> Result<()> {
        self.client
            .publish(self.topics.availability(), QOS, true, PAYLOAD_OFFLINE)
            .await?;
        self.client.disconnect().await?;

        Ok(())
    }
}
