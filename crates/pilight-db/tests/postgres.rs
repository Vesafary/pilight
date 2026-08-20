//! Integration tests against a real Postgres.
//!
//! See `common/mod.rs` for how to point these at a database.

mod common;

use pilight_db::domain::{
    BulbMode, CommandSource, LampStateUpdate, LampUpdate, NewLamp, NewLampCommand,
};
use pilight_db::repository::{
    CommandLogRepository, LampRepository, LampStateRepository, LampTypeRepository,
};
use pilight_db::{DbError, RemoteType, run_migrations};
use uuid::Uuid;

/// The migration harness uses `block_in_place`, so every test needs the
/// multi-threaded runtime. This alias keeps that decision in one place.
macro_rules! db_test {
    ($(#[$meta:meta])* async fn $name:ident() $body:block) => {
        $(#[$meta])*
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() $body
    };
}

fn a_lamp() -> NewLamp {
    NewLamp {
        name: "Couch".into(),
        room: Some("Living room".into()),
        remote_type: RemoteType::RgbCct,
        device_id: 0xBEEF,
        group: 1,
    }
}

// ---------------------------------------------------------------- migrations

db_test! {
    async fn migrations_are_idempotent() {
        let db = require_db!();

        // The harness already migrated; a second run must find nothing to do.
        let applied = run_migrations(&db.pool).await.unwrap();
        assert!(applied.is_empty(), "re-running migrations applied {applied:?}");
    }
}

// --------------------------------------------------------------- lamp types

db_test! {
    async fn lamp_types_mirror_the_driver() {
        let db = require_db!();

        let stored = db.repos.types.find_all().await.unwrap();
        assert_eq!(stored, RemoteType::ALL.to_vec());
    }
}

db_test! {
    async fn syncing_lamp_types_is_idempotent() {
        let db = require_db!();

        for _ in 0..3 {
            db.repos.types.sync_from_driver().await.unwrap();
        }

        assert_eq!(db.repos.types.find_all().await.unwrap().len(), RemoteType::ALL.len());
    }
}

db_test! {
    async fn only_rgb_cct_is_reported_as_drivable() {
        let db = require_db!();

        let supported = db.repos.types.find_supported().await.unwrap();
        assert_eq!(supported, vec![RemoteType::RgbCct]);
    }
}

// -------------------------------------------------------------------- lamps

db_test! {
    async fn creating_a_lamp_also_creates_its_state() {
        let db = require_db!();

        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();
        assert_eq!(lamp.name, "Couch");
        assert_eq!(lamp.device_id, 0xBEEF);
        assert_eq!(lamp.remote_type, RemoteType::RgbCct);

        // A lamp with no state row would be one we could never record anything about.
        let state = db.repos.states.find_by_lamp(lamp.id).await.unwrap()
            .expect("the state row is created in the same transaction");
        assert!(!state.power);
        assert_eq!(state.bulb_mode, BulbMode::White);
        assert_eq!(state.brightness, None, "we have not told it a brightness yet");
        assert_eq!(state.next_sequence, 0);
    }
}

db_test! {
    async fn two_lamps_cannot_share_an_address() {
        let db = require_db!();
        db.repos.lamps.create(a_lamp()).await.unwrap();

        let clash = NewLamp { name: "Another name".into(), ..a_lamp() };
        let error = db.repos.lamps.create(clash).await.unwrap_err();

        assert!(
            matches!(error, DbError::DuplicateAddress { device_id: 0xBEEF, group: 1, .. }),
            "expected a duplicate-address error, got {error}"
        );
    }
}

db_test! {
    async fn the_same_device_id_in_a_different_group_is_a_different_lamp() {
        let db = require_db!();

        db.repos.lamps.create(a_lamp()).await.unwrap();
        let other = db.repos.lamps.create(NewLamp {
            name: "Overhead".into(),
            group: 2,
            ..a_lamp()
        }).await.unwrap();

        assert_eq!(other.group, 2);
        assert_eq!(db.repos.lamps.find_all().await.unwrap().len(), 2);
    }
}

db_test! {
    async fn a_rejected_lamp_leaves_nothing_behind() {
        let db = require_db!();

        // Group 5 does not exist on a four-group family.
        let error = db.repos.lamps.create(NewLamp { group: 5, ..a_lamp() }).await.unwrap_err();
        assert!(matches!(error, DbError::Protocol(_)), "got {error}");

        assert!(db.repos.lamps.find_all().await.unwrap().is_empty());
    }
}

db_test! {
    async fn an_undrivable_family_is_refused() {
        let db = require_db!();

        let error = db.repos.lamps.create(NewLamp {
            remote_type: RemoteType::Rgbw,
            ..a_lamp()
        }).await.unwrap_err();

        assert!(matches!(error, DbError::Invalid(_)), "got {error}");
    }
}

db_test! {
    async fn lamps_are_found_by_the_address_a_bulb_listens_on() {
        let db = require_db!();
        let created = db.repos.lamps.create(a_lamp()).await.unwrap();

        let found = db.repos.lamps
            .find_by_address(RemoteType::RgbCct, 0xBEEF, 1)
            .await.unwrap();
        assert_eq!(found.map(|lamp| lamp.id), Some(created.id));

        // Same id, wrong group: a different bulb.
        let missing = db.repos.lamps
            .find_by_address(RemoteType::RgbCct, 0xBEEF, 2)
            .await.unwrap();
        assert!(missing.is_none());
    }
}

db_test! {
    async fn lamps_are_listed_by_room_then_name() {
        let db = require_db!();

        for (name, room, group) in [
            ("Overhead", Some("Kitchen"), 3),
            ("Couch",    Some("Living room"), 1),
            ("Corner",   Some("Kitchen"), 2),
            ("Orphan",   None, 4),
        ] {
            db.repos.lamps.create(NewLamp {
                name: name.into(),
                room: room.map(Into::into),
                group,
                ..a_lamp()
            }).await.unwrap();
        }

        let names: Vec<String> = db.repos.lamps.find_all().await.unwrap()
            .into_iter().map(|lamp| lamp.name).collect();
        assert_eq!(names, ["Corner", "Overhead", "Couch", "Orphan"],
            "rooms sort first, roomless lamps last");

        let kitchen: Vec<String> = db.repos.lamps.find_by_room("Kitchen").await.unwrap()
            .into_iter().map(|lamp| lamp.name).collect();
        assert_eq!(kitchen, ["Corner", "Overhead"]);
    }
}

db_test! {
    async fn a_lamp_can_be_renamed_and_have_its_room_cleared() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        let renamed = db.repos.lamps.update(lamp.id, LampUpdate {
            name: Some("Reading lamp".into()),
            room: None,
        }).await.unwrap();
        assert_eq!(renamed.name, "Reading lamp");
        assert_eq!(renamed.room.as_deref(), Some("Living room"), "None leaves it alone");

        let cleared = db.repos.lamps.update(lamp.id, LampUpdate {
            name: None,
            room: Some(None),
        }).await.unwrap();
        assert_eq!(cleared.room, None, "Some(None) clears it");
        assert_eq!(cleared.name, "Reading lamp");

        assert!(cleared.updated_at >= lamp.updated_at, "the trigger bumps updated_at");
    }
}

db_test! {
    async fn an_empty_update_is_a_no_op_rather_than_broken_sql() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        let unchanged = db.repos.lamps.update(lamp.id, LampUpdate::default()).await.unwrap();
        assert_eq!(unchanged.name, lamp.name);
    }
}

