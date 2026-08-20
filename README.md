# pilight

A MiLight / LimitlessLED bulb driver in Rust, meant to run on a Raspberry Pi with an
nRF24L01+ radio — talking to the bulbs directly over 2.4 GHz instead of going through
a MiLight WiFi gateway.

> **Status: working on real hardware.** Verified 2026-08-20 on a Raspberry Pi 3B+
> with an nRF24L01+, driving three FUT092 bulbs: registered over HTTP, discovered by
> Home Assistant, and set to colour, temperature and brightness on command. Every
> layer — protocol, radio, database, MQTT, API — exercised end to end against real
> bulbs.

## Why

The MiLight WiFi gateways are the usual way to control these bulbs, but they're
flaky, cloud-adjacent, and one more box to keep alive. The bulbs themselves listen on
a plain 2.4 GHz protocol that an nRF24L01+ can speak — a £2 radio module on a Pi's SPI
bus replaces the gateway entirely, with no cloud and no vendor app.

## Protocol

The on-air protocol is undocumented by the vendor and has been reverse-engineered by
the community. **[docs/protocol.md](docs/protocol.md)** is the full write-up: radio
configuration, PL1167 framing over an nRF24, the V2 obfuscation scheme, the command
tables for each bulb family, and verified test vectors.

The short version:

- Bulbs contain a **PL1167** transceiver. An **nRF24L01+** can imitate one by
  disabling its CRC and auto-ACK, and folding the PL1167 preamble/syncword/trailer
  into its 5-byte address.
- Two packet generations exist. **V1** (RGBW/CCT/RGB, 6–7 bytes) is plaintext.
  **V2** (RGB+CCT — FUT092, FUT089, FUT091 — 9 bytes) is obfuscated with a keyed XOR
  plus per-position additive offsets.
- A V2 packet is `[key, protocol_id, id_hi, id_lo, command, argument, sequence,
  group, checksum]`.
- Nothing is ever acknowledged. Commands are broadcast on three channels and repeated
  ~50 times.

`pilight` targets **V2 / RGB+CCT**.

## Usage

```rust
use pilight_proto::{GroupId, Nrf24Transceiver, RgbCctController};

let mut lamp = RgbCctController::builder(Nrf24Transceiver::open()?)
    .device_id(0xBEEF)          // pick one and keep it; there is no registry
    .group(GroupId::new(1, 4)?) // group 0 addresses all four at once
    .build()?;

lamp.on()?;
lamp.set_brightness(60)?;
lamp.set_hue(200)?;
lamp.set_kelvin(80)?;           // 0 = coolest, 100 = warmest
```

There's a CLI example that wraps the same API:

```sh
cargo run --example lamp -- --id 0xBEEF --group 1 on
cargo run --example lamp -- --id 0xBEEF --group 1 brightness 60
cargo run --example lamp -- --id 0xBEEF --group 1 hue 200
```

**Pairing**: power-cycle the bulb, then run `pair` within about three seconds. It
adopts whichever `(device_id, group)` it hears first. `unpair` factory-resets it the
same way.

## Layout

A Cargo workspace, split so the protocol code stays free of database, MQTT and
async dependencies:

```
crates/
├── pilight-proto/     The driver. No DB, no async, no tokio.
│   ├── encoder.rs     V2 obfuscation: keyed XOR + position offsets.
│   ├── packet.rs      The 9-byte V2 packet.
│   ├── framing.rs     PL1167 framing: length byte, CRC-16, bit reversal.
│   ├── remote.rs      The bulb families and their parameters.
│   ├── radio/         Hopping, repetition, the nRF24 and null backends.
│   └── controller/    Intents, the multi-lamp transmitter, RgbCctController.
├── pilight-db/        Postgres persistence (diesel-async).
│   ├── domain/        What the app works with: u8s, enums, no Diesel.
│   ├── models/        Row-shaped mirrors and the narrowing conversions.
│   └── repository/    Storage traits and their Pg implementations.
├── pilight-service/   LampService: the one place radio meets database.
├── pilight-mqtt/      Home Assistant bridge: discovery, topics, payloads.
├── pilight-api/       HTTP API: routes, DTOs, status mapping, auth.
└── pilightd/          The daemon that runs all of it.
```

The seams are deliberate: `Transceiver` for hardware, the repository traits for
storage. Both have test doubles, so everything above them runs without a Pi.

`LampService` is the hinge. One radio serves many lamps, so it holds the
transmitter behind a mutex and, for each command, looks the lamp up, takes a
sequence number, transmits **on a blocking thread** (a burst is a few hundred
milliseconds — it must not sit on the async runtime), revises the stored state, and
records the attempt. The MQTT bridge and the HTTP API are both thin skins over it.

