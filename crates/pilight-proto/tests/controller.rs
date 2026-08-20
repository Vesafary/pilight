//! Controller-level tests, driven through a recording transceiver so the whole
//! transmit path is exercised without hardware.

use pilight_proto::{
    Channel, Error, Frame, GroupId, RadioConfig, RgbCctCommand, RgbCctController, Transceiver,
    V2Encoder, V2Packet,
};
use std::sync::{Arc, Mutex};

/// One recorded transmission: the nRF24 channel it went out on, and the payload.
type Sent = (u8, Vec<u8>);

/// Records every (channel, payload) pair handed to it.
#[derive(Clone, Default)]
struct Recorder {
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Recorder {
    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }

    /// Decode every recorded payload back into a plaintext V2 packet.
    fn packets(&self) -> Vec<V2Packet> {
        self.sent()
            .iter()
            .map(|(_, payload)| {
                let packet = Frame::parse(payload).expect("valid frame");
                let mut bytes = [0u8; 9];
                bytes.copy_from_slice(packet.as_slice());
                V2Packet::from_encoded(bytes)
            })
            .collect()
    }

    /// Distinct packets, in order, collapsing the repetition burst.
    fn distinct_packets(&self) -> Vec<V2Packet> {
        let mut out: Vec<V2Packet> = Vec::new();
        for packet in self.packets() {
            if out.last() != Some(&packet) {
                out.push(packet);
            }
        }
        out
    }
}

impl Transceiver for Recorder {
    fn configure(&mut self, _config: &RadioConfig) -> Result<(), Error> {
        Ok(())
    }

    fn transmit(&mut self, channel: Channel, payload: &[u8]) -> Result<(), Error> {
        self.sent
            .lock()
            .unwrap()
            .push((channel.to_nrf24(), payload.to_vec()));
        Ok(())
    }
}

fn controller(recorder: Recorder) -> RgbCctController<Recorder> {
    RgbCctController::builder(recorder)
        .device_id(0xBEEF)
        .group(GroupId::new(1, 4).unwrap())
        .repeats(2)
        .gap(std::time::Duration::ZERO)
        .build()
        .expect("valid configuration")
}

#[test]
fn turning_on_sends_the_documented_packet() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.on().unwrap();

    let packets = recorder.distinct_packets();
    assert_eq!(packets.len(), 1);

    let packet = &packets[0];
    assert_eq!(packet.protocol_id(), 0x20);
    assert_eq!(packet.device_id(), 0xBEEF);
    assert_eq!(packet.command(), RgbCctCommand::OnOff as u8);
    assert_eq!(packet.argument(), 1, "group 1 ON");
    assert_eq!(packet.group(), 1);
    assert!(packet.checksum_is_valid());
}

#[test]
fn turning_off_offsets_the_argument_by_group_count_plus_one() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.off().unwrap();

    let packet = &recorder.distinct_packets()[0];
    assert_eq!(packet.command(), RgbCctCommand::OnOff as u8);
    assert_eq!(packet.argument(), 6, "group 1 OFF is 1 + (4 + 1)");
}

#[test]
fn night_mode_sets_the_held_bit_on_an_off_argument() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.night_mode().unwrap();

    let packet = &recorder.distinct_packets()[0];
    assert!(packet.is_held());
    assert_eq!(packet.command() & 0x7F, RgbCctCommand::OnOff as u8);
    assert_eq!(packet.argument(), 6);
}

#[test]
fn hue_is_rescaled_from_degrees_and_offset() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.set_hue(0).unwrap();

    let packet = &recorder.distinct_packets()[0];
    assert_eq!(packet.command(), RgbCctCommand::Color as u8);
    assert_eq!(packet.argument(), 0x5F, "hue 0 sits at the colour offset");
}

