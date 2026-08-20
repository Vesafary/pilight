//! Applying an intent to a lamp, end to end.

use crate::change::StateChange;
use crate::error::{Result, ServiceError};
use crate::event::{EVENT_BUFFER, LampEvent};
use pilight_db::domain::{
    Lamp, LampCommand, LampState, LampStateUpdate, LampUpdate, NewLamp, NewLampCommand,
};
use pilight_db::repository::{CommandLogRepository, LampRepository, LampStateRepository};
use pilight_db::{CommandSource, RemoteType, Repositories};
use pilight_proto::{GroupId, LampAddress, RgbCctIntent, RgbCctTransmitter, Transceiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

/// How many ON-all commands a factory reset takes.
const UNPAIR_COMMANDS: usize = 5;

/// Pause between two *distinct* commands to the same bulb.
///
/// Not the same as [`pilight_proto::DEFAULT_GAP`], which spaces the repeats
/// *within* one command. This is the gap between commands, and it matters:
/// sending "set temperature" the instant "switch on" finishes leaves the bulb
/// acting only on the first. Observed on real FUT092 bulbs — with no gap a
/// three-part request applied only its first part, and the rest were silently
/// dropped.
pub const DEFAULT_COMMAND_GAP: Duration = Duration::from_millis(300);

/// A lamp and what we believe about it.
#[derive(Debug, Clone)]
pub struct LampWithState {
    /// The lamp.
    pub lamp: Lamp,
    /// Its last known state.
    pub state: LampState,
}

/// Drives every registered lamp over one radio, keeping storage in step.
///
/// One radio serves many lamps, so the transmitter sits behind a mutex and each
/// command takes it in turn. Transmitting genuinely blocks — a burst is repeated
/// across three channels with pauses, a few hundred milliseconds in all — so it
/// runs on a blocking thread rather than on the async runtime.
pub struct LampService<T: Transceiver> {
    repos: Repositories,
    transmitter: Arc<Mutex<RgbCctTransmitter<T>>>,
    events: broadcast::Sender<LampEvent>,
    command_gap: Duration,
}

impl<T: Transceiver> Clone for LampService<T> {
    fn clone(&self) -> Self {
        Self {
            repos: self.repos.clone(),
            transmitter: Arc::clone(&self.transmitter),
            events: self.events.clone(),
            command_gap: self.command_gap,
        }
    }
}

impl<T: Transceiver> std::fmt::Debug for LampService<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LampService").finish_non_exhaustive()
    }
}

