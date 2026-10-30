#![cfg(feature = "game-fixtures")]
use fatalis_adaptive::{
    instrument::{compile, instrumentable_tables, Registers},
    thk::Thk,
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
fn digits(marker: &fatalis_adaptive::instrument::OptionMarker) -> [u8; 5] {
    let code = marker.table_code;
    let exit = marker.exit_index as u16;
    [
        (code / 100) as u8,
        ((code / 10) % 10) as u8,
        (code % 10) as u8,
        ((exit / 10) % 10) as u8,
        (exit % 10) as u8,
    ]
}

#[test]
fn wrappers_preserve_original_calls_and_weights() {
    let d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let result = compile(&d, &["main:n105:s1".into()], registers()).unwrap();
    let old = Thk::parse(MAIN.to_vec()).unwrap();
    let new = Thk::parse(result.bytes).unwrap();
    assert_eq!(result.manifest.markers.len(), 5);
    assert_eq!(new.nodes.len(), old.nodes.len() + 5);
    for (index, n) in old.nodes.iter().enumerate() {
        assert_eq!(n.id, new.nodes[index].id);
        assert_eq!(n.segments.len(), new.nodes[index].segments.len());
        for (j, s) in n.segments.iter().enumerate() {
            let mut expected = MAIN[s.offset..(s.offset + 128).min(MAIN.len())].to_vec();
            if let Some(m) = result.manifest.markers.iter().find(|m| {
                index == 105 && d.table(&m.table_id).unwrap().branches[m.exit_index].segment == j
            }) {
                expected[40..44].copy_from_slice(&(-1i32).to_le_bytes());
                expected[44..48].copy_from_slice(&(-1i32).to_le_bytes());
                expected[48..56].copy_from_slice(&(m.wrapper_node as i64).to_le_bytes());
            }
            let actual = &new.bytes[new.nodes[index].segments[j].offset
                ..new.nodes[index].segments[j].offset + expected.len()];
            assert_eq!(&expected, actual);
        }
    }
    for m in &result.manifest.markers {
        assert_eq!(m.id, i32::from(m.table_code) * 100 + m.exit_index as i32);
        let s = &new.nodes[m.wrapper_node].segments;
        let mut k = 0usize;
        for (slot, digit) in [14u8, 15, 16, 17, 18].iter().zip(digits(m)) {
            assert_eq!(s[k].function, 0x80 + i32::from(*slot));
            assert_eq!(s[k].p1, 1);
            k += 1;
            for _ in 0..digit {
                assert_eq!(s[k].function, 0x80 + i32::from(*slot));
                assert_eq!(s[k].p1, 0);
                k += 1;
            }
        }
        assert_eq!(s[k].function, 0x93, "counter slot 19 increments last");
        assert_eq!(s[k].p1, 0);
        k += 1;
        assert_eq!(s[k].external_thk, 55);
        assert_eq!(s[k].external_id, m.original_global_id);
        assert_eq!(s[k + 1].flow, 8);
        assert_eq!(s[k + 1].function, 2);
        assert_eq!(s[k + 2].marker, 1);
        assert_eq!(s.len(), k + 3);
    }
}

#[test]
fn wrapper_shapes_match_existing_bytecode() {
    let old = Thk::parse(MAIN.to_vec()).unwrap();
    for segment in [14, 26] {
        let s = &old.nodes[4].segments[segment];
        assert_eq!(s.function, 0);
        assert!(s.local >= 0);
        assert_eq!(s.marker, 0x40);
        assert!(s.p1 > 0);
    }
    assert_eq!(old.nodes[5].segments[1].external_thk, 55);
    assert_eq!(old.nodes[5].segments[11].flow, 8);
    let end = &old.nodes[5].segments[12];
    assert_eq!(end.marker, 1);
    let raw = &MAIN[end.offset..end.offset + 128];
    assert_eq!(
        i32::from_le_bytes(raw[36..40].try_into().unwrap()),
        10005,
        "existing node end encodes node index + 10000"
    );

    let d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let result = compile(&d, &["main:n105:s1".into()], registers()).unwrap();
    let new = Thk::parse(result.bytes.clone()).unwrap();
    let marker = &result.manifest.markers[0];
    let w = &new.nodes[marker.wrapper_node];
    let end = w.segments.last().unwrap();
    let raw = &result.bytes[end.offset..end.offset + 128];
    assert_eq!(
        i32::from_le_bytes(raw[36..40].try_into().unwrap()),
        marker.wrapper_node as i32 + 10000
    );
    let table = d.table(&marker.table_id).unwrap();
    let head = &new.nodes[table.node].segments[table.branches[marker.exit_index].segment];
    assert_eq!(head.function, 0, "weighted option head keeps function 0");
    assert!(head.p1 > 0, "weight must be preserved");
    assert_eq!(head.local, marker.wrapper_node as i64);
}

#[test]
fn node105_five_tables_have_30_stable_marker_ids() {
    let d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let tables = [
        "main:n105:s1",
        "main:n105:s11",
        "main:n105:s21",
        "main:n105:s31",
        "main:n105:s41",
    ];
    let result = compile(&d, &tables.map(String::from), registers()).unwrap();
    assert_eq!(result.manifest.markers.len(), 30);
    let mut counts = std::collections::BTreeMap::new();
    for marker in &result.manifest.markers {
        assert_eq!(
            marker.id,
            i32::from(marker.table_code) * 100 + marker.exit_index as i32
        );
        *counts.entry(marker.table_id.clone()).or_insert(0) += 1;
    }
    assert_eq!(counts["main:n105:s1"], 5);
    assert_eq!(counts["main:n105:s11"], 6);
    assert_eq!(counts["main:n105:s21"], 6);
    assert_eq!(counts["main:n105:s31"], 6);
    assert_eq!(counts["main:n105:s41"], 7);
    let first = result
        .manifest
        .markers
        .iter()
        .find(|m| m.table_id == "main:n105:s1")
        .unwrap();
    assert_eq!(first.table_code, 0);
    assert_eq!(first.id, first.exit_index as i32);
}

#[test]
fn ids_stable_across_target_selection_order() {
    let d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let a = compile(
        &d,
        &["main:n105:s1".into(), "main:n105:s11".into()],
        registers(),
    )
    .unwrap();
    let b = compile(
        &d,
        &["main:n105:s11".into(), "main:n105:s1".into()],
        registers(),
    )
    .unwrap();
    for x in a.manifest.markers {
        assert_eq!(
            x.id,
            b.manifest
                .markers
                .iter()
                .find(|m| m.table_id == x.table_id && m.exit_index == x.exit_index)
                .unwrap()
                .id
        );
    }
}

#[test]
fn all_instrumentable_tables_fit_decimal_codes_and_parse() {
    let d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let tables: Vec<String> = instrumentable_tables(&d)
        .unwrap()
        .into_iter()
        .map(|(table, _)| table)
        .collect();
    assert_eq!(tables.len(), 591);
    let result = compile(&d, &tables, registers()).unwrap();
    assert_eq!(result.manifest.markers.len(), 3282);
    assert!(
        result.bytes.len() > 10_000_000,
        "all 3282 wrappers should be substantially larger than the 960 KB baseline"
    );
    let new = Thk::parse(result.bytes.clone()).unwrap();
    assert_eq!(new.nodes.len(), 223 + 3282);
    let ids: std::collections::BTreeSet<_> = result.manifest.markers.iter().map(|m| m.id).collect();
    assert_eq!(ids.len(), 3282, "packed IDs must be unique");
    assert!(result
        .manifest
        .markers
        .iter()
        .all(|m| m.table_code < 591 && m.exit_index < 13));
    for m in &result.manifest.markers {
        let s = &new.nodes[m.wrapper_node].segments;
        assert_eq!(s.last().unwrap().marker, 1);
        assert_eq!(
            i32::from_le_bytes(
                result.bytes[s.last().unwrap().offset + 36..s.last().unwrap().offset + 40]
                    .try_into()
                    .unwrap()
            ),
            m.wrapper_node as i32 + 10000
        );
    }
}

#[test]
fn invalid_slots_and_empty_tables_fail_closed() {
    let d = AiDocument::parse(MAIN, GLOBAL).unwrap();
    let mut invalid = registers();
    invalid.table_code[0] = 22;
    assert!(compile(&d, &["main:n105:s1".into()], invalid).is_err());
    assert!(compile(&d, &[], registers()).is_err());
}
