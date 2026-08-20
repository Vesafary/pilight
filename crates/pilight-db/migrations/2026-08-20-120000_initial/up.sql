-- pilight initial schema.
--
-- Note on truthfulness: MiLight bulbs never acknowledge anything and cannot be
-- queried. Everything in `lamp_states` is what we last *told* a bulb, not what it
-- is actually doing. It drifts the moment someone picks up a physical remote.

-- Keep `updated_at` honest without every caller having to remember it.
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;


-- One row per bulb family. Seeded from pilight_proto::RemoteType; the
-- `lamp_types_match_the_driver` test asserts the two stay in step.
CREATE TABLE lamp_types (
    id                  SMALLINT PRIMARY KEY,
    slug                TEXT     NOT NULL UNIQUE,
    display_name        TEXT     NOT NULL,
    -- 1 = plaintext packets, 2 = obfuscated packets.
    protocol_generation SMALLINT NOT NULL CHECK (protocol_generation IN (1, 2)),
    -- Byte 1 of a V2 packet. NULL for V1 families, which have no such field.
    protocol_id         SMALLINT NULL     CHECK (protocol_id BETWEEN 0 AND 255),
    -- 0 means the remote is groupless and drives a single zone.
    num_groups          SMALLINT NOT NULL CHECK (num_groups BETWEEN 0 AND 8),
    -- Whether pilight can currently drive this family, as opposed to merely
    -- having it documented.
    driver_supported    BOOLEAN  NOT NULL DEFAULT FALSE,
    CONSTRAINT protocol_id_iff_v2 CHECK (
        (protocol_generation = 2) = (protocol_id IS NOT NULL)
    )
);


-- A lamp is a paired (family, device id, group). That triple is what the bulb
-- actually listens for; the name and room are ours.
CREATE TABLE lamps (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT        NOT NULL CHECK (length(trim(name)) > 0),
    room         TEXT        NULL     CHECK (room IS NULL OR length(trim(room)) > 0),
    lamp_type_id SMALLINT    NOT NULL REFERENCES lamp_types (id) ON DELETE RESTRICT,
    -- Postgres has no unsigned types; the driver's device id is a u16.
    device_id    INTEGER     NOT NULL CHECK (device_id BETWEEN 0 AND 65535),
    -- Group 0 addresses every group of that device id at once.
    group_id     SMALLINT    NOT NULL CHECK (group_id BETWEEN 0 AND 8),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Two lamps cannot share an address: they would be the same bulb.
    CONSTRAINT lamps_address_key UNIQUE (lamp_type_id, device_id, group_id)
);

CREATE INDEX lamps_room_idx ON lamps (room) WHERE room IS NOT NULL;

CREATE TRIGGER lamps_set_updated_at
    BEFORE UPDATE ON lamps
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();


-- Last known state, one row per lamp. Created alongside the lamp.
CREATE TABLE lamp_states (
    lamp_id       UUID        PRIMARY KEY REFERENCES lamps (id) ON DELETE CASCADE,
    power         BOOLEAN     NOT NULL DEFAULT FALSE,
    bulb_mode     TEXT        NOT NULL DEFAULT 'white'
                              CHECK (bulb_mode IN ('white', 'color', 'scene', 'night')),
    -- All NULL until the corresponding command has been sent at least once.
    brightness    SMALLINT    NULL CHECK (brightness BETWEEN 0 AND 100),
    hue           SMALLINT    NULL CHECK (hue        BETWEEN 0 AND 359),
    saturation    SMALLINT    NULL CHECK (saturation BETWEEN 0 AND 100),
    kelvin        SMALLINT    NULL CHECK (kelvin     BETWEEN 0 AND 100),
    scene         SMALLINT    NULL CHECK (scene      BETWEEN 0 AND 8),
    -- The V2 sequence byte to use for the next command. Persisted so a restart
    -- does not replay sequence numbers the bulb has already seen.
    next_sequence SMALLINT    NOT NULL DEFAULT 0 CHECK (next_sequence BETWEEN 0 AND 255),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER lamp_states_set_updated_at
    BEFORE UPDATE ON lamp_states
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();


-- Append-only record of what we sent. Useful because the bulbs cannot be asked:
-- when the lights are wrong, this is the only account of what was transmitted.
CREATE TABLE lamp_commands (
    id         BIGSERIAL   PRIMARY KEY,
    lamp_id    UUID        NOT NULL REFERENCES lamps (id) ON DELETE CASCADE,
    source     TEXT        NOT NULL CHECK (source IN ('api', 'mqtt', 'cli', 'schedule', 'sniffer')),
    command    TEXT        NOT NULL,
    argument   INTEGER     NULL,
    succeeded  BOOLEAN     NOT NULL,
    error      TEXT        NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT error_iff_failed CHECK (succeeded = (error IS NULL))
);

CREATE INDEX lamp_commands_lamp_id_created_at_idx
    ON lamp_commands (lamp_id, created_at DESC);
