//! Static physical-slot audit. Absence of script references is not proof of engine safety.
use crate::thk::Thk;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterUse {
    pub source: String,
    pub node: usize,
    pub segment: usize,
    pub slot: u8,
    pub right_operand: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RegisterAudit {
    pub sources: BTreeSet<String>,
    pub uses: Vec<RegisterUse>,
}
impl RegisterAudit {
    /// Failure leaves the accumulated report untouched. Sources must be unique.
    pub fn add(&mut self, source: &str, bytes: &[u8]) -> Result<(), String> {
        if self.sources.contains(source) {
            return Err(format!("duplicate audit source {source}"));
        }
        let thk = Thk::parse(bytes.to_vec())?;
        let mut uses = Vec::new();
        for (node, n) in thk.nodes.iter().enumerate() {
            for (segment, s) in n.segments.iter().enumerate() {
                let f = s.function.unsigned_abs();
                let slot = match f {
                    0x80..=0x93 => Some(f - 0x80),
                    0x94..=0xa7 => Some(f - 0x94),
                    0xa8 | 0xa9 => Some(20),
                    0xaa | 0xab => Some(21),
                    _ => None,
                };
                if let Some(slot) = slot {
                    uses.push(RegisterUse {
                        source: source.into(),
                        node,
                        segment,
                        slot: slot as u8,
                        right_operand: false,
                    });
                    if matches!(f, 0x94..=0xa7 | 0xa9 | 0xab) && s.p1 >= 14 {
                        let rhs = u8::try_from(s.p2)
                            .ok()
                            .filter(|s| *s <= 21)
                            .ok_or("invalid RHS register index")?;
                        uses.push(RegisterUse {
                            source: source.into(),
                            node,
                            segment,
                            slot: rhs,
                            right_operand: true,
                        });
                    }
                }
            }
        }
        self.sources.insert(source.into());
        self.uses.extend(uses);
        Ok(())
    }
    pub fn unused_slots(&self) -> Vec<u8> {
        (0..=21)
            .filter(|s| !self.uses.iter().any(|u| u.slot == *s))
            .collect()
    }
    pub fn require_unreferenced(&self, slots: &[u8]) -> Result<(), String> {
        if self.sources.is_empty() {
            return Err("empty register audit".into());
        }
        for slot in slots {
            if *slot > 21 {
                return Err("physical register slot must be 0..21".into());
            }
            if let Some(u) = self.uses.iter().find(|u| u.slot == *slot) {
                return Err(format!(
                    "slot {slot} referenced at {} node {} segment {}",
                    u.source, u.node, u.segment
                ));
            }
        }
        Ok(())
    }
}