impl<T: Transceiver + Send + 'static> LampService<T> {
    /// Wire a radio to a set of repositories.
    #[must_use]
    pub fn new(repos: Repositories, transmitter: RgbCctTransmitter<T>) -> Self {
        Self {
            repos,
            transmitter: Arc::new(Mutex::new(transmitter)),
            events: broadcast::channel(EVENT_BUFFER).0,
            command_gap: DEFAULT_COMMAND_GAP,
        }
    }

    /// Set the pause between distinct commands to the same bulb.
    ///
    /// Zero is only sensible in tests; see [`DEFAULT_COMMAND_GAP`].
    #[must_use]
    pub fn with_command_gap(mut self, gap: Duration) -> Self {
        self.command_gap = gap;
        self
    }

    /// The pause currently applied between distinct commands.
    #[must_use]
    pub const fn command_gap(&self) -> Duration {
        self.command_gap
    }

    /// Watch for changes to lamps.
    ///
    /// Used by the MQTT bridge to keep Home Assistant in step with changes made
    /// through the HTTP API.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<LampEvent> {
        self.events.subscribe()
    }

    /// Announce a change. A send with no subscribers is not an error.
    fn emit(&self, event: LampEvent) {
        let _ = self.events.send(event);
    }

    /// The repositories underneath, for callers that need to read directly.
    #[must_use]
    pub const fn repositories(&self) -> &Repositories {
        &self.repos
    }

    /// Every lamp with its last known state.
    pub async fn list(&self) -> Result<Vec<LampWithState>> {
        let lamps = self.repos.lamps.find_all().await?;
        let mut out = Vec::with_capacity(lamps.len());

        for lamp in lamps {
            let state = self
                .repos
                .states
                .find_by_lamp(lamp.id)
                .await?
                .ok_or(ServiceError::LampNotFound(lamp.id))?;
            out.push(LampWithState { lamp, state });
        }

        Ok(out)
    }

    /// One lamp with its last known state.
    pub async fn get(&self, lamp_id: Uuid) -> Result<LampWithState> {
        let lamp = self
            .repos
            .lamps
            .find_by_id(lamp_id)
            .await?
            .ok_or(ServiceError::LampNotFound(lamp_id))?;
        let state = self
            .repos
            .states
            .find_by_lamp(lamp_id)
            .await?
            .ok_or(ServiceError::LampNotFound(lamp_id))?;

        Ok(LampWithState { lamp, state })
    }

    /// Register a lamp and announce it.
    pub async fn register(&self, lamp: NewLamp) -> Result<LampWithState> {
        let lamp = self.repos.lamps.create(lamp).await?;
        let state = self
            .repos
            .states
            .find_by_lamp(lamp.id)
            .await?
            .ok_or(ServiceError::LampNotFound(lamp.id))?;

        self.emit(LampEvent::Registered(lamp.id));

        Ok(LampWithState { lamp, state })
    }

    /// Edit a lamp's name or room.
    pub async fn rename(&self, lamp_id: Uuid, changes: LampUpdate) -> Result<LampWithState> {
        let lamp = self.repos.lamps.update(lamp_id, changes).await?;
        let state = self
            .repos
            .states
            .find_by_lamp(lamp_id)
            .await?
            .ok_or(ServiceError::LampNotFound(lamp_id))?;

        self.emit(LampEvent::Updated(lamp_id));

        Ok(LampWithState { lamp, state })
    }

    /// Delete a lamp. Returns whether one was there to delete.
    pub async fn remove(&self, lamp_id: Uuid) -> Result<bool> {
        let removed = self.repos.lamps.delete(lamp_id).await?;

        // Emit even when nothing was deleted: retracting an entity that is already
        // gone is harmless, and leaving a stale one in Home Assistant is not.
        self.emit(LampEvent::Removed(lamp_id));

        Ok(removed)
    }

    /// The recent command history for a lamp.
    pub async fn history(&self, lamp_id: Uuid, limit: Option<i64>) -> Result<Vec<LampCommand>> {
        Ok(self.repos.commands.recent_for_lamp(lamp_id, limit).await?)
    }

    /// Apply a partial state change, expanding it into correctly ordered intents.
    ///
    /// Stops at the first failure: the later intents assume the earlier ones
    /// landed. Returns the lamp's state as of the last intent that succeeded.
    pub async fn change(
        &self,
        lamp_id: Uuid,
        change: StateChange,
        source: CommandSource,
    ) -> Result<LampWithState> {
        let intents = change.to_intents();
        if intents.is_empty() {
            return self.get(lamp_id).await;
        }

        let mut latest = None;
        for (index, intent) in intents.into_iter().enumerate() {
            // Let the bulb finish with the previous command. Without this it acts
            // on the first intent and ignores the rest.
            if index > 0 && !self.command_gap.is_zero() {
                tokio::time::sleep(self.command_gap).await;
            }

            match self.apply(lamp_id, intent, source).await {
                Ok(entry) => latest = Some(entry),
                Err(error) => {
                    // A partial failure still moved the lamp, so report the error
                    // rather than a state that was never reached.
                    tracing::error!(%lamp_id, ?intent, %error, "could not apply an intent");
                    return Err(error);
                }
            }
        }

        match latest {
            Some(entry) => Ok(entry),
            None => self.get(lamp_id).await,
        }
    }

    /// Factory-reset the bulb this lamp addresses.
    ///
    /// Power-cycle the bulb first, then call this within a few seconds. The lamp
    /// stays registered; the *bulb* forgets it.
    pub async fn unpair(&self, lamp_id: Uuid, source: CommandSource) -> Result<()> {
        let lamp = self
            .repos
            .lamps
            .find_by_id(lamp_id)
            .await?
            .ok_or(ServiceError::LampNotFound(lamp_id))?;

        // Unpair is five ON-all commands in quick succession, addressed to group 0
        // rather than this lamp's group.
        let all_groups =
            GroupId::new(0, lamp.remote_type.num_groups()).map_err(ServiceError::InvalidCommand)?;
        let address = LampAddress::new(lamp.device_id, all_groups);

        for index in 0..UNPAIR_COMMANDS {
            if index > 0 && !self.command_gap.is_zero() {
                tokio::time::sleep(self.command_gap).await;
            }

            let sequence = self.repos.states.take_sequence(lamp_id).await?;
            self.transmit(address, sequence, RgbCctIntent::Power(true))
                .await?;
        }

        if let Err(error) = self
            .repos
            .commands
            .record(NewLampCommand::succeeded(lamp_id, source, "unpair", None))
            .await
        {
            tracing::warn!(%lamp_id, %error, "could not write the command log");
        }

        Ok(())
    }

    /// Send one intent to one lamp, then record what happened.
    ///
    /// The order matters. The sequence number is taken first so that concurrent
    /// senders cannot collide; the radio goes next; storage is updated only if the
    /// radio accepted it. A failure is still written to the command log — when the
    /// lights are wrong, that log is the only account of what was attempted.
    pub async fn apply(
        &self,
        lamp_id: Uuid,
        intent: RgbCctIntent,
        source: CommandSource,
    ) -> Result<LampWithState> {
        let lamp = self
            .repos
            .lamps
            .find_by_id(lamp_id)
            .await?
            .ok_or(ServiceError::LampNotFound(lamp_id))?;

        if lamp.remote_type != RemoteType::RgbCct {
            return Err(ServiceError::UnsupportedFamily {
                remote_type: lamp.remote_type.to_string(),
            });
        }

        let group = GroupId::new(lamp.group, lamp.remote_type.num_groups())
            .map_err(ServiceError::InvalidCommand)?;

        // Validate before doing anything with side effects. Otherwise a bad
        // percentage burns a sequence number and writes a "radio failed" entry to
        // the audit log, and neither of those is true.
        intent.encode(group).map_err(ServiceError::InvalidCommand)?;

        let sequence = self.repos.states.take_sequence(lamp_id).await?;
        let address = LampAddress::new(lamp.device_id, group);

        let outcome = self.transmit(address, sequence, intent).await;

        // Record the attempt either way, but never let a logging failure mask the
        // real error.
        let record = match &outcome {
            Ok(()) => NewLampCommand::succeeded(lamp_id, source, intent.name(), intent.argument()),
            Err(error) => {
                NewLampCommand::failed(lamp_id, source, intent.name(), intent.argument(), error)
            }
        };
        if let Err(error) = self.repos.commands.record(record).await {
            tracing::warn!(%lamp_id, %error, "could not write the command log");
        }

        outcome?;

        let state = self
            .repos
            .states
            .update(lamp_id, state_update_for(intent))
            .await?;

        self.emit(LampEvent::StateChanged(lamp_id));

        Ok(LampWithState { lamp, state })
    }

    /// Hand the blocking radio work to a blocking thread.
    async fn transmit(
        &self,
        address: LampAddress,
        sequence: u8,
        intent: RgbCctIntent,
    ) -> Result<()> {
        let transmitter = Arc::clone(&self.transmitter);

        tokio::task::spawn_blocking(move || {
            let mut transmitter = transmitter
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            transmitter.send(address, sequence, intent)
        })
        .await?
        .map_err(ServiceError::Radio)
    }
}