db_test! {
    async fn updating_a_missing_lamp_says_so() {
        let db = require_db!();

        let error = db.repos.lamps.update(Uuid::new_v4(), LampUpdate {
            name: Some("Ghost".into()),
            room: None,
        }).await.unwrap_err();

        assert!(matches!(error, DbError::LampNotFound(_)), "got {error}");
    }
}

db_test! {
    async fn deleting_a_lamp_takes_its_state_and_history_with_it() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        db.repos.commands.record(
            NewLampCommand::succeeded(lamp.id, CommandSource::Cli, "on", None)
        ).await.unwrap();

        assert!(db.repos.lamps.delete(lamp.id).await.unwrap());

        assert!(db.repos.lamps.find_by_id(lamp.id).await.unwrap().is_none());
        assert!(db.repos.states.find_by_lamp(lamp.id).await.unwrap().is_none());
        assert!(db.repos.commands.recent_for_lamp(lamp.id, None).await.unwrap().is_empty());

        assert!(!db.repos.lamps.delete(lamp.id).await.unwrap(), "deleting twice is not an error");
    }
}

// -------------------------------------------------------------------- state

db_test! {
    async fn state_updates_round_trip() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        let state = db.repos.states.update(lamp.id, LampStateUpdate {
            power: Some(true),
            brightness: Some(60),
            ..Default::default()
        }).await.unwrap();

        assert!(state.power);
        assert_eq!(state.brightness, Some(60));
        assert_eq!(state.bulb_mode, BulbMode::White, "brightness says nothing about mode");
    }
}

