//! nRF24L01+ backend, driven over the Raspberry Pi's SPI bus.
//!
//! The nRF24 is not a PL1167, but it can be made to emit the same waveform. The
//! configuration below is deliberately unusual — CRC off, auto-ACK off, no
//! retransmit — because everything the radio would normally do for us is either
//! done in software ([`crate::Frame`]) or absent from the MiLight protocol.
//!
//! See `docs/protocol.md` §2.1.

use super::config::{ADDRESS_LEN, Channel, RadioConfig};
use super::diagnostics::{LinkCheck, check_link};
use crate::error::{Error, Result};
use embedded_nrf24l01::{Configuration, CrcMode, DataRate, NRF24L01, PIPES_COUNT, StandbyMode};
use rppal::gpio::{Gpio, OutputPin};
use rppal::spi::{Bus, Mode, SlaveSelect, Spi};
use std::convert::Infallible;

/// SPI clock. The nRF24 tolerates up to 10 MHz; 8 MHz leaves margin for the wiring.
pub const DEFAULT_SPI_CLOCK_HZ: u32 = 8_000_000;

/// BCM pin driving the radio's CE line.
pub const DEFAULT_CE_PIN: u8 = 25;

/// Transmit power level: `0` is -18 dBm, `3` is 0 dBm.
pub const DEFAULT_TX_POWER: u8 = 3;

/// A no-op output pin.
///
/// `embedded-nrf24l01` wants to drive CSN itself, but on a Pi the SPI peripheral
/// already asserts CE0 for exactly the duration of each transfer — which is
/// precisely the framing an nRF24 command needs. Handing the driver a pin that does
/// nothing lets the hardware chip-select do the work, and keeps the wiring to one
/// SPI bus plus one GPIO.
#[derive(Debug, Default, Clone, Copy)]
struct NoopPin;

impl embedded_hal_0_2::digital::v2::OutputPin for NoopPin {
    type Error = Infallible;

    fn set_low(&mut self) -> std::result::Result<(), Self::Error> {
        Ok(())
    }

    fn set_high(&mut self) -> std::result::Result<(), Self::Error> {
        Ok(())
    }
}

type Radio = NRF24L01<Infallible, OutputPin, NoopPin, Spi>;

/// An nRF24L01+ on the Pi's SPI bus, pretending to be a PL1167.
pub struct Nrf24Transceiver {
    radio: Option<StandbyMode<Radio>>,
    tx_power: u8,
    channel: Option<Channel>,
}

impl std::fmt::Debug for Nrf24Transceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nrf24Transceiver")
            .field("tx_power", &self.tx_power)
            .field("channel", &self.channel)
            .finish_non_exhaustive()
    }
}

impl Nrf24Transceiver {
    /// Open SPI0 with the default wiring: CSN on CE0, CE on BCM 25.
    ///
    /// # Errors
    ///
    /// Fails if SPI or GPIO cannot be opened — usually because SPI is not enabled
    /// (`dtparam=spi=on`) or the process lacks access to `/dev/spidev0.0` and
    /// `/dev/gpiomem`.
    ///
    /// A missing or miswired radio is reported as an error naming the likely
    /// cause, not as a panic — see [`Nrf24Transceiver::check`].
    pub fn open() -> Result<Self> {
        Self::with_pins(DEFAULT_CE_PIN, DEFAULT_SPI_CLOCK_HZ)
    }

    /// Open SPI and report on the link without configuring anything.
    ///
    /// For bring-up: this answers "is the radio wired up correctly" separately
    /// from "does the protocol work", which are the two questions a silent set of
    /// bulbs leaves you with.
    pub fn check(ce_pin: u8, clock_hz: u32) -> Result<LinkCheck> {
        let spi = Self::open_spi(clock_hz)?;
        // Take the CE pin too, so a pin already claimed by something else shows up
        // here rather than later.
        let _ce = Self::open_ce(ce_pin)?;

        check_link(&spi)
    }

    fn open_spi(clock_hz: u32) -> Result<Spi> {
        Spi::new(Bus::Spi0, SlaveSelect::Ss0, clock_hz, Mode::Mode0).map_err(|e| {
            Error::radio(format!(
                "could not open /dev/spidev0.0: {e}. Is SPI enabled \
                 (`dtparam=spi=on` in /boot/firmware/config.txt, then reboot), \
                 and is this user in the `spi` group?"
            ))
        })
    }

