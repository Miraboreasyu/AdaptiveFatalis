//! In-memory editing API. No process access, file I/O, CLI or deployment.
use crate::{
    geometry::{GeometryModel, Query, QueryResult},
    thk::{Project, RandomBlock, Thk},
};

/// One owned Fatalis AI document. Parsed segments are authoritative; tables are
/// a synchronized semantic index. Opaque header/padding/unknown fields are retained.
#[derive(Clone, Debug)]
pub struct AiDocument {
    project: Project,
    original_zero_arms: Vec<Vec<bool>>,
}
impl AiDocument {
    /// Parse a verified original main/Global pair into an editable document.
    pub fn parse(main: &[u8], global: &[u8]) -> Result<Self, String> {
        let project = Project::from_bytes(main.to_vec(), global.to_vec())?;
        let original_zero_arms = project
            .blocks
            .iter()
            .map(|b| b.branches.iter().map(|r| r.weight == 0).collect())
            .collect();
        Ok(Self {
            project,
            original_zero_arms,
        })
    }
    /// Reopen a saved document using a verified original as format/identity reference.
    /// Only probabilities may differ. Imported values become the current model state.
    pub fn parse_current(
        original_main: &[u8],
        global: &[u8],
        current_main: &[u8],
    ) -> Result<Self, String> {
        let mut doc = Self::parse(original_main, global)?;
        doc.project
            .patch_current_main_weights(current_main, &Default::default())?;
        doc.project.main = Thk::parse(current_main.to_vec())?;
        for table in &mut doc.project.blocks {
            for branch in &mut table.branches {
                branch.weight = doc.project.main.nodes[table.node].segments[branch.segment].p1;
            }
        }
        Ok(doc)
    }
    /// Current semantic table view, never the immutable baseline probabilities.
    pub fn tables(&self) -> &[RandomBlock] {
        &self.project.blocks
    }
    pub fn table(&self, id: &str) -> Option<&RandomBlock> {
        self.tables().iter().find(|t| t.id == id)
    }
    pub(crate) fn original_zero(&self, id: &str, exit: usize) -> bool {
        self.project
            .blocks
            .iter()
            .position(|t| t.id == id)
            .and_then(|i| self.original_zero_arms[i].get(exit))
            .copied()
            .unwrap_or(true)
    }
    /// Replace one complete distribution atomically. Does not serialize or modify bytes.
    /// A currently zeroed formerly-positive arm may be enabled again; original zero arms remain fixed.
    pub fn set_weights(&mut self, id: &str, weights: &[i32]) -> Result<(), String> {
        let index = self
            .project
            .blocks
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| format!("unknown table {id}"))?;
        let table = &self.project.blocks[index];
        if weights.len() != table.branches.len()
            || weights.iter().any(|w| !(0..=100).contains(w))
            || weights.iter().map(|&w| i64::from(w)).sum::<i64>() != 100
        {
            return Err(format!("invalid probability distribution for {id}"));
        }
        if self.original_zero_arms[index]
            .iter()
            .zip(weights)
            .any(|(&zero, &w)| zero && w != 0)
        {
            return Err("cannot activate original zero arm".into());
        }
        let node = table.node;
        for (branch, &weight) in self.project.blocks[index].branches.iter_mut().zip(weights) {
            branch.weight = weight;
            self.project.main.nodes[node].segments[branch.segment].p1 = weight;
        }
        Ok(())
    }
    /// Query the same current in-memory model that learning edits and serialization reads.
    pub fn query(&self, query: &Query) -> Result<QueryResult, String> {
        GeometryModel::new(&self.project).query(query)
    }
    /// Compile/serialize the parsed document to binary THK bytes.
    /// No edit request, byte offset, baseline file or subprocess is needed at export time.
    pub fn to_thk(&self) -> Result<Vec<u8>, String> {
        self.project.main.to_bytes()
    }
    /// Serialize this editable model with explicit innermost-table instrumentation.
    /// The manifest is the authoritative event-ID to table/option mapping.
    /// Slots are physical; this API does not claim to allocate game registers.
    pub fn to_instrumented_thk(
        &self,
        table_ids: &[String],
        registers: crate::instrument::Registers,
    ) -> Result<crate::instrument::InstrumentedThk, String> {
        crate::instrument::compile(self, table_ids, registers)
    }
    pub fn global_to_thk(&self) -> Result<Vec<u8>, String> {
        self.project.global.to_bytes()
    }
}
