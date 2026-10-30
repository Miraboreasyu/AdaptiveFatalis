//! Structural, opt-in THK instrumentation. Register slots must be audited separately.
//!
//! Every wrapped arm publishes a decimal event code into six int32 registers:
//! three digits of a table code, two digits of the option/exit index, and a
//! trailing event counter. Native THK has no constant assignment, so each digit
//! is cleared then incremented the required number of times. The counter is
//! written last; readers accept a sample only when the counter is unchanged
//! across the digit reads.
use crate::thk::Thk;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registers {
    /// Decimal digits of the table code: hundreds, tens, units.
    pub table_code: [u8; 3],
    /// Decimal digits of the exit/option index: tens, units.
    pub option: [u8; 2],
    pub counter: u8,
}
impl Registers {
    pub fn slots(&self) -> [u8; 6] {
        [
            self.table_code[0],
            self.table_code[1],
            self.table_code[2],
            self.option[0],
            self.option[1],
            self.counter,
        ]
    }
    pub fn validate(&self) -> Result<(), String> {
        let slots = self.slots();
        if slots.iter().any(|slot| *slot > 21) {
            return Err("register slots must be physical slots in 0..21".into());
        }
        if slots
            .iter()
            .enumerate()
            .any(|(i, slot)| slots[..i].contains(slot))
        {
            return Err("register slots must be six distinct physical slots".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OptionMarker {
    /// Packed ID = table_code * 100 + exit_index.
    pub id: i32,
    pub table_code: u16,
    pub table_id: String,
    pub exit_index: usize,
    pub wrapper_node: usize,
    pub original_global_id: i32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub source_sha256: String,
    pub output_sha256: String,
    pub registers: Registers,
    pub markers: Vec<OptionMarker>,
    pub safety: String,
}
#[derive(Clone, Debug)]
pub struct InstrumentedThk {
    pub bytes: Vec<u8>,
    pub manifest: Manifest,
}

impl Manifest {
    /// IDs are stable for a fixed target set; persist this manifest with its THK.
    /// Changing the target set requires a new tracker, never an old map.
    pub fn option_map(&self) -> crate::register_learning::OptionMap {
        self.markers
            .iter()
            .map(|m| (m.id, (m.table_id.clone(), m.exit_index)))
            .collect()
    }
}

fn segment() -> [u8; 128] {
    let mut b = [0; 128];
    b[8..12].copy_from_slice(&2i32.to_le_bytes());
    b[40..44].copy_from_slice(&(-1i32).to_le_bytes());
    b[44..48].copy_from_slice(&(-1i32).to_le_bytes());
    b[48..56].copy_from_slice(&(-1i64).to_le_bytes());
    b
}
fn unary(r: u8) -> i32 {
    if r < 20 {
        0x80 + i32::from(r)
    } else {
        0xa8 + 2 * i32::from(r - 20)
    }
}
fn put(b: &mut [u8], offset: usize, n: i32) {
    b[offset..offset + 4].copy_from_slice(&n.to_le_bytes());
}
fn digits(value: u16) -> [u8; 3] {
    [
        (value / 100) as u8,
        ((value / 10) % 10) as u8,
        (value % 10) as u8,
    ]
}
fn two_digits(value: u16) -> [u8; 2] {
    [((value / 10) % 10) as u8, (value % 10) as u8]
}

/// The exact structural subset the wrapper writer supports: one plain Global
/// entry call per originally positive arm, no opaque metadata or control.
fn plain_entry(
    main: &[u8],
    parsed: &Thk,
    table: &crate::thk::RandomBlock,
    exit_index: usize,
) -> Result<(), String> {
    let branch = &table.branches[exit_index];
    let s = &parsed.nodes[table.node].segments[branch.segment];
    if s.external_thk != 55
        || s.local != -1
        || s.action != 0
        || s.function > 2
        || s.function < 0
        || s.flow != 0
    {
        return Err(format!(
            "unsupported non-plain Global entry {} exit {exit_index}",
            table.id
        ));
    }
    let raw = &main[s.offset..(s.offset + 128).min(main.len())];
    let opaque_nonzero = raw
        .iter()
        .enumerate()
        .any(|(i, b)| !matches!(i, 0 | 2 | 8..=15 | 40..=55) && *b != 0);
    if opaque_nonzero || !matches!(s.marker, 0x40 | 0x80 | 0xc0) {
        return Err(format!(
            "unsupported metadata on {} exit {exit_index}",
            table.id
        ));
    }
    Ok(())
}
/// Discover tables the current wrapper writer can instrument, with the number
/// of originally positive arms. Discovery does not reserve registers or deploy.
pub fn instrumentable_tables(document: &crate::AiDocument) -> Result<Vec<(String, usize)>, String> {
    let main = document.to_thk()?;
    let parsed = Thk::parse(main.clone())?;
    let mut result = Vec::new();
    for table in document.tables() {
        let exits = (0..table.branches.len())
            .filter(|&i| !document.original_zero(&table.id, i))
            .collect::<Vec<_>>();
        // Mixed tables are included for their plain direct-Global arms; local-call
        // or metadata-bearing arms are left untouched and simply do not emit events.
        let plain = exits
            .iter()
            .filter(|&&i| plain_entry(&main, &parsed, table, i).is_ok())
            .count();
        if plain > 0 {
            result.push((table.id.clone(), plain));
        }
    }
    Ok(result)
}

/// Rebuild node table/data offsets and append wrapper nodes. Original node indices,
/// IDs, segment counts and opaque segment bytes are preserved (except call targets).
/// Only explicit leaf tables with unconditional, direct Global entry calls are supported.
/// This function never claims unused slots are engine-safe or allocates engine storage.
pub fn compile(
    document: &crate::AiDocument,
    table_ids: &[String],
    registers: Registers,
) -> Result<InstrumentedThk, String> {
    registers.validate()?;
    if table_ids.is_empty() {
        return Err("explicit nonempty target table list required".into());
    }
    let slots = registers.slots();
    let main = document.to_thk()?;
    let parsed = Thk::parse(main.clone())?;
    let mut audit = crate::register_audit::RegisterAudit::default();
    audit.add("main", &main)?;
    audit.add("Global", &document.global_to_thk()?)?;
    audit.require_unreferenced(&slots)?;
    let mut nodes: Vec<Vec<Vec<u8>>> = parsed
        .nodes
        .iter()
        .map(|n| {
            n.segments
                .iter()
                .map(|s| main[s.offset..(s.offset + 128).min(main.len())].to_vec())
                .collect()
        })
        .collect();
    let mut ids: Vec<u32> = parsed.nodes.iter().map(|n| n.id).collect();
    let mut seen = BTreeSet::new();
    let mut markers = Vec::new();
    let mut ordered = table_ids.to_vec();
    ordered.sort();
    for (table_index, table_id) in ordered.iter().enumerate() {
        if !seen.insert(table_id) {
            return Err("duplicate target table".into());
        }
        let table_code = u16::try_from(table_index).map_err(|_| "table code overflow")?;
        if table_code > 999 {
            return Err("more than 1000 target tables cannot be encoded".into());
        }
        let table = document
            .table(table_id)
            .ok_or_else(|| format!("unknown table {table_id}"))?;
        let mut table_markers = 0usize;
        for (exit_index, branch) in table.branches.iter().enumerate() {
            // Original zero arms are retained as-is. They cannot generate events.
            if document.original_zero(table_id, exit_index) {
                continue;
            }
            // Mixed tables: only direct plain Global arms get wrappers. Preserve the
            // original arm for nested/local/metadata options instead of rejecting it.
            if plain_entry(&main, &parsed, table, exit_index).is_err() {
                continue;
            }
            table_markers += 1;
            let exit = u16::try_from(exit_index).map_err(|_| "exit index overflow")?;
            if exit > 99 {
                return Err(format!(
                    "table {table_id} has more than 100 arms; decimal option encoding cannot represent it"
                ));
            }
            let id = i32::from(table_code) * 100 + i32::from(exit);
            let s = &parsed.nodes[table.node].segments[branch.segment];
            let wrapper_node = nodes.len();
            let mut body = Vec::new();
            let code_digits = digits(table_code);
            let option_digits = two_digits(exit);
            for (slot, digit) in slots
                .iter()
                .zip(code_digits.iter().chain(option_digits.iter()).copied())
            {
                let mut clear = segment();
                put(&mut clear, 8, unary(*slot));
                put(&mut clear, 12, 1);
                body.push(clear.to_vec());
                let mut one = segment();
                put(&mut one, 8, unary(*slot));
                let one = one.to_vec();
                for _ in 0..digit {
                    body.push(one.clone());
                }
            }
            let mut increment = segment();
            put(&mut increment, 8, unary(slots[5]));
            let mut call = segment();
            put(&mut call, 40, s.external_thk);
            put(&mut call, 44, s.external_id);
            let mut ret = segment();
            ret[1] = 8;
            let mut end = segment();
            end[0] = 1;
            put(
                &mut end,
                36,
                i32::try_from(wrapper_node + 10000).map_err(|_| "node end overflow")?,
            );
            body.extend([
                increment.to_vec(),
                call.to_vec(),
                ret.to_vec(),
                end.to_vec(),
            ]);
            nodes.push(body);
            // Local wrappers are addressed by index, never by node hash.
            ids.push(0);
            let head = &mut nodes[table.node][branch.segment];
            put(head, 40, -1);
            put(head, 44, -1);
            head[48..56].copy_from_slice(&(wrapper_node as i64).to_le_bytes());
            markers.push(OptionMarker {
                id,
                table_code,
                table_id: table_id.clone(),
                exit_index,
                wrapper_node,
                original_global_id: s.external_id,
            });
        }
        if table_markers == 0 {
            return Err(format!(
                "target table {table_id} has no direct plain positive arm to instrument"
            ));
        }
    }
    let count = i16::try_from(nodes.len()).map_err(|_| "THK node-count overflow")?;
    let mut out = main[..32].to_vec();
    out[6..8].copy_from_slice(&count.to_le_bytes());
    out.resize(32 + nodes.len() * 16, 0);
    for (index, segments) in nodes.iter().enumerate() {
        let offset = out.len();
        let t = 32 + index * 16;
        out[t..t + 8].copy_from_slice(&(offset as u64).to_le_bytes());
        out[t + 8..t + 12].copy_from_slice(&(segments.len() as u32).to_le_bytes());
        out[t + 12..t + 16].copy_from_slice(&ids[index].to_le_bytes());
        for (j, s) in segments.iter().enumerate() {
            out.extend_from_slice(s);
            // Original optional truncated final padding becomes mandatory once data follows it.
            if s.len() < 128 && !(index + 1 == nodes.len() && j + 1 == segments.len()) {
                out.resize(out.len() + 128 - s.len(), 0);
            }
        }
    }
    Thk::parse(out.clone())?;
    Ok(InstrumentedThk {
        manifest: Manifest {
            schema: 2,
            source_sha256: crate::thk::sha256(&main),
            output_sha256: crate::thk::sha256(&out),
            registers,
            markers,
            safety: "Explicit physical slots, NOT allocation. main/Global are audited, but other loaded THKs and engine use of slots 14..19 are not. The derived cThinkEm chain (monster+0x12278 -> AIData+0xAA8 -> cThinkEm+0x640) is source-derived from the rebuilt executable, not live-verified on the installed build. Decimal digits are written before counter++; a stable counter implies the digit writes completed, but multi-register publication is not atomic.".into(),
        },
        bytes: out,
    })
}
