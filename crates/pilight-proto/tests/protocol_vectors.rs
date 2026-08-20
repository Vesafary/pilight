//! Protocol conformance tests.
//!
//! Every vector here is cross-checked against a real captured packet or against
//! the reference implementation (sidoh/esp8266_milight_hub). See docs/protocol.md §7.

use pilight_proto::{
    Channel, Frame, GroupId, PROTOCOL_ID_FUT089, PROTOCOL_ID_FUT091, PROTOCOL_ID_RGB_CCT,
    RadioConfig, V2Encoder, V2Packet, crc16, reverse_bits,
};

/// A real over-the-air capture from an RGB+CCT remote (docs/protocol.md §7.1).
const CAPTURED_ENCODED: [u8; 9] = [0x1B, 0xD9, 0xED, 0x64, 0x52, 0xDD, 0xB3, 0x63, 0x1D];
const CAPTURED_DECODED: [u8; 9] = [0x1B, 0x20, 0x81, 0x64, 0x02, 0x51, 0x2C, 0x01, 0xE4];

#[test]
fn decodes_a_real_capture() {
    assert_eq!(V2Encoder::decode(CAPTURED_ENCODED), CAPTURED_DECODED);
}

#[test]
fn re_encodes_a_real_capture_byte_for_byte() {
    assert_eq!(V2Encoder::encode(CAPTURED_DECODED), CAPTURED_ENCODED);
}

#[test]
fn captured_packet_parses_into_meaningful_fields() {
    let packet = V2Packet::from_encoded(CAPTURED_ENCODED);

    assert_eq!(packet.key(), 0x1B);
    assert_eq!(packet.protocol_id(), PROTOCOL_ID_RGB_CCT);
    assert_eq!(packet.device_id(), 0x8164);
    assert_eq!(packet.command(), 0x02); // colour
    assert_eq!(packet.argument(), 0x51);
    assert_eq!(packet.sequence(), 0x2C);
    assert_eq!(packet.group(), 0x01);
    assert!(!packet.is_held());
    assert!(packet.checksum_is_valid());
}

#[test]
fn encodes_a_packet_built_from_scratch() {
    // key 0x00, RGB+CCT, device 0xBEEF, "group 1 ON", sequence 0
    let plain = [0x00, 0x20, 0xBE, 0xEF, 0x01, 0x01, 0x00, 0x01, 0x00];
    let expected = [0x00, 0xDB, 0x33, 0xC6, 0x66, 0xD1, 0xBA, 0x66, 0x9F];

    assert_eq!(V2Encoder::encode(plain), expected);
}

#[test]
fn decode_reports_checksum_plus_two_at_index_eight() {
    // The checksum byte is encoded with s1 = 2, so a decoder reads back checksum + 2.
    let plain = [0x00, 0x20, 0xBE, 0xEF, 0x01, 0x01, 0x00, 0x01, 0x00];
    let decoded = V2Encoder::decode(V2Encoder::encode(plain));

    assert_eq!(decoded[8], 0x88);
    assert_eq!(V2Encoder::checksum(&plain).wrapping_add(2), decoded[8]);
}

#[test]
fn round_trips_every_key_value() {
    // The jump-start correction kicks in for keys in [0x54, 0xD3]; make sure both
    // sides of that window survive a round trip, and that nothing overflow-panics.
    for key in 0..=u8::MAX {
        let plain = [key, 0x20, 0xBE, 0xEF, 0x04, 0x9F, 0x11, 0x03, 0x00];
        let decoded = V2Encoder::decode(V2Encoder::encode(plain));

        assert_eq!(
            &decoded[..8],
            &plain[..8],
            "round trip failed for key {key:#04X}"
        );
    }
}

#[test]
fn xor_key_matches_reference_table() {
    let expected: [u8; 16] = [
        0xB6, 0xB7, 0xB4, 0xB5, 0xAA, 0xAB, 0xA8, 0xA9, 0xAE, 0xAF, 0xAC, 0xAD, 0xA2, 0xA3, 0xA0,
        0xA1,
    ];
    for (key, want) in expected.iter().enumerate() {
        assert_eq!(V2Encoder::xor_key(key as u8), *want, "key {key:#04X}");
    }
}

#[test]
fn checksum_matches_the_capture() {
    // checksum = xor_key(p0) + sum(p1..=p7), and the decoded byte 8 is that + 2.
    let checksum = V2Encoder::checksum(&CAPTURED_DECODED);
    assert_eq!(checksum, 0xE2);
    assert_eq!(checksum.wrapping_add(2), CAPTURED_DECODED[8]);
}

