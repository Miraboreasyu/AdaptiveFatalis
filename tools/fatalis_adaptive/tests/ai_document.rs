#![cfg(feature = "game-fixtures")]
//! Direct API tests: no subprocess, JSON edits, executable or game files.
use fatalis_adaptive::{geometry::Query, thk::Thk, AiDocument};
const MAIN: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_00.thk");
const GLOBAL: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_55.thk");
fn weights(d: &AiDocument, id: &str) -> Vec<i32> {
    d.table(id)
        .unwrap()
        .branches
        .iter()
        .map(|r| r.weight)
        .collect()
}
#[test]
fn unchanged_document_roundtrips_exactly() {
    let d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    assert_eq!(d.to_thk().unwrap(), MAIN);
    assert_eq!(d.global_to_thk().unwrap(), GLOBAL);
}
#[test]
fn edit_model_query_compile_reparse() {
    let mut d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let id = "main:n105:s1";
    let next = [45, 20, 0, 0, 20, 10, 5, 0];
    d.set_weights(id, &next).unwrap();
    assert_eq!(weights(&d, id), next);
    let q = d
        .query(&Query {
            distance: 700.0,
            angle_cw: 10.0,
            entry_node: 105,
        })
        .unwrap();
    assert_eq!(q.tables.len(), 1);
    assert_eq!(
        q.tables[0]
            .options
            .iter()
            .map(|o| o.weight)
            .collect::<Vec<_>>(),
        next
    );
    let bytes = d.to_thk().unwrap();
    let differences: Vec<_> = MAIN
        .iter()
        .zip(&bytes)
        .enumerate()
        .filter_map(|(i, (a, b))| (a != b).then_some(i))
        .collect();
    assert_eq!(differences, [426268, 426396]);
    let parsed = Thk::parse(bytes.clone()).unwrap();
    assert_eq!(parsed.nodes[105].segments[1].p1, 45);
    let mut reopened = AiDocument::parse_current(MAIN, GLOBAL, &bytes).unwrap();
    assert_eq!(weights(&reopened, id), next);
    let other = "main:n105:s11";
    let other_weights = [35, 10, 0, 20, 20, 10, 5, 0];
    reopened.set_weights(other, &other_weights).unwrap();
    let again = AiDocument::parse_current(MAIN, GLOBAL, &reopened.to_thk().unwrap()).unwrap();
    assert_eq!(weights(&again, id), next);
    assert_eq!(weights(&again, other), other_weights);
}
#[test]
fn invalid_edit_atomic_and_zeroed_arm_can_recover() {
    let mut d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let id = "main:n105:s1";
    assert!(d.set_weights(id, &[1, 2]).is_err());
    assert!(d.set_weights(id, &[35, 29, 1, 0, 20, 10, 5, 0]).is_err());
    assert_eq!(d.to_thk().unwrap(), MAIN);
    d.set_weights(id, &[0, 65, 0, 0, 20, 10, 5, 0]).unwrap();
    d.set_weights(id, &[35, 30, 0, 0, 20, 10, 5, 0]).unwrap();
    assert_eq!(d.to_thk().unwrap(), MAIN);
}
#[test]
fn document_copies_edit_independently() {
    let d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let mut other = d.clone();
    other
        .set_weights("main:n105:s1", &[45, 20, 0, 0, 20, 10, 5, 0])
        .unwrap();
    assert_eq!(d.to_thk().unwrap(), MAIN);
    assert_ne!(other.to_thk().unwrap(), MAIN);
}
