//! Fatalis-only, byte-preserving THK reader. No game access or deployment.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub const MAIN_SHA256: &str = "61b879d2b24eaec808bea24e69371c98d7c789d1f9409d62860dcf2bbbc6f736";
pub const GLOBAL_SHA256: &str = "8139631c70c8fa068db94f650a5e065a1c6a408b480bf53e6de80287323e682f";
const STRIDE: usize = 128;
fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Guard {
    pub function: i32,
    pub parameter1: i32,
    pub parameter2: i32,
    pub negated: bool,
}
impl Guard {
    /// None means unimplemented/internal state, never true by default.
    pub fn geometry_matches(&self, distance: f64, angle_cw: f64) -> Option<bool> {
        if !distance.is_finite() || distance < 0.0 || !angle_cw.is_finite() {
            return None;
        }
        let yes = match self.function {
            8 => distance <= self.parameter1 as f64,
            9 => distance > self.parameter1 as f64,
            0x16 => {
                let a = angle_cw.rem_euclid(360.0);
                let lo = self.parameter1 as f64;
                let hi = self.parameter2 as f64;
                // Exact boundary semantics are not runtime-verified. Exclude boundaries.
                if a == lo || a == hi {
                    return None;
                }
                if lo <= hi {
                    a > lo && a < hi
                } else {
                    a > lo || a < hi
                }
            }
            _ => return None,
        };
        Some(if self.negated { !yes } else { yes })
    }
}
#[derive(Debug, Clone)]
pub struct Segment {
    pub offset: usize,
    pub marker: u8,
    pub flow: u8,
    pub branch: u8,
    pub function: i32,
    pub p1: i32,
    pub p2: i32,
    pub external_thk: i32,
    pub external_id: i32,
    pub local: i64,
    pub action: i32,
}
#[derive(Debug, Clone)]
pub struct Node {
    pub offset: usize,
    pub id: u32,
    pub segments: Vec<Segment>,
}
#[derive(Debug, Clone)]
pub struct Thk {
    pub bytes: Vec<u8>,
    pub nodes: Vec<Node>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub id: String,
    pub segment: usize,
    /// Exclusive bound; a nested random block remains inside its owning branch.
    pub end_segment: usize,
    pub weight: i32,
    /// Conservative recursive union, NOT a guaranteed first/terminal action.
    pub action_ids: Vec<i32>,
    /// Local lexical guards only. Model adds an explicitly verified upstream route.
    pub guards: Vec<Guard>,
    pub unknown_guard: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RandomBlock {
    pub id: String,
    pub node: usize,
    pub start_segment: usize,
    pub end_segment: usize,
    pub branches: Vec<Branch>,
}
#[derive(Debug, Clone)]
pub struct Project {
    pub main: Thk,
    pub global: Thk,
    pub blocks: Vec<RandomBlock>,
}

impl Thk {
    /// Serialize the parsed fields at their existing layout, preserving opaque bytes.
    /// This fixed-layout writer does not support adding/removing nodes or segments.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let layout = Self::parse(self.bytes.clone())?;
        if self.nodes.len() != layout.nodes.len() {
            return Err("node insertion/removal is unsupported".into());
        }
        let mut out = self.bytes.clone();
        for (index, (node, original)) in self.nodes.iter().zip(&layout.nodes).enumerate() {
            if node.offset != original.offset || node.segments.len() != original.segments.len() {
                return Err("node relocation/segment insertion is unsupported".into());
            }
            out[32 + index * 16 + 12..32 + index * 16 + 16].copy_from_slice(&node.id.to_le_bytes());
            for (s, old) in node.segments.iter().zip(&original.segments) {
                if s.offset != old.offset {
                    return Err("segment relocation is unsupported".into());
                }
                let o = s.offset;
                out[o] = s.marker;
                out[o + 1] = s.flow;
                out[o + 2] = s.branch;
                for (relative, value) in [
                    (8, s.function),
                    (12, s.p1),
                    (32, s.p2),
                    (40, s.external_thk),
                    (44, s.external_id),
                    (76, s.action),
                ] {
                    out[o + relative..o + relative + 4].copy_from_slice(&value.to_le_bytes());
                }
                out[o + 48..o + 56].copy_from_slice(&s.local.to_le_bytes());
            }
        }
        Self::parse(out.clone())?;
        Ok(out)
    }
    /// Structural parser. Original identity is checked by Project::from_bytes.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() < 32
            || &bytes[..4] != b"THK\0"
            || u16::from_le_bytes([bytes[4], bytes[5]]) != 40
            || u64_at(&bytes, 24) != 32
        {
            return Err("invalid THK v40 header".into());
        }
        let count = i16::from_le_bytes([bytes[6], bytes[7]]);
        if count <= 0 || 32 + count as usize * 16 > bytes.len() {
            return Err("invalid THK node count/table".into());
        }
        let table_end = 32 + count as usize * 16;
        let mut nodes = Vec::new();
        let mut ids = BTreeSet::new();
        let mut intervals = Vec::new();
        for n in 0..count as usize {
            let t = 32 + n * 16;
            let offset = usize::try_from(u64_at(&bytes, t)).map_err(|_| "node offset overflow")?;
            let count = u32_at(&bytes, t + 8) as usize;
            let id = u32_at(&bytes, t + 12);
            if id != 0 && !ids.insert(id) {
                return Err(format!("duplicate nonzero node id {id}"));
            }
            let end = offset
                .checked_add(count.checked_mul(STRIDE).ok_or("node size overflow")?)
                .ok_or("node end overflow")?;
            if count > 0 && (offset < table_end || end > bytes.len().saturating_add(12)) {
                return Err(format!("node {n} outside file"));
            }
            if count > 0 {
                intervals.push((offset, end));
            }
            let mut segments = Vec::new();
            let mut if_depth = 0usize;
            let mut random_sums = Vec::<i64>::new();
            for s in 0..count {
                let o = offset + s * STRIDE;
                // Only final optional 12-byte padding may be absent. Never zero-extend output.
                if o.checked_add(116).ok_or("segment overflow")? > bytes.len() {
                    return Err(format!("truncated required segment fields {n}:{s}"));
                }
                if o + STRIDE > bytes.len() && (s + 1 != count || end - bytes.len() > 12) {
                    return Err("non-final truncated segment".into());
                }
                let seg = Segment {
                    offset: o,
                    marker: bytes[o],
                    flow: bytes[o + 1],
                    branch: bytes[o + 2],
                    function: i32_at(&bytes, o + 8),
                    p1: i32_at(&bytes, o + 12),
                    p2: i32_at(&bytes, o + 32),
                    external_thk: i32_at(&bytes, o + 40),
                    external_id: i32_at(&bytes, o + 44),
                    local: i64::from_le_bytes(bytes[o + 48..o + 56].try_into().unwrap()),
                    action: i32_at(&bytes, o + 76),
                };
                match seg.branch {
                    2 => if_depth += 1,
                    4 if if_depth == 0 => return Err(format!("orphan else {n}:{s}")),
                    8 => {
                        if if_depth == 0 {
                            return Err("orphan endif".into());
                        }
                        if_depth -= 1;
                    }
                    _ => {}
                }
                match seg.marker {
                    0x40 => {
                        if !(0..=100).contains(&seg.p1) {
                            return Err("random weight outside 0..100".into());
                        }
                        random_sums.push(seg.p1 as i64);
                    }
                    0x80 | 0xc0 => {
                        if !(0..=100).contains(&seg.p1) {
                            return Err("random weight outside 0..100".into());
                        }
                        *random_sums.last_mut().ok_or("orphan random alternative")? +=
                            seg.p1 as i64;
                    }
                    _ => {}
                }
                if seg.branch == 1 && random_sums.pop() != Some(100) {
                    return Err(format!("random weights do not sum to 100 at {n}:{s}"));
                }
                segments.push(seg);
            }
            if if_depth != 0 || !random_sums.is_empty() {
                return Err(format!("unbalanced control structures in node {n}"));
            }
            nodes.push(Node {
                offset,
                id,
                segments,
            });
        }
        intervals.sort_unstable();
        if intervals.windows(2).any(|w| w[0].1 > w[1].0) {
            return Err("overlapping node segments".into());
        }
        Ok(Self { bytes, nodes })
    }
}

