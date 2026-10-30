use fatalis_adaptive::ScriptPolicy;
use serde_json::{json, Value};
use std::path::Path;

const LEARNERS: &[(&str, f64)] = &[
    ("policies/default.rhai", 1.0),
    ("policies/fast.rhai", 1.0),
    ("strategies/standard-malevolent/algorithm.rhai", 1.0),
    ("strategies/standard-benevolent/algorithm.rhai", -1.0),
    ("strategies/fast-malevolent/algorithm.rhai", 1.0),
    ("strategies/fast-benevolent/algorithm.rhai", -1.0),
];

fn load(path: &str, direction: f64) -> ScriptPolicy {
    ScriptPolicy::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join(path),
        json!({"direction": direction}),
    )
    .unwrap()
}

fn table() -> Value {
    json!({"id": "test-table", "branches": [
        {"id": "a", "base_weight": 50},
        {"id": "b", "base_weight": 50}
    ]})
}

fn event(branch: &str) -> Value {
    json!({"table_id": "test-table", "branch_id": branch,
        "player_loss_fraction": 1.0, "monster_loss_fraction": 0.0})
}

fn invalidated(branch: &str) -> Value {
    let mut value = event(branch);
    value["invalidate"] = json!(true);
    value["reason"] = json!("multiplayer batch discarded");
    value
}

#[test]
fn standard_and_fast_ignore_invalidated_events_in_live_and_replayed_updates() {
    for &(path, direction) in LEARNERS {
        let policy = load(path, direction);
        let empty = policy.update(&json!({}), &json!([])).unwrap();
        let discarded = json!([invalidated("a"), invalidated("b")]);
        let state = policy.update(&json!({}), &discarded).unwrap();
        assert_eq!(state, empty, "{path}");
        assert_eq!(policy.weights(&state, &table()).unwrap(), vec![50.0, 50.0]);

        let learned = policy.update(&json!({}), &json!([event("a")])).unwrap();
        assert_eq!(
            policy.update(&learned, &discarded).unwrap(),
            learned,
            "{path}"
        );
        let replay = policy
            .update(
                &json!({}),
                &json!([invalidated("b"), event("a"), invalidated("a")]),
            )
            .unwrap();
        assert_eq!(replay, learned, "{path}");
        assert_eq!(
            policy.weights(&replay, &table()).unwrap(),
            policy.weights(&learned, &table()).unwrap(),
            "{path}"
        );
        // The guard must run before accessing any learning fields.
        assert_eq!(
            policy
                .update(&learned, &json!([{"invalidate": true}]))
                .unwrap(),
            learned,
            "{path}"
        );
    }
}

#[test]
fn standard_and_fast_accept_legacy_missing_flag_and_explicit_false() {
    for &(path, direction) in LEARNERS {
        let policy = load(path, direction);
        let legacy = policy.update(&json!({}), &json!([event("a")])).unwrap();
        assert_eq!(legacy["effects"]["a"]["samples"], json!(1.0), "{path}");
        assert_eq!(legacy["effects"]["a"]["player_loss"], json!(1.0), "{path}");
        assert_eq!(legacy["effects"]["a"]["monster_loss"], json!(0.0), "{path}");
        let weights = policy.weights(&legacy, &table()).unwrap();
        assert!((weights[0] - 50.0) * direction > 0.0, "{path}");

        let mut accepted = event("a");
        accepted["invalidate"] = json!(false);
        assert_eq!(
            policy.update(&json!({}), &json!([accepted])).unwrap(),
            legacy,
            "{path}"
        );
    }
}

#[test]
fn super_ignores_invalidated_winners_and_accepts_legacy_and_false_flags() {
    let policy = load("policies/super_malevolent.rhai", 1.0);
    let empty = policy.update(&json!({}), &json!([])).unwrap();
    let discarded = json!([invalidated("a"), invalidated("b")]);
    assert_eq!(policy.update(&json!({}), &discarded).unwrap(), empty);
    assert_eq!(policy.weights(&empty, &table()).unwrap(), vec![50.0, 50.0]);

    let learned = policy.update(&json!({}), &json!([event("a")])).unwrap();
    assert_eq!(learned, json!({"winners": {"test-table": "a"}}));
    assert_eq!(
        policy.weights(&learned, &table()).unwrap(),
        vec![100.0, 0.0]
    );
    assert_eq!(policy.update(&learned, &discarded).unwrap(), learned);
    assert_eq!(
        policy
            .update(
                &json!({}),
                &json!([invalidated("b"), event("a"), invalidated("b")])
            )
            .unwrap(),
        learned
    );
    assert_eq!(
        policy
            .update(&learned, &json!([{"invalidate": true}]))
            .unwrap(),
        learned
    );

    let mut accepted = event("b");
    accepted["invalidate"] = json!(false);
    let updated = policy.update(&learned, &json!([accepted])).unwrap();
    assert_eq!(updated, json!({"winners": {"test-table": "b"}}));
    assert_eq!(
        policy.weights(&updated, &table()).unwrap(),
        vec![0.0, 100.0]
    );
}

#[test]
fn no_learning_remains_stateless_for_all_event_flags() {
    let policy = ScriptPolicy::no_learning().unwrap();
    let state = policy
        .update(
            &json!({"winners": {"test-table": "a"}}),
            &json!([event("a"), invalidated("b"), {"invalidate": true}]),
        )
        .unwrap();
    assert_eq!(state, json!({}));
    assert_eq!(policy.weights(&state, &table()).unwrap(), vec![50.0, 50.0]);
}
