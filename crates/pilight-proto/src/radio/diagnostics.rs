//! Bring-up diagnostics for the nRF24L01+.
//!
//! Deliberately built on **raw SPI**, not on the driver. When a radio does not
//! work, the first question is whether the SPI link is alive at all, and a check
//! that goes through the driver cannot answer that — it fails the same way for a
//! loose jumper as for a protocol mistake.
//!
//! Every nRF24 SPI command returns the `STATUS` register as its first byte, so a
//! single register read tells you whether anything is listening.

use crate::error::{Error, Result};
use rppal::spi::Spi;

/// `R_REGISTER` command: the low five bits are the register address.
const CMD_READ: u8 = 0x00;

/// `W_REGISTER` command.
const CMD_WRITE: u8 = 0x20;

/// Registers worth dumping, as `(name, address, width)`.
pub const REGISTERS: &[(&str, u8, usize)] = &[
    ("CONFIG", 0x00, 1),
    ("EN_AA", 0x01, 1),
    ("EN_RXADDR", 0x02, 1),
    ("SETUP_AW", 0x03, 1),
    ("SETUP_RETR", 0x04, 1),
    ("RF_CH", 0x05, 1),
    ("RF_SETUP", 0x06, 1),
    ("STATUS", 0x07, 1),
    ("RX_ADDR_P0", 0x0A, 5),
    ("TX_ADDR", 0x10, 5),
    ("RX_PW_P0", 0x11, 1),
    ("FIFO_STATUS", 0x17, 1),
];

/// The value `SETUP_AW` holds after a power-on reset: a 5-byte address.
pub const SETUP_AW_RESET: u8 = 0x03;

/// A scratch value written to `RF_CH` to prove MOSI works. Any legal channel does.
const PROBE_CHANNEL: u8 = 0x2A;

/// Read `len` bytes from a register. Returns `(status, value)`.
pub fn read_register(spi: &Spi, address: u8, len: usize) -> Result<(u8, Vec<u8>)> {
    let mut write = vec![0xFF; len + 1];
    write[0] = CMD_READ | (address & 0x1F);
    let mut read = vec![0u8; len + 1];

    spi.transfer(&mut read, &write)
        .map_err(|e| Error::Radio(Box::new(e)))?;

    Ok((read[0], read[1..].to_vec()))
}

/// Write a register. Returns the `STATUS` byte the radio replied with.
pub fn write_register(spi: &Spi, address: u8, value: &[u8]) -> Result<u8> {
    let mut write = vec![0u8; value.len() + 1];
    write[0] = CMD_WRITE | (address & 0x1F);
    write[1..].copy_from_slice(value);
    let mut read = vec![0u8; value.len() + 1];

    spi.transfer(&mut read, &write)
        .map_err(|e| Error::Radio(Box::new(e)))?;

    Ok(read[0])
}

/// What the SPI link looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkVerdict {
    /// The radio is answering and its registers hold plausible values.
    Ok,
    /// Every byte read back as `0x00`.
    StuckLow,
    /// Every byte read back as `0xFF`.
    StuckHigh,
    /// The radio answered, but with values that cannot be right.
    Implausible,
}

impl LinkVerdict {
    /// Whether the radio is usable.
    #[must_use]
    pub const fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }

    /// What this most likely means, in the order worth checking.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::Ok => "the radio is answering",
            Self::StuckLow => {
                "every byte read back as 0x00, so nothing is driving MISO. \
                 Check MISO (BCM 9, pin 21), that the module has 3.3V on VCC, \
                 and that GND is shared"
            }
            Self::StuckHigh => {
                "every byte read back as 0xFF, so MISO is floating high. \
                 Usually a disconnected MISO line, or the module is not powered"
            }
            Self::Implausible => {
                "the radio answered but its registers hold impossible values. \
                 Usually a too-fast SPI clock or long unshielded jumpers; \
                 try a lower clock, and keep the wires short"
            }
        }
    }
}

