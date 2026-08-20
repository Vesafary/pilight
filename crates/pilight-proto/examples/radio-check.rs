//! Bring-up tool for the nRF24L01+.
//!
//! Run this first, before anything else, when you connect a radio.
//!
//! ```text
//! # Is the radio wired up and answering?
//! cargo run --release --example radio-check
//!
//! # Transmit a real ON command on a loop, so you can watch a bulb.
//! cargo run --release --example radio-check -- --transmit --id 0xBEEF --group 1
//! ```
//!
//! The two questions it separates are the two you are left with when the lights
//! stay off: *is the radio working*, and *is the protocol right*. A register dump
//! answers the first without depending on the second.

use pilight_proto::radio::diagnostics::{check_link, format_dump};
use pilight_proto::{
    GroupId, LampAddress, Nrf24Transceiver, RGB_CCT_NUM_GROUPS, RgbCctIntent, RgbCctTransmitter,
};
use std::time::Duration;

const USAGE: &str = "\
usage: radio-check [--ce <pin>] [--clock <hz>] [--transmit [--id <hex>] [--group <n>]]

  --ce <pin>      BCM pin driving CE (default 25)
  --clock <hz>    SPI clock (default 8000000; try 4000000 with long wires)
  --transmit      after checking, send ON/OFF on a loop so you can watch a bulb
  --id <hex>      device id to transmit as (default 0xBEEF)
  --group <n>     group to address, 0-4 (default 1)";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }

    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let ce = flag("--ce").and_then(|v| v.parse().ok()).unwrap_or(25);
    let clock = flag("--clock")
        .and_then(|v| v.parse().ok())
        .unwrap_or(8_000_000);

    println!("nRF24L01+ bring-up check");
    println!("  SPI    /dev/spidev0.0 @ {clock} Hz, CSN on CE0 (BCM 8, pin 24)");
    println!("  CE     BCM {ce}");
    println!();

    let spi = match rppal::spi::Spi::new(
        rppal::spi::Bus::Spi0,
        rppal::spi::SlaveSelect::Ss0,
        clock,
        rppal::spi::Mode::Mode0,
    ) {
        Ok(spi) => spi,
        Err(error) => {
            eprintln!("Could not open /dev/spidev0.0: {error}");
            eprintln!();
            eprintln!("  - Is SPI enabled? Add `dtparam=spi=on` to /boot/firmware/config.txt");
            eprintln!("    (or `sudo raspi-config` -> Interface Options -> SPI), then reboot.");
            eprintln!("  - Is this user in the `spi` group? `sudo usermod -aG spi $USER`,");
            eprintln!("    then log out and back in.");
            std::process::exit(1);
        }
    };

    let check = check_link(&spi)?;

    println!("Registers:");
    print!("{}", format_dump(&check));
    println!();
    println!("{}", check.summary());

    if !check.is_ok() {
        println!();
        println!("Things to check, roughly in order of how often they are the problem:");
        println!("  1. VCC must be 3.3V, NOT 5V. The logic pins tolerate 5V; the supply does not.");
        println!("  2. A 10-100uF capacitor across the module's VCC/GND. Without it the module");
        println!("     browns out and the symptom looks exactly like a protocol bug.");
        println!("  3. MISO -> BCM 9 (pin 21), MOSI -> BCM 10 (pin 19), SCLK -> BCM 11 (pin 23),");
        println!("     CSN -> BCM 8 (pin 24), CE -> BCM {ce}, and a shared GND.");
        println!("  4. Long or unshielded jumpers: try --clock 4000000, or shorter wires.");
        println!("  5. To test the Pi's SPI on its own, disconnect the module and bridge");
        println!("     MOSI to MISO: a loopback makes reads echo whatever was written.");
        std::process::exit(1);
    }

    if !args.iter().any(|a| a == "--transmit") {
        println!();
        println!("The radio is fine. Run again with --transmit to put something on the air.");
        return Ok(());
    }

    // Drop the raw handle before the driver claims the bus.
    drop(spi);

    let device_id = flag("--id")
        .map(|v| parse_u16(&v))
        .transpose()?
        .unwrap_or(0xBEEF);
    let group = flag("--group").and_then(|v| v.parse().ok()).unwrap_or(1u8);
    let address = LampAddress::new(device_id, GroupId::new(group, RGB_CCT_NUM_GROUPS)?);

    println!();
    println!("Transmitting to device {device_id:#06X}, group {group}.");
    println!("Pair a bulb first: power-cycle it, then let this run for a few seconds.");
    println!("Ctrl-C to stop.");
    println!();

    let mut transmitter = RgbCctTransmitter::new(Nrf24Transceiver::with_pins(ce, clock)?)?;
    let mut sequence = 0u8;

    loop {
        for (label, intent) in [
            ("ON ", RgbCctIntent::Power(true)),
            ("50%", RgbCctIntent::Brightness(50)),
            ("OFF", RgbCctIntent::Power(false)),
        ] {
            print!("  {label} ... ");
            use std::io::Write;
            std::io::stdout().flush().ok();

            match transmitter.send(address, sequence, intent) {
                Ok(()) => println!("sent"),
                Err(error) => println!("FAILED: {error}"),
            }

            sequence = sequence.wrapping_add(1);
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

fn parse_u16(value: &str) -> Result<u16, std::num::ParseIntError> {
    match value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Some(hex) => u16::from_str_radix(hex, 16),
        None => value.parse(),
    }
}
