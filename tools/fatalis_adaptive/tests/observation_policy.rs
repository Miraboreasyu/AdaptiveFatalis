use fatalis_adaptive::ScriptPolicy;
use serde_json::json;
use std::path::Path;
#[test]
fn bundled_learners_ignore_raw_facts_but_keep_legacy_windows() {
    for name in [
        "policies/default.rhai",
        "policies/fast.rhai",
        "policies/super_malevolent.rhai",
        "strategies/standard-malevolent/algorithm.rhai",
        "strategies/standard-benevolent/algorithm.rhai",
        "strategies/fast-malevolent/algorithm.rhai",
        "strategies/fast-benevolent/algorithm.rhai",
    ] {
        let p = ScriptPolicy::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join(name), json!({}))
            .unwrap();
        let window = json!({"table_id":"t","branch_id":"a","player_loss_fraction":1.0,"monster_loss_fraction":0.0});
        let a = p.update(&json!({}), &json!([window.clone()])).unwrap();
        let b=p.update(&json!({}),&json!([{"kind":"player_hp_zero"},window,{"kind":"quest_deaths","quest_deaths_delta":1},{"kind":"part_threshold","delta":1}])).unwrap();
        assert_eq!(a, b, "{name}");
    }
}