fn lexical_guards(node: &Node) -> Vec<Vec<Guard>> {
    let mut stack: Vec<Vec<Guard>> = Vec::new();
    let mut out = Vec::new();
    for s in &node.segments {
        let atom = Guard {
            function: s.function,
            parameter1: s.p1,
            parameter2: s.p2,
            negated: false,
        };
        match s.branch {
            2 => stack.push(vec![atom]),
            4 => {
                if let Some(chain) = stack.last_mut() {
                    if let Some(g) = chain.last_mut() {
                        if !g.negated {
                            g.negated = true;
                        }
                    }
                    // functionType 2 is unconditional else, not another tested predicate.
                    if s.function > 2 {
                        chain.push(atom);
                    }
                }
            }
            8 => {
                stack.pop();
            }
            _ => {}
        }
        out.push(stack.iter().flatten().cloned().collect());
    }
    out
}
impl Project {
    pub fn from_paths(main: impl AsRef<Path>, global: impl AsRef<Path>) -> Result<Self, String> {
        Self::from_bytes(
            fs::read(main).map_err(|e| e.to_string())?,
            fs::read(global).map_err(|e| e.to_string())?,
        )
    }
    pub fn from_bytes(main: Vec<u8>, global: Vec<u8>) -> Result<Self, String> {
        if sha256(&main) != MAIN_SHA256 {
            return Err("Fatalis main original SHA256 mismatch".into());
        }
        if sha256(&global) != GLOBAL_SHA256 {
            return Err("Fatalis Global original SHA256 mismatch".into());
        }
        let mut p = Self {
            main: Thk::parse(main)?,
            global: Thk::parse(global)?,
            blocks: Vec::new(),
        };
        if p.main.nodes.len() != 223 || p.global.nodes.len() != 155 {
            return Err("Fatalis node counts mismatch".into());
        }
        p.blocks = p.enumerate()?;
        if p.blocks.len() != 603 {
            return Err(format!(
                "expected all 603 main random blocks, found {}",
                p.blocks.len()
            ));
        }
        Ok(p)
    }
    pub fn random_blocks(&self) -> &[RandomBlock] {
        &self.blocks
    }
    fn ref_target(&self, is_global: bool, s: &Segment) -> Result<Option<(bool, usize)>, String> {
        if s.external_thk == 55 {
            let idx = self
                .global
                .nodes
                .iter()
                .position(|n| n.id == s.external_id as u32)
                .ok_or_else(|| format!("unresolved Global node id {}", s.external_id))?;
            return Ok(Some((true, idx)));
        }
        if s.external_thk != -1 {
            return Err(format!("unsupported external THK {}", s.external_thk));
        }
        if s.local >= 0 {
            let idx = s.local as usize;
            let count = if is_global {
                self.global.nodes.len()
            } else {
                self.main.nodes.len()
            };
            if idx >= count {
                return Err("local node reference out of bounds".into());
            }
            return Ok(Some((is_global, idx)));
        }
        Ok(None)
    }
    fn collect_actions(
        &self,
        is_global: bool,
        segments: &[Segment],
        seen: &mut BTreeSet<(bool, usize)>,
        actions: &mut BTreeSet<i32>,
    ) -> Result<(), String> {
        for s in segments {
            if s.action > 0 {
                actions.insert(s.action);
            }
            if let Some((g, n)) = self.ref_target(is_global, s)? {
                if seen.insert((g, n)) {
                    let node = if g {
                        &self.global.nodes[n]
                    } else {
                        &self.main.nodes[n]
                    };
                    self.collect_actions(g, &node.segments, seen, actions)?;
                }
            }
        }
        Ok(())
    }
    fn enumerate(&self) -> Result<Vec<RandomBlock>, String> {
        let mut blocks = Vec::new();
        for (n, node) in self.main.nodes.iter().enumerate() {
            let guards = lexical_guards(node);
            let mut stack: Vec<(usize, Vec<usize>)> = Vec::new();
            for (i, s) in node.segments.iter().enumerate() {
                if s.marker == 0x40 {
                    stack.push((i, vec![i]));
                } else if s.marker == 0x80 || s.marker == 0xc0 {
                    stack.last_mut().ok_or("orphan random")?.1.push(i);
                }
                if s.branch == 1 {
                    let (start, heads) = stack.pop().ok_or("orphan endr")?;
                    let mut branches = Vec::new();
                    for (j, h) in heads.iter().copied().enumerate() {
                        let end = heads.get(j + 1).copied().unwrap_or(i);
                        let mut actions = BTreeSet::new();
                        self.collect_actions(
                            false,
                            &node.segments[h..end],
                            &mut BTreeSet::new(),
                            &mut actions,
                        )?;
                        let unknown_guard = guards[h]
                            .iter()
                            .any(|g| !matches!(g.function, 8 | 9 | 0x16))
                            || node.segments[h..end].iter().any(|s| {
                                matches!(s.branch, 2 | 4) && !matches!(s.function, 2 | 8 | 9 | 0x16)
                            });
                        branches.push(Branch {
                            id: format!("main:n{n}:s{h}"),
                            segment: h,
                            end_segment: end,
                            weight: node.segments[h].p1,
                            action_ids: actions.into_iter().collect(),
                            guards: guards[h].clone(),
                            unknown_guard,
                        });
                    }
                    blocks.push(RandomBlock {
                        id: format!("main:n{n}:s{start}"),
                        node: n,
                        start_segment: start,
                        end_segment: i,
                        branches,
                    });
                }
            }
        }
        blocks.sort_by_key(|b| (b.node, b.start_segment));
        Ok(blocks)
    }
    /// Accept an already adapted main only when every difference is an original
    /// random-head parameter1 byte. Structure and each distribution remain valid.
    pub fn patch_current_main_weights(
        &self,
        current: &[u8],
        changes: &BTreeMap<String, Vec<i32>>,
    ) -> Result<Vec<u8>, String> {
        if current.len() != self.main.bytes.len() {
            return Err("current main length differs from baseline".into());
        }
        let allowed: BTreeSet<usize> = self
            .blocks
            .iter()
            .flat_map(|b| {
                b.branches.iter().flat_map(move |r| {
                    let o = self.main.nodes[b.node].segments[r.segment].offset + 12;
                    o..o + 4
                })
            })
            .collect();
        if current
            .iter()
            .zip(&self.main.bytes)
            .enumerate()
            .any(|(i, (a, b))| a != b && !allowed.contains(&i))
        {
            return Err("current main has non-weight modifications; refusing mixed mod".into());
        }
        let parsed = Thk::parse(current.to_vec())?;
        for b in &self.blocks {
            for r in &b.branches {
                if r.weight == 0 && parsed.nodes[b.node].segments[r.segment].p1 != 0 {
                    return Err("current main activates original zero arm".into());
                }
            }
        }
        let mut adapted = self.clone();
        adapted.main.bytes = current.to_vec();
        // Segment offsets and baseline zero-arm identities deliberately remain original.
        adapted.patch_main_weights(changes)
    }
    /// Only parameter1's four original bytes change. All original zero arms stay zero.
    pub fn patch_main_weights(
        &self,
        changes: &BTreeMap<String, Vec<i32>>,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = self.main.bytes.clone();
        let mut allowed = BTreeSet::new();
        for (id, weights) in changes {
            let block = self
                .blocks
                .iter()
                .find(|b| &b.id == id)
                .ok_or_else(|| format!("unknown random block {id}"))?;
            if weights.len() != block.branches.len()
                || weights.iter().any(|w| !(0..=100).contains(w))
                || weights.iter().map(|&w| w as i64).sum::<i64>() != 100
            {
                return Err(format!("invalid weights for {id}"));
            }
            for (branch, &w) in block.branches.iter().zip(weights) {
                if branch.weight == 0 && w != 0 {
                    return Err(format!("cannot activate original zero arm {}", branch.id));
                }
                let o = self.main.nodes[block.node].segments[branch.segment].offset + 12;
                bytes[o..o + 4].copy_from_slice(&w.to_le_bytes());
                allowed.extend(o..o + 4);
            }
        }
        if bytes.len() != self.main.bytes.len()
            || bytes
                .iter()
                .zip(&self.main.bytes)
                .enumerate()
                .any(|(i, (a, b))| a != b && !allowed.contains(&i))
        {
            return Err("patch escaped weight allowlist".into());
        }
        let parsed = Thk::parse(bytes.clone())?;
        if parsed.nodes.len() != self.main.nodes.len()
            || parsed.nodes.iter().zip(&self.main.nodes).any(|(a, b)| {
                a.id != b.id || a.offset != b.offset || a.segments.len() != b.segments.len()
            })
        {
            return Err("patch changed THK structure".into());
        }
        Ok(bytes)
    }
}

