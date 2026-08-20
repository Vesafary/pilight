//! High-level control of an RGB+CCT bulb group.

use super::command::{GroupId, RgbCctCommand};
use super::intent::RgbCctIntent;
use super::transmitter::{LampAddress, RgbCctTransmitter};
use crate::error::{Error, Result};
use crate::packet::HELD_FLAG;
use crate::radio::Transceiver;
use std::time::Duration;

/// RGB+CCT remotes address four groups.
pub const RGB_CCT_NUM_GROUPS: u8 = 4;

/// How many ON-all commands a factory reset takes.
const UNPAIR_REPEATS: usize = 5;

/// Drives one group of RGB+CCT bulbs.
///
/// The bulbs never report back, so this type holds no notion of current state — it
/// only sends. Anything you want to display has to be tracked by the caller, and
/// will drift if someone picks up a physical remote.
#[derive(Debug)]
pub struct RgbCctController<T: Transceiver> {
    transmitter: RgbCctTransmitter<T>,
    device_id: u16,
    group: GroupId,
    sequence: u8,
}

impl<T: Transceiver> RgbCctController<T> {
    /// Start configuring a controller around `transceiver`.
    pub fn builder(transceiver: T) -> RgbCctControllerBuilder<T> {
        RgbCctControllerBuilder::new(transceiver)
    }

    /// The device id this controller transmits as.
    #[must_use]
    pub const fn device_id(&self) -> u16 {
        self.device_id
    }

    /// The group this controller addresses.
    #[must_use]
    pub const fn group(&self) -> GroupId {
        self.group
    }

    /// Re-target a different group without rebuilding the radio.
    pub fn set_group(&mut self, group: GroupId) {
        self.group = group;
    }

    /// Turn the group on.
    pub fn on(&mut self) -> Result<()> {
        self.apply(RgbCctIntent::Power(true))
    }

    /// Turn the group off.
    pub fn off(&mut self) -> Result<()> {
        self.apply(RgbCctIntent::Power(false))
    }

    /// Turn the group on or off.
    pub fn set_power(&mut self, on: bool) -> Result<()> {
        self.apply(RgbCctIntent::Power(on))
    }

    /// Drop the group into night mode — a long press on its OFF button.
    pub fn night_mode(&mut self) -> Result<()> {
        self.apply(RgbCctIntent::NightMode)
    }

    /// Set the hue, in degrees. Puts the bulb into colour mode.
    pub fn set_hue(&mut self, degrees: u16) -> Result<()> {
        self.apply(RgbCctIntent::Hue(degrees))
    }

    /// Set brightness as a percentage.
    pub fn set_brightness(&mut self, percent: u8) -> Result<()> {
        self.apply(RgbCctIntent::Brightness(percent))
    }

    /// Set saturation as a percentage.
    ///
    /// Only meaningful while the bulb is in colour mode — it shares a command byte
    /// with brightness, and the bulb's current mode decides which one it applies.
    /// Send [`RgbCctController::set_hue`] first if you are unsure.
    pub fn set_saturation(&mut self, percent: u8) -> Result<()> {
        self.apply(RgbCctIntent::Saturation(percent))
    }

    /// Set colour temperature as a percentage: 0 is the coolest end, 100 the warmest.
    ///
    /// This also forces the bulb into white mode, discarding the current hue or
    /// scene. Re-send those afterwards if you want them back.
    pub fn set_kelvin(&mut self, percent: u8) -> Result<()> {
        self.apply(RgbCctIntent::Kelvin(percent))
    }

    /// Select a scene, `0..9`.
    pub fn set_mode(&mut self, mode: u8) -> Result<()> {
        self.apply(RgbCctIntent::Scene(mode))
    }

    /// Speed the running scene up.
    pub fn mode_speed_up(&mut self) -> Result<()> {
        self.apply(RgbCctIntent::SceneSpeedUp)
    }

    /// Slow the running scene down.
    pub fn mode_speed_down(&mut self) -> Result<()> {
        self.apply(RgbCctIntent::SceneSpeedDown)
    }

    /// Pair this controller's `(device_id, group)` with a bulb.
    ///
    /// Power-cycle the bulb first; it adopts whoever shouts within a few seconds.
    pub fn pair(&mut self) -> Result<()> {
        self.on()
    }

