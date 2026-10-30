#![cfg(feature = "game-fixtures")]
use fatalis_adaptive::{policy_store::integer_weights, AiDocument, ScriptPolicy};
use serde_json::{json, Value};
use std::path::Path;
#[test]
fn threshold_is_per_window_and_last_qualifier_wins_only_its_table() {
    let document = AiDocument::parse(
        include_bytes!("../../../research/fatalis_thk/em013_00.thk"),
        include_bytes!("../../../research/fatalis_thk/em013_55.thk"),
    )
    .unwrap();
    let script = ScriptPolicy::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("policies/super_malevolent.rhai"),
        json!({}),
    )
    .unwrap();
    let table = document.table("main:n105:s1").unwrap();
    let event = |exit: usize, damage: f64| json!({"table_id":table.id,"branch_id":table.branches[exit].id,"player_loss_fraction":damage});
    let input = |table: &fatalis_adaptive::thk::RandomBlock| -> Value {
        json!({"id":table.id,"branches":table.branches.iter().map(|b|json!({"id":b.id,"base_weight":b.weight})).collect::<Vec<_>>()})
    };
    let base: Vec<i32> = table.branches.iter().map(|b| b.weight).collect();
    let state = script
        .update(
            &json!({}),
            &json!([event(0, 0.6), event(0, 0.6), event(1, 0.9)]),
        )
        .unwrap();
    assert_eq!(
        integer_weights(&base, &script.weights(&state, &input(table)).unwrap()).unwrap(),
        base
    );
    let state = script.update(&state, &json!([event(0, 0.900001)])).unwrap();
    let locked = integer_weights(&base, &script.weights(&state, &input(table)).unwrap()).unwrap();
    assert_eq!(locked[0], 100);
    assert_eq!(locked.iter().sum::<i32>(), 100);
    let state = script
        .update(&state, &json!([event(1, 1.2), event(0, 0.9)]))
        .unwrap();
    let locked = integer_weights(&base, &script.weights(&state, &input(table)).unwrap()).unwrap();
    assert_eq!(locked[1], 100);
    let other = document.table("main:n105:s11").unwrap();
    assert_eq!(
        integer_weights(
            &other.branches.iter().map(|b| b.weight).collect::<Vec<_>>(),
            &script.weights(&state, &input(other)).unwrap()
        )
        .unwrap(),
        other.branches.iter().map(|b| b.weight).collect::<Vec<_>>()
    );
    let restored: Value = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(
        script.weights(&state, &input(table)).unwrap(),
        script.weights(&restored, &input(table)).unwrap()
    );
}
