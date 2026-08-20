//! Drive one group of RGB+CCT bulbs from the command line.
//!
//! Requires an nRF24L01+ on SPI0 — see the wiring table in the README.
//!
//! ```text
//! cargo run --example lamp -- --id 0xBEEF --group 1 on
//! cargo run --example lamp -- --id 0xBEEF --group 1 brightness 60
//! cargo run --example lamp -- --id 0xBEEF --group 1 hue 200
//! ```

use pilight_proto::{GroupId, Nrf24Transceiver, RGB_CCT_NUM_GROUPS, RgbCctController};

const USAGE: &str = "\
usage: lamp [--id <hex>] [--group <0-4>] <command> [value]

commands:
  on | off | night | pair | unpair
  brightness <0-100>
  saturation <0-100>       (only takes effect in colour mode)
  kelvin     <0-100>       (0 = coolest, 100 = warmest; forces white mode)
  mode       <0-8>
  hue        <0-359>";

type BoxError = Box<dyn std::error::Error>;

/// Parsed command line: `--key value` pairs, then the positional arguments.
type Args = (Vec<(String, String)>, Vec<String>);

fn main() -> Result<(), BoxError> {
    let (options, positional) = split_args(std::env::args().skip(1))?;

    let device_id = match options.iter().find(|(k, _)| k == "id") {
        Some((_, v)) => parse_u16(v)?,
        None => 0xBEEF,
    };
    let group = match options.iter().find(|(k, _)| k == "group") {
        Some((_, v)) => v.parse::<u8>()?,
        None => 1,
    };

    let Some(command) = positional.first() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };

    let mut lamp = RgbCctController::builder(Nrf24Transceiver::open()?)
        .device_id(device_id)
        .group(GroupId::new(group, RGB_CCT_NUM_GROUPS)?)
        .build()?;

    let percent = || -> Result<u8, BoxError> {
        let raw = positional.get(1).ok_or("this command needs a value")?;
        Ok(raw.parse::<u8>()?)
    };

    match command.as_str() {
        "on" => lamp.on()?,
        "off" => lamp.off()?,
        "night" => lamp.night_mode()?,
        "pair" => lamp.pair()?,
        "unpair" => lamp.unpair()?,
        "brightness" => lamp.set_brightness(percent()?)?,
        "saturation" => lamp.set_saturation(percent()?)?,
        "kelvin" => lamp.set_kelvin(percent()?)?,
        "mode" => lamp.set_mode(percent()?)?,
        "hue" => {
            let raw = positional.get(1).ok_or("hue needs a value")?;
            lamp.set_hue(raw.parse::<u16>()?)?;
        }
        other => {
            eprintln!("unknown command `{other}`\n\n{USAGE}");
            std::process::exit(2);
        }
    }

    Ok(())
}

/// Split `--key value` pairs out from the positional arguments.
fn split_args(args: impl Iterator<Item = String>) -> Result<Args, BoxError> {
    let mut options = Vec::new();
    let mut positional = Vec::new();
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.strip_prefix("--") {
            Some(key) => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("`--{key}` needs a value"))?;
                options.push((key.to_string(), value));
            }
            None => positional.push(arg),
        }
    }

    Ok((options, positional))
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