#[test]
fn crc16_matches_the_reference_implementation() {
    let framed = [0x09, 0x00, 0xDB, 0x33, 0xC6, 0x66, 0xD1, 0xBA, 0x66, 0x9F];
    assert_eq!(crc16(&framed), 0xAC8F);
}

#[test]
fn reverse_bits_is_an_involution() {
    for b in 0..=u8::MAX {
        assert_eq!(reverse_bits(reverse_bits(b)), b);
    }
    assert_eq!(reverse_bits(0b1000_0000), 0b0000_0001);
    assert_eq!(reverse_bits(0x09), 0x90);
}

#[test]
fn builds_the_full_nrf24_payload() {
    let packet = [0x00, 0xDB, 0x33, 0xC6, 0x66, 0xD1, 0xBA, 0x66, 0x9F];
    let expected = [
        0x90, 0x00, 0xDB, 0xCC, 0x63, 0x66, 0x8B, 0x5D, 0x66, 0xF9, 0xF1, 0x35,
    ];

    let frame = Frame::build(&packet).expect("9 bytes fits in a frame");
    assert_eq!(frame.as_slice(), expected);
}

#[test]
fn frame_round_trips_through_parse() {
    let packet = [0x00, 0xDB, 0x33, 0xC6, 0x66, 0xD1, 0xBA, 0x66, 0x9F];
    let frame = Frame::build(&packet).unwrap();

    let parsed = Frame::parse(frame.as_slice()).expect("our own frame should parse");
    assert_eq!(parsed.as_slice(), packet);
}

#[test]
fn frame_parse_rejects_a_corrupted_payload() {
    let packet = [0x00, 0xDB, 0x33, 0xC6, 0x66, 0xD1, 0xBA, 0x66, 0x9F];
    let mut wire = Frame::build(&packet).unwrap().as_slice().to_vec();
    wire[4] ^= 0xFF;

    assert!(Frame::parse(&wire).is_err());
}

#[test]
fn radio_config_derives_the_documented_addresses() {
    // docs/protocol.md §2.3
    let cases: [(RadioConfig, [u8; 5], [u8; 3]); 5] = [
        (
            RadioConfig::RGBW,
            [0x4A, 0x1A, 0x8D, 0xE2, 0x55],
            [11, 42, 73],
        ),
        (
            RadioConfig::CCT,
            [0xAA, 0x5A, 0x05, 0x0A, 0x55],
            [6, 41, 76],
        ),
        (
            RadioConfig::RGB_CCT,
            [0x8A, 0x01, 0xE9, 0xC4, 0x56],
            [10, 41, 72],
        ),
        (
            RadioConfig::RGB,
            [0xD5, 0x33, 0x9B, 0x55, 0xAD],
            [5, 40, 75],
        ),
        (
            RadioConfig::FUT020,
            [0x55, 0xA5, 0xAA, 0x50, 0x50],
            [8, 43, 78],
        ),
    ];

    for (config, address, nrf_channels) in cases {
        assert_eq!(config.address(), address, "address for {}", config.name);
        for (i, channel) in nrf_channels.iter().enumerate() {
            assert_eq!(
                config.channels[i].to_nrf24(),
                *channel,
                "channel {i} for {}",
                config.name
            );
        }
    }
}

#[test]
fn rgb_cct_radio_config_is_the_documented_one() {
    let config = RadioConfig::RGB_CCT;
    assert_eq!(config.packet_len, 9);
    assert_eq!(config.syncword0, 0x7236);
    assert_eq!(config.syncword3, 0x1809);
    assert_eq!(
        config.channels,
        [Channel::new(8), Channel::new(39), Channel::new(70)]
    );
}

#[test]
fn group_ids_are_range_checked() {
    assert!(GroupId::new(0, 4).is_ok(), "group 0 means all groups");
    assert!(GroupId::new(4, 4).is_ok());
    assert!(GroupId::new(5, 4).is_err());
    assert!(GroupId::new(8, 8).is_ok(), "FUT089 has eight groups");
}

#[test]
fn protocol_ids_are_the_documented_ones() {
    assert_eq!(PROTOCOL_ID_RGB_CCT, 0x20);
    assert_eq!(PROTOCOL_ID_FUT091, 0x21);
    assert_eq!(PROTOCOL_ID_FUT089, 0x25);
}