    /// Factory-reset a bulb, forgetting whatever it is paired with.
    ///
    /// Power-cycle the bulb first, then call this within a few seconds.
    pub fn unpair(&mut self) -> Result<()> {
        let all_groups = GroupId::new(0, RGB_CCT_NUM_GROUPS)?;

        for _ in 0..UNPAIR_REPEATS {
            let argument = all_groups.on_off_argument(true);
            let sequence = self.take_sequence();
            self.transmitter.send_raw(
                LampAddress::new(self.device_id, all_groups),
                sequence,
                u8::from(RgbCctCommand::OnOff),
                argument,
            )?;
        }

        Ok(())
    }

    /// Send one intent to this controller's lamp.
    pub fn apply(&mut self, intent: RgbCctIntent) -> Result<()> {
        let sequence = self.take_sequence();

        self.transmitter.send(
            LampAddress::new(self.device_id, self.group),
            sequence,
            intent,
        )
    }

    /// Send an arbitrary command byte and argument.
    ///
    /// Escape hatch for the parts of the protocol this type does not model.
    pub fn command(&mut self, command: RgbCctCommand, argument: u8) -> Result<()> {
        self.send(u8::from(command), argument)
    }

    /// As [`RgbCctController::command`], but with the held (long-press) bit set.
    pub fn held_command(&mut self, command: RgbCctCommand, argument: u8) -> Result<()> {
        self.send(u8::from(command) | HELD_FLAG, argument)
    }

    /// Take the next sequence number.
    ///
    /// Wrapping, because a long-lived controller will pass 255. Advanced once per
    /// logical command, so a repeat burst carries a single number.
    fn take_sequence(&mut self) -> u8 {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        sequence
    }

    fn send(&mut self, command: u8, argument: u8) -> Result<()> {
        let sequence = self.take_sequence();

        self.transmitter.send_raw(
            LampAddress::new(self.device_id, self.group),
            sequence,
            command,
            argument,
        )
    }
}

/// Builder for [`RgbCctController`].
#[derive(Debug)]
pub struct RgbCctControllerBuilder<T: Transceiver> {
    transceiver: T,
    device_id: Option<u16>,
    group: Option<GroupId>,
    key: u8,
    sequence: u8,
    repeats: Option<usize>,
    gap: Option<Duration>,
}

impl<T: Transceiver> RgbCctControllerBuilder<T> {
    fn new(transceiver: T) -> Self {
        Self {
            transceiver,
            device_id: None,
            group: None,
            key: 0x00,
            sequence: 0,
            repeats: None,
            gap: None,
        }
    }

    /// The 16-bit identity to transmit as. Pick one and keep it.
    #[must_use]
    pub fn device_id(mut self, device_id: u16) -> Self {
        self.device_id = Some(device_id);
        self
    }

    /// Which group to address.
    #[must_use]
    pub fn group(mut self, group: GroupId) -> Self {
        self.group = Some(group);
        self
    }

    /// The obfuscation key byte. There is no benefit to varying it; `0x00` is fine.
    #[must_use]
    pub fn key(mut self, key: u8) -> Self {
        self.key = key;
        self
    }

    /// The sequence number to start from.
    #[must_use]
    pub fn sequence(mut self, sequence: u8) -> Self {
        self.sequence = sequence;
        self
    }

    /// How many times each command is repeated.
    #[must_use]
    pub fn repeats(mut self, repeats: usize) -> Self {
        self.repeats = Some(repeats);
        self
    }

    /// The pause between repeats.
    #[must_use]
    pub fn gap(mut self, gap: Duration) -> Self {
        self.gap = Some(gap);
        self
    }

    /// Configure the radio and build the controller.
    pub fn build(self) -> Result<RgbCctController<T>> {
        let device_id = self
            .device_id
            .ok_or(Error::MissingConfiguration("device_id"))?;
        let group = self.group.ok_or(Error::MissingConfiguration("group"))?;

        let mut transmitter = RgbCctTransmitter::new(self.transceiver)?.with_key(self.key);
        if let Some(repeats) = self.repeats {
            transmitter = transmitter.with_repeats(repeats);
        }
        if let Some(gap) = self.gap {
            transmitter = transmitter.with_gap(gap);
        }

        Ok(RgbCctController {
            transmitter,
            device_id,
            group,
            sequence: self.sequence,
        })
    }
}