db_test! {
    async fn setting_a_hue_moves_the_bulb_into_colour_mode() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        let state = db.repos.states.update(lamp.id, LampStateUpdate::hue(200)).await.unwrap();
        assert_eq!(state.hue, Some(200));
        assert_eq!(state.bulb_mode, BulbMode::Color);

        // ...and a Kelvin command drags it back to white, as the protocol does.
        let state = db.repos.states.update(lamp.id, LampStateUpdate::kelvin(80)).await.unwrap();
        assert_eq!(state.kelvin, Some(80));
        assert_eq!(state.bulb_mode, BulbMode::White);
        assert_eq!(state.hue, Some(200), "the old hue is remembered, not cleared");
    }
}

db_test! {
    async fn state_for_a_missing_lamp_says_so() {
        let db = require_db!();

        let error = db.repos.states
            .update(Uuid::new_v4(), LampStateUpdate::power(true))
            .await.unwrap_err();
        assert!(matches!(error, DbError::LampNotFound(_)), "got {error}");
    }
}

db_test! {
    async fn sequence_numbers_are_handed_out_in_order_and_wrap() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        for expected in 0u8..=10 {
            assert_eq!(db.repos.states.take_sequence(lamp.id).await.unwrap(), expected);
        }

        // Wind it to the top and check the wrap, which used to panic in the driver.
        db.repos.states.update(lamp.id, LampStateUpdate::default()).await.unwrap();
        for _ in 11u16..255 {
            db.repos.states.take_sequence(lamp.id).await.unwrap();
        }
        assert_eq!(db.repos.states.take_sequence(lamp.id).await.unwrap(), 255);
        assert_eq!(db.repos.states.take_sequence(lamp.id).await.unwrap(), 0);
    }
}

db_test! {
    async fn concurrent_senders_never_get_the_same_sequence_number() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        // The API and an MQTT handler can both be sending at once; handing the
        // same sequence byte to one bulb would make it ignore the second command.
        let handles: Vec<_> = (0..32)
            .map(|_| {
                let states = db.repos.states.clone();
                let id = lamp.id;
                tokio::spawn(async move { states.take_sequence(id).await.unwrap() })
            })
            .collect();

        let mut taken = Vec::new();
        for handle in handles {
            taken.push(handle.await.unwrap());
        }
        taken.sort_unstable();

        assert_eq!(taken, (0u8..32).collect::<Vec<_>>(), "every number handed out exactly once");
    }
}

// ------------------------------------------------------------- command log

db_test! {
    async fn commands_are_recorded_newest_first() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        for (command, argument) in [("on", None), ("brightness", Some(60)), ("hue", Some(200))] {
            db.repos.commands.record(
                NewLampCommand::succeeded(lamp.id, CommandSource::Api, command, argument)
            ).await.unwrap();
        }

        let recent = db.repos.commands.recent_for_lamp(lamp.id, None).await.unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].command, "hue");
        assert_eq!(recent[0].argument, Some(200));
        assert!(recent[0].succeeded);
    }
}

db_test! {
    async fn a_failed_command_keeps_its_reason() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        let recorded = db.repos.commands.record(NewLampCommand::failed(
            lamp.id, CommandSource::Mqtt, "brightness", Some(60), "radio busy",
        )).await.unwrap();

        assert!(!recorded.succeeded);
        assert_eq!(recorded.error.as_deref(), Some("radio busy"));
        assert_eq!(recorded.source, CommandSource::Mqtt);
    }
}

db_test! {
    async fn history_can_be_limited_and_pruned() {
        let db = require_db!();
        let lamp = db.repos.lamps.create(a_lamp()).await.unwrap();

        for _ in 0..5 {
            db.repos.commands.record(
                NewLampCommand::succeeded(lamp.id, CommandSource::Schedule, "on", None)
            ).await.unwrap();
        }

        assert_eq!(db.repos.commands.recent_for_lamp(lamp.id, Some(2)).await.unwrap().len(), 2);

        let pruned = db.repos.commands.prune(chrono::Utc::now()).await.unwrap();
        assert_eq!(pruned, 5);
        assert!(db.repos.commands.recent_for_lamp(lamp.id, None).await.unwrap().is_empty());
    }
}
