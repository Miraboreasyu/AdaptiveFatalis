#![cfg(feature = "game-fixtures")]
use fatalis_adaptive::{policy_store::integer_weights, AiDocument, ScriptPolicy};
use serde_json::json;
use std::path::Path;
#[test]
fn builtin_is_baseline_and_fast_reacts_more_in_both_directions() {
    let doc = AiDocument::parse(
        include_bytes!("../../../research/fatalis_thk/em013_00.thk"),
        include_bytes!("../../../research/fatalis_thk/em013_55.thk"),
    )
    .unwrap();
    let t = doc.table("main:n105:s1").unwrap();
    let table = json!({"id":t.id,"branches":t.branches.iter().map(|b|json!({"id":b.id,"base_weight":b.weight})).collect::<Vec<_>>()});
    let base = t.branches.iter().map(|b| b.weight).collect::<Vec<_>>();
    let events = json!([{"branch_id":t.branches[0].id,"player_loss_fraction":0.6,"monster_loss_fraction":0.0}]);
    let none = ScriptPolicy::no_learning().unwrap();
    let state = none.update(&json!({"winners":{"x":"y"}}), &events).unwrap();
    assert_eq!(state, json!({}));
    assert_eq!(
        integer_weights(&base, &none.weights(&state, &table).unwrap()).unwrap(),
        base
    );
    for direction in [1.0, -1.0] {
        let load = |name: &str| {
            ScriptPolicy::load(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("policies")
                    .join(name),
                json!({"direction":direction}),
            )
            .unwrap()
        };
        let standard = load("default.rhai");
        let fast = load("fast.rhai");
        let s = standard.update(&json!({}), &events).unwrap();
        let f = fast.update(&json!({}), &events).unwrap();
        let a = integer_weights(&base, &standard.weights(&s, &table).unwrap()).unwrap();
        let b = integer_weights(&base, &fast.weights(&f, &table).unwrap()).unwrap();
        if direction > 0.0 {
            assert!(b[0] > a[0] && a[0] > base[0]);
        } else {
            assert!(b[0] < a[0] && a[0] < base[0]);
        }
        assert_eq!(b.iter().sum::<i32>(), 100);
        for (i, w) in base.iter().enumerate() {
            if *w == 0 {
                assert_eq!(b[i], 0);
            }
        }
    }
}