    fn open_ce(ce_pin: u8) -> Result<OutputPin> {
        Ok(Gpio::new()
            .map_err(|e| {
                Error::radio(format!(
                    "could not open the GPIO chip: {e}. Is this user in the `gpio` group?"
                ))
            })?
            .get(ce_pin)
            .map_err(|e| Error::radio(format!("could not claim BCM {ce_pin} for CE: {e}")))?
            .into_output_low())
    }

    /// As [`Nrf24Transceiver::open`], but choosing the CE pin and SPI clock.
    pub fn with_pins(ce_pin: u8, clock_hz: u32) -> Result<Self> {
        let spi = Self::open_spi(clock_hz)?;
        let ce = Self::open_ce(ce_pin)?;

        // Check the link *before* handing the bus to the driver. Its constructor
        // asserts that a radio answers, so a loose jumper would otherwise abort the
        // process with a bare assertion failure — the worst possible message at
        // exactly the moment someone is wiring this up for the first time.
        let check = check_link(&spi)?;
        if !check.is_ok() {
            return Err(Error::radio(check.summary()));
        }

        let radio = NRF24L01::new(ce, NoopPin, spi)
            .map_err(|e| Error::radio(format!("nRF24 initialisation failed: {e:?}")))?;

        Ok(Self {
            radio: Some(radio),
            tx_power: DEFAULT_TX_POWER,
            channel: None,
        })
    }

    /// Set the transmit power, `0` (-18 dBm) to `3` (0 dBm).
    #[must_use]
    pub fn with_tx_power(mut self, tx_power: u8) -> Self {
        self.tx_power = tx_power.min(DEFAULT_TX_POWER);
        self
    }

    fn radio_mut(&mut self) -> Result<&mut StandbyMode<Radio>> {
        self.radio
            .as_mut()
            .ok_or_else(|| Error::radio("radio was left in an inconsistent state"))
    }
}

impl super::Transceiver for Nrf24Transceiver {
    fn configure(&mut self, config: &RadioConfig) -> Result<()> {
        let tx_power = self.tx_power;
        let address: [u8; ADDRESS_LEN] = config.address();
        let radio = self.radio_mut()?;

        let mut apply =
            || -> std::result::Result<(), embedded_nrf24l01::Error<rppal::spi::Error>> {
                // 1 Mbps matches the PL1167 symbol rate.
                radio.set_rf(&DataRate::R1Mbps, tx_power)?;
                // The PL1167 CRC is carried as payload bytes; the nRF24 must not add its own.
                radio.set_crc(CrcMode::Disabled)?;
                // MiLight is fire-and-forget broadcast: nothing ever acknowledges.
                radio.set_auto_ack(&[false; PIPES_COUNT])?;
                radio.set_auto_retransmit(0, 0)?;
                // The address stands in for the PL1167 preamble, syncword and trailer.
                radio.set_tx_addr(&address)?;
                radio.set_rx_addr(0, &address)?;
                radio.flush_tx()?;
                radio.flush_rx()?;
                radio.clear_interrupts()?;
                Ok(())
            };

        apply().map_err(|e| Error::radio(format!("configuring the nRF24 failed: {e:?}")))?;

        // The address trick only works at the full five bytes; anything else means
        // the trailer is not being absorbed and no packet will ever match.
        let width = radio
            .get_address_width()
            .map_err(|e| Error::radio(format!("reading SETUP_AW failed: {e:?}")))?;
        if usize::from(width) != ADDRESS_LEN {
            return Err(Error::radio(format!(
                "nRF24 reports a {width}-byte address width, need {ADDRESS_LEN}"
            )));
        }

        Ok(())
    }

    fn transmit(&mut self, channel: Channel, payload: &[u8]) -> Result<()> {
        let radio = self
            .radio
            .take()
            .ok_or_else(|| Error::radio("radio was left in an inconsistent state"))?;

        let mut tx = radio
            .tx()
            .map_err(|(_, e)| Error::radio(format!("entering TX mode failed: {e:?}")))?;

        let result =
            (|| -> std::result::Result<(), embedded_nrf24l01::Error<rppal::spi::Error>> {
                if self.channel != Some(channel) {
                    tx.set_frequency(channel.to_nrf24())?;
                }
                tx.send(payload)?;
                tx.wait_empty()?;
                Ok(())
            })();

        // Return to standby whatever happened, so the radio is never lost.
        let standby = tx
            .standby()
            .map_err(|e| Error::radio(format!("returning to standby failed: {e:?}")))?;
        self.radio = Some(standby);

        match result {
            Ok(()) => {
                self.channel = Some(channel);
                Ok(())
            }
            Err(e) => {
                self.channel = None;
                Err(Error::radio(format!("transmit failed: {e:?}")))
            }
        }
    }
}
