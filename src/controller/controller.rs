use crate::{Command, Argument, V2Encoder};


pub struct V2Packet {
    packet: [u8; 9],
}

impl V2Packet {
    pub fn new(packet: [u8; 9]) -> Self {
        Self {
            packet
        }
    }

    pub fn encode(&mut self) {
        V2Encoder::<9>::encode_packet(&mut self.packet);
    }

    pub fn from_controller(controller: &V2LampController, command: impl Command, argument: impl Argument) -> Self {
        Self {
            packet: [
                controller.key,
                controller.protocol_id,
                (controller.device_id >> 8) as u8,
                (controller.device_id & 0xFF) as u8,
                command.into(),
                argument.into(),
                controller.sequence_num,
                controller.group_id,
                controller.checksum,
            ]
        }
    }
}


pub struct V2LampController {
    key: u8,
    protocol_id: u8,
    device_id: u16,
    sequence_num: u8,
    group_id: u8,
    checksum: u8,
}

impl V2LampController {
    pub fn new(device_id: u16, protocol_id: u8) -> Self {
        Self {
            key: 0x00,
            protocol_id,
            device_id,
            sequence_num: 0,
            group_id: 0,
            checksum: 0,
        }
    }

    pub fn command(&mut self, command: impl Command, argument: impl Argument) {
        self.sequence_num += 1;
        let packet = V2Packet::from_controller(&self, command, argument);
    }
}
