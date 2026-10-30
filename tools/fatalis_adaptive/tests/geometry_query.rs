#![cfg(feature = "game-fixtures")]
//! Real-Fatalis offline regression tests: geometry chooses a probability table,
//! and explicit edits affect only the requested table's four-byte weight fields.
//! These tests never inspect a running game or write a fixture/game file.
use fatalis_adaptive::{
    geometry::{GeometryModel, Query, QueryResult, TableMatch},
    thk::{sha256, Project},
    weight_edit::{apply, EditRequest},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

fn project() -> Project {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/fatalis_thk");
    Project::from_paths(root.join("em013_00.thk"), root.join("em013_55.thk"))
        .expect("real original Fatalis THK fixtures must exist and match the pinned hashes")
}
fn query(project: &Project, entry_node: usize, distance: f64, angle_cw: f64) -> QueryResult {
    GeometryModel::new(project)
        .query(&Query {
            distance,
            angle_cw,
            entry_node,
        })
        .expect("valid geometry query must succeed")
}
fn table(result: &QueryResult, node: usize, segment: usize) -> &TableMatch {
    result
        .tables
        .iter()
        .find(|t| t.node == node && t.start_segment == segment)
        .unwrap_or_else(|| {
            panic!("missing node {node} segment {segment}; query returned {result:?}")
        })
}
fn request(current: &[u8], id: &str, weights: &[i32]) -> EditRequest {
    EditRequest {
        source_sha256: sha256(current),
        tables: BTreeMap::from([(id.into(), weights.into())]),
    }
}
fn assert_only_weight_bytes_changed(before: &[u8], after: &[u8], weights: &[usize]) {
    assert_eq!(
        before.len(),
        after.len(),
        "editing must not extend/truncate THK"
    );
    let allowed: BTreeSet<usize> = weights
        .iter()
        .flat_map(|&offset| offset..offset + 4)
        .collect();
    let changed: Vec<usize> = before
        .iter()
        .zip(after)
        .enumerate()
        .filter_map(|(offset, (a, b))| (a != b).then_some(offset))
        .collect();
    assert!(!changed.is_empty(), "this edit must actually change bytes");
    assert!(
        changed.iter().all(|offset| allowed.contains(offset)),
        "changed byte outside requested table's weight fields: {changed:?}"
    );
}

#[test]
fn node105_angle_routes_choose_five_independent_real_tables() {
    let project = project();
    let cases: &[(f64, usize, &[i32])] = &[
        (10.0, 1, &[35, 30, 0, 0, 20, 10, 5, 0]),
        (60.0, 11, &[25, 20, 0, 20, 20, 10, 5, 0]),
        (300.0, 21, &[25, 20, 0, 20, 20, 10, 5, 0]),
        (180.0, 31, &[10, 10, 0, 10, 20, 30, 20, 0]),
        (120.0, 41, &[10, 10, 0, 10, 20, 10, 25, 15, 0]),
    ];
    for &(angle, segment, expected) in cases {
        let result = query(&project, 105, 700.0, angle);
        let node105: Vec<_> = result.tables.iter().filter(|t| t.node == 105).collect();
        assert_eq!(
            node105.len(),
            1,
            "non-boundary angle {angle} must not mix node105 tables: {result:?}"
        );
        let matched = table(&result, 105, segment);
        assert_eq!(matched.table_id, format!("main:n105:s{segment}"));
        assert_eq!(
            matched.options.iter().map(|o| o.weight).collect::<Vec<_>>(),
            expected
        );
        assert!(!matched.routes.is_empty());
        assert!(matched.routes.iter().any(|r| r.node_path == [105]));
        for option in &matched.options {
            assert_eq!(
                option.weight_offset,
                project.main.nodes[105].segments[option.segment].offset + 12
            );
            assert_eq!(
                i32::from_le_bytes(
                    project.main.bytes[option.weight_offset..option.weight_offset + 4]
                        .try_into()
                        .unwrap()
                ),
                option.weight
            );
        }
    }
    let result = query(&project, 105, 700.0, 10.0);
    assert_eq!(
        table(&result, 105, 1)
            .options
            .iter()
            .map(|o| o.global_node)
            .collect::<Vec<_>>(),
        [
            Some(102),
            Some(103),
            Some(101),
            Some(110),
            Some(116),
            Some(113),
            Some(124),
            None
        ]
    );
}

#[test]
fn node11_upstream_distance_is_required_before_node105_angles() {
    let project = project();
    let near = query(&project, 11, 700.0, 10.0);
    let matched = table(&near, 105, 1);
    assert!(
        matched.routes.iter().any(|r| r.node_path == [11, 105]),
        "must retain the actual upstream route, not merely find matching global actions"
    );
    assert!(
        matched.routes.iter().any(|r| r
            .conditions
            .iter()
            .any(|c| c.geometry && (c.function == 8 || c.function == 9))),
        "upstream distance condition must survive route extraction"
    );
    assert_eq!(near.tables.iter().filter(|t| t.node == 105).count(), 1);
    let far = query(&project, 11, 1200.0, 10.0);
    assert!(
        far.tables.iter().all(|t| t.node != 105),
        "correct angle alone must not admit distance-ineligible node105: {far:?}"
    );
}

#[test]
fn exact_angle_endpoint_keeps_adjacent_routes_ambiguous() {
    let project = project();
    let result = query(&project, 105, 700.0, 30.0);
    let starts: BTreeSet<_> = result
        .tables
        .iter()
        .filter(|t| t.node == 105)
        .map(|t| t.start_segment)
        .collect();
    assert_eq!(starts, BTreeSet::from([1, 11, 41]),
        "unknown endpoint inclusion must retain both adjacent tables and else: if both intervals exclude 30 degrees, neither condition holds");
}

#[test]
fn edit_one_table_preserves_every_other_byte_and_can_chain_from_current() {
    let project = project();
    let first_query = query(&project, 105, 700.0, 10.0);
    let first = table(&first_query, 105, 1);
    let first_weights = [45, 20, 0, 0, 20, 10, 5, 0];
    let (edited, report) = apply(
        &project,
        &project.main.bytes,
        &request(&project.main.bytes, &first.table_id, &first_weights),
    )
    .unwrap();
    let offsets: Vec<_> = first.options.iter().map(|o| o.weight_offset).collect();
    assert_only_weight_bytes_changed(&project.main.bytes, &edited, &offsets);
    assert_eq!(report.source_sha256, sha256(&project.main.bytes));
    assert_eq!(report.output_sha256, sha256(&edited));
    assert!(report.non_weight_bytes_identical);
    assert_eq!(report.changed_tables.len(), 1);
    assert_eq!(report.changed_tables[0].table_id, first.table_id);
    assert_eq!(report.changed_tables[0].after, first_weights);
    assert_eq!(report.unchanged_length, project.main.bytes.len());
    for (option, expected) in first.options.iter().zip(first_weights) {
        assert_eq!(
            i32::from_le_bytes(
                edited[option.weight_offset..option.weight_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            expected
        );
    }
    let second_query = query(&project, 105, 700.0, 60.0);
    let second = table(&second_query, 105, 11);
    let second_weights = [35, 10, 0, 20, 20, 10, 5, 0];
    let (twice, second_report) = apply(
        &project,
        &edited,
        &request(&edited, &second.table_id, &second_weights),
    )
    .unwrap();
    assert_only_weight_bytes_changed(
        &edited,
        &twice,
        &second
            .options
            .iter()
            .map(|o| o.weight_offset)
            .collect::<Vec<_>>(),
    );
    assert_eq!(second_report.changed_tables.len(), 1);
    for option in &first.options {
        assert_eq!(
            &twice[option.weight_offset..option.weight_offset + 4],
            &edited[option.weight_offset..option.weight_offset + 4],
            "second edit must preserve prior first-table edit"
        );
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/fatalis_thk");
    assert_eq!(
        fs::read(root.join("em013_00.thk")).unwrap(),
        project.main.bytes,
        "tests must not modify original THK fixture"
    );
}

#[test]
fn edits_reject_stale_source_invalid_sum_and_activation_of_zero_arms() {
    let project = project();
    let result = query(&project, 105, 700.0, 10.0);
    let matched = table(&result, 105, 1);
    let valid = [45, 20, 0, 0, 20, 10, 5, 0];
    let initial_request = request(&project.main.bytes, &matched.table_id, &valid);
    let (edited, _) = apply(&project, &project.main.bytes, &initial_request).unwrap();
    assert!(
        apply(&project, &edited, &initial_request).is_err(),
        "old source hash must not edit changed input"
    );
    let bad_sum = [46, 20, 0, 0, 20, 10, 5, 0];
    assert!(apply(
        &project,
        &project.main.bytes,
        &request(&project.main.bytes, &matched.table_id, &bad_sum)
    )
    .is_err());
    // Keep sum=100, so refusal specifically covers awakening an original zero arm.
    let activated_zero = [34, 30, 1, 0, 20, 10, 5, 0];
    assert!(apply(
        &project,
        &project.main.bytes,
        &request(&project.main.bytes, &matched.table_id, &activated_zero)
    )
    .is_err());
}