They stay in step through a broadcast channel rather than knowing about each other:
`LampService` emits `LampEvent`s and the bridge reacts by announcing, retracting or
republishing. That is why a lamp registered over HTTP appears in Home Assistant, and
why a brightness change made with `curl` shows up on the dashboard.

Both interfaces also share `StateChange`, which turns "on, half brightness, blue"
into correctly ordered packets — one implementation, so they cannot drift apart on a
question the bulbs care about.

## HTTP API

For anything that is not Home Assistant — a dashboard, a script, `curl`. It is also
the only way to register a lamp.

```sh
BASE=http://localhost:8080

# Register a lamp. device_id is yours to pick; there is no registry.
curl -s -X POST $BASE/api/v1/lamps -H 'content-type: application/json' -d '{
  "name": "Couch", "room": "Living room",
  "remote_type": "rgb_cct", "device_id": 48879, "group": 1
}'

# Drive it. Absent fields are left alone; several can change at once.
curl -s -X PUT $BASE/api/v1/lamps/$ID/state -H 'content-type: application/json' \
  -d '{"power": true, "brightness": 60, "hue": 200, "saturation": 80}'
```

| Method | Path | Does |
|---|---|---|
| `GET` | `/health` | Liveness. No token needed. |
| `GET` | `/api/v1/lamp-types` | Which bulb families exist, and which are drivable. |
| `GET` | `/api/v1/lamps` | Every lamp with its state. Paginated. |
| `POST` | `/api/v1/lamps` | Register a lamp. |
| `GET` | `/api/v1/lamps/{id}` | One lamp. |
| `PATCH` | `/api/v1/lamps/{id}` | Rename, or move room. |
| `DELETE` | `/api/v1/lamps/{id}` | Forget a lamp. |
| `PUT` | `/api/v1/lamps/{id}/state` | Change what it is doing. |
| `GET` | `/api/v1/lamps/{id}/history` | What we sent, and whether it worked. |
| `POST` | `/api/v1/lamps/{id}/pair` | Adopt a power-cycled bulb. |
| `POST` | `/api/v1/lamps/{id}/unpair` | Factory-reset a power-cycled bulb. |

The full body for `PUT .../state`. Every field is optional; omitting one leaves that
setting alone.

| Field | Type | Means |
|---|---|---|
| `power` | bool | On or off. |
| `brightness` | 0–100 | Percent. Survives a mode switch. |
| `hue` | 0–359 | Degrees. Puts the bulb in colour mode. |
| `saturation` | 0–100 | Percent. Only takes effect in colour mode. |
| `kelvin` | 0–100 | 0 is coolest, 100 warmest. Drags the bulb into white mode. |
| `scene` | 0–8 | One of the nine built-in scenes. Home Assistant sees these as effects. |
| `night_mode` | bool | The dimmest setting the bulb has. A mode, not a brightness. |

The two list endpoints — `/lamps` and `/lamps/{id}/history` — take `?limit=` and
`?offset=`. `limit` defaults to **50** and is clamped to **1–500** rather than
trusted, so a client cannot ask for the whole command log by accident; a negative
`offset` is treated as zero. Both are echoed back in `meta`.

`GET /health` always answers **200**, describing what is wrong in the body rather
than in the status code — a monitor that only sees a `503` cannot tell a dead
database from a dead process. `status` is `ok` or `degraded`:

```json
{ "success": true, "error": null,
  "data": { "status": "ok", "database": true, "lamps": 3, "version": "0.1.0" } }
```

Every response has the same envelope, so a client can read `success` before deciding
what to do with the body:

```json
{ "success": true,  "data": { "…": "…" }, "error": null }
{ "success": true,  "data": [], "error": null, "meta": { "total": 3, "limit": 50, "offset": 0 } }
{ "success": false, "data": null, "error": "no lamp with id …" }
```

Status codes are chosen to point at whoever can fix the problem:

| Code | When |
|---|---|
| `400` | The request asked for something impossible — 200% brightness, group 5 on a four-group family. Caught before anything reaches the air. |
| `404` | No such lamp. |
| `409` | A lamp is already registered at that `(family, device_id, group)`. The body was fine; the world disagreed. |
| `501` | The family is documented but has no command layer yet. |
| `502` | The radio failed. It is upstream of us, so a `500` would send you looking in the wrong place. |
| `503` | The database is unreachable. Retry. |

### Authentication

Set `PILIGHT_API_TOKEN` and every `/api/v1` request needs
`Authorization: Bearer <token>`. `/health` never does, so a monitor can probe it
freely.

