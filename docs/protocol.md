# MiLight / LimitlessLED 2.4 GHz radio protocol

Reference notes for `pilight`. Everything here describes the **on-air protocol** used
by MiLight (a.k.a. LimitlessLED, EasyBulb, FUT-series) bulbs and their handheld
remotes — not the WiFi-gateway UDP protocol.

The protocol is not published by the vendor. It has been reverse-engineered by the
community; the primary sources are listed at the [bottom](#sources). Every algorithm
and constant on this page was re-derived and verified against a captured packet
before being written down — see [Test vectors](#7-test-vectors).

---

## 1. Layer overview

The bulbs contain a **PL1167** 2.4 GHz FSK transceiver. An **nRF24L01+** is not the
same chip, but it can be coerced into producing a bit-identical waveform by abusing
its address matching. So the stack has four layers:

```
┌──────────────────────────────────────────────────────────┐
│ 4. Application    on/off, hue, brightness, kelvin, scene  │  §5
├──────────────────────────────────────────────────────────┤
│ 3. Packet         9-byte V2 packet (or 6/7-byte V1)       │  §4
├──────────────────────────────────────────────────────────┤
│ 2. Obfuscation    per-packet keyed XOR + position offsets │  §4.2
├──────────────────────────────────────────────────────────┤
│ 1. PL1167 frame   preamble/syncword/trailer/len/CRC-16    │  §3
├──────────────────────────────────────────────────────────┤
│ 0. Radio          nRF24L01+, 1 Mbps GFSK, 3 channels      │  §2
└──────────────────────────────────────────────────────────┘
```

Two generations of packet format exist in the wild:

| Generation | Bulb families | Packet length | Obfuscated |
|---|---|---|---|
| **V1** | RGBW (FUT096), CCT (FUT007), RGB (FUT098), FUT020 | 6–7 bytes | No (plaintext) |
| **V2** | RGB+CCT (FUT092/FUT089/FUT091) | 9 bytes | Yes |

`pilight` targets **V2**, which is what the current RGB+CCT bulbs use. V1 is
documented in [§6](#6-v1-legacy-protocols) for completeness.

---

## 2. Radio layer

### 2.1 nRF24L01+ register configuration

To impersonate a PL1167 the nRF24 must be put in a very unusual mode:

| Setting | Value | Why |
|---|---|---|
| Data rate | **1 Mbps** | Matches the PL1167 symbol rate. |
| CRC | **disabled** | The PL1167 CRC is computed in software and carried as payload bytes. |
| Auto-ACK (`EN_AA`) | **disabled** on all pipes | MiLight is fire-and-forget broadcast. |
| Auto-retransmit | **disabled** | Repetition is done at the application layer instead. |
| Address width | **5 bytes** | Absorbs preamble + syncword + trailer (see §3.2). |
| Address (TX pipe / RX pipe 1) | per bulb family, §2.3 | Acts as the PL1167 syncword. |
| Payload size | **fixed**, `packet_len + 3` | length byte + packet + 2 CRC bytes. |
| Dynamic payload | **disabled** | Length is carried inside the payload, not by the radio. |

### 2.2 Channels

Each bulb family hops between three channels. A transmitter sends the *same* frame
on all three, in sequence, to survive interference.

The nRF24 channel register is offset by **+2** relative to the PL1167 channel number
(this offset was determined empirically by the community, and is what working
implementations use):

```
nrf24_channel = pl1167_channel + 2
frequency_MHz = 2400 + nrf24_channel
```

### 2.3 Per-family radio parameters

| Family | Syncword0 | Syncword3 | Preamble | Trailer | Pkt len | PL1167 ch | nRF24 ch | nRF24 address (byte 0 … byte 4) |
|---|---|---|---|---|---|---|---|---|
| RGBW (FUT096) | `0x147A` | `0x258B` | `0xAA` | `0x05` | 7 | 9, 40, 71 | 11, 42, 73 | `4A 1A 8D E2 55` |
| CCT (FUT007) | `0x050A` | `0x55AA` | `0xAA` | `0x05` | 7 | 4, 39, 74 | 6, 41, 76 | `AA 5A 05 0A 55` |
| **RGB+CCT (FUT092/089/091)** | `0x7236` | `0x1809` | `0xAA` | `0x05` | **9** | **8, 39, 70** | **10, 41, 72** | **`8A 01 E9 C4 56`** |
| RGB (FUT098) | `0x9AAB` | `0xBCCD` | `0x55` | `0x0A` | 6 | 3, 38, 73 | 5, 40, 75 | `D5 33 9B 55 AD` |
| FUT020 | `0x50A0` | `0xAA55` | `0xAA` | `0x0A` | 6 | 6, 41, 76 | 8, 43, 78 | `55 A5 AA 50 50` |

The address bytes are listed **in array order**, which is the order the `RF24`-style
APIs expect (`addr[0]` is the LSB, transmitted last). See §3.2 for how they are derived.

### 2.4 Repetition

A single transmission is unreliable. Real remotes repeat each frame many times.
Working implementations send **each packet on each of the 3 channels**, and repeat
the whole burst **~50 times** with a **~5 ms** gap for a one-shot command (on/off,
scene), or continuously while a value is being dragged (brightness, hue).

The receiver de-duplicates using `(packet[1] << 8) | packet[last]` as a crude packet
identity, so re-sending an identical packet is harmless — but the **sequence byte**
([§4.1](#41-packet-layout)) must stay constant across the repeats of one logical
command, and increment between distinct commands.

### 2.5 Distinct commands need a gap between them

⚠ **This is not in any published source; it was found on real FUT092 bulbs
(2026-08-20) and cost an afternoon.**

The repetition above spaces the repeats *within* one command. Two **different**
commands additionally need a pause between them. Sending "set colour temperature"
the instant the "switch on" burst finishes leaves the bulb acting on the first and
silently ignoring the second — no error, no partial effect, nothing.

The failure is particularly nasty because:

* It is invisible from the transmitter. Every packet is sent correctly, and a
  sniffing receiver decodes them all correctly. Only a bulb reveals it.
* It looks exactly like a *wrong argument*. A request of "on, warm, 40%" appeared to
  do nothing at all, which sends you hunting through offsets and scales — all of
  which were right.
* One-command requests always work, so it only appears once a caller batches.

**~300 ms between distinct commands is enough**; 0 ms is definitely not. The exact
threshold has not been characterised. A human pressing buttons on a real remote
never produces a gap small enough to hit this, which is presumably why no
reverse-engineering write-up mentions it.

`pilight` applies this in `LampService`, not in the radio layer, because the radio
sends one command at a time and cannot know whether another is following.

---

## 3. PL1167 framing

### 3.1 Over-the-air bit layout

Lengths in bits:

```
┌──────────┬───────────────┬────────────┬─────────────┬──────────────┬──────────┐
│ Preamble │  Syncword     │  Trailer   │ Packet len  │   Packet     │  CRC-16  │
│    (8)   │     (32)      │    (4)     │     (8)     │  (8 × len)   │   (16)   │
└──────────┴───────────────┴────────────┴─────────────┴──────────────┴──────────┘
```

Two properties make this awkward:

1. **The trailer is 4 bits.** Everything after it is nibble-misaligned relative to
   byte boundaries.
2. **Bit order is reversed.** The PL1167 transmits LSB-first within each byte; the
   nRF24 transmits MSB-first. Every byte must be bit-reversed before handing it to
   the nRF24, and after receiving from it.

### 3.2 Folding the trailer into the nRF24 address

The trick that makes an nRF24 work at all: set the nRF24 address width to **5 bytes**
and pack `trailer ‖ syncword ‖ preamble` into it. The nRF24's address matcher then
consumes the misaligned trailer for us, and the remaining payload lands byte-aligned.

Given a config `(syncword0, syncword3, preamble, trailer)`:

```
addr[4] = rev8( ((syncword0 <<  4) & 0xF0) | (preamble & 0x0F) )
addr[3] = rev8(  (syncword0 >>  4) & 0xFF )
addr[2] = rev8( ((syncword0 >> 12) & 0x0F) + ((syncword3 << 4) & 0xF0) )
addr[1] = rev8(  (syncword3 >>  4) & 0xFF )
addr[0] = rev8( ((syncword3 >> 12) & 0x0F) | ((trailer  << 4) & 0xF0) )
```

where `rev8(b)` reverses the bit order of a byte.

> The trailer value (`0x05` / `0x0A`) is *assumed constant* — it was read off captured
> packets, not derived. If packets that should be present never appear, the fallback
> is a 4-byte address containing only the syncword, and bit-shifting each received
> payload by a nibble in software.

### 3.3 CRC-16

Reflected CRC-16 with polynomial **`0x8408`** (the reflected form of CRC-16/CCITT
`0x1021`), initial value **`0x0000`**, no final XOR, no reflection of the output.

```rust
const CRC_POLY: u16 = 0x8408;

fn crc16(data: &[u8]) -> u16 {
    let mut state: u16 = 0;
    for &b in data {
        let mut byte = b;
        for _ in 0..8 {
            if ((byte as u16) ^ state) & 0x01 != 0 {
                state = (state >> 1) ^ CRC_POLY;
            } else {
                state >>= 1;
            }
            byte >>= 1;
        }
    }
    state
}
```

It is computed over **the length byte plus the packet**, *before* bit reversal.

### 3.4 Building the nRF24 payload

```
1.  packet   = encode_v2(plain_packet)          // §4.2, 9 bytes
2.  framed   = [packet.len() as u8] ++ packet   // 10 bytes
3.  crc      = crc16(&framed)
4.  payload  = framed.map(rev8)
               ++ [rev8(crc as u8), rev8((crc >> 8) as u8)]
5.  radio.set_channel(pl1167_channel + 2)
6.  radio.write(&payload)                       // 12 bytes, fixed size
```

Receiving is the mirror image: read the fixed-size payload, `rev8` every byte, split
off the trailing 2 CRC bytes, verify, then check that `payload[0] + 1 == received_len`
and hand `payload[1..]` to the packet layer.

---

## 4. V2 packet format (RGB+CCT)

### 4.1 Packet layout

Nine bytes, **after** de-obfuscation:

| Index | Field | Description |
|---|---|---|
| 0 | **Key** | Obfuscation key. Chosen freely by the transmitter; `0x00` is fine and is what most implementations hardcode. Transmitted in the clear. |
| 1 | **Protocol ID** | Bulb family. `0x20` = RGB+CCT (FUT092), `0x25` = FUT089 (8 groups), `0x21` = FUT091 (V2 CCT). |
| 2 | **Device ID (high)** | Remote/hub identity, big-endian. |
| 3 | **Device ID (low)** | |
| 4 | **Command** | Low 7 bits = command. **Bit 7 (`0x80`) = "held"**, used for long-press semantics such as night mode. |
| 5 | **Argument** | Command parameter. |
| 6 | **Sequence** | Incremented once per logical command; constant across repeats. |
| 7 | **Group** | Target group. `0` = all groups. **Not reliable on receive** — for on/off, take the group from the argument byte instead (§5.1). |
| 8 | **Checksum** | See §4.3. Overwritten by the encoder; its input value is ignored. |

A bulb is addressed by the `(device_id, group)` pair. Pairing is the act of teaching a
bulb one such pair — see [§5.5](#55-pairing-and-unpairing).

### 4.2 Obfuscation

Bytes 1–8 are obfuscated. Byte 0 is not (it is the key). The transform is a keyed XOR
plus a per-position additive offset.

#### XOR key derivation

The key byte `p0` is expanded into a one-byte XOR key by mangling its nibbles
independently:

```rust
fn xor_key(key: u8) -> u8 {
    let shift = if (key & 0x0F) < 0x04 { 0 } else { 1 };
    let x = (((key & 0xF0) >> 4) + shift + 6) % 8;
    let msn = (((4 + x) ^ 1) & 0x0F) << 4;
    let lsn = (((key & 0x0F) + 4) ^ 2) & 0x0F;
    msn | lsn
}
```

For `p0 = 0x00` this yields `0xB6`. The first sixteen values are:

| `p0` | 00 | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 0A | 0B | 0C | 0D | 0E | 0F |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `k` | B6 | B7 | B4 | B5 | AA | AB | A8 | A9 | AE | AF | AC | AD | A2 | A3 | A0 | A1 |

#### Position offsets

Each packet position has four candidate offsets; which one applies is selected by
`p0 % 4`.

| Position | Field | `p0%4 = 0` | `1` | `2` | `3` |
|---|---|---|---|---|---|
| 1 | protocol id | `0x45` | `0x1F` | `0x14` | `0x5C` |
| 2 | id high | `0x2B` | `0xC9` | `0xE3` | `0x11` |
| 3 | id low | `0x6D` | `0x5F` | `0x8A` | `0x2B` |
| 4 | command | `0xAF` | `0x03` | `0x1D` | `0xF3` |
| 5 | argument | `0x1A` | `0xE2` | `0xF0` | `0xD1` |
| 6 | sequence | `0x04` | `0xD8` | `0x71` | `0x42` |
| 7 | group | `0xAF` | `0x04` | `0xDD` | `0x07` |
| 8 | checksum | `0x61` | `0x13` | `0x38` | `0x64` |

> ⚠️ Chris Mullins' blog post lists row 5 as `5A 22 30 11`. That contradicts the table
> above, which is the one used by every working implementation and the one verified
> against a real capture in §7. Use the table above.

On top of that there is a **jump-start** correction: for positions 1–7, if the key
byte falls in the window `[0x54, 0xD3]` (i.e. `0x54 <= p0 < 0x54 + 0x80`), add `0x80`
to the offset. Position 8 (the checksum) does **not** get this correction.

```rust
const V2_OFFSET_JUMP_START: u8 = 0x54;

fn offset(position: u8, p0: u8, jump_start: u8) -> u8 {
    let bump = if jump_start > 0
        && p0 >= jump_start
        && p0 < jump_start.wrapping_add(0x80) { 0x80 } else { 0 };
    V2_OFFSETS[(position - 1) as usize][(p0 % 4) as usize].wrapping_add(bump)
}
```

Note `jump_start + 0x80` = `0x54 + 0x80` = `0xD4`, which does **not** overflow `u8`.
The offset table lookup *can* overflow when `bump` is added (`0x8A + 0x80`), so the
addition must wrap. **All arithmetic in this section is mod-256.**

#### Byte transform

```rust
fn encode_byte(byte: u8, s1: u8, xor_key: u8, s2: u8) -> u8 {
    ((byte.wrapping_add(s1)) ^ xor_key).wrapping_add(s2)
}

fn decode_byte(byte: u8, s1: u8, xor_key: u8, s2: u8) -> u8 {
    ((byte.wrapping_sub(s2)) ^ xor_key).wrapping_sub(s1)
}
```

`s1` is `0` for every byte except the checksum, where it is `2`. `s2` is the position
offset from the table.

#### Full encode / decode

```
encode(p):
    k   = xor_key(p[0])
    sum = k
    for i in 1..=7:
        sum += p[i]                                        # mod 256
        p[i] = encode_byte(p[i], 0, k, offset(i, p[0], 0x54))
    p[8] = encode_byte(sum, 2, k, offset(8, p[0], 0))      # note: no jump-start

decode(p):
    k = xor_key(p[0])
    for i in 1..=8:
        p[i] = decode_byte(p[i], 0, k, offset(i, p[0], 0x54))
```

Decode is **not** a perfect inverse of encode at index 8: decode applies the
jump-start correction and `s1 = 0` to the checksum byte, while encode does not.
That asymmetry is deliberate in every known implementation — for the common case
`p0 = 0x00` the jump-start window does not apply, so the two agree, and the decoded
byte 8 comes out as `checksum + 2` (§4.3).

### 4.3 Checksum

```
checksum = (xor_key(p[0]) + p[1] + p[2] + p[3] + p[4] + p[5] + p[6] + p[7]) mod 256
```

That is: the XOR key plus the sum of the seven plaintext body bytes. The key byte
`p[0]` itself is *not* included; nor is the checksum slot.

Because the checksum byte is encoded with `s1 = 2`, a decoder reading back an
encoded packet observes `(checksum + 2) mod 256` at index 8.

---

## 5. V2 command set

Commands are per-family. `command | 0x80` marks a **held** (long-press) button.

### 5.1 On / off / night (all V2 families)

Command `0x01`. The argument encodes both the group and the desired state:

```
argument = group_id + (if state == OFF { num_groups + 1 } else { 0 })
```

For a 4-group remote (`FUT092`, `FUT091`):

| Argument | Meaning |
|---|---|
| `0` | all groups ON |
| `1`–`4` | group 1–4 ON |
| `5` | all groups OFF |
| `6`–`9` | group 1–4 OFF |

For the 8-group `FUT089`, the OFF block starts at `9` instead.

**Night mode** is `0x01 | 0x80` with the *OFF* argument for the group.

When parsing a received packet, the group byte at index 7 is unreliable for these
commands — derive the group from the argument instead.

### 5.2 RGB+CCT — FUT092, protocol ID `0x20`, 4 groups

| Command | Name | Argument |
|---|---|---|
| `0x01` | ON / OFF / night / scene speed | group arg (§5.1), or `0x0A` = speed up, `0x0B` = speed down |
| `0x02` | Colour (hue) | `0x5F + hue8`, where `hue8 = hue_degrees × 255 / 360` |
| `0x03` | Kelvin | `((100 - pct) × 2) + 0xCC` (mod 256). The scale runs `0x94, 0x92, … 0x00, … 0xCE, 0xCC` — `0x94` is coolest (0%), `0xCC` warmest (100%) |
| `0x04` | Brightness **and** saturation | brightness: `0x8F + pct` · saturation: `0x0D + pct` |
| `0x05` | Scene / mode | mode number, `0`–`8` (9 modes) |

Notes and gotchas:

- **`0x04` is overloaded.** Whether it sets brightness or saturation depends on the
  bulb's current mode (white vs. colour). The offsets differ (`0x8F` vs `0x0D`), so
  the ranges don't collide in practice — brightness lands in `0x8F…0xF3`, saturation
  in `0x0D…0x71`.
- **There is no explicit "white" command.** Sending a Kelvin command (`0x03`) is what
  drives the bulb out of colour mode into white.
- **Mode is lost when you change temperature.** Setting Kelvin forces white mode. To
  change temperature while staying in colour/scene mode you must re-send the hue or
  scene command afterwards. Likewise, saturation only applies in colour mode, so
  changing it from white mode requires: set hue → set saturation → restore mode.
- **Batched changes need a gap.** "On, warm, 40%" is three packets, and sending them
  back to back means only the first takes effect. See [§2.5](#25-distinct-commands-need-a-gap-between-them).
- **The Kelvin scale wraps through zero.** It is not a contiguous range: it counts
  down from `0x94` in steps of two, passes `0x00` at 74%, continues from `0xFE`,
  and stops at `0xCC`. Compute it, don't clamp it.
- Argument ranges tolerate a little slop; receivers typically allow ~`0x13` values of
  overshoot on either end and clamp to 0/100.

### 5.3 FUT089 (8-group RGB+CCT / "B8" panel), protocol ID `0x25`, 8 groups

A different, cleaner command numbering — **not** compatible with FUT092:

| Command | Name | Argument |
|---|---|---|
| `0x01` | ON / OFF / night / white / scene speed | group arg (§5.1); `0x12` = speed up, `0x13` = speed down, `0x14` = **white mode** |
| `0x02` | Colour (hue) | `hue8` (no offset) |
| `0x05` | Brightness | `pct` (0–100, no offset) |
| `0x06` | Scene / mode | mode number |
| `0x07` | Kelvin **or** saturation | `100 - pct` for both |

`0x07` is overloaded the same way `0x04` is on FUT092, but here the two share an
identical encoding, so the current bulb mode is the *only* disambiguator. Unlike
FUT092, FUT089 has a real white-mode command (`0x01` / `0x14`).

### 5.4 FUT091 (V2 CCT-only), protocol ID `0x21`, 4 groups

| Command | Name | Argument |
|---|---|---|
| `0x01` | ON / OFF / night | group arg (§5.1) |
| `0x02` | Brightness | `((100 - pct) × 2) + 0x97` |
| `0x03` | Kelvin | `(pct × 2) + 0xC5` |

### 5.5 Pairing and unpairing

There is no negotiation. A bulb enters pairing mode when it is **power-cycled**, and
for a few seconds afterwards it will adopt the `(device_id, group)` of the first
ON command it hears.

- **Pair**: power-cycle the bulb, then send `ON` for the target group within ~3 s.
- **Unpair** (factory reset): power-cycle, then send `ON` for **group 0 (all)**
  repeatedly — five times in quick succession is what implementations use.

Because pairing is "whoever shouts first wins", pick a `device_id` and keep it. Any
16-bit value works; there is no registry and no collision detection.

---

## 6. V1 (legacy) protocols

Included for reference — `pilight` does not implement these. V1 packets are **not**
obfuscated; the bytes go out as-is (after PL1167 framing and bit reversal).

### 6.1 RGBW (FUT096) — 7 bytes

Layout: `[0xB0, id_hi, id_lo, colour, brightness_and_group, button, sequence]`
(protocol byte `0xB0`, colour at index 3, button at index 5).

| Button | Code | Button | Code |
|---|---|---|---|
| All ON | `0x01` | All OFF | `0x02` |
| Group 1 ON / OFF | `0x03` / `0x04` | Group 2 ON / OFF | `0x05` / `0x06` |
| Group 3 ON / OFF | `0x07` / `0x08` | Group 4 ON / OFF | `0x09` / `0x0A` |
| Speed up / down | `0x0B` / `0x0C` | Disco mode | `0x0D` |
| Brightness | `0x0E` | Colour | `0x0F` |
| All max / min level | `0x11` / `0x12` | Group *n* max / min | `0x13 + 2(n-1)` / `0x14 + 2(n-1)` |

The "max level" buttons are the only way to force a group to white. Night mode is a
long press on the corresponding OFF button, which maps onto the "min level" codes.

### 6.2 CCT (FUT007) — 7 bytes

Every action is a distinct button code (command at index 4); there is no argument byte,
so brightness and temperature are **relative** (repeat the step command *N* times,
10 steps across the range).

| Button | Code | Button | Code |
|---|---|---|---|
| All ON / OFF | `0x05` / `0x09` | Group 1 ON / OFF | `0x08` / `0x0B` |
| Group 2 ON / OFF | `0x0D` / `0x03` | Group 3 ON / OFF | `0x07` / `0x0A` |
| Group 4 ON / OFF | `0x02` / `0x06` | Brightness up / down | `0x0C` / `0x04` |
| Temperature up / down | `0x0E` / `0x0F` | | |

### 6.3 RGB (FUT098) — 6 bytes

Colour at index 3, command at index 4. Groupless (one zone only).

| Button | Code | Button | Code |
|---|---|---|---|
| OFF / ON | `0x01` / `0x02` | Brightness up / down | `0x03` / `0x04` |
| Speed up / down | `0x05` / `0x06` | Mode up / down | `0x07` / `0x08` |

Pairing uses the "speed up" code (`0x05`).

### 6.4 FUT020 — 6 bytes

Groupless remote with its own radio config (§2.3).

| Button | Code |
|---|---|
| Colour | `0x00` |
| Brightness down | `0x01` |
| Mode switch | `0x02` |
| Brightness up | `0x03` |
| ON / OFF (toggle) | `0x04` |
| Colour / white toggle | `0x05` |

---

## 7. Test vectors

### 7.1 V2 round-trip (real capture, RGB+CCT)

```
encoded (on air) : 1B D9 ED 64 52 DD B3 63 1D
decoded          : 1B 20 81 64 02 51 2C 01 E4
```

Reading the decoded packet:

| Field | Value | Meaning |
|---|---|---|
| key | `0x1B` | non-zero key — this came from a real remote, not a hub |
| protocol id | `0x20` | RGB+CCT (FUT092) ✔ confirms the decode |
| device id | `0x8164` | |
| command | `0x02` | colour (hue) |
| argument | `0x51` | `0x51 - 0x5F = 0xF2` (mod 256) → hue ≈ 342° |
| sequence | `0x2C` | |
| group | `0x01` | group 1 |
| checksum | `0xE4` | = `(xor_key(0x1B) + Σ p[1..7] + 2) mod 256` = `(0x5D + 0x85 + 2)` ✔ |

Re-encoding the decoded packet reproduces the capture byte-for-byte. This exercises
the jump-start path (`0x1B < 0x54`, so no bump) and a non-zero key.

### 7.2 V2 encode from scratch

Plaintext — key `0x00`, RGB+CCT, device `0xBEEF`, "group 1 ON", sequence 0:

```
plain    : 00 20 BE EF 01 01 00 01 00
encoded  : 00 DB 33 C6 66 D1 BA 66 9F
re-decode: 00 20 BE EF 01 01 00 01 88     ← index 8 is checksum+2, as expected
```

### 7.3 Full nRF24 frame for §7.2

```
packet          : 00 DB 33 C6 66 D1 BA 66 9F
with length byte: 09 00 DB 33 C6 66 D1 BA 66 9F
crc16           : AC8F
nRF24 payload   : 90 00 DB CC 63 66 8B 5D 66 F9 F1 35   (12 bytes)
nRF24 address   : 8A 01 E9 C4 56                        (5 bytes, addr[0] first)
nRF24 channels  : 10, 41, 72
```

---

## 8. Practical notes

- **Everything is mod-256.** Rust's debug builds panic on integer overflow. Use
  `wrapping_add` / `wrapping_sub` / `wrapping_mul` throughout the encoder, or the
  first packet you build will abort the process.
- **The nRF24 is not a PL1167.** Reception in particular is fragile: the address-match
  trick assumes a fixed trailer, and the nRF24 has no way to recover from a
  near-miss. Transmission is far more reliable than reception.
- **Repeat aggressively** (§2.4). A single burst is routinely dropped.
- **The bulbs never acknowledge anything.** There is no read-back. Any state you show
  in a UI is state *you* are tracking, and it will drift if someone uses a physical
  remote. Sniffing the remotes' traffic is the only way to resync.
- **Group 0 means "all groups"** and is honoured by every family that has groups.

---

## Sources

- Chris Mullins, *Reverse engineering the new Milight/LimitlessLED 2.4 GHz protocol* —
  <https://blog.christophermullins.com/2017/03/18/reverse-engineering-the-new-milightlimitlessled-2-4-ghz-protocol/> —
  the original V2 obfuscation write-up (note the row-5 offset discrepancy, §4.2).
- sidoh, *esp8266_milight_hub* — <https://github.com/sidoh/esp8266_milight_hub> —
  the reference implementation. `lib/MiLight/V2RFEncoding.cpp`,
  `lib/MiLight/*PacketFormatter.*`, `lib/Radio/PL1167_nRF24.cpp`,
  `lib/Radio/MiLightRadioConfig.*`.
- henryk, *openmili* — <https://github.com/henryk/openmili> — the original
  PL1167-over-nRF24 emulation and CRC, and the V1 protocol work.
- henryk, *Reverse-Engineering the milight on-air protocol* —
  <https://hackaday.io/project/5888-reverse-engineering-the-milight-on-air-protocol>.
