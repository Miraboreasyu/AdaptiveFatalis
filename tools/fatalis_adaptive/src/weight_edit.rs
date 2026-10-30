//! Offline, explicit per-table edits; no learning, game process, or deployment.
use crate::thk::{sha256, Project, Thk};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    /// Hash of the file the user actually inspected, preventing stale edits.
    pub source_sha256: String,
    /// Stable table ID -> weights in original arm order, including zero arms.
    pub tables: BTreeMap<String, Vec<i32>>,
}
#[derive(Debug, Serialize)]
pub struct TableChange {
    pub table_id: String,
    pub before: Vec<i32>,
    pub after: Vec<i32>,
    pub weight_offsets: Vec<usize>,
}
#[derive(Debug, Serialize)]
pub struct EditReport {
    pub source_sha256: String,
    pub output_sha256: String,
    pub changed_tables: Vec<TableChange>,
    pub changed_byte_offsets: Vec<usize>,
    pub unchanged_length: usize,
    pub non_weight_bytes_identical: bool,
}
pub fn apply(
    project: &Project,
    current: &[u8],
    request: &EditRequest,
) -> Result<(Vec<u8>, EditReport), String> {
    let source_sha256 = sha256(current);
    if request.source_sha256 != source_sha256 {
        return Err("edit source_sha256 does not match input THK".into());
    }
    // Compatibility adapter only. The document API owns editing and serialization.
    let mut document =
        crate::AiDocument::parse_current(&project.main.bytes, &project.global.bytes, current)?;
    for (id, weights) in &request.tables {
        document.set_weights(id, weights)?;
    }
    let output = document.to_thk()?;
    let parsed = Thk::parse(current.to_vec())?;
    let mut changed_tables = Vec::new();
    let mut allowed = BTreeSet::new();
    for (id, after) in &request.tables {
        let table = project
            .blocks
            .iter()
            .find(|b| &b.id == id)
            .ok_or("unknown table")?;
        let before: Vec<i32> = table
            .branches
            .iter()
            .map(|b| parsed.nodes[table.node].segments[b.segment].p1)
            .collect();
        let weight_offsets: Vec<usize> = table
            .branches
            .iter()
            .map(|b| project.main.nodes[table.node].segments[b.segment].offset + 12)
            .collect();
        for &offset in &weight_offsets {
            allowed.extend(offset..offset + 4);
        }
        if before != *after {
            changed_tables.push(TableChange {
                table_id: id.clone(),
                before,
                after: after.clone(),
                weight_offsets,
            });
        }
    }
    let changed_byte_offsets: Vec<usize> = current
        .iter()
        .zip(&output)
        .enumerate()
        .filter_map(|(i, (a, b))| (a != b).then_some(i))
        .collect();
    if current.len() != output.len() || changed_byte_offsets.iter().any(|i| !allowed.contains(i)) {
        return Err("internal invariant violated: non-requested-weight byte changed".into());
    }
    let report = EditReport {
        source_sha256,
        output_sha256: sha256(&output),
        changed_tables,
        changed_byte_offsets,
        unchanged_length: current.len(),
        non_weight_bytes_identical: true,
    };
    Ok((output, report))
}