/// What an intent implies about the lamp's state, once it has been sent.
///
/// This mirrors the protocol's own side effects: a Kelvin command forces the bulb
/// into white mode, a hue command into colour mode, and so on.
#[must_use]
pub fn state_update_for(intent: RgbCctIntent) -> LampStateUpdate {
    use pilight_db::BulbMode;

    match intent {
        RgbCctIntent::Power(on) => LampStateUpdate::power(on),
        RgbCctIntent::NightMode => LampStateUpdate {
            power: Some(true),
            bulb_mode: Some(BulbMode::Night),
            ..LampStateUpdate::default()
        },
        RgbCctIntent::Hue(degrees) => LampStateUpdate {
            power: Some(true),
            ..LampStateUpdate::hue(degrees)
        },
        RgbCctIntent::Brightness(percent) => LampStateUpdate {
            power: Some(true),
            ..LampStateUpdate::brightness(percent)
        },
        RgbCctIntent::Saturation(percent) => LampStateUpdate {
            power: Some(true),
            saturation: Some(percent.min(100)),
            bulb_mode: Some(BulbMode::Color),
            ..LampStateUpdate::default()
        },
        RgbCctIntent::Kelvin(percent) => LampStateUpdate {
            power: Some(true),
            ..LampStateUpdate::kelvin(percent)
        },
        RgbCctIntent::Scene(scene) => LampStateUpdate {
            power: Some(true),
            ..LampStateUpdate::scene(scene)
        },
        // Speed changes alter nothing we track.
        RgbCctIntent::SceneSpeedUp | RgbCctIntent::SceneSpeedDown => LampStateUpdate::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pilight_db::BulbMode;

    #[test]
    fn every_light_changing_intent_implies_the_lamp_is_on() {
        // A bulb that is off cannot show you a colour; if it obeyed, it is on.
        for intent in [
            RgbCctIntent::Hue(200),
            RgbCctIntent::Brightness(60),
            RgbCctIntent::Saturation(80),
            RgbCctIntent::Kelvin(50),
            RgbCctIntent::Scene(3),
            RgbCctIntent::NightMode,
        ] {
            assert_eq!(
                state_update_for(intent).power,
                Some(true),
                "{intent:?} should imply power on"
            );
        }
    }

    #[test]
    fn mode_changes_mirror_the_protocols_side_effects() {
        assert_eq!(
            state_update_for(RgbCctIntent::Hue(200)).bulb_mode,
            Some(BulbMode::Color)
        );
        assert_eq!(
            state_update_for(RgbCctIntent::Kelvin(50)).bulb_mode,
            Some(BulbMode::White),
            "a Kelvin command drags the bulb out of colour mode"
        );
        assert_eq!(
            state_update_for(RgbCctIntent::Scene(3)).bulb_mode,
            Some(BulbMode::Scene)
        );
        assert_eq!(
            state_update_for(RgbCctIntent::NightMode).bulb_mode,
            Some(BulbMode::Night)
        );
    }

    #[test]
    fn brightness_says_nothing_about_the_mode() {
        // It applies in both white and colour mode.
        assert_eq!(
            state_update_for(RgbCctIntent::Brightness(60)).bulb_mode,
            None
        );
    }

    #[test]
    fn scene_speed_changes_nothing_we_track() {
        assert!(state_update_for(RgbCctIntent::SceneSpeedUp).is_empty());
        assert!(state_update_for(RgbCctIntent::SceneSpeedDown).is_empty());
    }

    #[test]
    fn there_is_a_gap_between_commands_by_default() {
        // Regression: with no gap, a real FUT092 bulb applied only the first
        // intent of a batched request and silently dropped the rest.
        assert!(
            !DEFAULT_COMMAND_GAP.is_zero(),
            "batched intents must not be sent back to back"
        );
    }

    #[test]
    fn turning_off_does_not_pretend_the_lamp_is_on() {
        let update = state_update_for(RgbCctIntent::Power(false));
        assert_eq!(update.power, Some(false));
        assert_eq!(update.bulb_mode, None);
    }
}
