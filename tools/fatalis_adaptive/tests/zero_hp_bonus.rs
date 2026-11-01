use fatalis_adaptive::ScriptPolicy;
use serde_json::{json, Value};
use std::path::Path;
fn event(zero: bool) -> Value {
    json!({"kind":"option_window","table_id":"t","branch_id":"a","player_loss_fraction":0.2,"monster_loss_fraction":0.0,"player_hp_reached_zero":zero})
}
#[test]
fn zero_hp_rewards_once_and_reverses_for_benevolence() {
    for speed in ["standard", "fast"] {
        for (mode, dir) in [("malevolent", 1.0), ("benevolent", -1.0)] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("strategies")
                .join(format!("{speed}-{mode}"));
            let params: Value =
                serde_json::from_slice(&std::fs::read(root.join("params.json")).unwrap()).unwrap();
            let p = ScriptPolicy::load(&root.join("algorithm.rhai"), params.clone()).unwrap();
            let table = json!({"id":"t","branches":[{"id":"a","base_weight":50},{"id":"b","base_weight":50}]});
            let damage = p.update(&json!({}), &json!([event(false)])).unwrap();
            let zero=p.update(&json!({}),&json!([event(true),{"kind":"player_hp_zero"},{"kind":"quest_deaths","quest_deaths_delta":1},{"kind":"part_threshold","delta":4}])).unwrap();
            assert_eq!(zero["effects"]["a"]["zero_hp_count"], 1.0);
            assert!(
                (p.weights(&zero, &table).unwrap()[0] - p.weights(&damage, &table).unwrap()[0])
                    * dir
                    > 0.0
            );
            let mut off = params.clone();
            off["zero_hp_bonus"] = json!(0.0);
            let off = ScriptPolicy::load(&root.join("algorithm.rhai"), off).unwrap();
            assert_eq!(
                off.weights(&zero, &table).unwrap(),
                off.weights(&damage, &table).unwrap()
            );
            let mut invalid = event(true);
            invalid["invalidate"] = json!(true);
            assert_eq!(p.update(&damage, &json!([invalid])).unwrap(), damage);
            let mut legacy = event(false);
            legacy
                .as_object_mut()
                .unwrap()
                .remove("player_hp_reached_zero");
            assert_eq!(p.update(&json!({}), &json!([legacy])).unwrap(), damage);
            let old = json!({"effects":{"a":{"samples":1.0,"player_loss":0.2,"monster_loss":0.0}}});
            assert_eq!(
                p.weights(&old, &table).unwrap(),
                p.weights(&damage, &table).unwrap()
            );
            let info: Value =
                serde_json::from_slice(&std::fs::read(root.join("strategy.json")).unwrap())
                    .unwrap();
            assert_eq!(info["state_version"], 2);
        }
    }
}
