#![cfg(feature = "game-fixtures")]
use fatalis_adaptive::{
    instrument::Registers,
    learning::{Learning, Policy},
    register_learning::{apply_hunt, RegisterSnapshot, RegisterTracker},
    AiDocument,
};
const MAIN: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_00.thk");
const GLOBAL: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_55.thk");
fn registers() -> Registers {
    Registers {
        table_code: [14, 15, 16],
        option: [17, 18],
        counter: 19,
    }
}
#[test]
fn register_events_edit_model_then_recompile_with_same_ids() {
    let baseline = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let mut current = baseline.clone();
    let tables = vec!["main:n105:s1".into()];
    let slots = registers();
    let initial = current.to_instrumented_thk(&tables, slots).unwrap();
    let id = initial.manifest.markers[0].id;
    let mut tracker = RegisterTracker::new(initial.manifest.option_map());
    tracker.validate_options(&current).unwrap();
    for n in 1..=30 {
        tracker.push(Some(RegisterSnapshot {
            monster_token: 1,
            counter: n,
            option_id: id,
            player_hp: 100.0,
            player_max_hp: 100.0,
            monster_hp: 1000.0,
            monster_max_hp: 1000.0,
            timestamp: u64::from(n) * 10,
        }));
        tracker.push(Some(RegisterSnapshot {
            monster_token: 1,
            counter: n,
            option_id: id,
            player_hp: 20.0,
            player_max_hp: 100.0,
            monster_hp: 1000.0,
            monster_max_hp: 1000.0,
            timestamp: u64::from(n) * 10 + 1,
        }));
    }
    tracker.finish();
    let windows = tracker.drain_completed();
    assert_eq!(windows.len(), 30);
    let mut learning = Learning::default();
    apply_hunt(
        &baseline,
        &mut current,
        &mut learning,
        &Policy::default(),
        &windows,
    )
    .unwrap();
    assert!(current.table(&tables[0]).unwrap().branches[0].weight > 35);
    let next = current.to_instrumented_thk(&tables, slots).unwrap();
    assert_ne!(initial.bytes, next.bytes);
    assert_eq!(initial.manifest.option_map(), next.manifest.option_map());
}
#[test]
fn five_tables_learn_independently_and_keep_the_same_ids() {
    let baseline = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let mut current = baseline.clone();
    let tables: Vec<String> = [
        "main:n105:s1",
        "main:n105:s11",
        "main:n105:s21",
        "main:n105:s31",
        "main:n105:s41",
    ]
    .map(String::from)
    .to_vec();
    let slots = registers();
    let initial = current.to_instrumented_thk(&tables, slots).unwrap();
    assert_eq!(initial.manifest.markers.len(), 30);
    // Sample only the first exit of each table. If attribution leaked across
    // tables, every table's update would be diluted by the other tables' events.
    let sampled: Vec<_> = tables
        .iter()
        .map(|table| {
            initial
                .manifest
                .markers
                .iter()
                .find(|m| &m.table_id == table)
                .unwrap()
                .clone()
        })
        .collect();
    let mut tracker = RegisterTracker::new(initial.manifest.option_map());
    tracker.validate_options(&current).unwrap();
    let mut counter = 0u32;
    for marker in &sampled {
        for _ in 0..40 {
            counter += 1;
            let at = u64::from(counter) * 10;
            tracker.push(Some(RegisterSnapshot {
                monster_token: 1,
                counter,
                option_id: marker.id,
                player_hp: 100.0,
                player_max_hp: 100.0,
                monster_hp: 1000.0,
                monster_max_hp: 1000.0,
                timestamp: at,
            }));
            tracker.push(Some(RegisterSnapshot {
                monster_token: 1,
                counter,
                option_id: marker.id,
                player_hp: 20.0,
                player_max_hp: 100.0,
                monster_hp: 1000.0,
                monster_max_hp: 1000.0,
                timestamp: at + 1,
            }));
        }
    }
    tracker.finish();
    let windows = tracker.drain_completed();
    assert_eq!(windows.len(), sampled.len() * 40);
    let mut learning = Learning::default();
    apply_hunt(
        &baseline,
        &mut current,
        &mut learning,
        &Policy::default(),
        &windows,
    )
    .unwrap();
    for marker in &sampled {
        let before = baseline.table(&marker.table_id).unwrap().branches[marker.exit_index].weight;
        let after = current.table(&marker.table_id).unwrap().branches[marker.exit_index].weight;
        assert!(
            after > before,
            "{} exit {} did not increase: {before} -> {after}",
            marker.table_id,
            marker.exit_index
        );
    }
    let next = current.to_instrumented_thk(&tables, slots).unwrap();
    assert_eq!(initial.manifest.option_map(), next.manifest.option_map());
    assert_ne!(initial.bytes, next.bytes);
}
#[test]
fn used_slot_is_rejected() {
    let doc = AiDocument::parse(MAIN, GLOBAL).unwrap();
    assert!(doc
        .to_instrumented_thk(
            &["main:n105:s1".into()],
            Registers {
                table_code: [0, 15, 16],
                option: [17, 18],
                counter: 19
            }
        )
        .is_err());
}
