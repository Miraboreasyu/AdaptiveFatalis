#![cfg(feature = "game-fixtures")]
//! A synthetic action result must survive persistence and change only allowed THK weights.
use fatalis_adaptive::{config::Config, learning::Learning, runtime, thk::Project};
use std::{fs, path::Path};
#[test]
fn observed_exit_changes_next_hunt_without_rewriting_control_flow() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/fatalis_thk");
    if !root.join("em013_00.thk").exists() {
        return;
    }
    let p = Project::from_paths(root.join("em013_00.thk"), root.join("em013_55.thk")).unwrap();
    let block = p
        .blocks
        .iter()
        .find(|b| b.node == 75 && b.branches.iter().filter(|x| x.weight > 0).count() > 1)
        .unwrap();
    let chosen = block.branches.iter().find(|x| x.weight > 0).unwrap();
    let mut l = Learning::default();
    for _ in 0..100 {
        l.observe(&chosen.id, 0.8, 0.0, 1.0);
    }
    let persisted = serde_json::to_vec(&l).unwrap();
    let l: Learning = serde_json::from_slice(&persisted).unwrap();
    let weights = runtime::targets(&p, &l, &Config::default()).unwrap();
    let index = block
        .branches
        .iter()
        .position(|x| x.id == chosen.id)
        .unwrap();
    assert!(weights[&block.id][index] > chosen.weight);
    let changed = p
        .patch_current_main_weights(&p.main.bytes, &weights)
        .unwrap();
    assert_ne!(changed, p.main.bytes);
    assert_eq!(changed.len(), p.main.bytes.len());
    let again = p.patch_current_main_weights(&changed, &weights).unwrap();
    assert_eq!(again, changed);
    let mut friendly = Config::default();
    friendly.policy.orientation = -1.0;
    let friendly_weights = runtime::targets(&p, &l, &friendly).unwrap();
    assert!(friendly_weights[&block.id][index] < chosen.weight);
    // Ensure the fixture was read-only throughout.
    assert_eq!(fs::read(root.join("em013_00.thk")).unwrap(), p.main.bytes);
}