#[test]
fn brightness_and_saturation_share_a_command_but_not_a_range() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.set_brightness(0).unwrap();
    lamp.set_brightness(100).unwrap();
    lamp.set_saturation(0).unwrap();
    lamp.set_saturation(100).unwrap();

    let packets = recorder.distinct_packets();
    assert_eq!(packets.len(), 4);

    for packet in &packets {
        assert_eq!(
            packet.command(),
            RgbCctCommand::BrightnessOrSaturation as u8
        );
    }
    assert_eq!(packets[0].argument(), 0x8F);
    assert_eq!(packets[1].argument(), 0xF3);
    assert_eq!(packets[2].argument(), 0x0D);
    assert_eq!(packets[3].argument(), 0x71);
}

#[test]
fn kelvin_walks_the_documented_scale_from_coolest_to_warmest() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.set_kelvin(0).unwrap();
    lamp.set_kelvin(100).unwrap();

    let packets = recorder.distinct_packets();
    assert_eq!(packets[0].command(), RgbCctCommand::Kelvin as u8);
    assert_eq!(packets[0].argument(), 0x94, "0% => coolest end");
    assert_eq!(packets[1].argument(), 0xCC, "100% => warmest end");
}

#[test]
fn percentages_beyond_one_hundred_are_rejected() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    assert!(matches!(
        lamp.set_brightness(101),
        Err(Error::PercentageOutOfRange(101))
    ));
    assert!(lamp.set_saturation(101).is_err());
    assert!(lamp.set_kelvin(255).is_err());
    assert!(recorder.sent().is_empty(), "nothing should reach the air");
}

#[test]
fn sequence_number_increments_per_command_and_wraps() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    for _ in 0..300 {
        lamp.on().unwrap();
    }

    let packets = recorder.distinct_packets();
    assert_eq!(packets.len(), 300, "each command is a distinct packet");
    assert_eq!(packets[0].sequence(), 0);
    assert_eq!(packets[1].sequence(), 1);
    assert_eq!(packets[255].sequence(), 255);
    assert_eq!(
        packets[256].sequence(),
        0,
        "sequence wraps, it must not panic"
    );
}

#[test]
fn sequence_is_constant_across_the_repetition_burst() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.on().unwrap();

    let sequences: Vec<u8> = recorder.packets().iter().map(V2Packet::sequence).collect();
    assert!(sequences.len() > 1, "a command is repeated");
    assert!(sequences.iter().all(|s| *s == sequences[0]));
}

#[test]
fn every_command_is_sent_on_all_three_channels() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.on().unwrap();

    let channels: Vec<u8> = recorder.sent().iter().map(|(c, _)| *c).collect();
    // repeats(2) x 3 channels
    assert_eq!(channels, vec![10, 41, 72, 10, 41, 72]);
}

#[test]
fn unpair_targets_group_zero_repeatedly() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.unpair().unwrap();

    let packets = recorder.distinct_packets();
    assert_eq!(packets.len(), 5, "five ON-all commands");
    for packet in &packets {
        assert_eq!(packet.command(), RgbCctCommand::OnOff as u8);
        assert_eq!(packet.argument(), 0, "group 0 == all groups");
    }
}

#[test]
fn payloads_on_the_wire_are_valid_encoded_frames() {
    let recorder = Recorder::default();
    let mut lamp = controller(recorder.clone());

    lamp.set_mode(3).unwrap();

    for (_, payload) in recorder.sent() {
        assert_eq!(payload.len(), 12, "1 length byte + 9 packet + 2 CRC");
        let packet = Frame::parse(&payload).expect("CRC must check out");
        let mut bytes = [0u8; 9];
        bytes.copy_from_slice(packet.as_slice());

        let plain = V2Encoder::decode(bytes);
        assert_eq!(plain[1], 0x20);
        assert_eq!(plain[4], RgbCctCommand::Mode as u8);
        assert_eq!(plain[5], 3);
    }
}

#[test]
fn group_must_fit_the_remote() {
    let recorder = Recorder::default();
    let result = RgbCctController::builder(recorder)
        .device_id(0x1234)
        .group(GroupId::new(4, 4).unwrap())
        .build();
    assert!(result.is_ok());

    assert!(matches!(
        GroupId::new(9, 8),
        Err(Error::GroupOutOfRange { group: 9, max: 8 })
    ));
}