Leaving it unset means **no authentication**, and the daemon says so in a warning at
startup. That is a defensible default for a box on your own LAN and a bad one for
anything reachable from outside it — `PILIGHT_API_ADDR` defaults to `0.0.0.0:8080`,
so bind it to `127.0.0.1` or set a token if the machine is exposed.

There is no rate limiting: this is a handful of lamps on a home network, and the
radio is a far tighter bottleneck than the HTTP layer.

## Home Assistant

`pilightd` publishes [MQTT discovery](https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery)
messages, so the lamps appear in Home Assistant with no YAML.

```sh
docker compose up -d                       # Postgres + a local Mosquitto
export DATABASE_URL=postgres://pilight:pilight@localhost:55432/pilight
export PILIGHT_MQTT_HOST=homeassistant.local
cargo run --release -p pilightd
```

Register a lamp through [the HTTP API](#http-api) and it shows up as a light with
brightness, a colour wheel, a colour temperature slider and the nine scenes as
effects. Renames, deletions and state changes made over HTTP propagate immediately;
the bridge follows the service's event stream.

### Topics

```text
homeassistant/light/pilight_<uuid>/config   discovery, retained
homeassistant/status                        HA's birth message — we listen
pilight/status                              our availability, retained + LWT
pilight/lamp/<uuid>/state                   lamp state, retained
pilight/lamp/<uuid>/set                     commands
```

Both prefixes are configurable. Three details that are easy to get wrong and are
handled here:

- **A last will** marks the bridge offline if the daemon dies, so HA greys the
  lights out instead of showing state that has quietly stopped updating.
- **HA's birth message** triggers a full re-announce. Discovery messages are
  retained, but a Home Assistant that has forgotten an entity only gets it back if
  someone republishes.
- **`optimistic` is off** and `transition`/`flash` are disabled, because the bulbs
  can do none of those things.

### Unit conversions

| Home Assistant | Protocol |
|---|---|
| Brightness 0–255 | 0–100 percent |
| Colour temperature in Kelvin | 0–100, where 0 is coolest |
| `hs` colour, hue 0–360, saturation 0–100 | hue degrees, saturation percent |
| Effect `scene_0` … `scene_8` | scene 0–8 |

The Kelvin range defaults to **2700–6500 K**. MiLight does not publish it — that is
the range these bulbs are sold as, and what other implementations assume. Override
with `PILIGHT_MIN_KELVIN` / `PILIGHT_MAX_KELVIN` if your bulbs differ.

One HA message can ask for several things at once ("on, half brightness, blue"), and
the protocol cannot say that in one packet. The bridge expands it into intents and
**orders them deliberately**: temperature before hue (a Kelvin command drags the bulb
out of colour mode), hue before saturation (saturation only applies in colour mode),
brightness last (it survives a mode switch). Getting that order wrong is why colours
"don't stick".

### Configuration

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | — | **Required.** Postgres connection string. |
| `PILIGHT_RADIO` | `nrf24` | `none` discards every transmission — useful for testing the HA side on a laptop. It warns loudly. |
| `PILIGHT_MQTT_HOST` / `_PORT` | `localhost` / `1883` | Broker. |
| `PILIGHT_MQTT_USERNAME` / `_PASSWORD` | unset | Broker credentials. |
| `PILIGHT_MQTT_CLIENT_ID` | `pilight` | Client id on the broker. Two instances must differ. |
| `PILIGHT_MQTT_PREFIX` | `pilight` | Our topic prefix. |
| `PILIGHT_MQTT_DISCOVERY_PREFIX` | `homeassistant` | HA's discovery prefix. |
| `PILIGHT_MIN_KELVIN` / `PILIGHT_MAX_KELVIN` | `2700` / `6500` | Bulb range. |
| `PILIGHT_RADIO_REPEATS` | `50` | Bursts per command. |
| `PILIGHT_COMMAND_GAP_MS` | `300` | Pause between two *different* commands. See below. |
| `PILIGHT_API_ADDR` | `0.0.0.0:8080` | Where the HTTP API listens. |
| `PILIGHT_API_TOKEN` | unset | Bearer token for `/api/v1`. Unset means no auth. |
| `RUST_LOG` | `info` | Log filter. |

`PILIGHT_COMMAND_GAP_MS` is the one worth understanding before you change it. It is
*not* the gap between repeats of a single command — that lives in the radio layer.
It is the pause between two distinct commands, and without it a bulb acts on the
first and silently ignores the rest, so "on, warm, 40%" appears to do nothing at
all. 300 ms is enough on FUT092 and 0 is definitely not; the exact threshold has
not been characterised. [`docs/protocol.md` §2.5](docs/protocol.md#25-distinct-commands-need-a-gap-between-them)
has the full story.

TLS to the broker is behind the `tls` feature, off by default: rustls pulls in
`aws-lc-sys`, which needs a C toolchain and makes cross-compiling for a Pi
considerably more annoying. HA's Mosquitto add-on is plain 1883 on the LAN.

## Database

Lamps, the families they belong to, what we last told them, and an audit trail of
what we sent:

| Table | Holds |
|---|---|
| `lamp_types` | One row per bulb family. **Not hand-maintained** — it's a projection of `RemoteType::ALL`, upserted at startup, so adding a family needs no migration. |
| `lamps` | A paired `(family, device_id, group)` with a name and room. That triple is unique: two lamps sharing it would be the same bulb. |
| `lamp_states` | Last known state per lamp, plus the next V2 sequence byte. |
| `lamp_commands` | Append-only log of what was transmitted, by whom, and whether it worked. |

**`lamp_states` is a belief, not a reading.** MiLight bulbs never acknowledge
anything and cannot be queried, so every column reflects the last command we sent.
It goes stale the moment someone picks up a physical remote. Optional columns are
`NULL` until the corresponding command has been sent at least once — a freshly
paired bulb has a colour, but we do not know what it is.

Persisting `next_sequence` closes a gap: the sequence byte used to live only in the
controller, so every restart began again at zero. Consecutive distinct commands are
supposed to carry distinct sequence numbers, and a restart made that stop being true.
(What a bulb actually does with a replayed number is not something I've been able to
verify — the protocol notes say only that the byte must stay fixed across a repeat
burst and change between commands.) `take_sequence` bumps it in a single
`UPDATE ... RETURNING`, so the HTTP API and an MQTT handler can send concurrently
without handing the same byte to one bulb.

```rust
use pilight_db::{Repositories, build_pool, run_migrations};
use pilight_db::repository::{LampRepository, LampTypeRepository};
use pilight_db::domain::NewLamp;

let pool = build_pool(&std::env::var("DATABASE_URL")?)?;
run_migrations(&pool).await?;                  // idempotent; safe every start

let repos = Repositories::new(pool);
repos.types.sync_from_driver().await?;         // catalogue follows the driver

let lamp = repos.lamps.create(NewLamp {
    name: "Couch".into(),
    room: Some("Living room".into()),
    remote_type: RemoteType::RgbCct,
    device_id: 0xBEEF,
    group: 1,
}).await?;                                     // creates its state row too
```

Everything speaks `diesel-async` over `AsyncPgConnection`, including migrations
(via `AsyncMigrationHarness`). That means **no `libpq`** — nothing to install on
the Pi. It does mean migrations need the multi-threaded tokio runtime, because the
harness uses `block_in_place`; under `current_thread`, wrap them in
`spawn_blocking`.

### Running a database

```sh
docker compose up -d postgres
export DATABASE_URL=postgres://pilight:pilight@localhost:55432/pilight
export PILIGHT_TEST_DATABASE_URL=postgres://pilight:pilight@localhost:55432/pilight_test
```

After changing a migration, regenerate Diesel's view of the schema:

```sh
cd crates/pilight-db && diesel print-schema > src/schema.rs
```

## What's verified

`cargo test --workspace` runs 235 tests. The count does not change when Postgres and
a broker come up: the integration tests skip from *inside* a passing test, so the
same 235 run either way — they just assert a great deal more when the services are
there. See [Build](#build) for how to tell the two apart.

Protocol (all against a real capture or the reference implementation):

- **A real captured packet round-trips.** `1B D9 ED 64 52 DD B3 63 1D` decodes to a
  well-formed RGB+CCT packet and re-encodes byte-for-byte.
- **The obfuscation survives all 256 key values**, including the `[0x54, 0xD3]`
  jump-start window that used to overflow.
- **CRC, bit reversal and framing**: the full 12-byte nRF24 payload is asserted
  literally.
- **Intent encoding and ordering**, through a recording `Transceiver`.

Persistence (against a real Postgres):

- **Migrations are idempotent**; the `lamp_types` catalogue tracks `RemoteType::ALL`.
- **Creating a lamp creates its state row in the same transaction**; a rejected lamp
  leaves nothing behind; deleting one cascades.
- **Sequence numbers** are handed out in order, wrap at 255, and 32 concurrent tasks
  each get a distinct one.
- **Narrowing conversions reject rather than truncate** — a `device_id` of -1 or
  65536 is an error, not a wrapped `u16`.

Home Assistant (against a real Mosquitto, end to end):

- **A lamp appears with a usable config**, checked against the topics the bridge
  actually listens on.
- **A command published the way HA publishes it reaches the radio**, and state comes
  back.
- **A combined command produces one packet per intent, per channel** — which is what
  pins down the ordering.
- **HA's birth message re-announces**, including a lamp added after startup.
- **Malformed JSON is ignored, not fatal**; **shutdown publishes `offline`**.

HTTP API (the real router, a real Postgres, a counting radio):

- **Status codes are what they claim**: 409 on a duplicate address, 400 on an
  impossible value, 404 everywhere a lamp is missing, 400 on a malformed UUID.
- **An impossible value never reaches the air** — asserted by packet count, not by
  reading the error.
- **`PATCH` tells an absent field from an explicit `null`**, so renaming a lamp does
  not silently clear its room.
- **The sequence byte does not leak** into any response.
- **A combined change is ordered so the colour survives the temperature change.**
- **Auth is enforced** when a token is configured, and `/health` is exempt.
- **API changes reach the event stream**, which is what makes MQTT follow them.

I also ran the daemon for real: registered a lamp with `curl`, changed its state, and
confirmed the retained discovery and state messages arrived on the broker.

### On real hardware

The automated tests all run against `NullTransceiver`, which counts packets rather
than transmitting them — they prove everything *up to* the radio. The rest was
confirmed by hand on 2026-08-20, on a Pi 3B+ with an nRF24L01+ on SPI0:

- **`radio-check` returned a clean register dump** — every value its documented
  power-on default, `STATUS 0x0E`, `SETUP_AW 0x03`, and the write/read-back passed.
- **Bulbs obeyed.** Three lamps registered over HTTP appeared in Home Assistant from
  their retained discovery messages, and responded to hue, saturation, brightness,
  colour temperature and on/off.
- **An independent decoder confirmed the packets.** The esp8266_milight_hub already
  on the network sniffed a transmission and republished it as
  `milight/states/0x2/rgb_cct/1 {"bulb_mode":"color","color":{"r":0,"g":255,"b":0}}`
  — our device id, type, group and colour, decoded by someone else's implementation.

That single command confirms a lot of derived-from-reading work at once: the V2
obfuscation and checksum, the CRC-16 and PL1167 framing, the syncword-to-address
derivation, the `+2` channel offset, and the hue/saturation encodings.

It also settles the **chip-select decision** — CSN driven by the Pi's hardware CE0
with a no-op pin handed to the driver. That was reasoning, not evidence, until the
register dump and then the bulb proved it.

And it found a bug nothing else could have. Batched changes ("on, warm, 40%") did
nothing at all, because distinct commands were being sent back to back with no
pause and the bulb acted only on the first. Every packet was transmitted correctly
— the sniffer proved it — so no amount of testing against `NullTransceiver` would
have caught it. See `docs/protocol.md` §2.5; it is not documented anywhere else.

The daemon cross-compiles to a static `aarch64` binary with no C toolchain, and runs
on the Pi with nothing installed alongside it.

Without the test services configured, the integration tests skip, and cargo captures
the skip notice — so the run looks green, and the test count is unchanged. That is
why the number above cannot tell you whether they really ran; `-- --nocapture` can.
All three harnesses turn the skip into a hard failure when `CI` is set, so a silent
pass can only happen on a laptop.

## What changed from the original

The V2 encoder was already algorithmically correct, but nothing had ever reached the
air. Two bugs and one large gap:

1. **The encoder panicked in debug builds.** Every operation in the V2 scheme is
   mod-256, but the code used plain `+`/`-`/`+=`, so `encode_packet` aborted on the
   first packet at `src/encoder.rs:77` (pre-split layout; now
   `crates/pilight-proto/src/encoder.rs`). In `--release` it wrapped and produced the
   right answer — which is exactly the kind of asymmetry that reads as "the protocol
   is wrong". Now `wrapping_add`/`wrapping_sub` throughout, with a test that walks
   every key value.
2. **`sequence_num += 1` would panic after 255 commands.** Now wrapping, with a test
   that sends 300.
3. **There was no radio.** No dependencies, no SPI, no framing, no channel hopping;
   `V2LampController::command` built a packet, never encoded it, and dropped it.

Smaller: the unused `V2Encoder<const PACKET_SIZE: u8>` parameter is gone, the
`Command`/`Argument` marker traits are replaced by concrete enums, and the mutating
`encode_packet(&mut [u8; 9])` API is now a value-returning `V2Encoder::encode`.

## Hardware

| Part | Notes |
|---|---|
| Raspberry Pi | Any model with the 40-pin header. `rppal` 0.22 handles the Pi 5's RP1 as well as older BCM models. |
| nRF24L01+ | The **+** matters. A module with an external antenna helps. |
| 10–100 µF capacitor | Across the module's VCC/GND. **Not optional** — these modules brown out on transmit, and the symptom looks exactly like a protocol bug. |

The nRF24 is a **3.3 V** part. VCC must come from the Pi's 3.3 V rail, never 5 V.
The logic pins are 5 V-tolerant; the supply is not.

### Wiring

| nRF24L01+ | Pi header pin | BCM | Signal |
|---|---|---|---|
| GND | 6 | — | GND |
| VCC | 1 | — | 3.3 V |
| CE | 22 | 25 | Chip enable |
| CSN | 24 | 8 | SPI0 CE0 |
| SCK | 23 | 11 | SCLK |
| MOSI | 19 | 10 | MOSI |
| MISO | 21 | 9 | MISO |
| IRQ | — | — | unused |

CSN is driven by the Pi's **hardware** chip-select rather than in software: rppal
asserts CE0 for exactly the duration of each SPI transfer, which is precisely the
framing an nRF24 command needs. `--ce` and `--clock` move the CE pin and the clock
if you need to.

### Pi setup

1. **Enable SPI.** `sudo raspi-config` → Interface Options → SPI, or add
   `dtparam=spi=on` to `/boot/firmware/config.txt` (that path, not the older
   `/boot/config.txt`). Reboot. You should then have `/dev/spidev0.0`.
2. **Permissions.** Add yourself to both groups and log back in:
   ```sh
   sudo usermod -aG spi,gpio $USER
   ```
   Recent Raspberry Pi OS sets up udev rules so the `gpio` group can reach
   `/dev/gpiomem` and `/dev/gpiochipN` without root. If you get permission errors,
   that usually means an older image — update it rather than reaching for `sudo`.
3. **Build on your workstation, not on the Pi.** See
   [Cross-compiling](#cross-compiling) — it takes seconds instead of the best part
   of an hour, and a Pi 3B+ has 1 GB of RAM to link in.

### First contact

Before running the daemon, check the radio on its own:

```sh
cargo run --release -p pilight-proto --example radio-check
```

It reads the nRF24's registers over raw SPI — deliberately not through the driver,
so it can tell you whether the *link* works independently of whether the *protocol*
does. Those are the two questions a set of unlit bulbs leaves you with, and they
have completely different fixes.

```text
Registers:
  CONFIG       0x00  08
  EN_AA        0x01  3F
  SETUP_AW     0x03  03
  STATUS       0x07  0E
  TX_ADDR      0x10  E7 E7 E7 E7 E7
  ...

nRF24L01+ responding, registers read and write cleanly
```

All `00` means nothing is driving MISO — power, ground, or the MISO line. All `FF`
means MISO is floating. Either way the tool says so and lists what to check, in the
order it is usually wrong. It also writes a scratch value and reads it back, so a
dead MOSI is caught too, then restores it.

Once that passes, put something on the air and watch a bulb:

```sh
# Power-cycle the bulb first, then run this within a few seconds to pair it.
cargo run --release -p pilight-proto --example radio-check -- \
    --transmit --id 0xBEEF --group 1
```

It cycles ON → 50% → OFF every two seconds. If the register dump is clean but the
bulb does nothing, the problem is in the protocol or the pairing, not the wiring —
and `docs/protocol.md` is where to look.

## Build

```sh
cargo test --workspace          # 235 tests; no hardware, no services
cargo clippy --workspace --all-targets
cargo build --release
```

To include the integration tests:

```sh
docker compose up -d
export PILIGHT_TEST_DATABASE_URL=postgres://pilight:pilight@localhost:55432/pilight_test
export PILIGHT_TEST_MQTT_HOST=localhost PILIGHT_TEST_MQTT_PORT=51883
cargo test --workspace          # the same 235, now actually exercising the services
```

To run the whole thing on a machine with no radio:

```sh
DATABASE_URL=postgres://pilight:pilight@localhost:55432/pilight \
PILIGHT_RADIO=none PILIGHT_MQTT_PORT=51883 \
cargo run -p pilightd --no-default-features
```

### Cross-compiling

Nothing in this workspace links C — no `libpq`, no OpenSSL, no `ring`. That makes
cross-compiling unusually easy: Rust's bundled `rust-lld` and the musl target's
self-contained libc are the whole toolchain, so there is no Docker image, no
`cross`, and no `gcc-aarch64-linux-gnu` to install.

```sh
rustup target add aarch64-unknown-linux-musl
cargo build --release --target aarch64-unknown-linux-musl
```

`.cargo/config.toml` points that target at `rust-lld` already. The result is a
statically linked `aarch64` binary with no runtime dependencies:

```text
pilightd      7.3M   ELF 64-bit, ARM aarch64, statically linked
radio-check   812K
```

`scp` it to the Pi and run it. Nothing to install on the other end — no Rust
toolchain, no shared libraries, not even a matching glibc.

For reference, the same build on a Pi 3B+ takes the better part of an hour and can
run out of memory while linking. On a workstation it is under 30 seconds.

> **One musl caveat.** musl has no NSS, so a static binary cannot resolve `.local`
> (mDNS) hostnames. If you want to point `PILIGHT_MQTT_HOST` at
> `homeassistant.local` rather than an IP, either add an `/etc/hosts` entry on the
> Pi, or build against glibc instead — `sudo pacman -S aarch64-linux-gnu-gcc`, then
> `--target aarch64-unknown-linux-gnu`, which `.cargo/config.toml` also covers.

If you install the 32-bit Raspberry Pi OS image instead, the target is
`armv7-unknown-linux-musleabihf`.

Building natively on the Pi still works — `cargo build --release` — it is just slow.

### Feature flags

All default-on features are for the Pi; turning them off is how you build for a
machine that has no radio.

| Crate | Feature | Default | Does |
|---|---|---|---|
| `pilightd` | `nrf24` | on | Drive a real nRF24L01+. Without it the daemon only has the null radio, and needs `PILIGHT_RADIO=none`. |
| `pilightd` | `tls` | off | TLS to the broker; re-exports `pilight-mqtt/tls`. |
| `pilight-proto` | `nrf24` | on | The `rppal` + `embedded-nrf24l01` hardware backend. Off, the crate is pure protocol and builds anywhere. |
| `pilight-proto` | `serde` | off | `Serialize`/`Deserialize` for `RemoteType`. The db and HTTP layers turn it on. |
| `pilight-mqtt` | `tls` | off | rustls for the broker connection. Off by default because it pulls in `aws-lc-sys`, which needs a C toolchain and makes cross-compiling considerably more annoying. |

### Dependencies

| Crate | Why |
|---|---|
| [`rppal`](https://crates.io/crates/rppal) 0.22 | Pi SPI and GPIO. Its `hal` feature provides the embedded-hal 0.2 impls the radio driver needs. |
| [`embedded-nrf24l01`](https://crates.io/crates/embedded-nrf24l01) 0.2 | nRF24L01+ register driver. Chosen because it exposes `set_crc`, `set_auto_ack` and `set_auto_retransmit` — most Arduino-derived drivers hide exactly the knobs this protocol has to turn. |
| [`embedded-hal`](https://crates.io/crates/embedded-hal) 0.2 | Renamed to `embedded-hal-0-2`. The trait the two above agree on. |
| [`diesel`](https://crates.io/crates/diesel) 2.3 | Query DSL and derives. `postgres_backend` only, **not** `postgres` — that would link libpq. |
| [`diesel-async`](https://crates.io/crates/diesel-async) 0.9 | `AsyncPgConnection` over pure-Rust tokio-postgres, deadpool pooling, and the async migration harness. |
| [`rumqttc`](https://crates.io/crates/rumqttc) 0.25 | MQTT client, `default-features = false` to keep TLS opt-in. |
| [`axum`](https://crates.io/crates/axum) 0.8 | HTTP. `tower-http` adds request tracing and trailing-slash normalisation. |
| [`async-trait`](https://crates.io/crates/async-trait) | Keeps the repository traits object-safe. |

Rust 1.85 or newer (edition 2024); developed against 1.96.

## Deploying

`deploy/` has a systemd unit and a commented environment file. The short version:

```sh
# On the Pi, once.
sudo apt install postgresql
sudo -u postgres createuser --pwprompt pilight
sudo -u postgres createdb --owner=pilight pilight
sudo useradd --system --no-create-home --shell /usr/sbin/nologin pilight
sudo usermod -aG spi,gpio pilight

# From your workstation.
cargo build --release --target aarch64-unknown-linux-musl
scp target/aarch64-unknown-linux-musl/release/pilightd pi@raspberrypi:/tmp/
scp deploy/pilightd.{service,env.example} pi@raspberrypi:/tmp/

# Back on the Pi.
sudo install -m 0755 /tmp/pilightd /usr/local/bin/pilightd
sudo install -d -m 0750 /etc/pilight
sudo install -m 0640 /tmp/pilightd.env.example /etc/pilight/pilightd.env
sudo chown root:pilight /etc/pilight/pilightd.env
sudo $EDITOR /etc/pilight/pilightd.env          # at minimum: DATABASE_URL, MQTT host
sudo install -m 0644 /tmp/pilightd.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now pilightd
journalctl -u pilightd -f
```

The daemon creates its own tables on first start, so there is no migration step.
The *database* has to exist first; the schema does not.

### Settings

Only `DATABASE_URL` has no default — without it the daemon exits and says so.

| Variable | Default | Set it when |
|---|---|---|
| `DATABASE_URL` | — | **Always.** Postgres connection string. |
| `PILIGHT_MQTT_HOST` | `localhost` | The broker is elsewhere — usually your Home Assistant host. |
| `PILIGHT_MQTT_PORT` | `1883` | Non-standard port. |
| `PILIGHT_MQTT_USERNAME` / `_PASSWORD` | unset | The broker wants credentials. HA's Mosquitto add-on does. |
| `PILIGHT_MQTT_PREFIX` | `pilight` | It collides with something. |
| `PILIGHT_MQTT_DISCOVERY_PREFIX` | `homeassistant` | You changed HA's discovery prefix. |
| `PILIGHT_MQTT_CLIENT_ID` | `pilight` | Two instances share a broker. |
| `PILIGHT_API_TOKEN` | unset | **Read the note below.** |
| `PILIGHT_API_ADDR` | `0.0.0.0:8080` | You want it bound to `127.0.0.1`, or a different port. |
| `PILIGHT_RADIO` | `nrf24` | `none` to run without hardware. |
| `PILIGHT_RADIO_REPEATS` | `50` | Commands get missed (raise) or feel slow (lower). |
| `PILIGHT_COMMAND_GAP_MS` | `300` | Batched changes are being dropped (raise), or feel sluggish (lower). Do not set it to 0. |
| `PILIGHT_MIN_KELVIN` / `PILIGHT_MAX_KELVIN` | `2700` / `6500` | Your bulbs cover a different range. |
| `RUST_LOG` | `info` | Debugging: `pilightd=debug,pilight_mqtt=debug`. |

> **The API listens on `0.0.0.0:8080` with no authentication by default.** That is
> fine behind a trusted LAN and wrong for anything else. Set `PILIGHT_API_TOKEN`
> (`openssl rand -hex 32`), bind `PILIGHT_API_ADDR` to `127.0.0.1`, or both.
> pilightd logs a warning at startup when no token is set.

### Before the first run

Beyond the settings, three things have to be true on the Pi:

1. **SPI enabled** — `dtparam=spi=on` in `/boot/firmware/config.txt`, then reboot.
   You should have `/dev/spidev0.0`.
2. **The service user is in `spi` and `gpio`** — the unit does this with
   `SupplementaryGroups`, but the groups must exist, which they do once SPI is on.
3. **The radio actually answers** — run `radio-check` before starting the daemon.
   It is the difference between a clear readout and an unexplained silence.

On a Pi 3B+ with 1 GB of RAM, Postgres' defaults are a little generous. If memory
gets tight, `shared_buffers = 64MB` and `max_connections = 20` in
`postgresql.conf` are plenty — pilightd's pool only asks for 8 connections.

## Still missing

- [ ] **A UI.** Everything is `curl` or Home Assistant.
- [ ] **Receive path** — sniff physical remotes so stored state stops drifting.
      `CommandSource::Sniffer` is reserved for it. This is the single biggest gap:
      until it exists, state is a record of intent, not of reality.
- [ ] The other V2 families: FUT089 (8 groups, `0x25`) and FUT091 (`0x21`). Radio
      config, framing and the catalogue already know about them; only a command
      layer is missing, and both the service and the API refuse them explicitly
      rather than mis-driving them.
- [ ] V1 families (RGBW, CCT, RGB, FUT020) — documented but not implemented
- [ ] Mode-restore around the overloaded commands. Setting Kelvin drops the bulb into
      white mode, so hue/scene has to be re-sent. `StateChange` orders a *single*
      request correctly, but a bare `{"kelvin": 40}` still loses the colour;
      `lamp_states.bulb_mode` now knows enough to put it back.
- [ ] Scheduling — `CommandSource::Schedule` exists and nothing emits it.
- [ ] TLS for the HTTP API. Put it behind a reverse proxy for now.

## Prior art

- [sidoh/esp8266_milight_hub](https://github.com/sidoh/esp8266_milight_hub) — the
  reference implementation, C++ on an ESP8266. The protocol doc here is largely
  derived from reading it.
- [henryk/openmili](https://github.com/henryk/openmili) — the original
  PL1167-over-nRF24 emulation and V1 protocol work.
- [Chris Mullins' V2 write-up](https://blog.christophermullins.com/2017/03/18/reverse-engineering-the-new-milightlimitlessled-2-4-ghz-protocol/)
  — how the V2 obfuscation was cracked.

## License

Not yet chosen. `Cargo.toml` has a commented-out `license` field ready for whatever
you pick; it needs filling in before `cargo publish` would accept the crate.