/// The result of a bring-up check.
#[derive(Debug, Clone)]
pub struct LinkCheck {
    /// Whether the radio is answering, and what it means if not.
    pub verdict: LinkVerdict,
    /// Every register we could read, as `(name, address, bytes)`.
    pub registers: Vec<(&'static str, u8, Vec<u8>)>,
    /// Whether a value written to a register read back unchanged.
    ///
    /// A dump alone only proves MISO. This proves MOSI too, and that the chip is
    /// actually processing commands rather than echoing noise.
    pub write_readback_ok: bool,
}

impl LinkCheck {
    /// Whether the radio is fit to use.
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.verdict.is_ok() && self.write_readback_ok
    }

    /// A one-line summary.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_ok() {
            "nRF24L01+ responding, registers read and write cleanly".to_owned()
        } else if !self.verdict.is_ok() {
            format!("nRF24L01+ not responding: {}", self.verdict.explanation())
        } else {
            "nRF24L01+ reads back but does not accept writes: check MOSI (BCM 10, pin 19)"
                .to_owned()
        }
    }
}

/// Check the SPI link, without disturbing anything that matters.
///
/// Scribbles on `RF_CH` to prove writes land, then puts it back. Safe to call
/// before configuring the radio for real.
pub fn check_link(spi: &Spi) -> Result<LinkCheck> {
    let mut registers = Vec::with_capacity(REGISTERS.len());
    for (name, address, len) in REGISTERS {
        let (_, value) = read_register(spi, *address, *len)?;
        registers.push((*name, *address, value));
    }

    let all_bytes: Vec<u8> = registers
        .iter()
        .flat_map(|(_, _, value)| value.iter().copied())
        .collect();

    let verdict = if all_bytes.iter().all(|b| *b == 0x00) {
        LinkVerdict::StuckLow
    } else if all_bytes.iter().all(|b| *b == 0xFF) {
        LinkVerdict::StuckHigh
    } else if plausible(&registers) {
        LinkVerdict::Ok
    } else {
        LinkVerdict::Implausible
    };

    let write_readback_ok = if verdict.is_ok() {
        probe_write(spi)?
    } else {
        false
    };

    Ok(LinkCheck {
        verdict,
        registers,
        write_readback_ok,
    })
}

/// Do the registers hold values a real nRF24 could hold?
fn plausible(registers: &[(&'static str, u8, Vec<u8>)]) -> bool {
    let get = |name: &str| {
        registers
            .iter()
            .find(|(n, _, _)| *n == name)
            .and_then(|(_, _, v)| v.first().copied())
    };

    // SETUP_AW only has two meaningful bits, and 0 is a reserved illegal value.
    let address_width_sane = get("SETUP_AW").is_some_and(|aw| (1..=3).contains(&(aw & 0x03)));
    // The top bit of STATUS is always read as 0.
    let status_sane = get("STATUS").is_some_and(|status| status & 0x80 == 0);

    address_width_sane && status_sane
}

/// Write a scratch value and read it back, then restore the original.
fn probe_write(spi: &Spi) -> Result<bool> {
    let (_, original) = read_register(spi, 0x05, 1)?;

    write_register(spi, 0x05, &[PROBE_CHANNEL])?;
    let (_, read_back) = read_register(spi, 0x05, 1)?;

    // Put it back regardless, so this is safe to run against a live radio.
    write_register(spi, 0x05, &original)?;

    Ok(read_back.first() == Some(&PROBE_CHANNEL))
}

/// Format a register dump for a terminal.
#[must_use]
pub fn format_dump(check: &LinkCheck) -> String {
    use std::fmt::Write;

    let mut out = String::new();

    for (name, address, value) in &check.registers {
        let hex: Vec<String> = value.iter().map(|b| format!("{b:02X}")).collect();
        let _ = writeln!(out, "  {name:<12} 0x{address:02X}  {}", hex.join(" "));
    }

    out
}
