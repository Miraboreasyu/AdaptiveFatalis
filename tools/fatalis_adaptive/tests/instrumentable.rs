#![cfg(feature = "game-fixtures")]
use fatalis_adaptive::{instrument::instrumentable_tables, thk::Thk, AiDocument};
use std::collections::BTreeSet;
const MAIN: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_00.thk");
const GLOBAL: &[u8] = include_bytes!("../../../research/fatalis_thk/em013_55.thk");

#[test]
fn discovery_lists_all_plain_positive_arms() {
    let doc = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let tables = instrumentable_tables(&doc).unwrap();
    let total: usize = tables.iter().map(|(_, n)| n).sum();
    assert_eq!(tables.len(), 591);
    assert_eq!(total, 3282);
    assert!(tables.iter().any(|(id, n)| id == "main:n105:s1" && *n == 5));
}

#[test]
fn every_plain_positive_arm_is_discovered_and_only_those_are() {
    let doc = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let eligible: BTreeSet<String> = instrumentable_tables(&doc)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let main = doc.to_thk().unwrap();
    let parsed = Thk::parse(main.clone()).unwrap();
    let mut excluded = 0;
    let mut plain_total = 0;
    for table in doc.tables() {
        let plain = (0..table.branches.len())
            .filter(|&i| table.branches[i].weight > 0)
            .filter(|&i| {
                let s = &parsed.nodes[table.node].segments[table.branches[i].segment];
                s.external_thk == 55
                    && s.local == -1
                    && s.action == 0
                    && (0..=2).contains(&s.function)
                    && s.flow == 0
                    && matches!(s.marker, 0x40 | 0x80 | 0xc0)
            })
            .count();
        plain_total += plain;
        if !eligible.contains(&table.id) {
            excluded += 1;
            assert_eq!(
                plain, 0,
                "table {} has plain arms but was not discovered",
                table.id
            );
        }
    }
    assert_eq!(plain_total, 3282);
    assert_eq!(excluded, 12);
    assert_eq!(eligible.len(), 591);
}
