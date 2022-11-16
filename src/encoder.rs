const V2_OFFSET_JUMP_START: u8 = 0x54;

const V2_OFFSETS: [[u8; 4]; 8] = [
    [0x45, 0x1F, 0x14, 0x5C], // request type
    [0x2B, 0xC9, 0xE3, 0x11], // id 1
    [0x6D, 0x5F, 0x8A, 0x2B], // id 2
    [0xAF, 0x03, 0x1D, 0xF3], // command
    [0x1A, 0xE2, 0xF0, 0xD1], // argument
    [0x04, 0xD8, 0x71, 0x42], // sequence
    [0xAF, 0x04, 0xDD, 0x07], // group
    [0x61, 0x13, 0x38, 0x64], // checksum
];


pub struct V2Encoder<const PACKET_SIZE: u8> {}

impl<const PACKET_SIZE: u8> V2Encoder<PACKET_SIZE> {
    fn offset(byte: u8, key: u8, jumpstart: u8) -> u8 {
        let jump_modifier: u8 = match jumpstart > 0 && key >= jumpstart && key < jumpstart + 0x80 {
            true => 0x80,
            false => 0,
        };
        V2_OFFSETS[(byte - 1) as usize][(key % 4) as usize] + jump_modifier
    }
    
    fn xor_key(key: u8) -> u8 {
        let shift = match (key & 0x0F) < 0x04 {
            true => 0,
            false => 1,
        };
    
        let x = (((key & 0xF0) >> 4) + shift + 6) % 8; 
        let msn = (((4 + x) ^ 1) & 0x0F) << 4;
        let lsn = (((key & 0xF) + 4) ^ 2) & 0x0F;
        msn | lsn
    }
    
    fn decode_byte(byte: u8, s1: u8, key: u8, s2: u8) -> u8 {
        ((byte - s2) ^ key) - s1
    }
    
    fn encode_byte(byte: u8, s1: u8, key: u8, s2: u8) -> u8 {
        ((byte + s1) ^ key) + s2
    }
    
    pub fn decode_packet(packet: &mut [u8; 9]) {
        let b0 = packet[0];
        let key = Self::xor_key(b0);
    
        packet
            .iter_mut()
            .enumerate()
            .skip(1)
            .for_each(|(index, byte)| *byte = Self::decode_byte(
                *byte, 
                0, 
                key, 
                Self::offset(
                    index as u8, 
                    b0,
                    V2_OFFSET_JUMP_START
                )
            ));
    }
    
    pub fn encode_packet(packet: &mut [u8; 9]) {
        let b0 = packet[0];
        let key = Self::xor_key(b0);
        let mut sum = key;
    
        packet
            .iter_mut()
            .enumerate()
            .take(8)
            .skip(1)
            .for_each(|(index, byte)| {
                sum += *byte;
                *byte = Self::encode_byte(
                    *byte,
                    0,
                    key,
                    Self::offset(
                        index as u8, 
                        b0,
                        V2_OFFSET_JUMP_START
                    )
                );
            });
        packet[8] = Self::encode_byte(sum, 2, key, Self::offset(8, packet[0], 0));
    }
    
}