#[cfg(all(test, feature = "game-fixtures"))]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut b = vec![0u8; 48 + 4 * 128 - 12];
        b[..4].copy_from_slice(b"THK\0");
        b[4..6].copy_from_slice(&40u16.to_le_bytes());
        b[6..8].copy_from_slice(&1i16.to_le_bytes());
        b[24..32].copy_from_slice(&32u64.to_le_bytes());
        b[32..40].copy_from_slice(&48u64.to_le_bytes());
        b[40..44].copy_from_slice(&4u32.to_le_bytes());
        for (i, marker, w) in [(0, 0x40, 35i32), (1, 0x80, 65)] {
            let o = 48 + i * 128;
            b[o] = marker;
            b[o + 12..o + 16].copy_from_slice(&w.to_le_bytes());
        }
        b[48 + 2 * 128 + 2] = 1;
        b[48 + 3 * 128] = 1;
        b
    }
    #[test]
    fn final_optional_padding_is_not_extended() {
        let b = fixture();
        assert_eq!(Thk::parse(b.clone()).unwrap().bytes, b);
    }
    #[test]
    fn rejects_truncated_required_fields() {
        let mut b = fixture();
        b.truncate(b.len() - 1);
        assert!(Thk::parse(b).is_err());
    }
    #[test]
    fn rejects_non_normalized_random() {
        let mut b = fixture();
        b[60..64].copy_from_slice(&36i32.to_le_bytes());
        assert!(Thk::parse(b).is_err());
    }
    #[test]
    fn original_project_inventory_and_safe_patch() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research/fatalis_thk");
        if !root.join("em013_00.thk").exists() {
            return;
        }
        let p = Project::from_paths(root.join("em013_00.thk"), root.join("em013_55.thk")).unwrap();
        assert_eq!(p.blocks.len(), 603);
        assert_eq!(
            p.patch_main_weights(&BTreeMap::new()).unwrap(),
            p.main.bytes
        );
        let block = p
            .blocks
            .iter()
            .find(|b| b.branches.iter().filter(|r| r.weight > 1).count() >= 2)
            .unwrap();
        let mut weights: Vec<i32> = block.branches.iter().map(|r| r.weight).collect();
        let indices: Vec<usize> = weights
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > 1)
            .map(|(i, _)| i)
            .collect();
        weights[indices[0]] -= 1;
        weights[indices[1]] += 1;
        let changes = BTreeMap::from([(block.id.clone(), weights)]);
        let modified = p.patch_main_weights(&changes).unwrap();
        assert_eq!(modified.len(), 960004);
        assert_eq!(
            p.patch_current_main_weights(&modified, &BTreeMap::new())
                .unwrap(),
            modified
        );
        let mut corrupt = modified;
        corrupt[10] ^= 1;
        assert!(p
            .patch_current_main_weights(&corrupt, &BTreeMap::new())
            .is_err());
    }
    #[test]
    fn fixed_identity_rejects_unrelated_thk() {
        assert!(Project::from_bytes(fixture(), fixture()).is_err());
    }
    #[test]
    fn angle_wrap_and_unknown_boundary() {
        let g = Guard {
            function: 0x16,
            parameter1: 330,
            parameter2: 30,
            negated: false,
        };
        assert_eq!(g.geometry_matches(1.0, 359.0), Some(true));
        assert_eq!(g.geometry_matches(1.0, 180.0), Some(false));
        assert_eq!(g.geometry_matches(1.0, 30.0), None);
    }
}
