// @generated automatically by Diesel CLI.

diesel::table! {
    lamp_commands (id) {
        id -> Int8,
        lamp_id -> Uuid,
        source -> Text,
        command -> Text,
        argument -> Nullable<Int4>,
        succeeded -> Bool,
        error -> Nullable<Text>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    lamp_states (lamp_id) {
        lamp_id -> Uuid,
        power -> Bool,
        bulb_mode -> Text,
        brightness -> Nullable<Int2>,
        hue -> Nullable<Int2>,
        saturation -> Nullable<Int2>,
        kelvin -> Nullable<Int2>,
        scene -> Nullable<Int2>,
        next_sequence -> Int2,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    lamp_types (id) {
        id -> Int2,
        slug -> Text,
        display_name -> Text,
        protocol_generation -> Int2,
        protocol_id -> Nullable<Int2>,
        num_groups -> Int2,
        driver_supported -> Bool,
    }
}

diesel::table! {
    lamps (id) {
        id -> Uuid,
        name -> Text,
        room -> Nullable<Text>,
        lamp_type_id -> Int2,
        device_id -> Int4,
        group_id -> Int2,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::joinable!(lamp_commands -> lamps (lamp_id));
diesel::joinable!(lamp_states -> lamps (lamp_id));
diesel::joinable!(lamps -> lamp_types (lamp_type_id));

diesel::allow_tables_to_appear_in_same_query!(lamp_commands, lamp_states, lamp_types, lamps,);
